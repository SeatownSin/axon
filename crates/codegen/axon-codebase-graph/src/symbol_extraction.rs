//! Shared core for symbol capture extraction from tree-sitter queries.
//!
//! All symbol extractors in this crate walk the same query capture patterns
//! (`name.definition.*`, `name.reference.*`, `alias.original`, `alias.name`)
//! over a parsed tree. This module performs that walk exactly once: the
//! per-query capture classification and the per-match alias pairing live here
//! instead of being duplicated at each call site.
//!
//! Call sites adapt the canonical [`QueryCaptures`] result into their own
//! output shapes and UTF-8 policies:
//! - `scope_graph_from_definitions_query` (scope_graph) resolves `SymbolId`s
//!   and builds a full `ScopeGraph`
//! - the index builder (manager::builder) converts captures to 1-indexed-line
//!   `SymbolOccurrence`s with lossy UTF-8 conversion
//! - the index manager (index_manager) interns symbols directly into the
//!   index, strictly skipping invalid UTF-8 ranges

use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

use crate::types::Range;

/// Capture-name prefix for symbol definitions (e.g. `name.definition.function`).
const DEFINITION_PREFIX: &str = "name.definition.";
/// Capture-name prefix for symbol references (e.g. `name.reference.call`).
const REFERENCE_PREFIX: &str = "name.reference.";
/// Capture name for the original symbol of an import alias.
const ALIAS_ORIGINAL: &str = "alias.original";
/// Capture name for the imported-as name of an import alias.
const ALIAS_NAME: &str = "alias.name";

/// Classification of a query capture, precomputed once per query.
///
/// Matching is by PREFIX (`name.definition.`), so a capture named
/// `name.definition.method.static` classifies as a definition with symbol
/// `method.static`. This unifies a pre-consolidation difference: the old
/// `scope_graph_from_definitions_query` split on `.` and matched exactly
/// `["name", "definition", sym]`, silently ignoring capture names with more
/// than three parts, while the other extractors used this prefix rule. All 21
/// capture names currently shipped by the language queries have exactly three
/// parts, so no present behaviour changes; a future 4-part capture name will
/// now reach `symbol_id_of` on the scope-graph path instead of being dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    /// A `name.definition.<sym>` capture.
    Definition,
    /// A `name.reference.<sym>` capture.
    Reference,
    /// An `alias.original` capture.
    AliasOriginal,
    /// An `alias.name` capture.
    AliasName,
    /// Any other capture - ignored by symbol extraction.
    Other,
}

impl CaptureKind {
    /// Classify a capture name.
    fn from_capture_name(name: &str) -> Self {
        if name.starts_with(DEFINITION_PREFIX) {
            Self::Definition
        } else if name.starts_with(REFERENCE_PREFIX) {
            Self::Reference
        } else if name == ALIAS_ORIGINAL {
            Self::AliasOriginal
        } else if name == ALIAS_NAME {
            Self::AliasName
        } else {
            Self::Other
        }
    }
}

/// A symbol definition or reference occurrence captured from a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SymbolCapture {
    /// Byte range of the symbol in the source text.
    pub byte_range: std::ops::Range<usize>,
    /// Line/byte range of the symbol in the source text.
    pub range: Range,
    /// The query capture index this symbol came from.
    pub capture_index: u32,
}

/// A matched pair of `alias.original` and `alias.name` captures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AliasCapturePair {
    /// Byte range of the original symbol name in the source text.
    pub original: std::ops::Range<usize>,
    /// Byte range of the imported-as name in the source text.
    pub name: std::ops::Range<usize>,
}

/// Canonical result of walking symbol captures over a query.
///
/// Only positions are recorded; converting byte ranges to text is left to
/// the caller so it can pick its UTF-8 policy (lossy or strict-skip).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueryCaptures {
    /// `name.definition.*` captures, in query match order.
    pub defs: Vec<SymbolCapture>,
    /// `name.reference.*` captures, in query match order.
    pub refs: Vec<SymbolCapture>,
    /// Matched `alias.original`/`alias.name` capture pairs, in match order.
    pub aliases: Vec<AliasCapturePair>,
}

/// Resolve the symbol kind of a definition/reference capture, e.g.
/// `function` for a `name.definition.function` capture.
///
/// Returns `None` for captures that are not symbol definitions/references.
pub(crate) fn symbol_kind_of(query: &Query, capture_index: u32) -> Option<&str> {
    let name = query.capture_names()[capture_index as usize];
    if let Some(symbol) = name.strip_prefix(DEFINITION_PREFIX) {
        Some(symbol)
    } else {
        name.strip_prefix(REFERENCE_PREFIX)
    }
}

/// Walk the symbol captures of `query` over `root_node` and collect them
/// into a canonical [`QueryCaptures`] result.
///
/// Capture indices are classified once up front (O(capture count), not
/// O(match count)) and alias pairing happens per-match, so the walk is as
/// cheap as the per-call-site loops it replaces.
#[must_use]
pub(crate) fn extract_query_captures(
    query: &Query,
    root_node: Node<'_>,
    src: &[u8],
) -> QueryCaptures {
    // Pre-compute capture kinds for fast lookup (avoid string comparisons
    // in the hot loop).
    let kinds: Vec<CaptureKind> = query
        .capture_names()
        .iter()
        .map(|name| CaptureKind::from_capture_name(name))
        .collect();

    let mut captures = QueryCaptures {
        defs: Vec::with_capacity(64),
        refs: Vec::with_capacity(256),
        aliases: Vec::with_capacity(8),
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root_node, src);

    while let Some(match_) = matches.next() {
        let mut alias_original: Option<std::ops::Range<usize>> = None;
        let mut alias_name: Option<std::ops::Range<usize>> = None;

        for capture in match_.captures {
            let node = capture.node;
            let byte_range = node.byte_range();

            match kinds[capture.index as usize] {
                CaptureKind::Definition => captures.defs.push(SymbolCapture {
                    byte_range,
                    range: Range::for_tree_node(&node),
                    capture_index: capture.index,
                }),
                CaptureKind::Reference => captures.refs.push(SymbolCapture {
                    byte_range,
                    range: Range::for_tree_node(&node),
                    capture_index: capture.index,
                }),
                CaptureKind::AliasOriginal => alias_original = Some(byte_range),
                CaptureKind::AliasName => alias_name = Some(byte_range),
                CaptureKind::Other => {}
            }
        }

        // Record the alias pair if both sides are present in this match.
        if let (Some(original), Some(name)) = (alias_original, alias_name) {
            captures.aliases.push(AliasCapturePair { original, name });
        }
    }

    captures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::rust_lang;

    fn parse_rust(src: &str) -> (tree_sitter::Tree, tree_sitter::Query) {
        let config = rust_lang();
        let query = config.compile_query().expect("rust query should compile");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&config.language())
            .expect("set rust language");
        let tree = parser.parse(src, None).expect("parse rust source");
        (tree, query)
    }

    #[test]
    fn captures_definitions_with_symbol_kind() {
        let source = "fn greet(name: String) -> String {\n    name.into()\n}\n";
        let bytes = source.as_bytes().to_vec();
        let (tree, query) = parse_rust(source);

        let captures = extract_query_captures(&query, tree.root_node(), &bytes);

        let greet = captures
            .defs
            .iter()
            .find(|d| &bytes[d.byte_range.clone()] == b"greet")
            .expect("greet definition should be captured");
        assert_eq!(
            symbol_kind_of(&query, greet.capture_index),
            Some("function")
        );
    }

    #[test]
    fn captures_references() {
        let source = "fn greet(name: String) -> String {\n    name.into()\n}\n";
        let bytes = source.as_bytes().to_vec();
        let (tree, query) = parse_rust(source);

        let captures = extract_query_captures(&query, tree.root_node(), &bytes);

        let ref_names: Vec<&[u8]> = captures
            .refs
            .iter()
            .map(|r| &bytes[r.byte_range.clone()])
            .collect();
        assert!(ref_names.iter().any(|n| n == b"String"));
    }

    #[test]
    fn pairs_alias_captures_per_match() {
        let source = "use std::fmt::Write as Writer;\n";
        let bytes = source.as_bytes().to_vec();
        let (tree, query) = parse_rust(source);

        let captures = extract_query_captures(&query, tree.root_node(), &bytes);

        assert_eq!(captures.aliases.len(), 1);
        assert_eq!(&bytes[captures.aliases[0].original.clone()], b"Write");
        assert_eq!(&bytes[captures.aliases[0].name.clone()], b"Writer");
    }

    #[test]
    fn empty_query_yields_no_captures() {
        // The builder/manager fall back to an empty query on compile failure;
        // the walk must produce an empty result for it.
        let config = rust_lang();
        let query =
            tree_sitter::Query::new(&config.language(), "").expect("empty query should compile");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&config.language()).unwrap();
        let source = "fn main() {}\n";
        let tree = parser.parse(source, None).unwrap();

        let captures = extract_query_captures(&query, tree.root_node(), source.as_bytes());

        assert!(captures.defs.is_empty());
        assert!(captures.refs.is_empty());
        assert!(captures.aliases.is_empty());
    }
}
