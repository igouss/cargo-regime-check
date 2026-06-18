//! The classification calculus: subtract the declared transition `u` from a
//! diff and label every line. Pure and total — testable against hand-built
//! diffs with no `cargo public-api` present.
//!
//! This is the operational form of the paper's definition (arXiv:2606.01444):
//! transporting old items forward by the declared functor `u` and comparing to
//! the new surface leaves a *residual*; the residual is what must be justified.

use crate::domain::diff::{ApiDiff, ApiItem};
use crate::domain::transition::{Additive, Change, RegimeTransition, Removal, Rename};

/// How a declaration in `u` is matched against a diff item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchMode {
    /// Match the declared path against the item's *resolved identity path*,
    /// exactly. This is the faithful stand-in for the functor (priority-1).
    #[default]
    Identity,
    /// Legacy fallback: match by substring containment of the declared string
    /// in the raw token. Looser; under-reports residual. Kept for tokens whose
    /// structure the identity parser cannot resolve, and for back-compat.
    Substring,
}

/// What each diff line is, once `u` has been subtracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    /// Accounted by a declared rename — the iso part of `u`.
    TransportedIso,
    /// Declared discovery, carrying its ADR reference.
    DeclaredAdditive(String),
    /// Declared, acknowledged removal, carrying its reason.
    DeclaredRemoval(String),
    /// Declared, justified change, carrying its ADR reference.
    DeclaredChange(String),
    /// Undeclared new surface — the residual that needs an ADR.
    ResidualAdditive,
    /// Undeclared removal — a breaking change that needs acknowledging.
    ResidualRemoval,
    /// Undeclared signature change — needs justifying.
    ResidualChange,
}

impl Class {
    /// The stable snake_case name used in JSON output.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Class::TransportedIso => "transported_iso",
            Class::DeclaredAdditive(_) => "declared_additive",
            Class::DeclaredRemoval(_) => "declared_removal",
            Class::DeclaredChange(_) => "declared_change",
            Class::ResidualAdditive => "residual_additive",
            Class::ResidualRemoval => "residual_removal",
            Class::ResidualChange => "residual_change",
        }
    }

    /// Is this line undeclared residual (the part that fails the gate)?
    #[must_use]
    pub fn is_residual(&self) -> bool {
        matches!(
            self,
            Class::ResidualAdditive | Class::ResidualRemoval | Class::ResidualChange
        )
    }
}

/// One classified diff line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub item: ApiItem,
    pub class: Class,
}

/// Does this item satisfy declaration `decl` under `mode`?
fn matches(item: &ApiItem, decl: &str, mode: MatchMode) -> bool {
    match mode {
        MatchMode::Identity => !item.identity.path.is_empty() && item.identity.path == decl,
        MatchMode::Substring => item.token.contains(decl),
    }
}

/// Subtract the declared transition `u` from the diff, classifying every line.
#[must_use]
pub fn classify(diff: &ApiDiff, u: &RegimeTransition, mode: MatchMode) -> Vec<Classified> {
    // A rename only transports when BOTH sides are present in the diff: a `from`
    // in removed and a `to` in added. A dangling rename accounts for neither.
    let satisfied = |r: &Rename| -> bool {
        diff.removed
            .iter()
            .any(|x: &ApiItem| matches(x, &r.from, mode))
            && diff.added.iter().any(|x: &ApiItem| matches(x, &r.to, mode))
    };

    let mut out: Vec<Classified> = Vec::new();

    for a in &diff.added {
        let class: Class = if u
            .renames
            .iter()
            .any(|r: &Rename| satisfied(r) && matches(a, &r.to, mode))
        {
            Class::TransportedIso
        } else if let Some(d) = u
            .additive
            .iter()
            .find(|d: &&Additive| matches(a, &d.item, mode))
        {
            Class::DeclaredAdditive(d.adr.clone())
        } else {
            Class::ResidualAdditive
        };
        out.push(Classified {
            item: a.clone(),
            class,
        });
    }

    for rm in &diff.removed {
        let class: Class = if u
            .renames
            .iter()
            .any(|r: &Rename| satisfied(r) && matches(rm, &r.from, mode))
        {
            Class::TransportedIso
        } else if let Some(d) = u
            .removals
            .iter()
            .find(|d: &&Removal| matches(rm, &d.item, mode))
        {
            Class::DeclaredRemoval(d.reason.clone())
        } else {
            Class::ResidualRemoval
        };
        out.push(Classified {
            item: rm.clone(),
            class,
        });
    }

    for ch in &diff.changed {
        let class: Class =
            if let Some(d) = u.changes.iter().find(|d: &&Change| {
                matches(&ch.new, &d.item, mode) || matches(&ch.old, &d.item, mode)
            }) {
                Class::DeclaredChange(d.adr.clone())
            } else {
                Class::ResidualChange
            };
        out.push(Classified {
            item: ch.new.clone(),
            class,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::ApiChange;
    use crate::domain::transition::RegimeKind;

    fn refactor(renames: Vec<Rename>) -> RegimeTransition {
        RegimeTransition {
            kind: RegimeKind::Refactor,
            renames,
            additive: vec![],
            removals: vec![],
            changes: vec![],
        }
    }

    fn empty() -> RegimeTransition {
        RegimeTransition {
            kind: RegimeKind::Transition,
            renames: vec![],
            additive: vec![],
            removals: vec![],
            changes: vec![],
        }
    }

    // ---- zero ----
    #[test]
    fn empty_diff_classifies_to_nothing() {
        let got: Vec<Classified> = classify(&ApiDiff::default(), &empty(), MatchMode::Identity);
        assert!(got.is_empty());
    }

    // ---- one, per bucket ----
    #[test]
    fn undeclared_add_is_residual_additive() {
        let diff: ApiDiff = ApiDiff {
            added: vec![ApiItem::new("pub fn c::brand_new()")],
            ..Default::default()
        };
        let got: Vec<Classified> = classify(&diff, &empty(), MatchMode::Identity);
        assert_eq!(got[0].class, Class::ResidualAdditive);
    }

    #[test]
    fn undeclared_remove_is_residual_removal() {
        let diff: ApiDiff = ApiDiff {
            removed: vec![ApiItem::new("pub fn c::gone()")],
            ..Default::default()
        };
        let got: Vec<Classified> = classify(&diff, &empty(), MatchMode::Identity);
        assert_eq!(got[0].class, Class::ResidualRemoval);
    }

    #[test]
    fn satisfied_rename_transports_both_sides() {
        let diff: ApiDiff = ApiDiff {
            added: vec![ApiItem::new("pub fn c::new_name()")],
            removed: vec![ApiItem::new("pub fn c::old_name()")],
            ..Default::default()
        };
        let u: RegimeTransition = refactor(vec![Rename {
            from: "c::old_name".to_owned(),
            to: "c::new_name".to_owned(),
        }]);
        let got: Vec<Classified> = classify(&diff, &u, MatchMode::Identity);
        assert!(got
            .iter()
            .all(|c: &Classified| c.class == Class::TransportedIso));
    }

    #[test]
    fn declared_additive_carries_its_adr() {
        let diff: ApiDiff = ApiDiff {
            added: vec![ApiItem::new("pub fn c::feature()")],
            ..Default::default()
        };
        let mut u: RegimeTransition = empty();
        u.additive = vec![Additive {
            item: "c::feature".to_owned(),
            adr: "ADR-0007".to_owned(),
        }];
        let got: Vec<Classified> = classify(&diff, &u, MatchMode::Identity);
        assert_eq!(got[0].class, Class::DeclaredAdditive("ADR-0007".to_owned()));
    }

    #[test]
    fn declared_change_carries_its_adr() {
        let diff: ApiDiff = ApiDiff {
            changed: vec![ApiChange {
                old: ApiItem::new("pub fn c::w(u8) -> u8"),
                new: ApiItem::new("pub fn c::w(u64) -> u64"),
            }],
            ..Default::default()
        };
        let mut u: RegimeTransition = empty();
        u.changes = vec![Change {
            item: "c::w".to_owned(),
            adr: "ADR-2".to_owned(),
        }];
        let got: Vec<Classified> = classify(&diff, &u, MatchMode::Identity);
        assert_eq!(got[0].class, Class::DeclaredChange("ADR-2".to_owned()));
    }

    // ---- many / discrimination ----
    #[test]
    fn rename_only_satisfied_when_both_sides_present() {
        // `to` present but no matching `from` removed -> NOT a rename, it's new.
        let diff: ApiDiff = ApiDiff {
            added: vec![ApiItem::new("pub fn c::new_name()")],
            ..Default::default()
        };
        let u: RegimeTransition = refactor(vec![Rename {
            from: "c::old_name".to_owned(),
            to: "c::new_name".to_owned(),
        }]);
        let got: Vec<Classified> = classify(&diff, &u, MatchMode::Identity);
        assert_eq!(got[0].class, Class::ResidualAdditive);
    }

    #[test]
    fn identity_match_is_exact_not_substring() {
        // `c::feature` declared, but item path is `c::feature_flag`: identity
        // mode must NOT match (substring would have falsely accounted it).
        let diff: ApiDiff = ApiDiff {
            added: vec![ApiItem::new("pub fn c::feature_flag()")],
            ..Default::default()
        };
        let mut u: RegimeTransition = empty();
        u.additive = vec![Additive {
            item: "c::feature".to_owned(),
            adr: "ADR-1".to_owned(),
        }];
        let id: Vec<Classified> = classify(&diff, &u, MatchMode::Identity);
        assert_eq!(id[0].class, Class::ResidualAdditive);
        let sub: Vec<Classified> = classify(&diff, &u, MatchMode::Substring);
        assert_eq!(sub[0].class, Class::DeclaredAdditive("ADR-1".to_owned()));
    }
}
