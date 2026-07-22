# Getting Started

Axon is a local-first, terminal-based AI coding assistant. It runs as a TUI
(Terminal User Interface) that understands your codebase, executes shell
commands, edits files, searches the web, and manages tasks.

You can use it interactively as a full-screen TUI, run it headlessly for
scripting and CI/CD, or integrate it into editors via the Agent Client Protocol
(ACP).

> Axon is an independent fork of xAI's Apache-2.0-licensed Grok Build; it is not
> affiliated with xAI and makes no network calls to any xAI service.

---

## Installation

Axon is distributed through the project repository,
[SeatownSin/axon](https://github.com/SeatownSin/axon).

Install the CLI from npm (macOS, Linux, or Windows via Git Bash or PowerShell):

```bash
npm i -g @axon-official/axon
```

Or build from source: clone the repository and follow the build steps in the
README. Prebuilt binaries are published on the repository's GitHub Releases page.

Verify the installation:

```bash
axon --version
```

Update to the latest version at any time (pulls from GitHub Releases):

```bash
axon update
```

---

## First Launch

Start Axon by running:

```bash
axon
```

On first launch with no model configured, Axon runs a **first-run setup
wizard**. Instead of signing in to any hosted service, the wizard scans
`localhost` and your local network for OpenAI-compatible model servers —
Ollama, LM Studio, llama.cpp, and vLLM — and writes your choice to
`~/.axon/config.toml`. Local servers on loopback addresses need no API key and
no login.

To use a hosted provider instead (OpenAI, Anthropic, or any OpenAI-compatible
endpoint), add a model with your own API key — bring-your-own-key (BYOK) — or
set the `AXON_API_KEY` environment variable:

```bash
export AXON_API_KEY="sk-..."
axon
```

See [Authentication](02-authentication.md) for the full set of options,
including per-model keys, the `AXON_API_KEY` fallback, external auth providers,
and enterprise OIDC against your own IdP.

---

## Basic Interaction

Once a model is configured, Axon presents a full-screen TUI with two main areas:

- **Scrollback** -- the conversation history showing your prompts, Axon's responses, tool calls, file edits, and more.
- **Prompt** -- the input area at the bottom where you type messages.

Type a message and press `Enter` to send it. Axon reads files, runs commands, and edits code as needed. Each tool run streams into the scrollback in real time.

Press `Tab` to move focus between the prompt and the scrollback. While a turn is running, `Ctrl+C` cancels it (or clears a non-empty draft first); `Esc` is a no-op mid-turn. Idle, press `Esc` twice within 800ms to clear a non-empty prompt, or (with an empty prompt and conversation messages) to open rewind — see [Keyboard Shortcuts](03-keyboard-shortcuts.md#escape). With the scrollback focused, use the arrow keys to select entries and to collapse or expand them. To navigate with `j`/`k` and fold with `h`/`l` instead, enable Vim mode.

### File References

Use `@` in your prompt to attach files:

```
@src/main.rs              # Attach a file
@src/main.rs:10-50        # Attach lines 10-50
@src/                     # Browse a directory
```

The `@` operator opens a fuzzy file picker. By default it respects `.gitignore` and hides dotfiles. Prefix with `!` to search hidden files:

```
@!.github                 # Search hidden files
@!.env                    # Attach a .env file
```

### Permissions

By default, Axon asks for permission before executing shell commands or editing files. You can approve individually or toggle always-approve mode:

- Press `Ctrl+O` to toggle always-approve mode
- Use the `--yolo` flag at launch: `axon --yolo`
- Type `/always-approve` in the prompt to toggle the mode

---

## Key Concepts

### Sessions

Every conversation is a **session**. Sessions are automatically saved to `~/.axon/sessions/` and can be resumed later. Each session tracks the full conversation history, tool calls, file edits, and task state.

- Start a new session: `Ctrl+N` or `/new`
- Resume a previous session: `/resume` in the TUI, or `--resume <ID>` from the CLI
- Continue the most recent session: `axon -c`

### Scrollback

The scrollback is the main display area. It shows:

- **User prompts** -- your messages, rendered as sticky headers
- **Agent messages** -- Axon's responses with full markdown rendering and syntax highlighting
- **Thinking blocks** -- the agent's reasoning process (collapsible)
- **Tool calls** -- file edits (with inline diffs), command executions, search results, and more
- **Task lists** -- TODO items tracking progress

Collapse or expand the selected entry with the `Left`/`Right` arrow keys (or `h`/`l` and `e` in Vim mode). In Vim mode, press `y` to copy its content and `Y` to copy its metadata (for example, the command that ran). Press `Enter` to open it in the fullscreen viewer (in any mode).

### Tools

Axon has built-in tools for:

| Tool | Description |
|------|-------------|
| `read_file` / `search_replace` | Read and edit files with line-precise changes |
| `grep` | Regex search across your codebase (powered by ripgrep) |
| `list_dir` | List directory contents |
| `run_terminal_command` | Execute shell commands |
| `web_search` / `web_fetch` | Search the web and fetch URLs |
| `todo_write` | Create and manage task lists |
| `spawn_subagent` | Spawn parallel subagent sessions |
| `memory_search` | Search cross-session memory |

Tools can be extended with [MCP servers](05-configuration.md#mcp-servers) for integrations like GitHub, databases, and more.

### Slash Commands

Type `/` in the prompt to access commands. These provide quick actions without writing a full prompt:

```
/model local                      # Switch model
/compact                          # Compress conversation history
/always-approve                   # Toggle always-approve mode
/new                              # Start a new session
```

See [Slash Commands](04-slash-commands.md) for the complete reference.

---

## Common Launch Options

```bash
# Launch the interactive TUI and submit an initial prompt as the first turn
axon "fix the failing auth test and run it"

# Initial prompt in a new git worktree. Use --worktree=<name> (with `=`) so the
# prompt isn't swallowed as the worktree name — `axon -w "refactor module X"`
# would treat "refactor module X" as the worktree label, not the prompt.
axon --worktree=feat "refactor module X"

# Base the worktree on a specific branch (e.g. main) instead of the current HEAD:
axon -w --ref main "implement feature from main"


# Start in a specific project directory
axon --cwd ~/projects/my-app

# Add project-specific rules
axon --rules "Always use TypeScript. Prefer functional components."

# Auto-approve all tool executions
axon --yolo

# Use a specific model
axon -m local

# Resume a previous session
axon --resume <session-id>

# Continue the most recent session
axon -c

# Experimental scrollback-native render mode. Sticky: plain `axon` reopens in
# the mode last chosen via --minimal/--fullscreen (or /minimal//fullscreen).
axon --minimal

# Back to the standard fullscreen TUI (and make it sticky again)
axon --fullscreen

# Headless mode (for scripts)
axon -p "Explain this codebase"
```

---

## Headless Mode

Run Axon non-interactively for scripting, CI/CD, and automation:

```bash
axon -p "Your prompt here"
```

Output formats:

| Format | Flag | Description |
|--------|------|-------------|
| `plain` | (default) | Human-readable text |
| `json` | `--output-format json` | Single JSON object with `text`, `stopReason`, `sessionId`, and `requestId` |
| `streaming-json` | `--output-format streaming-json` | NDJSON event stream for real-time processing |

Example CI/CD usage:

```bash
axon -p "Review changes for bugs" --output-format json --yolo | jq -r '.text'
```

---

## Project Rules (AGENTS.md)

Add per-project instructions by creating an `AGENTS.md` file in your repository. Axon reads these files and injects their contents as a project-instructions message at the start of the conversation:

```
~/.axon/AGENTS.md           # Global rules (apply to all projects)
<repo-root>/AGENTS.md       # Repository-level rules
<cwd>/AGENTS.md             # Directory-level rules (highest priority)
```

Deeper files take precedence. Axon also reads `CLAUDE.md` files for compatibility.

---

## Where to Go Next

| Document | What You Will Learn |
|----------|-------------------|
| [Authentication](02-authentication.md) | Local model auto-detect, per-model API keys, `AXON_API_KEY`, external auth, OIDC |
| [Keyboard Shortcuts](03-keyboard-shortcuts.md) | Complete reference for all key bindings |
| [Slash Commands](04-slash-commands.md) | All available `/` commands |
| [Configuration](05-configuration.md) | config.toml, pager.toml, environment variables |
