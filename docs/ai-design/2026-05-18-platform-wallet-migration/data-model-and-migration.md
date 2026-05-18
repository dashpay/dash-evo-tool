# Data Model and Migration

**Purpose:** Conversion table of DET types to `platform-wallet` targets, one-time migration procedure with backup/fail-safe, and dead fields to delete.

[← back to README](README.md)

---

Relates to: [phasing.md § P3](phasing.md#phase-table) — P3 implements the migration procedure; [g2-mock-boundary.md](g2-mock-boundary.md) — `PersistedWalletLoader` seam (seed re-registration); [dip14-migration-hardstop.md](dip14-migration-hardstop.md) — SUPERSEDED; migrate-or-quarantine apparatus WITHDRAWN (see "Accepted fund-accessibility trade-off" below).

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

**Stage B — async post-unlock one-shot** (`src/database/migration_pw.rs`), invoked from `AppContext::ensure_wallet_backend` (`src/context/mod.rs:634`, async, post-unlock — seed+SDK+persister+WalletBackend available). Gated by `platform_wallet_migration_pending`. Guarded by a `tokio::sync::Mutex` owned by `AppContext`, acquired BEFORE the marker check (exactly one Stage-B run process-wide under reentrant/concurrent `ensure_wallet_backend`). Strictly lazy: if user never unlocks, Stage B never runs, marker persists across unbounded launches, app fully usable in P2 `WalletBackendNotYetWired`-degraded state.

Stage-B steps (each idempotent; marker-gated; legacy DROP strictly last):
1. Backup precondition: `data.db.premigration` exists, or re-create idempotently.
2. Re-register every wallet via `SeedReregistrationLoader`/`create_wallet_from_seed_bytes` (no-op if registered).
3. `add_identity` each `QualifiedIdentity` blob (no-op if present; blob+platform-address+token tables RETAINED, upstream "Outside scope").
4. Re-establish DashPay contacts on upstream `derive_contact_xpub`/`derive_contact_payment_address(es)` ONLY — no DET re-derivation, no comparison, no classify. Upsert-keyed `(owner,contact)`. No quarantine path.
5. Finalize — **single fork**:
   - **SUCCESS:** durable flush → drop legacy wallet/utxo/spv/DashPay/contact tables → clear `platform_wallet_migration_pending` → `premigration` retired per policy.
   - **EXCEPTION** (crash/kill/power-loss/new-persister corruption/seed-decrypt failure): do NOT clear marker; do NOT drop legacy tables; next launch restore from `data.db.premigration` if new persister corrupt, then re-run from marker. Restore ONLY on exception, never otherwise.

**Simplified marker lifecycle:** Only `platform_wallet_migration_pending` is live. It clears ⇔ all wallets re-registered AND all identities added AND all contacts re-established upstream AND legacy tables dropped. `dashpay_dip14_quarantine_active` (column added in commit `6d348566`) is now INERT/RESERVED — removal is DEFERRED to P4's batched dead-column cleanup (do NOT add a P3 migration to drop it).

**Single-key wallets:** rows preserved untouched, flagged unsupported ([single-key-mock.md](single-key-mock.md)).

### Accepted fund-accessibility trade-off (user decision, 2026-05-18)

- **Affected:** DashPay contact-payment receive addresses derived via DET's legacy DIP-14 path (`src/backend_task/dashpay/dip14_derivation.rs:18,176`). Mainnet/account-0 addresses coincide with upstream (unaffected); testnet/devnet (coin-type `1'` vs DET `5'`) and non-account-0 contacts derive to different addresses under upstream.

- **Impact:** Funds received at non-reproduced legacy addresses (non-mainnet OR non-account-0 DashPay contacts) are NOT visible/spendable in this version. NOT destroyed — still derivable from seed via the old path; accessible by running the previous app version against the retained `data.db.premigration` backup. Mainnet+main-account DashPay and all non-DashPay funds are unaffected.

- **Withdrawn apparatus:** This exposure was previously handled by `dip14-migration-hardstop.md` §6.1–6.4. That apparatus is WITHDRAWN and the exposure ACCEPTED, with the mandatory one-time notice below as the sole compensating control.

- **Mandatory one-time informational notice** (info `MessageBanner`, shown once after successful Stage-B completion, gated by a new one-shot `settings.platform_wallet_migration_notice_shown`, dismissible, unconditional for EVERY migrated user since the affected subset can no longer be computed, jargon-free, technical detail to logs only). Exact user-facing text:

  > *"This update changes how DashPay contact payment addresses are calculated. Payments you received from DashPay contacts on Testnet or Devnet, or on a secondary account, may not appear in this version. Your funds are not lost — your previous data has been saved as a backup, and you can still access these payments using the previous version of the app. Mainnet payments on your main account are unaffected."*

- **Notice must be unconditional:** Removing the quarantine net removed the only per-user detector, so the notice MUST be unconditional for all migrated users and MUST NOT be downgraded to optional/conditional during implementation (A04 fail-safe → fail-informed; the notice is the sole compensating control).

### Dead Fields / Types → Deleted

After migration these become dead and are deleted in P4:

- Most of `Wallet` struct: `address_balances`, `utxos`, `transactions`, balance columns, `known_addresses`, `watched_addresses`
- `WalletSeedHash`-keyed reconcile maps
- `core_wallet_name` (RPC)
- `core_backend_mode`, `use_local_spv_node`, `auto_start_spv` settings
- `WalletTransaction` struct and DB table
- UTXO model and DB table (`database/utxo.rs`)
