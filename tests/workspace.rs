//! `--workspace` acceptance tests: gate every gated member of a fixture workspace
//! in one real invocation of the built binary, and assert the aggregate exit code,
//! the aggregated JSON view-model, and the mandated failure guidance.
//!
//! The fixture workspace (`tests/fixtures/workspace/`) is a virtual `[workspace]`
//! with three members: `crate_a` (a CLEAN refactor), `crate_b` (an UNDECLARED
//! residual — FAIL), and `crate_c` (NO regime file — never gated). The captured
//! diffs live in `tests/fixtures/workspace-diffs/`. Each test copies the fixture
//! into its own scratch workspace so the runs are isolated and never mutate the
//! source tree. Crates are name-sorted in the report, so `crate_a`/`crate_b`/
//! `crate_c` are always at indices 0/1/2.

mod common;

use common::{
    copy_fixture_tree, fixture, fresh_dir, git_commit_all, git_init_commit, run, run_in, Run,
};

/// Parse a run's stdout as JSON (the `--format json` view-model).
fn json_of(run: &Run) -> serde_json::Value {
    serde_json::from_str(&run.stdout).expect("stdout is valid JSON")
}

// ---- (1) --diff-dir gates A and B, ignores C: residual -> exit 1, counts add up.
#[test]
fn diff_dir_gates_a_and_b_ignores_c_with_residual_exit_1() {
    let ws: std::path::PathBuf = copy_fixture_tree("workspace", "ws-diff-dir");

    let r: Run = run_in(
        &ws,
        &[
            "--workspace",
            "--diff-dir",
            &fixture("workspace-diffs"),
            "--format",
            "json",
        ],
    );

    assert_eq!(r.code, 1);
    let value: serde_json::Value = json_of(&r);
    assert_eq!(
        value["counts"],
        serde_json::json!({
            "crates": 2,
            "clean": 1,
            "residual": 1,
            "errored": 0,
            "skipped": 0
        })
    );
    assert_eq!(value["crates"][0]["name"], "crate_a");
    assert_eq!(value["crates"][1]["name"], "crate_b");
    assert!(value["crates"][2].is_null());
}

// ---- (2) crate_b's embedded report equals a standalone single-crate gate of the
//          same diff + regime: aggregation ADDS, never rewrites.
#[test]
fn crate_b_embedded_report_equals_standalone_single_crate_run() {
    let ws: std::path::PathBuf = copy_fixture_tree("workspace", "ws-embed");

    let workspace: serde_json::Value = json_of(&run_in(
        &ws,
        &[
            "--workspace",
            "--diff-dir",
            &fixture("workspace-diffs"),
            "--format",
            "json",
        ],
    ));
    let regime: std::path::PathBuf = ws.join("crate_b").join("regime-transition.toml");
    let standalone: serde_json::Value = json_of(&run(&[
        "--regime",
        regime.to_str().expect("regime path is utf-8"),
        "--diff",
        &fixture("workspace-diffs/crate_b.diff"),
        "--format",
        "json",
    ]));

    assert_eq!(workspace["crates"][1]["name"], "crate_b");
    assert_eq!(workspace["crates"][1]["report"], standalone);
}

// ---- (3) a gated crate whose diff file is missing is ERRORED (never a silent
//          skip); the sibling still gates clean, and the whole run exits 2.
#[test]
fn missing_crate_b_diff_is_errored_and_run_exits_2() {
    let ws: std::path::PathBuf = copy_fixture_tree("workspace", "ws-missing-b");
    let diffs: std::path::PathBuf = fresh_dir("ws-missing-b-diffs");
    std::fs::copy(
        fixture("workspace-diffs/crate_a.diff"),
        diffs.join("crate_a.diff"),
    )
    .expect("seed crate_a.diff only (crate_b.diff deliberately absent)");

    let r: Run = run_in(
        &ws,
        &[
            "--workspace",
            "--diff-dir",
            diffs.to_str().expect("diff-dir path is utf-8"),
            "--format",
            "json",
        ],
    );

    assert_eq!(r.code, 2);
    let value: serde_json::Value = json_of(&r);
    assert_eq!(value["crates"][0]["status"], "clean");
    assert_eq!(value["crates"][1]["status"], "errored");
    assert!(value["crates"][1]["error"]
        .as_str()
        .expect("errored entry carries an error message")
        .contains("crate_b"));
}

// ---- (4) default (process) mode on a DIRTY tree exits 2 BEFORE any public-api
//          call, and the message names the in-tree-checkout caveat AND the git
//          worktree escape hatch.
#[test]
fn dirty_tree_in_process_mode_exits_2_naming_caveat_and_worktree() {
    let ws: std::path::PathBuf = copy_fixture_tree("workspace", "ws-dirty");
    git_init_commit(&ws);
    std::fs::write(ws.join("UNCOMMITTED.txt"), "dirty\n").expect("dirty the working tree");

    let r: Run = run_in(&ws, &["--workspace", "--base", "HEAD"]);

    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("in-tree"));
    assert!(r.stderr.contains("git worktree add"));
}

// ---- (5) --changed-only skips (but still lists) a crate no file touched; the
//          exit reflects only the crate that changed.
#[test]
fn changed_only_skips_unchanged_crate_b_and_exits_0_for_clean_crate_a() {
    let ws: std::path::PathBuf = copy_fixture_tree("workspace", "ws-changed-only");
    git_init_commit(&ws);
    std::fs::write(
        ws.join("crate_a").join("src").join("lib.rs"),
        "pub fn new_name() -> u8 {\n    0\n}\n\n// touched for the --changed-only test\n",
    )
    .expect("change only crate_a");
    git_commit_all(&ws, "change crate_a only");

    let r: Run = run_in(
        &ws,
        &[
            "--workspace",
            "--diff-dir",
            &fixture("workspace-diffs"),
            "--changed-only",
            "--base",
            "HEAD~1",
            "--format",
            "json",
        ],
    );

    assert_eq!(r.code, 0);
    let value: serde_json::Value = json_of(&r);
    assert_eq!(value["crates"][0]["name"], "crate_a");
    assert_eq!(value["crates"][0]["status"], "clean");
    assert_eq!(value["crates"][1]["name"], "crate_b");
    assert_eq!(value["crates"][1]["status"], "skipped");
}
