# Architecture

**Purpose:** Target component layout, `DetWalletManager` newtype rationale, `PlatformWalletInfo`/`IdentityManager` placement, persistence design, two-DB coexistence, and secret boundary.

[← back to README](README.md)

---

## A. Target Architecture

### `PlatformWalletInfo` Placement

`PlatformWalletInfo` becomes the `W` type parameter of the upstream `WalletManager<W>` owned by `SpvManager` (`src/spv/manager.rs:133,159,366`), replacing `ManagedWalletInfo`.

The upstream composition is:

```rust
// packages/rs-platform-wallet/src/wallet/platform_wallet.rs
PlatformWalletInfo {
    wallet_info: ManagedWalletInfo,
    identity_manager: IdentityManager,
}
```

`ManagedWalletInfo` is unchanged inside `PlatformWalletInfo`. This is the structural reason the migration is tractable: the inner type is preserved, and the outer type adds to it rather than replacing it.

### `IdentityManager` Placement

`IdentityManager` becomes the in-memory authority for identities, credits, and DashPay contact sync state, fed and persisted via the upstream changeset pipeline.

dash-evo-tool's `QualifiedIdentity` bincode blob in the `identity` table (`src/database/identities.rs:157`) **remains** dash-evo-tool's persisted truth and UI model. The upstream trait doc (`packages/rs-platform-wallet/src/changeset/traits.rs`, "Outside scope" section) explicitly defers moving that blob to a later milestone ("evo-tool task #130 / Phase 9c"). The two representations coexist by upstream design, not as a workaround. See [open-questions.md § #6](open-questions.md) for the confirmation needed.

### `DetWalletManager` Newtype

**Decision: introduce a local `DetWalletManager` newtype; no `WalletBackend` trait; RPC path untouched; enum dispatch retained.**

Today `SpvManager::wallet()` returns `Arc<AsyncRwLock<WalletManager<ManagedWalletInfo>>>` (`src/spv/manager.rs:706`), leaking the upstream generic to four consumers:

| Consumer | Location |
|---|---|
| `reconcile_spv_wallets` | `src/context/wallet_lifecycle.rs:759` |
| `build_spv_unsigned_transaction_multi` | `src/backend_task/core/mod.rs:677` |
| `sign_spv_transaction` | `src/backend_task/core/mod.rs:900` |
| MCP wallet tool | `src/mcp/tools/wallet.rs:73` |

Wrapping in `pub(crate) struct DetWalletManager(WalletManager<…>)` and exposing only DET-shaped accessors localizes the generic flip to one module. Phase 1 wraps with `ManagedWalletInfo`; Phase 3 changes only the inner type. This is a frozen contract boundary — the four consumers above see no change at Phase 3. (Rust best-practice rationale: M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE.)

**Why no `WalletBackend` trait.** SPV and RPC are two data sources, not polymorphic behavior. `CoreBackendMode` enum and `FeatureGate` already localize the ~34 branch sites. A trait would add `dyn`/generic indirection the codebase does not need. The newtype is the abstraction (M-DI-HIERARCHY: types > generics > dyn).

### RPC Path — Untouched

Verified by `grep` across all of `src/backend_task/identity/` and `src/backend_task/dashpay/` for `spv_manager|SpvManager|reconcile_spv|.wallet()|WalletManager|load_wallet_from_seed|next_bip44`: **zero matches**.

Identity and DashPay reach Platform only via `self.sdk` (DAPI) — e.g., `src/backend_task/identity/register_identity.rs:39,535`; `load_identity_from_wallet.rs:27-58`. RPC payments use `core_client.send_raw_transaction` (`src/backend_task/core/mod.rs:543`), never `WalletManager`. The generic swap cannot reach the RPC payment path. See [spv-rpc-correctness.md](spv-rpc-correctness.md) for the per-phase verdict table.

---

## C. Persistence Design

### `PlatformWalletPersistence` — The Concrete Trait

Confirmed unchanged at PR #3625 head `738091f734…` (`packages/rs-platform-wallet/src/changeset/traits.rs:118`):

```rust
pub trait PlatformWalletPersistence: Send + Sync {
    fn store(&self, wallet_id: WalletId, changeset: PlatformWalletChangeSet) -> Result<(), PersistenceError>;
    fn flush(&self, wallet_id: WalletId) -> Result<(), PersistenceError>;
    fn load(&self) -> Result<ClientStartState, PersistenceError>;
    fn get_core_tx_record(&self, wallet_id: WalletId, txid: &Txid)
        -> Result<Option<TransactionRecord>, PersistenceError> { Ok(None) } // default impl
}
```

Key properties:
- Sync, `Send+Sync`, object-safe — usable behind `Arc<dyn PlatformWalletPersistence>`.
- `&self` receiver with `wallet_id` parameter; the impl owns its internal locking.
- `store` MAY flush inline. The canonical `SqliteWalletPersister` flushes on every `store` (no batch window). Callers must not assume `store` is I/O-free — concurrency implications apply (ASVS A04 note).
- `PersistenceError` is a concrete `thiserror` enum (`LockPoisoned`, `Backend(String)`), with `From<String>/<&str>`.

dash-evo-tool wraps this as a dedicated `TaskError::PlatformWalletPersistence { #[source] PersistenceError }` — per CLAUDE.md error-taxonomy rules, no catch-all string variant.

### Consume, Do Not Build

The canonical SQLite persister already exists: `SqliteWalletPersister` in the new `platform-wallet-storage` crate (PR #3625). It ships 18 per-wallet tables, refinery+barrel migrations, online backup, automatic pre-destructive backups, and a maintenance CLI. The PR body states it was extracted from dash-evo-tool's own downstream persister. dash-evo-tool writes no persister code — it instantiates and wires one. This reduces Phase 2 effort to S/M, not L.

### Persister Scope

From the upstream trait doc:

**In scope (persister owns):**
- Wallet core state: height, address-pool watermarks, UTXO set, per-account tx records
- Asset locks
- Identity-level state (`IdentityEntry`): wallet_id/index, DPNS usernames, top-up history, lifecycle status, public key storage, DashPay profile and payment history
- Contacts: sent/incoming/established

**Out of scope (DET keeps owning):**
- The raw `identity.data` `QualifiedIdentity` blob
- Platform addresses
- Token balances

### Two-DB Coexistence (Recommended)

Keep dash-evo-tool's existing `Database` (owns `QualifiedIdentity` blob, platform addresses, token balances — all explicitly out of persister scope). The persister owns its own separate `.db` file with its own refinery migrations.

Rationale: zero migration collision, lowest blast radius, the persister self-manages its schema. Unifying into one file is deferred until justified. dash-evo-tool's `wallet`/`utxos`/`wallet_transactions` tables remain reconcile-fed projections until/unless Gate G2 closes upstream.

Existing users: this is additive and forward-only. No data movement on upgrade — the `QualifiedIdentity` blob stays in place per upstream design.

For `DiskStorageManager` (SPV chain/header/filter cache): see [verification.md § E.2](verification.md#e2--diskstoragemanager-byte-compat).

### Secret Boundary

Confirmed against `packages/rs-platform-wallet-storage/SECRETS.md`: the persister never sees seeds, mnemonics, or private keys. The `identity_keys` schema stores public-material only; this is CI-enforced (`tests/secrets_scan.rs` forbids `private|mnemonic|seed|xpriv|secret` in schema).

dash-evo-tool's adapter needs no additional encryption boundary. dash-evo-tool's existing seed encryption (`src/model/wallet/encryption.rs`, `src/model/qualified_identity/encrypted_key_storage.rs`) is untouched and unrelated. No upstream contradiction found. A future upstream `SecretStore` submodule is reserved-only — track, no action needed now. An ASVS V14.2 regression check runs at Phase 2 review to confirm the DET seed-encryption store is untouched.
