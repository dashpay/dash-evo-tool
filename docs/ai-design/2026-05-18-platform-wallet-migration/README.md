# DET → platform-wallet: Clean-Slate Backend Rewrite

## Executive Summary

DET becomes a thin adapter: `platform-wallet` owns chain sync, HD wallet management, identity lifecycle, DashPay, asset locks, and persistence. DET's `src/spv/` is deleted entirely — `SpvRuntime` inside `platform-wallet` drives all of it. The `CoreBackendMode` RPC/SPV dual-path disappears; there is one backend. The upgrade path is a one-time from_/into_ migration on first launch (backup → re-register wallets from seed → migrate identity/DashPay state → drop legacy tables). Single-key wallets are mocked with a clean swap boundary for a future upstream non-HD type.

> **STATUS**
>
> SPEC ONLY — no implementation until user-approved.
> Supersedes the prior incremental plan (architecture.md, migration-plan.md, spv-rpc-correctness.md, verification.md — all deleted).
> Verified at PR #3625 head `738091f734e05c7a1b822bb1ebff336c93b67891`.

## Load-Bearing Confirmed Assumption

**Decision #1 — `platform-wallet` owns SPV internally: CONFIRMED at source. Not contradicted.**

`packages/rs-platform-wallet/Cargo.toml` declares `dash-spv` as a direct dependency. `SpvRuntime` (`packages/rs-platform-wallet/src/spv/runtime.rs`) constructs `DashSpvClient::new()` internally, owns `PeerNetworkManager` + `DiskStorageManager`, and exposes `run(config, cancel_token)` — its own sync loop. There is no host-feed API. `PlatformWalletManager` owns the `SpvRuntime`. DET's `src/spv/` is deletable; only a thin ConnectionStatus display adapter remains. See [upstream-reality.md](upstream-reality.md) for the full evidence chain.

## Hard Sequencing Gates

**G1 — PR #3625 merge + pin bump.**
PR #3625 (`platform-wallet-storage`) is open, draft, not merged (base `v3.1-dev`, milestone v3.1.0, last updated 2026-05-14). DET's platform pin (`Cargo.toml:21`) is `54048b9352…`, which predates the persister crate. Phase P3+ are blocked until #3625 merges and DET bumps to a containing rev. P0–P1 are not blocked (spike can compile against the PR branch).

**G2 — upstream `Wallet::from_persisted` (`load()` gap).**
`ClientStartState.wallets` is not reconstructed by `persister.load()` (`LOAD_UNIMPLEMENTED = ["ClientStartState::wallets"]` in `rs-platform-wallet-storage/src/sqlite/persister.rs`). Upstream works around this by re-registering wallets from seed at startup (`create_wallet_from_seed_bytes → load_persisted()`). DET must retain encrypted seeds and re-register each wallet from seed on every launch; the persister supplies identity/contact/UTXO/asset-lock deltas around it. Not a blocker — it is the prescribed upstream pattern — but it is a frozen contract. See [upstream-reality.md § G2 Caveat](upstream-reality.md#g2-caveat--walletfrom_persisted-load-gap).

## Table of Contents

| File | Description |
|---|---|
| [upstream-reality.md](upstream-reality.md) | Verified upstream facts: what `platform-wallet` owns, the `src/spv/`-deletion answer, G2 caveat |
| [backend-architecture.md](backend-architecture.md) | New `src/wallet_backend/` module, `AppContext` placement, threading, event flow replacing reconcile, error model |
| [backendtask-contract.md](backendtask-contract.md) | Full kept/modified/removed/new `BackendTask` table; net frontend impact |
| [data-model-and-migration.md](data-model-and-migration.md) | Conversion table, one-time migration procedure with backup/fail-safe, dead fields |
| [removal-inventory.md](removal-inventory.md) | DELETE vs RETAIN lists; RPC backend mode fate; thin Core-RPC mining utility |
| [single-key-mock.md](single-key-mock.md) | `SingleKeyBackend` trait boundary, stub behavior, user message, isolation |
| [phasing.md](phasing.md) | P0–P5 phase table with gates; skills/agents/crew; QA matrix; highest-risk assumption verdict |
| [open-questions.md](open-questions.md) | Eight decisions/questions still needed from the user, with architect recommendations |
| [feature-coverage.md](feature-coverage.md) | Supporting analysis: RPC-vs-SPV capability matrix; DET features absent from `platform-wallet` |

## Open Decisions Still Needed

See [open-questions.md](open-questions.md) for full context and architect recommendations:

- **#1** G1 timing — wait for #3625 merge vs. temporarily pin to PR branch for P0–P2
- **#2** G2 seed-re-registration UX — acceptable today, or wait for upstream persisted rehydration
- **#3** ZMQ listener — audit and likely drop once wallet no longer uses Core RPC
- **#4** Devnet identity discovery — confirm DET-permanent
- **#5** DashPay scope boundary — confirm hybrid split
- **#6** DIP-14/15 parity policy — policy if P0 probe shows divergence
- **#7** Single-key timeline — confirm "mock now, swap later" acceptable for one release
- **#8** One-release no-op grace for removed tasks vs. immediate removal
