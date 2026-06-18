//! Adapter for `regime-transition.toml` (the declared functor `u`). It is the
//! single owner of the TOML format: it PARSES the file into the framework-free
//! [`RegimeTransition`], RENDERS the minimal template for the
//! `--template`/error paths, and RENDERS the remediation snippet + directive for
//! each [`RequiredAction`] the gate emits. serde/toml live here, not in the
//! domain.

use serde::Deserialize;

use crate::domain::gate::RequiredAction;
use crate::domain::transition::{Additive, Change, RegimeKind, RegimeTransition, Removal, Rename};

#[derive(Debug, thiserror::Error)]
pub enum RegimeFileError {
    #[error("could not parse regime-transition.toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("meta.kind must be `refactor` or `transition`, got `{0}`")]
    BadKind(String),
}

#[derive(Debug, Deserialize)]
struct RegimeFile {
    meta: Meta,
    #[serde(default)]
    rename: Vec<RenameEntry>,
    #[serde(default)]
    additive: Vec<AdditiveEntry>,
    #[serde(default)]
    removal: Vec<RemovalEntry>,
    #[serde(default)]
    change: Vec<ChangeEntry>,
}

#[derive(Debug, Deserialize)]
struct Meta {
    kind: String,
}

#[derive(Debug, Deserialize)]
struct RenameEntry {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
struct AdditiveEntry {
    item: String,
    adr: String,
}

#[derive(Debug, Deserialize)]
struct RemovalEntry {
    item: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct ChangeEntry {
    item: String,
    adr: String,
}

/// Parse the declared transition from TOML text.
pub fn parse(text: &str) -> Result<RegimeTransition, RegimeFileError> {
    let file: RegimeFile = toml::from_str(text)?;
    let kind: RegimeKind = match file.meta.kind.as_str() {
        "refactor" => RegimeKind::Refactor,
        "transition" => RegimeKind::Transition,
        other => return Err(RegimeFileError::BadKind(other.to_owned())),
    };
    Ok(RegimeTransition {
        kind,
        renames: file
            .rename
            .into_iter()
            .map(|r: RenameEntry| Rename {
                from: r.from,
                to: r.to,
            })
            .collect(),
        additive: file
            .additive
            .into_iter()
            .map(|a: AdditiveEntry| Additive {
                item: a.item,
                adr: a.adr,
            })
            .collect(),
        removals: file
            .removal
            .into_iter()
            .map(|r: RemovalEntry| Removal {
                item: r.item,
                reason: r.reason,
            })
            .collect(),
        changes: file
            .change
            .into_iter()
            .map(|c: ChangeEntry| Change {
                item: c.item,
                adr: c.adr,
            })
            .collect(),
    })
}

/// A minimal valid `regime-transition.toml`, emitted by `--template` and printed
/// when the file is missing or malformed so an agent can drop it in and edit.
#[must_use]
pub fn template() -> String {
    r#"# regime-transition.toml — the declared functor `u` for this change.
#
# meta.kind = "refactor"   -> the public API may only be RENAMED/MOVED; any
#                             added/removed/changed surface FAILS the gate.
# meta.kind = "transition" -> added/removed/changed surface is allowed, but each
#                             item below must be declared.
#
# Place this file at the crate root (next to Cargo.toml) and point the gate at it
# with `--regime regime-transition.toml`.
[meta]
kind = "transition"

# Renames/moves (the iso part of `u`). Honoured only when BOTH the old item is
# removed AND the new item is added in the diff.
# [[rename]]
# from = "mycrate::OldName"
# to   = "mycrate::NewName"

# Intentional new public items (declared discovery).
# [[additive]]
# item = "mycrate::new_thing"
# adr  = "ADR-0001"

# Intentional removals (breaking; bump the major version).
# [[removal]]
# item   = "mycrate::gone"
# reason = "unused since v1"

# Intentional signature changes.
# [[change]]
# item = "mycrate::widened"
# adr  = "ADR-0002"
"#
    .to_owned()
}

/// The human/agent directive for one [`RequiredAction`]: what to DO, with both
/// the intentional and the accidental branch spelled out. stdout alone resolves
/// a FAIL because this plus [`snippet`] say exactly how.
#[must_use]
pub fn directive(action: RequiredAction) -> &'static str {
    match action {
        RequiredAction::DeclareAdditive => {
            "undeclared added surface. If intentional: append the [[additive]] block below and \
             ensure meta.kind = \"transition\". If accidental: make the item pub(crate) or remove it."
        }
        RequiredAction::DeclareRemoval => {
            "undeclared removal (breaking). If intentional: append the [[removal]] block below and \
             bump the crate's major version. If accidental: restore the item."
        }
        RequiredAction::DeclareChange => {
            "undeclared signature change. If intentional: append the [[change]] block below with an \
             ADR reference. If accidental: revert the signature."
        }
        RequiredAction::ReclassifyAsTransition => {
            "marked `refactor` but this line adds/removes/changes public surface. A refactor must be \
             endofunctorial (residual 0). Set meta.kind = \"transition\"."
        }
    }
}

/// The copy-pasteable `regime-transition.toml` snippet that, appended verbatim,
/// makes `path` accounted-for. The array-of-tables forms append cleanly to the
/// end of the file; the reclassify form is an edit to `[meta]`, shown as such.
#[must_use]
pub fn snippet(action: RequiredAction, path: &str) -> String {
    let path: String = escape(path);
    match action {
        RequiredAction::DeclareAdditive => format!(
            "[[additive]]\nitem = \"{path}\"\nadr  = \"ADR-XXXX\"  # replace with the ADR/issue justifying this new item\n"
        ),
        RequiredAction::DeclareRemoval => format!(
            "[[removal]]\nitem   = \"{path}\"\nreason = \"REPLACE: why this item is gone (breaking change — bump the major version)\"\n"
        ),
        RequiredAction::DeclareChange => format!(
            "[[change]]\nitem = \"{path}\"\nadr  = \"ADR-XXXX\"  # replace with the ADR justifying the new signature\n"
        ),
        RequiredAction::ReclassifyAsTransition => {
            "# Edit the existing [meta] table — do not append a second one:\n[meta]\nkind = \"transition\"\n"
                .to_owned()
        }
    }
}

/// Escape a TOML basic-string value (paths can contain `"` only in exotic cases,
/// but quote defensively so the snippet always parses).
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse: one / many ----
    #[test]
    fn parses_kind_and_entries() {
        let text: &str = r#"
[meta]
kind = "transition"

[[rename]]
from = "c::Old"
to = "c::New"

[[additive]]
item = "c::T::method"
adr = "ADR-0007"
"#;
        let u: RegimeTransition = parse(text).unwrap();
        assert_eq!(u.kind, RegimeKind::Transition);
        assert_eq!(u.renames.len(), 1);
        assert_eq!(u.additive[0].adr, "ADR-0007");
    }

    #[test]
    fn minimal_file_has_empty_lists() {
        let u: RegimeTransition = parse("[meta]\nkind = \"refactor\"\n").unwrap();
        assert_eq!(u.kind, RegimeKind::Refactor);
        assert!(u.renames.is_empty() && u.additive.is_empty());
    }

    #[test]
    fn rejects_unknown_kind() {
        let err: RegimeFileError = parse("[meta]\nkind = \"whatever\"\n").unwrap_err();
        assert!(matches!(err, RegimeFileError::BadKind(_)));
    }

    // ---- template / snippet round-trips back through parse ----
    #[test]
    fn template_is_valid_and_parses() {
        let u: RegimeTransition = parse(&template()).unwrap();
        assert_eq!(u.kind, RegimeKind::Transition);
    }

    #[test]
    fn additive_snippet_appended_to_template_parses_and_declares_path() {
        let mut doc: String = template();
        doc.push('\n');
        doc.push_str(&snippet(RequiredAction::DeclareAdditive, "c::new_thing"));
        let u: RegimeTransition = parse(&doc).unwrap();
        assert!(u
            .additive
            .iter()
            .any(|a: &Additive| a.item == "c::new_thing"));
    }

    #[test]
    fn every_appendable_snippet_parses() {
        for (action, present) in [
            (RequiredAction::DeclareAdditive, "additive"),
            (RequiredAction::DeclareRemoval, "removal"),
            (RequiredAction::DeclareChange, "change"),
        ] {
            let mut doc: String = template();
            doc.push('\n');
            doc.push_str(&snippet(action, "c::x"));
            let u: RegimeTransition = parse(&doc).unwrap();
            let declared: bool = match present {
                "additive" => u.additive.iter().any(|e: &Additive| e.item == "c::x"),
                "removal" => u.removals.iter().any(|e: &Removal| e.item == "c::x"),
                _ => u.changes.iter().any(|e: &Change| e.item == "c::x"),
            };
            assert!(declared, "{present} snippet did not declare the path");
        }
    }
}
