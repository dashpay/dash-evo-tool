# DET → `platform-wallet` Migration

## Executive Summary

This design covers the incremental migration of dash-evo-tool's wallet infrastructure to the upstream `platform-wallet` crate and its companion `platform-wallet-storage` persister. The core structural insight is that `PlatformWalletInfo` is a composition over the existing `ManagedWalletInfo` — it wraps, not replaces it — which is the structural reason the migration is tractable phase by phase. The equally important supply-side insight is that a canonical SQLite persister (`SqliteWalletPersister`, from the new `platform-wallet-storage` crate in PR #3625) already exists upstream; dash-evo-tool's job is to consume and wire it, not build one.

> **STATUS**
>
> Investigation and planning: COMPLETE
> Implementation: NOT STARTED
> Blocked on: Gate G1 (PR #3625 merge + platform pin bump) before Phases 2–4 can begin.
> Phases 0–1 are executable now.

## Hard Sequencing Gates

Two gates are non-negotiable before certain phases can start:

**G1 — PR #3625 merge + pin bump.**
PR #3625 (upstream `platform-wallet-storage`) is open, draft, not merged (base `v3.1-dev`, milestone v3.1.0, last updated 2026-05-14). dash-evo-tool's platform pin (`Cargo.toml:21`) points to `54048b9352…`, which predates the persister crate. Phases 2, 3, and 4 are blocked until #3625 merges and dash-evo-tool bumps to a containing rev. Phases 0 and 1 are unblocked — they use only `ManagedWalletInfo`, already available.

**G2 — upstream `Wallet::from_persisted` (the `load()` gap).**
Confirmed at PR #3625 head (`738091f734…`): `persister.rs` declares `LOAD_UNIMPLEMENTED = ["ClientStartState::wallets"]`; `load()` populates only `platform_addresses`, not wallet handles. The upstream `Wallet::from_persisted` constructor does not yet exist. Consequence: Phase 3 must not rely on `persister.load()` to rebuild wallets. dash-evo-tool's seed-driven `SpvManager::load_wallet_from_seed` remains the wallet-rehydration path; the persister supplies identity/contact/asset-lock/UTXO deltas around it. This is the frozen Phase-2↔Phase-3 contract until G2 closes upstream.

## Table of Contents

| File | Description |
|---|---|
| [architecture.md](architecture.md) | Target component layout, `DetWalletManager` newtype rationale, persistence design, two-DB coexistence, secret boundary |
| [migration-plan.md](migration-plan.md) | Four-phase plan with effort/risk/blast-radius table, sequencing gates, skills and agents, QA matrix |
| [spv-rpc-correctness.md](spv-rpc-correctness.md) | Per-phase correctness verdicts for SPV and RPC modes; the mandatory RPC-rehydration E2E gate |
| [verification.md](verification.md) | All verification findings: DIP-14/15 parity analysis, `DiskStorageManager` byte-compat, PR #3625 drift check, Phase-0 runtime probe specs |
| [open-questions.md](open-questions.md) | Four decisions still needed from the user, with architect recommendations and decision rationale |

## Open Decisions Still Needed

See [open-questions.md](open-questions.md) for full context and recommendations. The blocking items are:

- **#4** `DiskStorageManager` rebuild UX — silent re-sync vs explicit user prompt
- **#5** DashPay scope boundary — confirm the hybrid split between persister and DET ownership
- **#6** `QualifiedIdentity` longevity — confirm alignment with upstream's deferral through this migration
- **#7** Devnet fallback longevity — confirm two code paths are acceptable indefinitely
- **#3-resid** DIP-14/15 mismatch policy — if E.1 probe shows divergence, approve migration-tool approach as the Phase-4 unblock
