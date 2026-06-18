//! The load-bearing acceptance test: a directive is only real if an agent can
//! act on it. From a FAILing run, append the tool's own emitted `remediation`
//! verbatim to the regime file, re-run, and assert it now exits 0. Anything less
//! is a tool that prints plausible instructions an agent cannot actually apply.

mod common;

use common::{fixture, gate_json, remediations, run, scratch};

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
