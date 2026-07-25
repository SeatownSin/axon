# Contributing

**Axon** is a personal, local-first fork of xAI's Grok Build (see
[Relationship to upstream](README.md#relationship-to-upstream)). It is **not**
the upstream project, and it is maintained on a best-effort basis with no
support guarantees.

## How to reach the project

The public **issue tracker is disabled** on this repository. That means:

- **Pull requests** are the way to propose a change or report a bug you have
  already diagnosed. A small PR with a failing test is worth more than a bug
  report here.
- **Security problems** never go in a PR description — follow
  [`SECURITY.md`](SECURITY.md).
- Changes concerning the **upstream** product itself (not this fork's
  modifications) belong upstream — see
  [Relationship to upstream](README.md#relationship-to-upstream).

Review and merges happen when time allows; there is no response-time
commitment.

## The one hard rule

**A change must not reintroduce network calls to xAI infrastructure.** That is
the reason this fork exists — see
[What's different](README.md#whats-different-from-upstream). The guarantee is
enforced by a shared predicate at the point a socket would open, and by tests
that derive their fixtures from the blocked-host list. If a change needs to
touch that machinery, say so explicitly in the PR.

## Before you open a PR

CI runs `fmt`, `clippy`, and `cargo check --workspace --all-targets` on **Linux
and Windows**, with warnings promoted to errors. Reproduce all of it locally —
a plain `cargo check` will *not* show you what CI sees, because the toolchain
action injects `-D warnings`:

```sh
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
```

`--all-targets` is the load-bearing flag: tests, examples and benches are where
platform-specific breakage hides, and a plain `cargo build` never compiles
them. If your change is platform-specific, gate it (`#[cfg(unix)]`) rather than
leaving dead code on the other platform — that is what CI fails on.

Tests: run them under **WSL2 / Linux**. Much of the suite hard-codes Unix paths,
so a Windows-native run reports ~600 failures that are harness artifacts, not
product bugs. See [Running the tests](README.md#running-the-tests).

One-time setup so `git blame` skips the workspace-wide rustfmt commit:

```sh
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

## Conventions

- **Target specific crates** (`cargo check -p <crate>`) — full-workspace builds
  are slow.
- The root `Cargo.toml` is **generated upstream**; prefer editing per-crate
  `Cargo.toml` files. Lint policy lives in `[workspace.lints]`, and crates opt
  in with `[lints] workspace = true`.
- Explain *why* in commit messages. This tree carries a lot of deliberate,
  non-obvious decisions; the next person needs the reason, not a restatement of
  the diff.
- By contributing, you agree your contribution is licensed under the
  [Apache License, Version 2.0](LICENSE), the same terms as this repository.

## Licensing of this source

This repository is a modified fork of xAI's Apache-2.0-licensed source. By
downloading or using it, you agree that your use is governed by the
[Apache License, Version 2.0](LICENSE).
