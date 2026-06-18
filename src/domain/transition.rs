//! The declared transition `u`: the functor the author asserts maps the old
//! public API onto the new one, plus the kind of claim being made. Pure data —
//! the TOML that produces it lives in the `regime_file` adapter.

/// The claim the author is making about a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegimeKind {
    /// Endofunctorial: the surface may only be renamed/moved (residual must be 0).
    Refactor,
    /// A genuine schema change whose residual is allowed but must be declared.
    Transition,
}

impl RegimeKind {
    /// The stable lowercase name used in TOML and JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RegimeKind::Refactor => "refactor",
            RegimeKind::Transition => "transition",
        }
    }
}

/// `u`, declared: a rename/move of one item to another (the iso part of `u`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

/// Declared discovery: an added item justified by an ADR reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Additive {
    pub item: String,
    pub adr: String,
}

/// Declared, acknowledged removal (a breaking change the author owns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub item: String,
    pub reason: String,
}

/// Declared, justified signature change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub item: String,
    pub adr: String,
}

/// The whole declared transition `u` plus the kind of claim being made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegimeTransition {
    pub kind: RegimeKind,
    pub renames: Vec<Rename>,
    pub additive: Vec<Additive>,
    pub removals: Vec<Removal>,
    pub changes: Vec<Change>,
}
