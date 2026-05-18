# Data Model and Migration

**Purpose:** Conversion table of DET types to `platform-wallet` targets, one-time migration procedure with backup/fail-safe, and dead fields to delete.

[← back to README](README.md)

---

Relates to: [phasing.md § P3](phasing.md#phase-table) — P3 implements the migration procedure; [g2-mock-boundary.md](g2-mock-boundary.md) — `PersistedWalletLoader` seam (seed re-registration); [dip14-migration-hardstop.md](dip14-migration-hardstop.md) — per-contact DashPay derivation migration and quarantine.

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

### Migration execution model — two stage, marker-gated (RATIFIED)

The DET DB-migration framework (`src/database/initialization.rs::initialize` → `try_perform_migration` → `apply_version_changes(version, tx:&Connection,…)` `:121,:350,:386`) is synchronous, SQL-only, single-rusqlite-transaction, runs at DB-init BEFORE wallet unlock. The platform-wallet migration needs unlocked seed + async + WalletBackend — none available there. Hence two stages:

**Stage A — SQL migration v35** (`apply_version_changes` arm `35`; `DEFAULT_DB_VERSION` `34`→`35` `:38`). Sync, in-tx, idempotent via version bump. Actions: (1) set persistent marker `settings.platform_wallet_migration_pending=1` inside the v35 tx; (2) NO destructive step. The retained `data.db.premigration` backup is created POST-commit (NOT inside the live write-tx — use SQLite online-backup API or guarded post-`commit()` copy keyed off the marker), distinct from rolling `backups/data_backup_*.db` (`backup_db` `:463`). The MARKER (not the backup file's existence) is the authoritative "pending" signal; retained backup is (re)created idempotently on first post-marker launch even if the user never unlocks.

**Stage B — async post-unlock one-shot** (`src/database/migration_pw.rs`), invoked from `AppContext::ensure_wallet_backend` (`src/context/mod.rs:634`, async, post-unlock — seed+SDK+persister+WalletBackend available). Gated by `platform_wallet_migration_pending`. Guarded by a `tokio::sync::Mutex` owned by `AppContext`, acquired BEFORE the marker check (exactly one Stage-B run process-wide under reentrant/concurrent `ensure_wallet_backend`). Strictly lazy: if user never unlocks, Stage B never runs, marker persists across unbounded launches, app fully usable in P2 `WalletBackendNotYetWired`-degraded state. Multi-wallet partial completion legal — marker clears only when every wallet migrated-or-classified.

Stage-B steps (each idempotent; marker-gated; legacy DROP strictly last):
1. Re-register every wallet via `SeedReregistrationLoader`/`create_wallet_from_seed_bytes` (no-op if registered).
2. `add_identity` each `QualifiedIdentity` blob (no-op if present; blob+platform-address+token tables RETAINED, upstream "Outside scope").
3. Migrate DashPay profile+established contacts (upsert-keyed `(owner,contact)`).
4. DIP-14/15 migrate-or-quarantine per [dip14-migration-hardstop.md](dip14-migration-hardstop.md) §6.1–6.4 (authoritative for predicate) — historical index range = UNION of receive `[0,highest_receive_index]` and send `[0,next_send_index−1]` (saturating), both from `Database::get_contact_address_indices` (`src/database/dashpay.rs:649`), no sampled prefix.
5. Conditional finalize:

   - **All migrated, none quarantined:** durably flush persister → drop legacy wallet/utxo/spv/DashPay/contact tables → clear `platform_wallet_migration_pending` → `data.db.premigration` retirable.

   - **≥1 quarantined:** quarantine is a SUCCESSFUL TERMINAL CLASSIFICATION, NOT a failure. Clear `platform_wallet_migration_pending` (nothing left to attempt). Set `settings.dashpay_dip14_quarantine_active=1`. RETAIN legacy DashPay/contact tables. RETAIN `data.db.premigration` while quarantine flag set. Blocking calm Base58-identified banner per §6.4. Non-DashPay wallet function proceeds.

   - **Stage-B exception** (crash/kill/power-loss/irreconcilable-non-quarantinable/new-persister corruption): do NOT clear marker; do NOT drop legacy tables; next launch restore from `data.db.premigration` if new persister corrupt, then re-run Stage B from marker (idempotent). Restore-from-premigration occurs ONLY on this exceptional path — NEVER because contacts quarantined.

**Marker lifecycle (normative):** `platform_wallet_migration_pending` cleared ⇔ every wallet re-registered AND every identity added AND every contact classified (migrated XOR quarantined). `dashpay_dip14_quarantine_active` independent; set iff ≥1 quarantined; gates legacy-DashPay-table + `data.db.premigration` retention. Both clear ⇒ `data.db.premigration` retirable.

**Single-key wallets:** rows preserved untouched, flagged unsupported ([single-key-mock.md](single-key-mock.md)).

### Dead Fields / Types → Deleted

After migration these become dead and are deleted in P4:

- Most of `Wallet` struct: `address_balances`, `utxos`, `transactions`, balance columns, `known_addresses`, `watched_addresses`
- `WalletSeedHash`-keyed reconcile maps
- `core_wallet_name` (RPC)
- `core_backend_mode`, `use_local_spv_node`, `auto_start_spv` settings
- `WalletTransaction` struct and DB table
- UTXO model and DB table (`database/utxo.rs`)
