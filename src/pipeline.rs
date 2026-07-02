//! Application composition: the single-crate pipeline, factored out so both the
//! single-crate CLI and workspace mode drive the *same*
//! `parse → classify → gate → build` sequence per crate instead of duplicating
//! it.
//!
//! Hexagonal note: this is the composition layer, one step *outside* the pure
//! domain. It wires the `public_api_diff` adapter into the domain calculus
//! (`classify`, `gate`) and assembles the `report` view-model. The domain stays
//! unaware of it and of I/O; dependencies point inward. This function is pure
//! with respect to the outside world — the caller owns producing `diff_text`
//! (from stdin, a file, or a `cargo public-api` subprocess) — which is exactly
//! what lets workspace mode call it once per gated crate.

use crate::adapters::public_api_diff;
use crate::domain::classify::{classify, Classified, MatchMode};
use crate::domain::gate::{gate, GateResult};
use crate::domain::transition::RegimeTransition;
use crate::report::{self, Report};

/// Run the single-crate pipeline for one `(u, diff)` pair: parse the
/// `cargo public-api diff` text, classify every line against the declared
/// transition `u`, gate the classification under `u.kind`, and assemble the
/// stable [`Report`].
///
/// This is the one and only place the `parse → classify → gate → build`
/// sequence lives; the CLI's single-crate path and workspace mode both call it,
/// so their per-crate verdicts cannot drift apart.
#[must_use]
pub fn classify_and_gate(u: &RegimeTransition, diff_text: &str, mode: MatchMode) -> Report {
    let classified: Vec<Classified> = classify(&public_api_diff::parse(diff_text), u, mode);
    let result: GateResult = gate(&classified, u.kind);
    report::build(&classified, &result, u.kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::transition::{Additive, RegimeKind, Rename};
    use crate::report::Verdict;

    fn refactor(renames: Vec<Rename>) -> RegimeTransition {
        RegimeTransition {
            kind: RegimeKind::Refactor,
            renames,
            additive: vec![],
            removals: vec![],
            changes: vec![],
        }
    }

    fn transition(additive: Vec<Additive>) -> RegimeTransition {
        RegimeTransition {
            kind: RegimeKind::Transition,
            renames: vec![],
            additive,
            removals: vec![],
            changes: vec![],
        }
    }

    // ---- zero: an empty diff is a clean pass with nothing to account ----
    #[test]
    fn empty_diff_passes_with_zero_total() {
        let u: RegimeTransition = refactor(vec![]);
        let report: Report = classify_and_gate(&u, "", MatchMode::Identity);
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.counts.total, 0);
    }

    // ---- one: a single undeclared added line is residual and fails ----
    #[test]
    fn one_undeclared_addition_fails_as_residual() {
        let diff_text: &str = "Added items to the public API\n=============================\n+pub fn c::brand_new()\n";
        let u: RegimeTransition = refactor(vec![]);
        let report: Report = classify_and_gate(&u, diff_text, MatchMode::Identity);
        assert_eq!(report.verdict, Verdict::Fail);
        assert_eq!(report.counts.residual, 1);
    }

    // ---- many: a satisfied rename transports both sides to a clean pass ----
    #[test]
    fn many_transported_lines_pass_under_a_declared_rename() {
        let diff_text: &str = "Removed items from the public API\n=================================\n-pub fn c::old_name()\n\nAdded items to the public API\n=============================\n+pub fn c::new_name()\n";
        let u: RegimeTransition = refactor(vec![Rename {
            from: "c::old_name".to_owned(),
            to: "c::new_name".to_owned(),
        }]);
        let report: Report = classify_and_gate(&u, diff_text, MatchMode::Identity);
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.counts.total, 2);
    }

    // ---- many: an intentional addition, declared, passes as a transition ----
    #[test]
    fn many_declared_additions_pass_under_a_transition() {
        let diff_text: &str =
            "Added items to the public API\n=============================\n+pub fn c::feature()\n";
        let u: RegimeTransition = transition(vec![Additive {
            item: "c::feature".to_owned(),
            adr: "ADR-0001".to_owned(),
        }]);
        let report: Report = classify_and_gate(&u, diff_text, MatchMode::Identity);
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.counts.residual, 0);
    }
}
