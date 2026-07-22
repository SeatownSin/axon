# Axon

A local-first, privacy-focused terminal-based AI coding assistant and agentic harness.

Use it interactively as a TUI, or integrate it into your own apps via headless mode and the Agent Client Protocol (ACP).

Axon is an independent fork of xAI's Apache-2.0-licensed Grok Build; it is not affiliated with xAI. Telemetry and remote-login flows have been removed — Axon runs against your own local or BYOK (bring-your-own-key) models.

## Quick Start

```bash
# Build and run from a clone of the repo
cargo run -p axon-pager-bin

# Or install the published CLI
npm i -g @axon-official/axon

# Interactive TUI
axon

# Headless (for scripts/automation)
axon -p "Explain this codebase"

# Agent mode (for IDE/app integration)
axon agent stdio
```

## Contents

- [Installation](#installation)
- [Model Setup & Authentication](#model-setup--authentication) — first-run wizard, BYOK API keys, external auth providers
- **Using Axon**
  - [Interactive TUI](#interactive-tui) — shortcuts, slash commands, file references
  - [Headless Mode](#headless-mode) — scripting, CI/CD, output formats
  - [Agent Mode](#agent-mode) — stdio, ACP integration
  - [SSH Passthrough](#ssh-passthrough-axon-ssh) — Apple Terminal clipboard support
- **Configuration**
  - [Config File](#configuration) — general settings, LSP, enterprise deployment
  - [Custom Models](#custom-models) — BYOK, Ollama, OpenAI, custom endpoints
  - [MCP Servers](#mcp-servers) — external tool integrations
- **Customization**
  - [Project Rules (AGENTS.md)](#agentsmd) — per-project system prompt instructions
  - [Skills](#skills) — reusable prompt packages
  - [Agent Profiles](#agent-profiles) — custom agent definitions
  - [Subagents](#subagents) — parallel child sessions, roles, personas
  - [Plugins](#plugins) — external tool/skill packages
  - [Hooks](#hooks) — project lifecycle scripts
- **Features**
  - [Memory](#memory) — cross-session knowledge persistence
  - [Sandbox](#sandbox) — OS-level filesystem/network isolation
- **Reference**
  - [Introspection (`axon inspect`)](#introspection)
  - [Claude Code Compatibility](#claude-code-compatibility)
  - [Built-in Tools](#built-in-tools)
  - [Session Persistence](#session-persistence) — storage layout, resume
  - [File Locations](#file-locations)
  - [Environment Variables](#environment-variables)
  - [Troubleshooting](#troubleshooting)
- [Building with Axon](#building-with-axon) — headless API, ACP SDK integration

---

## Installation

**Build from source** (requires a Rust toolchain). Clone the repository and run the binary crate:

```bash
git clone https://github.com/SeatownSin/axon
cd axon
cargo run -p axon-pager-bin
```

To install it onto your `PATH`:

```bash
cargo install --path crates/codegen/axon-pager-bin
```

**Or install the published CLI** via npm:

```bash
npm i -g @axon-official/axon
```

Verify installation:

```bash
axon --version
```

Update to the latest version. `axon update` pulls the newest binary from the
project's GitHub Releases (`SeatownSin/axon`):

```bash
axon update
```

---

## Model Setup & Authentication

Axon has **no hosted login of any kind** — there is no account to create and no
vendor to sign in to. You point it at a model you control (a local server or any
OpenAI-compatible endpoint you have a key for), and that's it.

### First-Run Setup Wizard

On first launch with no model configured, Axon runs a small plain-terminal
wizard **before** the TUI starts. It scans `localhost` **and your local network**
for running OpenAI-compatible model servers — Ollama, LM Studio, llama.cpp, vLLM,
and similar — and offers to write the result to `~/.axon/config.toml`:

```bash
axon
#   Scanning localhost and your local network for model servers…
#   Detected models:
#     [1] Ollama            localhost:11434
#     [2] LM Studio         192.168.1.42:1234
#     [r] rescan   [m] enter an endpoint manually   [q] quit
```

Pick a detected server, or choose `m` to type an OpenAI-compatible base URL
(e.g. `http://localhost:8080/v1`) yourself. The wizard writes a `[model.local]`
entry and sets it as the default. Loopback servers are treated as **no-auth**
automatically (see below); a LAN server can be given a key if it needs one.

If nothing is found, start a server — `ollama serve`, LM Studio, llama.cpp, or
vLLM — then rescan or enter its endpoint manually.

### Loopback No-Auth Auto-Detect

Local model servers reachable on loopback (`localhost` / `127.0.0.1`) require **no
API key and no login**. Axon detects the loopback address and sends requests
without an `Authorization` header, so a freshly started Ollama or llama.cpp
server just works out of the box.

### BYOK: API Keys for Remote Providers

To use a hosted OpenAI-compatible provider (OpenAI, Anthropic, OpenRouter,
Together, Groq, a LAN server that requires auth, etc.), supply your own key. You
have three options, resolved in this order:

1. **Per-model `api_key`** — set the key inline on a `[model.*]` entry.
2. **Per-model `env_key`** — name the environment variable that holds the key.
3. **`AXON_API_KEY`** — a global fallback used when a model defines neither of
   the above.

```toml
# ~/.axon/config.toml
[model.openai-gpt]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"        # read from this env var

[model.my-lan-server]
model = "your-model-id"
base_url = "http://192.168.1.42:8000/v1"
api_key = "sk-..."                # or set inline
```

```bash
# Global fallback for any model without its own key
export AXON_API_KEY="sk-..."
axon
```

See [Custom Models](#custom-models) for the full model-configuration reference.

### OIDC (Enterprise SSO Against Your Own IdP)

For enterprise deployments, Axon can authenticate against **your own** OpenID
Connect Identity Provider (Okta, Azure AD, Auth0, Keycloak, …) and send the
resulting token to **your own** OpenAI-compatible proxy. No Axon-hosted service
is involved — every endpoint below is one you operate.

**1. Register a public client in your IdP:**
- Grant type: Authorization Code with PKCE
- Redirect URI: `http://127.0.0.1/callback` (the CLI uses a random ephemeral port; most IdPs treat loopback redirects as port-agnostic per [RFC 8252 §7.3](https://tools.ietf.org/html/rfc8252#section-7.3))
- No client secret (PKCE only, per [RFC 8252](https://tools.ietf.org/html/rfc8252))

**2. Configure the CLI** (config file or env vars):

```toml
# ~/.axon/config.toml
[axon_com_config.oidc]
issuer = "https://acme.okta.com"
client_id = "0oa1b2c3d4e5f6g7h8i9"
```

```bash
# Or via environment variables
export AXON_OIDC_ISSUER="https://acme.okta.com"
export AXON_OIDC_CLIENT_ID="0oa1b2c3d4e5f6g7h8i9"
```

Point inference at your own proxy so the OIDC token has somewhere to go:
```bash
export AXON_CLI_CHAT_PROXY_BASE_URL="https://llm-proxy.acme.com/v1"
```

**3. Run `axon`.** The CLI discovers endpoints via `{issuer}/.well-known/openid-configuration`, opens your IdP login page, and stores tokens in `~/.axon/auth.json`. The OIDC token is sent as `Authorization: Bearer` to the proxy you configured. Tokens auto-refresh silently via the stored `refresh_token`.

**Optional fields:**

| Field | Default | Notes |
|-------|---------|-------|
| `scopes` | `["openid", "profile", "email", "offline_access"]` | `offline_access` enables silent token refresh; add custom scopes if needed |
| `audience` | None | Required by some IdPs (e.g. Auth0) |

### External Auth Provider

For environments where browser-based login isn't possible (sandboxed VMs, CI runners, air-gapped networks), delegate authentication to an external binary or script. This is the recommended approach for enterprise deployments where your company runs its own auth infrastructure (SSO, device code flows, certificate auth, etc.).

Axon is provider-agnostic — it doesn't know or care how your binary authenticates. It just runs the command, reads a token from stdout, and stores it. Your binary is a black box that handles the entire auth flow.

#### How It Works

```
┌──────────────┐     sh -c     ┌────────────────────────┐
│     Axon     │──────────────▶│  your auth binary      │
│              │               │                        │
│  reads       │◀── stdout ────│  prints token          │
│  auth.json   │               │                        │
│              │   (stderr)    │  prints status/URLs    │──▶ user's terminal
└──────────────┘               └────────────────────────┘
```

1. Axon runs your command via `sh -c "<command>"`
2. Your binary does whatever auth flow it needs (SSO login, device code, cert exchange, etc.)
3. **stderr** → displayed directly to the user (use for login URLs, status messages, progress)
4. **stdout** → captured by Axon and saved to `~/.axon/auth.json` as the access token
5. exit 0 → success; exit non-zero → Axon falls through to interactive login

#### The stdout / stderr Contract

This is the most important thing to get right:

| Stream | What to print | Who sees it |
|--------|---------------|-------------|
| **stdout** | The token — nothing else | Axon (parsed and stored in `auth.json`) |
| **stderr** | Login URLs, status messages, errors, progress | The user (displayed in their terminal) |

**Do not print anything to stdout except the token.** No progress messages, no debug output, no "Login successful!" text. Axon reads stdout verbatim and tries to parse it as a token. Any extra text will break parsing.

#### stdout Token Format

The token on stdout can be either:

**1. Bare string** — just the raw token, nothing else:
```
eyJhbGciOiJSUzI1NiIs...
```

**2. JSON** — with optional refresh token and expiry:
```json
{"access_token": "eyJhbGciOi...", "refresh_token": "ref-tok", "expires_in": 3600}
```

Use JSON if your tokens expire and you want Axon to automatically re-run the binary before expiry. The `expires_in` field (seconds until expiry) tells Axon when to proactively refresh. Without it, Axon assumes tokens last 30 days.

#### Minimal Example

```bash
#!/bin/sh
# Print login URL / status to stderr (user sees this)
echo "Authenticating via Acme Corp SSO..." >&2
echo "Visit: https://sso.acme.com/device-login?code=ABCD-1234" >&2

# ... do the auth flow, get a token ...

# Print ONLY the token to stdout (Axon captures this)
echo "eyJhbGciOiJSUzI1NiIs..."
```

#### Configuration

```toml
# ~/.axon/config.toml
[auth]
auth_provider_command = "/usr/local/bin/my-auth-provider"
auth_provider_label = "Acme Corp"   # optional — customizes the TUI login button
auth_token_ttl = 3600               # optional — token lifetime in seconds (see below)
```

```bash
# Or via environment variables
export AXON_AUTH_PROVIDER_COMMAND="/usr/local/bin/my-auth-provider"
export AXON_AUTH_PROVIDER_LABEL="Acme Corp"   # optional
export AXON_AUTH_TOKEN_TTL=3600               # optional
```

If your binary outputs a bare token string (not JSON with `expires_in`), set `auth_token_ttl` to the token's expected lifetime in seconds. Without it, Axon cannot detect expiry proactively and will only refresh after a 401.

The command is run via `sh -c`, so it can be a binary path, a shell script, or a pipeline.

When `auth_provider_label` is set, the TUI welcome screen shows **"Login with Acme Corp"** on the login button. In headless mode (`axon -p`), the label has no effect — stderr from your binary is printed directly to the terminal.

> **Enterprise setup:** For a complete enterprise `config.toml` combining external auth, corporate proxy, and telemetry settings, see [Enterprise Deployment](#enterprise-deployment) in the Configuration section.

#### Example: Device Code Flow Provider

```bash
#!/bin/sh
# 1. Request device code from your IdP
RESP=$(curl -s -X POST https://auth.acme.com/device/code -d "client_id=axon-cli")
CODE=$(echo "$RESP" | jq -r '.user_code')
URL=$(echo "$RESP" | jq -r '.verification_uri')
DEVICE_CODE=$(echo "$RESP" | jq -r '.device_code')

# 2. Show login URL to user (stderr — user sees this in their terminal)
echo "Open $URL and enter code: $CODE" >&2

# 3. Poll until user approves
while true; do
  TOKEN=$(curl -s -X POST https://auth.acme.com/device/token \
    -d "device_code=$DEVICE_CODE&grant_type=urn:ietf:params:oauth:grant-type:device_code" \
    | jq -r 'select(.access_token) | .access_token')
  [ -n "$TOKEN" ] && break
  sleep 5
done

# 4. Print token to stdout — JSON format enables auto-refresh
echo "{\"access_token\": \"$TOKEN\", \"expires_in\": 3600}"
```

#### Example: Auth Binary with Refresh Support

When Axon needs to refresh an expired token, it re-runs your binary with `AXON_AUTH_EXPIRED=1` set in the environment. Your binary can use this to take a faster silent-refresh path:

```bash
#!/bin/sh
if [ "$AXON_AUTH_EXPIRED" = "1" ]; then
    # Token expired — attempt silent refresh (no user interaction)
    echo "Refreshing token..." >&2
    TOKEN=$(my-company-auth --refresh --silent)
else
    # First login — full interactive SSO flow
    echo "Authenticating via Acme Corp SSO..." >&2
    TOKEN=$(my-company-auth --login --interactive)
fi

if [ -z "$TOKEN" ]; then
    echo "Authentication failed" >&2
    exit 1
fi

echo "{\"access_token\": \"$TOKEN\", \"expires_in\": 3600}"
```

`AXON_AUTH_EXPIRED` is optional — if your binary ignores it, Axon still works. It just runs the same flow for both login and refresh.

### Automatic Credential Refresh

Axon supports automatic credential refresh for external auth providers and OIDC. When Axon detects that your token is expired (either locally based on `expires_in`, or when the server returns a 401), it automatically re-runs your `auth_provider_command` to obtain new credentials before retrying the request.

This is transparent — you don't need to do anything. Axon handles it in the background during your session.

**When does refresh happen?**

- **Before expiry:** If your binary returned `expires_in` in its JSON output, or you set `auth_token_ttl` in config, Axon re-runs the binary ~5 minutes before the token expires, so you never see an auth error.
- **On auth error:** If the server rejects a request with 401/403 (e.g. token was revoked or expired), Axon re-runs the binary and retries the request once.
- **OIDC:** If you're using OIDC and have a `refresh_token`, Axon silently refreshes via your IdP without re-opening the browser.

**Tuning the refresh buffer:**

```bash
# Axon refreshes tokens 5 minutes before expiry by default.
# Set to 0 to only refresh on 401. Set higher for very short-lived tokens.
export AXON_AUTH_EARLY_INVALIDATION_SECS=300
```

**Keep in mind:**
- When using `auth_provider_command`, you don't need to run `axon login` before starting — Axon runs your binary automatically on first launch. You _can_ run `axon login` to explicitly hydrate `auth.json` ahead of time if you prefer.
- If both OIDC and `auth_provider_command` are configured: at **login** time, Axon tries OIDC silent refresh first (if a `refresh_token` exists), then the external binary. During a **session**, whichever method is configured is used exclusively — if `auth_provider_command` is set it handles all mid-session refreshes; otherwise OIDC silent refresh is used.
- Your binary's stderr output is displayed to the user but interactive stdin is not supported. This works well for browser-based SSO flows where the binary displays a URL and you complete authentication in the browser.

#### Troubleshooting Auth

Enable debug logging to trace the auth flow:

```bash
axon --debug-file /tmp/axon-auth.log -p "hello"
tail -f /tmp/axon-auth.log
```

Common log messages:

| Log message | What it means |
|-------------|---------------|
| `auth: running external auth provider` | Your binary is being called (includes the command and whether it's a refresh) |
| `auth: external auth provider returned fresh token` | Success — token was parsed and stored |
| `auth: external auth provider failed` | Binary exited non-zero, or exited 0 but stdout was empty/unparseable (the `error` field has details) |
| `auth: external auth provider timed out (likely needs interactive auth), killing` | Binary didn't exit before the timeout (60s initial, 5s mid-session refresh) and was killed |
| `auth: failed to start external auth provider` | The command couldn't be spawned (e.g. binary not found) |

---

## Interactive TUI

The TUI (Terminal User Interface) provides a full interactive coding environment.

### Launch

```bash
axon [OPTIONS]
```

### Options

| Flag                       | Description                                                            |
| -------------------------- | ---------------------------------------------------------------------- |
| `--cwd <PATH>`             | Set working directory (default: current directory)                     |
| `--prompt <TEXT>`          | Send an initial prompt immediately after startup                       |
| `--rules <TEXT>`           | Append custom rules to the system prompt                               |
| `--always-approve`         | Auto-approve all tool executions without confirmation                  |
| `--sandbox <PROFILE>`      | OS-level filesystem/network guardrails (see [Sandbox](#sandbox))       |
| `--light`                  | Use light theme (macOS Basic) instead of dark                          |
| `--single-turn`            | Exit after first response (requires `--prompt`)                        |
| `--no-memory`              | Force-disable cross-session memory (overrides all other settings)      |
| `--subagents`              | Enable subagent/task tool support (see [Subagents](#subagents))        |
| `--disable-web-search`     | Remove web search tool from the agent toolset                          |
| `--agent-profile <PATH>`   | Load a custom agent definition file (see [Agent Profiles](#agent-profiles)) |
| `--experimental-memory`    | Enable cross-session memory persistence (see [Memory](#memory))        |
| `--allow <RULE>`           | Permission allow rule with glob patterns (repeatable). See [Permission Rules](#permission-rules-allow--deny). |
| `--deny <RULE>`            | Permission deny rule with glob patterns (repeatable). See [Permission Rules](#permission-rules-allow--deny). |

### Examples

```bash
# Start in a specific project
axon --cwd ~/projects/my-app

# Start with an initial task
axon --prompt "Review this codebase and suggest improvements"

# Add project-specific rules
axon --rules "Always use TypeScript. Prefer functional components."

# Auto-approve mode for trusted tasks
axon --always-approve --prompt "Format all files"
```

### Keyboard Shortcuts

| Key                          | Action                          |
| ---------------------------- | ------------------------------- |
| `Enter`                      | Send message                    |
| `Shift+Enter` or `Alt+Enter` | Insert newline                  |
| `Ctrl+M`                     | Toggle multiline input mode     |
| `Ctrl+C` or `Esc`            | Cancel current operation        |
| `Ctrl+D` or `Ctrl+Q`         | Quit (with confirmation)        |
| `Ctrl+O`                     | Toggle always-approve mode |
| `Ctrl+T`                     | Toggle TODO/task panel          |
| `Ctrl+R`                     | Search prompt history           |
| `Ctrl+V`                     | Paste from clipboard            |
| `Ctrl+U`                     | Undo last input change          |
| `Ctrl+G`                     | Move foreground task to background |
| `Ctrl+P`                     | Toggle debug panel              |

### Slash Commands

Type `/` in the input to access commands:

| Command                            | Alias     | Description                                              |
| ---------------------------------- | --------- | -------------------------------------------------------- |
| `/model <name>`                    | `/m`      | Switch to a different model                              |
| `/new`                             |           | Start a new session (clears context)                     |
| `/load [workspace] [session]`      | `/resume` | Load a previous session                                  |
| `/rewind <prompt>`                 |           | Rewind to a previous prompt (restores files)             |
| `/compact [context]`               |           | Compact conversation history                             |
| `/always-approve [on\|off]`        | `/yolo`   | Toggle auto-approve mode                                 |
| `/multiline`                       | `/ml`     | Toggle multiline input mode                              |
| `/memory [workspace\|global] <text>` |         | Append text to a memory file (requires `--experimental-memory`) |
| `/flush`                           |           | Save current session knowledge to memory now             |
| `/skills [name]`                   |           | List skills or inject a skill into context               |
| `/plugins [list\|reload\|trust]`   | `/plugin` | Manage plugins (list, reload, trust)                     |
| `/hooks-list`                      |           | Show hooks loaded in this session                        |
| `/hooks-trust`                     |           | Trust this folder for hooks (writes folder trust)        |
| `/hooks-add <path>`                |           | Add a custom hook file or directory                      |
| `/exit`                            | `/quit`   | Exit the TUI                                             |

```bash
# Example usage in TUI:
/model local
/new
/rewind
```

### Features

- **Syntax highlighting** for code blocks
- **Inline diffs** showing file changes before they're applied
- **Tool execution progress** with real-time output
- **TODO panel** tracking task progress
- **Session persistence** — conversations auto-save and can be resumed
- **History search** — `Ctrl+R` to search previous prompts

### File References (`@`)

Use the `@` operator in your prompt to attach file contents to your message. Type `@` followed by a filename or path to open a fuzzy file picker, then press `Tab` or `Enter` to select.

```
@src/main.rs              # Attach a file
@src/main.rs:10-50        # Attach lines 10–50 of a file
@src/                     # Browse a directory (end with /)
```

**Exposing hidden files with `!`**

By default, the `@` file picker respects `.gitignore` rules and hides dotfiles (files and directories starting with `.`). To search hidden files — such as `.github/`, `.vscode/`, `.env`, or other dotfiles — prefix your query with `!`:

```
@!.github                 # Search for .github/ and other hidden files
@!.vscode/settings.json   # Find .vscode/settings.json
@!.env                    # Attach a .env file
```

The `!` modifier allows you to attach any file in the project regardless of ignore rules.

---

## Headless Mode

Run Axon non-interactively from the command line. Use headless mode when you need to:

- **Automate tasks** — CI/CD pipelines, pre-commit hooks, cron jobs
- **Script workflows** — Batch process files, chain with other tools
- **Build integrations** — Spawn as a sub-agent, embed in larger systems
- **Parse output programmatically** — JSON output for downstream processing

Headless mode accepts a single prompt, executes it with full tool access, and returns the result.

### Basic Usage

```bash
axon -p "Your prompt here"
```

### Options

| Flag                    | Description                                           |
| ----------------------- | ----------------------------------------------------- |
| `-p, --single <PROMPT>` | The prompt to send (required)                         |
| `-m, --model <MODEL>`   | Model to use (e.g., `local`)                    |
| `-s, --session-id <ID>` | Create or resume a headless session with this ID      |
| `-r, --resume <ID>`     | Resume an existing session (errors if not found)      |
| `-c, --continue`        | Continue the most recent session in current directory |
| `--cwd <PATH>`          | Working directory                                     |
| `--output-format <FMT>` | Output format: `plain`, `json`, `streaming-json`      |
| `--always-approve`      | Auto-approve tool executions                          |
| `--rules <TEXT>`        | Custom rules for the system prompt                    |
| `--tools <TOOLS>`       | Allowlist of built-in tools (comma-separated). Only the listed tools will be available; all others are removed. Headless mode only. |
| `--disallowed-tools <TOOLS>` | Denylist of built-in tools to remove (comma-separated). Listed tools are stripped from the agent's toolset. Supports `Agent` / `Agent(type)` entries to restrict subagent spawning (see below). Headless mode only. |
| `--max-turns <N>`       | Maximum number of agentic turns before stopping       |
| `--reasoning-effort` / `--effort <LEVEL>` | Reasoning effort (`none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`; also per-model menu ids like `deep`). TUI and headless. |
| `--permission-mode <MODE>` | Permission mode for tool approvals                 |
| `--allow <RULE>`        | Permission allow rule with glob patterns (repeatable). See below. |
| `--deny <RULE>`         | Permission deny rule with glob patterns (repeatable). See below.  |

#### Tool Filtering (`--tools` / `--disallowed-tools`)

Use `--tools` to restrict the agent to an explicit set of tools (allowlist), or `--disallowed-tools` to remove specific tools from the default set (denylist). Both accept a comma-separated list of tool names.

Tool names correspond to the internal tool IDs shown below. For quick reference:

| Display Name   | Tool ID for `--tools` / `--disallowed-tools` |
| -------------- | --------------------------------------------- |
| bash           | `run_terminal_cmd`                            |
| grep           | `grep`                                        |
| read_file      | `read_file`                                   |
| search_replace | `search_replace`                              |
| list_dir       | `list_dir`                                    |
| web_search     | `web_search`                                  |
| web_fetch      | `web_fetch`                                   |
| todo_write     | `todo_write`                                  |
| task           | `task`                                        |

```bash
# Only allow read-only tools
axon -p "Explain this codebase" --tools "read_file,grep,list_dir"

# Remove web access and file editing
axon -p "Review this code" --disallowed-tools "web_search,web_fetch,search_replace"

# Remove shell access
axon -p "Review this code" --disallowed-tools "run_terminal_cmd"
```

`--disallowed-tools` also supports special `Agent` entries to control subagent spawning:

| Entry                          | Effect                                                  |
| ------------------------------ | ------------------------------------------------------- |
| `Agent`                        | Block **all** subagent spawning                         |
| `Agent(explore)`               | Block the `explore` subagent type only                  |
| `Agent(explore, plan)`         | Block multiple specific types                           |

```bash
# Allow tools but prevent the agent from spawning any subagents
axon -p "Fix this bug" --disallowed-tools "Agent"

# Block only the explore subagent
axon -p "Refactor this module" --disallowed-tools "Agent(explore)"
```

When `--tools` is set, only the listed tools are available and default tool injection is disabled. When both flags are present, `--disallowed-tools` runs after `--tools` — use this to start from an allowlist and then remove specific entries.

> **Note:** `--tools`, `--disallowed-tools`, and `--max-turns` are only supported in headless mode (`-p`). If used in the interactive TUI, a warning is printed and the flag is ignored. `--reasoning-effort`/`--effort` and `--permission-mode` work in both modes.

#### Permission Rules (`--allow` / `--deny`)

Permission rules control whether specific tool invocations are auto-approved, denied, or require confirmation. Unlike `--disallowed-tools` (which removes tools entirely from the agent's toolset), permission rules leave tools available but gate their execution.

Rules use `ToolPrefix(glob_pattern)` syntax. Supported prefixes:

| Prefix        | What it controls                   |
| ------------- | ---------------------------------- |
| `Bash(...)`   | Shell command execution            |
| `Edit(...)`   | File editing (path glob)           |
| `Write(...)`  | File writing (path glob)           |
| `Read(...)`   | File reading (path glob)           |
| `Grep(...)`   | Search operations (path glob)      |
| `WebFetch(...)` | URL fetching (glob or `domain:host`) |
| `MCPTool(...)` | MCP tool invocations              |

Glob patterns support `*` (single-level wildcard) and `**` (recursive). A bare prefix without parentheses matches all invocations of that type. Claude Code's `Bash(cmd:*)` rules are also accepted and are equivalent to prefix matching on `cmd`.

```bash
# Deny all shell commands matching "rm*"
axon -p "Clean up this project" --deny "Bash(rm*)"

# Allow npm commands, deny everything else dangerous
axon -p "Set up the project" --allow "Bash(npm*)" --deny "Bash(sudo*)"

# Deny edits outside src/
axon -p "Refactor the code" --deny "Edit(/etc/**)"

# Allow all bash commands (auto-approve without prompting)
axon -p "Build the project" --allow "Bash"

# Combine: allow fetching docs sites, deny other URLs
axon --allow "WebFetch(domain:docs.rs)" --deny "WebFetch(*)"
```

`--allow` and `--deny` can be repeated to add multiple rules. Deny rules take precedence over allow rules. These flags work in both TUI and headless mode.

### Examples

```bash
# Simple question
axon -p "What does this project do?"

# Use a specific model
axon -p "Optimize this function" -m local

# Get JSON output for parsing
axon -p "List all TODO comments in the codebase" --output-format json

# Streaming JSON for real-time processing
axon -p "Explain the architecture" --output-format streaming-json

# Multi-turn conversation (session ID is returned in JSON output)
axon -p "Remember: the secret number is 42" --output-format json
axon -p "What's the secret number?" --resume <sessionId>

# Resume most recent session
axon -p "Continue where we left off" -c

# Run in a different directory
axon -p "Run the tests" --cwd ~/projects/other-app --always-approve
```

### Scripting with Named Sessions

For CI and automation, `-s/--session-id` lets you choose your own session ID:

```bash
# Start a session namespaced to a PR
axon -p "Review the changes in this PR" -s "critique-myrepo-pr-123"

# Continue in the same session
axon -p "Now check for security issues" -s "critique-myrepo-pr-123"
```

If the session exists it picks up where you left off; if not, a new one is created.
This differs from `--resume`, which errors when the session doesn't exist.

> **Note:** `-s/--session-id` is for headless mode (`-p/--single`) only.
> In the interactive TUI, use `/load` or `--resume`.

### Output Formats

**plain** (default) — Human-readable text:

```
Here's a summary of the codebase...
```

**json** — Single JSON object after completion:

```json
{
  "text": "Here's a summary of the codebase...",
  "stopReason": "EndTurn",
  "sessionId": "abc123",
  "requestId": "xyz789"
}
```

**streaming-json** — Newline-delimited JSON events:

```json
{"type":"text","data":"Here's"}
{"type":"text","data":" a summary"}
{"type":"thought","data":"Analyzing the directory structure..."}
{"type":"end","stopReason":"EndTurn","sessionId":"abc123","requestId":"xyz789"}
```

### Scripting Examples

```bash
# Pipe output to a file
axon -p "Generate a README" > README.md

# Parse JSON output with jq
axon -p "List files" --output-format json | jq -r '.text'

# CI/CD: automated code review
axon -p "Review changes for bugs and security issues." \
  --output-format json --always-approve | jq -r '.text' > review.md

# Pipeline: chain with other tools
git diff --staged | axon -p "Write a concise commit message for these changes"

# Batch: process multiple files
for file in src/*.js; do
  axon -p "Migrate $file from CommonJS to ES modules." --always-approve
done

# Pre-commit hook
axon -p "Review staged changes for obvious bugs. Reply OK if fine, or list issues." \
  --always-approve --output-format json | jq -r '.text' | grep -q "^OK" || exit 1
```

> **Note:** Headless mode starts a fresh session by default. Use `-s <id>` to maintain context across calls.

---

## Agent Mode

Run Axon as an ACP (Agent Client Protocol) agent for integration with IDEs, editors, and custom tooling.

### stdio Transport

For direct integration with ACP clients:

```bash
axon agent stdio
```

Communication happens via JSON-RPC over stdin/stdout. This mode is used by:

- IDE extensions (Zed, Neovim, Emacs, etc.)
- Custom automation tools
- ACP client libraries

### Options

| Flag                  | Description                                                                         |
| --------------------- | ----------------------------------------------------------------------------------- |
| `-m, --model <MODEL>` | Override the default model ID (e.g., `local`)                                |
| `--always-approve`    | Start in always-approve mode (auto-approve all tool executions without confirmation) |
| `--reauth`            | Force re-authentication flow                                                        |

<details>
<summary><strong>Advanced: WebSocket Relay</strong></summary>

To expose the agent over the internet (instead of local network), run a WebSocket relay server and have the agent connect to it:

```bash
axon agent headless --axon-ws-url wss://your-relay.example.com/ws
```

The agent connects OUT to your relay, and your web clients connect to the same relay. Useful for building web UIs where browsers can't spawn local processes.

</details>

---

## SSH Passthrough (`axon ssh`)

Use `axon ssh` instead of plain `ssh` when connecting to remote hosts in terminals that lack native support (e.g. Apple Terminal) for local OSC 52 clipboard interception.

```bash
# Basic usage (same args as ssh)
axon ssh user@host

# With SSH flags
axon ssh -t user@host
axon ssh -L 8080:localhost:8080 user@host

# With remote command
axon ssh user@host -- tmux attach
```

On macOS, if the terminal doesn't natively handle OSC 52, `axon ssh` runs SSH inside a local PTY that intercepts clipboard sequences and writes them to `pbcopy`. Both plain OSC 52 and tmux DCS passthrough are handled. Terminals with native OSC 52 (iTerm2, Ghostty, Kitty, WezTerm, Alacritty) get a plain `ssh` exec with no wrapper.

This runs entirely locally.

---

## Building with Axon

Axon can be used as an OpenAI-compatible chat completion backend. Choose between two integration modes:

| Mode         | Use Case                                                           |
| ------------ | ------------------------------------------------------------------ |
| **Headless** | Simple chat API, scripts, automation, OpenAI SDK drop-in           |
| **ACP SDK**  | IDE integrations, tool visibility, thought streams, permission UIs |

---

### Headless Mode (Simple Chat Completion)

Use headless mode for simple integrations. Spawns `axon -p` and parses JSON output.

#### Python - Headless

```python
import asyncio
import json
import os

class AxonChat:
    """Simple OpenAI-compatible wrapper using headless mode."""

    def __init__(self, cwd="."):
        self.cwd = cwd
        self.env = {**os.environ}

    def _build_cmd(self, prompt, model, stream):
        return ["axon", "-p", prompt, "-m", model, "--cwd", self.cwd,
                "--output-format", "streaming-json" if stream else "json", "--always-approve"]

    async def create(self, messages, model="local", stream=False):
        prompt = messages[-1]["content"] if len(messages) == 1 else "\n".join(
            f"{m['role']}: {m['content']}" for m in messages
        )
        cmd = self._build_cmd(prompt, model, stream)

        if stream:
            return self._stream(cmd)

        proc = await asyncio.create_subprocess_exec(
            *cmd, env=self.env, stdout=asyncio.subprocess.PIPE
        )
        stdout, _ = await proc.communicate()
        data = json.loads(stdout.decode()) if stdout else {"text": ""}
        return {
            "choices": [{
                "message": {"role": "assistant", "content": data.get("text", "")},
                "finish_reason": "stop"
            }]
        }

    async def _stream(self, cmd):
        proc = await asyncio.create_subprocess_exec(
            *cmd, env=self.env, stdout=asyncio.subprocess.PIPE
        )
        async for line in proc.stdout:
            if not line.strip():
                continue
            event = json.loads(line)
            if event.get("type") == "text":
                yield {"choices": [{"delta": {"content": event["data"]}}]}
            elif event.get("type") == "end":
                yield {"choices": [{"delta": {}, "finish_reason": "stop"}]}


# Usage
async def main():
    client = AxonChat(cwd=".")

    # Non-streaming
    response = await client.create([{"role": "user", "content": "What files are here?"}])
    print(response["choices"][0]["message"]["content"])

    # Streaming
    async for chunk in await client.create(
        [{"role": "user", "content": "List files"}], stream=True
    ):
        print(chunk["choices"][0]["delta"].get("content", ""), end="", flush=True)

asyncio.run(main())
```

#### TypeScript - Headless

```typescript
import { execa } from "execa";

class AxonChat {
  constructor(private cwd = ".") {}

  private buildArgs(prompt: string, model: string, stream: boolean) {
    return [
      "-p",
      prompt,
      "-m",
      model,
      "--cwd",
      this.cwd,
      "--output-format",
      stream ? "streaming-json" : "json",
      "--always-approve",
    ];
  }

  async create(
    messages: { role: string; content: string }[],
    { model = "local", stream = false } = {},
  ) {
    const prompt =
      messages.length === 1
        ? messages[0].content
        : messages.map((m) => `${m.role}: ${m.content}`).join("\n");

    if (stream) return this.streamResponse(prompt, model);

    const { stdout } = await execa(
      "axon",
      this.buildArgs(prompt, model, false),
    );
    const data = JSON.parse(stdout || '{"text":""}');
    return {
      choices: [
        {
          message: { role: "assistant", content: data.text || "" },
          finish_reason: "stop",
        },
      ],
    };
  }

  async *streamResponse(prompt: string, model: string) {
    const proc = execa("axon", this.buildArgs(prompt, model, true));
    for await (const chunk of proc.stdout!) {
      for (const line of chunk.toString().split("\n").filter(Boolean)) {
        const event = JSON.parse(line);
        if (event.type === "text") {
          yield { choices: [{ delta: { content: event.data } }] };
        } else if (event.type === "end") {
          yield { choices: [{ delta: {}, finish_reason: "stop" }] };
        }
      }
    }
  }
}

// Usage
const client = new AxonChat(".");

// Non-streaming
const response = await client.create([
  { role: "user", content: "What files are here?" },
]);
console.log(response.choices[0].message.content);

// Streaming
for await (const chunk of await client.create(
  [{ role: "user", content: "List files" }],
  { stream: true },
)) {
  process.stdout.write(chunk.choices[0].delta?.content || "");
}
```

---

### ACP SDK (Rich Agent Integration)

Use the Agent Client Protocol for full access to tool calls, thoughts, plans, and permissions.

#### Python - ACP SDK

```python
import asyncio
import json

class AxonACPChat:
    """Rich OpenAI-compatible wrapper using ACP protocol."""

    def __init__(self, cwd="."):
        self.cwd = cwd
        self.proc = None
        self.session_id = None

    async def init(self):
        self.proc = await asyncio.create_subprocess_exec(
            "axon", "agent", "stdio",
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE
        )

        # Initialize
        await self._request("initialize", {
            "protocolVersion": "1",
            "clientCapabilities": {
                "fs": {"readTextFile": True, "writeTextFile": True},
                "terminal": True
            }
        })

        # Create session
        result = await self._request("session/new", {
            "cwd": self.cwd,
            "mcpServers": []
        })
        self.session_id = result["sessionId"]
        return self

    async def _request(self, method, params):
        msg = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
        self.proc.stdin.write(f"{msg}\n".encode())
        await self.proc.stdin.drain()

        line = await self.proc.stdout.readline()
        return json.loads(line).get("result", {})

    async def create(self, messages, model="local", stream=False):
        prompt = [{"type": "text", "text": m["content"]} for m in messages]

        # For streaming, yield chunks as they arrive
        if stream:
            return self._stream(prompt)

        result = await self._request("session/prompt", {
            "sessionId": self.session_id,
            "prompt": prompt
        })
        return {
            "choices": [{
                "message": {"role": "assistant", "content": result.get("text", "")},
                "finish_reason": result.get("stopReason", "stop").lower()
            }]
        }

    async def _stream(self, prompt):
        # Send prompt request
        msg = json.dumps({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/prompt",
            "params": {"sessionId": self.session_id, "prompt": prompt}
        })
        self.proc.stdin.write(f"{msg}\n".encode())
        await self.proc.stdin.drain()

        # Read streaming updates
        while True:
            line = await self.proc.stdout.readline()
            if not line:
                break

            data = json.loads(line)

            # Handle notifications
            if data.get("method") == "session/update":
                update = data["params"]["update"]
                session_update = update.get("sessionUpdate")

                if session_update == "agent_message_chunk":
                    yield {"choices": [{"delta": {"content": update["content"]["text"]}}]}
                elif session_update == "agent_thought_chunk":
                    yield {"choices": [{"delta": {"thought": update["content"]["text"]}}]}
                elif session_update == "tool_call":
                    yield {"choices": [{"delta": {"tool_call": {
                        "name": update["tool"],
                        "status": "pending"
                    }}}]}
                elif session_update == "plan":
                    yield {"choices": [{"delta": {"plan": update["entries"]}}]}

            # Handle final response
            elif "result" in data:
                yield {"choices": [{"delta": {}, "finish_reason": "stop"}]}
                break


# Usage
async def main():
    client = await AxonACPChat(cwd=".").init()

    # Streaming with rich updates
    async for chunk in await client.create(
        [{"role": "user", "content": "Refactor the main function"}],
        stream=True
    ):
        delta = chunk["choices"][0]["delta"]
        if "content" in delta:
            print(delta["content"], end="", flush=True)
        if "thought" in delta:
            print(f"\n[Thinking: {delta['thought']}]", end="")
        if "tool_call" in delta:
            print(f"\n[Tool: {delta['tool_call']}]")
        if "plan" in delta:
            print(f"\n[Plan: {delta['plan']}]")

asyncio.run(main())
```

#### TypeScript - ACP SDK

```typescript
import { spawn, ChildProcess } from "child_process";
import * as readline from "readline";

class AxonACPChat {
  private proc!: ChildProcess;
  private sessionId!: string;
  private rl!: readline.Interface;

  constructor(private cwd = ".") {}

  async init() {
    this.proc = spawn("axon", ["agent", "stdio"]);
    this.rl = readline.createInterface({ input: this.proc.stdout! });

    // Initialize
    await this.request("initialize", {
      protocolVersion: "1",
      clientCapabilities: {
        fs: { readTextFile: true, writeTextFile: true },
        terminal: true,
      },
    });

    // Create session
    const { sessionId } = await this.request("session/new", {
      cwd: this.cwd,
      mcpServers: [],
    });
    this.sessionId = sessionId;
    return this;
  }

  private async request(method: string, params: any): Promise<any> {
    return new Promise((resolve) => {
      const msg = JSON.stringify({ jsonrpc: "2.0", id: 1, method, params });
      this.proc.stdin!.write(msg + "\n");

      this.rl.once("line", (line) => {
        resolve(JSON.parse(line).result || {});
      });
    });
  }

  async create(
    messages: { role: string; content: string }[],
    { model = "local", stream = false } = {},
  ) {
    const prompt = messages.map((m) => ({ type: "text", text: m.content }));

    if (stream) return this.streamResponse(prompt);

    const result = await this.request("session/prompt", {
      sessionId: this.sessionId,
      prompt,
    });

    return {
      choices: [
        {
          message: { role: "assistant", content: result.text || "" },
          finish_reason: result.stopReason?.toLowerCase() || "stop",
        },
      ],
    };
  }

  async *streamResponse(prompt: { type: string; text: string }[]) {
    const msg = JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "session/prompt",
      params: { sessionId: this.sessionId, prompt },
    });
    this.proc.stdin!.write(msg + "\n");

    for await (const line of this.rl) {
      const data = JSON.parse(line);

      if (data.method === "session/update") {
        const update = data.params.update;
        switch (update.sessionUpdate) {
          case "agent_message_chunk":
            yield { choices: [{ delta: { content: update.content?.text } }] };
            break;
          case "agent_thought_chunk":
            yield { choices: [{ delta: { thought: update.content?.text } }] };
            break;
          case "tool_call":
            yield {
              choices: [
                {
                  delta: {
                    tool_call: {
                      name: update.tool,
                      args: update.arguments,
                      status: "pending",
                    },
                  },
                },
              ],
            };
            break;
          case "plan":
            yield { choices: [{ delta: { plan: update.entries } }] };
            break;
        }
      } else if (data.result) {
        yield { choices: [{ delta: {}, finish_reason: "stop" }] };
        break;
      }
    }
  }
}

// Usage
const client = await new AxonACPChat(".").init();

// Streaming with rich updates
for await (const chunk of await client.create(
  [{ role: "user", content: "Refactor main" }],
  { stream: true },
)) {
  const delta = chunk.choices[0].delta;
  if (delta.content) process.stdout.write(delta.content);
  if (delta.thought) console.log(`\n[Thinking: ${delta.thought}]`);
  if (delta.tool_call)
    console.log(`\n[Tool: ${JSON.stringify(delta.tool_call)}]`);
  if (delta.plan) console.log(`\n[Plan: ${JSON.stringify(delta.plan)}]`);
}
```

---

### ACP Protocol Reference

Axon implements the [Agent Client Protocol (ACP)](https://agentclientprotocol.com), a standard for AI agent communication.

#### Architecture

```
┌─────────────────────────────────────────┐
│           ACP Client                    │
│  (IDE, Editor, Custom Application)      │
└──────────────────┬──────────────────────┘
                   │ JSON-RPC over stdio
┌──────────────────▼──────────────────────┐
│           axon agent stdio              │
│                                         │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  │
│  │ Session │  │  Tools  │  │   MCP   │  │
│  │ Manager │  │ Registry│  │ Servers │  │
│  └─────────┘  └─────────┘  └─────────┘  │
└─────────────────────────────────────────┘
```

#### SDKs

| Language   | Package                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------- |
| TypeScript | [`@agentclientprotocol/sdk`](https://www.npmjs.com/package/@agentclientprotocol/sdk)     |
| Rust       | [`agent-client-protocol`](https://crates.io/crates/agent-client-protocol)                |
| Python     | [`agent-client-protocol-python`](https://github.com/PsiACE/agent-client-protocol-python) |
| Go         | [`acp-go-sdk`](https://github.com/coder/acp-go-sdk)                                      |
| Kotlin     | [`acp`](https://github.com/agentclientprotocol/kotlin-sdk)                               |

#### Resources

- [ACP Specification](https://agentclientprotocol.com/protocol/prompt-turn)
- [Protocol Introduction](https://agentclientprotocol.com/overview/introduction)

#### Compatible Clients

| Client                                                   | Status      |
| -------------------------------------------------------- | ----------- |
| [Zed](https://zed.dev/docs/ai/external-agents)           | ✓ Supported |
| [Neovim](https://neovim.io) (CodeCompanion, avante.nvim) | ✓ Supported |
| [Emacs](https://github.com/xenodium/agent-shell)         | ✓ Supported |
| [marimo notebook](https://github.com/marimo-team/marimo) | ✓ Supported |
| JetBrains                                                | Coming soon |

---

## Configuration

Axon reads configuration from `~/.axon/config.toml`. If the file doesn't exist, Axon uses sensible defaults. You only need to specify values you want to override.

Each feature section below documents its own config. This section covers the general-purpose settings that don't have their own top-level section.

### General Settings

```toml
[cli]
auto_update = true                     # check for updates on launch

[models]
default = "local"                      # model used for new sessions
web_search = "your-web-search-model"   # model used by the web_search tool (must support the Responses API)

[ui]
max_thoughts_width = 120               # max column width for reasoning display

[features]
support_permission = false             # prompt before tool execution
lsp_tools = false                      # expose the lsp tool (see LSP Servers below)
codebase_indexing = true               # code graph indexing (true, false, or glob patterns)

[session]
auto_compact_threshold_percent = 85    # auto-compact at this % of context window
load_envrc = true                      # load .envrc environment variables into bash commands

[tools]
respect_gitignore = true               # filter gitignored files from tools (env: AXON_RESPECT_GITIGNORE)

[toolset.bash]
timeout_secs = 120.0                   # command timeout in seconds
output_byte_limit = 65536              # max output size (64KB)

[toolset.web_fetch]
proxy_endpoint = "https://proxy.example.com"   # egress proxy URL (all requests routed through it)
allowed_domains = ["docs.rs", "github.com"]     # override the built-in ~84-domain allowlist

[shortcuts]
send = ["Enter"]
newline = ["Shift+Enter", "Alt+Enter"]
quit = ["Ctrl+D", "Ctrl+Q"]
confirm_quit = true
```

### Telemetry

Telemetry has been removed from this fork. There is no product analytics,
trace export, crash reporting, feedback API, or remote settings prefetch — the
phone-home code paths are deleted, and there is no `[telemetry]` config sink to
enable. Axon sends nothing about your usage anywhere.

### LSP Servers

Axon can connect to Language Server Protocol (LSP) servers configured in JSON files. LSP integration gives Axon language-aware code intelligence while it works in your repository.

LSP support is used in two ways:

- **Passive diagnostics** — after edits, Axon can surface language-server diagnostics such as errors and warnings.
- **The `lsp` tool** — Axon can actively query the language server for `goToDefinition`, `findReferences`, `hover`, `goToImplementation`, `documentSymbol`, and `workspaceSymbol`.

Reference: [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)

#### Config locations

Axon looks for server definitions in:

- project config: `<repo>/.axon/lsp.json`
- user config: `~/.axon/lsp.json`

If the same server name appears in both places, the project config wins.

#### Tool enablement

Having an `lsp.json` file is enough for passive diagnostics. The model-visible `lsp` tool is exposed when both of these are true:

- LSP tools are enabled (`AXON_LSP_TOOLS=1` or `[features] lsp_tools = true`)
- the merged LSP configuration is non-empty

Enable the tool for one run:

```bash
AXON_LSP_TOOLS=1 axon
```

Or enable it in config:

```toml
[features]
lsp_tools = true
```

If LSP tools are enabled but no usable server config is found, Axon emits a non-fatal warning in logs and continues without the `lsp` tool. If config exists but every server fails to start, the tool may still be present and will fail on first use with a startup error.

#### Example `lsp.json`

```json
{
  "typescript": {
    "command": "typescript-language-server",
    "args": ["--stdio"],
    "extensionToLanguage": {
      ".ts": "typescript",
      ".tsx": "typescriptreact"
    },
    "startupTimeout": 30000
  }
}
```

#### Required fields

| Field | Description |
|-------|-------------|
| `command` | Server binary to execute. For `stdio`, this must be available in `PATH` or be an absolute path. |
| `extensionToLanguage` | Maps file extensions to LSP language IDs. |

#### Optional fields

| Field | Description |
|-------|-------------|
| `args` | Command-line arguments for the server process. |
| `transport` | `stdio` (default) or `socket`. |
| `env` | Extra environment variables for the server process. |
| `initializationOptions` | JSON passed during LSP initialize. |
| `settings` | Configuration sent via workspace settings updates. |
| `workspaceFolder` | Override workspace folder path sent to the server. |
| `startupTimeout` | Max startup wait in milliseconds before startup is considered failed. |
| `shutdownTimeout` | Max graceful shutdown wait in milliseconds. |
| `restartOnCrash` | Whether to restart the server after a crash. |
| `maxRestarts` | Maximum restart attempts before giving up. |

#### Installing language servers

Axon does not bundle language server binaries. You must install the server yourself and make sure the configured `command` is runnable on your machine.

Examples:

| Language | Server | Install example |
|----------|--------|-----------------|
| TypeScript | `typescript-language-server` | `npm install -g typescript-language-server typescript` |
| Python | `pyright` | `npm install -g pyright` or `pip install pyright` |
| Rust | `rust-analyzer` | Install `rust-analyzer` using your platform's recommended method |

#### Notes

- Passive diagnostics do **not** require `AXON_LSP_TOOLS=1`; they run whenever an applicable server is configured and starts successfully.
- Passive diagnostics are currently driven by `search_replace` edits; they are not a general watcher for arbitrary shell or git mutations in the workspace.
- The `lsp` tool is intentionally hidden when disabled or unconfigured so the model does not plan around unavailable capabilities.
- Same-workspace subagents reuse the parent session's live LSP runtime instead of starting a duplicate server pool.
- That reuse means the child inherits the parent's LSP server set for the shared workspace; child-local LSP config differences are not loaded in the reused-runtime path.

### Enterprise Deployment

A complete `config.toml` for an enterprise deployment with external auth, corporate proxy, and telemetry disabled:

```toml
[cli]
auto_update = false

[auth]
auth_provider_command = "/usr/local/bin/my-company-auth-provider"
auth_provider_label = "Acme Corp"
auth_token_ttl = 3600               # if your provider outputs bare tokens

[models]
default = "company-model"

[model.company-model]
model = "your-model-id"
base_url = "https://llm-proxy.acme.com/"
name = "Company Model (Proxy)"
context_window = 256000

[features]
support_permission = false

[toolset.bash]
timeout_secs = 120.0
```

With this config, `axon` runs your auth binary, stores the token, and routes inference through your corporate proxy. See [Model Setup & Authentication](#model-setup--authentication) for full auth setup details.

---

## AGENTS.md

Add project-specific instructions by creating an agent rules file (e.g., `AGENTS.md`). Axon reads these files and appends their contents to the system prompt.

Axon scans for agent rules in this order:

1. `~/.axon/` (global rules)
2. If inside a git repo: every directory from the repo root → current working directory (inclusive)
3. If **not** inside a git repo: only the current working directory

Within each directory, Axon checks for these filenames:

- `Agents.md`, `Claude.md`, `AGENT.md`, `AGENTS.md`

Ordering matters: files found later (deeper directories) come last, so they effectively take precedence if instructions conflict. Files ignored by gitignore are skipped. Each file is capped at 10,000 characters (truncated with a warning if exceeded).

> **Note:** The `--rules` flag appends _additional_ rules on top of any discovered agent files, so you can combine both for session-specific customization.

---

## Skills

Skills are reusable prompt packages that extend Axon with specialized workflows, domain knowledge, and tool integrations. Use them to encode repeatable procedures that would otherwise require re-explaining each session.

### Skill Locations

Axon discovers skills from these directories (in priority order):

| Location                    | Scope | Priority |
| --------------------------- | ----- | -------- |
| `./.axon/skills/`           | Local | Highest  |
| `<repo_root>/.axon/skills/` | Repo  | Medium   |
| `~/.axon/skills/`           | User  | Lowest   |
| `~/.claude/skills/`         | User  | Lowest   |

Skills with the same name are deduplicated — higher priority locations override lower ones.

Repo-scoped skills (Local and Repo) respect `.gitignore` and are filtered out if ignored. User-scoped skills (`~/.axon/skills/`) are outside the repo and never filtered.

### Configuration

Add extra skill directories or exclude paths via `[skills]` in config.toml:

```toml
[skills]
paths = ["~/my-team-skills"]          # additional directories to scan
ignore = ["~/my-team-skills/wip"]     # paths to exclude
```

### Creating a Skill

Each skill lives in its own directory with a `SKILL.md` file:

```
~/.axon/skills/
└── commit/
    └── SKILL.md
```

**SKILL.md format:**

```markdown
---
name: commit
description: Create well-formatted git commits following conventional commit standards. Use when the user wants to commit changes or asks for /commit.
---

# Git Commit Skill

Review staged changes and create a commit with a clear, conventional message.

## Steps

1. Run `git diff --staged` to see changes
2. Summarize what changed and why
3. Create commit message following conventional commits format
4. Run `git commit -m "..."` with the message
```

**Required frontmatter fields:**

| Field         | Description                                                                  |
| ------------- | ---------------------------------------------------------------------------- |
| `name`        | Skill identifier (lowercase, hyphens, max 64 chars)                          |
| `description` | What the skill does and when to use it—this is how Axon decides to invoke it |

### Using Skills

**In the TUI:**

```bash
/skills              # List available skills
/skills commit       # Inject the "commit" skill into context
```

**The model can also invoke skills automatically** when it recognizes a relevant task. The skill's `description` field determines when this happens.

**Slash command shorthand:**

Users can reference skills as `/skill-name` (e.g., `/commit`). When you see this pattern, Axon invokes the corresponding skill.

> **Tip:** The `description` field is critical — it determines when Axon automatically invokes the skill. Be specific about trigger phrases and use cases.

---

## Agent Profiles

Agent profiles control the system prompt, toolset, and behavior of a session. A profile is a `.md` file with YAML frontmatter, or a named agent discovered from disk.

Axon discovers agent definitions from `.axon/agents/` (project), `~/.axon/agents/` (user), and built-in agents. Priority (highest wins):

1. `--agent-profile <PATH>` CLI flag
2. `[agent]` section in `config.toml`
3. `AXON_AGENT` env var
4. Default built-in agent

```toml
# ~/.axon/config.toml
[agent]
name = "my-custom-agent"             # Discovered by name
# definition = "/path/to/agent.md"   # OR: explicit path
```

```bash
axon --agent-profile ./my-agent.md
# or
export AXON_AGENT="my-custom-agent"
```

---

## Subagents

Subagents spawn independent child sessions that handle tasks in parallel. Each child has its own context window and can optionally inherit the parent's conversation history. Enabled by default.

### Disabling

```bash
export AXON_SUBAGENTS=0              # Environment variable
```

```toml
# ~/.axon/config.toml
[subagents]
enabled = false
```

### Toggles and Model Overrides

Disable specific subagent types while keeping the system enabled, or route them to different models:

```toml
[subagents.toggle]
explore = true                       # default — omitted agents are enabled
plan = false                         # disable plan subagent

[subagents.models]
explore = "local"                   # route explore to a lighter model
```

By default a subagent inherits the parent session's model. Only an explicit
per-agent pin overrides that: `[subagents.models].<agent>` (highest priority),
then the agent definition's `model`. Both pins apply unconditionally,
regardless of which model the parent is on.

### Roles and Personas

Roles define reusable capability/model defaults. Personas layer tone and behavior instructions onto the child prompt.

```toml
[subagents.roles.researcher]
description = "Deep research agent"
default_capability_mode = "read-only"
model = "local"
prompt_file = ".axon/prompts/researcher.md"

[subagents.personas.concise]
instructions = "Be extremely concise. No filler words."
# instructions_file = ".axon/personas/concise.md"  # or load from file
```

Both are also discovered from `.axon/roles/*.toml` and `.axon/personas/*.toml` files respectively. If a requested persona is not found, the spawn fails (fail-closed).

---

## Plugins

Plugins extend Axon with additional tools, skills, and MCP servers from external packages.

### Plugin Locations

| Location                    | Scope   |
| --------------------------- | ------- |
| `.axon/plugins/`            | Project |
| `~/.axon/plugins/`          | User    |
| `--plugin-dir <PATH>` (CLI) | Session |

### Configuration

```toml
# ~/.axon/config.toml
[plugins]
paths = ["~/my-plugins/custom-tools"]       # additional plugin directories
disabled = ["user/a1b2c3d4/noisy-plugin"]   # plugin IDs to skip
```

Manage plugins at runtime with `/plugins list`, `/plugins reload`, or `/plugins trust <path>`.

---

## Hooks

Hooks run project scripts on tool and session lifecycle events (pre/post-tool-use, session start/end). Projects must be explicitly trusted before their hooks execute.

Axon discovers hooks from `.axon/hooks/` in the project directory. Manage them with:

```
/hooks-list              # show hooks loaded in this session
/hooks-trust             # trust this project for hook execution
/hooks-add <path>        # add a custom hook file or directory
```

---

## Custom Models

Add custom model endpoints to use alternative providers or self-hosted models. You can also override built-in models with custom settings.

### Model Configuration

The name in the TOML header (`my-model` in `[model.my-model]`) is what appears in the model picker. The `model` field is the identifier sent to the API. If `model` is omitted, the header name is sent to the API directly.

```toml
[model.my-model]
model = "model-id"                    # Model identifier sent to API
base_url = "https://api.example.com/v1"  # OpenAI-compatible endpoint
name = "Display Name"                 # Shown in model picker
description = "Model description"     # Optional description
api_key = "sk-..."                    # API key for this provider (optional)
env_key = "OPENAI_API_KEY"            # Env var(s) holding the API key (string or array; first set wins)
temperature = 0.7                     # Sampling temperature (0.0-2.0)
top_p = 0.95                          # Nucleus sampling parameter
max_completion_tokens = 8192          # Max tokens per response
context_window = 256000               # Total context window in tokens (for auto-compact)
```

**Credential resolution order:** `api_key` → `env_key` → `AXON_API_KEY`. If neither `api_key` nor `env_key` is set, Axon falls back to the global `AXON_API_KEY` environment variable. (Loopback endpoints need no key at all — see [Loopback No-Auth Auto-Detect](#loopback-no-auth-auto-detect).)

The `context_window` parameter is used to calculate when auto-compact should trigger. If not specified, Axon falls back to built-in defaults for known models.

### Overriding Built-in Models

You can override specific fields of built-in models without redefining everything. Only specify the fields you want to change:

```toml
# Override just the API key for a default model
[model.axon-build]
api_key = "my-api-key"

# Override temperature and add a custom API key
[model.axon-4.20-0309-reasoning]
temperature = 0.5
api_key = "sk-custom"
```

**How it works:** When you override a built-in model, Axon starts with the default configuration (including the correct `base_url` from your `[endpoints]` setting), then applies only the fields you specify. Unspecified fields inherit from the default.

**Priority order:**
1. Your config (`[model.*]`) — highest priority
2. Prefetched models from remote `/v1/models`
3. Hardcoded defaults — lowest priority

**Web search model:** Set `[models] web_search`, `AXON_WEB_SEARCH_MODEL`, or `--web-search-model` to point the `web_search` tool at a different model. The target endpoint must support the Responses API and web search.

> **Overriding with a custom model:** Setting `[models] web_search` alone is not
> enough if the model isn't already in the catalog (built-in defaults or
> `axon models` output). You also need a `[model.*]` entry so Axon knows
> how to reach it. Without both, web search is silently disabled.
>
> ```toml
> [models]
> web_search = "my-custom-model"       # 1. tell web search which model to use
>
> [model.my-custom-model]              # 2. tell Axon how to reach it
> model = "my-custom-model"
> api_backend = "responses"            # required — web search uses the Responses API
> # base_url, api_key, env_key optional — defaults to cli-chat-proxy
> ```

### Examples

**OpenAI-compatible endpoint:**

```toml
[model.local-llama]
model = "llama-3.1-70b"
base_url = "http://localhost:8080/v1"
name = "Local Llama"
temperature = 0.8
```

**Ollama:**

```toml
[model.ollama-codellama]
model = "codellama"
base_url = "http://localhost:11434/v1"
name = "CodeLlama (Ollama)"
```

**Together AI:**

```toml
[model.together-mixtral]
model = "mistralai/Mixtral-8x7B-Instruct-v0.1"
base_url = "https://api.together.xyz/v1"
name = "Mixtral 8x7B"
env_key = "TOGETHER_API_KEY"
```

**OpenAI:**

```toml
[model.gpt-4o]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
name = "GPT-4o"
env_key = "OPENAI_API_KEY"
```

### Using Custom Models

```bash
# List available models (including custom)
axon models

# Use in TUI via slash command
/model my-model

# Use in headless mode
axon -p "Hello" -m my-model

# Set as default
# In config.toml:
[models]
default = "my-model"
```

### Custom Models Endpoint

Point Axon at a custom OpenAI-compatible `/v1/models` endpoint. Useful when models are served behind a corporate gateway or self-hosted inference stack.

**Environment variables:**

| Variable | Required | Description |
|----------|----------|-------------|
| `AXON_MODELS_BASE_URL` | Yes | Base URL for inference / chat completions (e.g. `https://api.acme.com/v1`). The model list is fetched from `{base_url}/models` automatically |
| `AXON_API_KEY` | Yes | API key sent as `Authorization: Bearer` to the custom endpoint |
| `AXON_MODELS_LIST_URL` | No | Override the model list URL if it differs from `{base_url}/models` |

**Setup:**

```bash
export AXON_MODELS_BASE_URL="https://api.acme.com/v1"
export AXON_API_KEY="sk-..."
axon
```

Axon fetches the model list from `{AXON_MODELS_BASE_URL}/models` on startup and sends inference requests to `AXON_MODELS_BASE_URL`. This follows the standard OpenAI-compatible convention used by OpenAI, Anthropic, OpenRouter, Groq, Together.ai, and others.

If your model list endpoint differs from `{base_url}/models`, set `AXON_MODELS_LIST_URL` explicitly.

**Combining with `[endpoints]` config:** You can also set endpoints in `~/.axon/config.toml`:

```toml
[endpoints]
models_base_url = "https://api.acme.com/v1"

# Override just the API key for a specific model
[model.axon-build]
api_key = "my-api-key"
```

When using `[endpoints]` with partial model overrides, the `base_url` is inherited from the endpoints config — you don't need to specify it in each `[model.*]` section.

**Auth behavior:** When `models_base_url` is set, Axon uses API key auth (`Authorization: Bearer`) instead of session auth. `axon login` is not required — only the API key.

---

## MCP Servers

Extend Axon's capabilities with [Model Context Protocol](https://modelcontextprotocol.io) servers.

### Configuration

MCP servers are configured in `~/.axon/config.toml`:

```toml
[mcp_servers.<name>]
command = "/path/to/server"           # Server executable
args = ["--flag", "value"]            # Command arguments
env = { VAR = "value" }               # Environment variables
headers = { "X-Header" = "value" }    # Optional HTTP headers (Streamable HTTP)
enabled = true                        # Enable/disable (default: true)
startup_timeout_sec = 30              # Init timeout (default: 30)
tool_timeout_sec = 60                 # Tool call timeout (default: 60)
tool_timeouts = { create_issue = 120, search = 30 }  # Per-tool timeout overrides (seconds)
```

### Project-Scoped MCP Servers

MCP servers can also be configured per-project in `.axon/config.toml`. Axon walks from the current directory up to the git repo root, loading `.axon/config.toml` at each level:

| Location                        | Scope             | Priority |
| ------------------------------- | ----------------- | -------- |
| `~/.axon/config.toml`           | All projects      | Lowest   |
| `<repo-root>/.axon/config.toml` | This repository   | ↑        |
| `<cwd>/.axon/config.toml`       | Current directory | Highest  |

If a project defines a server with the same name as a global one, the project version **replaces** it entirely (fields are not merged — omitted fields get defaults, not the global values). Servers defined only in the global config are unaffected.

**Example:** commit a `.axon/config.toml` in your repo to share MCP servers across the team:

```
my-project/
├── .axon/
│   └── config.toml
├── src/
└── ...
```

```toml
# .axon/config.toml
[mcp_servers.linear]
command = "npx"
args = ["-y", "mcp-remote", "https://mcp.linear.app/mcp"]
```

If you also have a `linear` server in `~/.axon/config.toml`, the project version replaces it entirely.

> **Note:** Only `[mcp_servers]` is supported in project-scoped `.axon/config.toml`. Other config sections (models, shortcuts, etc.) are only read from `~/.axon/config.toml`.

### Tool Naming

MCP tools are namespaced with the server name:

- Server `filesystem` with tool `read_file` → `filesystem__read_file`
- Server `github` with tool `create_issue` → `github__create_issue`

### Example Servers

**Filesystem access:**

```toml
[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed/directory"]
```

**GitHub integration:**

```toml
[mcp_servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_xxxxxxxxxxxx" }
```

**Postgres database:**

```toml
[mcp_servers.postgres]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres", "postgresql://user:pass@localhost/db"]
```

**Custom server:**

```toml
[mcp_servers.my-tools]
command = "/usr/local/bin/my-mcp-server"
args = ["--config", "/etc/my-mcp.json"]
startup_timeout_sec = 30
tool_timeout_sec = 120
```

**Streamable HTTP with session id header:**

```toml
[mcp_servers.my-http-mcp]
url = "http://localhost:5000/api/mcp"
headers = { "x-session-id" = "{{session_id}}" }
```

### Available MCP Servers

See the [MCP Server Registry](https://github.com/modelcontextprotocol/servers) for community servers:

- Filesystem, Git, GitHub, GitLab
- PostgreSQL, SQLite, Redis
- Slack, Discord, Linear
- Puppeteer, Playwright
- And many more

---

## Memory

> **Experimental:** requires `--experimental-memory` (or `AXON_MEMORY=1` / `[memory] enabled = true` in config).

Cross-session memory lets Axon remember facts, decisions, code patterns, and debugging workflows across separate sessions in the same project.

### How it works

Memory is stored as Markdown files under `~/.axon/memory/`:
- **Global** (`~/.axon/memory/MEMORY.md`) — facts that apply across all your projects
- **Workspace** (`~/.axon/memory/<project-slug>-<hash8>/MEMORY.md`) — project-specific conventions and context
- **Session logs** (`~/.axon/memory/<project-slug>-<hash8>/sessions/`) — per-session summaries

Workspace directories are suffixed with a short hash for uniqueness (e.g. `axon-a3f7b2c9/`). The hash is derived from the git remote URL so all clones and worktrees of the same repository share the same memory directory.

An SQLite index enables fast hybrid search (FTS5 keyword + optional vector KNN) across all memory files.

### Enabling memory

```bash
# Per-session flag
axon --experimental-memory

# Environment variable (persists for the shell session)
export AXON_MEMORY=1
axon

# Config file (persists permanently)
# ~/.axon/config.toml
[memory]
enabled = true
```

### What gets saved automatically

At the end of each session, Axon saves a **structured metadata summary** to the daily session log:
- Message counts (user / assistant / tool)
- Topics — the first few real user prompts from the session
- Tool-usage breakdown (e.g., `read_file: 4, search_replace: 3`)
- File paths that were read or edited
- Date and session ID

Shell commands are intentionally **not** recorded in automatic saves — command
strings often embed secrets (tokens, API keys, DSNs) and auto-save runs silently.
For command history, use `/flush`, which is user-initiated and produces an
LLM-generated summary rather than raw verbatim output.

This summary is searchable in future sessions but does **not** capture full content or reasoning.

### Capturing rich knowledge with `/flush`

For richer capture — decisions, patterns, debugging workflows, API discoveries — use `/flush` in the TUI. This triggers an LLM-generated summary of the current session's most important content and writes it to a dated session log under `~/.axon/memory/<project-slug>-<hash8>/sessions/`, where it is indexed and searchable in future sessions.

Use `/flush` when you want to preserve important context before compaction or at any point during a productive session.

```
/flush
```

### Appending to memory manually

You can append facts directly from the TUI without leaving the session:

```
/memory workspace Use Rust for all backend services.
/memory global Prefer 2-space indentation in TypeScript.
/memory global Preferred editor: VS Code with Vim keybindings.
```

Omit `workspace` or `global` and it defaults to workspace scope.

### Searching memory

Axon searches memory automatically on the first turn of each session and after compaction. The first-turn injection can be disabled or given its own score threshold under `[memory.initial_injection]`. You can also invoke `memory_search` and `memory_get` directly via the model prompt:

```
Search memory for "auth middleware patterns"
Read my workspace MEMORY.md
```

### CLI commands

```bash
# Open workspace MEMORY.md in $EDITOR / $VISUAL
axon memory edit

# Open global MEMORY.md
axon memory edit --global

# Show memory statistics: file count, chunk count, and index size
axon memory stats
```

### Configuration reference

Key options under `[memory]` in `~/.axon/config.toml`:

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable memory (can also be set via CLI flag or env var) |
| `session.save_on_end` | `true` | Write the lightweight metadata summary on session end |
| `watcher.enabled` | `true` | Watch `~/.axon/memory/` for external edits and reindex on search |
| `search.max_results` | `6` | Default number of memory results to return |
| `search.min_score` | `0.35` | Minimum relevance score threshold for explicit memory search and recovery paths |
| `initial_injection.enabled` | `true` | Enable automatic first-turn memory injection |
| `initial_injection.min_score` | `0.0` | Override score threshold for first-turn injection (`0.0` preserves historical no-filter behavior) |
| `embedding.model` | *(unset)* | Embedding model for vector search; unset disables embeddings |
| `embedding.dimensions` | `1024` | Embedding vector dimensions |

---

## Sandbox

Axon can restrict what the agent process and its spawned commands can access on
your filesystem and network using OS-level kernel primitives (Landlock on Linux,
Seatbelt on macOS). This is off by default.

### Quick Start

```bash
# Run with workspace sandbox (read everywhere, write only to CWD + /tmp)
axon --sandbox workspace

# Read-only mode (agent can read but not write anything)
axon --sandbox read-only

# Maximum isolation (read/write CWD only, no child network)
axon --sandbox strict
```

### Built-in Profiles

| Profile         | FS Read            | FS Write                  | Child Network | Use Case                 |
| --------------- | ------------------ | ------------------------- | ------------- | ------------------------ |
| `off` (default) | Unrestricted       | Unrestricted              | Unrestricted  | No sandbox               |
| `workspace`     | Everywhere         | CWD + `/tmp` + `~/.axon/` | Allowed       | Normal development       |
| `read-only`     | Everywhere         | `~/.axon/` only           | Blocked       | Exploration, code review |
| `strict`        | CWD + system paths | CWD + `/tmp` + `~/.axon/` | Blocked       | Untrusted code           |

Sensitive paths (`~/.ssh/`, `~/.aws/`, `~/.gnupg/`, `~/.axon/auth/`) are always
write-protected regardless of profile.

### Custom Profiles

Create `~/.axon/sandbox.toml` (global) or `.axon/sandbox.toml` (per-project):

```toml
[profiles.devbox]
# Start from a built-in profile, then add overrides
extends = "workspace"
restrict_network = true

# Paths the agent can read but NOT write/delete
read_only = ["/data"]

# Additional writable paths
read_write = ["/tmp/scratch"]

# Paths denied entirely
deny = ["/data/shared-secrets"]
```

Use it:

```bash
axon --sandbox devbox
```

### How It Works

The sandbox is applied to the **entire axon process** at startup using kernel
primitives — not per-command wrapping. This means all tool operations are
covered:

- `read_file`, `search_replace`, `list_dir` — restricted by Landlock/Seatbelt in-process
- `bash` commands, `grep` (rg) — child processes inherit FS restrictions automatically
- Network — child processes can be blocked via seccomp (Linux)

The sandbox is **irreversible** once applied. This is a security feature — the
model cannot convince the agent to relax restrictions at runtime.

### Current Limitations

- **Platform support**: Sandbox enforcement uses Landlock on Linux (kernel ≥ 5.13)
  and Seatbelt on macOS. If the sandbox cannot be applied (e.g., unsupported
  kernel, missing entitlements), Axon logs a warning and continues without
  enforcement.

- **Network restrictions are partial**: Profiles with `restrict_network` block
  network in **child processes** (bash commands, scripts) via seccomp, but
  built-in tools that make HTTP requests in-process (web search, LLM API) are
  not affected. The agent needs network access to function, so process-level
  network cannot be blocked.

### Event Logging

Sandbox events (profile applied, violations) are logged to `~/.axon/sandbox-events.jsonl`
for local auditing and debugging.

---

## Introspection

Use `axon inspect` to see everything Axon discovers in the current directory:

```bash
axon inspect          # human-readable output
axon inspect --json   # machine-readable JSON
```

The output shows all loaded configuration organized by type:

- **Project Instructions** — AGENTS.md / CLAUDE.md files with token counts
- **Skills** — from `.axon/skills/`, `~/.axon/skills/`, plugins, and config paths
- **Agents** — built-in, user-defined, and plugin-provided subagents
- **Plugins** — discovered plugins with what each provides (skills, agents, hooks, MCPs)
- **MCP Servers** — from `config.toml`, plugins, `~/.claude.json`, and `.mcp.json`
- **LSP Servers** — language servers from `lsp.json` and plugins
- **Hooks** — project and plugin hooks
- **Permissions, Config Sources** — which config files are active

Plugin-provided components appear in their respective sections with a `[plugin: name]` tag, so you can see at a glance where each skill, MCP server, or agent originates.

---

## Claude Code Compatibility

Axon automatically discovers configuration from Claude Code directories alongside native `.axon/` paths. No extra setup is needed.

### What is picked up

| Component         | Claude Code location                                 | How Axon uses it                 |
| ----------------- | ---------------------------------------------------- | -------------------------------- |
| **Skills**        | `.claude/skills/`, `~/.claude/skills/`               | Loaded as skills (same as `.axon/skills/`) |
| **Agents**        | `.claude/agents/`, `~/.claude/agents/`               | Loaded as subagents              |
| **Plugins**       | `.claude/plugins/`, `~/.claude/plugins/`             | Discovered with all components   |
| **Installed plugins** | `~/.claude/plugins/installed_plugins.json`        | Each `installPath` is loaded     |
| **Marketplaces**  | `~/.claude/plugins/known_marketplaces.json`          | Plugin dirs from `installLocation` |
| **MCP servers**   | `~/.claude.json`, `.mcp.json`                        | Loaded alongside `config.toml`   |
| **Project rules** | `CLAUDE.md`, `.claude/CLAUDE.md`                     | Loaded as project instructions   |
| **Permissions**   | `.claude/settings.json`, `.claude/settings.local.json` | Fallback when no TOML config   |

### Plugin components

Claude Code plugins can provide skills (`skills/`), commands (`commands/`), agents (`agents/`), hooks (`hooks/hooks.json`), MCP servers (`.mcp.json`), and LSP servers (`.lsp.json`). All component types are discovered and used by Axon at runtime.

---

## Built-in Tools

Axon includes these tools by default:

| Tool             | Description                                                    |
| ---------------- | -------------------------------------------------------------- |
| `read_file`      | Read file contents with line numbers                           |
| `search_replace` | Make precise edits to files                                    |
| `grep_search`    | Search with regex patterns (ripgrep)                           |
| `list_dir`       | List directory contents                                        |
| `bash`           | Execute shell commands                                         |
| `web_search`     | Search the web for up-to-date information                      |
| `web_fetch`      | Fetch a specific URL and return its content as markdown        |
| `todo_write`     | Create and manage task lists                                   |
| `task`           | Launch subagent sessions (requires `--subagents`)              |
| `kill_task`      | Terminate a running background task or subagent                |
| `get_task_output` | Get output and status from a background task or subagent      |
| `memory_search`  | Search cross-session memory (requires `--experimental-memory`) |
| `memory_get`     | Read a memory file by path                                     |
| `search_tool`    | Discover available integration tools (MCP)                     |
| `use_tool`       | Call an integration tool discovered via `search_tool`           |
| `lsp`            | Code intelligence via language servers (requires `lsp_tools`)  |

### Controlling Available Tools

In headless mode, you can restrict or remove tools with the `--tools` (allowlist) and `--disallowed-tools` (denylist) flags. See [Headless Mode](#headless-mode) for details and examples.

In agent profiles, use the `tools` and `disallowedTools` frontmatter fields:

```yaml
---
tools:
  - read_file
  - grep_search
  - list_dir
disallowedTools:
  - web_search
  - Agent(explore)
---
```

### `web_fetch`

Fetch a specific URL and return its content as markdown. **Disabled by default** — enable with `AXON_WEB_FETCH=1`. 

When no custom `allowed_domains` is set, the tool permits a default allowlist of useful documentation sites (language docs, frameworks, cloud providers, databases, etc.). Domains not on the allowlist prompt the user for approval; `--always-approve` auto-approves all. Domain matching is case-insensitive, strips `www.` prefixes, and supports path-scoped entries (e.g. `example.com/docs`).

---

## Session Persistence

Axon automatically persists conversations to disk. This works across all modes: TUI, headless, and agent stdio.

### Storage Layout

Sessions are stored under `~/.axon/sessions/`, organized by URL-encoded working directory:

```
~/.axon/sessions/<encoded-cwd>/<session-id>/
  summary.json            # metadata: title, timestamps, model, message count
  updates.jsonl           # ACP session update stream (conversation + tool calls)
  chat_history.jsonl      # raw chat messages sent to the model
  plan.json               # TODO/task list state
  rewind_points.jsonl     # file snapshots for /rewind undo
  signals.json            # session signals (turn count, token usage)
  feedback.jsonl          # user feedback and ratings
  compaction_checkpoints/ # saved state from auto-compact
  subagents/              # child session directories (when subagents are enabled)
```

`summary.json` is the index entry — it contains the session title, model ID, creation/update timestamps, and parent session reference (for restored sessions). `updates.jsonl` is the authoritative conversation log that drives `/load` and session restore.

### TUI

Sessions persist automatically as you chat. To start fresh:

```
/new
```

The TUI creates a new session each time you launch unless you continue a previous one.

### Headless Mode

Control session behavior with flags:

```bash
# New session each time (default)
axon -p "Hello"

# Create or resume a named session
axon -p "Remember: X=42" -s my-session
axon -p "What is X?" -s my-session

# Resume existing session (errors if not found)
axon -p "Continue" -r my-session

# Continue most recent session in current directory
axon -p "What were we doing?" -c
```

Session ID is returned in JSON output:

```bash
axon -p "Hello" --output-format json | jq -r '.sessionId'
```

### Agent stdio (ACP)

When building with ACP, sessions are managed via protocol methods:

```typescript
// Create new session
const { sessionId } = await connection.request("session/new", {
  cwd: "/path/to/project",
  mcpServers: [],
});

// Load existing session
await connection.request("session/load", {
  sessionId: "existing-session-id",
  cwd: "/path/to/project",
  mcpServers: [],
});
```

The agent persists all session updates automatically. Clients can reconnect and load previous sessions by ID.

---

## File Locations

| Path                  | Description                                         |
| --------------------- | --------------------------------------------------- |
| `~/.axon/config.toml` | Configuration file                                  |
| `~/.axon/sessions/`   | Persisted sessions (organized by working directory) |
| `~/.axon/auth.json`   | Authentication credentials (auto-managed)           |
| `~/.axon/memory/`     | Cross-session memory files and index                |
| `~/.axon/skills/`     | User-scoped skill definitions                       |
| `~/.axon/plugins/`    | User-scoped plugins                                 |
| `~/.axon/agents/`     | User-scoped agent definitions                       |
| `.axon/config.toml`   | Project-scoped config (MCP servers)                 |
| `.axon/skills/`       | Project-scoped skill definitions                    |
| `.axon/plugins/`      | Project-scoped plugins                              |
| `.axon/agents/`       | Project-scoped agent definitions                    |
| `.axon/hooks/`        | Project-scoped hooks                                |
| `.axon/lsp.json`      | LSP server configuration                            |
| `~/.claude/skills/`   | User-scoped skills (Claude Code compat)             |
| `~/.claude/plugins/`  | User-scoped plugins (Claude Code compat)            |
| `~/.claude.json`      | MCP servers (Claude Code compat)                    |
| `.mcp.json`           | Project-scoped MCP servers (Claude Code compat)     |

---

## Environment Variables

| Variable                         | Description                                                                                              |
| -------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `AXON_API_KEY`         | Global BYOK API key. Used for custom-endpoint auth and as the fallback for any model without its own `api_key`/`env_key` |
| `AXON_CLI_CHAT_PROXY_BASE_URL`  | Base URL for the OpenAI-compatible chat proxy used with OIDC/session auth. No default in this fork — set it to your own proxy |
| `AXON_MODELS_BASE_URL`          | Custom base URL for inference. Model list auto-fetched from `{base_url}/models` (see [Custom Models Endpoint](#custom-models-endpoint)) |
| `AXON_MODELS_LIST_URL`          | Override the model list URL if it differs from `{AXON_MODELS_BASE_URL}/models`                                              |
| `AXON_AUTH_PROVIDER_COMMAND`     | External auth binary (alternative to config file). See [External Auth Provider](#external-auth-provider) |
| `AXON_AUTH_TOKEN_TTL`            | Token lifetime in seconds for external auth providers that output bare tokens. See [External Auth Provider](#external-auth-provider) |
| `AXON_AUTH_EARLY_INVALIDATION_SECS` | Seconds before `expires_at` to consider a token expired (default: `300`). See [Automatic Credential Refresh](#automatic-credential-refresh) |
| `AXON_OIDC_ISSUER`              | OIDC issuer URL (alternative to config file). See [OIDC](#oidc-enterprise-sso-against-your-own-idp)                             |
| `AXON_OIDC_CLIENT_ID`           | OIDC client ID (alternative to config file). See [OIDC](#oidc-enterprise-sso-against-your-own-idp)                              |
| `AXON_HOME`                     | Override config directory (default: `~/.axon`)                                                           |
| `AXON_SUBAGENTS`                | Enable (`1`) or disable (`0`) subagent/task tool support                                                 |
| `AXON_MEMORY`                   | Enable (`1`) or disable (`0`) cross-session memory                                                       |
| `AXON_AGENT`                    | Custom agent definition path or name (see [Agent Profiles](#agent-profiles))                             |
| `AXON_WEB_FETCH`                | Enable (`1`) or disable (`0`) the `web_fetch` tool                                                       |
| `AXON_WEB_FETCH_PROXY`          | Egress proxy URL for `web_fetch` requests (overridden by `[toolset.web_fetch] proxy_endpoint`)           |
| `AXON_RESPECT_GITIGNORE`        | Disable `.gitignore` filtering in tools when set to `0`                                                  |
| `AXON_DEPLOYMENT_KEY`           | Management API key for enterprise deployments                                                            |
| `AXON_LOG_FILE`                 | Enable file logging by providing a file path (the value is used verbatim as the path)                    |
| `AXON_DEBUG_LOG`                | Debug firehose (set by `--debug`): truthy routes per-session logs to `~/.axon/debug/<sessionId>.txt`, a path writes that one file |
| `RUST_LOG`                      | Log filter for stderr (headless `-p` defaults to `off`, other non-TUI modes to `error`; TUI captures stderr) and for the `AXON_LOG_FILE` log; the `--debug` firehose ignores it |

---

## Shell Completions

Generate completions for your shell and install them to enable tab completion for `axon` commands and flags.

**Note:** The paths below are recommended defaults. Some environments do not automatically source the standard locations — you may need to adapt them to your shell framework or distro conventions.

### Bash

Generate and install:

```bash
mkdir -p ~/.local/share/bash-completion/completions
axon completions bash > ~/.local/share/bash-completion/completions/axon
```

Reload your shell or run `source ~/.bashrc`.

Alternative (Axon-managed location):

```bash
mkdir -p ~/.axon/completions/bash
axon completions bash > ~/.axon/completions/bash/axon.bash
```

Add to `~/.bashrc`:

```bash
[[ -r "$HOME/.axon/completions/bash/axon.bash" ]] && source "$HOME/.axon/completions/bash/axon.bash"
```

### Zsh

Generate and install:

```bash
mkdir -p ~/.zsh/completions
axon completions zsh > ~/.zsh/completions/_axon
```

Add to `~/.zshrc`:

```zsh
fpath=(~/.zsh/completions $fpath)
autoload -Uz compinit
compinit
```

Alternative (Axon-managed location):

```bash
mkdir -p ~/.axon/completions/zsh
axon completions zsh > ~/.axon/completions/zsh/_axon
```

Add to `~/.zshrc`:

```zsh
fpath=("$HOME/.axon/completions/zsh" $fpath)
autoload -Uz compinit
compinit
```

### After Upgrading

Regenerate completions after upgrading `axon` — the script reflects the CLI of the installed version.

---

## Troubleshooting

### Debug logging

Write logs to a file for debugging. The TUI captures stderr, so `RUST_LOG` alone won't produce visible output in production — use `axon --debug` or `AXON_LOG_FILE` instead:

```bash
# Per-session debug log (~/.axon/debug/<sessionId>.txt)
axon --debug

# Log to a custom path
AXON_LOG_FILE=/tmp/axon-debug.log axon

# Tail the most-recently-opened session's log in another terminal (Unix symlink)
tail -f ~/.axon/debug/latest.txt
```

The `--debug` firehose uses a fixed filter (first-party crates at `debug`) and is not narrowed by `RUST_LOG`. A `AXON_LOG_FILE` log defaults to `debug` and honors `RUST_LOG`, so you can set module-level filters for targeted debugging:

```bash
# Debug auth, info for everything else
AXON_LOG_FILE=/tmp/axon-debug.log RUST_LOG="info,axon_shell::auth=debug" axon
```

### Authentication fails

```bash
# Clear credentials and re-login
axon login

# Debug auth issues — check the log for "auth:" entries
axon --debug-file /tmp/axon-auth.log -p "hello"
grep "auth:" /tmp/axon-auth.log
```

### Model not found

```bash
# List available models
axon models

# Check config.toml for typos in [model.*] sections
```

### MCP server not starting

```bash
# Test the server command manually
npx -y @modelcontextprotocol/server-filesystem /path

# Increase startup timeout in config
[mcp_servers.filesystem]
startup_timeout_sec = 30
```

### Command timeout

```toml
# Increase bash timeout in config.toml
[toolset.bash]
timeout_secs = 300.0
```

### Inspecting session data

Session files are plain JSON/JSONL and can be inspected directly:

```bash
# Find sessions for the current directory
ls ~/.axon/sessions/

# Read session metadata
cat ~/.axon/sessions/<encoded-cwd>/<session-id>/summary.json | jq .

# View conversation history
cat ~/.axon/sessions/<encoded-cwd>/<session-id>/updates.jsonl | head -20

# Count turns in a session
wc -l ~/.axon/sessions/<encoded-cwd>/<session-id>/chat_history.jsonl
```

### Context window full

If auto-compact triggers too often, lower the threshold to compact earlier and preserve more headroom:

```toml
[session]
auto_compact_threshold_percent = 70    # default is 85
```

---

## License

Licensed under the Apache License, Version 2.0. See the repository root
`LICENSE` file.
