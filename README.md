# Axon

[![CI](https://github.com/SeatownSin/axon/actions/workflows/ci.yml/badge.svg)](https://github.com/SeatownSin/axon/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/SeatownSin/axon?sort=semver)](https://github.com/SeatownSin/axon/releases/latest)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

**Axon** is a **local-first, privacy-focused fork** of **Grok Build** — xAI's
terminal-based AI coding agent — rebranded and modified so it makes **no
network calls to xAI infrastructure** and runs entirely against **local or
third-party (BYOK) models**. It is published as the
[`axon`](https://github.com/SeatownSin/axon) repository.

> **Not affiliated with, endorsed by, or supported by xAI.** This is an
> independent modification of xAI's Apache-2.0-licensed source. See
> [Relationship to upstream](#relationship-to-upstream).

It runs as a full-screen TUI that understands your codebase, edits files,
executes shell commands, and manages long-running tasks — interactively,
headlessly for scripting/CI, or embedded in editors via the Agent Client
Protocol (ACP). The build artifact is `axon-pager` and installs as the `axon`
command; its config lives in `~/.axon`.

[What's different](#whats-different-from-upstream) ·
[Install](#install) ·
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
- **Rebranded as Axon.** The welcome screen (a new mark), model picker,
  notifications, and theme names carry the Axon identity — no `grok`/`xAI`
  branding is shown in the UI. The bundled themes are **Axon Night** (a cerebral
  cool-slate default) and **Axon Day**. The rename runs all the way down: the
  `axon` command, the `axon-*` crates, `~/.axon`, `AXON_*` env vars, and the
  theme names. `AXON_API_KEY` and `AXON_HOME` are the only names read — there
  are no pre-rename aliases.
- **First-run setup wizard.** With no model configured, launch drops into a
  short wizard that scans **`localhost` and your local network** for running
  model servers — probing the ports actually in use, so it finds servers on
  non-standard/dynamic ports (LM Studio, for one, rarely sits on its documented
  default) — and writes the config for you, replacing the (removed) login
  screen.
- **Windows support.** The proto codegen no longer depends on `/dev/stdout`, so
  the workspace builds natively on Windows — and the app runs natively there
  too (the async runtime is given a large stack, so the composed entrypoint
  doesn't overflow the small Windows main-thread stack at startup).
- **Updates from this repo.** `axon update` pulls GitHub Releases from
  `SeatownSin/axon`, not the x.ai CDN.

The inference request path itself is unchanged and provider-neutral (OpenAI
Chat Completions / Responses, or Anthropic Messages) — only *where* it is
allowed to connect changed.

> **The plugin marketplace ships pointing at nobody.** Upstream hardcoded
> `github.com/xai-org/plugin-marketplace` as the "official" source, registered
> it into your config on first run, and cloned it when you accepted a plugin
> suggestion. That default is gone: name your own source with
> `[marketplace] official_source` (or `AXON_MARKETPLACE_OFFICIAL_SOURCE`) and
> it gets that status. Unconfigured, nothing is registered and nothing is
> fetched.

## Install

Prebuilt binaries are attached to every [release](https://github.com/SeatownSin/axon/releases/latest)
for **Linux and Windows** on `x86_64` and `aarch64`. There is no prebuilt macOS
binary (the runners are billed for this account) — macOS users
[build from source](#building-from-source), which is fully supported.

```sh
# macOS / Linux / Git-Bash / WSL — installs to ~/.axon/bin
curl -fsSL https://raw.githubusercontent.com/SeatownSin/axon/main/crates/codegen/axon-pager/scripts/install.sh | bash
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/SeatownSin/axon/main/crates/codegen/axon-pager/scripts/install.ps1 | iex
```

Both scripts download from this repo's GitHub Releases and touch no xAI
infrastructure. Pass a version to pin one (`... | bash -s 0.3.1`); set
`AXON_BIN_DIR` to install elsewhere. Or just grab the asset for your platform
and put it on your `PATH` — it is a single static binary named
`axon-<version>-<os>-<arch>`.

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
cargo run -p axon-pager-bin              # build + launch the TUI
cargo build -p axon-pager-bin --release  # release binary: target/release/axon-pager
cargo check -p axon-pager-bin            # fast validation
```

**First launch.** With no model configured, the first run drops into a short
setup wizard: it scans `localhost` **and your local network** for running model
servers (Ollama, LM Studio, llama.cpp, vLLM) — probing the ports actually
listening, so it finds servers on non-standard ports too — lets you pick a
detected model or enter an endpoint manually, writes it to
`~/.axon/config.toml`, and starts straight into a session. Quit the wizard and
it exits cleanly. Prefer to set things up ahead of time? Configure a model up
front ([below](#configuring-a-local-model)) and launch goes directly to a
session — no wizard, no login. There is no browser auth flow to xAI in this
build.

## Configuring a local model

The first-run wizard writes this for you (auto-detecting servers on `localhost`
and your LAN, on any port), but you can also add or edit models in
`~/.axon/config.toml` by hand. A loopback endpoint needs nothing else — no key,
no login:

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
usual.

If the server hosts a **reasoning** model, it may need to be told to separate
the chain-of-thought from the answer — vLLM's reasoning parsers stay inert
otherwise, leaving the reasoning inside `content` where it is stored and re-sent
as history on every later turn:

```toml
chat_template_kwargs = { enable_thinking = true }
```

Full details:
[`docs/user-guide/11-custom-models.md`](crates/codegen/axon-pager/docs/user-guide/11-custom-models.md).

## Updates

`axon update` checks **GitHub Releases** on this repo
(`SeatownSin/axon`) via the `gh` CLI. Publish releases with a
`v<version>` tag and assets named `axon-<version>-<os>-<arch>` (a `.exe` suffix
is also accepted on Windows). Automatic on-launch update checks are removed;
`axon update` is explicit only.

## Running the tests

The workspace **builds** on Linux and Windows, and CI gates both. Test *runs*
are a different story: much of the suite assumes a Unix layout (hard-coded
`/tmp` paths in helpers, advisory file locking), so **~600 tests fail on
Windows-native for harness reasons, not product bugs**. Run the suite under
**WSL2 / Linux** for a clean signal:

```sh
PROTOC=/path/to/protoc cargo test -p axon-shell --lib
```

Working from a Windows checkout over `/mnt`? Point `CARGO_TARGET_DIR` at a
Linux-native path — building onto the 9p mount is dramatically slower.

A `.gitattributes` pins LF line endings so a Windows checkout doesn't break the
pinned-copy template tests.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/codegen/axon-pager-bin` | Composition-root package; builds the `axon-pager` binary |
| `crates/codegen/axon-pager` | The TUI: scrollback, prompt, modals, rendering |
| `crates/codegen/axon-shell` | Agent runtime + leader/stdio/headless entry points |
| `crates/codegen/axon-tools` | Tool implementations (terminal, file edit, search, ...) |
| `crates/codegen/axon-workspace` | Host filesystem, VCS, execution, checkpoints |
| `crates/codegen/...` | The rest of the CLI crate closure (config, MCP, markdown, sandbox, ...) |
| `crates/common/`, `crates/build/`, `prod/mc/` | Small shared leaf crates pulled in by the closure |
| `third_party/` | Vendored upstream source (Mermaid diagram stack) |

> [!IMPORTANT]
> The root `Cargo.toml` (workspace members, dependency versions, lints,
> profiles) is **generated** upstream — prefer editing per-crate `Cargo.toml`
> files.

## Development

```sh
cargo check -p <crate>     # always target specific crates; full-workspace builds are slow
cargo test -p axon-config  # per-crate tests (see "Running the tests" re: WSL)
cargo clippy -p <crate>    # lint config: clippy.toml at the repo root
cargo fmt --all            # rustfmt.toml at the repo root
```

CI runs `fmt`, `clippy`, and `cargo check --workspace --all-targets` on Linux
**and Windows**, with warnings promoted to errors. To see what it will see
before you push:

```sh
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
```

`--all-targets` matters — tests, examples and benches are where
platform-specific breakage hides, and a plain `cargo build` never compiles
them. One-time setup so `git blame` skips the workspace-wide rustfmt commit:

```sh
git config blame.ignoreRevsFile .git-blame-ignore-revs
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
- [`crates/codegen/axon-tools/THIRD_PARTY_NOTICES.md`](crates/codegen/axon-tools/THIRD_PARTY_NOTICES.md)
  — crate-local notice for the codex and opencode ports
- [`third_party/NOTICE`](third_party/NOTICE) — vendored Mermaid-stack index
