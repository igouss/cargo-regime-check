//! Pure structural parse of one `cargo public-api` token line into a resolved
//! [`ApiIdentity`]. No serde, no toml, no I/O — string arithmetic only.
//!
//! `cargo public-api` already did the genuinely hard part: it resolved types,
//! generics, lifetimes and re-exports into one normalized token per item. We
//! only split that token into `(kind, path, signature)` so the functor `u` can
//! match items by their *resolved path* — exactly — instead of by the substring
//! containment the prototype used. The path is the identity; matching on it is
//! the faithful stand-in for the functor until `cargo public-api` grows a
//! structured `--output json` we can consume directly (see `AGENTS.md` roadmap).
//!
//! Token grammar, learned from real `cargo public-api 0.52.0` output:
//! - `pub fn kvstore::Reader::get_async<'a>(&'a self) -> ...`
//! - `pub struct kvstore::DbConfig`
//! - `pub kvstore::StoreError::NotFound`                 (enum variant, no keyword)
//! - `pub kvstore::StoreError::NotFound::path: PathBuf`  (field, no keyword)
//! - `pub trait kvstore::Store: Reader + Writer + ...`   (supertraits)
//! - `pub type c::T::Error = core::convert::Infallible`  (associated type)
//! - `impl core::fmt::Debug for kvstore::Db`             (impl header)

/// The kind of public-API item, taken from the leading keyword the token
/// carries. Enum variants and struct fields carry no keyword, so both land in
/// [`ItemKind::Member`]; an `impl` header is [`ItemKind::Impl`]; anything whose
/// keyword we do not recognize is [`ItemKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Mod,
    Struct,
    Enum,
    Union,
    Trait,
    Fn,
    Type,
    Const,
    Static,
    Macro,
    /// Enum variant or struct field — `cargo public-api` emits these with no
    /// leading keyword (`pub crate::Enum::Variant`).
    Member,
    /// An `impl` header line.
    Impl,
    Other,
}

/// The resolved identity of one public-API item.
///
/// `path` is the `::`-separated item path with every generic-argument list
/// stripped (`core::convert::Into<U>` -> `core::convert::Into`); it is what the
/// declared functor `u` matches on. `signature` is the normalized remainder
/// (fn params + return, field/const type, alias rhs, supertrait bounds) — it is
/// carried for display and future change-direction classification, never for
/// matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiIdentity {
    pub kind: ItemKind,
    pub path: String,
    pub signature: Option<String>,
}

/// Recognized leading keywords, longest/qualified first so `unsafe fn` wins over
/// `fn` and `const fn` wins over the bare `const` item.
const KEYWORDS: &[(&str, ItemKind)] = &[
    ("unsafe extern fn ", ItemKind::Fn),
    ("unsafe fn ", ItemKind::Fn),
    ("async fn ", ItemKind::Fn),
    ("const fn ", ItemKind::Fn),
    ("extern fn ", ItemKind::Fn),
    ("fn ", ItemKind::Fn),
    ("mod ", ItemKind::Mod),
    ("struct ", ItemKind::Struct),
    ("enum ", ItemKind::Enum),
    ("union ", ItemKind::Union),
    ("trait ", ItemKind::Trait),
    ("type ", ItemKind::Type),
    ("const ", ItemKind::Const),
    ("static ", ItemKind::Static),
    ("macro ", ItemKind::Macro),
];

/// Parse one token line into its [`ApiIdentity`]. Total and deterministic: an
/// unrecognized shape still yields an identity whose `path` is the best-effort
/// remainder, so the caller never has to handle a parse failure (it just fails
/// *closed* — an unmatched path stays residual).
#[must_use]
pub fn parse(token: &str) -> ApiIdentity {
    let token: &str = token.trim();

    if let Some(rest) = impl_header(token) {
        return ApiIdentity {
            kind: ItemKind::Impl,
            path: strip_generics(rest),
            signature: None,
        };
    }

    let after_vis: &str = token.strip_prefix("pub ").unwrap_or(token);
    let (kind, after_kind): (ItemKind, &str) = detect_kind(after_vis);

    let (head, signature): (&str, Option<String>) = split_signature(after_kind);
    ApiIdentity {
        kind,
        path: strip_generics(head),
        signature,
    }
}

/// `impl ...` header? Returns the part after the leading `impl`, with the
/// leading generic param list and any `where` clause removed, so what remains is
/// the impl's trait/subject. `None` for non-impl tokens.
fn impl_header(token: &str) -> Option<&str> {
    let rest: &str = token.strip_prefix("impl")?;
    // `impl` must be followed by a space or `<`, not be a prefix of `implements`.
    let rest: &str = match rest.chars().next() {
        Some(' ') => rest.trim_start(),
        Some('<') => skip_balanced(rest).trim_start(),
        _ => return None,
    };
    let rest: &str = match rest.find(" where ") {
        Some(i) => &rest[..i],
        None => rest,
    };
    Some(rest.trim())
}

/// Consume the leading kind keyword. Falls back to [`ItemKind::Member`] (enum
/// variant / struct field, which carry no keyword).
fn detect_kind(s: &str) -> (ItemKind, &str) {
    for (kw, kind) in KEYWORDS {
        if let Some(rest) = s.strip_prefix(kw) {
            return (*kind, rest);
        }
    }
    (ItemKind::Member, s)
}

/// Split `path[<generics>][signature]` at the first depth-0 signature delimiter:
/// `(` (fn params), `: ` (type ascription / supertraits), ` = ` (alias rhs), or
/// ` where `. Depth tracks `<` `(` `[` so delimiters inside generics or params
/// are ignored. `->` is never reached because `(` always comes first for fns.
fn split_signature(s: &str) -> (&str, Option<String>) {
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        if depth == 0 {
            let tail: &str = &s[i..];
            if c == '('
                || tail.starts_with(": ")
                || tail.starts_with(" = ")
                || tail.starts_with(" where ")
            {
                return (s[..i].trim(), Some(tail.trim().to_owned()));
            }
        }
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    (s.trim(), None)
}

/// Remove every balanced `<...>` group from a path, collapsing
/// `core::convert::Into<U>` to `core::convert::Into` and
/// `Trait::method<'a, 'b>` to `Trait::method`.
fn strip_generics(s: &str) -> String {
    let mut out: String = String::with_capacity(s.len());
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_owned()
}

/// Skip a leading balanced `<...>` group, returning the remainder.
fn skip_balanced(s: &str) -> &str {
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &s[i + c.len_utf8()..];
                }
            }
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- zero / degenerate ----
    #[test]
    fn empty_token_yields_empty_path() {
        let id: ApiIdentity = parse("");
        assert_eq!(id.path, "");
        assert_eq!(id.kind, ItemKind::Member);
    }

    // ---- one, per shape ----
    #[test]
    fn plain_fn_path_and_signature() {
        let id: ApiIdentity = parse("pub fn demo::old_name(u8) -> u8");
        assert_eq!(id.kind, ItemKind::Fn);
        assert_eq!(id.path, "demo::old_name");
        assert_eq!(id.signature.as_deref(), Some("(u8) -> u8"));
    }

    #[test]
    fn struct_has_no_signature() {
        let id: ApiIdentity = parse("pub struct demo::Foo");
        assert_eq!(id.kind, ItemKind::Struct);
        assert_eq!(id.path, "demo::Foo");
        assert_eq!(id.signature, None);
    }

    #[test]
    fn struct_generics_are_stripped_from_path() {
        let id: ApiIdentity = parse("pub struct demo::Foo<T>");
        assert_eq!(id.path, "demo::Foo");
    }

    #[test]
    fn enum_variant_has_no_keyword() {
        let id: ApiIdentity = parse("pub kvstore::StoreError::NotFound");
        assert_eq!(id.kind, ItemKind::Member);
        assert_eq!(id.path, "kvstore::StoreError::NotFound");
    }

    #[test]
    fn struct_field_splits_on_type_ascription() {
        let id: ApiIdentity = parse("pub kvstore::DbConfig::path: std::path::PathBuf");
        assert_eq!(id.path, "kvstore::DbConfig::path");
        assert_eq!(id.signature.as_deref(), Some(": std::path::PathBuf"));
    }

    #[test]
    fn trait_supertraits_go_to_signature() {
        let id: ApiIdentity = parse("pub trait kvstore::Store: kvstore::Reader + kvstore::Writer");
        assert_eq!(id.kind, ItemKind::Trait);
        assert_eq!(id.path, "kvstore::Store");
        assert_eq!(
            id.signature.as_deref(),
            Some(": kvstore::Reader + kvstore::Writer")
        );
    }

    #[test]
    fn associated_type_alias_rhs_goes_to_signature() {
        let id: ApiIdentity = parse("pub type c::T::Error = core::convert::Infallible");
        assert_eq!(id.kind, ItemKind::Type);
        assert_eq!(id.path, "c::T::Error");
        assert_eq!(id.signature.as_deref(), Some("= core::convert::Infallible"));
    }

    #[test]
    fn unsafe_fn_keyword_is_recognized() {
        let id: ApiIdentity = parse("pub unsafe fn c::Config::clone_to_uninit(&self, *mut u8)");
        assert_eq!(id.kind, ItemKind::Fn);
        assert_eq!(id.path, "c::Config::clone_to_uninit");
    }

    #[test]
    fn async_trait_desugared_method_strips_lifetime_generics() {
        let token: &str = "pub fn kvstore::Reader::get_async<'life0, 'life1, 'async_trait>(&'life0 self, &'life1 str) -> core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::option::Option<alloc::string::String>> + core::marker::Send + 'async_trait)>> where Self: 'async_trait, 'life0: 'async_trait, 'life1: 'async_trait";
        let id: ApiIdentity = parse(token);
        assert_eq!(id.path, "kvstore::Reader::get_async");
    }

    #[test]
    fn impl_header_keeps_trait_and_subject() {
        let id: ApiIdentity = parse("impl core::fmt::Debug for kvstore::Db");
        assert_eq!(id.kind, ItemKind::Impl);
        assert_eq!(id.path, "core::fmt::Debug for kvstore::Db");
    }

    #[test]
    fn impl_header_drops_param_generics_and_where() {
        let id: ApiIdentity =
            parse("impl<T, U> core::convert::Into<U> for kvstore::StoreError where U: core::convert::From<T>");
        assert_eq!(id.kind, ItemKind::Impl);
        assert_eq!(id.path, "core::convert::Into for kvstore::StoreError");
    }

    // ---- many / discrimination ----
    #[test]
    fn double_colon_is_not_a_type_ascription() {
        // The `::` separators must not be mistaken for the `: ` of a field type.
        let id: ApiIdentity = parse("pub fn a::b::c::deeply::nested()");
        assert_eq!(id.path, "a::b::c::deeply::nested");
        assert_eq!(id.signature.as_deref(), Some("()"));
    }

    #[test]
    fn const_item_vs_const_fn_are_distinguished() {
        let item: ApiIdentity = parse("pub const c::MAX: u8");
        assert_eq!(item.kind, ItemKind::Const);
        assert_eq!(item.path, "c::MAX");
        let func: ApiIdentity = parse("pub const fn c::ctor() -> c::T");
        assert_eq!(func.kind, ItemKind::Fn);
        assert_eq!(func.path, "c::ctor");
    }
}
