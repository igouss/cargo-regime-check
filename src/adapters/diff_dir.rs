//! Adapter: the alternate diff source — read a crate's pre-captured diff from a
//! directory of `<crate-name>.diff` files (`--diff-dir`).
//!
//! This mode exists so workspace gating can run without the in-tree checkout
//! `cargo public-api`'s `base..HEAD` form performs (which would clobber a dirty
//! tree). The caller supplies a directory; each gated crate's diff is the file
//! `<dir>/<crate_name>.diff`, whose contents are exactly the text
//! [`crate::adapters::public_api_diff::parse`] consumes.
//!
//! Hexagonal split (the reason this file has two layers):
//! - [`diff_path`] is a **pure** function `(dir, crate_name) → path`. It builds
//!   the `<dir>/<crate_name>.diff` location without touching the filesystem, so
//!   path construction is unit-tested directly.
//! - [`read`] is the **filesystem** boundary: it reads that path and maps a
//!   *missing* file to a distinct [`DiffDirError::Missing`] — a gated crate with
//!   no diff is an error (contributing to a code-2 run), NEVER a silent skip,
//!   which would let the gate pass green having checked nothing for that crate.
//!
//! The pure domain (`classify`/`gate`/`identity`/`diff`/`transition`) never
//! learns this exists; dependencies point inward.

use std::path::{Path, PathBuf};

/// The `.diff` extension every per-crate diff file in a `--diff-dir` carries.
const DIFF_EXTENSION: &str = "diff";

/// Failure modes of reading a crate's diff file. The pure [`diff_path`] seam
/// never fails. [`DiffDirError::Missing`] is deliberately distinct from the
/// catch-all [`DiffDirError::Io`]: a gated crate with no diff file is a hard
/// error the orchestrator records as an *errored* crate (exit 2), so it must be
/// distinguishable from a permission or encoding failure — and never collapse
/// into a silent skip.
#[derive(Debug, thiserror::Error)]
pub enum DiffDirError {
    #[error(
        "no diff file for gated crate `{crate_name}` at {path}: a gated crate with no diff is an error, not a skip (generate `{crate_name}.diff` or drop the crate's regime-transition.toml)"
    )]
    Missing { crate_name: String, path: String },
    #[error("could not read diff file {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// The path a crate's captured diff lives at: `<dir>/<crate_name>.diff`.
/// **Pure**: no filesystem access — the location half of [`read`], testable in
/// isolation. Uses [`Path::join`] + [`PathBuf::set_extension`] so an unusual
/// crate name is placed as a single path component (never split on `/`).
#[must_use]
pub fn diff_path(dir: &Path, crate_name: &str) -> PathBuf {
    let mut path: PathBuf = dir.join(crate_name);
    path.set_extension(DIFF_EXTENSION);
    path
}

/// Read the captured diff for `crate_name` from `dir`, returning its text. The
/// **filesystem** boundary: it resolves the location via [`diff_path`] and reads
/// it, mapping a *missing* file to [`DiffDirError::Missing`] (a distinct, named
/// error a gated crate with no diff must surface as) and any other read failure
/// to [`DiffDirError::Io`]. A present file's contents are returned verbatim.
pub fn read(dir: &Path, crate_name: &str) -> Result<String, DiffDirError> {
    let path: PathBuf = diff_path(dir, crate_name);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(DiffDirError::Missing {
                crate_name: crate_name.to_owned(),
                path: path.display().to_string(),
            })
        }
        Err(source) => Err(DiffDirError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- diff_path: `<dir>/<crate_name>.diff` construction ----

    // one: a plain crate name becomes a single `.diff` component under the dir.
    #[test]
    fn diff_path_appends_crate_name_dot_diff() {
        assert_eq!(
            diff_path(Path::new("/diffs"), "alpha"),
            PathBuf::from("/diffs/alpha.diff")
        );
    }

    // a hyphenated crate name is placed as one component, extension appended.
    #[test]
    fn diff_path_keeps_hyphenated_name_as_one_component() {
        assert_eq!(
            diff_path(Path::new("/tmp/d"), "my-port-crate"),
            PathBuf::from("/tmp/d/my-port-crate.diff")
        );
    }

    // ---- read: the missing-file error variant is distinct and named ----

    // a crate with no diff file under a nonexistent dir is `Missing`, never
    // `Io` and never a silent empty read.
    #[test]
    fn missing_diff_file_is_a_named_missing_error() {
        let err: DiffDirError =
            read(Path::new("/no/such/regime-check-diff-dir"), "absent-crate").unwrap_err();
        assert!(matches!(err, DiffDirError::Missing { .. }));
    }

    // the Missing error names the crate whose diff is absent (so the operator
    // knows which `<crate>.diff` to generate).
    #[test]
    fn missing_error_names_the_crate() {
        let err: DiffDirError =
            read(Path::new("/no/such/regime-check-diff-dir"), "absent-crate").unwrap_err();
        assert!(
            err.to_string().contains("absent-crate"),
            "message should name the crate: {err}"
        );
    }

    // a present diff file is read back verbatim (the happy path).
    #[test]
    fn present_diff_file_is_read_verbatim() {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("regime-check-diff-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let contents: &str = "Added items to the public API\n=====\n+pub fn demo::x()\n";
        std::fs::write(diff_path(&dir, "gamma"), contents).expect("write diff file");

        let text: String = read(&dir, "gamma").expect("read present diff file");

        assert_eq!(text, contents);
    }
}
