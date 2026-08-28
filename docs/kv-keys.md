# DET k/v key reference

`DetKv` wraps the upstream `platform_wallet_storage::KvStore`. Values are encoded as `[ schema_version (1 byte) | bincode(payload) ]` using `bincode::config::standard()`. Keys are colon-separated namespaces. Every `DetKv` call takes a `DetScope` argument: `DetScope::Global` = global slot, `DetScope::Wallet(&seed_hash)` = per-wallet slot (cascades on wallet delete), `DetScope::Identity(&id)` = per-identity slot (active — used for identities, top-ups, scheduled votes, and DashPay `private`/`address_index` overlays), `DetScope::Token { identity_id, token_id }` = per-token slot (defined and mapped, currently unused — token balances are read live from upstream). `DetScope::Identity` and `DetScope::Token` map to the upstream `meta_identity` / `meta_token` tables; their metadata is reaped by an upstream `AFTER DELETE` soft-cascade when the parent object row is removed. `DetScope` is the DET-side seam over the upstream `ObjectId` enum — the upstream scope type never crosses the wallet-backend boundary.

Three backing stores exist:

| Store | Path | Contents |
|-------|------|----------|
| `det-app.sqlite` | `<data_dir>/det-app.sqlite` | Cross-network settings, wallet metadata, migration sentinel, single-key metadata |
| `det-<net>.sqlite` | `<data_dir>/det-<net>.sqlite` | Per-network identities, tokens, contracts, DashPay overlays, platform addresses, selected wallet |
| `SecretStore` | `<data_dir>/secrets/det-secrets.*` | Encrypted HD-wallet seed envelopes and imported single-key private bytes |

In the per-domain tables below, a `Scope` of `None` denotes `DetScope::Global`.

---

## Settings

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `det:settings:v1` | `None` | `det-app.sqlite` | `AppSettings` via `AppSettingsWire` | `network`, `root_screen_type`, `dash_qt_path`, `overwrite_dash_conf`, `disable_zmq`, `theme_mode`, `_reserved_core_backend_mode` (retired; reserved byte), `onboarding_completed`, `show_evonode_tools`, `user_mode`, `close_dash_qt_on_exit`, `auto_start_spv` |

Source: `src/model/settings.rs`, `src/context/settings_db.rs`

---

## Wallet metadata sidecar

DET-owned per-wallet display fields (alias, main flag, Dash Core wallet name link, pre-computed xpub). Stored in the cross-network `det-app.sqlite` so the wallet picker can enumerate every known wallet at cold boot without touching the per-network persister.

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `<network>:wallet_meta:<seed_hash_base58>` | `None` | `det-app.sqlite` | `WalletMeta` | `alias: String`, `is_main: bool`, `core_wallet_name: Option<String>`, `xpub_encoded: Vec<u8>` |

`<network>` is one of `mainnet`, `testnet`, `devnet`, `regtest`. `<seed_hash_base58>` is the 32-byte `WalletSeedHash` base58-encoded. Global (`DetScope::Global`) scope is used instead of per-wallet scope because the upstream `WalletId` does not exist until a wallet is registered with `PlatformWalletManager`.

Source: `src/wallet_backend/wallet_meta.rs`, `src/model/wallet/meta.rs`

---

## Single-key metadata sidecar

Enumerable index of imported single-key wallets. The private bytes live in `SecretStore`; this sidecar holds only the display-facing metadata so cold-boot hydration can reconstruct the in-memory index without enumerating the (non-enumerable) vault.

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `<network>:single_key_meta:<base58_addr>` | `None` | `det-app.sqlite` | `ImportedKey` | `address: String`, `alias: Option<String>`, `network: Network`, `has_passphrase: bool`, `passphrase_hint: Option<String>` |

`<network>` is the same vocabulary as wallet metadata. Global scope for the same reason: the sidecar must be listable independently of any per-wallet `WalletId`.

Source: `src/wallet_backend/single_key.rs`, `src/model/single_key.rs`

---

## Migration sentinel

Completion record written by the finish-unwire migration (`BackendTask::MigrationTask::FinishUnwire`). Read on every cold-start to short-circuit re-migration. Written once per network — idempotent.

The sentinel is **per-network**: each network gets its own key so an upgrade completed on mainnet does not suppress the migration for testnet after a network switch.

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `det:migration:finish_unwire:<network>:v1` | `None` | `det-app.sqlite` | `MigrationCompletion` | `completed_at: i64` (Unix seconds), `sha: String` (build SHA), `network_count: u32` |

`<network>` is one of `mainnet`, `testnet`, `devnet`, `regtest`. Example: `det:migration:finish_unwire:mainnet:v1`. The key is produced by `sentinel_key_for(network)` using the constants `SENTINEL_KEY_PREFIX` (`"det:migration:finish_unwire"`) and `SENTINEL_KEY_VERSION` (`"v1"`).

Source: `src/backend_task/migration/finish_unwire.rs` (`sentinel_key_for`, `SENTINEL_KEY_PREFIX`, `SENTINEL_KEY_VERSION`)

---

## Wallet selection

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `det:selected_wallet:v1` | `None` | `det-<net>.sqlite` | `SelectedWallet` | `hd_wallet_hash: Option<[u8;32]>`, `single_key_hash: Option<[u8;32]>` |

Source: `src/model/selected_wallet.rs`, `src/wallet_backend/mod.rs`

---

## Identities

The identity blob and top-up history are **identity-scoped** (`DetScope::Identity(&id)`) so the upstream soft-cascade reaps them when the identity row is deleted. `DetScope::Identity` has no cross-identity listing, so a Global `det:identity_index:v1` slot holds the complete id roster the load-all paths iterate. `det:identity_order:v1` is a separate user-ordering view (may lag the full set) and stays Global.

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:identity:v1` | `DetScope::Identity(&id)` | `det-<net>.sqlite` | `StoredQualifiedIdentity` | Fields: `qi_bytes` (inner bincode, redacted in `Debug`), `status: u8`, `identity_type: String`, `wallet_hash: Option<[u8;32]>`, `wallet_index: Option<u32>` |
| `det:identity_index:v1` | `None` | `det-<net>.sqlite` | `Vec<[u8;32]>` | Complete enumeration index of stored identity ids. Rewritten wholesale on every add/remove, so all read-modify-write access is serialized by one process-wide lock — absence from this roster authorizes the vault-cleanup sweep to delete an identity's private keys, and a lost update would forge that evidence |
| `det:identity_order:v1` | `None` | `det-<net>.sqlite` | `Vec<[u8;32]>` | User-chosen display ordering of identity ID raw bytes |
| `det:top_ups:v1` | `DetScope::Identity(&id)` | `det-<net>.sqlite` | `BTreeMap<u32, u64>` | Top-up history: account index → credits |
| `det:vault_cleanup_pending:v1:<id_base58>` | `None` | `det-<net>.sqlite` | `Vec<(StoredPrivateKeyTarget, KeyID)>` | Durable manifest of the vault-key placements a `delete_local_qualified_identity` call must still clear; persisted before that call's first mutation, cleared once every listed key is deleted (a clear that fails leaves a stale manifest, not a failed removal — the sweep re-runs the idempotent deletes and clears it). Global-scoped (not `DetScope::Identity`) so it survives the index removal that unlists `id` — the exact step it exists to protect against. Resumed by the boot-time `AppContext::resume_pending_vault_cleanups` sweep. |
| `det:identity_unloaded:v1:<id_base58>` | `None` | `det-<net>.sqlite` | `u64` | Presence-only marker that the user unloaded this identity from this device; the stored value is the unload's unix timestamp, kept for diagnostics only and never read for a decision. Written by `delete_local_qualified_identity` **before** `index_remove_identity` delists the identity, and by the devnet wipe for every identity it clears, so no window exists in which the identity is gone and the marker is not yet on file. Consulted by `AppContext::store_discovered_identity` — the single guarded store used by the discovery passes and by the `finish_unwire` migration import — which refuses to store an identity carrying one. That refusal is what makes an unload survive the automatic passes (boot sweep, post-unlock, wallet import, resumed migration), all of which re-derive or re-read the same identity from material that outlives the removal. The absent-record branch of `write_local_qualified_identity_locked` declines the same way, as a backstop covering every update path that could otherwise re-create a record for an identity a removal had already taken away. Global-scoped for the same reason as the manifest above: it must outlive the identity's own scope. Retired only by `insert_local_qualified_identity`, i.e. by the user deliberately loading the identity again. Never expired on a timer — an expiring tombstone is a resurrection with a delay — and never reaped: it grows by one entry per identity the user has actually unloaded on this network, a bound set by user action rather than by anything automatic, which any change letting an automatic path write these would break. |

**Exception to the cascade above**: an identity stored without a wallet association (`wallet_hash: None`) is *also* mirrored into the upstream `identities` table under the **unowned scope** — the all-zero `WalletId`, which upstream stores as a NULL `wallet_id`. Masternode/evonode nodes are the expected case, but any wallet-less identity DET stores takes this path (e.g. a `User` identity looked up by id with no owning wallet). The scope is load-bearing, not incidental: a NULL `wallet_id` activates no foreign key, so no wallet's `ON DELETE CASCADE` reaches the row, and the `cascade_meta_on_identity_delete` trigger — which would delete the identity's `meta_identity` rows, i.e. the `det:identity:v1` record above — never fires for it. That is the whole point: filing the same identity under a real wallet's scope would make removing that unrelated wallet destroy the node's DET record. These rows are also kept out of every wallet's `IdentityManager`, because the `identities` upsert promotes an unowned row to the first wallet that flushes it.

Written by `WalletBackend::ensure_identity_unowned` on insert; withdrawn — tombstoned, never row-deleted — by `WalletBackend::remove_unowned_identity` on `delete_local_qualified_identity`, best-effort. Both calls read the unowned scope back before reporting success — upstream logs a persister failure and returns `Ok` on either write — so a lost mirror or a lost tombstone is reported, never counted as done. Both directions are retried at the next boot by the two-way `AppContext::reconcile_unowned_identities`: it registers what the sidecar holds and upstream lacks, and withdraws what upstream holds and the sidecar lacks. Read back by `WalletBackend::unowned_identity_ids` (upstream's `load_unowned_identities`; the ordinary `load()` enumerates wallets and cannot reach them). The mirrored row carries no public keys and a `status` frozen at `Unknown` — upstream's out-of-wallet write path persists identity/balance/revision only ([tracked upstream](https://github.com/dashpay/platform/issues/4443)) — so the `det:identity:v1` record above stays the sole complete copy for these identities.

`wallet_hash` is decided by that mirror as well as by the caller. `insert_local_qualified_identity` runs the mirror **before** the vault and k/v writes. A caller's `None` becomes a stored `wallet_hash: None` when the mirror returns `Ok` — the unowned row was read back present — and, when it does not, only where the record already claims to be wallet-free (the `WalletLess` case below). Every mirror error is treated alike, whether it proved the row absent or only failed to read: upstream swallows a persist failure into `Ok(())` and also silently skips a row already filed under a wallet, so no error distinguishes a lost write from a refused one. What the failure costs is decided by what the record already claims (`stored_wallet_scope`, three cases):

- **Linked** (a wallet hash *and* an index on file) — the existing link is kept against the caller's `None`, and the user is warned via a sticky banner plus a `warn` log. A kept link leaves the wallet-less set the boot reconcile drives off, so no boot revisits it; a mislabel, not a loss.
- **WalletLess** (a record already carrying `wallet_hash: None`) — the write proceeds. It restates the wallet-free claim rather than making it, so the field the mirror guards does not change, and refusing would forfeit the rest of the write (an alias, or a whole refreshed identity on `LoadIdentity`) without repairing anything. Warned all the same — a `warn` log, plus a sticky banner raised only once the record is actually on disk — since the divergence outlives the write. The banner carries no identity id: it never auto-dismisses, the tray holds five and dedupes by exact text, so one message per identity would evict whatever else the user was reading. The id is in the `warn` log line; the banner's details carry the underlying error, which names the identity only when the error itself does. This is the common path: masternodes and evonodes are wallet-less by design and refresh through it.
- **Unestablished** (no record, or a `wallet_hash` without an index) — the write is **refused** with the mirror's own error and nothing is persisted, not even a vault entry: a wallet-free record here would be DET's own invention. A `stored_wallet_scope` read that itself fails is refused the same way, with the mirror's error, not the readback's.

Regression tests for all three branches, and for the swallowed mirror write and swallowed removal tombstone, live in `src/context/wallet_lifecycle/tests.rs`.

Source: `src/context/identity_db.rs`, `src/wallet_backend/identity_ops.rs`

---

## Scheduled votes

Scheduled votes are **voter-scoped** (`DetScope::Identity(&voter_id)`); the contested name is the key suffix. A Global `det:scheduled_vote_voters:v1` slot holds the complete set of voter ids that have at least one scheduled vote, driving the network-wide read / clear paths (Identity scope has no cross-voter listing).

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:scheduled_vote:<contested_name>` | `DetScope::Identity(&voter_id)` | `det-<net>.sqlite` | `StoredScheduledVote` | Fields: `voter_id: [u8;32]`, `contested_name: String`, `choice: StoredVoteChoice`, `unix_timestamp: u64`, `executed_successfully: bool` |
| `det:scheduled_vote_voters:v1` | `None` | `det-<net>.sqlite` | `Vec<[u8;32]>` | Enumeration index of voter ids with scheduled votes |

Source: `src/context/identity_db.rs`

---

## Contested names (DPNS)

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:contested_name:<normalized_name>` | `None` | `det-<net>.sqlite` | `StoredContestedName` | Fields: `normalized_contested_name`, `locked_votes`, `abstain_votes`, `awarded_to`, `end_time`, `locked`, `last_updated`, `contestants: Vec<StoredContestant>` |

`StoredContestant` fields: `id: [u8;32]`, `name`, `info`, `votes: u32`, `created_at`, `created_at_block_height`, `created_at_core_block_height`, `document_id: [u8;32]`.

Source: `src/context/contested_names_db.rs`

---

## Contracts

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:contract:<base58_contract_id>` | `None` | `det-<net>.sqlite` | `StoredContract` | Fields: `contract_bytes: Vec<u8>` (platform-serialized), `alias: Option<String>` |

Source: `src/context/contract_token_db.rs`

---

## Tokens

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:token:<base58_token_id>` | `None` | `det-<net>.sqlite` | `StoredToken` | Fields: `config_bytes: Vec<u8>` (bincode `TokenConfiguration`), `alias: String`, `data_contract_id: [u8;32]`, `position: u16` |
| `det:token_order:v1` | `None` | `det-<net>.sqlite` | `Vec<([u8;32],[u8;32])>` | Ordered `(token_id, identity_id)` pairs for My Tokens screen |

Per-`(identity, token)` balances are no longer cached by DET. They are read live from the upstream `IdentitySyncManager` through the `TokenBalanceView` seam (`src/wallet_backend/token_balance.rs`), which is fed a lock-free snapshot refreshed off the UI thread.

Source: `src/context/contract_token_db.rs`

---

## Platform addresses

Both keys use **per-wallet scope** (`DetScope::Wallet(&seed_hash)`) so entries cascade on wallet removal. Reads/writes route through the `PlatformAddressView` seam (`src/wallet_backend/platform_address.rs`); the cache stays active because upstream's public per-address reader exposes balance but not the DET-tracked nonce.

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:platform_addr:<canonical_address>` | `DetScope::Wallet(&seed_hash)` | `det-<net>.sqlite` | `StoredPlatformAddressInfo` | Fields: `balance: u64`, `nonce: u32` |
| `det:platform_sync:v1` | `DetScope::Wallet(&seed_hash)` | `det-<net>.sqlite` | `StoredPlatformSyncInfo` | Fields: `last_sync_timestamp: u64`, `sync_height: u64` |

Source: `src/context/platform_address_db.rs`, `src/wallet_backend/platform_address.rs`

---

## DashPay sidecar

The per-network `det-<net>.sqlite` already partitions DashPay data by network, so no `<network>:` prefix is needed within a key. Owner-specific decisions and recovery state use `DetScope::Identity(&owner)`; the owner id is carried by the scope and the upstream soft-cascade reaps those values when the owner identity row is deleted.

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:dashpay:blocked:<base58_contact_id>` | `DetScope::Identity(&owner)` | `det-<net>.sqlite` | `()` | Presence-only flag: contact is blocked |
| `det:dashpay:declined:<base58_counterparty_id>` | `DetScope::Identity(&owner)` | `det-<net>.sqlite` | `()` | Presence-only flag: incoming contact request declined |
| `det:dashpay:withdrawn:<base58_counterparty_id>` | `DetScope::Identity(&owner)` | `det-<net>.sqlite` | `()` | Presence-only flag: outgoing contact request withdrawn |
| `det:dashpay:request_action:<decline|cancel>:<base58_request_id>` | `DetScope::Identity(&owner)` | `det-<net>.sqlite` | `ContactRequestActionPhase` | Durable recovery phase for a paid hide/corrective-unhide followed by a local marker write |
| `det:dashpay:timestamps:<base58_entity_id>` | `None` | `det-<net>.sqlite` | `(i64, i64)` | DET-local `(created_at_ms, updated_at_ms)` |
| `det:dashpay:private:<base58_contact>` | `DetScope::Identity(&owner)` | `det-<net>.sqlite` | `ContactPrivateInfo` | Fields: `nickname: String`, `notes: String`, `is_hidden: bool` |
| `det:dashpay:address_index:<base58_contact>` | `DetScope::Identity(&owner)` | `det-<net>.sqlite` | `ContactAddressIndex` | Fields: `owner_identity_id: Vec<u8>`, `contact_identity_id: Vec<u8>`, `next_send_index: u32`, `highest_receive_index: u32`, `bloom_registered_count: u32` |
| `det:dashpay:addr_map:<base58_owner>:<address>` | `None` | `det-<net>.sqlite` | `([u8;32], u32)` | Reverse map: wallet address → `(contact_id_bytes, index)` |

Source: `src/wallet_backend/dashpay.rs`, `src/model/dashpay.rs`

---

## SecretStore entries

The `SecretStore` file backend (`<data_dir>/secrets/det-secrets.*`) stores opaque encrypted blobs. It is **not** a `KvStore` and does not use the `DetKv` bincode-plus-version-byte envelope. Entries are addressed by `(WalletId scope, label)` pairs.

### HD wallet seed envelopes

| Service (scope) | Label | Value encoding | Struct | Fields |
|-----------------|-------|----------------|--------|--------|
| `WalletId(seed_hash)` | `envelope.v1` | `[ STORED_SEED_ENVELOPE_VERSION (1 byte) \| bincode::serde(payload) ]` (no DetKv wrapper; leading version byte prepended by the storage layer) | `StoredSeedEnvelope` | `encrypted_seed: Vec<u8>`, `salt: Vec<u8>`, `nonce: Vec<u8>`, `password_hint: Option<String>`, `uses_password: bool`, `xpub_encoded: Vec<u8>` |

One entry per HD wallet. `seed_hash` is the 32-byte `WalletSeedHash` reused as the upstream `SecretWalletId`. The outer vault adds Argon2id + XChaCha20-Poly1305 at-rest encryption on top of DET's own AES-GCM per-wallet password layer.

Source: `src/wallet_backend/wallet_seed_store.rs` (`ENVELOPE_LABEL`), `src/model/wallet/seed_envelope.rs`

### Imported single-key private bytes

| Service (scope) | Label | Value encoding | Notes |
|-----------------|-------|----------------|-------|
| `SINGLE_KEY_NAMESPACE_ID` (fixed constant) | `single_key_priv.<base58_addr>` | 32 raw key bytes | One entry per imported WIF address |

`SINGLE_KEY_NAMESPACE_ID` is a fixed `[u8; 32]` (SHA-256 of `"det-single-key-namespace"`) shared by all imported keys — single-key entries are not per-HD-wallet. The label uses a dot separator because the upstream label allowlist (`^[A-Za-z0-9._-]{1,64}$`) rejects colons.

Source: `src/wallet_backend/single_key.rs` (`SINGLE_KEY_PRIV_LABEL_PREFIX`, `SINGLE_KEY_NAMESPACE_BYTES`)

---

## Summary counts

| Store | Key count |
|-------|-----------|
| `det-app.sqlite` | 4 (settings, wallet-meta sidecar, single-key-meta sidecar, migration sentinel) |
| `det-<net>.sqlite` | 23 (across 8 domains) |
| `SecretStore` | 2 label patterns (seed envelopes, imported-key private bytes) |
| **Total** | **29** |

Prefixed/templated keys (e.g. `det:identity:<id>`) are counted once per prefix, not per instance. `SecretStore` entries are counted as label-pattern families, not per-wallet instances.
