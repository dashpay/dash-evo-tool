# DET k/v key reference

`DetKv` wraps the upstream `platform_wallet_storage::KvStore`. Values are encoded as `[ schema_version (1 byte) | bincode(payload) ]` using `bincode::config::standard()`. Keys are colon-separated namespaces. Every `DetKv` call takes an `Option<&WalletId>` scope: `None` = global slot, `Some(&id)` = per-wallet slot (cascades on wallet delete).

Two backing stores exist:

| Store | Path | Contents |
|-------|------|----------|
| `det-app.sqlite` | `<data_dir>/det-app.sqlite` | Cross-network settings |
| `platform-wallet.sqlite` | `<data_dir>/spv/<net>/platform-wallet.sqlite` | Everything else (per-network) |

---

## Settings

| Key | Scope | Store | Value type | Fields |
|-----|-------|-------|------------|--------|
| `det:settings:v1` | `None` | `det-app.sqlite` | `AppSettings` via `AppSettingsWire` | `network`, `root_screen_type`, `dash_qt_path`, `overwrite_dash_conf`, `disable_zmq`, `theme_mode`, `core_backend_mode`, `onboarding_completed`, `show_evonode_tools`, `user_mode`, `close_dash_qt_on_exit`, `auto_start_spv` |

Source: `src/model/settings.rs`, `src/context/settings_db.rs`

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

Both keys use **per-wallet scope** (`Some(&seed_hash)`) so entries cascade on wallet removal.

| Key | Scope | Store | Value type | Notes |
|-----|-------|-------|------------|-------|
| `det:platform_addr:<canonical_address>` | `Some(&wallet_seed_hash)` | `platform-wallet.sqlite` | `StoredPlatformAddressInfo` | Fields: `balance: u64`, `nonce: u32` |
| `det:platform_sync:v1` | `Some(&wallet_seed_hash)` | `platform-wallet.sqlite` | `StoredPlatformSyncInfo` | Fields: `last_sync_timestamp: u64`, `sync_height: u64` |

Source: `src/context/platform_address_db.rs`

---

## DashPay sidecar

All sidecar keys use **global scope** (`None`). The per-network `platform-wallet.sqlite` already partitions by network, so no `<network>:` prefix is needed within the key.

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

## Summary counts

| Store | Key count |
|-------|-----------|
| `det-app.sqlite` | 1 |
| `platform-wallet.sqlite` | 17 (across 8 domains) |
| **Total** | **18** |

Prefixed/templated keys (e.g. `det:identity:<id>`) are counted once per prefix, not per instance.
