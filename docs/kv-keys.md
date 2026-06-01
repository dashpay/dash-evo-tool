# DET k/v key reference

`DetKv` wraps the upstream `platform_wallet_storage::KvStore`. Values are encoded as `[ schema_version (1 byte) | bincode(payload) ]` using `bincode::config::standard()`. Keys are colon-separated namespaces. Every `DetKv` call takes a `DetScope` argument: `DetScope::Global` = global slot, `DetScope::Wallet(&seed_hash)` = per-wallet slot (cascades on wallet delete). `DetScope::Identity` / `DetScope::Token` are reserved for the Wave 2 scope promotions and not yet written. `DetScope` is the DET-side seam over the upstream `ObjectId` enum — the upstream scope type never crosses the wallet-backend boundary.

Three backing stores exist:

| Store | Path | Contents |
|-------|------|----------|
| `det-app.sqlite` | `<data_dir>/det-app.sqlite` | Cross-network settings, wallet metadata, migration sentinel, single-key metadata |
| `platform-wallet.sqlite` | `<data_dir>/spv/<net>/platform-wallet.sqlite` | Per-network identities, tokens, contracts, DashPay overlays, platform addresses, selected wallet |
| `SecretStore` | `<data_dir>/secrets/det-secrets.*` | Encrypted HD-wallet seed envelopes and imported single-key private bytes |

In the per-domain tables below, a `Scope` of `None` denotes `DetScope::Global` and `Some(&seed_hash)` denotes `DetScope::Wallet(&seed_hash)`.

---

## DEV: on-disk schema break on this branch — reset local wallet DBs

On the active development branch the upstream `platform-wallet-storage` pin moved to a layout that changed the on-disk schema (the old `kv_store` table was replaced by the `meta_*` tables) with a **divergent `V001` migration**. A `platform-wallet.sqlite` written by an earlier pin will not open: refinery aborts on the divergent checksum, the open surfaces `WalletStorageError::Migration`, and DET maps it to the `WalletDataIncompatible` error (banner: *"Your wallet data is not compatible with this version of the app and cannot be opened. Remove the local wallet data so the app can create it fresh, then restart."*).

This is expected during development. To continue, **delete both local DET wallet databases** and let the app recreate them:

- `<data_dir>/spv/<net>/platform-wallet.sqlite` (per-network persister)
- `<data_dir>/det-app.sqlite` (cross-network settings / sidecars / migration sentinel)

`<data_dir>` is the per-OS app directory (Linux `~/.config/dash-evo-tool/`, macOS `~/Library/Application Support/Dash-Evo-Tool/`, Windows `%APPDATA%\Dash-Evo-Tool\config\`). Wallet seeds are recoverable from your recovery phrase; on-chain state re-syncs.

Cross-links: [migration data model](ai-design/2026-05-18-platform-wallet-migration/data-model-and-migration.md), platform todos `f5897abd` (per-token balance reader) and the `08b0ed9` storage-schema bump.

---

## Settings

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `det:settings:v1` | `None` | `det-app.sqlite` | `AppSettings` via `AppSettingsWire` | `network`, `root_screen_type`, `dash_qt_path`, `overwrite_dash_conf`, `disable_zmq`, `theme_mode`, `core_backend_mode`, `onboarding_completed`, `show_evonode_tools`, `user_mode`, `close_dash_qt_on_exit`, `auto_start_spv` |

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
| `det:selected_wallet:v1` | `None` | `platform-wallet.sqlite` | `SelectedWallet` | `hd_wallet_hash: Option<[u8;32]>`, `single_key_hash: Option<[u8;32]>` |

Source: `src/model/selected_wallet.rs`, `src/wallet_backend/mod.rs`

---

## Identities

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:identity:<base58_identity_id>` | `None` | `platform-wallet.sqlite` | `StoredQualifiedIdentity` | Fields: `qi_bytes` (inner bincode), `status: u8`, `identity_type: String`, `wallet_hash: Option<[u8;32]>`, `wallet_index: Option<u32>` |
| `det:identity_order:v1` | `None` | `platform-wallet.sqlite` | `Vec<[u8;32]>` | Ordered list of identity ID raw bytes |
| `det:top_ups:<base58_identity_id>` | `None` | `platform-wallet.sqlite` | `BTreeMap<u32, u64>` | Top-up history: account index → credits |

Source: `src/context/identity_db.rs`

---

## Scheduled votes

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:scheduled_vote:<base58_voter_id>:<contested_name>` | `None` | `platform-wallet.sqlite` | `StoredScheduledVote` | Fields: `voter_id: [u8;32]`, `contested_name: String`, `choice: StoredVoteChoice`, `unix_timestamp: u64`, `executed_successfully: bool` |

Source: `src/context/identity_db.rs`

---

## Contested names (DPNS)

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:contested_name:<normalized_name>` | `None` | `platform-wallet.sqlite` | `StoredContestedName` | Fields: `normalized_contested_name`, `locked_votes`, `abstain_votes`, `awarded_to`, `end_time`, `locked`, `last_updated`, `contestants: Vec<StoredContestant>` |

`StoredContestant` fields: `id: [u8;32]`, `name`, `info`, `votes: u32`, `created_at`, `created_at_block_height`, `created_at_core_block_height`, `document_id: [u8;32]`.

Source: `src/context/contested_names_db.rs`

---

## Contracts

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:contract:<base58_contract_id>` | `None` | `platform-wallet.sqlite` | `StoredContract` | Fields: `contract_bytes: Vec<u8>` (platform-serialized), `alias: Option<String>` |

Source: `src/context/contract_token_db.rs`

---

## Tokens

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:token:<base58_token_id>` | `None` | `platform-wallet.sqlite` | `StoredToken` | Fields: `config_bytes: Vec<u8>` (bincode `TokenConfiguration`), `alias: String`, `data_contract_id: [u8;32]`, `position: u16` |
| `det:token_balance:<base58_identity_id>:<base58_token_id>` | `None` | `platform-wallet.sqlite` | `u64` | Raw balance in token base units |
| `det:token_order:v1` | `None` | `platform-wallet.sqlite` | `Vec<([u8;32],[u8;32])>` | Ordered `(token_id, identity_id)` pairs for My Tokens screen |

Source: `src/context/contract_token_db.rs`

---

## Platform addresses

Both keys use **per-wallet scope** (`DetScope::Wallet(&seed_hash)`) so entries cascade on wallet removal. Reads/writes route through the `PlatformAddressView` seam (`src/wallet_backend/platform_address.rs`); the cache stays active because upstream's public per-address reader exposes balance but not the DET-tracked nonce.

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:platform_addr:<canonical_address>` | `DetScope::Wallet(&seed_hash)` | `platform-wallet.sqlite` | `StoredPlatformAddressInfo` | Fields: `balance: u64`, `nonce: u32` |
| `det:platform_sync:v1` | `DetScope::Wallet(&seed_hash)` | `platform-wallet.sqlite` | `StoredPlatformSyncInfo` | Fields: `last_sync_timestamp: u64`, `sync_height: u64` |

Source: `src/context/platform_address_db.rs`, `src/wallet_backend/platform_address.rs`

---

## DashPay sidecar

All sidecar keys use **global scope** (`DetScope::Global`). The per-network `platform-wallet.sqlite` already partitions by network, so no `<network>:` prefix is needed within the key.

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:dashpay:blocked:<base58_contact_id>` | `None` | `platform-wallet.sqlite` | `()` | Presence-only flag: contact is blocked |
| `det:dashpay:rejected:<base58_counterparty_id>` | `None` | `platform-wallet.sqlite` | `()` | Presence-only flag: contact request rejected |
| `det:dashpay:timestamps:<base58_entity_id>` | `None` | `platform-wallet.sqlite` | `(i64, i64)` | DET-local `(created_at_ms, updated_at_ms)` |
| `det:dashpay:private:<base58_owner>:<base58_contact>` | `None` | `platform-wallet.sqlite` | `ContactPrivateInfo` | Fields: `nickname: String`, `notes: String`, `is_hidden: bool` |
| `det:dashpay:address_index:<base58_owner>:<base58_contact>` | `None` | `platform-wallet.sqlite` | `ContactAddressIndex` | Fields: `owner_identity_id: Vec<u8>`, `contact_identity_id: Vec<u8>`, `next_send_index: u32`, `highest_receive_index: u32`, `bloom_registered_count: u32` |
| `det:dashpay:addr_map:<base58_owner>:<address>` | `None` | `platform-wallet.sqlite` | `([u8;32], u32)` | Reverse map: wallet address → `(contact_id_bytes, index)` |

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
| `platform-wallet.sqlite` | 17 (across 8 domains) |
| `SecretStore` | 2 label patterns (seed envelopes, imported-key private bytes) |
| **Total** | **23** |

Prefixed/templated keys (e.g. `det:identity:<id>`) are counted once per prefix, not per instance. `SecretStore` entries are counted as label-pattern families, not per-wallet instances.
