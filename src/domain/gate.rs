//! The gate: decide whether the classified diff is admissible under the claimed
//! [`RegimeKind`], and for every inadmissible line emit the structured
//! [`RequiredAction`] an agent must take. Pure policy — no strings about TOML
//! format live here (that belongs to the `regime_file` adapter).

use crate::domain::classify::{Class, Classified};
use crate::domain::diff::ApiItem;
use crate::domain::transition::RegimeKind;

/// The structured directive for one failing line: exactly what must change to
/// make it admissible. The `regime_file` adapter renders these into prose plus a
/// copy-pasteable TOML snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredAction {
    /// Undeclared added surface: declare `[[additive]]` (and set kind=transition)
    /// if intentional, else hide or remove it.
    DeclareAdditive,
    /// Undeclared removal: declare `[[removal]]` and bump major if intentional,
    /// else restore the item.
    DeclareRemoval,
    /// Undeclared signature change: declare `[[change]]` with an ADR if
    /// intentional, else revert the signature.
    DeclareChange,
    /// Declared add/remove/change under a `refactor` claim: this is a
    /// transition, not a refactor — set `meta.kind = "transition"`.
    ReclassifyAsTransition,
}

/// The single source of truth for "is this line a violation, and if so what is
/// required?". Returns `None` for an admissible line.
#[must_use]
pub fn required_action(class: &Class, kind: RegimeKind) -> Option<RequiredAction> {
    match (class, kind) {
        (Class::ResidualAdditive, _) => Some(RequiredAction::DeclareAdditive),
        (Class::ResidualRemoval, _) => Some(RequiredAction::DeclareRemoval),
        (Class::ResidualChange, _) => Some(RequiredAction::DeclareChange),
        (Class::DeclaredAdditive(_), RegimeKind::Refactor)
        | (Class::DeclaredRemoval(_), RegimeKind::Refactor)
        | (Class::DeclaredChange(_), RegimeKind::Refactor) => {
            Some(RequiredAction::ReclassifyAsTransition)
        }
        _ => None,
    }
}

/// A line the gate rejected, with the action required to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub item: ApiItem,
    pub action: RequiredAction,
}

/// Outcome of the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub passed: bool,
    pub violations: Vec<Violation>,
}

/// Gate the classified diff against the claimed [`RegimeKind`].
///
/// - Any residual (undeclared add/remove/change) is always a violation.
/// - A `Refactor` claim additionally rejects *declared* additive/removal/change:
///   if you must declare new or lost surface, it is a transition, not a refactor.
#[must_use]
pub fn gate(items: &[Classified], kind: RegimeKind) -> GateResult {
    let violations: Vec<Violation> = items
        .iter()
        .filter_map(|c: &Classified| {
            required_action(&c.class, kind).map(|action: RequiredAction| Violation {
                item: c.item.clone(),
                action,
            })
        })
        .collect();
    GateResult {
        passed: violations.is_empty(),
        violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::ApiItem;

    fn classified(class: Class) -> Classified {
        Classified {
            item: ApiItem::new("pub fn c::x()"),
            class,
        }
    }

    // ---- zero ----
    #[test]
    fn gate_passes_on_no_items() {
        let res: GateResult = gate(&[], RegimeKind::Refactor);
        assert!(res.passed);
    }

    // ---- one ----
    #[test]
    fn gate_fails_on_residual_additive() {
        let res: GateResult = gate(
            &[classified(Class::ResidualAdditive)],
            RegimeKind::Transition,
        );
        assert!(!res.passed);
        assert_eq!(res.violations[0].action, RequiredAction::DeclareAdditive);
    }

    #[test]
    fn refactor_rejects_declared_additive_as_a_transition() {
        let res: GateResult = gate(
            &[classified(Class::DeclaredAdditive("ADR-1".to_owned()))],
            RegimeKind::Refactor,
        );
        assert!(!res.passed);
        assert_eq!(
            res.violations[0].action,
            RequiredAction::ReclassifyAsTransition
        );
    }

    #[test]
    fn transition_accepts_declared_additive() {
        let res: GateResult = gate(
            &[classified(Class::DeclaredAdditive("ADR-1".to_owned()))],
            RegimeKind::Transition,
        );
        assert!(res.passed);
    }

    // ---- many ----
    #[test]
    fn gate_collects_every_violation_and_skips_accounted() {
        let items: Vec<Classified> = vec![
            classified(Class::ResidualAdditive),
            classified(Class::ResidualRemoval),
            classified(Class::TransportedIso),
        ];
        let res: GateResult = gate(&items, RegimeKind::Transition);
        assert_eq!(res.violations.len(), 2); // iso is fine; two residuals fail
    }
}
