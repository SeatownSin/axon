# grok-build-local

A **local-first, privacy-focused fork** of **Grok Build** (`grok`) — xAI's
terminal-based AI coding agent — modified so it **never contacts xAI
infrastructure** and runs entirely against **local or third-party
(BYOK) models**.

> **Not affiliated with, endorsed by, or supported by xAI.** This is an
> independent modification of xAI's Apache-2.0-licensed source. See
> [Relationship to upstream](#relationship-to-upstream).

It runs as a full-screen TUI that understands your codebase, edits files,
executes shell commands, and manages long-running tasks — interactively,
headlessly for scripting/CI, or embedded in editors via the Agent Client
Protocol (ACP). The binary artifact is `xai-grok-pager` and installs as `grok`.

[What's different](#whats-different-from-upstream) ·
[Building](#building-from-source) ·
[Local models](#configuring-a-local-model) ·
[Updates](#updates) ·
[Testing](#running-the-tests) ·
[Upstream & license](#relationship-to-upstream)

---

## What's different from upstream

This fork removes every path that sends data to, or pulls data from, xAI/Grok
servers, and adds first-class support for local models. The changes:

- **No xAI network egress — enforced at the network boundary.** A shared
  predicate refuses any request to `x.ai`/`grok.com` (and subdomains) at the
  point a socket would open: the inference client, OIDC login **and** token
  refresh, device-code login, the model-catalog and subagent-bundle fetches,
  managed-config, the sandbox/relay/workspace backends, memory embeddings,
  voice STT, and session-storage upload. No config, env var, or remote setting
  can re-enable it.
- **Telemetry and phone-home removed.** The Mixpanel crate is deleted; product
  analytics, OTLP trace export, Sentry, the feedback/session-signals API, the
  startup announcements/settings prefetch, the changelog CDN pull, billing/
  paywall checks, and automatic update polling are all gone or hard-disabled.
- **Local & BYOK models, no login.** Point a `[model.*]` entry at any
  OpenAI-compatible endpoint. Loopback servers (Ollama, llama.cpp, LM Studio,
  vLLM at `localhost`/`127.0.0.1`/`[::1]`) are auto-detected as no-auth: no API
  key, no browser login, and your session token is never sent to them.
  `context_window` is optional (defaults to 200k). See
  [Configuring a local model](#configuring-a-local-model).
- **Grok models hidden.** The xAI-hosted default models are hidden from the
  picker (they're unusable here); your local/BYOK models are all that show.
- **Windows build support.** The proto codegen no longer depends on
  `/dev/stdout`, so the workspace builds natively on Windows.
- **Updates from this repo.** `grok update` pulls GitHub Releases from
  `SeatownSin/grok-build-local`, not the x.ai CDN.

The inference request path itself is unchanged and provider-neutral (OpenAI
Chat Completions / Responses, or Anthropic Messages) — only *where* it is
allowed to connect changed.

## Building from source

Requirements:

- **Rust** — the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml);
  `rustup` installs it automatically on first build.
- **protoc** — proto codegen needs Protocol Buffers.
  - *macOS / Linux:* [`bin/protoc`](bin/protoc) resolves via
    [DotSlash](https://dotslash-cli.com) (`cargo install dotslash`), or falls
    back to a `protoc` on `PATH`.
  - *Windows:* the `bin/protoc` DotSlash shim is Linux-only — install
    [protoc](https://github.com/protocolbuffers/protobuf/releases) and put it on
    `PATH` or set `PROTOC` to its full path.

```sh
cargo run -p xai-grok-pager-bin              # build + launch the TUI
cargo build -p xai-grok-pager-bin --release  # release binary: target/release/xai-grok-pager
cargo check -p xai-grok-pager-bin            # fast validation
```

On first launch there is **no login** — configure a local or BYOK model (below)
and start a session. There is no browser auth flow to xAI in this build.

## Configuring a local model

Add a model to `~/.grok/config.toml`. A loopback endpoint needs nothing else —
no key, no login:

```toml
[model.local]
model = "your-model-id"                 # slug your server expects
base_url = "http://localhost:11434/v1"  # Ollama / llama.cpp / LM Studio / vLLM
name = "Local model"                    # shown in the picker
context_window = 8192                   # optional; defaults to 200000

[models]
default = "local"                       # make it the default for new sessions
```

For a non-loopback server that also needs no auth, set `no_auth = true`. For a
keyed provider (OpenAI, Anthropic, …), set `api_key`/`env_key` and `base_url` as
usual. Full details:
[`docs/user-guide/11-custom-models.md`](crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md).

## Updates

`grok update` checks **GitHub Releases** on this repo
(`SeatownSin/grok-build-local`) via the `gh` CLI. Publish releases with a
`v<version>` tag and assets named `grok-<version>-<os>-<arch>` (a `.exe` suffix
is also accepted on Windows). Automatic on-launch update checks are removed;
`grok update` is explicit only.

## Running the tests

Most of the test suite assumes a Unix layout (hard-coded `/tmp` paths in
helpers, advisory file locking), so **~600 tests fail on Windows-native for
harness reasons, not product bugs**. Run the suite under **WSL2 / Linux** for a
clean signal:

```sh
PROTOC=/path/to/protoc cargo test -p xai-grok-shell --lib
```

A `.gitattributes` pins LF line endings so a Windows checkout doesn't break the
pinned-copy template tests.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/codegen/xai-grok-pager-bin` | Composition-root package; builds the `xai-grok-pager` binary |
| `crates/codegen/xai-grok-pager` | The TUI: scrollback, prompt, modals, rendering |
| `crates/codegen/xai-grok-shell` | Agent runtime + leader/stdio/headless entry points |
| `crates/codegen/xai-grok-tools` | Tool implementations (terminal, file edit, search, ...) |
| `crates/codegen/xai-grok-workspace` | Host filesystem, VCS, execution, checkpoints |
| `crates/codegen/...` | The rest of the CLI crate closure (config, MCP, markdown, sandbox, ...) |
| `crates/common/`, `crates/build/`, `prod/mc/` | Small shared leaf crates pulled in by the closure |
| `third_party/` | Vendored upstream source (Mermaid diagram stack) |

> [!IMPORTANT]
> The root `Cargo.toml` (workspace members, dependency versions, lints,
> profiles) is **generated** upstream — prefer editing per-crate `Cargo.toml`
> files.

## Development

```sh
cargo check -p <crate>        # always target specific crates; full-workspace builds are slow
cargo test -p xai-grok-config # per-crate tests (see "Running the tests" re: WSL)
cargo clippy -p <crate>       # lint config: clippy.toml at the repo root
cargo fmt --all               # rustfmt.toml at the repo root
```

## Relationship to upstream

This repository is a modified fork of xAI's **Grok Build**, published by xAI at
[x.ai/cli](https://x.ai/cli) under the Apache License, Version 2.0. The upstream
tree this fork is based on is recorded as commit
[`f9736c7`](SOURCE_REV) (the SpaceXAI monorepo SHA in [`SOURCE_REV`](SOURCE_REV)).

The modifications are summarized in [What's different](#whats-different-from-upstream)
and captured in this repository's git history. Upstream documentation lives at
[docs.x.ai/build/overview](https://docs.x.ai/build/overview) and largely still
applies, **except** where this fork changes behavior (authentication, model
selection, updates, telemetry). "Grok" and "xAI" are trademarks of their
respective owner; their use here is nominative, to identify the upstream work.

## License

First-party code is licensed under the **Apache License, Version 2.0** — see
[`LICENSE`](LICENSE). Per Apache-2.0 §4(b), this fork carries modifications to
xAI's original files; the changes are described above and in the git history.

Third-party and vendored code remains under its original licenses:

- [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES) — crates.io / git dependencies,
  bundled UI themes, and in-tree source ports (including openai/codex and
  sst/opencode tool implementations)
- [`crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md`](crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md)
  — crate-local notice for the codex and opencode ports
- [`third_party/NOTICE`](third_party/NOTICE) — vendored Mermaid-stack index
