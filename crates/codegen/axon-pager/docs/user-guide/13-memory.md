# Cross-Session Memory

Memory lets Axon recall facts, decisions, and patterns from earlier sessions. Axon indexes the information you save and searches it automatically, so a new session can reuse relevant context.

---

## What Is Memory?

Without memory, each Axon session starts fresh: the model knows nothing about previous sessions. When you enable memory, Axon can:

- Recall project conventions you explained before.
- Reuse debugging steps that worked.
- Carry architectural decisions forward across sessions.
- Avoid re-asking questions it already has answers to.

Memory is experimental and disabled by default.

---

## Enabling Memory

### Per-Session Flag

```bash
axon --experimental-memory
```

### Environment Variable

```bash
export AXON_MEMORY=1
axon
```

### Config File (Persistent)

```toml
# ~/.axon/config.toml
[memory]
enabled = true
```

### Force-Disable

To disable memory even when other settings enable it:

```bash
axon --no-memory
```

Or:

```bash
export AXON_MEMORY=0
```

The `--no-memory` flag has absolute highest priority and always disables memory.

### Mid-Session Toggle

> **This is not a fourth way to enable memory.** `/memory on` only works in a
> session that *already had memory configured at startup* by one of the three
> methods above. In a session started without memory there is no memory backend
> to switch on, and the `/memory` command is not offered at all -- it is hidden
> from the command list rather than failing with a message. **Enabling memory in
> a running session that started without it requires a restart.**

Within a session that started with memory enabled, you can toggle it off and
back on without restarting:

```
/memory on
/memory off
```

The toggle is session-scoped -- it does not persist to `config.toml`. Toggling
off unregisters the `memory_search` and `memory_get` tools and drops the storage
handle, but keeps every file on disk. Toggling on re-initializes storage from
the session's existing backend configuration and re-registers the tools.

You can also toggle from inside the `/memory` modal by pressing `t`.

### Priority Order

1. `--no-memory` CLI flag (always disables)
2. `--experimental-memory` CLI flag (enables)
3. `AXON_MEMORY` env var: `1`/`true` enables, `0`/`false` disables
4. `[memory]` section in config.toml
5. Default: disabled

This order is resolved **once, at startup**. If it resolves to disabled, the
session is built with no memory configuration at all, which is why the
mid-session toggle has nothing to re-enable.

---

## How Memory Is Stored

Memory is stored as Markdown files under `~/.axon/memory/`:

| Location | Scope | Description |
|----------|-------|-------------|
| `~/.axon/memory/MEMORY.md` | Global | Facts that apply across all your projects |
| `~/.axon/memory/<project-slug>-<hash8>/MEMORY.md` | Workspace | Project-specific conventions and context |
| `~/.axon/memory/<project-slug>-<hash8>/sessions/` | Sessions | Per-session summaries and logs |

Axon suffixes each workspace directory with a short hash of the repository's identity. The identity is the `origin` remote in `org/repo` form when the directory is a Git repository with an `origin` remote, or the directory path otherwise. Because clones and worktrees of the same repository share an `origin` remote, they also share one memory directory.

An SQLite index supports hybrid search across all memory files:
- **FTS5** provides full-text search for keyword matching.
- **vec0** provides vector search for semantic similarity. Vector search is optional and requires an embedding.

---

## Automatic Saves

When a session ends, Axon saves a structured metadata summary to that session's daily log. The summary contains:

- Message counts (user, assistant, and tool results).
- Topics: the first few substantive user prompts from the session, up to five.
- The session date and time (UTC).

Axon builds the summary from conversation metadata without an LLM call, without added latency. Axon skips the save for trivial sessions -- those with fewer than three substantive prompts, or fewer than 50 bytes of user text.

The summary does not record tool usage, file paths, or shell commands. The session ID forms part of the log filename. To turn automatic saves off, set `session.save_on_end = false`. For richer capture of decisions, patterns, and reasoning, use `/flush`.

---

## Saving Rich Knowledge with /flush

For richer capture -- decisions, patterns, debugging workflows, API discoveries -- use `/flush` in the TUI:

```
/flush
```

This triggers an LLM-generated summary of the current session's most important content and writes it to a dated session log. The summary is indexed and searchable in future sessions.

Use `/flush` when you want to preserve important context:
- Before compaction (which discards old conversation turns)
- At the end of a productive debugging session
- After discovering important patterns or conventions

---

## Working with Memory

### Remember

Ask Axon to remember something, and it appends the note to a `MEMORY.md` file -- the workspace file for project-specific items, or the global `~/.axon/memory/MEMORY.md` for cross-project preferences:

```
> remember to always open PR links after pushing
```

Axon records entries as durable statements under organized headings, such as `## Preferences`, `## Project Context`, or `## Debugging`. The file watcher reindexes the change on the next memory search, so the new entry is searchable within the current session.

You can also save a note directly with the `/remember` command:

```
/remember always open PR links after pushing
```

Run `/remember` with no text to enter remember mode, where the next line you type becomes the note. Either way, Axon opens a review panel showing the note (with an optional rewritten version you can toggle with `Tab`); the note is written only after you confirm. On save, Axon shows `Memory saved to ~/.axon/memory/MEMORY.md`.

### Forget

Ask Axon to forget something, and it finds and removes the matching entry:

```
> forget the snake_case convention
```

Forget is best-effort: the model searches memory and removes entries that match. For guaranteed removal, edit the files under `~/.axon/memory/` directly and delete the entry yourself. To locate a file, open the `/memory` browser and press `y` to copy its path.

### Recall

Ask what Axon remembers:

```
> what do you remember?
```

Axon searches across all memory files and summarizes what it knows, grouped by source: global preferences, project-specific knowledge, and session history. Use `/memory` to browse the raw files.

### Direct Editing

You can edit memory files directly under `~/.axon/memory/`. The file watcher reindexes your changes on the next memory search. Use `/flush` to save the current session now, and `/dream` to consolidate session logs into organized topics.

---

## Browsing Memory with /memory

The `/memory` command opens a modal showing all memory files:

```
/memory
```

Files are grouped by scope:
- **Global** -- cross-project memory (`MEMORY.md`).
- **Workspace** -- project-specific memory (`MEMORY.md`).
- **Sessions** -- per-session summaries, in reverse chronological order.

The modal uses a split-pane layout: the file list on the left, a read-only content preview on the right. The preview updates as you move through the list.

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Move through the file list |
| `PgUp`/`PgDn` | Jump 10 entries |
| `/` | Filter the file list |
| `y` | Copy the selected file's path to the clipboard |
| `x` | Delete the selected session file (press `x` again to confirm) |
| `t` | Toggle memory on or off |
| `Ctrl+F` | Toggle fullscreen |
| `Esc` | Close the modal, or exit filter mode |

The preview pane is read-only. Scroll it with the mouse wheel or by dragging its scrollbar. You can delete only session files, not the global or workspace `MEMORY.md`.

When the memory modal's content area is under 80 columns, the modal hides the preview pane and shows the file list only.

You can also open `/memory` from the command palette.

---

## Memory Notifications

When you save a note with `/remember`, Axon confirms in the scrollback:

```
Memory saved to ~/.axon/memory/MEMORY.md
```

Background saves — flush, dream, and session-end — run silently and do not post a scrollback message. Use `/memory` at any time to browse what Axon has stored.

---

## Dream Consolidation with /dream

The `/dream` command consolidates scattered memory fragments into organized topics:

```
/dream
```

Dream reorganizes individual session logs and memory entries into a coherent, deduplicated knowledge base, which reduces noise and improves search quality over time. `/dream` requires memory to be enabled.

### Auto-Dream

Dream also runs automatically. By default, Axon checks the consolidation gates when a session ends and runs Dream once enough time has passed and enough sessions have accumulated:

```toml
[memory.dream]
enabled = true     # Run automatic consolidation (default: true)
min_hours = 4      # Minimum hours between consolidations
min_sessions = 3   # Minimum sessions since the last consolidation
# check_interval_secs is unset by default, so Dream runs only at session end.
# Set it to a positive number of seconds to also check on a periodic interval.
```

---

## How Memory Affects Prompts

### First-Turn Injection

On the first turn of each session, Axon automatically searches memory for content relevant to the current project and injects it as context. This means Axon starts with knowledge from previous sessions without a reminder.

First-turn injection can be configured:

```toml
[memory.initial_injection]
enabled = true     # Enable or disable first-turn injection
min_score = 0.0    # Optional score threshold; unset by default, which applies no filtering
```

### After Compaction

Memory is also searched after auto-compaction to recover relevant context that may have been discarded.

---

## Memory Search

Axon searches memory automatically, but you can also trigger searches manually in the chat:

```
Search memory for "auth middleware patterns"
Read my workspace MEMORY.md
```

The model has access to two memory tools:
- `memory_search` -- Hybrid search across all memory (vector + full-text)
- `memory_get` -- Read a specific memory file by path

### Hybrid Scoring

Every search runs two queries -- a BM25 keyword query and a vector similarity
query -- and then *fuses* the two result lists into one ranking. How they are
fused is set by `search.fusion`:

**`"rrf"` -- Reciprocal Rank Fusion (the default since 0.3.9).** Each list
contributes `1 / (k + rank)` per chunk, weighted by `vector_weight` and
`text_weight`, and the sums are renormalized so the best hit scores 1.0. Only
the *order* of each list is read, never the raw scores, so BM25 and cosine
never have to be made comparable. `search.rrf_k` sets `k` (default `1`; the
RRF paper's value is 60, which flattens the advantage of the top ranks).

**`"weighted"` -- normalize and blend (the default up to 0.3.8).** Each signal
is normalized to [0,1] and combined by weight -- vector 0.7, text 0.3 by
default.

RRF is the default because it is markedly better at paraphrased queries, where
the wording shares little vocabulary with the stored text. Measured against
`weighted` on two corpora with real embeddings, recall on paraphrase queries
went 36%->72% and 50%->83% and MRR roughly doubled, at the cost of an
occasional one- or two-place slip on queries that already share wording with
their answer.

Results are then filtered by `search.min_score` (default: 0.35). Note the
threshold means different things under the two strategies: under `"weighted"`
it is an absolute score, while under `"rrf"` -- where the top hit is always 1.0
-- it is relative to the best hit in that search.

> **When embeddings are unavailable:** under `"rrf"`, a chunk that appears in
> only one list caps at that list's share of the weight, so an un-embedded
> chunk tops out at 0.3 under default weights -- below the default `min_score`.
> Chunks are embedded on search and at session start, so this only bites while
> the embedding provider is failing. Set `min_score = 0.0`, or
> `fusion = "weighted"`, to keep such chunks visible.

### Source Weights

Each memory source has a weight multiplier applied to its score. All sources default to `1.0`, and you can adjust any of them under `[memory.search.source_weights]`:

| Source | Weight | Description |
|--------|--------|-------------|
| `workspace` | 1.0 | Project-specific memory |
| `session` | 1.0 | Session logs |
| `global` | 1.0 | Cross-project memory |

### Temporal Decay

Session memories decay over time so recent sessions are prioritized:

```toml
[memory.search.temporal_decay]
enabled = true           # Enable time-based decay
half_life_days = 7.0     # Score halves after this many days
```

Only session chunks decay. Global and workspace memories are exempt since they contain curated long-term knowledge.

### MMR (Maximal Marginal Relevance)

MMR re-ranking penalizes redundant results to improve diversity:

```toml
[memory.search.mmr]
enabled = false          # Opt-in diversity re-ranking
lambda = 0.7             # 0.0 = max diversity, 1.0 = pure relevance
```

---

## CLI Commands

The `axon memory` command manages memory from the shell. It has three
subcommands -- `clear`, `edit`, and `stats`:

```bash
# Clear workspace memory (MEMORY.md, sessions/, and index.sqlite). This is the default scope.
axon memory clear

# The same scope, stated explicitly
axon memory clear --workspace

# Clear the global MEMORY.md
axon memory clear --global

# Clear both workspace and global memory
axon memory clear --all

# Skip the confirmation prompt (-y is the short form)
axon memory clear --yes

# Open this workspace's MEMORY.md in $VISUAL / $EDITOR
axon memory edit

# Open the global MEMORY.md instead
axon memory edit --global

# Report file counts by scope, total size, indexed chunk count, and index size
axon memory stats
```

`memory edit` resolves the editor as `$VISUAL`, then `$EDITOR`, falling back to
Notepad on Windows and vi elsewhere. It creates the file if it does not exist
yet, which is the normal case in a workspace that has not run with memory
enabled before.

`memory stats` reads everything from disk at call time, so it is safe to run
while Axon is open and it reports what a new session would actually load.

---

## Configuration Reference

### Core Settings (`[memory]`)

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable memory |
| `session.save_on_end` | `true` | Write metadata summary on session end |
| `watcher.enabled` | `true` | Watch `~/.axon/memory/` for external edits and reindex |

### Index Settings (`[memory.index]`)

| Key | Default | Description |
|-----|---------|-------------|
| `max_chunk_chars` | `1600` | Maximum chunk size in characters |
| `chunk_overlap_chars` | `320` | Character overlap between chunks |

### Embedding Settings (`[memory.embedding]`)

| Key | Default | Description |
|-----|---------|-------------|
| `provider` | `"api"` | Embedding provider (currently `"api"`) |
| `model` | *(provider default)* | Embedding model name |
| `dimensions` | `1024` | Embedding vector dimensions |

### Search Settings (`[memory.search]`)

| Key | Default | Description |
|-----|---------|-------------|
| `max_results` | `6` | Maximum search results |
| `min_score` | `0.35` | Minimum relevance score. Absolute under `fusion = "weighted"`; relative to the best hit under `fusion = "rrf"` |
| `fusion` | `"rrf"` | How the keyword and vector lists are combined: `"rrf"` (Reciprocal Rank Fusion, order only) or `"weighted"` (normalize each signal and blend). See [Hybrid Scoring](#hybrid-scoring) |
| `rrf_k` | `1` | RRF rank constant; larger flattens the advantage of the top ranks. Ignored unless `fusion = "rrf"` |
| `vector_weight` | `0.7` | Weight for vector similarity |
| `text_weight` | `0.3` | Weight for BM25 text similarity |
| `bm25_saturation` | `0.0` | Knee constant for an absolute BM25 transform. `0.0` keeps the historical min-max normalization, in which the best keyword hit scores 1.0 however weak it is. Setting it above `0` was measured and did **not** improve retrieval; `fusion = "rrf"` addresses the same scale mismatch and is the supported answer |

### Initial Injection Settings (`[memory.initial_injection]`)

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable first-turn memory injection |
| `min_score` | unset | Score threshold for first-turn results. When unset, Axon applies no threshold, which is equivalent to `0.0`. |

### Dream Settings (`[memory.dream]`)

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable automatic Dream consolidation |
| `min_hours` | `4` | Minimum hours between consolidations |
| `min_sessions` | `3` | Minimum sessions since the last consolidation |
| `stale_lock_secs` | `3600` | Seconds before a stale consolidation lock is reclaimed |
| `check_interval_secs` | unset | Periodic check interval in seconds. When unset, Dream runs only at session end. |

### Flush Settings (`[compaction.memory_flush]`)

You configure flush under `[compaction]`, not `[memory]`, because it is a compaction behavior.

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable the pre-compaction memory flush |
| `soft_threshold_tokens` | `4000` | Token headroom before the compact threshold that triggers a flush |
| `max_flush_write_chars` | `8000` | Maximum characters the flush may write to memory |
| `flush_model` | unset | Model for the flush turn. When unset, Axon uses the session's primary model. |
| `idle_timeout_secs` | unset | Idle seconds before a background flush. When unset, flush runs only before compaction. |
| `semantic_dedup_threshold` | unset | Cosine-similarity threshold for de-duplicating flushed content. When unset, defaults to `0.92`. |

### Pruning Settings (`[compaction.pruning]`)

You configure pruning under `[compaction]`, not `[memory]`, because it is a compaction behavior.

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable tool-result pruning |
| `keep_last_n_turns` | `3` | Number of recent turns whose tool results are never pruned |
| `soft_trim_threshold` | `4000` | Character threshold above which old tool results are soft-trimmed |
| `soft_trim_head` | `1500` | Characters kept from the start of a soft-trimmed result |
| `soft_trim_tail` | `1500` | Characters kept from the end of a soft-trimmed result |
| `hard_clear_age_turns` | `10` | Turn age after which tool results are replaced with a placeholder |

---

## Memory Staleness

When a session memory is old, Axon attaches a staleness note to it in search results. Older results get a stronger reminder to verify the current state before you rely on them. These notes help you spot stored facts that might no longer be accurate. Global and workspace memories never receive staleness notes, because they hold curated long-term knowledge.

---

## File Watcher

By default, Axon watches `~/.axon/memory/` for external file changes. If you edit memory files directly (e.g., in your editor), the changes are picked up automatically on the next memory search:

- Created or modified files are reindexed.
- Deleted files have their stale chunks removed from the index.

```toml
[memory.watcher]
enabled = true    # default
```

---

## Troubleshooting

### Memory Not Working

1. Verify memory is enabled: check `axon inspect` output.
2. Check the flag: `axon --experimental-memory` or `AXON_MEMORY=1`.
3. Check for `--no-memory` or `AXON_MEMORY=0` overriding your config.

### Memory Not Appearing in Sessions

Memory is injected on the first turn. If you started a session before enabling memory, start a new session with `/new`.

### Viewing Memory Files

Use `/memory` in the TUI to browse all memory files with a preview. You can also access them directly:

```bash
ls ~/.axon/memory/
cat ~/.axon/memory/MEMORY.md
$EDITOR ~/.axon/memory/MEMORY.md
```

### Debug Logging

```bash
RUST_LOG=debug AXON_LOG_FILE=/tmp/axon.log axon
grep "memory" /tmp/axon.log
```
