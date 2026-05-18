# Removal Inventory

**Purpose:** Definitive DELETE vs RETAIN lists; RPC backend mode fate and the thin Core-RPC mining utility that survives it.

[← back to README](README.md)

---

Cross-references: [backendtask-contract.md](backendtask-contract.md) for task-level disposition; [feature-coverage.md § Section 2](feature-coverage.md#section-2--reverse-gap-det-features-absent-from-platform-wallet) for the full reverse-gap analysis.

## E. Destructive Removal Inventory

### DELETE

**Chain sync infrastructure — fully delegated to upstream `SpvRuntime`:**
- Entire `src/spv/` — `manager.rs` (1528L), `error.rs`, `mod.rs`, `tests.rs`
- `reconcile_spv_wallets` + `sync_spv_account_addresses` + `spv_setup_finality_listener` + `spv_setup_reconcile_listener` + `handle_spv_finality_event` (`src/context/wallet_lifecycle.rs:619-985`)

**RPC wallet path:**
- `send_wallet_payment_via_rpc`
- RPC arms of `refresh_wallet_info`
- `recover_asset_locks` RPC body
- `core_client_for_wallet` for wallet ops (`src/context/mod.rs:686` — see RPC Fate below for what survives)
- `bootstrap_wallet_addresses` RPC branch (`wallet_lifecycle.rs:353`)
- `try_import_address` (`model/wallet/mod.rs:1202`)

**Backend mode machinery:**
- `CoreBackendMode` enum
- `FeatureGate::{SpvBackend,RpcBackend}`
- Every `core_backend_mode()` branch (~34 sites)
- `set_core_backend_mode*` (`src/context/mod.rs:427-502`)

**SPV `ConnectionStatus` plumbing — replaced by thin event-fed adapter:**
- `dash_spv::sync::*` imports
- SPV atomics + `set_spv_status` (`src/context/connection_status.rs:8,55,168` etc.)

**Wallet model and DashPay derivation:**
- Most of `src/model/wallet/mod.rs` (`Wallet` struct minus what migration reads)
- `src/model/wallet/utxos.rs`, balance/UTXO/tx logic
- `src/backend_task/dashpay/dip14_derivation.rs` + `hd_derivation.rs` (delegated upstream, subject to one-time migration execution + hard-stop path proven — see [dip14-migration-hardstop.md §6.5](dip14-migration-hardstop.md#65--p0-probe-and-phasing-interaction) and [phasing.md QA matrix](phasing.md#qa-matrix))

**DET wallet/UTXO/tx persistence:**
- `src/database/wallet.rs` — `wallet`/`utxo`/`tx` tables + balance writers
- `src/database/utxo.rs` (after migration)

**SPV context wiring:**
- `src/context_provider_spv.rs` — SPV provider wiring
- `spv_context_provider`/`rpc_context_provider` switching

**Dead settings:**
- `core_backend_mode`, `use_local_spv_node`, `auto_start_spv` (`database/initialization.rs:511-512`)

### RETAIN

**UI** — `src/ui/**` minus: RPC-mode toggle, Core-wallet picker, "Local Dash Core node" settings (`network_chooser_screen`). SPV progress UI rewired to upstream `sync_progress()` source.

**MCP / CLI** — `src/mcp/**`, `src/bin/det_cli/**`. `ensure_spv_synced` rewired to `SpvRuntime::sync_progress()`.

**Shielded / zk** — `src/backend_task/shielded/*`, `model/wallet/shielded.rs`, `context/shielded.rs`, `database/shielded.rs`, `model/grovestark_prover.rs`. Out of scope (or upstream shielded feature, future).

**Contested-name voting / scheduled votes.**

**Token administration** — 17 files in `src/backend_task/tokens/*`.

**Document / contract CRUD + generic state-transition broadcast.**

**Fee estimation** — `src/model/fee_estimation.rs` (CLAUDE.md: centralized fee logic, never inline).

**Identity persistence** — `QualifiedIdentity` model + `database/identities.rs` blob; upstream "Outside scope."

**Platform-address + token-balance tables** — upstream "Outside scope."

**Single-key wallet** — `model/wallet/single_key.rs`, `database/single_key_wallet.rs`, single-key task paths — preserved, stubbed, not dropped. See [single-key-mock.md](single-key-mock.md).

**Settings / network config** — minus dead columns.

**ZMQ listener** — `components/core_zmq_listener` — audit before P4; delete only if no non-wallet consumer exists (Decision #3 — see [open-questions.md #3](open-questions.md)).

---

## Retained Code Requiring SDK-Rev Signature Updates at P0.5

The following clusters appear in the P0.5 error list but are **explicitly NOT migration deletions**. They are retained code broken only by upstream API drift. Future readers must not mistake these drift fixes for removal inventory items.

| Cluster | Location | Classification | Notes |
|---|---|---|---|
| **E — Qualified identity** | `src/model/qualified_identity/mod.rs` | SDK-DRIFT-FIXUP — RETAIN | Fix `sign`/`sign_create_witness` lifetimes, `AddressProvider` members, missing assoc types `Tag`/`Address`. Out of migration scope. |
| **F — Shielded bundle** | `src/backend_task/shielded/bundle.rs` | SDK-DRIFT-FIXUP — RETAIN | Fix `AddressKey`→`AddressOps`, async `?`-on-`Pin<Box<dyn Future>>`, trait sigs. If unfixable in budget, escalate as BLOCKING; do not stub. |
| **G — Identity/contract tasks** | `src/backend_task/identity/**`, `src/backend_task/contract/**` | SDK-DRIFT-FIXUP — RETAIN | Mechanical signature and import updates. Never stub. |

**Rule:** Never place `unimplemented!()` or a stub error in Cluster E, F, or G code. Silent capability loss on retained code is an A04 violation. If a fixup is too expensive, it surfaces as a blocking finding — not a quiet stub.

---

## RPC Backend Mode — Fate

`CoreBackendMode` collapses entirely. `platform-wallet` is SPV-internal only — no RPC/full-node wallet option (`Cargo.toml` has `dash-spv` only; `SpvRuntime` is the sole chain backend). With chain sync owned by `platform-wallet`, the dual RPC/SPV wallet mode disappears: enum, settings, UI toggle, all RPC wallet call sites — deleted.

**What was RPC-only and is now simply gone (no replacement needed):**
- `importaddress` — no server-side wallet under the new backend
- Named-wallet RPC (`-rpcwallet` namespaces, `core_client_for_wallet`, `Wallet.core_wallet_name`)
- `list_unspent` / `get_raw_transaction` retrospective scans — obsolete once `AssetLockManager` tracks continuously
- `recover_asset_locks` RPC body — replaced by `AssetLockManager` continuous tracking

**What was RPC-only and is genuinely still needed:**

Mining (`generatetoaddress`, Regtest/Devnet only) is the sole capability DET still needs that has no `platform-wallet` equivalent. Cross-reference: [feature-coverage.md § Category 1](feature-coverage.md#category-1--genuinely-rpc-only--impossible-under-spv).

**Spec: thin Core-RPC mining utility.**

Extract a standalone `src/core_rpc_util.rs` — approximately one Core RPC client + `generate_to_address`. Outside `WalletBackend`. Gated to Regtest/Devnet. Invoked only by `CoreTask::MineBlocks`. Not a wallet backend; no `CoreBackendMode`; no named-wallet support. Contract for `CoreTask::MineBlocks` is unchanged.
