//! Presentation layer: assemble the gate's decision into a stable view-model
//! ([`Report`]) and render it for humans ([`human`]) or machines ([`json`]).
//!
//! The view-model is the documented contract an agent depends on. Each item
//! carries its `required_action` (prose directive) and `remediation` (a
//! copy-pasteable `regime-transition.toml` snippet) so stdout alone resolves a
//! FAIL. Both are rendered via the `regime_file` adapter, the owner of the TOML
//! format.

pub mod human;
pub mod json;

use serde::Serialize;

use crate::adapters::regime_file;
use crate::domain::classify::{Class, Classified};
use crate::domain::gate::{required_action, GateResult, RequiredAction};
use crate::domain::transition::RegimeKind;

/// The overall gate verdict. Serializes to `"pass"` / `"fail"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
}

/// Headline tallies. `accounted = total - residual`; `violations` additionally
/// counts declared-surface-under-refactor (Case C), which is accounted yet still
/// rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub accounted: usize,
    pub residual: usize,
    pub violations: usize,
}

/// One classified line in the stable schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportItem {
    /// The full `cargo public-api` token.
    pub token: String,
    /// The resolved identity path the functor matches on.
    pub path: String,
    /// Stable class name (e.g. `residual_additive`, `transported_iso`).
    pub class: &'static str,
    /// The ADR/reason that accounted a *declared* line; `null` otherwise.
    pub detail: Option<String>,
    /// Prose directive for a failing line — what to DO. `null` if admissible.
    pub required_action: Option<String>,
    /// Copy-pasteable `regime-transition.toml` snippet that makes this line
    /// accounted-for. `null` if admissible.
    pub remediation: Option<String>,
}

/// The whole report — the documented `--format json` schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub verdict: Verdict,
    pub kind: &'static str,
    pub counts: Counts,
    pub items: Vec<ReportItem>,
}

/// The ADR/reason carried by a *declared* class, for display.
fn class_detail(class: &Class) -> Option<String> {
    match class {
        Class::DeclaredAdditive(adr) | Class::DeclaredChange(adr) => Some(adr.clone()),
        Class::DeclaredRemoval(reason) => Some(reason.clone()),
        _ => None,
    }
}

/// Assemble the report from the classified diff and the gate result.
#[must_use]
pub fn build(items: &[Classified], result: &GateResult, kind: RegimeKind) -> Report {
    let report_items: Vec<ReportItem> = items
        .iter()
        .map(|c: &Classified| {
            let action: Option<RequiredAction> = required_action(&c.class, kind);
            ReportItem {
                token: c.item.token.clone(),
                path: c.item.identity.path.clone(),
                class: c.class.as_str(),
                detail: class_detail(&c.class),
                required_action: action
                    .map(|a: RequiredAction| regime_file::directive(a).to_owned()),
                remediation: action
                    .map(|a: RequiredAction| regime_file::snippet(a, &c.item.identity.path)),
            }
        })
        .collect();

    let residual: usize = items
        .iter()
        .filter(|c: &&Classified| c.class.is_residual())
        .count();
    let total: usize = items.len();

    Report {
        verdict: if result.passed {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        kind: kind.as_str(),
        counts: Counts {
            total,
            accounted: total - residual,
            residual,
            violations: result.violations.len(),
        },
        items: report_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classify::{classify, MatchMode};
    use crate::domain::diff::{ApiDiff, ApiItem};
    use crate::domain::gate::gate;
    use crate::domain::transition::RegimeTransition;

    fn transition() -> RegimeTransition {
        RegimeTransition {
            kind: RegimeKind::Transition,
            renames: vec![],
            additive: vec![],
            removals: vec![],
            changes: vec![],
        }
    }

    fn fail_report() -> Report {
        let diff: ApiDiff = ApiDiff {
            added: vec![ApiItem::new("pub fn c::brand_new()")],
            ..Default::default()
        };
        let u: RegimeTransition = transition();
        let classified: Vec<Classified> = classify(&diff, &u, MatchMode::Identity);
        let result: GateResult = gate(&classified, u.kind);
        build(&classified, &result, u.kind)
    }

    // ---- zero ----
    #[test]
    fn empty_diff_is_pass_with_no_items() {
        let u: RegimeTransition = transition();
        let classified: Vec<Classified> = classify(&ApiDiff::default(), &u, MatchMode::Identity);
        let result: GateResult = gate(&classified, u.kind);
        let report: Report = build(&classified, &result, u.kind);
        assert_eq!(report.verdict, Verdict::Pass);
        assert!(report.items.is_empty());
    }

    // ---- one ----
    #[test]
    fn residual_item_carries_action_and_remediation() {
        let report: Report = fail_report();
        assert_eq!(report.verdict, Verdict::Fail);
        let item: &ReportItem = &report.items[0];
        assert_eq!(item.class, "residual_additive");
        assert!(item.required_action.is_some());
        assert!(item
            .remediation
            .as_deref()
            .unwrap()
            .contains("c::brand_new"));
    }

    // ---- round-trip ----
    #[test]
    fn json_serializes_documented_keys() {
        let report: Report = fail_report();
        let json: String = serde_json::to_string(&report).unwrap();
        for key in [
            "verdict",
            "kind",
            "counts",
            "items",
            "token",
            "class",
            "required_action",
            "remediation",
        ] {
            assert!(json.contains(key), "missing key {key} in {json}");
        }
    }
}
