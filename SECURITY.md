# Security Policy

**Axon** is a personal, local-first fork of xAI's Grok Build. It carries no
formal security program, bounty, or guaranteed response time — reports are
handled on a best-effort basis and are appreciated.

## Supported versions

Only the **latest release** is supported. Fixes land on `main` and ship in the
next release; there are no backports to earlier tags.

## Where to report

This repository has its public issue tracker **disabled**, so there is no way
to file a security problem publicly — which is the intent.

- **A problem in this fork's own changes** — the no-egress / local-model /
  Windows-build modifications described in the
  [README](README.md#whats-different-from-upstream): report it privately via
  GitHub's "Report a vulnerability" on this repository
  ([Security → Advisories](https://github.com/SeatownSin/axon/security/advisories/new)).
  Private reporting is enabled, so that form is the route — it opens a private
  advisory only the maintainer can see. If you cannot use it, contact the
  maintainer privately through their GitHub profile rather than publishing
  details.

- **A problem inherited from upstream** that also affects xAI's Grok Build
  (unmodified code): report it to xAI through their program at
  <https://hackerone.com/x>. This fork does not represent xAI and cannot triage
  or fix upstream issues on their behalf.

When in doubt, treat it as a fork issue and report privately here.

## What is most worth reporting

This fork exists to guarantee one property: **it never contacts the upstream
vendor's infrastructure.** Reports against that guarantee are the most valuable
thing you can send.

- **Any path that reaches vendor infrastructure** — a request that escapes the
  network-boundary predicate, a code path that constructs a client without it,
  or a config/env/remote-setting combination that re-enables one.
- **Credential leakage** — a session token or API key reaching an endpoint it
  should not (notably: a local/BYOK model server), or written to logs, session
  transcripts, or crash output.
- **Escapes from an explicit control** — the permission gate, the sandbox
  profiles, path containment for file tools, or the marketplace's source
  pinning being bypassed without the user's approval.

## What is not a vulnerability

- **The agent running commands or editing files you approved.** Executing shell
  commands and writing to your working tree is the product, not a flaw. Reports
  need to show a control being *bypassed*, not used.
- **Prompt injection from content you deliberately fed the agent.** Inherent to
  agents that read untrusted text. It becomes a report when injected content
  defeats an explicit control — e.g. causes a tool call that the permission gate
  should have stopped.
- **Upstream behaviour this fork does not modify** — see the second bullet
  under [Where to report](#where-to-report).
