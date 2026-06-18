//! The before→after public-API diff — the raw material the gate works on.
//! Each [`ApiItem`] carries the [`ApiIdentity`] parsed from its token, so the
//! classifier matches the declared functor `u` by resolved path, not substring.

use crate::domain::identity::{self, ApiIdentity};

/// One public-API item: the normalized token line `cargo public-api` emits
/// (e.g. `pub fn mycrate::foo() -> bool`) plus its parsed [`ApiIdentity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiItem {
    pub token: String,
    pub identity: ApiIdentity,
}

impl ApiItem {
    /// Build an item from its token, parsing the identity once up front.
    pub fn new(token: impl Into<String>) -> Self {
        let token: String = token.into();
        let identity: ApiIdentity = identity::parse(&token);
        Self { token, identity }
    }

    /// The resolved item path — the thing the functor `u` matches on.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.identity.path
    }
}

/// A same-path signature change: the old token and the new token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiChange {
    pub old: ApiItem,
    pub new: ApiItem,
}

/// The before→after public-API diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiDiff {
    pub added: Vec<ApiItem>,
    pub removed: Vec<ApiItem>,
    pub changed: Vec<ApiChange>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_parses_its_identity_path() {
        let item: ApiItem = ApiItem::new("pub fn demo::foo() -> bool");
        assert_eq!(item.path(), "demo::foo");
    }

    #[test]
    fn default_diff_is_empty() {
        let diff: ApiDiff = ApiDiff::default();
        assert!(diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty());
    }
}
