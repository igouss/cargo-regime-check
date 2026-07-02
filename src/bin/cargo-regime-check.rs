//! `cargo regime-check --regime regime-transition.toml [--diff <file>|-] [--format json]`
//!
//! Reads a `cargo public-api diff` (stdin by default), subtracts the declared
//! transition `u`, and exits non-zero if any residual is undeclared. On FAIL it
//! prints exactly what to DO per residual line — stdout alone resolves it.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use cargo_regime_check::adapters::regime_file;
use cargo_regime_check::domain::classify::MatchMode;
use cargo_regime_check::domain::transition::RegimeTransition;
use cargo_regime_check::pipeline;
use cargo_regime_check::report::workspace::WorkspaceReport;
use cargo_regime_check::report::{self, Report};
use cargo_regime_check::workspace::{self, DiffMode};

const USAGE: &str = "\
cargo-regime-check — gate a public-API diff against a declared transition `u`.

A change is admissible iff every diff line is transported by a declared rename or
declared as additive/removal/change. `meta.kind = \"refactor\"` forbids any
declared residual (it must be an endofunctor); `\"transition\"` allows declared
residual only.

USAGE:
  cargo +nightly public-api -p <crate> diff <old>..<new> \\
    | cargo regime-check --regime regime-transition.toml [OPTIONS]
  cargo regime-check --regime <toml> --diff <file>

OPTIONS:
  -r, --regime <FILE>   declared transition (regime-transition.toml).
                        Required unless --explain / --template.
  -d, --diff <FILE|->   cargo public-api diff text; default: stdin (-).
      --format <FMT>    output format: human (default) | json
      --json            alias for --format json
      --match <MODE>    identity (default; exact resolved-path) | substring (legacy)
      --explain         print the model + known approximations, then exit 0
      --template        print a minimal regime-transition.toml, then exit 0
      --capabilities    print the machine-readable contract as JSON, then exit 0
  -h, --help            print this help, then exit 0

WORKSPACE:
  Gate EVERY workspace member that carries a regime-transition.toml in one run
  (discovered via `cargo metadata`). Without --workspace the tool is single-crate
  and the flags below are a usage error; --regime/--diff are single-crate flags
  and are a usage error WITH --workspace (each gated crate uses its own regime,
  and diffs come from the process or --diff-dir).

  cargo regime-check --workspace --base origin/main [OPTIONS]
  cargo regime-check --workspace --diff-dir target/regime-diffs [OPTIONS]

      --workspace       gate all gated crates at once; aggregate the verdicts.
      --base <GIT-REF>  diff `base..HEAD`. Required in the default (process) mode
                        and with --changed-only; ignored in pure --diff-dir mode.
      --diff-dir <DIR>  read each gated crate's pre-captured `<crate>.diff` from DIR
                        instead of running `cargo public-api`. Performs no git
                        checkout (and skips the dirty-tree refusal). A gated crate
                        with no diff file is an error, never a silent skip.
      --changed-only    skip (but still list) gated crates no file changed since
                        --base.

  Default (process) mode shells out to `cargo +nightly public-api -p <crate> diff
  <base>..HEAD` and refuses a dirty working tree (that command git-checks-out each
  commit in-tree to build rustdoc JSON, clobbering uncommitted changes). The
  aggregate exit code is the max over crates: 2 (error) > 1 (residual) > 0 (clean).

EXIT CODES:
  0  clean — every line transported or declared
  1  undeclared / contradictory residual — act on the directives printed to stdout
  2  usage / I/O / parse error

The tool never prompts and emits no colour in --format json: deterministic,
re-runnable, branch on the exit code. See AGENTS.md for the full contract.";

const EXPLAIN: &str = "\
cargo-regime-check — the model
==============================

A public-API change is admissible iff every line of the diff lies in the IMAGE of
the declared transport `u`. Anything outside that image is RESIDUAL:

  residual = (new public surface)  \\  image(u applied to the old surface)

  - the renames in regime-transition.toml ARE the functor `u` (its iso part).
  - meta.kind = \"refactor\"   => `u` must be an ENDOFUNCTOR: residual must be 0.
                                Declaring any add/remove/change contradicts the
                                claim — it is then a transition, not a refactor.
  - meta.kind = \"transition\" => residual is allowed, but every residual line must
                                be DECLARED ([[additive]] / [[removal]] / [[change]]).

This operationalizes \"residual content beyond functorial transport\" from Wang &
Buehler, arXiv:2606.01444 (old artifacts transported by the left Kan extension and
compared to the post-transition state), and the schema-as-category /
migration-as-functor machinery of Spivak's functorial data migration (CQL). The
category theory defines the buckets; none of it is in the code.

Known approximations (where the implementation diverges from the theory)
------------------------------------------------------------------------
1. The functor matches by RESOLVED IDENTITY PATH, parsed from `cargo public-api`'s
   already-normalized token — not from a structured identity field.
   `cargo public-api` 0.52.0 has no `--output json`; when it gains one, the diff
   adapter should consume it directly. `--match substring` restores the looser
   legacy behaviour for tokens the parser cannot resolve.
2. Renames are DECLARED, not INFERRED. The tool certifies a declared `u`; it does
   not discover it (no structural-hash rename inference / left Kan extension yet).
   A wrong `u` yields a confident-but-wrong green.
3. A changed signature is one undifferentiated bucket; widening (≈ additive) vs
   narrowing (≈ lossy) is not yet told apart by sub/supertype.
4. `--workspace` gates every workspace member with a regime-transition.toml in one
   run and aggregates their verdicts (exit 2 > 1 > 0), but each crate is still gated
   INDEPENDENTLY: there is no cross-crate transport, so a rename that moves an item
   from one crate to another reads as residual in both. Parallel diff generation is
   a non-goal.

See AGENTS.md for the full contract and the ranked roadmap.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Human,
    Json,
}

/// A fully resolved invocation: exactly one of the two modes the tool runs in.
/// `--help/--explain/--template/--capabilities` never reach here — they print and
/// exit 0 during parsing.
enum Invocation {
    Single(SingleArgs),
    Workspace(WorkspaceArgs),
}

/// The single-crate command: gate one piped/`--diff` diff against one `--regime`.
struct SingleArgs {
    regime_path: Option<String>,
    diff_path: Option<String>,
    format: Format,
    match_mode: MatchMode,
}

/// The `--workspace` command: gate every gated crate and aggregate. Holds the
/// framework-free [`workspace::Config`] plus the output format the CLI renders in.
struct WorkspaceArgs {
    config: workspace::Config,
    format: Format,
}

/// The raw, unvalidated flags parsed from argv, before mode resolution. Parsing
/// (token → field) is kept separate from resolution (field → [`Invocation`],
/// with the cross-flag validation) so each has one responsibility.
struct Raw {
    regime_path: Option<String>,
    diff_path: Option<String>,
    format: Format,
    match_mode: MatchMode,
    workspace: bool,
    base: Option<String>,
    diff_dir: Option<String>,
    changed_only: bool,
}

/// A usage/IO/parse failure: message, what to do about it, and whether to show
/// the template.
#[derive(Debug)]
struct Failure {
    message: String,
    hint: String,
    show_template: bool,
}

fn main() -> ExitCode {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    // Invoked as `cargo regime-check ...`: cargo passes the subcommand name.
    if argv.first().map(String::as_str) == Some("regime-check") {
        argv.remove(0);
    }

    // Bare invocation: show help and exit 0 — the first thing an agent tries works.
    if argv.is_empty() {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    match parse_args(&argv) {
        Ok(Some(Invocation::Single(args))) => run_single(&args),
        Ok(Some(Invocation::Workspace(args))) => run_workspace(&args),
        Ok(None) => ExitCode::SUCCESS,
        Err(f) => emit_failure(&f, Format::Human),
    }
}

/// Run the single-crate pipeline and map its verdict to the process exit code
/// (pass → 0, residual → 1, IO/parse failure → 2). Behaviour here is the tool's
/// original single-crate contract, unchanged.
fn run_single(args: &SingleArgs) -> ExitCode {
    match run(args) {
        Ok(report) => {
            print!("{}", render(&report, args.format));
            if matches!(report.verdict, report::Verdict::Pass) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE // exit 1
            }
        }
        Err(f) => emit_failure(&f, args.format),
    }
}

/// Run workspace mode against the current working directory and map the aggregated
/// verdict to the process exit code: `pass → 0`, `fail → 1`, `error → 2` (the
/// per-crate max, `error > residual > clean`). A whole-run failure — a dirty tree,
/// nothing gated, an unusable flag combination, or a failed metadata/git query —
/// is emitted as an exit-2 error.
fn run_workspace(args: &WorkspaceArgs) -> ExitCode {
    let cwd: PathBuf = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(source) => return emit_failure(&cwd_failure(&source), args.format),
    };
    match workspace::run(&args.config, &cwd) {
        Ok(report) => {
            print!("{}", render_workspace(&report, args.format));
            ExitCode::from(report.verdict.exit_code())
        }
        Err(f) => emit_failure(&workspace_failure(&f), args.format),
    }
}

/// Parse argv. Returns `Ok(None)` when a self-contained flag already handled the
/// whole invocation (help/explain/template/capabilities) and the process should
/// exit 0. Otherwise it collects the raw flags and hands them to [`resolve`],
/// which decides the mode and validates the flag combination.
fn parse_args(argv: &[String]) -> Result<Option<Invocation>, Failure> {
    let mut regime_path: Option<String> = None;
    let mut diff_path: Option<String> = None;
    let mut format: Format = Format::Human;
    let mut match_mode: MatchMode = MatchMode::Identity;
    let mut workspace: bool = false;
    let mut base: Option<String> = None;
    let mut diff_dir: Option<String> = None;
    let mut changed_only: bool = false;

    let mut i: usize = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--explain" => {
                println!("{EXPLAIN}");
                return Ok(None);
            }
            "--template" => {
                print!("{}", regime_file::template());
                return Ok(None);
            }
            "--capabilities" => {
                println!("{}", capabilities());
                return Ok(None);
            }
            // Intent inference: `--json` is the obvious guess for JSON output —
            // accept it as an alias for `--format json` instead of erroring.
            "--json" => format = Format::Json,
            "-r" | "--regime" => {
                i += 1;
                regime_path = Some(value(argv, i, "--regime")?);
            }
            "-d" | "--diff" => {
                i += 1;
                diff_path = Some(value(argv, i, "--diff")?);
            }
            "--format" => {
                i += 1;
                format = match value(argv, i, "--format")?.as_str() {
                    "human" => Format::Human,
                    "json" => Format::Json,
                    other => return Err(usage(format!("unknown --format `{other}`"))),
                };
            }
            "--match" => {
                i += 1;
                match_mode = match value(argv, i, "--match")?.as_str() {
                    "identity" => MatchMode::Identity,
                    "substring" => MatchMode::Substring,
                    other => return Err(usage(format!("unknown --match `{other}`"))),
                };
            }
            "--workspace" => workspace = true,
            "--base" => {
                i += 1;
                base = Some(value(argv, i, "--base")?);
            }
            "--diff-dir" => {
                i += 1;
                diff_dir = Some(value(argv, i, "--diff-dir")?);
            }
            "--changed-only" => changed_only = true,
            other => return Err(unknown_arg(other)),
        }
        i += 1;
    }

    resolve(Raw {
        regime_path,
        diff_path,
        format,
        match_mode,
        workspace,
        base,
        diff_dir,
        changed_only,
    })
    .map(Some)
}

/// Resolve the raw flags into a concrete [`Invocation`], enforcing the cross-flag
/// rules so no flag is ever a silent no-op:
///
/// - WITH `--workspace`: `--regime`/`--diff` are single-crate flags with no meaning
///   here (each gated crate uses its own regime; diffs come from the process or
///   `--diff-dir`), so supplying them is a usage error. `--diff-dir` selects the
///   diff source; its absence means process mode. `--base` is threaded through and
///   validated *inside* [`workspace::run`] against the chosen mode.
/// - WITHOUT `--workspace`: any workspace-only flag (`--base`, `--diff-dir`,
///   `--changed-only`) is a usage error rather than being silently ignored.
fn resolve(raw: Raw) -> Result<Invocation, Failure> {
    if raw.workspace {
        if raw.regime_path.is_some() {
            return Err(single_flag_with_workspace(
                "--regime",
                "each gated crate uses its own regime-transition.toml",
            ));
        }
        if raw.diff_path.is_some() {
            return Err(single_flag_with_workspace(
                "--diff",
                "workspace diffs come from the public-api process or --diff-dir",
            ));
        }
        let diff_mode: DiffMode = match raw.diff_dir {
            Some(dir) => DiffMode::Dir(PathBuf::from(dir)),
            None => DiffMode::Process,
        };
        Ok(Invocation::Workspace(WorkspaceArgs {
            config: workspace::Config {
                base: raw.base,
                diff_mode,
                changed_only: raw.changed_only,
                match_mode: raw.match_mode,
            },
            format: raw.format,
        }))
    } else {
        if raw.base.is_some() {
            return Err(workspace_flag_without_workspace("--base"));
        }
        if raw.diff_dir.is_some() {
            return Err(workspace_flag_without_workspace("--diff-dir"));
        }
        if raw.changed_only {
            return Err(workspace_flag_without_workspace("--changed-only"));
        }
        Ok(Invocation::Single(SingleArgs {
            regime_path: raw.regime_path,
            diff_path: raw.diff_path,
            format: raw.format,
            match_mode: raw.match_mode,
        }))
    }
}

/// The classify→gate→build pipeline, with all I/O and parse failures surfaced.
fn run(args: &SingleArgs) -> Result<Report, Failure> {
    let Some(regime_path) = args.regime_path.as_deref() else {
        return Err(usage("--regime <toml> is required".to_owned()));
    };

    let regime_text: String =
        std::fs::read_to_string(regime_path).map_err(|e: std::io::Error| Failure {
            message: format!("cannot read regime file `{regime_path}`: {e}"),
            hint: format!(
                "create it — write the template below to `{regime_path}` (next to Cargo.toml) \
                 or run `cargo regime-check --template > {regime_path}`."
            ),
            show_template: true,
        })?;

    let u: RegimeTransition = regime_file::parse(&regime_text).map_err(|e| Failure {
        message: format!("malformed regime file `{regime_path}`: {e}"),
        hint: "fix it against the minimal valid template below.".to_owned(),
        show_template: true,
    })?;

    let diff_text: String = read_diff(args.diff_path.as_deref()).map_err(|e| Failure {
        message: format!("cannot read diff: {e}"),
        hint: "pipe `cargo +nightly public-api -p <crate> diff <old>..<new>` into stdin, \
               or pass --diff <file>."
            .to_owned(),
        show_template: false,
    })?;

    Ok(pipeline::classify_and_gate(&u, &diff_text, args.match_mode))
}

fn read_diff(path: Option<&str>) -> std::io::Result<String> {
    match path {
        None | Some("-") => {
            let mut buf: String = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
        Some(p) => std::fs::read_to_string(p),
    }
}

fn render(report: &Report, format: Format) -> String {
    match format {
        Format::Human => report::human::render(report),
        Format::Json => format!("{}\n", report::json::render(report)),
    }
}

/// Render the aggregated workspace report. JSON gets a trailing newline to match
/// the single-crate JSON path, so a shell redirect ends the file with one.
fn render_workspace(report: &WorkspaceReport, format: Format) -> String {
    match format {
        Format::Human => report::workspace::render_human(report),
        Format::Json => format!("{}\n", report::workspace::render_json(report)),
    }
}

fn value(argv: &[String], i: usize, flag: &str) -> Result<String, Failure> {
    argv.get(i)
        .cloned()
        .ok_or_else(|| usage(format!("{flag} needs a value")))
}

fn usage(message: String) -> Failure {
    Failure {
        message,
        hint: "run `cargo regime-check --help` for usage.".to_owned(),
        show_template: false,
    }
}

/// A workspace-only flag supplied without `--workspace` (exit 2). Never a silent
/// no-op: the flag would do nothing in single-crate mode, so it is an error.
fn workspace_flag_without_workspace(flag: &str) -> Failure {
    Failure {
        message: format!("{flag} only applies with --workspace"),
        hint: format!(
            "add --workspace to gate every gated crate at once, or drop {flag}. \
             run `cargo regime-check --help` for the WORKSPACE section."
        ),
        show_template: false,
    }
}

/// A single-crate flag supplied together with `--workspace` (exit 2). Never a
/// silent no-op: workspace mode does not consult `--regime`/`--diff`.
fn single_flag_with_workspace(flag: &str, why: &str) -> Failure {
    Failure {
        message: format!(
            "{flag} is a single-crate flag and has no meaning with --workspace ({why})"
        ),
        hint: "drop --workspace to gate one crate, or drop this flag to gate the whole workspace. \
               run `cargo regime-check --help` for usage."
            .to_owned(),
        show_template: false,
    }
}

/// Adapt a whole-run [`workspace::Failure`] (exit 2) to the CLI's [`Failure`], so
/// it is emitted through the same human/json path as every other failure.
fn workspace_failure(failure: &workspace::Failure) -> Failure {
    Failure {
        message: failure.message.clone(),
        hint: failure.hint.clone(),
        show_template: false,
    }
}

/// The current-directory read failure (exit 2) that aborts `--workspace` before
/// any enumeration: workspace mode is rooted at the cwd.
fn cwd_failure(source: &std::io::Error) -> Failure {
    Failure {
        message: format!("cannot determine the current directory: {source}"),
        hint:
            "run --workspace from a readable working directory — a Cargo workspace or crate root."
                .to_owned(),
        show_template: false,
    }
}

/// Every flag the parser accepts — the corpus for "did you mean" suggestions.
const KNOWN_FLAGS: &[&str] = &[
    "--regime",
    "--diff",
    "--format",
    "--match",
    "--json",
    "--explain",
    "--template",
    "--capabilities",
    "--help",
    "--workspace",
    "--base",
    "--diff-dir",
    "--changed-only",
];

/// An unknown argument, with a Levenshtein-nearest "did you mean" when one is
/// close enough — an agent that mistypes once learns the spelling.
fn unknown_arg(arg: &str) -> Failure {
    let suggestion: Option<&&str> = KNOWN_FLAGS
        .iter()
        .map(|flag: &&str| (levenshtein(arg, flag), flag))
        .filter(|(d, _): &(usize, &&str)| *d <= 3)
        .min_by_key(|(d, _): &(usize, &&str)| *d)
        .map(|(_, flag): (usize, &&str)| flag);
    let hint: String = match suggestion {
        Some(flag) => format!("did you mean `{flag}`? run `cargo regime-check --help` for usage."),
        None => "run `cargo regime-check --help` for usage.".to_owned(),
    };
    Failure {
        message: format!("unknown argument `{arg}`"),
        hint,
        show_template: false,
    }
}

/// Plain Levenshtein edit distance (no unsafe, small inputs).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost: usize = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// The machine-readable contract: version, exit codes, formats, match modes, and
/// the report schema. Lets an agent read the contract straight from the tool
/// instead of remembering it. Deterministic (a static value).
fn capabilities() -> String {
    let value: serde_json::Value = serde_json::json!({
        "tool": "cargo-regime-check",
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": 1,
        "exit_codes": {
            "0": "clean — every line transported or declared",
            "1": "undeclared/contradictory residual",
            "2": "usage/IO/parse error"
        },
        "formats": ["human", "json"],
        "match_modes": ["identity", "substring"],
        "regime_kinds": ["refactor", "transition"],
        "classes": [
            "transported_iso",
            "declared_additive",
            "declared_removal",
            "declared_change",
            "residual_additive",
            "residual_removal",
            "residual_change"
        ],
        "required_actions": [
            "DeclareAdditive",
            "DeclareRemoval",
            "DeclareChange",
            "ReclassifyAsTransition"
        ],
        "report_schema": {
            "verdict": "pass|fail",
            "kind": "refactor|transition",
            "counts": ["total", "accounted", "residual", "violations"],
            "items": ["token", "path", "class", "detail", "required_action", "remediation"]
        },
        "requires_nightly_for": "cargo public-api (run separately and piped in; this tool runs on stable)",
        "workspace": {
            "flag": "--workspace",
            "flags": {
                "--workspace": "gate every workspace member carrying a regime-transition.toml in one run",
                "--base": "git ref for base..HEAD; required in the default process mode and with --changed-only; ignored in pure --diff-dir mode",
                "--diff-dir": "read each gated crate's pre-captured <crate>.diff from a directory instead of running cargo public-api (no git checkout; a gated crate with no diff file is an error, not a skip)",
                "--changed-only": "skip (but still list) gated crates no file changed since --base"
            },
            "verdicts": ["pass", "fail", "error"],
            "verdict_exit_codes": {
                "pass": 0,
                "fail": 1,
                "error": 2
            },
            "exit_priority": "aggregate is the max over crates: error(2) > residual(1) > clean(0)",
            "counts": ["crates", "clean", "residual", "errored", "skipped"],
            "counts_note": "crates = clean + residual + errored (the evaluated crates); skipped crates are NOT counted in crates",
            "crate_statuses": ["clean", "residual", "errored", "skipped"],
            "crate_entry": {
                "always": ["name", "status"],
                "clean|residual": ["report"],
                "errored": ["error", "hint"],
                "skipped": ["reason"],
                "report": "the single-crate report object, embedded UNCHANGED (equal to a standalone single-crate --format json run of that crate)"
            }
        }
    });
    serde_json::to_string_pretty(&value).expect("capabilities value serializes")
}

/// Print a failure (exit 2). In JSON mode it goes to stdout as a structured
/// object so an agent can parse it; in human mode it goes to stderr with the
/// template inline when relevant.
fn emit_failure(f: &Failure, format: Format) -> ExitCode {
    match format {
        Format::Json => {
            let template: Option<String> = f.show_template.then(regime_file::template);
            let value: serde_json::Value = serde_json::json!({
                "error": f.message,
                "hint": f.hint,
                "template": template,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("error value serializes")
            );
        }
        Format::Human => {
            eprintln!("error: {}", f.message);
            eprintln!("hint: {}", f.hint);
            if f.show_template {
                eprintln!("\n--- minimal regime-transition.toml ---");
                eprint!("{}", regime_file::template());
            }
        }
    }
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Baseline raw flags: bare single-crate invocation with nothing set. Each test
    /// mutates only the fields it exercises.
    fn raw() -> Raw {
        Raw {
            regime_path: None,
            diff_path: None,
            format: Format::Human,
            match_mode: MatchMode::Identity,
            workspace: false,
            base: None,
            diff_dir: None,
            changed_only: false,
        }
    }

    /// Extract the [`workspace::Config`] from a resolved invocation, panicking if it
    /// resolved to single-crate mode — keeps the asserting test at complexity 1.
    fn expect_workspace(invocation: Invocation) -> workspace::Config {
        match invocation {
            Invocation::Workspace(args) => args.config,
            Invocation::Single(_) => panic!("expected a workspace invocation"),
        }
    }

    /// Extract the [`SingleArgs`] from a resolved invocation, panicking if it
    /// resolved to workspace mode.
    fn expect_single(invocation: Invocation) -> SingleArgs {
        match invocation {
            Invocation::Single(args) => args,
            Invocation::Workspace(_) => panic!("expected a single-crate invocation"),
        }
    }

    // ---- mode selection ----

    // bare flags (no --workspace, no workspace flags) stay single-crate.
    #[test]
    fn bare_flags_resolve_to_single_mode() {
        let args: SingleArgs = expect_single(resolve(raw()).unwrap());
        assert!(args.regime_path.is_none());
    }

    // --workspace without --diff-dir selects the default (public-api process) mode.
    #[test]
    fn workspace_without_diff_dir_is_process_mode() {
        let mut r: Raw = raw();
        r.workspace = true;
        let config: workspace::Config = expect_workspace(resolve(r).unwrap());
        assert_eq!(config.diff_mode, DiffMode::Process);
    }

    // --workspace with --diff-dir selects the pre-captured directory diff source.
    #[test]
    fn workspace_with_diff_dir_is_dir_mode() {
        let mut r: Raw = raw();
        r.workspace = true;
        r.diff_dir = Some("/diffs".to_owned());
        let config: workspace::Config = expect_workspace(resolve(r).unwrap());
        assert_eq!(config.diff_mode, DiffMode::Dir(PathBuf::from("/diffs")));
    }

    // --workspace threads --base, --changed-only, and --match straight into the
    // framework-free config (base-requirement validation happens in workspace::run).
    #[test]
    fn workspace_threads_base_changed_only_and_match_into_config() {
        let mut r: Raw = raw();
        r.workspace = true;
        r.base = Some("origin/main".to_owned());
        r.changed_only = true;
        r.match_mode = MatchMode::Substring;
        let config: workspace::Config = expect_workspace(resolve(r).unwrap());
        assert_eq!(
            config,
            workspace::Config {
                base: Some("origin/main".to_owned()),
                diff_mode: DiffMode::Process,
                changed_only: true,
                match_mode: MatchMode::Substring,
            }
        );
    }

    // ---- workspace flags without --workspace are usage errors, never no-ops ----

    #[test]
    fn base_without_workspace_is_usage_error() {
        let mut r: Raw = raw();
        r.base = Some("main".to_owned());
        assert!(resolve(r).is_err());
    }

    #[test]
    fn diff_dir_without_workspace_is_usage_error() {
        let mut r: Raw = raw();
        r.diff_dir = Some("/diffs".to_owned());
        assert!(resolve(r).is_err());
    }

    #[test]
    fn changed_only_without_workspace_is_usage_error() {
        let mut r: Raw = raw();
        r.changed_only = true;
        assert!(resolve(r).is_err());
    }

    // ---- single-crate flags with --workspace are usage errors, never no-ops ----

    #[test]
    fn regime_with_workspace_is_usage_error() {
        let mut r: Raw = raw();
        r.workspace = true;
        r.regime_path = Some("regime-transition.toml".to_owned());
        assert!(resolve(r).is_err());
    }

    #[test]
    fn diff_with_workspace_is_usage_error() {
        let mut r: Raw = raw();
        r.workspace = true;
        r.diff_path = Some("some.diff".to_owned());
        assert!(resolve(r).is_err());
    }

    // ---- failure constructors carry the mandated content ----

    // the without-workspace usage error names the offending flag.
    #[test]
    fn workspace_flag_without_workspace_names_the_flag() {
        let failure: Failure = workspace_flag_without_workspace("--base");
        assert!(failure.message.contains("--base"));
    }

    // the whole-run adapter preserves the workspace failure's message and hint.
    #[test]
    fn workspace_failure_preserves_message_and_hint() {
        let source: workspace::Failure = workspace::Failure {
            message: "boom".to_owned(),
            hint: "do x".to_owned(),
        };
        let failure: Failure = workspace_failure(&source);
        assert_eq!(failure.message, "boom");
        assert_eq!(failure.hint, "do x");
    }

    // ---- capabilities advertises the workspace contract additively ----

    #[test]
    fn capabilities_includes_the_workspace_contract() {
        let value: serde_json::Value =
            serde_json::from_str(&capabilities()).expect("capabilities is valid JSON");
        assert_eq!(value["workspace"]["flag"], "--workspace");
        assert!(value["workspace"]["counts"].is_array());
        assert_eq!(value["workspace"]["verdict_exit_codes"]["error"], 2);
    }

    // ---- existing single-crate capabilities keys remain (additive change) ----

    #[test]
    fn capabilities_keeps_the_single_crate_keys() {
        let value: serde_json::Value =
            serde_json::from_str(&capabilities()).expect("capabilities is valid JSON");
        assert_eq!(
            value["exit_codes"]["1"],
            "undeclared/contradictory residual"
        );
        assert!(value["report_schema"]["items"].is_array());
    }
}
