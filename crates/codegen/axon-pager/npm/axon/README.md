# Axon

A local-first, privacy-focused AI coding agent for your terminal. Fast, flicker-free TUI built for plans, subagents, and parallel work — running entirely against your local or BYOK models, with no calls to xAI infrastructure by default.

**[Repository](https://github.com/SeatownSin/axon)**

> Axon is an independent fork of xAI's Apache-2.0-licensed **Grok Build**. Not affiliated with, endorsed by, or supported by xAI.

## Install

```bash
npm i -g @axon-official/axon
```

## Get Started

```bash
# Launch the interactive TUI
axon

# Run a single task
axon -p "Explain this codebase"
```

On first launch, with no model configured, Axon drops into a short setup wizard that scans `localhost` and your local network for running model servers (Ollama, LM Studio, llama.cpp, vLLM) and writes the config for you. There is no browser auth flow. To configure a model by hand, edit `~/.axon/config.toml`:

```toml
[model.local]
model = "your-model-id"
base_url = "http://localhost:11434/v1"
name = "Local model"

[models]
default = "local"
```

## Update

```bash
axon update
```

Or if installed via npm:

```bash
npm i -g @axon-official/axon@latest
```

## Supported Platforms

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon (arm64), x86_64 |
| Linux | x86_64, arm64 |
| Windows | x86_64, arm64 |

## Documentation

Full documentation lives in the repository's
[`docs/user-guide`](https://github.com/SeatownSin/axon/tree/main/crates/codegen/axon-pager/docs/user-guide):
configuration, MCP servers, custom models, headless mode, agent mode, and more.
