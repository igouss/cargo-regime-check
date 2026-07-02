//! The load-bearing acceptance test: a directive is only real if an agent can
//! act on it. From a FAILing run, append the tool's own emitted `remediation`
//! verbatim to the regime file, re-run, and assert it now exits 0. Anything less
//! is a tool that prints plausible instructions an agent cannot actually apply.

mod common;

use std::path::{Path, PathBuf};

use common::{
    copy_fixture_tree, fixture, gate_json, remediations, remediations_of, run, run_in, scratch, Run,
};

/// Run the gate (FAIL expected), append every emitted remediation to the base
/// regime, re-run, and return the second run's exit code.
fn apply_and_rerun(base_regime: &str, diff: &str, scratch_name: &str) -> i32 {
    let first = gate_json(base_regime, diff);
    assert_eq!(first.code, 1, "base regime must FAIL before remediation");

    let base_text: String = std::fs::read_to_string(base_regime).expect("read base regime");
    let fixed_text: String = format!("{base_text}\n{}", remediations(&first.stdout));
    let fixed_path = scratch(scratch_name);
    std::fs::write(&fixed_path, fixed_text).expect("write fixed regime");

    run(&["--regime", fixed_path.to_str().unwrap(), "--diff", diff]).code
}

// ---- one: the synthetic demo diff (add + remove + change residual) ----
#[test]
fn demo_remediation_makes_the_gate_pass() {
    let code: i32 = apply_and_rerun(
        &fixture("demo-base.toml"),
        &fixture("demo.diff"),
        "demo-fixed.toml",
    );
    assert_eq!(
        code, 0,
        "applying remediation verbatim must make the gate PASS"
    );
}

// ---- many: a real captured ISP split (renames + new traits/impls/removal/change) ----
#[test]
fn real_kvstore_remediation_makes_the_gate_pass() {
    let code: i32 = apply_and_rerun(
        &fixture("kvstore-base.toml"),
        &fixture("kvstore-isp-split.diff"),
        "kvstore-fixed.toml",
    );
    assert_eq!(
        code, 0,
        "applying remediation verbatim must make the real-crate gate PASS"
    );
}

// ---- workspace path -------------------------------------------------------
//
// The same contract, one level up: from a FAILing `--workspace --diff-dir` run,
// append EACH residual crate's emitted `remediation` verbatim to that crate's own
// `regime-transition.toml`, re-run, and assert the whole workspace now exits 0.
// This proves the aggregated JSON's per-crate remediation is real — actionable by
// an agent with no human in the loop — not merely plausible-looking text.

/// Append `snippets` (a crate's verbatim emitted remediation) to
/// `<ws>/<crate_name>/regime-transition.toml`, separated by a newline — the
/// per-crate analogue of the single-crate append above.
fn append_crate_remediation(ws: &Path, crate_name: &str, snippets: &str) {
    let regime: PathBuf = ws.join(crate_name).join("regime-transition.toml");
    let base: String = std::fs::read_to_string(&regime).expect("read crate regime");
    let fixed: String = format!("{base}\n{snippets}");
    std::fs::write(&regime, fixed).expect("write remediated crate regime");
}

/// Append every RESIDUAL crate's emitted remediation snippets verbatim to that
/// crate's regime file. The iteration over the aggregated report lives here (a
/// helper), keeping the test body at cyclomatic complexity 1.
fn apply_residual_remediations(ws: &Path, workspace_report: &serde_json::Value) {
    workspace_report["crates"]
        .as_array()
        .expect("crates is an array")
        .iter()
        .filter(|entry: &&serde_json::Value| entry["status"] == "residual")
        .for_each(|entry: &serde_json::Value| {
            let name: &str = entry["name"].as_str().expect("crate name is a string");
            append_crate_remediation(ws, name, &remediations_of(&entry["report"]));
        });
}

/// Run `--workspace --diff-dir` (FAIL expected), append every residual crate's
/// emitted remediation verbatim to that crate's regime file, then re-run and
/// return the second run's exit code.
fn apply_workspace_remediation_and_rerun(ws: &Path, diff_dir: &str) -> i32 {
    let first: Run = run_in(
        ws,
        &["--workspace", "--diff-dir", diff_dir, "--format", "json"],
    );
    assert_eq!(first.code, 1, "workspace must FAIL before remediation");

    let report: serde_json::Value =
        serde_json::from_str(&first.stdout).expect("workspace stdout is valid JSON");
    apply_residual_remediations(ws, &report);

    run_in(ws, &["--workspace", "--diff-dir", diff_dir]).code
}

// ---- one: the fixture workspace's single residual crate (crate_b) ----
#[test]
fn workspace_remediation_makes_every_residual_crate_pass() {
    let ws: PathBuf = copy_fixture_tree("workspace", "agent-loop-ws");
    let code: i32 =
        apply_workspace_remediation_and_rerun(ws.as_path(), &fixture("workspace-diffs"));
    assert_eq!(
        code, 0,
        "appending each residual crate's remediation verbatim must make the workspace PASS"
    );
}
