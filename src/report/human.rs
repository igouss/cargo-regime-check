//! Human renderer: the default, readable report. Accounted lines are shown
//! quietly; every residual line is followed by its directive and a
//! copy-pasteable fix, so a human (or an agent reading stdout) can resolve a
//! FAIL without the JSON form.

use std::fmt::Write as _;

use crate::report::{Report, ReportItem, Verdict};

/// `(sign, note)` for the accounted/quiet listing of a class.
fn label(class: &str) -> (char, &'static str) {
    match class {
        "transported_iso" => ('~', "rename (transported)"),
        "declared_additive" => ('+', "declared discovery"),
        "declared_removal" => ('-', "declared removal"),
        "declared_change" => ('!', "declared change"),
        "residual_additive" => ('+', "undeclared added surface"),
        "residual_removal" => ('-', "undeclared removal (breaking)"),
        "residual_change" => ('!', "undeclared signature change"),
        _ => ('?', "unknown"),
    }
}

/// Indent every line of `block` by four spaces, for the inline fix snippet.
fn indent(block: &str) -> String {
    block
        .lines()
        .map(|l: &str| format!("    {l}"))
        .collect::<Vec<String>>()
        .join("\n")
}

/// Render the whole report as a plain-text string.
#[must_use]
pub fn render(report: &Report) -> String {
    let mut out: String = String::new();
    let kind_note: &str = match report.kind {
        "refactor" => "refactor (residual must be 0)",
        _ => "transition (residual must be declared)",
    };
    let _ = writeln!(out, "regime-check: {kind_note}");
    let _ = writeln!(
        out,
        "  {} item(s) — {} accounted, {} residual\n",
        report.counts.total, report.counts.accounted, report.counts.residual
    );

    let accounted: Vec<&ReportItem> = report
        .items
        .iter()
        .filter(|i: &&ReportItem| i.required_action.is_none())
        .collect();
    if !accounted.is_empty() {
        let _ = writeln!(out, "accounted (no review needed):");
        for item in accounted {
            let (sign, note): (char, &str) = label(item.class);
            match &item.detail {
                Some(detail) => {
                    let _ = writeln!(out, "  {sign} {}   [{note}: {detail}]", item.token);
                }
                None => {
                    let _ = writeln!(out, "  {sign} {}   [{note}]", item.token);
                }
            }
        }
        let _ = writeln!(out);
    }

    let violations: Vec<&ReportItem> = report
        .items
        .iter()
        .filter(|i: &&ReportItem| i.required_action.is_some())
        .collect();

    if violations.is_empty() {
        let _ = writeln!(
            out,
            "verdict: PASS — every line is transported or declared. (exit 0)"
        );
        return out;
    }

    let _ = writeln!(
        out,
        "RESIDUAL — {} line(s) need action:\n",
        violations.len()
    );
    for item in &violations {
        let _ = writeln!(out, "  ✗ {}", item.token);
        if let Some(action) = &item.required_action {
            let _ = writeln!(out, "      → {action}");
        }
        if let Some(fix) = &item.remediation {
            let _ = writeln!(out, "      fix (append to your regime-transition.toml):");
            let _ = writeln!(out, "{}", indent(fix));
        }
    }
    let _ = writeln!(
        out,
        "verdict: FAIL — {} undeclared/contradictory line(s). (exit 1)",
        report.counts.violations
    );

    debug_assert_eq!(report.verdict, Verdict::Fail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Counts, Report, ReportItem, Verdict};

    fn pass() -> Report {
        Report {
            verdict: Verdict::Pass,
            kind: "transition",
            counts: Counts {
                total: 0,
                accounted: 0,
                residual: 0,
                violations: 0,
            },
            items: vec![],
        }
    }

    fn fail() -> Report {
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

    // ---- zero ----
    #[test]
    fn pass_report_says_pass() {
        let text: String = render(&pass());
        assert!(text.contains("verdict: PASS"));
    }

    // ---- one ----
    #[test]
    fn fail_report_shows_directive_and_fix() {
        let text: String = render(&fail());
        assert!(text.contains("verdict: FAIL"));
        assert!(text.contains("undeclared added surface"));
        assert!(text.contains("[[additive]]"));
        assert!(text.contains("c::brand_new"));
    }

    // ---- many ----
    #[test]
    fn accounted_and_residual_are_both_listed() {
        let mut report: Report = fail();
        report.items.push(ReportItem {
            token: "pub fn c::kept()".to_owned(),
            path: "c::kept".to_owned(),
            class: "transported_iso",
            detail: None,
            required_action: None,
            remediation: None,
        });
        report.counts.total = 2;
        report.counts.accounted = 1;
        let text: String = render(&report);
        assert!(text.contains("accounted (no review needed)"));
        assert!(text.contains("c::kept"));
        assert!(text.contains("RESIDUAL"));
    }
}
