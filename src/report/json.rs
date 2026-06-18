//! Machine renderer: the [`Report`] as deterministic, colourless JSON.
//!
//! Field order is fixed by the struct definition (no maps), so re-runs on the
//! same input are byte-identical — an agent can diff them.

use crate::report::Report;

/// Render the report as pretty JSON. Infallible for our value types; a
/// serialization failure would be a bug, so it is surfaced as such.
#[must_use]
pub fn render(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("Report serializes to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Counts, Report, ReportItem, Verdict};

    fn sample() -> Report {
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
                token: "pub fn c::x()".to_owned(),
                path: "c::x".to_owned(),
                class: "residual_additive",
                detail: None,
                required_action: Some("do the thing".to_owned()),
                remediation: Some("[[additive]]\nitem = \"c::x\"\n".to_owned()),
            }],
        }
    }

    #[test]
    fn renders_valid_json_round_trip() {
        let json: String = render(&sample());
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["verdict"], "fail");
        assert_eq!(parsed["items"][0]["path"], "c::x");
    }

    #[test]
    fn is_deterministic() {
        assert_eq!(render(&sample()), render(&sample()));
    }
}
