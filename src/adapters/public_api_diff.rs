//! Adapter: parse the textual output of `cargo public-api diff` into the
//! framework-free [`ApiDiff`].
//!
//! `cargo public-api` does the genuinely hard part — resolving types, generics,
//! lifetimes, re-exports into one normalized token per item, and matching old to
//! new by identity so its three sections (removed / changed / added) are already
//! identity-diffed. We only bucket its output. Removed/added lines are prefixed
//! `-`/`+`; a changed item is a `-old` immediately followed by its `+new`.
//!
//! `cargo public-api` sometimes emits the *same* impl line twice (once per
//! grouping context); we deduplicate identical tokens within a section so the
//! gate counts each item once.

use std::collections::HashSet;

use crate::domain::diff::{ApiChange, ApiDiff, ApiItem};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Removed,
    Changed,
    Added,
}

/// Parse `cargo public-api diff` text. Tolerant of an optional `+`/`-` prefix on
/// add/remove lines (the section header already fixes the bucket); the `+`/`-`
/// in the *changed* section is load-bearing — it pairs old with new. Identical
/// tokens within a section are deduplicated.
#[must_use]
pub fn parse(text: &str) -> ApiDiff {
    let mut diff: ApiDiff = ApiDiff::default();
    let mut section: Section = Section::None;
    let mut pending_old: Option<ApiItem> = None;
    let mut seen_removed: HashSet<String> = HashSet::new();
    let mut seen_added: HashSet<String> = HashSet::new();
    let mut seen_changed: HashSet<(String, String)> = HashSet::new();

    for raw in text.lines() {
        let line: &str = raw.trim_end();

        if line.starts_with("Removed items") {
            section = Section::Removed;
            pending_old = None;
            continue;
        }
        if line.starts_with("Changed items") {
            section = Section::Changed;
            pending_old = None;
            continue;
        }
        if line.starts_with("Added items") {
            section = Section::Added;
            pending_old = None;
            continue;
        }
        // header underline, blanks, and the literal "(none)" carry no items.
        if line.is_empty() || line == "(none)" || line.chars().all(|c: char| c == '=') {
            continue;
        }

        match section {
            Section::Removed => {
                let token: &str = line.strip_prefix('-').map(str::trim_start).unwrap_or(line);
                if seen_removed.insert(token.to_owned()) {
                    diff.removed.push(ApiItem::new(token));
                }
            }
            Section::Added => {
                let token: &str = line.strip_prefix('+').map(str::trim_start).unwrap_or(line);
                if seen_added.insert(token.to_owned()) {
                    diff.added.push(ApiItem::new(token));
                }
            }
            Section::Changed => {
                if let Some(rest) = line.strip_prefix('-') {
                    pending_old = Some(ApiItem::new(rest.trim_start()));
                } else if let Some(rest) = line.strip_prefix('+') {
                    if let Some(old) = pending_old.take() {
                        let new: ApiItem = ApiItem::new(rest.trim_start());
                        let key: (String, String) = (old.token.clone(), new.token.clone());
                        if seen_changed.insert(key) {
                            diff.changed.push(ApiChange { old, new });
                        }
                    }
                }
            }
            Section::None => {}
        }
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    // REAL output captured from `cargo public-api 0.52.0` (verified 2026-06-18).
    // public-api drops parameter NAMES: the token is `demo::old_name(u8) -> u8`.
    const SAMPLE: &str = "\
Removed items from the public API
=================================
-pub fn demo::doomed()
-pub fn demo::old_name(u8) -> u8

Changed items in the public API
===============================
-pub fn demo::widening(u8) -> u8
+pub fn demo::widening(u64) -> u64

Added items to the public API
=============================
+pub fn demo::brand_new() -> i32
+pub fn demo::new_name(u8) -> u8
";

    #[test]
    fn parses_each_section() {
        let diff: ApiDiff = parse(SAMPLE);
        assert_eq!(diff.removed.len(), 2);
        assert_eq!(diff.added.len(), 2);
        assert_eq!(diff.changed.len(), 1);
    }

    #[test]
    fn changed_pairs_old_with_new() {
        let diff: ApiDiff = parse(SAMPLE);
        assert!(diff.changed[0].old.token.contains("u8"));
        assert!(diff.changed[0].new.token.contains("u64"));
    }

    #[test]
    fn none_sections_yield_no_items() {
        let text: &str = "Removed items from the public API\n=====\n(none)\n";
        let diff: ApiDiff = parse(text);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn duplicate_impl_lines_are_deduplicated() {
        // public-api emits the same impl line twice; the gate must see it once.
        let text: &str = "\
Added items to the public API
=============================
+impl kvstore::Reader for kvstore::Db
+impl kvstore::Reader for kvstore::Db
";
        let diff: ApiDiff = parse(text);
        assert_eq!(diff.added.len(), 1);
    }
}
