//! Workspace presentation layer: fold the per-crate single-crate [`Report`]s
//! into one aggregated view-model ([`WorkspaceReport`]) and render it for humans
//! ([`render_human`]) or machines ([`render_json`]).
//!
//! This is a *pure* aggregation, one step outside the domain: it composes values
//! the orchestrator already produced (a `Report` per gated crate, or a recorded
//! tool-error / skip) and never touches git, processes, or the filesystem. The
//! pure domain (`classify`/`gate`/`identity`/`diff`/`transition`) is unchanged
//! and unaware of it; dependencies point inward.
//!
//! Aggregation adds, it never rewrites: a gated crate embeds its single-crate
//! [`Report`] object UNCHANGED under a `report` field, so the embedded object
//! parses byte-for-byte EQUAL to a standalone single-crate `--format json` run
//! of that crate. Field order is fixed by the struct definitions and `Option`
//! payloads are skipped when absent, so re-runs on the same input are
//! byte-identical — an agent can diff them.

use std::fmt::Write as _;

use serde::Serialize;

use crate::report::{Report, Verdict};

/// The aggregated workspace verdict. Serializes to `"pass"` / `"fail"` /
/// `"error"`. Mirrors the single-crate pass/fail vocabulary and adds `error`
/// for the tool-error class. Maps to the process exit code via [`Self::exit_code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceVerdict {
    Pass,
    Fail,
    Error,
}

impl WorkspaceVerdict {
    /// The process exit code this verdict maps to: `pass → 0`, `fail → 1`,
    /// `error → 2`. The exit priority is `error(2) > residual/fail(1) >
    /// clean/pass(0)`, encoded once here so the driving adapter cannot drift.
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            WorkspaceVerdict::Pass => 0,
            WorkspaceVerdict::Fail => 1,
            WorkspaceVerdict::Error => 2,
        }
    }
}

/// The per-crate status in the aggregated schema. Serializes to
/// `"clean"` / `"residual"` / `"errored"` / `"skipped"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CrateStatus {
    Clean,
    Residual,
    Errored,
    Skipped,
}

/// The outcome of evaluating one workspace crate.
///
/// - `Gated` — the crate was gated and produced a single-crate [`Report`]
///   (whose own verdict resolves to `clean` or `residual`).
/// - `Errored` — the crate's diff could not be produced (e.g. its tool failed or
///   its diff file was missing); recorded, never silently dropped.
/// - `Skipped` — the crate was deliberately not evaluated (e.g. nothing relevant
///   changed under `--changed-only`); recorded with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrateOutcome {
    Gated(Report),
    Errored {
        message: String,
        hint: Option<String>,
    },
    Skipped {
        reason: String,
    },
}

impl CrateOutcome {
    /// The stable status name for this outcome. A gated crate's status follows
    /// its embedded report's verdict.
    #[must_use]
    pub fn status(&self) -> CrateStatus {
        match self {
            CrateOutcome::Gated(report) => match report.verdict {
                Verdict::Pass => CrateStatus::Clean,
                Verdict::Fail => CrateStatus::Residual,
            },
            CrateOutcome::Errored { .. } => CrateStatus::Errored,
            CrateOutcome::Skipped { .. } => CrateStatus::Skipped,
        }
    }
}

/// One crate's entry in the aggregated report: its name and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateEntry {
    pub name: String,
    pub outcome: CrateOutcome,
}

/// Headline tallies. `crates = clean + residual + errored` counts the *evaluated*
/// crates; `skipped` crates are deliberately NOT part of `crates` (they were not
/// evaluated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WorkspaceCounts {
    pub crates: usize,
    pub clean: usize,
    pub residual: usize,
    pub errored: usize,
    pub skipped: usize,
}

/// The aggregated workspace report — the view-model the workspace renderers
/// consume. Crates are sorted by name for determinism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceReport {
    pub verdict: WorkspaceVerdict,
    pub counts: WorkspaceCounts,
    pub crates: Vec<CrateEntry>,
}

/// Fold the per-crate outcomes into the aggregated [`WorkspaceReport`].
///
/// Crates are sorted by name (deterministic output). The verdict follows the
/// exit priority `error > residual > clean`: `error` if any crate errored, else
/// `fail` if any gated crate is residual, else `pass`.
#[must_use]
pub fn build(mut entries: Vec<CrateEntry>) -> WorkspaceReport {
    entries.sort_by(|a: &CrateEntry, b: &CrateEntry| a.name.cmp(&b.name));

    let mut clean: usize = 0;
    let mut residual: usize = 0;
    let mut errored: usize = 0;
    let mut skipped: usize = 0;
    for entry in &entries {
        match entry.outcome.status() {
            CrateStatus::Clean => clean += 1,
            CrateStatus::Residual => residual += 1,
            CrateStatus::Errored => errored += 1,
            CrateStatus::Skipped => skipped += 1,
        }
    }

    let verdict: WorkspaceVerdict = if errored > 0 {
        WorkspaceVerdict::Error
    } else if residual > 0 {
        WorkspaceVerdict::Fail
    } else {
        WorkspaceVerdict::Pass
    };

    WorkspaceReport {
        verdict,
        counts: WorkspaceCounts {
            crates: clean + residual + errored,
            clean,
            residual,
            errored,
            skipped,
        },
        crates: entries,
    }
}

// ---- JSON rendering -------------------------------------------------------

/// Serialization view of the whole workspace report. A dedicated view keeps the
/// pure model free of the JSON shape (the `status` is derived and the payload is
/// per-variant) while a fixed field order keeps the output byte-stable.
#[derive(Serialize)]
struct WorkspaceView<'a> {
    verdict: WorkspaceVerdict,
    counts: WorkspaceCounts,
    crates: Vec<CrateView<'a>>,
}

/// Serialization view of one crate entry. Exactly one payload field is present
/// per status; the rest are skipped. A gated crate embeds its [`Report`] by
/// reference so it serializes UNCHANGED.
#[derive(Serialize)]
struct CrateView<'a> {
    name: &'a str,
    status: CrateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<&'a Report>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
}

/// Project one crate entry onto its serialization view.
fn crate_view(entry: &CrateEntry) -> CrateView<'_> {
    match &entry.outcome {
        CrateOutcome::Gated(report) => CrateView {
            name: &entry.name,
            status: entry.outcome.status(),
            report: Some(report),
            error: None,
            hint: None,
            reason: None,
        },
        CrateOutcome::Errored { message, hint } => CrateView {
            name: &entry.name,
            status: CrateStatus::Errored,
            report: None,
            error: Some(message),
            hint: hint.as_deref(),
            reason: None,
        },
        CrateOutcome::Skipped { reason } => CrateView {
            name: &entry.name,
            status: CrateStatus::Skipped,
            report: None,
            error: None,
            hint: None,
            reason: Some(reason),
        },
    }
}

/// Render the aggregated report as deterministic, colourless pretty JSON. Each
/// gated crate embeds its single-crate [`Report`] object unchanged.
#[must_use]
pub fn render_json(report: &WorkspaceReport) -> String {
    let mut crates: Vec<CrateView<'_>> = Vec::with_capacity(report.crates.len());
    for entry in &report.crates {
        crates.push(crate_view(entry));
    }
    let view: WorkspaceView<'_> = WorkspaceView {
        verdict: report.verdict,
        counts: report.counts,
        crates,
    };
    serde_json::to_string_pretty(&view).expect("WorkspaceReport serializes to JSON")
}

// ---- human rendering ------------------------------------------------------

/// The upper-case verdict word for the human header.
fn verdict_word(verdict: WorkspaceVerdict) -> &'static str {
    match verdict {
        WorkspaceVerdict::Pass => "PASS",
        WorkspaceVerdict::Fail => "FAIL",
        WorkspaceVerdict::Error => "ERROR",
    }
}

/// One summary line for a crate: name + status + its counts (gated) or its
/// message/reason (errored/skipped). Errored crates append their hint.
fn summary_line(entry: &CrateEntry) -> String {
    match &entry.outcome {
        CrateOutcome::Gated(report) => {
            let (sign, status): (char, &str) = match report.verdict {
                Verdict::Pass => ('✓', "clean"),
                Verdict::Fail => ('✗', "residual"),
            };
            format!(
                "  {sign} {name}   {status}   ({total} item(s), {accounted} accounted, {residual} residual)",
                name = entry.name,
                total = report.counts.total,
                accounted = report.counts.accounted,
                residual = report.counts.residual,
            )
        }
        CrateOutcome::Errored { message, hint } => match hint {
            Some(hint) => format!(
                "  ! {name}   errored   ({message})\n      hint: {hint}",
                name = entry.name
            ),
            None => format!("  ! {name}   errored   ({message})", name = entry.name),
        },
        CrateOutcome::Skipped { reason } => {
            format!("  · {name}   skipped   ({reason})", name = entry.name)
        }
    }
}

/// Render the aggregated report as a plain-text summary: a headline, one line
/// per crate (every crate listed — clean, residual, errored, and skipped are all
/// explicit, never silently absent), then the FULL single-crate residual detail
/// (directive + copy-pasteable fix) for each failing crate.
#[must_use]
pub fn render_human(report: &WorkspaceReport) -> String {
    let mut out: String = String::new();
    let word: &str = verdict_word(report.verdict);
    let counts: &WorkspaceCounts = &report.counts;

    let _ = writeln!(
        out,
        "regime-check --workspace: {word} — {} crate(s) evaluated",
        counts.crates
    );
    let _ = writeln!(
        out,
        "  {} clean, {} residual, {} errored, {} skipped\n",
        counts.clean, counts.residual, counts.errored, counts.skipped
    );

    let _ = writeln!(out, "crates:");
    for entry in &report.crates {
        let _ = writeln!(out, "{}", summary_line(entry));
    }

    if counts.residual > 0 {
        let _ = writeln!(out, "\nresidual detail:");
        for entry in &report.crates {
            if let CrateOutcome::Gated(single) = &entry.outcome {
                if single.verdict == Verdict::Fail {
                    let _ = writeln!(out, "\n── {} ──", entry.name);
                    let _ = write!(out, "{}", crate::report::human::render(single));
                }
            }
        }
    }

    let _ = writeln!(
        out,
        "\nverdict: {word} (exit {})",
        report.verdict.exit_code()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Counts, Report, ReportItem, Verdict};

    fn pass_report() -> Report {
        Report {
            verdict: Verdict::Pass,
            kind: "transition",
            counts: Counts {
                total: 1,
                accounted: 1,
                residual: 0,
                violations: 0,
            },
            items: vec![ReportItem {
                token: "pub fn c::kept()".to_owned(),
                path: "c::kept".to_owned(),
                class: "transported_iso",
                detail: None,
                required_action: None,
                remediation: None,
            }],
        }
    }

    fn fail_report() -> Report {
        Report {
            verdict: Verdict::Fail,
            kind: "transition",
            counts: Counts {
                total: 1,
                accounted: 0,
                residual: 1,
                violations: 1,
            },
            items: vec![ReportItem {
                token: "pub fn c::brand_new()".to_owned(),
                path: "c::brand_new".to_owned(),
                class: "residual_additive",
                detail: None,
                required_action: Some("undeclared added surface.".to_owned()),
                remediation: Some("[[additive]]\nitem = \"c::brand_new\"\n".to_owned()),
            }],
        }
    }

    fn gated(name: &str, report: Report) -> CrateEntry {
        CrateEntry {
            name: name.to_owned(),
            outcome: CrateOutcome::Gated(report),
        }
    }

    fn errored(name: &str, hint: Option<&str>) -> CrateEntry {
        CrateEntry {
            name: name.to_owned(),
            outcome: CrateOutcome::Errored {
                message: "boom".to_owned(),
                hint: hint.map(str::to_owned),
            },
        }
    }

    fn skipped(name: &str) -> CrateEntry {
        CrateEntry {
            name: name.to_owned(),
            outcome: CrateOutcome::Skipped {
                reason: "unchanged".to_owned(),
            },
        }
    }

    // ---- zero ----
    #[test]
    fn no_crates_is_pass_with_zero_counts() {
        let report: WorkspaceReport = build(vec![]);
        assert_eq!(report.verdict, WorkspaceVerdict::Pass);
        assert_eq!(
            report.counts,
            WorkspaceCounts {
                crates: 0,
                clean: 0,
                residual: 0,
                errored: 0,
                skipped: 0,
            }
        );
        assert!(report.crates.is_empty());
    }

    // ---- one ----
    #[test]
    fn one_clean_crate_is_pass() {
        let report: WorkspaceReport = build(vec![gated("a", pass_report())]);
        assert_eq!(report.verdict, WorkspaceVerdict::Pass);
        assert_eq!(
            report.counts,
            WorkspaceCounts {
                crates: 1,
                clean: 1,
                residual: 0,
                errored: 0,
                skipped: 0,
            }
        );
    }

    // ---- many: error dominates residual dominates clean; counts add up ----
    #[test]
    fn many_mixed_crates_error_dominates_and_counts_add_up() {
        let report: WorkspaceReport = build(vec![
            gated("clean-a", pass_report()),
            gated("resid-b", fail_report()),
            errored("err-c", None),
            skipped("skip-d"),
        ]);
        assert_eq!(report.verdict, WorkspaceVerdict::Error);
        assert_eq!(
            report.counts,
            WorkspaceCounts {
                crates: 3,
                clean: 1,
                residual: 1,
                errored: 1,
                skipped: 1,
            }
        );
    }

    // ---- many: residual without any error is fail; skipped stays uncounted ----
    #[test]
    fn residual_without_error_is_fail() {
        let report: WorkspaceReport = build(vec![
            gated("clean-a", pass_report()),
            gated("resid-b", fail_report()),
            skipped("skip-c"),
        ]);
        assert_eq!(report.verdict, WorkspaceVerdict::Fail);
        assert_eq!(report.counts.crates, 2);
        assert_eq!(report.counts.skipped, 1);
    }

    // ---- many: crates are sorted by name ----
    #[test]
    fn crates_are_sorted_by_name() {
        let report: WorkspaceReport = build(vec![
            gated("zebra", pass_report()),
            gated("alpha", pass_report()),
            gated("mango", pass_report()),
        ]);
        assert_eq!(report.crates[0].name, "alpha");
        assert_eq!(report.crates[1].name, "mango");
        assert_eq!(report.crates[2].name, "zebra");
    }

    // ---- verdict → exit code mapping ----
    #[test]
    fn verdict_exit_codes_are_zero_one_two() {
        assert_eq!(WorkspaceVerdict::Pass.exit_code(), 0);
        assert_eq!(WorkspaceVerdict::Fail.exit_code(), 1);
        assert_eq!(WorkspaceVerdict::Error.exit_code(), 2);
    }

    // ---- the embedded report is the single-crate report, unchanged ----
    #[test]
    fn gated_entry_embeds_the_single_crate_report_unchanged() {
        let single: Report = fail_report();
        let standalone: serde_json::Value =
            serde_json::from_str(&crate::report::json::render(&single)).unwrap();
        let workspace: WorkspaceReport = build(vec![gated("a", single)]);
        let rendered: serde_json::Value = serde_json::from_str(&render_json(&workspace)).unwrap();
        assert_eq!(rendered["crates"][0]["report"], standalone);
    }

    // ---- JSON carries the derived status and the per-variant payloads ----
    #[test]
    fn json_carries_status_and_variant_payloads() {
        let workspace: WorkspaceReport = build(vec![
            gated("clean-a", pass_report()),
            gated("resid-b", fail_report()),
            errored("err-c", Some("try again")),
            skipped("skip-d"),
        ]);
        let value: serde_json::Value = serde_json::from_str(&render_json(&workspace)).unwrap();
        assert_eq!(value["verdict"], "error");
        assert_eq!(value["crates"][0]["status"], "clean");
        assert_eq!(value["crates"][1]["status"], "errored");
        assert_eq!(value["crates"][1]["error"], "boom");
        assert_eq!(value["crates"][1]["hint"], "try again");
        assert_eq!(value["crates"][2]["status"], "residual");
        assert_eq!(value["crates"][3]["status"], "skipped");
        assert_eq!(value["crates"][3]["reason"], "unchanged");
    }

    // ---- JSON is deterministic (byte-identical re-runs) ----
    #[test]
    fn json_is_deterministic() {
        let workspace: WorkspaceReport = build(vec![gated("a", fail_report()), skipped("b")]);
        assert_eq!(render_json(&workspace), render_json(&workspace));
    }

    // ---- human: every crate is listed, and failing crates get full detail ----
    #[test]
    fn human_lists_every_crate_and_shows_residual_detail() {
        let workspace: WorkspaceReport = build(vec![
            gated("clean-a", pass_report()),
            gated("resid-b", fail_report()),
            errored("err-c", Some("try again")),
            skipped("skip-d"),
        ]);
        let text: String = render_human(&workspace);
        assert!(text.contains("clean-a"));
        assert!(text.contains("resid-b"));
        assert!(text.contains("err-c"));
        assert!(text.contains("skip-d"));
        assert!(text.contains("residual detail"));
        assert!(text.contains("[[additive]]"));
        assert!(text.contains("ERROR"));
    }

    // ---- human: a clean run has no residual-detail section ----
    #[test]
    fn human_pass_has_no_residual_detail() {
        let workspace: WorkspaceReport = build(vec![gated("a", pass_report())]);
        let text: String = render_human(&workspace);
        assert!(text.contains("PASS"));
        assert!(!text.contains("residual detail"));
    }
}
