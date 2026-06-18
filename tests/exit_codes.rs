//! The documented exit-code contract an agent branches on: 0 clean, 1 undeclared
//! residual, 2 usage/IO/parse error. Plus the JSON schema's headline keys.

mod common;

use common::{example, fixture, gate_json, run};

// ---- one: each exit code ----
#[test]
fn declared_transition_passes_with_exit_0() {
    let r = run(&[
        "--regime",
        &example("transition.toml"),
        "--diff",
        &fixture("demo.diff"),
    ]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("verdict: PASS"));
}

#[test]
fn false_refactor_claim_fails_with_exit_1() {
    let r = run(&[
        "--regime",
        &example("refactor.toml"),
        "--diff",
        &fixture("demo.diff"),
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.contains("verdict: FAIL"));
}

#[test]
fn missing_regime_file_is_usage_error_exit_2() {
    let r = run(&[
        "--regime",
        "/does/not/exist.toml",
        "--diff",
        &fixture("demo.diff"),
    ]);
    assert_eq!(r.code, 2);
}

#[test]
fn unknown_argument_is_usage_error_exit_2() {
    let r = run(&["--nonsense"]);
    assert_eq!(r.code, 2);
}

// ---- idempotence ----
#[test]
fn reruns_are_byte_identical() {
    let first = gate_json(&example("transition.toml"), &fixture("demo.diff"));
    let second = gate_json(&example("transition.toml"), &fixture("demo.diff"));
    assert_eq!(first.stdout, second.stdout);
}

// ---- self-contained flags exit 0 ----
#[test]
fn explain_and_template_and_help_exit_0() {
    assert_eq!(run(&["--explain"]).code, 0);
    assert_eq!(run(&["--template"]).code, 0);
    assert_eq!(run(&["--help"]).code, 0);
}

// ---- agent ergonomics: discoverability + intent inference ----
#[test]
fn bare_invocation_shows_help_and_exits_0() {
    let r = run(&[]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("USAGE"));
}

#[test]
fn capabilities_is_valid_json_with_exit_code_dictionary() {
    let r = run(&["--capabilities"]);
    assert_eq!(r.code, 0);
    let value: serde_json::Value = serde_json::from_str(&r.stdout).expect("capabilities json");
    assert_eq!(
        value["exit_codes"]["1"],
        "undeclared/contradictory residual"
    );
    assert!(value["report_schema"]["items"].is_array());
}

#[test]
fn json_flag_is_an_alias_for_format_json() {
    let r = run(&[
        "--regime",
        &example("transition.toml"),
        "--diff",
        &fixture("demo.diff"),
        "--json",
    ]);
    assert_eq!(r.code, 0);
    let value: serde_json::Value = serde_json::from_str(&r.stdout).expect("json output");
    assert_eq!(value["verdict"], "pass");
}

#[test]
fn unknown_flag_suggests_the_nearest_known_flag() {
    let r = run(&["--regimee", "x"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("did you mean `--regime`"));
}

// ---- json error is structured ----
#[test]
fn missing_regime_in_json_mode_emits_structured_error() {
    let r = run(&[
        "--regime",
        "/does/not/exist.toml",
        "--diff",
        &fixture("demo.diff"),
        "--format",
        "json",
    ]);
    assert_eq!(r.code, 2);
    let value: serde_json::Value = serde_json::from_str(&r.stdout).expect("json error");
    assert!(value["error"].is_string());
    assert!(value["template"].is_string());
}
