//! Shared helpers for the CLI integration tests: spawn the real built binary and
//! resolve fixture/example paths.
//!
//! Each integration-test crate compiles this module independently and uses only
//! a subset of the helpers, so unused-warnings here are expected.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Absolute path to a file under `tests/fixtures/`.
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

/// Result of a CLI run: exit code, captured stdout, and captured stderr.
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run the built binary with the given args, returning exit code, stdout, stderr.
#[must_use]
pub fn run(args: &[&str]) -> Run {
    let out: Output = Command::new(env!("CARGO_BIN_EXE_cargo-regime-check"))
        .args(args)
        .output()
        .expect("spawn cargo-regime-check");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).expect("stdout is utf-8"),
        stderr: String::from_utf8(out.stderr).expect("stderr is utf-8"),
    }
}

/// Run the gate against a regime + diff, in JSON mode.
#[must_use]
pub fn gate_json(regime: &str, diff: &str) -> Run {
    run(&["--regime", regime, "--diff", diff, "--format", "json"])
}

/// Concatenate every non-null `remediation` snippet from a JSON report, in order.
#[must_use]
pub fn remediations(report_json: &str) -> String {
    let report: serde_json::Value =
        serde_json::from_str(report_json).expect("report is valid JSON");
    report["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .filter_map(|item: &serde_json::Value| item["remediation"].as_str())
        .collect::<Vec<&str>>()
        .join("\n")
}
