# Phasing

**Purpose:** P0–P5 phase table with goals, gates, effort, and risk; skills/agents/crew assignments; QA matrix; highest-risk assumption verdict.

[← back to README](README.md)

---

Gates G1 and G2 are defined in [README.md § Gate Posture](README.md#gate-posture-updated) and detailed in [upstream-reality.md § G2 Caveat](upstream-reality.md#g2-caveat--walletfrom_persisted-load-gap).

## Combined Gate Posture

With Decision #1 (pin to #3625 head now) and Decision #2 (G2 downgraded via `PersistedWalletLoader` seam), **implementation is no longer upstream-blocked**.

**G1 — release-hardening track, not a start blocker.**
DET is pinned to PR #3625 head. P0–P2 start immediately. G1 resolves to: track #3625 until it merges, bump pin to a released rev before shipping P3+. Not a gate on any phase start.

**G2 — deferred swap, not a gate.**
`SeedReregistrationLoader` ships in P1 with correct behavior. `UpstreamFromPersisted` is reserved for when upstream `Wallet::from_persisted` lands — one-line construction swap, zero blast radius. See [g2-mock-boundary.md](g2-mock-boundary.md).

**Only release-blocking gate: P3c–P3e simplified Stage-B engine + QA lane.**
The simplified Stage-B engine (upstream-only derivation, no quarantine) must be implemented (P3c) and QA-proven (P3d–P3e) before the release ships. See [data-model-and-migration.md](data-model-and-migration.md) — "Accepted fund-accessibility trade-off" and [open-questions.md #6](open-questions.md#decision-6--dip-1415-parity-policy).

## G. Phasing (From-Scratch Rewrite)

Each phase is independently reviewable. Do not collapse phases.

### Phase Table

| Phase | Goal | Files | Effort | Risk | Frozen contract |
|---|---|---|---|---|---|
| **P0 Spike & verify** (DONE — PROCEED) | Stand up `PlatformWalletManager` + upstream `SqliteWalletPersister` in a harness; prove `SpvRuntime` drives sync end-to-end; run DIP-14/15 golden-vector parity probe + `DiskStorageManager` behavior; confirm event surface. Pin to #3625 head (Decision #1). G2 confirmed open (load() returns empty `ClientStartState.wallets`) — `PersistedWalletLoader`/`SeedReregistrationLoader` premise validated. **Harness-shape constraint: pre-P0.5 spike harnesses MUST be standalone crates, not `tests/*.rs`** — those link the SDK-drift-broken lib. | Standalone crate harness only; `Cargo.toml` (feature-gated dep) | M | Med | Verified upstream API + probe results; DIP-14/15 divergence recorded as release-blocking finding |
| **P0.5 Compile Floor** | Atomically bump `dash-sdk` + `rs-sdk-trusted-context-provider` `54048b…`→`738091f734…`; add `platform-wallet` (feature `serde`) + `platform-wallet-storage` git deps at `738091f…`; then DELETE/STUB/FIXUP exactly enough of the old wallet stack to reach green `cargo build` + `cargo clippy --all-features --all-targets -- -D warnings`. Tests need NOT pass — failing tests are left failing or marked `#[ignore]` + `// TODO(P0.5): re-enable in P{1,2,3}`. No production wallet behavior is expected; wallet ops are inert or removed. **Co-land constraint:** the pin bump is NOT separable from the deletions — no compiling intermediate exists. P0.5 IS the atomic floor commit (or a tight commit series on the branch). P1+ build green on top of it. See [P0.5 Compile-Floor Task List](#p05-compile-floor-task-list) below. | `Cargo.toml:21+`, `src/spv/**`, `src/context/wallet_lifecycle.rs:619-985`, `src/backend_task/core/mod.rs` (heavy), `src/model/wallet/mod.rs` (heavy), `src/model/qualified_identity/mod.rs` (fixup only), `src/backend_task/shielded/bundle.rs` (fixup only), identity/contract tasks (fixup only) | M–L | Medium (over-deletion / under-stubbing) | Workspace compiles; clippy-clean; P1+ build on this floor |
| **P1 WalletBackend skeleton + EventBridge** | New `src/wallet_backend/` wrapping `PlatformWalletManager`; `EventBridge`: `PlatformEventHandler` → `TaskResult` MPSC; `PersistedWalletLoader` trait + `SeedReregistrationLoader` impl (G2 seam — see [g2-mock-boundary.md](g2-mock-boundary.md)); no DET wiring yet (parallel to old path, behind a feature). Builds on the P0.5 green floor. | New `src/wallet_backend/*`, `src/backend_task/error.rs` (typed variants) | L | Med | `WalletBackend` public method set; `EventBridge`→`TaskResult` mapping; `PersistedWalletLoader` seam |
| **P2 BackendTask rewire** | Point wallet/identity/DashPay task arms at `WalletBackend`; replace P0.5 stubs with real `WalletBackend` calls; extract Core-RPC mining utility. | `src/backend_task/{mod,core,wallet,identity,dashpay}/*`, `src/context/*`, new `src/core_rpc_util.rs` | L | High | `BackendTask` result variants stable (frontend contract) |
| **P3 One-time migration** | Two-stage marker-gated migration (see P3a–P3e below). Ratified architecture: Stage A SQL v35 (sync, pre-unlock, sets marker) + Stage B async post-unlock engine at `ensure_wallet_backend`. Re-scoped 2026-05-18: drop backwards compatibility, upstream-only DashPay derivation, no quarantine. Gated on G1. | New `src/database/migration_pw.rs`, `database/initialization.rs` | L | High | Migration forward-only, fail-safe, idempotent; two-stage design ratified |
| **P4a Tx/UTXO/Balance UI data-path rewire** | Add `DetWalletBalance`/`DetUtxo` view models; retain `WalletTransaction` (detach from `Wallet`/DB); add 4 `WalletBackend` accessors + `TransactionRecord`→`WalletTransaction` mapping (relocated from deleted reconcile); add `WalletSnapshot` (`ArcSwap`) + `EventBridge` recompute/swap/emit-`Refresh`; rewire HD reads in wallets UI to read snapshot via `app_context.wallet_backend()`. Single-key paths untouched. **Blocks P4b.** | `src/wallet_backend/mod.rs`, `src/ui/wallets/wallets_screen/mod.rs`, `src/ui/wallets/address_table.rs`, `src/ui/screens/create_asset_lock_screen.rs` | L | Med | Post-migration HD wallets screen shows correct balance/tx/utxo from upstream. Reviewer gate: no spendable-input selection from snapshot. |
| **P4a.5 Fund-safety spend-path completion** | Three spend-path correctness tasks gated between P4a and P4b. **(1) Add `AssetLockKind::Shielded`** and rewire `src/backend_task/shielded/bundle.rs:463,478` (Path 1 — real coin-selection) from `generic_asset_lock_transaction` / `select_unspent_utxos_for`-over-`Wallet.utxos` → `WalletBackend::create_asset_lock_proof` (upstream-authoritative selection at construction time). **(2) Remove `RegisterIdentityFundingMethod::FundWithUtxo` + `TopUpIdentityFundingMethod::FundWithUtxo` variants**, QR-direct-fund UI, and the three functions `registration_asset_lock_transaction_for_utxo` / `top_up_asset_lock_transaction_for_utxo` / `asset_lock_transaction_for_utxo_from_private_key` (Path 2 — no upstream funding-outpoint API exists at #3625 head; cannot be preserved; removed with disclosure via post-migration notice). **(3) Slim `context/transaction_processing.rs::received_transaction_finality`** to asset-lock-finality-only: delete the `Wallet.utxos` / `address_balances` / legacy-`utxos`-table writes; RETAIN the asset-lock detection/registration branch (`store_asset_lock_transaction` + finality-wait channel that `broadcast_and_commit_asset_lock` / `wait_for_asset_lock_proof` depend on). ZMQ call sites `app.rs:1267,1285` stay. Tests per §4.4. **Exit:** zero live readers of `Wallet.{utxos,address_balances,transactions}`; zero callers of `select_unspent_utxos_for` / `generic_asset_lock_transaction` / `*_for_utxo*`. **Blocks P4b.** | `src/backend_task/shielded/bundle.rs`, `src/backend_task/identity/mod.rs`, `src/context/transaction_processing.rs`, funding-method enums + UI, `AssetLockKind` enum | M | Med | Exit: zero live readers of legacy wallet-UTXO fields; zero callers of deleted functions; asset-lock finality channel intact |
| **P4b Mechanical dead-code/UI prune** | Remove RPC-mode toggle, Core-wallet picker, local-node settings UI; delete `Wallet.{transactions,utxos,address_balances,confirmed_balance,unconfirmed_balance,total_balance,spv_balance_known,address_total_received}` + dead methods; `src/database/utxo.rs` + legacy balance/utxo/tx queries; batched schema cleanup (`dashpay_dip14_quarantine_active` + RPC-era dead columns); ZMQ-listener audit + drop-if-unused. **Only after P4a.5 exit criteria are met.** Additional deletions gated on P4a.5 exit: `select_utxos_with_fee_retry`, `generic_asset_lock_transaction`, `registration_asset_lock_transaction_for_utxo`, `top_up_asset_lock_transaction_for_utxo`, `asset_lock_transaction_for_utxo_from_private_key`, `remove_selected_utxos`, `build_multi_recipient_payment_transaction`. | `src/ui/**`, `src/database/**`, `src/model/wallet/mod.rs`, `src/context/**`, remaining dead code from P0.5 stubs | L | Med | Final state; docs + user-stories updated with dropped back-compat + accepted trade-off |
| **P5 Hardening** | Single-key swap-readiness, `ConnectionStatus` adapter polish, full QA matrix including migration lane + post-migration UI data-path test; §2(d) notice regression; **Smythe double-spend/fund-safety audit (RELEASE-BLOCKING — see below)**; single push to #860. User-stories/docs note dropped back-compat + accepted trade-off. | Cross-cutting | M | Low | Release-ready; Smythe audit green |

**Sequencing:** P0 done (PROCEED). P0.5 done (GREEN). P1 done (GREEN). P2 done (GREEN). P3 is the highest-risk phase (irreversible data migration) — two-stage marker-gated design ratified (see P3a–P3e); re-scoped 2026-05-18 (drop back-compat, upstream-only, no quarantine). P3a (GREEN, commit `6d348566`). P3b (GREEN, commit `d5a3e51b`; `classify_contact` + 7 tests now DEAD — deleted in P3c). P3c–P3e complete (GREEN). P4 split into P4a (UI data-path rewire, blocks P4a.5) and P4a.5 (fund-safety spend-path completion, blocks P4b) and P4b (mechanical prune, only after P4a.5); P4-partial done. P5 pending. Run continuing. Fund-safety spend path is upstream-authoritative from P2; the display-only gap is addressed in P4a; the remaining spend-path correctness tasks (Path 1 shielded, Path 2 FundWithUtxo removal, Path 3 finality slim) are addressed in P4a.5 before P4b's prune. P3 ships only after G1 resolves to a released rev. The only release-blocking gate is the simplified Stage-B engine + QA lane (P3c–P3e), plus the P5 Smythe fund-safety audit.

---

## P3 Sub-Steps (Ratified Two-Stage Design, re-scoped 2026-05-18)

P3 is decomposed into five sequenced sub-steps. Do not collapse them.

**P3a — Stage-A SQL v35 + markers + premigration backup. DONE (commit `6d348566`).**
SQL migration arm `35` in `apply_version_changes`: set `settings.platform_wallet_migration_pending=1` inside the v35 transaction. Post-commit, create `data.db.premigration` using the SQLite online-backup API (NOT inside the live write-tx). Added `settings.dashpay_dip14_quarantine_active` column (now INERT/RESERVED — see data-model-and-migration.md; removal deferred to P4). Tests: v35 idempotency, marker set, premigration file created post-commit and internally consistent, no destructive step executes.

**P3b — Stage-B DIP-14/15 predicate — TDD. DONE (commit `d5a3e51b`).**
`classify_contact` and its 7 tests are now DEAD — the predicate is not used by the simplified Stage-B engine. DELETED in P3c (same commit as the engine).

**P3c — Simplified Stage-B engine wired at `ensure_wallet_backend`.**
Implement `src/database/migration_pw.rs` with the single-branch model: (1) backup precondition; (2) re-register wallets via `SeedReregistrationLoader`/`create_wallet_from_seed_bytes` (idempotent); (3) `add_identity` each `QualifiedIdentity` blob (idempotent); (4) re-establish DashPay contacts on upstream `derive_contact_xpub`/`derive_contact_payment_address(es)` ONLY — no DET re-derivation, no comparison, no classify, upsert-keyed `(owner,contact)`; (5) finalize SUCCESS fork only (exception fork handled by marker-not-cleared). Delete `classify_contact` and its 7 tests in the same commit. Wire at `AppContext::ensure_wallet_backend` (`src/context/mod.rs:634`) behind the retained `AppContext` `tokio::sync::Mutex`, gated by `platform_wallet_migration_pending` marker. Legacy DROP (step 5) NOT yet enabled in this sub-step.

**P3d — Restore/idempotency invariants + finalize (no quarantine).**
Dedicated tests before enabling finalize: backup-before-destroy invariant; DROP strictly last + post-durable-flush; crash at each sub-step + relaunch is idempotent; reentrant `ensure_wallet_backend` runs exactly one Stage-B; restore ONLY on exception (never otherwise); user-never-unlocks (marker persists, backup exists, app usable). Enable single-fork finalize (step 5) only after these tests pass.

**P3e — QA lane (standalone-crate harness).**
Fixtures MUST cover: Stage-B crash at each sub-step + relaunch idempotency; restore-from-premigration on injected new-persister corruption; user-never-unlocks (marker persists, app usable, backup exists); reentrant `ensure_wallet_backend` (single Stage-B run); send-side-only contact. No quarantine fixtures — quarantine apparatus is WITHDRAWN. PLUS a mandatory release-blocking test: legacy-address-abandonment notice shows exactly once, one-shot `settings.platform_wallet_migration_notice_shown` gates it, dismissible, jargon-free, shown to all migrated users. Release-blocking; runs alongside P3d and P5 regression.

---

## P4 Sub-Steps

P4 is split into two sequenced sub-steps. P4b must not start before P4a exit criteria are met.

### P4a — Tx/UTXO/Balance UI Data-Path Rewire (blocks P4b)

**Goal:** eliminate all wallets-screen reads from the legacy `Wallet` model and DB tables that P3c's migration drops. The fund-safety spend path is already upstream-authoritative (P2) — this sub-step is display-only.

**Deliverables:**

1. **New view models:** `DetWalletBalance { confirmed: u64, unconfirmed: u64, total: u64 }` and `DetUtxo { outpoint, value, script_pubkey, address }`. `WalletTransaction` is retained as-is; detached from `Wallet`/DB.

2. **Four `WalletBackend` accessors:** `wallet_balance`, `transaction_history`, `utxos`, `address_balances` — DET types only across the boundary (see [backend-architecture.md § WalletBackend Read-Accessor Surface + WalletSnapshot Push Model](backend-architecture.md#walletbackend-read-accessor-surface--walletsnapshot-push-model)).

3. **`TransactionRecord`→`WalletTransaction` mapping** relocated from the deleted `reconcile_spv_wallets` into `WalletBackend`.

4. **`WalletSnapshot` + `EventBridge` push:** `WalletSnapshot` (`ArcSwap`) per wallet; `EventBridge` recomputes and atomically swaps on upstream callbacks; emits existing `TaskResult::Refresh`.

5. **UI rewire:** HD reads in `src/ui/wallets/wallets_screen/mod.rs` (`:451,469,517,527,1141,1156,1227,1245,1478-1589,1845,1861,2593`), `src/ui/wallets/address_table.rs:120`, `src/ui/screens/create_asset_lock_screen.rs` — all read from `app_context.wallet_backend()` snapshot. `WalletTransaction`-row helpers unchanged. Single-key paths untouched (Decision #7 stubbed).

**Exit criteria:** post-migration HD wallets screen shows correct balance, transaction history, and UTXO set from the upstream snapshot — not blank, not stale.

**Reviewer gate (A04):** no code path selects spendable inputs from `WalletSnapshot`. Any such path must be rejected. The spend path remains `WalletBackend::send_payment` / `create_asset_lock_proof` (upstream live UTXO set, P2).

### P4a.5 — Fund-Safety Spend-Path Completion (blocks P4b)

**Goal:** close the three open spend-path correctness gaps before P4b's mechanical prune. P4a is the prerequisite; P4b must not start before P4a.5 exit criteria are met.

**Path 1 — Shielded asset-lock coin-selection (real coin-selection).**
Add `AssetLockKind::Shielded` to the `AssetLockKind` enum. Rewire `src/backend_task/shielded/bundle.rs:463,478` from the legacy path (`generic_asset_lock_transaction` + `select_unspent_utxos_for` over the snapshot `Wallet.utxos`) to `WalletBackend::create_asset_lock_proof`, which delegates coin-selection to the upstream wallet at construction time (authoritative live UTXO set, no snapshot-based selection). This closes the last path where DET could perform coin-selection from a snapshot.

**Path 2 — FundWithUtxo removal (no upstream funding-outpoint API).**
No funding-outpoint API exists in `platform-wallet` at PR #3625 head. The `FundWithUtxo` path cannot be preserved or emulated. Remove:
- `RegisterIdentityFundingMethod::FundWithUtxo` variant
- `TopUpIdentityFundingMethod::FundWithUtxo` variant
- All QR-direct-fund UI that surfaces these variants
- `registration_asset_lock_transaction_for_utxo`
- `top_up_asset_lock_transaction_for_utxo`
- `asset_lock_transaction_for_utxo_from_private_key`

This is a user-facing capability removal. It is disclosed via the one-time post-migration notice (see [data-model-and-migration.md § Mandatory one-time informational notice](data-model-and-migration.md#accepted-fund-accessibility-trade-off-user-decision-2026-05-18)).

**Path 3 — `received_transaction_finality` slim (asset-lock-finality-only).**
Slim `context/transaction_processing.rs::received_transaction_finality` to handle only asset-lock finality. Delete the `Wallet.utxos` / `address_balances` / legacy-`utxos`-table write branches. RETAIN the asset-lock detection and registration branch: `store_asset_lock_transaction` + the finality-wait channel that `broadcast_and_commit_asset_lock` and `wait_for_asset_lock_proof` depend on. ZMQ call sites at `app.rs:1267,1285` stay — ZMQ is still required for asset-lock detection.

**Tests (§4.4):**
- Post-migration asset-lock via upstream (`WalletBackend::create_asset_lock_proof`) end-to-end.
- Path 3: asset-lock finality detection without any `Wallet` mutation (no `Wallet.utxos` / `address_balances` write).
- Crash-retry no-double-broadcast: store-before-broadcast + upstream dedup.

**Exit criteria:** zero live readers of `Wallet.{utxos,address_balances,transactions}`; zero callers of `select_unspent_utxos_for` / `generic_asset_lock_transaction` / `registration_asset_lock_transaction_for_utxo` / `top_up_asset_lock_transaction_for_utxo` / `asset_lock_transaction_for_utxo_from_private_key`; asset-lock finality channel verified intact.

---

### P4b — Mechanical Dead-Code/UI Prune (only after P4a.5)

**Goal:** delete all code whose last readers were relocated by P4a. These are deletable ONLY after P4a exits — P4a is the prerequisite.

**Deliverables:**

- Remove RPC-mode toggle, Core-wallet picker, and "Local Dash Core node" settings UI.
- Delete now-unreachable `Wallet` fields: `transactions`, `utxos`, `address_balances`, `confirmed_balance`, `unconfirmed_balance`, `total_balance`, `spv_balance_known`, `address_total_received`.
- Delete dead `Wallet` methods: `total_balance_duffs`, `confirmed_balance_duffs`, `has_balance`, `max_balance`, `update_spv_balances`, `set_transactions`, `update_address_balance`, `select_unspent_utxos_for`, `select_utxos_with_fee_retry`, `remove_selected_utxos`, `build_multi_recipient_payment_transaction`.
- Delete functions confirmed dead by P4a.5 exit: `generic_asset_lock_transaction`, `registration_asset_lock_transaction_for_utxo`, `top_up_asset_lock_transaction_for_utxo`, `asset_lock_transaction_for_utxo_from_private_key`.
- Delete `src/database/utxo.rs` and legacy balance/utxo/tx queries in `src/database/wallet.rs` (`:233,254,302,734`).
- Batched schema cleanup: `dashpay_dip14_quarantine_active` (INERT/RESERVED — see [backend-architecture.md](backend-architecture.md)), remaining RPC-era dead settings columns.
- ZMQ-listener usage audit: drop if no non-wallet consumer remains.
- Remove `TaskError::DashPayContactDerivationIrreconcilable` if no other caller.
- M-NO-TOMBSTONES: delete, do not comment out.

**Crew assignment:** Correctness reviewer mandatory. DIP-14/15 parity gate must be green before this sub-step ships.

---

## P5 — Smythe Double-Spend / Fund-Safety Audit (RELEASE-BLOCKING gate)

The Smythe security audit is a **release-blocking gate** at P5. No push to #860 until all six invariants below are confirmed green. A single failing invariant = no push.

**Scope — Invariants I1–I6:**

| ID | Invariant | Pass condition |
|---|---|---|
| **I1** | Authoritative selection at construction | No code path selects spendable inputs from `WalletSnapshot` or any `Wallet.utxos` snapshot. All coin-selection goes through `WalletBackend::create_asset_lock_proof` or `WalletBackend::send_payment` (upstream live UTXO set). |
| **I2** | No DET-side parallel spend engine | The functions `select_unspent_utxos_for`, `select_utxos_with_fee_retry`, `generic_asset_lock_transaction`, `registration_asset_lock_transaction_for_utxo`, `top_up_asset_lock_transaction_for_utxo`, `asset_lock_transaction_for_utxo_from_private_key`, `remove_selected_utxos`, `build_multi_recipient_payment_transaction` are deleted, not orphaned. No dead caller, no commented-out call, no unreachable arm. |
| **I3** | `FundWithUtxo` removal disclosed | The one-time post-migration notice text ships in the release build. `RegisterIdentityFundingMethod::FundWithUtxo` and `TopUpIdentityFundingMethod::FundWithUtxo` variants are gone. No dead erroring arm remains in any match on either enum. |
| **I4** | Crash-retry no-double-broadcast | Asset-lock transactions are stored (durable) before broadcast. Upstream deduplication prevents double-broadcast on retry. Store-before-broadcast ordering is verified by test. |
| **I5** | Path 3 deletion leaves asset-lock detection intact | `received_transaction_finality` no longer writes to `Wallet.utxos` / `address_balances` / legacy `utxos` table. The asset-lock detection branch (`store_asset_lock_transaction` + finality-wait channel) is fully functional. `broadcast_and_commit_asset_lock` and `wait_for_asset_lock_proof` succeed in test without any `Wallet` mutation. |
| **I6** | No frame-thread blocking | No code path added in P4a, P4a.5, or P4b causes the egui frame thread to await or block on a wallet operation. All upstream calls are dispatched through `BackendTask` / `WalletBackend` async methods. |

**P5 audit test lanes (in addition to the standard QA matrix):**

| Lane | What |
|---|---|
| Post-migration asset-lock via upstream | `WalletBackend::create_asset_lock_proof` succeeds end-to-end; no `select_unspent_utxos_for` / `generic_asset_lock_transaction` call in the hot path. |
| Path 3 asset-lock finality without Wallet mutation | `received_transaction_finality` correctly detects an asset lock and fires the finality-wait channel; no write to `Wallet.utxos` / `address_balances` / `utxos` table occurs. |
| Crash-retry no-double-broadcast | Simulated crash between store and broadcast; relaunch retries broadcast; upstream dedup prevents double-spend; test asserts single on-chain tx. |

---

## P0.5 Compile-Floor Task List

This section is the authoritative checklist for P0.5. Work through the seven clusters in order. The pin bump (Step 0) must land in the same atomic commit (or series) as the deletions — no intermediate that compiles with the old deps and the old code.

### Step 0 — Dependency Bump

`Cargo.toml:21+` (P0 confirmed zero version conflicts, no `[patch]`, all DET SDK features still present):

- Bump `dash-sdk` + `rs-sdk-trusted-context-provider` from `54048b…` to `738091f734…`.
- Add `platform-wallet` (feature `serde`) + `platform-wallet-storage` git deps at `738091f…`.

### Cluster A — `src/spv/` (8 errors in manager.rs)

**Classification: DELETE**

DELETE the entire `src/spv/` module tree: `manager.rs`, `error.rs`, `mod.rs`, `tests.rs`. Remove `mod spv;` declaration and all `crate::spv::*` imports throughout the workspace.

Rationale: chain sync is owned by `platform-wallet`'s `SpvRuntime`. No DET sync code is needed.

### Cluster B — `src/context/wallet_lifecycle.rs:619-985` (3 errors)

**Classification: DELETE**

Delete the following functions from `wallet_lifecycle.rs:619-985`:
- `reconcile_spv_wallets`
- `sync_spv_account_addresses`
- `spv_setup_finality_listener`
- `spv_setup_reconcile_listener`
- `handle_spv_finality_event`

Also delete the `spv_manager()` accessor and its field wiring in `context/mod.rs:97,295-360`.

### Cluster C — `src/backend_task/core/mod.rs` (8 errors)

**Classification: DELETE / STUB (mixed)**

**Delete:**
- `send_wallet_payment_via_spv`
- `build_spv_unsigned_transaction_multi` (`core/mod.rs:677`)
- `sign_spv_transaction` (`core/mod.rs:900`)
- `send_wallet_payment_via_rpc`
- `CoreBackendMode` enum and all `core_backend_mode()` branch sites
- `core_client_for_wallet` for wallet ops (`context/mod.rs:686`)
- RPC arms of `refresh_wallet_info` and `recover_asset_locks`

**Stub** (return `TaskError::WalletBackendNotYetWired`):

All retained dispatch arms whose implementation is being replaced in P2 return the new typed variant from P0.5 onward:

```rust
#[error("This action is being upgraded and is temporarily unavailable. \
    Please use the previous version of the app to transact, \
    or wait for the next update.")]
WalletBackendNotYetWired,
```

Specifically: `run_wallet_task` / `send_wallet_payment` / `RefreshWalletInfo` / `GenerateReceiveAddress` / `CreateAssetLock` / `FundPlatformAddress*`.

**Keep as-is:**
- `CoreTask::MineBlocks` on thin Core-RPC client (Regtest/Devnet only).

**Single-key arms:** return `TaskError::SingleKeyWalletsUnsupported` from P0.5 onward (see [single-key-mock.md](single-key-mock.md)).

### Cluster D — `src/model/wallet/mod.rs` (9 errors)

**Classification: DELETE / RETAIN-MINIMAL (mixed)**

**Delete:**
- Dead balance/UTXO/tx surface
- `utxos.rs`
- `update_spv_balances`
- `reconcile` maps

**Retain minimal** (required by P3 migration + `database/wallet.rs` read path):
- Seed handle
- `WalletSeed` / `ClosedKeyItem`
- Alias, `is_main`, `seed_hash`, network

**Highest over-deletion risk.** If the retained skeleton is wider or narrower than needed, P3 migration will fail. Flag any uncertainty as a blocking finding before proceeding.

### Cluster E — `src/model/qualified_identity/mod.rs` (4 errors)

**Classification: SDK-DRIFT-FIXUP — RETAIN, do NOT stub or delete**

Fix the following mechanically:
- `sign` / `sign_create_witness` lifetime annotations
- `AddressProvider` member updates
- Missing associated types `Tag` / `Address`

These are upstream API drift fixes, not migration deletions. Mis-classifying this cluster as a stub candidate would cause silent capability loss (A04 violation). If any fix proves too expensive within budget, escalate as a BLOCKING finding — do not replace with `unimplemented!()`.

### Cluster F — `src/backend_task/shielded/bundle.rs` (6 errors)

**Classification: SDK-DRIFT-FIXUP — RETAIN, do NOT stub or delete**

Fix the following:
- `AddressKey` → `AddressOps` rename
- `async ?`-on-`Pin<Box<dyn Future>>` usage
- Trait signature updates

If any fix is unfixable within budget: escalate as a BLOCKING finding. Do NOT stub shielded ops — silent capability loss is forbidden (A04). Never use `unimplemented!()` for retained code.

### Cluster G — Identity / Contract Tasks (4 errors)

**Classification: SDK-DRIFT-FIXUP — RETAIN, do NOT stub or delete**

Mechanical signature and import updates only. Never stub identity or contract tasks.

---

### Disambiguation Summary

Three distinct operations that MUST NOT be confused:

a. **DELETE** (Clusters A, B, C-delete, D-delete): code that belongs to the old SPV/RPC wallet stack, fully replaced by `platform-wallet`. Recoverable via git history on the branch — no tombstones, no commented-out code (M-NO-TOMBSTONES).

b. **STUB** (Cluster C-stub): retained dispatch arms that will be rewired in P2, rendered temporarily inert with a typed error. Stubs exist from P0.5 onward; they disappear in P2 when real `WalletBackend` calls replace them.

c. **SDK-DRIFT-FIXUP** (Clusters E, F, G): code that is out of the migration scope entirely, broken only because upstream API signatures changed. Fix these; never delete or stub them. **This is the highest-risk mis-classification** — if E/F/G code ends up behind `unimplemented!()`, capability is silently lost.

**Data recoverability:** P0.5 and P4 deletions are recoverable via git history on the branch. No commented-out code is left in place (M-NO-TOMBSTONES). P0.5 and P4 touch code only — no DB schema change before P3, no user data is at risk. P3 adds `*.db.premigration` (retained until successful migration). Consistent with A04 fail-safe ordering.

---

## I. Skills, Agents, and QA Matrix

### Governing Workflow

Standard Requirements → Architecture (this spec) → Implementation → QA → Review, per phase. All 8 decisions resolved (see [open-questions.md](open-questions.md)). Implementation is unblocked.

### Crew Assignments

| Phase | Lead crew | Mandatory reviewers | Skills enforced |
|---|---|---|---|
| P0 | Research/spike agent + Architect | — | rust-best-practices (M-PRIOR-ART, M-STATIC-VERIFICATION) |
| P0.5 | Rust impl agent | Architect (delete/stub/fixup classification review) | rust-best-practices (M-NO-TOMBSTONES); security (A04 over-deletion check) |
| P1 | Rust impl agent | Architect (boundary/frozen-contract review) | rust-best-practices (M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE, M-SERVICES-CLONE) |
| P2 | Rust impl agent | Architect (BackendTask contract) | rust-best-practices (error taxonomy, M-APP-ERROR); security (A09 error wrapping) |
| P3 | Rust impl agent | Data-integrity reviewer — mandatory | security (A08 safe deserialization, A04 fail-safe migration, ASVS V14.2 secret boundary — seeds never enter persister) |
| P4a | Rust impl agent | Architect (fund-safety reviewer gate — no snapshot-based coin selection) | rust-best-practices (M-DONT-LEAK-TYPES); security (A04 — no snapshot-based spend path) |
| P4a.5 | Rust impl agent | Smythe security reviewer (mandatory — I1–I6 invariants; FundWithUtxo removal disclosure; asset-lock finality channel intact) | security (A04 double-spend, no parallel spend engine); rust-best-practices (M-NO-TOMBSTONES) |
| P4b | Rust impl agent | Correctness reviewer — mandatory (DIP-14/15 parity gate green; P4a.5 exit green) | rust-best-practices (M-NO-TOMBSTONES, test quality) |
| P5 | Rust impl agent + Architect + Smythe (release-blocking audit) | — | Full static verification; Smythe I1–I6 audit |

### QA Matrix

All phases run the standard baseline:
- `cargo clippy --all-features --all-targets -- -D warnings`
- `cargo +nightly fmt --all`
- `cargo test --all-features --workspace`

Plus targeted lanes:

| Lane | Phases | What |
|---|---|---|
| **Compile-floor verification** | P0.5 | `cargo build` + `cargo clippy` green. Tests need not pass — failing tests left with `#[ignore]` + `// TODO(P0.5): re-enable in P{1,2,3}`. |
| **SpvRuntime-owned-sync verification** | P0 | Prove `PlatformWalletManager.start()` → blocks sync → balances/tx appear via `PlatformEventHandler` with **no DET sync code running**. This is the load-bearing assumption of the whole spec — it must be confirmed before P1. |
| **DIP-14/15 golden-vector parity** | P0 (probe), P4 (regression) | Byte-equality of contact xpub + payment addresses + `calculate_account_reference`, both identifier classes (low / full 256-bit), both networks. Full-256-bit divergence = accepted structural finding (recorded trade-off in data-model-and-migration.md); no longer release-blocking in the prior sense. See [data-model-and-migration.md](data-model-and-migration.md) — "Accepted fund-accessibility trade-off". |
| **`PersistedWalletLoader` seam** | P1, P5 (regression) | Mock yields exactly N `WalletRegistration` for N seed-store wallets; backend registers all N and `start()` succeeds. Swap-boundary compiles with alternate `StubFromPersisted`. Seed-decrypt failure surfaces existing typed `TaskError`. See [g2-mock-boundary.md §G2.6](g2-mock-boundary.md#g26--phasing-and-qa). |
| **Simplified Stage-B migration lane (release-blocking)** | P3c–P3e, P4 | Fixtures: wallets re-registered, identities added, contacts re-established on upstream derivation, legacy tables dropped on SUCCESS, backup exists, exception path restores, marker clears ⇔ complete success, reentrant single-run, user-never-unlocks. No quarantine fixtures — quarantine apparatus WITHDRAWN. PLUS: legacy-address-abandonment notice (shows exactly once, one-shot flag, dismissible, all migrated users). See [data-model-and-migration.md](data-model-and-migration.md). |
| **One-time-migration lane** | P3 | Synthetic legacy DB (HD + single-key + identities + DashPay) → migrate → assert: wallets re-registered via `SeedReregistrationLoader`, identities present, contacts re-established upstream, legacy HD/UTXO/SPV/DashPay/contact tables dropped, single-key preserved+flagged, backup file exists, failure path restores. |
| **Fund-safety spend-path (P4a.5 test lanes)** | P4a.5, P5 | Post-migration asset-lock via `WalletBackend::create_asset_lock_proof` (no legacy coin-selection in hot path); Path 3 asset-lock finality without `Wallet` mutation; crash-retry no-double-broadcast. All three lanes release-blocking at P5 Smythe audit. |
| **Backend E2E (testnet, `tests/backend-e2e/`)** | P2, P4 | Wallet load, balance, send, identity register/top-up, DashPay contact — through `WalletBackend`. |
| **ConnectionStatus adapter** | P4, P5 | UI sync-progress visually matches former SPV behavior, fed from `SpvRuntime::sync_progress()`. |
| **Single-key stub** | P2, P5 | Stub returns typed error + correct banner; swap-boundary trait compiles with a no-op alternate impl. |
| **Post-migration UI data-path test (release-blocking)** | P4a, P5 | Synthetic legacy DB → P3c migrate (tables dropped) → assert wallets screen shows correct balance + tx history + UTXO set from the upstream snapshot (not blank, not stale); assert snapshot updates on an `EventBridge` `TaskResult::Refresh`. Assert empty pre-sync snapshot renders as "syncing", not a zero-balance bug. Runs alongside §2(d) migration-notice regression + migration crash/restore lane + clippy `-D warnings` + `+nightly fmt` + full workspace + backend-e2e. Single push to #860 at P5 end. |

`docs/user-stories.md` updated at P4 (RPC mode removed, single-key degraded). `claudius:lessons-learned` invoked at each phase close.

---

## Highest-Risk Assumption — Explicit Verdict

**Decision #1 ("platform-wallet owns SPV internally") is CONFIRMED, not contradicted, at PR #3625 head.**

Evidence: `Cargo.toml` direct `dash-spv` dep; `SpvRuntime` constructs/owns `DashSpvClient` and runs its own sync loop (`run`/`spawn_in_background`); `PlatformWalletManager` owns the `SpvRuntime`; no host-feed API exists. DET's `src/spv/` is deletable. The only residue is a thin DET-side `ConnectionStatus` display adapter fed by upstream events — not chain sync.

**Remaining live risks (updated for resolved decisions):**

1. **G1 — release-hardening track.** DET is now pinned to #3625 head (Decision #1). G1 is no longer a start blocker; it resolves at release time to a merged + released platform rev for P3+.
2. **G2 — deferred swap.** Mitigated by `PersistedWalletLoader` seam ([g2-mock-boundary.md](g2-mock-boundary.md)). Not a gate.
3. **DIP-14/15 migration — simplified Stage-B QA lane.** The simplified upstream-only migration path must be implemented (P3c) and QA-proven (P3d–P3e). P0 full-256-bit probe divergence is now an ACCEPTED trade-off (user decision 2026-05-18) — see [data-model-and-migration.md](data-model-and-migration.md) "Accepted fund-accessibility trade-off". Quarantine apparatus WITHDRAWN.
4. **P0.5 mis-classification risk.** Highest risk: treating Clusters E/F/G (SDK-drift fixups) as stub candidates. Any `unimplemented!()` in retained code is a silent capability loss. Escalate expensive fixups; never stub retained code.
