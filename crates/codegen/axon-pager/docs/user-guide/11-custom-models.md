# Custom Models

Axon connects to custom model endpoints for alternative providers, self-hosted
models, and overriding built-in settings. This guide explains how to select
models, configure endpoints, and integrate third-party providers.

---

## Default Model

Axon is local-first and ships with no hosted default model. On first run the
setup wizard detects local OpenAI-compatible servers (Ollama, LM Studio,
llama.cpp, vLLM) and writes your choice to `~/.axon/config.toml` — a loopback
server needs no API key and no login. You can also add hosted providers with
your own key (BYOK). Once at least one model is configured, start a session.

List all available models:

```bash
axon models
```

---

## Selecting a Model

### CLI Flag

```bash
axon -p "Hello" -m local
```

### Slash Command

In the TUI, switch models during a session:

```
/model local
```

Or use the alias:

```
/m local
```

### Model Picker (Ctrl+M)

Press `Ctrl+M` from the scrollback pane to open the model picker. It lists all available models, both built-in and custom, and lets you switch with a single keystroke. With the prompt focused, `Ctrl+M` toggles multiline input instead -- use `/model` to switch without leaving the prompt.

### Config Default

Set a persistent default in `~/.axon/config.toml`:

```toml
[models]
default = "local"
```

---

## Supported API Backends

Axon supports three API backends. Set `api_backend` in your `[model.*]` config to choose which protocol the model uses:

| Value | API | Default |
|-------|-----|---------|
| `"chat_completions"` | OpenAI Chat Completions (`/v1/chat/completions`) | Yes |
| `"responses"` | OpenAI Responses (`/v1/responses`) | |
| `"messages"` | Anthropic Messages (`/v1/messages`) | |

When you omit `api_backend`, Axon uses `chat_completions`.

To send provider-specific authentication or version headers -- for example, Anthropic's `x-api-key` -- use the `extra_headers` field described below. Axon sends those headers verbatim with every request to the endpoint.

---

## Configuring Custom Models

Add custom model endpoints in `~/.axon/config.toml` under `[model.<name>]` sections:

```toml
[model.your-model-id]
model = "model-id"                        # Model identifier sent to the API
base_url = "https://api.example.com/v1"   # OpenAI-compatible endpoint
name = "Display Name"                     # Shown in the model picker
description = "Model description"          # Optional description
api_key = "sk-..."                        # API key for this provider (optional)
env_key = "OPENAI_API_KEY"                # Env var holding the API key (optional; string or array)
api_backend = "chat_completions"          # "chat_completions", "responses", or "messages"
temperature = 0.7                         # Sampling temperature
top_p = 0.95                              # Nucleus sampling parameter
max_completion_tokens = 8192              # Maximum tokens per response
context_window = 128000                   # Total context window in tokens
extra_headers = { "x-api-key" = "sk-..." } # Extra request headers, sent verbatim (optional)
```

### Credential Resolution

Axon resolves the API key in this order:

1. The `api_key` field in the model config
2. The environment variable(s) named by `env_key` — a single string or an array of names. The first set, non-empty value wins (for example `env_key = ["ANTHROPIC_AUTH_TOKEN", "LC_ANTHROPIC_AUTH_TOKEN"]` for SSH `LC_*` forwarding)
3. An active session token (from an external auth provider or enterprise OIDC), for a model with no `api_key`/`env_key` of its own
4. The `AXON_API_KEY` environment variable (global fallback)

**Exception — no-auth endpoints:** a model with `no_auth = true`, or any
model whose `base_url` is a loopback address, skips this chain entirely
(unless it sets its own `api_key`/`env_key`): requests are sent with no
`Authorization` header, and no credential is ever forwarded to the local
server.

### Context Window

The `context_window` value tells Axon when to trigger auto-compaction. When you override a known model, Axon inherits that model's context window. When you define a new model and omit `context_window`, Axon defaults to 200,000 tokens, so set it explicitly to match your provider.

### Chat Template Kwargs

Some local reasoning backends only separate the model's chain-of-thought from its answer when the request asks them to. vLLM's `poolside_v1` and `deepseek_v3` reasoning parsers install a real parser **only** when the request carries `enable_thinking` (or the `thinking` alias) in `chat_template_kwargs`; without it they fall back to an identity parser that leaves the whole chain-of-thought -- and a stray `</think>` -- in the assistant's `content`.

That matters more than the stray tag. Reasoning left in `content` is written to the transcript and re-sent as conversation history on every later turn, while reasoning returned in its own field is dropped by the chat template. On one local 120B model, the same conversation rendered at 1,437 tokens with the chain-of-thought inline versus 153 tokens with it separated -- identical to deleting it outright.

```toml
[model.my-local-reasoner]
model = "my-model"
base_url = "http://127.0.0.1:8000/v1"
no_auth = true
context_window = 262144
chat_template_kwargs = { enable_thinking = true }
```

The table is forwarded verbatim as the OpenAI-compatible `chat_template_kwargs` request field, and is omitted entirely when unset, so providers that reject unknown body fields are unaffected until a model opts in. Different backends spell the key differently -- Qwen3 uses `enable_thinking`, poolside and deepseek accept either that or `thinking` -- which is why this is a free-form table rather than a fixed flag.

Prefer `enable_thinking` to the `thinking` alias. vLLM derives that same key from `reasoning_effort` when the caller has not already set it, so with the alias a `reasoning_effort = "none"` configured on a model, role or persona contradicts this setting: the model emits its reasoning and then stops, returning an empty answer.

### Global Default Headers

To apply the same headers to *every* model in the catalog -- built-in, prefetched from `/v1/models`, or custom -- set them once under the global `[models]` section instead of repeating them per model:

```toml
[models]
extra_headers = { "X-Request-Tags" = "team=example,env=prod" }
```

These act as a base for each model's inference requests. A per-model `[model.<id>].extra_headers` entry overrides the global default **per key** (matched case-insensitively): a key set on the model wins, while any global-only keys are still inherited by that model. Like the per-model field, they ride on that model's inference calls -- not on separate services such as image generation or video generation -- which makes them handy for attribution tags (for example, cost tracking) without re-declaring them whenever a new model appears.

### Global Default Values

A few common per-model settings can also be set once under `[models]` as a default for *every* model. A per-model `[model.<id>]` value always wins; the global only fills in where a model (or the server's model list) left the field unset:

```toml
[models]
temperature                 = 0.7
top_p                       = 0.95
max_completion_tokens       = 8192
max_retries                 = 8
inference_idle_timeout_secs = 600
stream_tool_calls           = true
```

This is a small, fixed set of environment-wide knobs. Settings that identify a specific model (`model`, `base_url`, `api_key`, `context_window`, ...) cannot be defaulted this way, and a few settings with their own dedicated configuration -- auto-compaction (`[session]`), the system-prompt label (`[agent]`), and reasoning effort (`[models].default_reasoning_effort`) -- keep their existing homes.

> **Note on `stream_tool_calls`:** this one affects request *shape*, not just sampling. A few endpoints (some BYOK providers) expect it left unset; if a global `stream_tool_calls = true` causes problems for such a model, opt that model out with `stream_tool_calls = false` in its `[model.<id>]` block.

---

## Overriding Model Fields

You can override specific fields of a model already in the catalog (one you
defined, or one prefetched from a `/v1/models` endpoint) without redefining
everything. Use the model's id as the section key and specify only the fields
you want to change:

```toml
# Override only the API key for a model
[model.local]
api_key = "my-api-key"

# Override temperature and add a custom API key
[model.local]
temperature = 0.5
api_key = "sk-custom"
```

When you override an existing model, Axon starts with that model's configuration (including its `base_url`), then applies only the fields you specify. Unspecified fields are inherited.

### Priority Order

1. Your config (`[model.*]`) -- highest priority
2. Prefetched models from remote `/v1/models`
3. Hardcoded defaults -- lowest priority

---

## Provider Examples

### Anthropic (Claude)

Use Claude models directly via the Anthropic Messages API:

```toml
[model.claude-opus]
model = "claude-opus-4-6"
base_url = "https://api.anthropic.com/v1"
name = "Claude Opus 4.6"
api_backend = "messages"
context_window = 200000
extra_headers = { "x-api-key" = "sk-ant-...", "anthropic-version" = "2023-06-01" }
```

The `messages` backend uses the Anthropic Messages protocol. Anthropic authenticates with an `x-api-key` header rather than `Authorization: Bearer`, so pass your key through `extra_headers`, which Axon sends verbatim.

### OpenAI (Chat Completions)

```toml
[model.gpt-4o]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
name = "GPT-4o"
env_key = "OPENAI_API_KEY"
```

`api_backend` defaults to `"chat_completions"`, so you don't need to set it explicitly for OpenAI.

### OpenAI (Responses API)

If your provider supports the newer Responses API:

```toml
[model.gpt-4o-responses]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
name = "GPT-4o (Responses)"
api_backend = "responses"
env_key = "OPENAI_API_KEY"
```

### Ollama (Local Models)

Run models locally with [Ollama](https://ollama.ai):

```toml
[model.ollama-codellama]
model = "codellama"
base_url = "http://localhost:11434/v1"
name = "CodeLlama (Ollama)"
```

Make sure Ollama is running (`ollama serve`) and the model is pulled (`ollama pull codellama`).

Loopback endpoints (`localhost`, `127.0.0.1`, `[::1]`) are automatically
treated as **no-auth**: no API key is required, no login is needed at
startup, and no credential is sent to the local server. This
works out of the box for Ollama, llama.cpp, LM Studio, and vLLM. For a
non-loopback endpoint that also needs no authentication (for example a
model server on your LAN), set `no_auth = true` explicitly:

```toml
[model.lan-llama]
model = "llama-3.1-70b"
base_url = "http://192.168.1.50:8080/v1"
name = "LAN Llama"
no_auth = true
context_window = 128000
```

### Together AI

```toml
[model.together-mixtral]
model = "mistralai/Mixtral-8x7B-Instruct-v0.1"
base_url = "https://api.together.xyz/v1"
name = "Mixtral 8x7B"
env_key = "TOGETHER_API_KEY"
```

### Local OpenAI-Compatible Server

Any server that implements the OpenAI Chat Completions or Responses API
(llama.cpp `llama-server`, LM Studio, vLLM, …):

```toml
[model.local-llama]
model = "llama-3.1-70b"
base_url = "http://localhost:8080/v1"
name = "Local Llama"
temperature = 0.8
```

No `api_key`, `env_key`, or login is needed for loopback endpoints, and
`context_window` defaults to 200,000 when omitted — set it to your model's
real window (e.g. `8192` for many small local models), otherwise
auto-compaction will never trigger and long sessions will overflow the
server's context. Streaming, tool calls, and reasoning content all work
over the default `chat_completions` backend.

To make a local model the default for new sessions:

```toml
[models]
default = "local-llama"
```

### Fully Offline

With only local models configured, the CLI makes no network requests at
startup. To also suppress the optional startup fetch of a remote model
catalog, disable remote fetches in `~/.axon/config.toml` (deliberately
config-only — no env var can re-arm it):

```toml
[features]
remote_fetch = false
```

### Caveat: Local Proxies on Localhost

Because loopback endpoints are treated as no-auth, a local proxy running on
`localhost` no longer receives a session token automatically. If you route
Axon traffic through a local gateway that requires auth, set an explicit
`api_key` on that model entry.

---

## Custom Models Endpoint

Point Axon at a custom OpenAI-compatible `/v1/models` endpoint instead of the default. Use this when your models sit behind a corporate gateway or a self-hosted inference service.

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `AXON_MODELS_BASE_URL` | Yes | Base URL for inference. Axon fetches the model list from `{base_url}/models`. |
| `AXON_API_KEY` | No for loopback URLs | API key sent as `Authorization: Bearer`. Loopback endpoints (e.g. `http://localhost:11434/v1`) are queried without auth when no key is set. |
| `AXON_MODELS_LIST_URL` | No | Override the model-list URL when it differs from `{base_url}/models`. |

### Setup

```bash
export AXON_MODELS_BASE_URL="https://api.acme.com/v1"
export AXON_API_KEY="sk-..."
axon
```

### Config File Alternative

```toml
[endpoints]
models_base_url = "https://api.acme.com/v1"

# Override only the API key for a specific model
[model.local]
api_key = "my-api-key"
```

When you use `[endpoints]` with partial model overrides, Axon inherits the `base_url` from the endpoints config, so you do not need to specify it in each `[model.*]` section.

### Auth Behavior

When you set `models_base_url`, Axon uses API key auth (`Authorization: Bearer`) instead of session auth. No login is required -- the API key is enough.

---

## Web Search Model

The `web_search` tool uses a separate model. Configure it with:

```toml
[models]
web_search = "my-search-model"
```

Or via environment variable:

```bash
export AXON_WEB_SEARCH_MODEL="my-search-model"
```

If you point web search at a custom model, you also need a `[model.*]` entry so Axon can reach it. Server-side ("backend") web search runs only when the model sets `supports_backend_search = true` (and the build enables backend search); it does not depend on `api_backend`:

```toml
[models]
web_search = "my-custom-model"

[model.my-custom-model]
model = "my-custom-model"
supports_backend_search = true
```

---

## Using Custom Models

```bash
# List available models (including custom)
axon models

# Use in the TUI via slash command
/model my-model

# Use in headless mode
axon -p "Hello" -m my-model

# Set as default in config.toml:
[models]
default = "my-model"
```

---

## Enterprise Deployment

A complete config for an enterprise deployment with custom models routed
through your own proxy:

```toml
[cli]
auto_update = false

[auth]
auth_provider_command = "/usr/local/bin/my-company-auth-provider"
auth_provider_label = "Acme Corp"
auth_token_ttl = 3600

[models]
default = "company-llm"

[model.company-llm]
model = "your-model-id"
base_url = "https://llm-proxy.acme.com/"
name = "Company LLM (Proxy)"
context_window = 128000
```

---

## Troubleshooting

### Model Not Found

```bash
# List available models
axon models

# Check config.toml for typos in [model.*] sections
```

### Connection Errors

Verify the endpoint is reachable:

```bash
curl -s https://api.example.com/v1/models \
  -H "Authorization: Bearer $AXON_API_KEY"
```

### Debug Logging

```bash
RUST_LOG=debug AXON_LOG_FILE=/tmp/axon.log axon
tail -f /tmp/axon.log
```

Look for log entries containing `model` or `sampling` to trace model selection and API calls.
