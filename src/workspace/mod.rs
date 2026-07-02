//! Application use-case: workspace-mode orchestration — the impure glue that
//! ties the driven adapters, the per-crate [`pipeline`], and the workspace
//! [`report`](crate::report::workspace) together into one run.
//!
//! This is a *control* in the ECB sense: it sequences a use case by talking to
//! ports (the adapters) and folds their results through the pure aggregation.
//! The pure domain (`classify`/`gate`/`identity`/`diff`/`transition`) never
//! learns this exists — dependencies point inward. Every decision that *can* be
//! made without spawning a process is factored into a pure helper so it is
//! unit-testable; the spawning boundaries ([`cargo_metadata`], [`git`],
//! [`public_api_process`]) are exercised by the integration suite instead.
//!
//! The run, in order (see [`run`]):
//! 1. **Enumerate** the gated crates. *Zero discovered is a hard error* (exit 2),
//!    never a false green: `--workspace` must gate something.
//! 2. In process mode, **refuse a dirty tree** — `cargo public-api` git-checks-out
//!    each commit in-tree to build rustdoc JSON, which would clobber uncommitted
//!    changes. `--diff-dir` mode performs no checkout and skips this check.
//! 3. With `--changed-only`, **skip** gated crates no changed file touched — but
//!    they are *listed* as skipped, never silently absent.
//! 4. **Evaluate** every gated, non-skipped crate through the single-crate
//!    pipeline. A per-crate failure (unreadable/malformed regime, missing diff,
//!    tool error) is *recorded* as an `Errored` entry and does **not** abort the
//!    remaining crates. Sequential is fine for v1; parallel diff generation is a
//!    non-goal.
//! 5. **Fold** the per-crate outcomes into the aggregated, name-sorted report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::adapters::cargo_metadata::{self, GatedCrate};
use crate::adapters::git::{self, CrateRoot};
use crate::adapters::{diff_dir, public_api_process, regime_file};
use crate::domain::classify::MatchMode;
use crate::domain::transition::RegimeTransition;
use crate::pipeline;
use crate::report::workspace::{build, CrateEntry, CrateOutcome, WorkspaceReport};
use crate::report::Report;

/// Where each gated crate's public-API diff comes from.
///
/// - `Process` — the default: shell out to `cargo +nightly public-api -p <crate>
///   diff <base>..HEAD`. Needs `--base` and refuses a dirty tree.
/// - `Dir(path)` — read a pre-captured `<crate>.diff` from `path`. Performs no
///   git checkout, so it neither needs `--base` (unless `--changed-only`) nor
///   checks tree dirtiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffMode {
    Process,
    Dir(PathBuf),
}

/// The workspace run's configuration — the decoded intent of the `--workspace`
/// flags, framework-free. The driving adapter (the CLI) builds this; [`run`]
/// consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The base ref for `base..HEAD`. Required in process mode and whenever
    /// `--changed-only` is set; ignored (and may be absent) otherwise.
    pub base: Option<String>,
    /// The diff source (see [`DiffMode`]).
    pub diff_mode: DiffMode,
    /// Skip gated crates that no changed file touched (still listing them).
    pub changed_only: bool,
    /// The identity-matching mode threaded through to the per-crate pipeline.
    pub match_mode: MatchMode,
}

impl Config {
    /// Whether `--base` must be supplied: process mode always needs it (to build
    /// `cargo public-api diff base..HEAD`), and `--changed-only` needs it in any
    /// mode (to compute `git diff base..HEAD`). Pure `--diff-dir` without
    /// `--changed-only` touches no git and ignores `--base`.
    #[must_use]
    fn base_required(&self) -> bool {
        matches!(self.diff_mode, DiffMode::Process) || self.changed_only
    }

    /// The base ref to use, validated against [`Self::base_required`]. Returns
    /// `Ok(Some(base))` when required and supplied, `Ok(None)` when not required
    /// (so it is *ignored*, even if supplied), and a usage [`Failure`] (exit 2)
    /// when required but absent — a workspace flag combination that cannot run is
    /// a usage error, never a silent no-op.
    fn resolved_base(&self) -> Result<Option<&str>, Failure> {
        if self.base_required() {
            match self.base.as_deref() {
                Some(base) => Ok(Some(base)),
                None => Err(base_required_failure(self)),
            }
        } else {
            Ok(None)
        }
    }
}

/// A whole-run failure: a condition that aborts the entire workspace run before
/// (or instead of) producing a report. It maps to exit code 2.
///
/// Distinct from a per-crate `Errored` *outcome*, which is recorded inside the
/// [`WorkspaceReport`] and does not abort the run (though it too drives the
/// aggregate verdict to `error`/exit 2). A `Failure` means the run could not even
/// be attempted coherently: nothing to gate, a dirty tree, an unusable flag
/// combination, or a failed enumeration/git query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub hint: String,
}

/// A per-crate failure encountered while evaluating one gated crate. Internal:
/// it is immediately folded into a [`CrateOutcome::Errored`] so it is *recorded*,
/// never propagated (which would abort the sibling crates).
struct CrateError {
    message: String,
    hint: Option<String>,
}

/// Run workspace mode against the workspace rooted at `dir`.
///
/// Returns the aggregated [`WorkspaceReport`] (whose verdict maps to exit 0/1/2)
/// on a run that got far enough to gate crates, or a whole-run [`Failure`]
/// (exit 2) when the run could not be attempted (see [`Failure`]).
pub fn run(config: &Config, dir: &Path) -> Result<WorkspaceReport, Failure> {
    // 1. Enumerate the gated crates. Zero discovered is a hard error, never a
    //    false green — the "--require-domain instinct": refuse to exit having
    //    gated nothing.
    let gated: Vec<GatedCrate> = cargo_metadata::gated_crates(dir).map_err(enumeration_failure)?;
    if gated.is_empty() {
        return Err(no_gated_crates_failure());
    }

    // Validate the --base requirement before any git/process work (a usage error).
    let base: Option<&str> = config.resolved_base()?;

    // 2. Process mode refuses a dirty tree; --diff-dir mode performs no checkout
    //    and skips this check (short-circuit: is_dirty is not called otherwise).
    if matches!(config.diff_mode, DiffMode::Process) && git::is_dirty(dir).map_err(git_failure)? {
        return Err(dirty_tree_failure());
    }

    // 3. --changed-only: gated crates no changed file touched are skipped (listed,
    //    never absent). base is Some here — required and validated above.
    let skip: BTreeSet<String> = match (config.changed_only, base) {
        (true, Some(base)) => skip_set(dir, base, &gated)?,
        _ => BTreeSet::new(),
    };

    // 4. Evaluate each gated, non-skipped crate. A per-crate failure is recorded,
    //    not fatal. 5. Fold into the aggregated, name-sorted report.
    let entries: Vec<CrateEntry> = evaluate_all(config, dir, &gated, base, &skip);
    Ok(build(entries))
}

/// The set of gated-crate names to *skip* under `--changed-only`: those whose
/// root no changed file touched. Runs `git diff --name-only base..HEAD`, maps the
/// gated crates' roots into the workspace-relative frame git reports in, and
/// derives the skip set as the members `crates_touched` did *not* return.
///
/// A gated crate whose root cannot be expressed relative to `dir` is deliberately
/// omitted from the skip candidates (it is evaluated, not skipped) — refusing to
/// risk a false skip that would hide a real change.
fn skip_set(dir: &Path, base: &str, gated: &[GatedCrate]) -> Result<BTreeSet<String>, Failure> {
    let changed: Vec<PathBuf> = git::changed_files(dir, base).map_err(git_failure)?;
    let members: Vec<CrateRoot> = gated
        .iter()
        .filter_map(|crate_: &GatedCrate| {
            crate_
                .root
                .strip_prefix(dir)
                .ok()
                .map(|rel: &Path| CrateRoot {
                    name: crate_.name.clone(),
                    root: rel.to_path_buf(),
                })
        })
        .collect();
    let touched: BTreeSet<String> = git::crates_touched(&changed, &members);
    let skipped: BTreeSet<String> = members
        .into_iter()
        .map(|member: CrateRoot| member.name)
        .filter(|name: &String| !touched.contains(name))
        .collect();
    Ok(skipped)
}

/// Evaluate every gated crate in enumeration order, honouring the skip set. Each
/// crate becomes exactly one [`CrateEntry`]: `Skipped` if in `skip`, else the
/// outcome of [`evaluate_crate`] (`Gated` or the recorded `Errored`). A per-crate
/// error never removes or reorders siblings — every gated crate is present.
fn evaluate_all(
    config: &Config,
    dir: &Path,
    gated: &[GatedCrate],
    base: Option<&str>,
    skip: &BTreeSet<String>,
) -> Vec<CrateEntry> {
    let mut entries: Vec<CrateEntry> = Vec::with_capacity(gated.len());
    for crate_ in gated {
        let outcome: CrateOutcome = if skip.contains(&crate_.name) {
            CrateOutcome::Skipped {
                reason: skip_reason(base),
            }
        } else {
            evaluate_crate(config, dir, crate_, base)
        };
        entries.push(CrateEntry {
            name: crate_.name.clone(),
            outcome,
        });
    }
    entries
}

/// Evaluate one gated crate to its [`CrateOutcome`], folding any per-crate error
/// into a recorded [`CrateOutcome::Errored`] rather than propagating it.
fn evaluate_crate(
    config: &Config,
    dir: &Path,
    gated: &GatedCrate,
    base: Option<&str>,
) -> CrateOutcome {
    match evaluate_gated(config, dir, gated, base) {
        Ok(report) => CrateOutcome::Gated(report),
        Err(error) => CrateOutcome::Errored {
            message: error.message,
            hint: error.hint,
        },
    }
}

/// The fallible core of evaluating one gated crate: read + parse its regime file,
/// acquire its diff via the configured source, then run the single-crate
/// [`pipeline`]. Every failure is a [`CrateError`] the caller records.
fn evaluate_gated(
    config: &Config,
    dir: &Path,
    gated: &GatedCrate,
    base: Option<&str>,
) -> Result<Report, CrateError> {
    let regime_text: String =
        std::fs::read_to_string(&gated.regime_path).map_err(|source: std::io::Error| {
            CrateError {
                message: format!(
                    "cannot read regime file {}: {source}",
                    gated.regime_path.display()
                ),
                hint: Some("ensure the crate's regime-transition.toml is readable.".to_owned()),
            }
        })?;
    let u: RegimeTransition =
        regime_file::parse(&regime_text).map_err(|error: regime_file::RegimeFileError| {
            CrateError {
                message: format!(
                    "malformed regime file {}: {error}",
                    gated.regime_path.display()
                ),
                hint: Some(
                    "fix it against the minimal template from `cargo regime-check --template`."
                        .to_owned(),
                ),
            }
        })?;
    let diff_text: String = acquire_diff(config, dir, &gated.name, base)?;
    Ok(pipeline::classify_and_gate(
        &u,
        &diff_text,
        config.match_mode,
    ))
}

/// Acquire one crate's diff text from the configured [`DiffMode`]. In process
/// mode `base` is `Some` (validated in [`run`]); the defensive `None` arm keeps
/// this total rather than panicking on an invariant the caller upholds.
fn acquire_diff(
    config: &Config,
    dir: &Path,
    name: &str,
    base: Option<&str>,
) -> Result<String, CrateError> {
    match &config.diff_mode {
        DiffMode::Process => {
            let base: &str = base.ok_or_else(|| CrateError {
                message: "internal error: --base missing in process mode".to_owned(),
                hint: Some("pass --base <ref>.".to_owned()),
            })?;
            public_api_process::diff(dir, name, base).map_err(
                |error: public_api_process::PublicApiError| {
                    CrateError {
                message: format!("`cargo public-api` failed for `{name}`: {error}"),
                hint: Some(
                    "ensure `cargo public-api` is installed and a nightly toolchain is available \
                     (`cargo +nightly public-api --version`); or capture diffs and use --diff-dir."
                        .to_owned(),
                ),
            }
                },
            )
        }
        DiffMode::Dir(diff_dir_path) => {
            diff_dir::read(diff_dir_path, name).map_err(|error: diff_dir::DiffDirError| {
                CrateError {
                    hint: Some(diff_dir_hint(&error)),
                    message: error.to_string(),
                }
            })
        }
    }
}

/// The reason recorded on a crate skipped by `--changed-only`. `base` is `Some`
/// wherever the skip set is non-empty (skips are computed only in changed-only
/// mode, where `--base` is required).
fn skip_reason(base: Option<&str>) -> String {
    match base {
        Some(base) => {
            format!("no file under the crate root changed since {base} (--changed-only)")
        }
        None => "no file under the crate root changed (--changed-only)".to_owned(),
    }
}

/// The remediation hint for a `--diff-dir` read failure. A *missing* diff for a
/// gated crate is an error, never a silent skip: the hint says how to generate it.
fn diff_dir_hint(error: &diff_dir::DiffDirError) -> String {
    match error {
        diff_dir::DiffDirError::Missing { crate_name, .. } => format!(
            "generate `{crate_name}.diff` into the --diff-dir (e.g. \
             `cargo +nightly public-api -p {crate_name} diff <base>..HEAD > <dir>/{crate_name}.diff`), \
             or drop that crate's regime-transition.toml to stop gating it. A gated crate with no diff \
             is an error, not a skip."
        ),
        diff_dir::DiffDirError::Io { .. } => {
            "check the diff file's permissions and that it is valid UTF-8.".to_owned()
        }
    }
}

// ---- whole-run failure constructors ---------------------------------------

/// The zero-gated-crates failure (exit 2): refuse to exit green having gated
/// nothing, and say how to opt a crate in.
fn no_gated_crates_failure() -> Failure {
    Failure {
        message: "no workspace member has a regime-transition.toml; --workspace gated nothing"
            .to_owned(),
        hint: "add a regime-transition.toml at a crate root (next to its Cargo.toml) to gate it — \
               bootstrap one with `cargo regime-check --template > <crate>/regime-transition.toml`. \
               Refusing to exit 0 having gated nothing."
            .to_owned(),
    }
}

/// The missing-`--base` usage failure (exit 2), naming *why* the base is required
/// for the current mode and the `--diff-dir` escape from needing one.
fn base_required_failure(config: &Config) -> Failure {
    let why: &str = if matches!(config.diff_mode, DiffMode::Process) {
        "the default (public-api process) mode diffs `base..HEAD`"
    } else {
        "`--changed-only` diffs `base..HEAD` to find which crates changed"
    };
    Failure {
        message: format!("--base <ref> is required: {why}"),
        hint:
            "pass --base <ref> (e.g. --base origin/main); or, to gate pre-captured diffs with no \
               git, use --diff-dir <dir> without --changed-only."
                .to_owned(),
    }
}

/// The dirty-working-tree failure (exit 2). Its wording NAMES all three things an
/// operator needs: the in-tree-checkout caveat, the `git worktree` escape hatch,
/// and the `--diff-dir` alternative.
fn dirty_tree_failure() -> Failure {
    Failure {
        message: "working tree is dirty; refusing to run public-api process mode — \
                  `cargo public-api` git-checks-out each commit in-tree to build rustdoc JSON, \
                  which would clobber your uncommitted changes"
            .to_owned(),
        hint: "commit or stash your changes; or build in a throwaway checkout with \
               `git worktree add /tmp/regime-wt HEAD` and run there; or capture the diffs ahead of \
               time and gate them with --diff-dir <dir> (no checkout, and the dirtiness check is skipped)."
            .to_owned(),
    }
}

/// Map a `cargo metadata` enumeration failure to a whole-run [`Failure`] (exit 2).
fn enumeration_failure(error: cargo_metadata::CargoMetadataError) -> Failure {
    Failure {
        message: format!("could not enumerate workspace crates: {error}"),
        hint:
            "run --workspace from a Cargo workspace or crate root where `cargo metadata` succeeds."
                .to_owned(),
    }
}

/// Map a git-query failure (dirtiness or changed-files) to a whole-run
/// [`Failure`] (exit 2).
fn git_failure(error: git::GitError) -> Failure {
    Failure {
        message: format!("git query failed: {error}"),
        hint: "run --workspace inside a git repository, and ensure --base names a ref reachable \
               from HEAD."
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::workspace::CrateStatus;

    // A minimal readable regime file — enough for `regime_file::parse` to succeed.
    const REGIME: &str = "[meta]\nkind = \"transition\"\n";

    fn process_config(base: Option<&str>) -> Config {
        Config {
            base: base.map(str::to_owned),
            diff_mode: DiffMode::Process,
            changed_only: false,
            match_mode: MatchMode::Identity,
        }
    }

    fn dir_config(base: Option<&str>, changed_only: bool, dir: PathBuf) -> Config {
        Config {
            base: base.map(str::to_owned),
            diff_mode: DiffMode::Dir(dir),
            changed_only,
            match_mode: MatchMode::Identity,
        }
    }

    // A unique scratch subdirectory for one test (isolated across tests and runs).
    fn scratch(name: &str) -> PathBuf {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("regime-ws-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn gated_crate(root: &Path, name: &str) -> GatedCrate {
        GatedCrate {
            name: name.to_owned(),
            root: root.to_path_buf(),
            regime_path: root.join("regime-transition.toml"),
        }
    }

    // ---- --base requirement (pure config validation) ----

    // zero: process mode with no base is a usage failure (exit 2, not a no-op).
    #[test]
    fn process_mode_requires_base() {
        let config: Config = process_config(None);
        assert!(config.resolved_base().is_err());
    }

    // one: process mode with a base resolves to exactly that base.
    #[test]
    fn process_mode_with_base_resolves_it() {
        let config: Config = process_config(Some("origin/main"));
        assert_eq!(config.resolved_base().unwrap(), Some("origin/main"));
    }

    // --changed-only needs a base even in diff-dir mode (git diffs base..HEAD).
    #[test]
    fn changed_only_requires_base_even_in_dir_mode() {
        let config: Config = dir_config(None, true, PathBuf::from("/diffs"));
        assert!(config.resolved_base().is_err());
    }

    // pure diff-dir without --changed-only ignores an absent base (no git needed).
    #[test]
    fn pure_dir_mode_ignores_absent_base() {
        let config: Config = dir_config(None, false, PathBuf::from("/diffs"));
        assert_eq!(config.resolved_base().unwrap(), None);
    }

    // pure diff-dir ignores even a supplied base (it is not used without git).
    #[test]
    fn pure_dir_mode_ignores_supplied_base() {
        let config: Config = dir_config(Some("main"), false, PathBuf::from("/diffs"));
        assert_eq!(config.resolved_base().unwrap(), None);
    }

    // ---- per-crate evaluation (diff-dir source, temp dir; no spawning) ----

    // zero: no gated crates yields no entries.
    #[test]
    fn evaluate_all_zero_crates_is_empty() {
        let config: Config = dir_config(None, false, PathBuf::from("/diffs"));
        let entries: Vec<CrateEntry> =
            evaluate_all(&config, Path::new("/ws"), &[], None, &BTreeSet::new());
        assert!(entries.is_empty());
    }

    // one: a single gated crate whose diff is present is Gated (clean here).
    #[test]
    fn evaluate_all_one_gated_crate_with_diff_is_gated() {
        let root: PathBuf = scratch("one-gated");
        let crate_root: PathBuf = root.join("solo");
        let diffs: PathBuf = root.join("diffs");
        std::fs::create_dir_all(&crate_root).expect("mk solo");
        std::fs::create_dir_all(&diffs).expect("mk diffs");
        std::fs::write(crate_root.join("regime-transition.toml"), REGIME).expect("write regime");
        std::fs::write(diffs.join("solo.diff"), "").expect("write diff");
        let gated: Vec<GatedCrate> = vec![gated_crate(&crate_root, "solo")];
        let config: Config = dir_config(None, false, diffs);

        let entries: Vec<CrateEntry> = evaluate_all(&config, &root, &gated, None, &BTreeSet::new());

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome.status(), CrateStatus::Clean);
    }

    // many: one crate's missing diff is RECORDED as Errored and does NOT abort the
    // sibling — both crates are present, in enumeration order.
    #[test]
    fn per_crate_error_is_recorded_and_does_not_abort_others() {
        let root: PathBuf = scratch("recorded-not-fatal");
        let alpha_root: PathBuf = root.join("alpha");
        let beta_root: PathBuf = root.join("beta");
        let diffs: PathBuf = root.join("diffs");
        std::fs::create_dir_all(&alpha_root).expect("mk alpha");
        std::fs::create_dir_all(&beta_root).expect("mk beta");
        std::fs::create_dir_all(&diffs).expect("mk diffs");
        std::fs::write(alpha_root.join("regime-transition.toml"), REGIME).expect("alpha regime");
        std::fs::write(beta_root.join("regime-transition.toml"), REGIME).expect("beta regime");
        std::fs::write(diffs.join("alpha.diff"), "").expect("alpha diff");
        // beta.diff is deliberately absent.
        let gated: Vec<GatedCrate> = vec![
            gated_crate(&alpha_root, "alpha"),
            gated_crate(&beta_root, "beta"),
        ];
        let config: Config = dir_config(None, false, diffs);

        let entries: Vec<CrateEntry> = evaluate_all(&config, &root, &gated, None, &BTreeSet::new());

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].outcome.status(), CrateStatus::Clean);
        assert_eq!(entries[1].outcome.status(), CrateStatus::Errored);
    }

    // a crate in the skip set is Skipped without any diff acquisition — proven by
    // omitting its diff file: were it evaluated it would have Errored.
    #[test]
    fn a_skipped_crate_is_not_evaluated() {
        let root: PathBuf = scratch("skipped");
        let crate_root: PathBuf = root.join("solo");
        let diffs: PathBuf = root.join("diffs");
        std::fs::create_dir_all(&crate_root).expect("mk solo");
        std::fs::create_dir_all(&diffs).expect("mk diffs");
        std::fs::write(crate_root.join("regime-transition.toml"), REGIME).expect("write regime");
        // solo.diff deliberately absent.
        let gated: Vec<GatedCrate> = vec![gated_crate(&crate_root, "solo")];
        let config: Config = dir_config(Some("main"), true, diffs);
        let skip: BTreeSet<String> = BTreeSet::from(["solo".to_owned()]);

        let entries: Vec<CrateEntry> = evaluate_all(&config, &root, &gated, Some("main"), &skip);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome.status(), CrateStatus::Skipped);
    }

    // ---- whole-run failures carry the mandated guidance ----

    // the dirty-tree failure names the caveat, the worktree escape, and --diff-dir.
    #[test]
    fn dirty_tree_failure_names_caveat_worktree_and_diff_dir() {
        let failure: Failure = dirty_tree_failure();
        let full: String = format!("{} {}", failure.message, failure.hint);
        assert!(full.contains("in-tree"));
        assert!(full.contains("git worktree add"));
        assert!(full.contains("--diff-dir"));
    }

    // the zero-gated failure refuses a green and says how to opt a crate in.
    #[test]
    fn no_gated_crates_failure_refuses_green_and_says_how() {
        let failure: Failure = no_gated_crates_failure();
        let full: String = format!("{} {}", failure.message, failure.hint);
        assert!(full.contains("gated nothing"));
        assert!(full.contains("regime-transition.toml"));
    }
}
