# Migration Plan

**Purpose:** Four-phase delivery plan with effort, risk, blast-radius, frozen contracts, sequencing gates, crew assignments, and the QA matrix.

[← back to README](README.md)

---

## B. Phased Migration Plan

### Phase Table

| Phase | Goal | Files | Blast radius | Effort | Risk | Frozen contract out |
|---|---|---|---|---|---|---|
| **0 Spike** | Add `platform-wallet`+`serde` feature behind disabled flag; run mandatory runtime probes (see [verification.md § E.4](verification.md#e4--residual-runtime-probes-phase-0)) | `Cargo.toml`, `tests/` spikes | None (feature off) | M | Low | Verified upstream API + probe results |
| **1 Newtype-wrap `wallet()`** | `DetWalletManager` newtype; rewire 4 consumers; `W` still `ManagedWalletInfo` (pure refactor) | `src/spv/manager.rs`, `src/spv/mod.rs`, `src/context/wallet_lifecycle.rs:759`, `src/backend_task/core/mod.rs:677,900`, `src/mcp/tools/wallet.rs:73` | ~5 files, ~34 call sites — mechanical | M | Medium (signature churn; behavior must be byte-identical) | `DetWalletManager` public method set |
| **2 Adopt `platform-wallet-storage`** | Consume canonical `SqliteWalletPersister`; wire `Arc<dyn PlatformWalletPersistence>`; add `TaskError::PlatformWalletPersistence` | `Cargo.toml`, `src/context/`, `src/backend_task/error.rs` | Config + wiring (no persister code written) | S/M | Medium (RPC rehydration path newly exercised) | Persister instance + 2-DB layout |
| **3 Swap generic → `PlatformWalletInfo`** | Flip `DetWalletManager` inner type; `IdentityManager` live (dual-write vs `QualifiedIdentity`); Devnet fallback branch | `src/spv/manager.rs` (1 generic param), persister wiring, `src/backend_task/identity/*`, `src/backend_task/error.rs` | Localized by Phase-1 newtype | L | High (parity, Devnet, wallet-rehydration gap G2) | `IdentityManager` is runtime identity source |
| **4 Delete hand-rolled code** | Remove `Wallet.identities` cache, hand-rolled credits, duplicated DIP-14/15 derivation; add startup sanity check + migration tool | `src/model/wallet/mod.rs:360`, `src/backend_task/dashpay/{dip14_derivation,hd_derivation,encryption}.rs`, related DB | Wide but compiler-guided | M | Medium (gated on E.1 parity probe green) | Final state; docs updated |

### Sequencing Gates

Two gates are hard and non-negotiable. They are also documented in the [README](README.md).

**Gate G1 — PR #3625 merge + pin bump.**
Phases 2, 3, and 4 are blocked until upstream PR #3625 merges and dash-evo-tool bumps its platform pin to a containing rev. Phases 0 and 1 are unblocked.

The `platform-wallet/serde` feature commit (`e26945cfdf`) is independently cherry-pickable and may land earlier than the full #3625 merge — track this as a possible partial unblock for Phase 0 feature-flag work.

**Gate G2 — upstream `Wallet::from_persisted` (`load()` gap).**
Phase 3 must not rely on `persister.load()` to rebuild wallet handles. dash-evo-tool's seed-driven `SpvManager::load_wallet_from_seed` (`src/spv/manager.rs:935`) remains the wallet-rehydration path. The persister supplies identity/contact/asset-lock/UTXO deltas around it. This frozen contract governs the Phase-2↔Phase-3 interface until G2 closes upstream. See [verification.md § E.3](verification.md#e3--re-confirmation-at-pr-3625-head-drift-check) for the confirmed status.

---

## G. Skills, Agents, and QA Matrix

### Governing Workflow

Standard Requirements → Architecture → Implementation → QA → Review, applied per phase. Each phase is a self-contained increment — do not collapse phases. Phase-2+ start gated on G1. Phase-3 design is frozen against G2.

### Crew Assignments

| Phase | Lead crew | Mandatory reviewers | Skills enforced |
|---|---|---|---|
| 0 Spike | Research/investigation agent + Architect sign-off | — | rust-best-practices (M-PRIOR-ART, M-STATIC-VERIFICATION) |
| 1 Newtype | Rust implementation agent | Architect (frozen-contract review) | rust-best-practices (M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE, C-STRUCT-PRIVATE) |
| 2 Adopt persister | Rust implementation agent | Security reviewer advisory (ASVS V14.2 regression: confirm DET seed-encryption store untouched; secret boundary is upstream-owned) | security-best-practices (A02/V14.2, A08 deserialization — upstream-owned), rust-best-practices (error taxonomy, M-APP-ERROR) |
| 3 Generic swap | Rust implementation agent + Architect (parity/Devnet strategy) | Security reviewer advisory | security-best-practices (A04 secure design, A09 error handling — wrap `PersistenceError`/Devnet typed), rust-best-practices |
| 4 Delete | Rust implementation agent | Correctness reviewer mandatory (E.1 parity gate green) | rust-best-practices (test quality, M-NO-TOMBSTONES), security-best-practices (A08 integrity) |

### QA Matrix

All phases run the standard baseline:
- `cargo clippy --all-features --all-targets -- -D warnings`
- `cargo +nightly fmt --all`
- `cargo test --all-features --workspace`

Plus targeted lanes:

| Lane | Phases | What |
|---|---|---|
| SPV golden-path E2E (testnet, `tests/backend-e2e/`) | 1, 3 | Wallet load, balance, send, identity register/top-up |
| RPC-mode identity + DashPay E2E | 2, 3, 4 | The named RPC-rehydration gate (see [spv-rpc-correctness.md § Phase 2](spv-rpc-correctness.md)) — identity list, credits, contact addresses through the new persister path |
| RPC + Devnet + DashPay lane | 3, 4 | Thinnest-tested corner: Devnet identity-discovery fallback under RPC mode; DashPay contact derivation |
| DIP-14/15 golden-vector | 0 (probe), 4 (regression) | Byte-equality assertions per [verification.md § E.1](verification.md#e1--dip-1415-dashpay-derivation-parity) |
| `DiskStorageManager` data-dir diff | 0 | Compat determination per [verification.md § E.2](verification.md#e2--diskstoragemanager-byte-compat) |

`docs/user-stories.md` is updated at Phases 3 and 4. `claudius:lessons-learned` is invoked at each phase close.
