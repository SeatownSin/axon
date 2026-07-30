# Monitoring Usage

**Axon does not phone home.** This fork removes the upstream telemetry and
analytics subsystem entirely: there is no product-analytics pipeline, no
session/trace upload, no Sentry, no feedback API, and no OpenTelemetry (OTLP)
export stream. There is nothing to opt out of, because nothing is sent.

- **No calls to xAI.** Axon makes no network requests to any xAI service, and
  those hosts are refused at the network boundary regardless of configuration.
- **No account, usage, or billing data.** Axon has no hosted account, so there
  is no remote usage dashboard, quota page, or billing surface to display.
- **No external telemetry sink.** The `AXON_TELEMETRY_*` and `AXON_EXTERNAL_OTEL`
  knobs from upstream are not wired to any exporter in this build; there is no
  `[telemetry]` config block that ships data anywhere.

## What you can still see locally

Usage information that Axon surfaces is computed and displayed **in-session
only** — it never leaves your machine:

- **Token counts and cost estimates** for the current session are shown in the
  TUI (cost figures are local estimates based on the model you configured, not
  billed amounts from any provider).
- **Session history** lives under `~/.axon/sessions/` and can be inspected with
  your own tools.
- **Per-subagent usage** can be recorded by a `SubagentStop` hook, which
  receives `usageByModel`: the model each subagent actually called, its input
  and output tokens, its call count, and the time spent in the API. Since a hook
  is your own script, the numbers go wherever you send them — a local file, by
  default nowhere else. See [Hooks](10-hooks.md).

  Two limits worth knowing before you build on it. This is the **only** hook
  payload carrying usage, so main-agent turns are not covered — `Stop` reports a
  reason and nothing more. And the sibling `tokensUsed` field is the subagent's
  final **context size**, not what it generated, so a rate derived from it is
  meaningless; use `outputTokens` over `apiDurationMs` instead.

If you need fleet-wide observability across every turn rather than per subagent,
run everything through your own OpenAI-compatible proxy
(`AXON_CLI_CHAT_PROXY_BASE_URL`) and collect metrics at that proxy — Axon itself
emits no telemetry.

---

> Axon is an independent fork of xAI's Apache-2.0-licensed Grok Build; it is not
> affiliated with xAI. The upstream product's OpenTelemetry usage-export feature
> is not present in this fork.
