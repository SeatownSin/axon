//! A tree-sitter [`LspBackend`] for when no language server is configured.
//!
//! `axon-codebase-graph` already indexes the whole repository for the ACP
//! editor client (`axon/code/*` extension methods). It needs no external
//! process and no configuration: 2,276 files, 102,580 definitions and 872,955
//! references index in **954 ms** cold, with a 66 ms warm query. Without this
//! backend, a workspace with no `lsp.json` gets `tool_ctx.lsp = None`, the
//! `lsp` tool answers "LSP tool is unavailable", and the agent falls back to
//! grep for every navigation question.
//!
//! # What this backend will and will not answer
//!
//! Measured against this repository on 2026-07-27 (see
//! `.axon/ideas/code-graph-investigation.md`):
//!
//! * **Definitions are exact.** Every symbol tested resolved to the correct
//!   single definition, by position and by name.
//! * **References are exact when they resolve, and silently empty when they do
//!   not.** `log_code_nav_timing` returned exactly its 4 call sites (grep's 5th
//!   hit is the definition itself) and `set_live` exactly its 3. But
//!   `take_reasoning` (a method with 9 call sites) and `reasoning_content` (a
//!   struct field with 96) both returned **zero**, while `set_live`,
//!   `reasoning_items`, `goto_definition` and `observe` resolved fine. The
//!   failure is selective and gives no signal that it happened.
//!
//! The index is not at fault: a position-based go-to-definition *from*
//! `take_reasoning`'s call site resolves correctly to its definition. Forward
//! resolution works; the reverse by-name lookup fails to surface the same edge.
//!
//! # Why an empty reference result is never returned bare
//!
//! Handed `References (0):` for `take_reasoning`, a local model concluded it
//! "is not called anywhere in the codebase. It appears to be dead code." The
//! function ships in v0.3.3 and is called from the streaming loop. Given grep
//! output for the same symbol it answered correctly.
//!
//! That asymmetry is the whole design constraint here. Grep fails as *noise*,
//! which a model filters and which costs only tokens. This index fails as
//! *silence*, which reads as authoritative and costs correctness. So an empty
//! reference set is never reported as a fact: [`literal_scan`] re-runs the
//! question as a textual search and those hits are returned instead, labelled
//! as unresolved. The tool may be imprecise, but it must not assert a negative
//! it has not established.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axon_codebase_graph::{
    IndexBuilder, Location, Navigator, ScopeGraphIndex, get_cache_path, load_index, save_index,
};
use tokio::sync::OnceCell;

use super::manager::DiagnosticsSummary;
use super::types::{FileDiagnosticEntry, LspBackend, LspOperation, LspToolInput, LspToolResult};

/// Cap on hits returned by the textual fallback. A symbol with hundreds of
/// textual matches is a question grep should answer directly, not something to
/// paste wholesale into a model's context.
const FALLBACK_HIT_LIMIT: usize = 40;

/// Cap on files the textual fallback will read before giving up. The index
/// build itself touches ~2,300 files in under a second, but that is parallel
/// tree-sitter work; this is a sequential substring scan on a fallback path.
const FALLBACK_FILE_LIMIT: usize = 5_000;

pub struct CodeGraphBackend {
    root: PathBuf,
    navigator: Arc<OnceCell<Option<Arc<Navigator>>>>,
}

impl std::fmt::Debug for CodeGraphBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeGraphBackend")
            .field("root", &self.root)
            .field("indexed", &self.navigator.initialized())
            .finish()
    }
}

impl CodeGraphBackend {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            navigator: Arc::new(OnceCell::new()),
        }
    }

    /// Build or load the index, once. Returns `None` if indexing failed, in
    /// which case every operation degrades to the textual fallback rather than
    /// to an error -- a slow correct answer beats a fast unavailable one.
    async fn navigator(&self) -> Option<Arc<Navigator>> {
        let root = self.root.clone();
        self.navigator
            .get_or_init(|| async move {
                // Indexing is CPU-bound rayon work and must not run on a
                // runtime worker.
                tokio::task::spawn_blocking(move || build_navigator(&root))
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "code-graph index task panicked");
                        None
                    })
            })
            .await
            .clone()
    }
}

fn build_navigator(root: &Path) -> Option<Arc<Navigator>> {
    let cache_path = get_cache_path(root);
    let index: ScopeGraphIndex = match load_index(&cache_path) {
        Ok(index) => index,
        Err(_) => {
            let started = std::time::Instant::now();
            let index = match IndexBuilder::new().build(root) {
                Ok(index) => index,
                Err(e) => {
                    tracing::warn!(error = %e, root = %root.display(), "code-graph index build failed");
                    return None;
                }
            };
            if let Err(e) = save_index(&cache_path, &index) {
                // A cache we could not write costs a rebuild next session, not
                // correctness.
                tracing::debug!(error = %e, "code-graph index cache write failed");
            }
            tracing::info!(
                root = %root.display(),
                elapsed_ms = started.elapsed().as_millis(),
                "code-graph index built"
            );
            index
        }
    };
    Some(Arc::new(Navigator::new(index)))
}

/// Word-boundary literal scan, used when the index resolves nothing.
///
/// Deliberately dumb: it matches the symbol as a whole word and reports every
/// hit. That is grep's failure mode -- visible noise -- which is the one this
/// module is willing to accept.
fn literal_scan(root: &Path, symbol: &str) -> Vec<Location> {
    let mut hits = Vec::new();
    let mut files_read = 0usize;

    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .build();

    for entry in walker.flatten() {
        if hits.len() >= FALLBACK_HIT_LIMIT || files_read >= FALLBACK_FILE_LIMIT {
            break;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(path) else {
            continue; // binary or unreadable; not an error worth surfacing
        };
        files_read += 1;
        for (idx, line) in text.lines().enumerate() {
            if !contains_word(line, symbol) {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(path);
            hits.push(Location::new(
                rel.to_string_lossy().replace('\\', "/"),
                idx + 1,
            ));
            if hits.len() >= FALLBACK_HIT_LIMIT {
                break;
            }
        }
    }
    hits
}

/// Whole-word containment, so `reasoning` does not match `reasoning_content`.
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let mut from = 0usize;
    while let Some(pos) = haystack[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + needle.len();
        if from >= haystack.len() {
            break;
        }
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn render(symbol: &str, label: &str, locations: &[Location]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{label} of `{symbol}` ({}):", locations.len());
    for loc in locations {
        let _ = writeln!(out, "  {}:{}", loc.path, loc.line);
    }
    out
}

/// The message that replaces a bare zero.
fn render_unresolved(symbol: &str, hits: &[Location]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "The symbol index resolved no references for `{symbol}`. This does NOT mean \
         there are none -- resolution is known to fail for some methods and struct \
         fields, so treat an index miss as no information rather than as evidence \
         that `{symbol}` is unused."
    );
    if hits.is_empty() {
        let _ = writeln!(
            out,
            "A whole-word textual search also found no occurrences, which is \
             stronger evidence, but still confirm before treating `{symbol}` as dead."
        );
        return out;
    }
    let _ = writeln!(
        out,
        "\nFalling back to a whole-word textual search, which found {} occurrence(s){}:",
        hits.len(),
        if hits.len() >= FALLBACK_HIT_LIMIT {
            " (truncated)"
        } else {
            ""
        }
    );
    for loc in hits {
        let _ = writeln!(out, "  {}:{}", loc.path, loc.line);
    }
    let _ = writeln!(
        out,
        "\nThese are textual matches, not resolved references: they include the \
         definition, comments and same-named symbols on other types."
    );
    out
}

fn unsupported(op: &LspOperation) -> LspToolResult {
    LspToolResult {
        text: format!(
            "`{op}` needs a language server, and none is configured for this workspace. \
             Definitions and references are being served by the built-in symbol index, \
             which does not compute types, documentation or trait implementations. \
             Configure ~/.axon/lsp.json or <cwd>/.axon/lsp.json for full code intelligence."
        ),
        is_error: true,
    }
}

fn missing_target() -> LspToolResult {
    LspToolResult {
        text: "Provide either `query` (symbol name) or `file_path` + `line` + `character` \
               (0-indexed position)."
            .to_string(),
        is_error: true,
    }
}

#[async_trait::async_trait]
impl LspBackend for CodeGraphBackend {
    fn ensure_started_background(&self) {
        // Warm the index off the hot path so the first navigation call does not
        // pay the ~1s build.
        let root = self.root.clone();
        let cell = Arc::clone(&self.navigator);
        tokio::spawn(async move {
            cell.get_or_init(|| async move {
                tokio::task::spawn_blocking(move || build_navigator(&root))
                    .await
                    .unwrap_or(None)
            })
            .await;
        });
    }

    async fn ensure_ready(&self) -> Result<(), String> {
        // Never fails: with no index the backend still answers textually.
        let _ = self.navigator().await;
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.navigator.initialized()
    }

    async fn dispatch(&self, input: &LspToolInput) -> LspToolResult {
        let nav = self.navigator().await;
        let root = self.root.clone();

        match input.operation {
            LspOperation::Hover
            | LspOperation::GoToImplementation
            | LspOperation::DocumentSymbol => unsupported(&input.operation),

            LspOperation::GoToDefinition | LspOperation::WorkspaceSymbol => {
                let result = match (&nav, resolve_target(input)) {
                    (Some(nav), Some(Target::Position { file, row, col })) => {
                        match nav.goto_definition(&file, row, col) {
                            Ok(r) => Some(r),
                            Err(e) => {
                                tracing::debug!(error = ?e, "code-graph goto_definition failed");
                                None
                            }
                        }
                    }
                    (Some(nav), Some(Target::Name(name))) => {
                        Some(nav.goto_definition_by_name(&name, None))
                    }
                    (_, None) => return missing_target(),
                    (None, _) => None,
                };
                match result {
                    Some(r) if !r.locations.is_empty() => LspToolResult {
                        text: render(&r.symbol, "Definitions", &r.locations),
                        is_error: false,
                    },
                    _ => {
                        let symbol = target_symbol(input);
                        let hits =
                            tokio::task::spawn_blocking(move || literal_scan(&root, &symbol))
                                .await
                                .unwrap_or_default();
                        LspToolResult {
                            text: render_unresolved(&target_symbol(input), &hits),
                            is_error: false,
                        }
                    }
                }
            }

            LspOperation::FindReferences => {
                let result = match (&nav, resolve_target(input)) {
                    (Some(nav), Some(Target::Position { file, row, col })) => {
                        match nav.goto_references(&file, row, col, false) {
                            Ok(r) => Some(r),
                            Err(e) => {
                                tracing::debug!(error = ?e, "code-graph goto_references failed");
                                None
                            }
                        }
                    }
                    (Some(nav), Some(Target::Name(name))) => {
                        Some(nav.goto_references_by_name(&name, None, false))
                    }
                    (_, None) => return missing_target(),
                    (None, _) => None,
                };
                match result {
                    Some(r) if !r.locations.is_empty() => LspToolResult {
                        text: render(&r.symbol, "References", &r.locations),
                        is_error: false,
                    },
                    // The case this module exists for: never a bare zero.
                    _ => {
                        let symbol = target_symbol(input);
                        let scan_symbol = symbol.clone();
                        let hits =
                            tokio::task::spawn_blocking(move || literal_scan(&root, &scan_symbol))
                                .await
                                .unwrap_or_default();
                        LspToolResult {
                            text: render_unresolved(&symbol, &hits),
                            is_error: false,
                        }
                    }
                }
            }
        }
    }

    async fn drain_diagnostics(&self, _timeout: std::time::Duration) -> Option<DiagnosticsSummary> {
        None // a symbol index produces no diagnostics
    }

    async fn notify_file_changed(&self, _path: &Path, _content: &str) {
        // The index supports incremental reindexing from file events, but
        // wiring that here would duplicate the watcher axon-workspace already
        // runs. Until this backend subscribes to it, results are as fresh as
        // the last build; stale entries surface as an index miss, which the
        // textual fallback then answers correctly.
    }

    async fn read_diagnostics(&self, _paths: &[PathBuf]) -> Vec<FileDiagnosticEntry> {
        Vec::new()
    }
}

enum Target {
    Position {
        file: PathBuf,
        row: usize,
        col: usize,
    },
    Name(String),
}

/// Position wins when fully specified: it is the direction that resolves
/// reliably, including for the methods the by-name lookup misses.
fn resolve_target(input: &LspToolInput) -> Option<Target> {
    if let (Some(file), Some(line), Some(character)) =
        (input.file_path.as_ref(), input.line, input.character)
    {
        return Some(Target::Position {
            file: PathBuf::from(file),
            // The tool speaks 0-indexed positions; the navigator is 1-indexed.
            row: line as usize + 1,
            col: character as usize + 1,
        });
    }
    input
        .query
        .as_ref()
        .filter(|q| !q.trim().is_empty())
        .map(|q| Target::Name(q.trim().to_string()))
}

/// Best-effort symbol name for the fallback scan and for error text.
fn target_symbol(input: &LspToolInput) -> String {
    input.query.clone().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_word_respects_identifier_boundaries() {
        assert!(contains_word("let x = take_reasoning();", "take_reasoning"));
        assert!(contains_word("take_reasoning", "take_reasoning"));
        // The bug this guards: a prefix must not match a longer identifier.
        assert!(!contains_word(
            "pub reasoning_content: Option<String>",
            "reasoning"
        ));
        assert!(!contains_word(
            "fn take_reasoning_inner()",
            "take_reasoning"
        ));
        assert!(contains_word("delta.take_reasoning()", "take_reasoning"));
    }

    #[test]
    fn contains_word_handles_repeats_without_looping() {
        assert!(contains_word("a.b(); x_b_y; b", "b"));
        assert!(!contains_word("abc abc abc", "b"));
    }

    /// An empty reference result must never read as "nothing uses this".
    #[test]
    fn unresolved_message_refuses_to_assert_a_negative() {
        let msg = render_unresolved("take_reasoning", &[]);
        assert!(msg.contains("does NOT mean"), "got: {msg}");
        assert!(msg.contains("no information"), "got: {msg}");

        let hits = vec![Location::new("src/a.rs", 190)];
        let msg = render_unresolved("take_reasoning", &hits);
        assert!(msg.contains("src/a.rs:190"), "got: {msg}");
        assert!(
            msg.contains("textual matches, not resolved references"),
            "got: {msg}"
        );
    }

    #[test]
    fn position_beats_name_because_it_resolves_more_reliably() {
        let input = LspToolInput {
            operation: LspOperation::GoToDefinition,
            file_path: Some("src/main.rs".into()),
            line: Some(9),
            character: Some(14),
            query: Some("ignored".into()),
        };
        match resolve_target(&input) {
            Some(Target::Position { row, col, .. }) => {
                // 0-indexed in, 1-indexed out.
                assert_eq!((row, col), (10, 15));
            }
            _ => panic!("expected a position target"),
        }
    }

    #[test]
    fn name_target_used_when_position_incomplete() {
        let input = LspToolInput {
            operation: LspOperation::WorkspaceSymbol,
            file_path: Some("src/main.rs".into()),
            line: Some(9),
            character: None,
            query: Some("  ChatChunkDelta  ".into()),
        };
        match resolve_target(&input) {
            Some(Target::Name(n)) => assert_eq!(n, "ChatChunkDelta"),
            _ => panic!("expected a name target"),
        }
    }

    #[test]
    fn no_target_is_an_explicit_error_not_an_empty_answer() {
        let input = LspToolInput {
            operation: LspOperation::FindReferences,
            file_path: None,
            line: None,
            character: None,
            query: Some("   ".into()),
        };
        assert!(resolve_target(&input).is_none());
    }
}
