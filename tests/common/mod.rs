//! Shared helpers for the CLI integration tests: spawn the real built binary and
//! resolve fixture/example paths.
//!
//! Each integration-test crate compiles this module independently and uses only
//! a subset of the helpers, so unused-warnings here are expected.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Absolute path to a file (or directory) under `tests/fixtures/`.
#[must_use]
pub fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Absolute path to a file under `examples/`.
#[must_use]
pub fn example(name: &str) -> String {
    format!("{}/examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// A per-test scratch path inside cargo's integration-test tmp dir.
#[must_use]
pub fn scratch(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

/// A fresh, empty scratch directory: any prior copy is removed so re-runs start
/// clean. Used to seed a `--diff-dir` or to receive a copied fixture tree.
#[must_use]
pub fn fresh_dir(name: &str) -> PathBuf {
    let dir: PathBuf = scratch(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Result of a CLI run: exit code, captured stdout, and captured stderr.
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run the built binary with `args`, optionally with `dir` as the working
/// directory, returning exit code, stdout, and stderr. The single spawn seam both
/// [`run`] and [`run_in`] funnel through.
#[must_use]
fn run_impl(dir: Option<&Path>, args: &[&str]) -> Run {
    let mut command: Command = Command::new(env!("CARGO_BIN_EXE_cargo-regime-check"));
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let out: Output = command.output().expect("spawn cargo-regime-check");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).expect("stdout is utf-8"),
        stderr: String::from_utf8(out.stderr).expect("stderr is utf-8"),
    }
}

/// Run the built binary with the given args, returning exit code, stdout, stderr.
#[must_use]
pub fn run(args: &[&str]) -> Run {
    run_impl(None, args)
}

/// Run the built binary with `dir` as the working directory — `--workspace` mode
/// is rooted at the cwd, so a per-test scratch workspace is exercised by pointing
/// the binary at it here.
#[must_use]
pub fn run_in(dir: &Path, args: &[&str]) -> Run {
    run_impl(Some(dir), args)
}

/// Recursively copy the directory tree at `src` into `dst` (created if absent).
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry: std::fs::DirEntry = entry.expect("read dir entry");
        let from: PathBuf = entry.path();
        let to: PathBuf = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy file");
        }
    }
}

/// Copy the fixture directory `tests/fixtures/<fixture_rel>` into a fresh scratch
/// directory and return that scratch path — the workspace root the CLI runs
/// against. Copying keeps the run off the read-only in-repo fixture (so a stray
/// `Cargo.lock` from `cargo metadata` never lands in the source tree) and gives
/// each test its own mutable, git-init-able workspace.
#[must_use]
pub fn copy_fixture_tree(fixture_rel: &str, scratch_name: &str) -> PathBuf {
    let src: PathBuf = PathBuf::from(fixture(fixture_rel));
    let dst: PathBuf = fresh_dir(scratch_name);
    copy_tree(&src, &dst);
    dst
}

/// Run `git <args>` in `dir`, asserting success. The one git spawn seam the
/// scratch-repo helpers funnel through.
fn git_run(dir: &Path, args: &[&str]) {
    let status: std::process::ExitStatus = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Initialize a git repository at `dir` and commit its whole tree as the initial
/// state. Identity and signing are set locally so the commit succeeds regardless
/// of the host's global git config.
pub fn git_init_commit(dir: &Path) {
    git_run(dir, &["init", "-q"]);
    git_run(dir, &["config", "user.email", "test@regime.invalid"]);
    git_run(dir, &["config", "user.name", "Regime Test"]);
    git_run(dir, &["config", "commit.gpgsign", "false"]);
    git_run(dir, &["add", "-A"]);
    git_run(dir, &["commit", "-q", "-m", "workspace fixture"]);
}

/// Stage and commit the whole tree at `dir` with `message` — a follow-up commit on
/// a repo already initialized by [`git_init_commit`].
pub fn git_commit_all(dir: &Path, message: &str) {
    git_run(dir, &["add", "-A"]);
    git_run(dir, &["commit", "-q", "-m", message]);
}

/// Run the gate against a regime + diff, in JSON mode.
#[must_use]
pub fn gate_json(regime: &str, diff: &str) -> Run {
    run(&["--regime", regime, "--diff", diff, "--format", "json"])
}

/// Concatenate every non-null `remediation` snippet from a parsed single-crate
/// report value, in order. The workspace path feeds this the embedded per-crate
/// `report` object; the single-crate path parses a string first ([`remediations`]).
#[must_use]
pub fn remediations_of(report: &serde_json::Value) -> String {
    report["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .filter_map(|item: &serde_json::Value| item["remediation"].as_str())
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Concatenate every non-null `remediation` snippet from a JSON report string, in
/// order — the string-front wrapper over [`remediations_of`].
#[must_use]
pub fn remediations(report_json: &str) -> String {
    let report: serde_json::Value =
        serde_json::from_str(report_json).expect("report is valid JSON");
    remediations_of(&report)
}
