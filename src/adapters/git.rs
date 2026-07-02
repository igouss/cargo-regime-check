//! Adapter: the two git queries workspace mode needs, each split so its pure
//! core is unit-testable without a real repository.
//!
//! 1. **Dirtiness** — default (public-api *process*) mode refuses to run against
//!    a dirty working tree, because `cargo public-api`'s `<base>..HEAD` form
//!    git-checks-out each commit **in-tree** to build rustdoc JSON, which would
//!    clobber uncommitted changes. [`is_dirty`] runs `git status --porcelain`;
//!    [`porcelain_dirty`] is the pure text -> bool it delegates to.
//! 2. **Changed files** — `--changed-only` skips crates nothing touched.
//!    [`changed_files`] runs `git diff --name-only <base>..HEAD`;
//!    [`parse_name_only`] is the pure text -> paths split, and [`crates_touched`]
//!    is the pure mapping from changed paths to the set of crate names touched.
//!
//! Hexagonal split (the reason this file has two layers): every process
//! invocation ([`is_dirty`], [`changed_files`]) is a thin shell over a pure
//! function ([`porcelain_dirty`], [`parse_name_only`] / [`crates_touched`]), so
//! all the decision logic is tested without spawning git. The pure domain
//! (`classify`/`gate`/`identity`/`diff`/`transition`) never learns git exists;
//! dependencies point inward.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Failure modes of the git queries. The pure helpers never fail; only the
/// process boundary can — a missing repository, a bad `base` ref, or non-UTF-8
/// output. Surfacing these as errors (never a silent clean/empty result) keeps
/// the gate from a false green.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run `git {args}` in {dir}: {source}")]
    Spawn {
        dir: String,
        args: String,
        source: std::io::Error,
    },
    #[error("`git {args}` failed (exit {code}) in {dir}: {stderr}")]
    Git {
        dir: String,
        args: String,
        code: String,
        stderr: String,
    },
    #[error("`git {args}` output was not valid UTF-8: {source}")]
    Utf8 {
        args: String,
        source: std::string::FromUtf8Error,
    },
}

/// Run `git <args>` with `dir` as the working directory and return its stdout.
/// The single **process** boundary for this adapter; both public queries funnel
/// through it so spawn / non-zero-exit / non-UTF-8 handling lives in one place.
fn run_git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let output: Output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|source: std::io::Error| GitError::Spawn {
            dir: dir.display().to_string(),
            args: args.join(" "),
            source,
        })?;

    if !output.status.success() {
        let code: String = output
            .status
            .code()
            .map(|c: i32| c.to_string())
            .unwrap_or_else(|| "signal".to_owned());
        let stderr: String = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(GitError::Git {
            dir: dir.display().to_string(),
            args: args.join(" "),
            code,
            stderr,
        });
    }

    let text: String =
        String::from_utf8(output.stdout).map_err(|source: std::string::FromUtf8Error| {
            GitError::Utf8 {
                args: args.join(" "),
                source,
            }
        })?;
    Ok(text)
}

/// Whether `git status --porcelain` output denotes a dirty working tree: any
/// non-blank line is a pending change (staged, unstaged, or untracked), so a
/// clean tree's empty output is the only not-dirty case. **Pure** — the
/// text -> bool half of [`is_dirty`], testable without a repository.
#[must_use]
pub fn porcelain_dirty(status_text: &str) -> bool {
    status_text
        .lines()
        .any(|line: &str| !line.trim().is_empty())
}

/// Whether the working tree at `dir` is dirty (has staged, unstaged, or
/// untracked changes). Runs `git status --porcelain` and delegates the verdict
/// to [`porcelain_dirty`]. Default (process) mode calls this to refuse to run
/// against a dirty tree, which `cargo public-api`'s in-tree checkout would clobber.
pub fn is_dirty(dir: &Path) -> Result<bool, GitError> {
    let status_text: String = run_git(dir, &["status", "--porcelain"])?;
    Ok(porcelain_dirty(&status_text))
}

/// Split `git diff --name-only` output into paths (one per non-blank line),
/// relative to the repository root exactly as git reports them. **Pure** — the
/// text -> paths half of [`changed_files`].
#[must_use]
pub fn parse_name_only(name_only_text: &str) -> Vec<PathBuf> {
    name_only_text
        .lines()
        .filter(|line: &&str| !line.trim().is_empty())
        .map(|line: &str| PathBuf::from(line))
        .collect()
}

/// The files changed between `base` and `HEAD` in the repository at `dir`,
/// relative to the repository root. Runs `git diff --name-only <base>..HEAD` and
/// delegates the split to [`parse_name_only`]. `--changed-only` feeds these into
/// [`crates_touched`] to skip crates nothing touched.
pub fn changed_files(dir: &Path, base: &str) -> Result<Vec<PathBuf>, GitError> {
    let range: String = format!("{base}..HEAD");
    let name_only_text: String = run_git(dir, &["diff", "--name-only", &range])?;
    Ok(parse_name_only(&name_only_text))
}

/// A crate's identity for change-detection: its `name` and its `root` **relative
/// to the workspace root**, in the same frame as the paths `git diff
/// --name-only` reports. Kept local (rather than reusing
/// [`super::cargo_metadata::WorkspaceMember`], whose root is absolute) so the
/// pure touch-mapping is independent of how members were enumerated and testable
/// with plain relative paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateRoot {
    pub name: String,
    pub root: PathBuf,
}

/// The set of crate names touched by `changed_paths`: a crate is touched iff any
/// changed path lies under its root. The match is **component-wise** (via
/// [`Path::starts_with`]), so root `alpha` matches `alpha/src/lib.rs` but never
/// the sibling crate `alpha-utils/...`. Both sides are relative to the workspace
/// root. **Pure**; returns a [`BTreeSet`] so the result is name-sorted and
/// deterministic. `--changed-only` skips every gated crate NOT in this set.
#[must_use]
pub fn crates_touched(changed_paths: &[PathBuf], members: &[CrateRoot]) -> BTreeSet<String> {
    members
        .iter()
        .filter(|member: &&CrateRoot| {
            changed_paths
                .iter()
                .any(|path: &PathBuf| path.starts_with(&member.root))
        })
        .map(|member: &CrateRoot| member.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- porcelain_dirty: zero / one / many pending changes ----

    // zero: a clean tree emits nothing, so it is not dirty.
    #[test]
    fn clean_tree_is_not_dirty() {
        assert!(!porcelain_dirty(""));
    }

    // one: a single modified-file status line is dirty.
    #[test]
    fn one_pending_change_is_dirty() {
        assert!(porcelain_dirty(" M src/lib.rs\n"));
    }

    // many: several status lines (modified + untracked) are dirty.
    #[test]
    fn many_pending_changes_are_dirty() {
        assert!(porcelain_dirty(" M src/lib.rs\n?? extra.txt\n"));
    }

    // guard: blank-only output (stray newlines) stays clean, never a false dirty.
    #[test]
    fn blank_lines_stay_clean() {
        assert!(!porcelain_dirty("\n\n"));
    }

    // ---- parse_name_only: zero / one / many changed paths ----

    // zero: empty diff output yields no paths.
    #[test]
    fn zero_changed_paths() {
        assert!(parse_name_only("").is_empty());
    }

    // one: a single line yields one path, relative to the repo root.
    #[test]
    fn one_changed_path() {
        assert_eq!(
            parse_name_only("alpha/src/lib.rs\n"),
            vec![PathBuf::from("alpha/src/lib.rs")]
        );
    }

    // many: several lines yield several paths; blank lines are skipped.
    #[test]
    fn many_changed_paths_skipping_blanks() {
        assert_eq!(
            parse_name_only("alpha/src/lib.rs\n\ncrates/beta/Cargo.toml\n"),
            vec![
                PathBuf::from("alpha/src/lib.rs"),
                PathBuf::from("crates/beta/Cargo.toml"),
            ]
        );
    }

    // ---- crates_touched: zero / one / many touched crates ----

    fn member(name: &str, root: &str) -> CrateRoot {
        CrateRoot {
            name: name.to_owned(),
            root: PathBuf::from(root),
        }
    }

    // zero: no changed paths touches no crate.
    #[test]
    fn zero_changes_touch_nothing() {
        let members: Vec<CrateRoot> = vec![member("alpha", "alpha")];
        let changed: Vec<PathBuf> = vec![];
        let touched: BTreeSet<String> = crates_touched(&changed, &members);
        assert!(touched.is_empty());
    }

    // one: a change under one crate's root touches exactly that crate.
    #[test]
    fn one_change_under_a_root_touches_that_crate() {
        let members: Vec<CrateRoot> = vec![member("alpha", "alpha"), member("beta", "crates/beta")];
        let changed: Vec<PathBuf> = vec![PathBuf::from("alpha/src/lib.rs")];
        let touched: BTreeSet<String> = crates_touched(&changed, &members);
        assert_eq!(touched, BTreeSet::from(["alpha".to_owned()]));
    }

    // many: changes under two of three crates touch exactly those two.
    #[test]
    fn many_changes_touch_their_crates() {
        let members: Vec<CrateRoot> = vec![
            member("alpha", "alpha"),
            member("beta", "crates/beta"),
            member("gamma", "crates/gamma"),
        ];
        let changed: Vec<PathBuf> = vec![
            PathBuf::from("crates/gamma/src/x.rs"),
            PathBuf::from("alpha/Cargo.toml"),
        ];
        let touched: BTreeSet<String> = crates_touched(&changed, &members);
        assert_eq!(
            touched,
            BTreeSet::from(["alpha".to_owned(), "gamma".to_owned()])
        );
    }

    // guard: a sibling whose name is a string-prefix (`alpha` vs `alpha-utils`)
    // is NOT touched — the match is component-wise, not substring.
    #[test]
    fn string_prefix_sibling_is_not_touched() {
        let members: Vec<CrateRoot> = vec![
            member("alpha", "alpha"),
            member("alpha-utils", "alpha-utils"),
        ];
        let changed: Vec<PathBuf> = vec![PathBuf::from("alpha-utils/src/lib.rs")];
        let touched: BTreeSet<String> = crates_touched(&changed, &members);
        assert_eq!(touched, BTreeSet::from(["alpha-utils".to_owned()]));
    }
}
