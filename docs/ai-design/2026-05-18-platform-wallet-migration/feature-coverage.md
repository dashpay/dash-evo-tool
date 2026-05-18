# Feature Coverage

**Purpose:** Two analyses — (1) the RPC-vs-SPV capability matrix classifying what is fundamentally RPC-only, fixably SPV-unwired, or equivalent in both modes; (2) the reverse gap: DET features absent from `platform-wallet` at PR #3625 head, including the corrected upstream export surface that widens Phase-4 deletion opportunity.

> **Note:** Supporting analysis for the clean-slate rewrite spec — see [README.md](README.md). The incremental-phase references in this file (e.g. "Phase 1–4") are from the prior incremental plan and are superseded. The substantive capability and gap analysis remains valid background for the rewrite.

[← back to README](README.md)

---

## Section 1 — RPC-vs-SPV Capability Matrix

Categories:
- **1 = Fundamentally impossible under a light client** — protocol constraint; cannot be fixed by wiring.
- **2 = Possible under SPV but RPC-only in current code** — a wiring gap, not a protocol gap; fixable independently of this migration.
- **3 = Equivalent in both modes** — already works identically regardless of backend.

### Category 1 — Genuinely RPC-Only / Impossible Under SPV

| Feature | Entry point | Protocol reason |
|---|---|---|
| Mining / `generatetoaddress` | `MineBlocks`, `src/backend_task/core/mod.rs:346` | Block production (template, mempool assembly, PoW) is a full-node function. Always RPC; Regtest/Devnet-only feature. |
| Arbitrary historical tx lookup by txid | `get_raw_transaction`, `src/backend_task/core/recover_asset_locks.rs:21` | BIP157/158 SPV only sees transactions matching a registered filter. DET sidesteps needed cases via DAPI, not Core. |
| Retrospective UTXO scan over an arbitrary address set | `recover_asset_locks` RPC mechanism, `src/backend_task/core/recover_asset_locks.rs:20-24` | Needs node-side address index; SPV must have been watching as blocks arrived. |
| `importaddress` into Core's own wallet | `src/model/wallet/mod.rs:1202`, used by `refresh_single_key_wallet_info.rs:40` | No server-side wallet exists under SPV. |
| Named multi-wallet Core RPC | `core_client_for_wallet` `src/context/mod.rs:686`; `Wallet.core_wallet_name` `src/model/wallet/mod.rs:390` | Presupposes a Core node with `-rpcwallet` namespaces. |

> NOTE on the "recover asset locks" entry: the **user-facing goal** (recover known asset locks) IS achieved under SPV via live InstantLock/ChainLock event reconciliation (`src/context/wallet_lifecycle.rs:619`). The SPV arm of `recover_asset_locks` returns zero-count success (`src/backend_task/core/recover_asset_locks.rs:30-39`). Only the RPC retrospective-scan *technique* is category 1 — not the feature itself.

### Category 2 — The ONE Fixable User-Facing Gap

Single-key / non-HD wallet **balance and UTXO refresh** hard-errors under SPV:

`src/backend_task/core/refresh_single_key_wallet_info.rs:23` returns `Err(TaskError::OperationRequiresDashCore{...})`.

A single arbitrary P2PKH address is matchable by BIP158 compact block filters. The SPV path simply never registers these ad-hoc keys — it is a wiring gap, not a protocol impossibility. Fixable independently of this migration (it never touches `WalletManager<W>`).

### Category 3 — Equivalent in Both Modes

| Feature | Evidence |
|---|---|
| HD wallet refresh / send | `src/backend_task/core/mod.rs:230`, `src/context/wallet_lifecycle.rs:353` |
| Single-key *send* (broadcast is mode-aware) | `src/context/transaction_processing.rs:22-51`; `src/backend_task/core/send_single_key_wallet_payment.rs:176` |
| Raw broadcast | `src/backend_task/core/mod.rs:510-512` |
| Asset-lock create + finality | `src/backend_task/core/create_asset_lock.rs:42,95` |
| Generate receive address | `src/backend_task/wallet/generate_receive_address.rs:20` |
| Fund platform from UTXOs | `src/backend_task/wallet/fund_platform_address_from_wallet_utxos.rs:114-134` |
| Shielded post-timeout refresh | `src/backend_task/shielded/bundle.rs:575` (degraded mode, not impossible) |
| All identity / DashPay / Platform ops | Backend-agnostic via DAPI (`self.sdk`); verified zero matches for `SpvManager`/`WalletManager` in `src/backend_task/identity/` and `src/backend_task/dashpay/` — see [backend-architecture.md](backend-architecture.md) |
| Mining (Regtest/Devnet) | `src/backend_task/core/mod.rs:330-358` — always RPC; `CoreBackendMode` already branches here, SPV mode returns early |

### Bottom Line

An SPV-heavy future costs essentially **one fixable feature** (single-key wallet refresh — category 2) plus mining (developer-only). No production user-facing capability is permanently lost to SPV.

The `platform-wallet` migration is orthogonal to this gap. `dash-spv` stays the only light-client engine; the RPC-only Core paths never touch `WalletManager`, so the generic swap neither widens nor narrows the SPV/RPC gap.

---

## Section 2 — Reverse Gap: DET Features Absent from `platform-wallet`

Verified at PR #3625 head `738091f734e05c7a1b822bb1ebff336c93b67891`.

### Important: Upstream Provides More Than First Surveyed

> **This is a consequential finding for Phase 4.** The upstream `lib.rs` at PR head exports a significantly broader surface than the initial architecture survey credited. Several DET (b)-class orchestration files can shrink further in Phase 4 than originally planned. This **widens the deletion opportunity** — it does NOT add a gate or reshape G1/G2.

Confirmed upstream exports at PR head include:

- `PlatformWalletManager`
- `SpvRuntime` — owns `DashSpvClient` and drives its own sync loop; `PlatformWalletManager` owns `SpvRuntime`; DET `src/spv/` is fully deletable (see [upstream-reality.md](upstream-reality.md))
- `broadcaster`
- `AssetLockManager`
- `CoreWallet` / `WalletBalance`
- `IdentityManager` / `ManagedIdentity`
- `IdentityWallet<B>` — SDK handle covering identity-lifecycle AND DashPay operations
- Full DashPay type set: `ContactRequest`, `EstablishedContact`, `DashPayProfile`
- Full DashPay derivation functions: `derive_contact_xpub`, `derive_contact_payment_address(_es)`, `derive_auto_accept_private_key`, `calculate_account_reference`, `calculate_avatar_hash`, `calculate_dhash_fingerprint`
- `DpnsNameInfo` — read-only data type; no register flow upstream
- `IdentityFunding` / `TopUpFundingMethod`
- `TokenBalanceChangeSet` / `IdentityTokenSyncInfo` — balance sync only, not token administration
- `PlatformAddressSyncManager`

This means DashPay derivation functions (`derive_contact_xpub`, `derive_contact_payment_address(_es)`, `calculate_account_reference`) are **upstream-full** — the DET hand-rolled equivalents (`src/backend_task/dashpay/dip14_derivation.rs`, `hd_derivation.rs`) are P4 deletion targets, conditioned on one-time migration execution + hard-stop path proven per the migrate-or-quarantine policy (see [dip14-migration-hardstop.md §6.5](dip14-migration-hardstop.md#65--p0-probe-and-phasing-interaction) and [phasing.md QA matrix](phasing.md#qa-matrix)).

### Feature Gap Table

| DET feature / domain | Upstream status | Class | DET files that stay | Upstream ref |
|---|---|---|---|---|
| **SPV chain sync** | Full — `SpvRuntime` constructs `DashSpvClient` internally and runs its own sync loop; `PlatformWalletManager` owns `SpvRuntime`; no host-feed API exists | (a→deleted) | DET `src/spv/**` and `reconcile_spv_wallets` are deleted; only a thin `ConnectionStatus` adapter remains — see [upstream-reality.md](upstream-reality.md) and [removal-inventory.md](removal-inventory.md) | `SpvRuntime`, `PlatformWalletManager` |
| **Shielded / zk** | None | (a) | `src/backend_task/shielded/*`, `src/model/wallet/shielded.rs`, `src/context/shielded.rs`, `src/database/shielded.rs`, `src/model/grovestark_prover.rs` | — |
| **DPNS registration + contested-name / masternode voting** | `DpnsNameInfo` read-only type only; no register flow | (a) | `src/backend_task/identity/register_dpns_name.rs`, `src/backend_task/contested_names/*`, `src/database/scheduled_votes.rs` | `DpnsNameInfo` |
| **Token administration** (17 files) | `TokenBalanceChangeSet`/`IdentityTokenSyncInfo` for balance sync only | (a) | `src/backend_task/tokens/*` | Balance sync types only |
| **Document / data-contract CRUD + generic ST broadcast** | None | (a) | `src/backend_task/{document,contract,register_contract,update_data_contract,broadcast_state_transition}.rs` | — |
| **Fee estimation** | None | (a) | `src/model/fee_estimation.rs` | — |
| **Persister-excluded DET persistence** | Explicitly out of upstream scope | (a) | `QualifiedIdentity` blob (`src/database/identities.rs:157`), platform-address balances (`src/backend_task/wallet/fetch_platform_address_balances.rs`), token balances | Upstream trait doc "Outside scope" section |
| **GUI / MCP / CLI / settings / ZMQ** | None | (a) | `src/ui/**`, `src/mcp/**`, `src/bin/det_cli/**`, `src/context/settings_db.rs`, `components/core_zmq_listener` | — |
| **Identity lifecycle orchestration** (register/topup/transfer/withdraw/add-key ST flow) + `QualifiedIdentity` model | `IdentityWallet<B>`, `IdentityManager`, `ManagedIdentity` — upstream primitives; DET orchestration and blob stay | (b) | `src/backend_task/identity/*` (shrinks in Phase 4), `src/database/identities.rs` | `IdentityWallet<B>`, `IdentityManager` |
| **DashPay orchestration** (contact-request/accept/auto-accept/incoming-payment/avatar I/O) | `IdentityWallet<B>` covers lifecycle + DashPay ops; full type set and derivation functions upstream | (b) | `src/backend_task/dashpay/*` except DIP-14/15 derivation (deleted P4, conditioned on migration executed + hard-stop path proven — see [dip14-migration-hardstop.md](dip14-migration-hardstop.md)) | `ContactRequest`, `EstablishedContact`, `DashPayProfile`, `derive_contact_xpub`, etc. |
| **Asset-lock funding-flow orchestration** | `AssetLockManager` upstream | (b) | Orchestration wiring in `src/backend_task/core/create_asset_lock.rs`, `src/context/transaction_processing.rs` | `AssetLockManager` |
| **Token-balance display** | `TokenBalanceChangeSet`/`IdentityTokenSyncInfo` for sync | (b) | Display/UI layer, token balance DB | Balance sync types |
| **DashPay ECDH encryption** | `derive_auto_accept_private_key` upstream; full ECDH pending Phase-0 parity confirmation | (b) | `src/backend_task/dashpay/encryption.rs` | `derive_auto_accept_private_key` |
| **Single-key / non-HD wallets** | None — no non-HD wallet type at PR head; not excluded by any documented scope boundary | **(c)** | `src/model/wallet/single_key.rs`, `src/database/single_key_wallet.rs`, `src/backend_task/core/{send_single_key_wallet_payment,refresh_single_key_wallet_info}.rs` | — |

### Classes Defined

- **(a) Out of upstream scope by design** — DET owns permanently; no migration action.
- **(b) Partial** — upstream provides a primitive or type; DET orchestration stays but may shrink in Phase 4 given the expanded upstream surface.
- **(c) In-scope but genuinely missing at this rev** — exactly one entry.

### The One Category-(c) Item: Single-Key / Non-HD Wallets

`PlatformWalletInfo` / `ManagedWalletInfo` is HD-seed-based. Upstream has no non-HD wallet type at PR head, and no documented scope boundary excludes it.

**Treatment in the rewrite:** Single-key wallets are mocked — operations return a typed `TaskError::SingleKeyWalletsUnsupported`; existing data is preserved and surfaced read-only. The `SingleKeyBackend` trait boundary makes a future swap a one-file construction change. See [single-key-mock.md](single-key-mock.md) and [open-questions.md #7](open-questions.md).

This also explains the category-2 SPV gap from Section 1: the single-key refresh gap (`refresh_single_key_wallet_info.rs:23`) is rendered moot by the stub — the code path no longer runs under the new backend.
