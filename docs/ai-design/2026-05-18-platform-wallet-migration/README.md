# DET → platform-wallet: Clean-Slate Backend Rewrite

## Executive Summary

DET becomes a thin adapter: `platform-wallet` owns chain sync, HD wallet management, identity lifecycle, DashPay, asset locks, and persistence. DET's `src/spv/` is deleted entirely — `SpvRuntime` inside `platform-wallet` drives all of it. The `CoreBackendMode` RPC/SPV dual-path disappears; there is one backend. The upgrade path is a one-time from_/into_ migration on first launch (backup → re-register wallets from seed → migrate identity/DashPay state → drop legacy tables). Single-key wallets are mocked with a clean swap boundary for a future upstream non-HD type.

> **STATUS**
>
> P0 — DONE (GREEN). P0.5 — DONE (GREEN). P1 — DONE (GREEN). P2 — DONE (GREEN).
> P3 — ratified two-stage marker-gated migration architecture; in progress (P3a–P3e). Run is mid-execution on this branch.
> P4–P5 — pending P3 completion.
> Only release-blocking gate: Decision #6 DIP-14/15 migration/hard-stop QA lane (P3+P4).
> Supersedes the prior incremental plan (architecture.md, migration-plan.md, spv-rpc-correctness.md, verification.md — all deleted).
> Verified at PR #3625 head `738091f734e05c7a1b822bb1ebff336c93b67891`.

## Load-Bearing Confirmed Assumption

**Decision #1 — `platform-wallet` owns SPV internally: CONFIRMED at source. Not contradicted.**

`packages/rs-platform-wallet/Cargo.toml` declares `dash-spv` as a direct dependency. `SpvRuntime` (`packages/rs-platform-wallet/src/spv/runtime.rs`) constructs `DashSpvClient::new()` internally, owns `PeerNetworkManager` + `DiskStorageManager`, and exposes `run(config, cancel_token)` — its own sync loop. There is no host-feed API. `PlatformWalletManager` owns the `SpvRuntime`. DET's `src/spv/` is deletable; only a thin ConnectionStatus display adapter remains. See [upstream-reality.md](upstream-reality.md) for the full evidence chain.

## Gate Posture (Updated)

With Decision #1 (pin to #3625 head now) and Decision #2 (G2 downgraded via `PersistedWalletLoader` seam), implementation is no longer upstream-blocked.

**G1 — PR #3625 — now a release-hardening track, not a start blocker.**
DET is pinned to PR #3625 head now. P0–P2 proceed immediately. G1 resolves to: track #3625 until it merges, then bump pin to a released rev before shipping P3+. See [phasing.md § Combined Gate Posture](phasing.md#combined-gate-posture).

**G2 — `Wallet::from_persisted` gap — downgraded to deferred swap.**
`ClientStartState.wallets` is not reconstructed by `persister.load()` at PR head (`LOAD_UNIMPLEMENTED = ["ClientStartState::wallets"]` in `rs-platform-wallet-storage/src/sqlite/persister.rs`). Mitigated by the `PersistedWalletLoader` seam: `SeedReregistrationLoader` ships now with seed-re-registration behavior; `UpstreamFromPersisted` is a one-line swap when upstream ships `Wallet::from_persisted`. G2 is no longer a gate. See [g2-mock-boundary.md](g2-mock-boundary.md) and [upstream-reality.md § G2 Caveat](upstream-reality.md#g2-caveat--walletfrom_persisted-load-gap).

**Only release-blocking gate: Decision #6 DIP-14/15 migration/hard-stop QA lane.**
The per-contact migrate-or-quarantine path must be implemented and QA-proven before P3+P4 ship. See [dip14-migration-hardstop.md](dip14-migration-hardstop.md) and [phasing.md](phasing.md).

## Table of Contents

| File | Description |
|---|---|
| [upstream-reality.md](upstream-reality.md) | Verified upstream facts: what `platform-wallet` owns, the `src/spv/`-deletion answer, G2 caveat |
| [backend-architecture.md](backend-architecture.md) | New `src/wallet_backend/` module, `AppContext` placement, threading, event flow replacing reconcile, error model |
| [backendtask-contract.md](backendtask-contract.md) | Full kept/modified/removed/new `BackendTask` table; net frontend impact |
| [data-model-and-migration.md](data-model-and-migration.md) | Conversion table, one-time migration procedure with backup/fail-safe, dead fields |
| [removal-inventory.md](removal-inventory.md) | DELETE vs RETAIN lists; RPC backend mode fate; thin Core-RPC mining utility |
| [single-key-mock.md](single-key-mock.md) | `SingleKeyBackend` trait boundary, stub behavior, user message, isolation |
| [phasing.md](phasing.md) | P0–P5 phase table (including P0.5 compile floor) with gates; skills/agents/crew; QA matrix; highest-risk assumption verdict |
| [g2-mock-boundary.md](g2-mock-boundary.md) | `PersistedWalletLoader` seam design — seed-re-registration now, one-line swap when upstream `Wallet::from_persisted` lands |
| [dip14-migration-hardstop.md](dip14-migration-hardstop.md) | DIP-14/15 per-contact migrate-or-quarantine policy, hard-stop behavior, escalation, revised P4 gate |
| [open-questions.md](open-questions.md) | All 8 decisions — now fully RESOLVED |
| [feature-coverage.md](feature-coverage.md) | Supporting analysis: RPC-vs-SPV capability matrix; DET features absent from `platform-wallet` |

## Decisions — All Resolved

See [open-questions.md](open-questions.md) for full resolutions:

- **#1** G1 timing — RESOLVED: pin to #3625 head now; release-hardening only
- **#2** G2 seed-re-registration — RESOLVED: `PersistedWalletLoader` seam; G2 downgraded
- **#3** ZMQ listener — RESOLVED: audit before P4; delete if wallet-only
- **#4** Devnet identity discovery — RESOLVED: DET-permanent
- **#5** DashPay scope boundary — RESOLVED: hybrid split confirmed
- **#6** DIP-14/15 parity policy — RESOLVED: migrate or hard-stop + escalate (see [dip14-migration-hardstop.md](dip14-migration-hardstop.md))
- **#7** Single-key timeline — RESOLVED: mock now, swap later
- **#8** Removed tasks grace — RESOLVED: hard-remove immediately
