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
- **Per-session usage** can be recorded by a `SessionEnd` hook, which receives
  `usageByModel` for the session itself: the model it actually called, input and
  output tokens, call count, and time spent in the API. This covers an ordinary
  interactive session and a headless `axon --agent <name>` run alike. Since a
  hook is your own script, the numbers go wherever you send them — a local file,
  by default nowhere else. See [Hooks](10-hooks.md).
- **Per-subagent usage** is the same payload on a `SubagentStop` hook, reporting
  what one spawned subagent spent.

  Three things to know before building on these. **A subagent session fires
  both events**, so recording each one separately double-counts that run;
  `SessionEnd` sets `"isSubagent": true` so you can skip it. The sibling
  `tokensUsed` field is a final **context size**, not what was generated, so a
  rate derived from it is meaningless — use `outputTokens` over `apiDurationMs`.
  And `Stop` carries what a single TURN spent, which is the event to use for
  headless runs — `SessionEnd` does not fire there.

  Before 0.3.7, `SubagentStop` was the only payload carrying usage, so a
  top-level run reported nothing at all and only orchestrated subagents could be
  measured.

If you need fleet-wide observability across every turn rather than per subagent,
run everything through your own OpenAI-compatible proxy
(`AXON_CLI_CHAT_PROXY_BASE_URL`) and collect metrics at that proxy — Axon itself
emits no telemetry.

---

> Axon is an independent fork of xAI's Apache-2.0-licensed Grok Build; it is not
> affiliated with xAI. The upstream product's OpenTelemetry usage-export feature
> is not present in this fork.
