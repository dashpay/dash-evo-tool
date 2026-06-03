# ADR — Finish data.db Unwire

**Date:** 2026-05-29
**Status:** Draft (PR not merged)
**Stacks on:** PR #860 (platform-wallet loose seam), PR #861 (deferred-domains unwire)
**Related:** `docs/ai-design/2026-05-28-migration-tool/notes.md`

---

## 1. Context

PR #860 unwired identities, tokens, contracts, DashPay overlays, and the shielded commitment
tree from `data.db`. PR #861 closed the DashPay deferral (D1–D4d). Remaining live paths:
`wallet`, `wallet_addresses`, `wallet_transactions`, `utxos`, `single_key_wallet`,
`shielded_notes`, and residual `settings` helpers.

Nagatha's audit identified six `wallet.rs` helpers and two `single_key_wallet.rs` helpers with
no live callers, and `initialization.rs` still creating removed-domain tables on fresh installs.
Diziet's UX doc (Phase 1a/1b) defined D-1..D-5 decisions and three user journeys (Alex / Priya
/ Jordan). This ADR records those decisions, the resulting architecture, and the floor SHAs the
migration-tool author needs.

---

## 2. Decisions

### D-1 — Migration banner UX

**Decision:** Non-blocking `Info` banner ("Updating storage… your wallet is safe") that
auto-dismisses on success. No modal blocker.

**Rationale:** Common cold-start is post-Stage-B, completing in under one second; a modal
is overhead. Silent mode (alt a) hides activity from Alex; a modal blocker (alt b) is
disproportionate for the common case.

**Implemented by:** T-MIG-02.

---

### D-2 — Single-key wallet fate

**Decision:** Keep via upstream `SecretStore` (file backend at `<data_dir>/secrets/`). Legacy
`single_key_wallet` rows migrate to labels `single_key_priv.<base58_addr>` per `WalletId`. UI
shows "Imported key — \<addr\>" with a distinct badge.

**Rationale:** Priya actively spends from cold-storage imported keys; silent removal leaks
user trust. `SecretStore` wraps key material with minimal surface area. Drop-with-sunset (alt a)
is future work when user population data is available; a DET-only sidecar (alt b) adds a
persistence layer `SecretStore` already replaces.

**Implemented by:** T-SK-01 + T-SK-02 + T-SK-03.

---

### D-3 — Shielded notes storage

**Decision:** Per-network sqlite sidecar at `<data_dir>/spv/<network>/det-shielded.sqlite`.
Lazy-created on first insert (zero-shielded users see no file). Schema mirrors legacy
`shielded_notes` 1:1.

**Rationale:** Stage-B and DashPay established the per-network sqlite sibling pattern
(`src/wallet_backend/shielded.rs`, `dashpay.rs`). Co-locating with grovedb's
`commitment_tree` under `spv/<network>/` keeps all shielded state in one directory and
simplifies rollback. Per-wallet k/v (alt a) splits shielded state across paths; waiting for
upstream grovedb parity (alt b) has no scheduled milestone.

**Implemented by:** T-SH-01 + T-SH-02 + T-SH-03.

---

### D-4 — `data.db` file fate after cutover

**Decision:** Leave dormant on disk, untouched. The separate migration-tool PR drains it later.

**Rationale:** Preserves NFR-1 and matches the migration-tool plan (see `notes.md`
§asset_lock_transaction). Rename-to-`data.db.legacy` (alt a) breaks the tool's path
assumption — defer until the tool ships. Delete (alt b) is irreversible; rejected.

**Reference:** `src/database/initialization.rs` — `legacy_detected()` gate added here.

---

### D-5 — Backward-compat reference SHAs

**Decision:** Record two floor SHAs. The migration-tool author reads legacy code from the
earlier SHA and the new wallet API from this PR's merge SHA.

- `35eb07bf67b48a74f14de2f1cd2a907412cc0b9a` — last commit with all DET `data.db` read/write
  paths intact (PR #860 pre-unwire tip). Use this to read every legacy code path in context.
- `b0fecacb` — PR #861 merge point; last commit with deferred-domain code before DashPay and
  shielded unwire landed.
- **[PLACEHOLDER — fill at merge time]** — replace this bullet with the squash-merge commit
  SHA of the finish-unwire PR once it lands on `v1.0-dev`. That SHA is the new floor for
  wallet-state paths. Do not leave this placeholder in place after merging: it will mislead
  the migration-tool author into thinking no post-Stage-B floor SHA exists.

**Rationale:** Two SHAs are the minimum to cover pre-Stage-B reads (legacy schema) and
post-Stage-B wallet-row migration. A single SHA (`35eb07bf` only, alt a) forces the tool
author to reconstruct post-Stage-B schema changes from history. A release tag (alt b) is
heavier than necessary.

---

## 3. Architectural Shifts

### Storage layers post-finish-unwire

```
+--- upstream (authoritative) -----------------------------------+
| PlatformWalletManager  wallet / utxo / tx / address           |
| SqlitePersister        per-network:  platform-wallet.sqlite    |
| SecretStore            per-data-dir: secrets/det-secrets.*    |
+--- DET sidecars ------------------------------------------------+
| det-app.sqlite         cross-network k/v (migration sentinel)  |
| spv/<net>/det-shielded.sqlite   per-network shielded notes     |
| spv/<net>/commitment_tree (grovedb-owned, data.db for now)     |
+--- dormant -----------------------------------------------------+
| data.db                legacy rows; no live DET readers        |
+----------------------------------------------------------------+
```

### Key structural changes

- `WalletBackend` gains `secret_store()` + `single_key() -> SingleKeyView` +
  `shielded(network) -> ShieldedView`. Same `DashpayView` adapter pattern.
- New `BackendTask::MigrationTask::FinishUnwire`; `MigrationStatus` enum (Idle/Running/
  Success/Failed) in `src/context/migration_status.rs`.
- `initialization.rs` gates `CREATE TABLE` for all six removed tables behind `legacy_detected()`.
- Completion sentinel key `det:migration:finish_unwire:v1` in `det-app.sqlite`.

---

## 4. Out of Scope (deferred to other PRs)

Per Nagatha §9: migration tool itself (separate PR); dropping single-key entirely (D-2 = keep;
revisit with user-population data); grovedb shielded-storage parity (blocked upstream);
`initialization.rs` tombstone CREATEs for pre-#860 tables (`proof_log`, `contestant`,
`contract`, …) — separate ADR; renaming `data.db.legacy` (D-4 = leave untouched; defer);
multi-account UTXO; OS-keychain SecretStore; encrypted-at-rest sidecar.

---

## 5. Test Coverage Reference

Full specification: `/tmp/marvin-finish-unwire-test-spec.md` (64 test cases).

| Domain | TC IDs | Count |
|--------|--------|-------|
| Wallet state (FR-1.x) | TC-W-001..010 | 10 |
| Single-key / SecretStore (FR-2.x) | TC-SK-001..010 | 10 |
| Shielded sidecar (FR-3.x) | TC-SH-001..009 | 9 |
| Migration mechanics + UX (FR-4/5.x) | TC-MIG-001..014 | 14 |
| Developer experience (FR-6.x) | TC-DEV-001..009 | 9 |
| Performance (NFR-5) | TC-PERF-001..004 | 4 |
| Accessibility (§2.3) | TC-A11Y-001..008 | 8 |

Known gaps (feature-flag-only or manual-only fixture size):

- **TC-SK-010** — D-2 alt drop path; requires non-default build flag.
- **TC-A11Y-008** — Focus-trap in modal export dialog; same D-2 alt build dependency.
- **TC-PERF-003** — 30 s worst-case migration with 10k-UTXO fixture; nightly or manual.

---

## 6. Migration Tool Inputs

For the migration-tool author.

**Legacy code:** check out `35eb07bf67b48a74f14de2f1cd2a907412cc0b9a` — all DET `data.db`
schemas and ORM helpers are intact there.

**Tables to drain:**

| Table(s) | Destination | Note |
|----------|-------------|------|
| `wallet` | upstream `wallet_metadata` per-network | Stage-B mirrors post-`e2f83466`; only pre-Stage-B rows need migration |
| `wallet_addresses` | upstream HD address cache | Re-derive from seed; verify before inserting |
| `wallet_transactions` | upstream tx store | Carry `block_hash`, `block_height`, `is_coinbase` |
| `utxos` | upstream `core_utxos` | Confirm against live SPV scan; stale rows must not override |
| `single_key_wallet` | `SecretStore` label `single_key_priv.<addr>` scoped to `SINGLE_KEY_NAMESPACE_ID` (fixed constant, shared across all imported keys) + `det-app.sqlite` sidecar `<network>:single_key_meta:<addr>` | See `src/wallet_backend/single_key.rs` |
| `shielded_notes`, `shielded_wallet_meta` | `spv/<network>/det-shielded.sqlite` | Schema mirror 1:1; cursor row in same sidecar |
| `asset_lock_transaction` | drop — rows are inert | Module deleted at `733f9e23` |
| `contact_private_info`, `dashpay_*` | upstream `ManagedIdentity` / DET DashPay k/v | Closed in PR #861; see `notes.md` §DashPay |

**Migration completion sentinel:** `det:migration:finish_unwire:v1` in `det-app.sqlite`
(via `AppContext::app_kv()`). Read before migrating to ensure idempotency.

**Shielded sidecar path:** `<data_dir>/spv/<network>/det-shielded.sqlite`
(`<network>` = `mainnet` | `testnet` | `devnet`).

---

## 7. KV Keyspace Reference (canonical)

All keyspaces active after this PR lands. For the complete reference with field-level detail see `docs/kv-keys.md`.

### `det-app.sqlite` — cross-network store

| Key pattern | Scope | Value type | Owner module |
|-------------|-------|------------|--------------|
| `det:settings:v1` | `None` | `AppSettings` | `src/context/settings_db.rs` |
| `<network>:wallet_meta:<seed_hash_b58>` | `None` | `WalletMeta` | `src/wallet_backend/wallet_meta.rs` |
| `<network>:single_key_meta:<addr>` | `None` | `ImportedKey` | `src/wallet_backend/single_key.rs` |
| `det:migration:finish_unwire:v1` | `None` | `MigrationCompletion` | `src/backend_task/migration/finish_unwire.rs` |

### `platform-wallet.sqlite` — per-network store (`<data_dir>/spv/<network>/`)

| Key pattern | Scope | Value type | Owner module |
|-------------|-------|------------|--------------|
| `det:selected_wallet:v1` | `None` | `SelectedWallet` | `src/wallet_backend/mod.rs` |
| `det:identity:<id_b58>` | `None` | `StoredQualifiedIdentity` | `src/context/identity_db.rs` |
| `det:identity_order:v1` | `None` | `Vec<[u8;32]>` | `src/context/identity_db.rs` |
| `det:top_ups:<id_b58>` | `None` | `BTreeMap<u32, u64>` | `src/context/identity_db.rs` |
| `det:scheduled_vote:<voter_b58>:<contested_name>` | `None` | `StoredScheduledVote` | `src/context/identity_db.rs` |
| `det:contested_name:<norm_name>` | `None` | `StoredContestedName` | `src/context/contested_names_db.rs` |
| `det:contract:<contract_id_b58>` | `None` | `StoredContract` | `src/context/contract_token_db.rs` |
| `det:token:<token_id_b58>` | `None` | `StoredToken` | `src/context/contract_token_db.rs` |
| `det:token_balance:<id_b58>:<token_id_b58>` | `None` | `u64` | `src/context/contract_token_db.rs` |
| `det:token_order:v1` | `None` | `Vec<([u8;32],[u8;32])>` | `src/context/contract_token_db.rs` |
| `det:platform_addr:<addr>` | `Some(&wallet_seed_hash)` | `StoredPlatformAddressInfo` | `src/context/platform_address_db.rs` |
| `det:platform_sync:v1` | `Some(&wallet_seed_hash)` | `StoredPlatformSyncInfo` | `src/context/platform_address_db.rs` |
| `det:dashpay:blocked:<contact_b58>` | `None` | `()` | `src/wallet_backend/dashpay.rs` |
| `det:dashpay:rejected:<counterparty_b58>` | `None` | `()` | `src/wallet_backend/dashpay.rs` |
| `det:dashpay:timestamps:<entity_b58>` | `None` | `(i64, i64)` | `src/wallet_backend/dashpay.rs` |
| `det:dashpay:private:<owner_b58>:<contact_b58>` | `None` | `ContactPrivateInfo` | `src/wallet_backend/dashpay.rs` |
| `det:dashpay:address_index:<owner_b58>:<contact_b58>` | `None` | `ContactAddressIndex` | `src/wallet_backend/dashpay.rs` |
| `det:dashpay:addr_map:<owner_b58>:<address>` | `None` | `([u8;32], u32)` | `src/wallet_backend/dashpay.rs` |

### `SecretStore` — encrypted file vault (`<data_dir>/secrets/det-secrets.*`)

Not a `KvStore`; values are raw encrypted bytes, not the `DetKv` version-byte envelope.

| Scope (`WalletId`) | Label pattern | Value | Owner module |
|--------------------|---------------|-------|--------------|
| `WalletId(seed_hash)` | `envelope.v1` | bincode(`StoredSeedEnvelope`) | `src/wallet_backend/wallet_seed_store.rs` |
| `SINGLE_KEY_NAMESPACE_ID` (fixed) | `single_key_priv.<base58_addr>` | 32 raw key bytes | `src/wallet_backend/single_key.rs` |

---

## 8. Change Log

```
2026-05-29 — Initial ADR. Companion to T-* commits:
             e761cb7c (T-SK-01), fefba738 (T-SH-01),
             c32918a8 (T-MIG-01), 68ec561e (T-DEV-01),
             465a0684 (T-SK-02), a546c379 (T-SH-02).
             [Further commits appended on merge.]
```
