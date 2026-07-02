//! Adapter: the default diff source — shell out to `cargo public-api` on
//! **nightly** to produce a crate's `base..HEAD` public-API diff text.
//!
//! `cargo public-api` needs nightly to emit rustdoc JSON, so this crate never
//! calls it in-process; it invokes it as a *separate* process:
//!
//! ```text
//! cargo +nightly public-api -p <crate> diff <base>..HEAD
//! ```
//!
//! with the workspace directory as the working directory. Its stdout is exactly
//! the textual diff [`crate::adapters::public_api_diff::parse`] consumes, so this
//! adapter's only job is process construction, invocation, and failure mapping.
//!
//! Hexagonal split (the reason this file has two layers):
//! - [`args`] is a **pure** function `(crate_name, base) → argv`. It builds the
//!   exact command line without running anything, so command construction is
//!   unit-tested in CI even though the real nightly toolchain is not present.
//! - [`diff`] is the **process** boundary: it spawns `cargo` with those args and
//!   maps a spawn failure or non-zero exit to a distinct error carrying stderr.
//!
//! The program is the literal `cargo` (the rustup shim), never the `CARGO` env
//! var cargo sets for its subcommands: the leading `+nightly` toolchain selector
//! is a rustup feature, and only the shim on `PATH` honours it. The pure domain
//! (`classify`/`gate`/`identity`/`diff`/`transition`) never learns this exists;
//! dependencies point inward.

use std::path::Path;
use std::process::{Command, Output};

/// The rustup `cargo` shim. Spelled out (rather than the `CARGO` env var the
/// [`super::cargo_metadata`] adapter honours) because the leading `+nightly`
/// toolchain override in [`args`] is a rustup feature: only the shim on `PATH`
/// interprets it, whereas `CARGO` points at a concrete toolchain's cargo that
/// would treat `+nightly` as an unknown argument.
const CARGO_SHIM: &str = "cargo";

/// Failure modes of the `cargo public-api` invocation. The pure [`args`] seam
/// never fails; only the process boundary can — the toolchain/subcommand is
/// missing (spawn), the tool reports an error (non-zero exit), or its stdout is
/// not UTF-8. Each surfaces as an error carrying its context (stderr for a
/// non-zero exit) so a gated crate is never silently treated as clean.
#[derive(Debug, thiserror::Error)]
pub enum PublicApiError {
    #[error("could not run `cargo {args}` in {dir}: {source} (is `cargo public-api` installed and is a nightly toolchain available?)")]
    Spawn {
        dir: String,
        args: String,
        source: std::io::Error,
    },
    #[error("`cargo {args}` failed (exit {code}) in {dir}: {stderr}")]
    Process {
        dir: String,
        args: String,
        code: String,
        stderr: String,
    },
    #[error("`cargo {args}` output was not valid UTF-8: {source}")]
    Utf8 {
        args: String,
        source: std::string::FromUtf8Error,
    },
}

/// The argument vector for `cargo +nightly public-api -p <crate_name> diff
/// <base>..HEAD`. **Pure**: it constructs the command line without running it,
/// which is the whole point of the split — the exact invocation is asserted in
/// CI even though CI has no nightly toolchain to execute it against. The leading
/// `+nightly` selects the toolchain (see [`CARGO_SHIM`]); the range is the
/// `base..HEAD` form that makes `cargo public-api` diff `base` against `HEAD`.
#[must_use]
pub fn args(crate_name: &str, base: &str) -> Vec<String> {
    vec![
        "+nightly".to_owned(),
        "public-api".to_owned(),
        "-p".to_owned(),
        crate_name.to_owned(),
        "diff".to_owned(),
        format!("{base}..HEAD"),
    ]
}

/// Run `cargo public-api` for `crate_name` in the workspace at `dir`, diffing
/// `base..HEAD`, and return its stdout (the diff text). The **process** boundary:
/// it builds the command line via [`args`], spawns [`CARGO_SHIM`], and maps a
/// spawn failure or non-zero exit to a [`PublicApiError`] carrying stderr —
/// never a silent empty diff, which would read as a false-clean crate.
pub fn diff(dir: &Path, crate_name: &str, base: &str) -> Result<String, PublicApiError> {
    let argv: Vec<String> = args(crate_name, base);
    let pretty: String = argv.join(" ");

    let output: Output = Command::new(CARGO_SHIM)
        .args(&argv)
        .current_dir(dir)
        .output()
        .map_err(|source: std::io::Error| PublicApiError::Spawn {
            dir: dir.display().to_string(),
            args: pretty.clone(),
            source,
        })?;

    if !output.status.success() {
        let code: String = output
            .status
            .code()
            .map(|c: i32| c.to_string())
            .unwrap_or_else(|| "signal".to_owned());
        let stderr: String = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(PublicApiError::Process {
            dir: dir.display().to_string(),
            args: pretty,
            code,
            stderr,
        });
    }

    let text: String =
        String::from_utf8(output.stdout).map_err(|source: std::string::FromUtf8Error| {
            PublicApiError::Utf8 {
                args: pretty,
                source,
            }
        })?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- args: the exact `cargo +nightly public-api diff` invocation ----

    // one: the canonical invocation, asserted whole so a drift in order, flags,
    // or the `base..HEAD` range shape is caught.
    #[test]
    fn args_are_the_nightly_public_api_diff_invocation() {
        assert_eq!(
            args("my-crate", "origin/main"),
            vec![
                "+nightly".to_owned(),
                "public-api".to_owned(),
                "-p".to_owned(),
                "my-crate".to_owned(),
                "diff".to_owned(),
                "origin/main..HEAD".to_owned(),
            ]
        );
    }

    // the crate name is placed verbatim right after `-p`, not hardcoded.
    #[test]
    fn args_place_crate_name_after_dash_p() {
        let argv: Vec<String> = args("alpha", "HEAD~1");
        assert_eq!(argv[2..4], ["-p".to_owned(), "alpha".to_owned()]);
    }

    // the diff range is exactly `<base>..HEAD` for the given base ref.
    #[test]
    fn args_embed_the_base_head_range() {
        let argv: Vec<String> = args("beta", "v1.2.3");
        assert_eq!(argv.last(), Some(&"v1.2.3..HEAD".to_owned()));
    }
}
