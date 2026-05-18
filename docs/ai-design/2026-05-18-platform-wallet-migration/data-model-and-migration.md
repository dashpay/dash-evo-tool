# Data Model and Migration

**Purpose:** Conversion table of DET types to `platform-wallet` targets, one-time migration procedure with backup/fail-safe, and dead fields to delete.

[← back to README](README.md)

---

Relates to: [phasing.md § P3](phasing.md#phase-table) — P3 implements the migration procedure; [open-questions.md #2](open-questions.md) — G2 seed-re-registration UX.

## D. Data Model and Conversions

### One-Time Migration

Runs on first launch post-upgrade. Idempotent and fail-safe (A04). See procedure below.

### Conversion Surface

| DET type | `platform-wallet` target | Direction | Lossy / dropped |
|---|---|---|---|
| `model::wallet::Wallet` (`src/model/wallet/mod.rs:342`) + encrypted seed | Re-register via `create_wallet_from_seed_bytes(network, seed_bytes, WalletAccountCreationOptions)` | DET → upstream (seed-driven, per G2) | `Wallet.{address_balances, utxos, transactions, confirmed/unconfirmed/total_balance, known/watched_addresses}` all dropped — upstream re-derives and re-syncs. Only seed + alias + `is_main` migrate. |
| `model::wallet::WalletSeed`/`ClosedKeyItem` (encrypted seed, salt, nonce, password_hint) | DET-retained seed store — NOT into persister (secret boundary, `SECRETS.md`) | Stays DET | None |
| `QualifiedIdentity` bincode blob (`src/database/identities.rs:157`) | Retained as-is (upstream "Outside scope") + seed `IdentityManager` via `add_identity` | DET → upstream | DET keeps blob; upstream `ManagedIdentity` sync-metadata starts empty, repopulated on first sync |
| `WalletTransaction` (`mod.rs:581`) | Upstream `TransactionRecord` (re-synced) | Dropped | Transaction history re-synced from chain |
| UTXO / balance rows (`database/utxo.rs`, wallet balance cols) | Upstream UTXO set (re-synced) | Dropped | Re-derived |
| DashPay contacts / profile (`database/dashpay.rs`, `database/contacts.rs`) | Upstream `EstablishedContact` / `ContactRequest` / `DashPayProfile` via `add_*` | DET → upstream | Established-contact derivation re-derived from seed + identities; DET payment-history / avatar cache retained DET-side |
| Platform addresses, token balances | DET-retained tables (upstream "Outside scope") | Stays DET | None |
| `SingleKeyWallet` (`model/wallet/single_key.rs`) | No target (see [single-key-mock.md](single-key-mock.md)) | Stays DET, stubbed | Preserved in legacy table, not migrated, surfaced as unsupported |
| Settings (`database/settings.rs`) incl. `core_backend_mode` | DET settings minus `core_backend_mode` / `use_local_spv_node` / `auto_start_spv` | Stays DET | Those columns dead (RPC mode gone) — drop in migration |

### Migration Procedure

Runs on first launch post-upgrade. Steps are idempotent; the procedure is fail-safe (A04).

**Step 1.** Detect schema-version marker `< new_version`.

**Step 2.** Back up the legacy DB file before any destructive step: copy `*.db → *.db.premigration`.

**Step 3.** For each legacy `Wallet`: decrypt seed (existing DET path), call `create_wallet_from_seed_bytes`; `load_persisted()` rehydrates identity/contact/address deltas from the upstream persister (fresh DB, empty first time — repopulated by first sync).

**Step 4.** For each `QualifiedIdentity` blob: keep the DET identity table; call `add_identity` into upstream `IdentityManager` so it is sync-tracked.

**Step 5.** Migrate DashPay established contacts and profile into upstream via `add_*`.

**Step 6.** On full success: drop legacy tables `wallet`, `utxos`, `wallet_transactions`, SPV-derived rows, and the dead settings columns. Keep retained tables: identity blob, platform addresses, tokens, `single_key_wallet`, settings (minus dead cols), contested votes, shielded, contacts payment cache.

**Step 7.** On any failure: do not drop legacy tables; restore from `*.db.premigration`; surface a calm, actionable banner:

> "Wallet upgrade could not complete. Your data is safe. Please restart; if it recurs, your previous data remains intact."

No jargon, no "contact support" (CLAUDE.md error-message rules).

**Step 8.** Single-key wallets: rows preserved untouched, flagged unsupported ([single-key-mock.md](single-key-mock.md)).

### Dead Fields / Types → Deleted

After migration these become dead and are deleted in P4:

- Most of `Wallet` struct: `address_balances`, `utxos`, `transactions`, balance columns, `known_addresses`, `watched_addresses`
- `WalletSeedHash`-keyed reconcile maps
- `core_wallet_name` (RPC)
- `core_backend_mode`, `use_local_spv_node`, `auto_start_spv` settings
- `WalletTransaction` struct and DB table
- UTXO model and DB table (`database/utxo.rs`)
