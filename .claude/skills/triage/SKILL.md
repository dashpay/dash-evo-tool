---
name: triage
description: "Use when the user says \"triage\" or asks to triage a dash-evo-tool GitHub issue: reproduce, root-cause, attribute to a crate, estimate severity with the user, then post a short status comment."
argument-hint: <issue-number-or-url>
---

# Triage

Org = `dashpay`. Stable = `master` (release branch; `v1.0-dev` is active dev — see CLAUDE.md § Branching).

1. Fetch the issue ($ARGUMENTS); check it against `master`.
2. Reproduce and root-cause per `claudius:bug-investigation`. Prefer a unit/integration test (CLAUDE.md § Testing); fall back to `desktop-gui` (+ `docs/gui-testing/README.md`) only if a test can't reach it.
3. Attribute the root cause to its owning crate: this repo, or an upstream `dashpay`-org git dependency (check `Cargo.toml` pins — currently `platform` for `dash-sdk`/`platform-wallet`/`platform-wallet-storage`/`rs-sdk-trusted-context-provider`, `grovestark` for `grovestark`).
4. Estimate severity with `claudius:severity`; report it to the user.
5. Discuss repro, root cause, crate, and severity with the user — before any GitHub write.
6. Root cause in another `dashpay` repo → propose filing an issue there, link it from this one. Root cause in `dash-evo-tool` → normal fix flow.
7. Once the user agrees: comment on the issue — status update, ≤200 characters, plus links to anything filed/related. Every GitHub write needs the user's go-ahead first (`git-and-github` Safety Rules).
