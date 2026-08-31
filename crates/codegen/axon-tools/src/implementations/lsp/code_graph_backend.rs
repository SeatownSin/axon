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
//!
//! # Staying fresh
//!
//! Two independent paths keep the index current, which matters because a stale
//! index answers a *plausible* wrong location rather than nothing:
//!
//! 1. **Filesystem events.** This backend does not own an index; it holds the
//!    shared per-cwd [`IndexManagerHandle`] from `CodebaseIndexManager`.
//!    `session/fs_watch.rs` and `axon-workspace/src/fs_notify.rs` already
//!    translate watcher events into `FileEvent`s and send them to that same
//!    handle, so edits from any source reindex without this module
//!    participating. Sharing also halves the memory: one 10.7 MB index serves
//!    both this tool and the ACP editor client.
//! 2. **The agent's own search-replace edits.** A watcher only runs for
//!    sessions that need one, so [`LspBackend::notify_file_changed`] sends a
//!    `FileEvent::modified` as well. This covers the case that matters most --
//!    an agent usually asks about code it just changed -- without depending on
//!    a watcher being configured.
//!
//! Note the second path is **narrower than it sounds**:
//! `reminders::lsp_diagnostics` only calls `notify_file_changed` for
//! `SearchReplaceOutput::EditsApplied`, so a whole-file write or a newly
//! created file does not reindex through it. Those depend on path 1, and
//! without a watcher they stay stale until the next rebuild.
//!
//! Both paths are send-and-forget. The actor reindexes on its own thread, so a
//! query issued immediately after a write may still see the previous revision.
//! Every one of these gaps degrades the same way -- the index misses, and the
//! textual scan answers from disk -- which is why the fallback is not optional
//! garnish but the thing that makes staleness safe.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axon_codebase_graph::{FileEvent, IndexManagerHandle, SymbolLocation};

use super::manager::DiagnosticsSummary;
use super::types::{FileDiagnosticEntry, LspBackend, LspOperation, LspToolInput, LspToolResult};

/// Cap on files the textual fallback will read before giving up. The index
/// build itself touches ~2,300 files in under a second, but that is parallel
/// tree-sitter work; this is a sequential substring scan on a fallback path.
const FALLBACK_FILE_LIMIT: usize = 5_000;

/// How many files the fallback lists. Results are grouped by file rather than
/// listed line by line, which is what makes this bound generous enough to be
/// irrelevant in practice: `reasoning_content` has 99 matching lines across 12
/// files, and the widest symbol sampled in this repository touched 18.
const FALLBACK_REPORT_FILES: usize = 60;

/// How many line numbers to show per file before eliding the rest. The count is
/// always reported in full, so eliding never hides the scale of a match.
const FALLBACK_LINES_PER_FILE: usize = 12;

/// Wraps the **shared** [`IndexManagerHandle`] rather than owning an index.
///
/// The handle comes from `CodebaseIndexManager`, the same per-cwd actor the ACP
/// editor client uses, so this backend and `axon/code/*` read one index instead
/// of two copies of the same 10.7 MB. That sharing is also what keeps results
/// fresh: `session/fs_watch.rs` and `axon-workspace/src/fs_notify.rs` already
/// translate filesystem events into `FileEvent`s and `send_event` them to
/// whichever handle that manager holds. Building a private index here would
/// have been simpler and would have gone stale on the first edit.
pub struct CodeGraphBackend {
    root: PathBuf,
    index: Arc<IndexManagerHandle>,
}

impl std::fmt::Debug for CodeGraphBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No file count here on purpose. `get_file_count` ends in
        // `blocking_recv`, and Debug is formatted lazily -- a `tracing` macro
        // in an async fn would evaluate this on a runtime thread and panic.
        // A formatter that can take the process down is not worth one field.
        f.debug_struct("CodeGraphBackend")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl CodeGraphBackend {
    pub fn new(root: PathBuf, index: Arc<IndexManagerHandle>) -> Self {
        Self { root, index }
    }
}

/// What the textual fallback found, grouped by file.
///
/// Grouping is the point, not a formatting nicety. An earlier version listed
/// raw matching lines and truncated at 40 of them; measured against
/// `reasoning_content` -- 99 lines across 12 files -- that cap dropped four of
/// the five files that actually touch the field, while still presenting 40
/// concrete-looking results. Truncation was in directory-walk order, so what
/// survived was an arbitrary prefix rather than the most relevant hits. Files
/// are the unit a "where is this used" question is really asking about, and
/// they are two orders of magnitude fewer.
#[derive(Debug, Default)]
struct ScanResult {
    /// `(relative path, matching line numbers)`, walk order, capped at
    /// [`FALLBACK_REPORT_FILES`] entries.
    files: Vec<(String, Vec<usize>)>,
    /// Every match found, including those in files beyond the report cap.
    total_hits: usize,
    /// Every file containing a match, including those beyond the report cap.
    total_files: usize,
    /// The walk stopped on [`FALLBACK_FILE_LIMIT`], so counts are lower bounds.
    walk_truncated: bool,
}

/// Word-boundary literal scan, used when the index resolves nothing.
///
/// Deliberately dumb: it matches the symbol as a whole word and counts every
/// hit. That is grep's failure mode -- visible noise -- which is the one this
/// module is willing to accept, having measured that a model filters it
/// correctly. What a model cannot recover from is a silently truncated answer,
/// so totals here are always complete even when the listing is not.
fn literal_scan(root: &Path, symbol: &str) -> ScanResult {
    let mut result = ScanResult::default();
    let mut files_read = 0usize;

    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .build();

    for entry in walker.flatten() {
        if files_read >= FALLBACK_FILE_LIMIT {
            result.walk_truncated = true;
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

        let mut lines_here: Vec<usize> = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            if contains_word(line, symbol) {
                lines_here.push(idx + 1);
            }
        }
        if lines_here.is_empty() {
            continue;
        }

        // Counted before the report cap, so the totals stay honest however
        // many files are listed.
        result.total_hits += lines_here.len();
        result.total_files += 1;
        if result.files.len() < FALLBACK_REPORT_FILES {
            let rel = path.strip_prefix(root).unwrap_or(path);
            result
                .files
                .push((rel.to_string_lossy().replace('\\', "/"), lines_here));
        }
    }
    result
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

/// Appended to every non-empty *reference* result.
///
/// References inside a macro invocation are not indexed at all -- tree-sitter
/// parses a macro body as an opaque token tree, so the query that captures
/// `x.foo()` finds nothing inside `assert_eq!(x.foo(), y)`. In this repository
/// that hides 25,278 call sites in assert-family macros alone, plus the bodies
/// of 177 `select!` and 13 `stream!` blocks -- including the whole of
/// `stream_chat_completions`.
///
/// So a non-empty result is a *lower bound*, never a census. Warning only on
/// the empty case (which is what this module did first) is the same mistake in
/// a smaller costume: it treats "5 references" as complete when the true answer
/// might be 5 plus every test that exercises the symbol.
const MACRO_CAVEAT: &str = "\nNote: references inside macro invocations are not indexed \
(tree-sitter does not parse macro bodies), so anything used only inside `assert_eq!`, \
`select!`, `stream!` or similar is missing from this list. Treat the count as a lower \
bound; search textually before concluding a symbol is unused in tests.";

fn render(symbol: &str, label: &str, locations: &[SymbolLocation]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{label} of `{symbol}` ({}):", locations.len());
    for loc in locations {
        let _ = writeln!(out, "  {}:{}", loc.path, loc.line);
    }
    out
}

/// The message that replaces a bare zero.
fn render_unresolved(symbol: &str, scan: &ScanResult) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "The symbol index resolved no references for `{symbol}`. This does NOT mean \
         there are none -- resolution is known to fail for some methods and struct \
         fields, so treat an index miss as no information rather than as evidence \
         that `{symbol}` is unused."
    );
    if scan.total_hits == 0 {
        let _ = writeln!(
            out,
            "A whole-word textual search also found no occurrences{}, which is \
             stronger evidence, but still confirm before treating `{symbol}` as dead.",
            if scan.walk_truncated {
                " in the files searched (the search itself was cut short, so this is not exhaustive)"
            } else {
                ""
            }
        );
        return out;
    }

    let _ = writeln!(
        out,
        "\nFalling back to a whole-word textual search: {} occurrence(s) across {} file(s).",
        scan.total_hits, scan.total_files
    );
    for (path, lines) in &scan.files {
        let shown = lines.len().min(FALLBACK_LINES_PER_FILE);
        let rendered: Vec<String> = lines[..shown].iter().map(usize::to_string).collect();
        let _ = writeln!(
            out,
            "  {path} ({}): {}{}",
            lines.len(),
            rendered.join(", "),
            if lines.len() > shown {
                format!(", +{} more", lines.len() - shown)
            } else {
                String::new()
            }
        );
    }
    // Any elision is stated with its true size, because a listing that looks
    // complete is the failure mode that motivated grouping by file at all.
    if scan.total_files > scan.files.len() {
        let _ = writeln!(
            out,
            "  ... and {} further file(s) not listed.",
            scan.total_files - scan.files.len()
        );
    }
    if scan.walk_truncated {
        let _ = writeln!(
            out,
            "  (The search stopped early, so these counts are lower bounds.)"
        );
    }
    let _ = writeln!(
        out,
        "\nThese are textual matches, not resolved references: they include the \
         definition, comments and same-named symbols on other types."
    );
    out
}

/// Run the textual scan and wrap it, off the runtime worker.
///
/// Every path that would otherwise return an empty result funnels through here,
/// so there is exactly one place that can produce "the index found nothing" and
/// it is incapable of saying so bare.
async fn fallback_result(root: &Path, symbol: String) -> LspToolResult {
    let root = root.to_path_buf();
    let scan_symbol = symbol.clone();
    match tokio::task::spawn_blocking(move || literal_scan(&root, &scan_symbol)).await {
        Ok(scan) => LspToolResult {
            text: render_unresolved(&symbol, &scan),
            is_error: false,
        },
        // Treating a failed scan as an empty scan would be the exact sin this
        // module exists to prevent: `render_unresolved` calls an empty result
        // "stronger evidence" of absence, which would then be a claim derived
        // from a crash rather than from reading the tree.
        Err(e) => {
            tracing::warn!(error = %e, symbol = %symbol, "textual fallback scan failed");
            LspToolResult {
                text: format!(
                    "The symbol index resolved nothing for `{symbol}`, and the textual \
                     fallback search failed to run. This is not evidence that `{symbol}` \
                     is unused -- no search completed. Retry, or search manually."
                ),
                is_error: true,
            }
        }
    }
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
        // Deliberately empty. The actor is spawned by
        // `CodebaseIndexManager::get_or_create` before this backend is built
        // and loads or builds its index on its own thread, so there is nothing
        // here left to start.
        //
        // This used to probe `get_file_count()` for a debug line. That call
        // ends in `blocking_recv` (`index_manager.rs`), and EVERY caller of
        // this method is async -- `agent_ops` at session start, and
        // `reminders::lsp_diagnostics` after each edit -- so it panicked the
        // whole session with "Cannot block the current thread from within a
        // runtime" as soon as `[features] lsp_tools` was enabled without an
        // `lsp.json` to configure real servers. That is the exact
        // configuration the fallback exists to serve, so the no-server path
        // could not be used at all. A liveness probe is not worth a process.
    }

    async fn ensure_ready(&self) -> Result<(), String> {
        // Never fails. An index that is missing, cold or mid-rebuild still
        // yields an answer here, because every query falls through to the
        // textual scan -- a slow correct answer beats a fast unavailable one.
        Ok(())
    }

    fn is_ready(&self) -> bool {
        // Always ready, which is the honest answer for this backend rather
        // than a convenient one: unlike a language server it has no process to
        // start and no handshake to lose. A cold, missing or half-built index
        // still yields a result, because every query falls through to the
        // textual scan -- exactly what `ensure_ready` above documents.
        //
        // Asking the actor instead would mean `blocking_recv` on a caller that
        // may be async, which is the panic described in
        // `ensure_started_background`. A readiness check that can kill the
        // process is worse than no readiness check.
        true
    }

    async fn dispatch(&self, input: &LspToolInput) -> LspToolResult {
        let root = self.root.clone();

        match input.operation {
            LspOperation::Hover
            | LspOperation::GoToImplementation
            | LspOperation::DocumentSymbol => unsupported(&input.operation),

            LspOperation::GoToDefinition | LspOperation::WorkspaceSymbol => {
                let found = match resolve_target(input) {
                    None => return missing_target(),
                    Some(Target::Position { file, row, col }) => {
                        match self.index.goto_definition(file, row, col).await {
                            Ok(Ok(r)) => Some((r.symbol, r.locations)),
                            Ok(Err(e)) => {
                                tracing::debug!(error = ?e, "code-graph goto_definition failed");
                                None
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "code-graph index actor is gone");
                                None
                            }
                        }
                    }
                    Some(Target::Name(name)) => {
                        match self.index.find_definitions(name.clone(), None).await {
                            Ok(locs) => Some((name, locs)),
                            Err(e) => {
                                tracing::warn!(error = %e, "code-graph index actor is gone");
                                None
                            }
                        }
                    }
                };
                match found {
                    Some((symbol, locs)) if !locs.is_empty() => LspToolResult {
                        text: render(&symbol, "Definitions", &locs),
                        is_error: false,
                    },
                    _ => fallback_result(&root, target_symbol(input)).await,
                }
            }

            LspOperation::FindReferences => {
                let found = match resolve_target(input) {
                    None => return missing_target(),
                    Some(Target::Position { file, row, col }) => {
                        match self.index.goto_references(file, row, col, false).await {
                            Ok(Ok(r)) => Some((r.symbol, r.locations)),
                            Ok(Err(e)) => {
                                tracing::debug!(error = ?e, "code-graph goto_references failed");
                                None
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "code-graph index actor is gone");
                                None
                            }
                        }
                    }
                    Some(Target::Name(name)) => {
                        match self.index.find_references(name.clone(), None).await {
                            Ok(locs) => Some((name, locs)),
                            Err(e) => {
                                tracing::warn!(error = %e, "code-graph index actor is gone");
                                None
                            }
                        }
                    }
                };
                match found {
                    Some((symbol, locs)) if !locs.is_empty() => LspToolResult {
                        // A hit list is a lower bound, not a census -- see
                        // MACRO_CAVEAT. Definitions get no such caveat because
                        // a definition is not hidden by being used in a macro.
                        text: render(&symbol, "References", &locs) + MACRO_CAVEAT,
                        is_error: false,
                    },
                    // The case this module exists for: never a bare zero.
                    _ => fallback_result(&root, target_symbol(input)).await,
                }
            }
        }
    }

    async fn drain_diagnostics(&self, _timeout: std::time::Duration) -> Option<DiagnosticsSummary> {
        None // a symbol index produces no diagnostics
    }

    /// Reindex a file the agent just edited.
    ///
    /// The filesystem watchers cover edits from any source, but only when a
    /// watcher is running for the session. This hook covers the agent's own
    /// edits regardless.
    ///
    /// Caveat worth knowing before relying on it: `reminders::lsp_diagnostics`
    /// calls this **only** for `SearchReplaceOutput::EditsApplied`, so a
    /// whole-file write or a newly created file arrives here never. Those need
    /// the watcher.
    ///
    /// Send-only: the actor reindexes on its own thread and a query issued
    /// immediately after may still see the previous revision. Stale entries
    /// surface as a miss, which the textual scan then answers from disk.
    async fn notify_file_changed(&self, path: &Path, _content: &str) {
        if let Err(e) = self
            .index
            .send_event(FileEvent::modified(path.to_path_buf()))
        {
            tracing::debug!(error = %e, path = %path.display(), "code-graph reindex event dropped");
        }
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

    /// A *non-empty* reference list is also incomplete, because macro bodies
    /// are unindexed. Warning only on the empty case would treat "5 references"
    /// as a census when every test using the symbol may be missing.
    #[test]
    fn non_empty_reference_list_is_labelled_a_lower_bound() {
        let locs = vec![
            SymbolLocation::new("src/a.rs", 12),
            SymbolLocation::new("src/b.rs", 40),
        ];
        let msg = render("take_reasoning", "References", &locs) + MACRO_CAVEAT;
        assert!(msg.contains("src/a.rs:12"), "got: {msg}");
        assert!(msg.contains("lower bound"), "got: {msg}");
        assert!(msg.contains("macro invocations"), "got: {msg}");

        // Definitions are exact and must NOT carry the caveat -- a definition
        // is not hidden by being referenced inside a macro.
        let defs = render("take_reasoning", "Definitions", &locs);
        assert!(!defs.contains("lower bound"), "got: {defs}");
    }

    /// An empty reference result must never read as "nothing uses this".
    #[test]
    fn unresolved_message_refuses_to_assert_a_negative() {
        let msg = render_unresolved("take_reasoning", &ScanResult::default());
        assert!(msg.contains("does NOT mean"), "got: {msg}");
        assert!(msg.contains("no information"), "got: {msg}");

        let scan = ScanResult {
            files: vec![("src/a.rs".into(), vec![190])],
            total_hits: 1,
            total_files: 1,
            walk_truncated: false,
        };
        let msg = render_unresolved("take_reasoning", &scan);
        assert!(msg.contains("src/a.rs"), "got: {msg}");
        assert!(msg.contains("190"), "got: {msg}");
        assert!(
            msg.contains("textual matches, not resolved references"),
            "got: {msg}"
        );
    }

    /// The measured defect this grouping replaced: 99 matching lines across 12
    /// files used to be truncated to the first 40 *lines*, which dropped four
    /// of the five files that actually touched the field while still looking
    /// like 40 concrete results. Every file must survive, and the true totals
    /// must be stated.
    #[test]
    fn every_file_survives_and_totals_are_never_understated() {
        // Shape taken from the real `reasoning_content` measurement.
        let files: Vec<(String, Vec<usize>)> = (0..12)
            .map(|i| (format!("src/f{i}.rs"), (1..=8).collect::<Vec<usize>>()))
            .collect();
        let scan = ScanResult {
            total_hits: files.iter().map(|(_, l)| l.len()).sum(),
            total_files: files.len(),
            files,
            walk_truncated: false,
        };
        let msg = render_unresolved("reasoning_content", &scan);
        for i in 0..12 {
            assert!(
                msg.contains(&format!("src/f{i}.rs")),
                "lost file {i}: {msg}"
            );
        }
        assert!(
            msg.contains("96 occurrence(s) across 12 file(s)"),
            "got: {msg}"
        );
        assert!(!msg.contains("further file(s) not listed"), "got: {msg}");
    }

    /// Elision is allowed; silent elision is not.
    #[test]
    fn elision_states_its_own_size() {
        // More lines in one file than are shown.
        let scan = ScanResult {
            files: vec![("src/a.rs".into(), (1..=30).collect())],
            total_hits: 30,
            total_files: 1,
            walk_truncated: false,
        };
        let msg = render_unresolved("x", &scan);
        assert!(msg.contains("src/a.rs (30)"), "got: {msg}");
        assert!(
            msg.contains(&format!(", +{} more", 30 - FALLBACK_LINES_PER_FILE)),
            "got: {msg}"
        );

        // More files than are listed: the count of the remainder is stated.
        let scan = ScanResult {
            files: vec![("src/a.rs".into(), vec![1])],
            total_hits: 70,
            total_files: 70,
            walk_truncated: true,
        };
        let msg = render_unresolved("x", &scan);
        assert!(
            msg.contains("70 occurrence(s) across 70 file(s)"),
            "got: {msg}"
        );
        assert!(msg.contains("69 further file(s) not listed"), "got: {msg}");
        assert!(msg.contains("lower bounds"), "got: {msg}");
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
