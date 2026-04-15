# Persister Unification — Complete Refactor Plan

**Status**: draft, pending alignment
**Owner**: TBD
**Created**: 2026-04-15
**Supersedes**: piecemeal fixes in `feat/platform-wallet2` (commits `fd906491`..`af9ce3d6`)

## Problem statement

Wallet and identity state is written to SQLite through **two parallel paths** that cannot see each other:

1. **Persister** (`SqliteWalletPersister` in `src/changeset/sqlite.rs`) — owns a scoped subset: core wallet state, contacts, asset locks, identity metadata (minus the raw blob), dashpay profiles, dashpay payments.
2. **Direct `Database::*` calls** from backend tasks — own the rest: `identity.data` blob (bincoded `QualifiedIdentity`), platform address balances, token balances, identity removals, primary-identity tracking, last-scanned-index, shielded notes, payments.

The split was created during the incremental rollout of the persister; several fields live on the `PlatformWalletChangeSet` struct but get **silently dropped at `tracing::debug!` level** by `flush_inner` because the persister hasn't grown ownership of them yet.

**Consequences observable today:**

- Silent data drops at debug level (invisible in production log levels).
- UI reads bypass the in-memory model and query SQL directly in at least 7 places (`contacts_list`, `contact_requests`, `contact_details`, `send_payment` x2, `contact_profile_viewer`, `incoming_payments`, `dashpay.rs`), creating a two-clock problem between in-memory state and SQL.
- No single source of truth — the persister's `load()` rebuilds from tables that other code paths wrote, so drift between the two paths shows up as ghost state.
- Three historical bug fixes this cycle (`977f1ff4`, `b6959419`, `4eeaa6a8`) were caused by confusion between which path should write what.
- Some direct `Database::*` writes discard errors entirely (`let _ = db.insert_shielded_note(…)` → shielded-fund-loss-on-disk-error).

This document lays out the full accumulated research, the target architecture, and a staged execution plan.

---

## Prior research — complete dump

### Five-agent review of `feat/platform-wallet2`

Five specialised agents reviewed the pre-rebase diff in parallel. Summary of findings:

#### Agent 1 — code-reviewer

**Critical:**
- **C1**: `tests/backend-e2e/*` fails to compile after `WalletSeedHash` alias deletion (later fixed by the colleague in a follow-up commit on-branch).
- **C2**: `network_str.parse().unwrap_or(Testnet)` in `ensure_wallet_id_column_and_backfill` — silent wrong-network wallet_id derivation. **FIXED** in `ca572ab1` (SF-C6).

**High:**
- **H1**: ~40 sites with stale `v40`/`v41`/`Phase 9b` doc comments. **FIXED** in `af9ce3d6` (CR-H1).
- **H2**: 8 mislabeled `seed = %hex(wallet_id)` log fields. **FIXED** in `af9ce3d6` (CR-H2).
- **H3**: Orphaned TODO + dead `Account` construction in `bootstrap_dashpay_contact_accounts` (line 1245). **OPEN**.
- **H4**: `write_contact_requests` doc comment lies about `established` handling. **OPEN**.
- **H5**: `write_identity_dashpay_subset` stale phase references. **OPEN** (some cleaned).
- **H6**: `WalletMigrationScreen` typed-error and i18n violations. **FIXED** in `af9ce3d6` (CR-H6/TD-5/TD-8).

**Medium:**
- **M1**: `try_unlock` returned `Result<(), String>`. **FIXED**.
- **M2**: Empty-block code in row rendering (dead conditional branch). **OPEN**.
- **M3**: Stale dead variables `islock_data`, `chain_height` in asset lock load. **OPEN**.
- **M5**: Persister flush re-merge LWW comment direction ambiguous. **OPEN**.

#### Agent 2 — silent-failure-hunter

**Critical — introduced by `feat/platform-wallet2`:**
- **SF-C3**: `store()` swallowed flush errors. **FIXED** in `ca572ab1`.
- **SF-C5**: Backfill silently skipped malformed-seed wallets + locked-wallet query didn't filter `uses_password=1`. **FIXED** in `ca572ab1`.
- **SF-C6**: Strict network parse with `"dash"` alias support. **FIXED** in `ca572ab1`.
- **SF-C7**: `set_wallet_id_for_locked_wallet` rows-affected check + `try_unlock` zero-bytes rejection. **FIXED** in `ca572ab1`.

**Critical — pre-existing on `v1.0-dev`:**
- **SF-C1**: `let _ = db.insert_shielded_note(...)` at `backend_task/shielded/sync.rs:149`. Note is added to in-memory `shielded_state.notes` but never persisted — **shielded fund loss** on next restart if the write failed.
- **SF-C2**: `let _ = db.save_payment(...)` at `ui/dashpay/send_payment.rs:843` + `save_payment` doesn't write `wallet_id`. Even successful writes are orphaned by persister `load()` filter.
- **SF-C4**: `get_wallet_identity_indices` returns empty set on error at `database/identities.rs:324-329`. Identity index collisions possible.

**High:**
- **SF-H1**: `[0u8; 32]` sentinel wallet_id still silently filters and deletes at `context/mod.rs:330-340`. **OPEN**.
- **SF-H2**: `contact_requests.platform_created_at_ms` NULL coerces to 0 silently at `changeset/sqlite.rs:627-631`. **OPEN**.
- **SF-H3**: `register_with_platform_wallet_manager` silently skips on lock contention. **OPEN**.
- **SF-H4**: `replay_persisted_state_after_bootstrap` silently returns on lock failure. **OPEN**.
- **SF-H5**: `bootstrap_dashpay_contact_accounts` skips with `debug!` only — DashPay incoming payments go undetected. **OPEN**.

**Medium:**
- **M1**: `get_shielded_balance` returns 0 on any read error via `unwrap_or(0)` at `database/shielded.rs:293-301`. **OPEN**.
- **M2**: `update_onboarding_completed` silently discarded at `ui/welcome_screen.rs:180`. **OPEN**.
- **M3**: `save_dashpay_profile_avatar_bytes` silently discarded at `ui/dashpay/profile_screen.rs:1437`. **OPEN**.
- **M4**: `get_identity_alias` uses `.ok()` — masks corruption. **OPEN**.
- **M5**: `save_dashpay_profile` / `save_payment` public writers don't write `wallet_id`. Active UI call sites: `profile_screen.rs:1462, 1486, 1525`, `contact_details.rs:602`, `contact_requests.rs:458`. **OPEN**.

#### Agent 3 — pr-test-analyzer

**Critical:**
- **TC-C1**: Locked-wallet guard never tested. **FIXED** in `af9ce3d6`.
- **TC-C2**: Persister `flush()` failure-path backup unverified. **FIXED** in `af9ce3d6`.
- **TC-C3**: `WalletMigrationScreen` DB-error path untested. **FIXED** in `af9ce3d6`.

**High:**
- **TC-H1**: `EstablishedContact` load-side reconstruction not asserted (bidirectional pairing in persister load). **OPEN**.
- **TC-H2**: `bootstrap_dashpay_contact_accounts` zero tests. **OPEN**.
- **TC-H3**: `sync_identity_to_platform_wallet` after contact-reload removal not tested. **OPEN**.
- **TC-H4**: v33→v34 idempotency when `wallet_id` column already exists untested. **OPEN**.
- **TC-H5**: `derive_wallet_id_from_seed` has no known-vector pin. **OPEN**.

**Medium:**
- **TC-M1**: Mutex poisoning recovery on persister untested. **OPEN**.
- **TC-M2**: Asset lock states beyond `Broadcast` untested. **OPEN**.
- **TC-M3**: v33→v34 with malformed seed + locked-wallet guard interaction untested. **OPEN**.
- **TC-M4**: Wallet_id collision detection untested. **OPEN**.
- **TC-M5**: DashPay contact crypto round-trip both directions untested. **OPEN**.

#### Agent 4 — type-design-analyzer

**Ordered by impact, all OPEN except #5 #6 #8:**

1. Promote `WalletId` from `pub type` alias to newtype struct (~50 files). Tracked as task #142. **DEFERRED**.
2. Introduce `WalletSeedHash` newtype distinct from `WalletId`. User decided against this during P1 planning (seed_hash is legacy-only). **CLOSED — not doing**.
3. `MigrationError.details: String` sentinel → typed enum. **OPEN**.
4. `SqlitePersistError::Encode(String)` → `Encode(#[source] bincode::EncodeError)`. **OPEN**.
5. `WalletMigrationEntry::try_unlock` typed error. **FIXED** in `af9ce3d6`.
6. `DashPayProfile` avatar fields `Option<Vec<u8>>` → `Option<[u8; N]>`. **FIXED** during the rebase with the colleague's commit.
7. `PlatformWalletPersistence::Error` associated type instead of `Box<dyn Error>`. **OPEN**.
8. Kill the `wallet_id: [0u8; 32]` dummy in `WalletMigrationEntry::try_unlock`. **FIXED** in `af9ce3d6`.

#### Agent 5 — architecture-strategist

**Top 3 concerns:**

- **ARCH-1**: `*self = new_state` swap in `finish_post_migration_init` is a maintenance bomb. Proposed `AppLifecycle` enum at `eframe::App` level. Tracked as task #141 with inline `TODO(ARCH-1, task #141)` planted at the swap site. **DEFERRED** (user skipped, wants to revisit later).

- **ARCH-2**: 7 UI/backend sites read `db.load_dashpay_contacts` directly. Full list:
  - `src/ui/dashpay/contact_requests.rs:292`
  - `src/ui/dashpay/contacts_list.rs:124`
  - `src/ui/dashpay/contact_details.rs:102`
  - `src/ui/dashpay/send_payment.rs:89`
  - `src/ui/dashpay/send_payment.rs:541`
  - `src/ui/dashpay/contact_profile_viewer.rs:77`
  - `src/backend_task/dashpay/incoming_payments.rs:107`
  - `src/backend_task/dashpay.rs:184`
  These bypass the in-memory `ManagedIdentity.established_contacts` populated by the persister. **OPEN**.

- **ARCH-3**: No CI guard against silent persister sub-changeset drops. The `tracing::debug!` drop logs in `flush_inner` are documentation, not enforcement. **OPEN** — and this is the topic of this document.

**Top 3 strengths (do not regress):**
- Phase-1 column-add committed before the main migration transaction (resumable after crash).
- Persister's atomicity contract: one `rusqlite::Transaction` per `flush_inner`.
- `IdentityEntry` full-snapshot DELETE-on-None semantics for dashpay profile clearing.

---

## Current mess inventory

### Silently dropped sub-changeset fields in `flush_inner`

Measured in current code at `src/changeset/sqlite.rs:940-1070`. Each drop logs at `tracing::debug!`.

| # | Field | Reason comment gives | Actual category | Production emitters |
|---|---|---|---|---|
| 1 | `PlatformAddressChangeSet` | "backend tasks own platform address persistence" | **Deliberate scope boundary** — platform balances come from RPC queries. | Need to grep. |
| 2 | `TokenBalanceChangeSet` | "backend tasks own token balance persistence" | **Deliberate scope boundary** — token balances are backend-task-queried. | Need to grep. |
| 3 | `dashpay_profiles` overlay | "load-only, written via identity subset" | **Structural** — the field exists on the struct only so `load()` can return profile data alongside the core changeset. Write path is `IdentityEntry.dashpay_profile`. | Should be zero production emitters — if present, caller bug. |
| 4 | `dashpay_payments_overlay` | same | same as #3 | same as #3 |
| 5 | `IdentityChangeSet.removed` | "backend tasks own identity removal" | **Deliberate scope boundary** — identity deletion via `Database::remove_wallet` / direct SQL. | Need to grep. |
| 6 | `IdentityChangeSet.primary_identity` | "backend tasks own primary tracking" | **Deliberate** — which identity is "primary" is evo-tool UI metadata. | Need to grep. |
| 7 | `IdentityChangeSet.last_scanned_index` | same | **Deliberate** — highest HD index scanned, backend-task-owned. | Need to grep. |

An existing `debug_assert!` at `sqlite.rs:1061-1069` catches the worst case — a changeset containing ONLY these top-level identity fields and nothing else — but is lost in release builds and only covers that one corner.

### Production `Database::*` writes that bypass the persister

Identified from `#[cfg(test)]`-not-gated public methods in `src/database/*.rs`:

- **Identity blob** (the big one, task #130):
  - `Database::insert_local_qualified_identity` — ~9 call sites across `backend_task/identity/*.rs`
  - `Database::update_local_qualified_identity` — a few call sites
  - Reason deferred: masternode identities go through single-key wallets that don't have a `WalletId`, so per-wallet changeset flow doesn't cleanly apply.
- **DashPay profiles**:
  - `Database::save_dashpay_profile` — called from `profile_screen.rs` (3 sites), `contact_details.rs`, `contact_requests.rs`. **Also missing `wallet_id` in the write**.
  - `Database::save_dashpay_profile_avatar_bytes` — called from `profile_screen.rs`. **Also silently discarded** (`let _ =`).
- **DashPay payments**:
  - `Database::save_payment`, `Database::update_payment_status` — called from `send_payment.rs`, `payments.rs`. **Also missing `wallet_id` in the write**.
- **Shielded notes**:
  - `Database::insert_shielded_note` — called from `backend_task/shielded/sync.rs`. **Also silently discarded + missing wallet_id depending on path.**
  - `Database::mark_shielded_note_spent`, `Database::delete_shielded_notes` — spendability lifecycle.
  - `Database::upsert_shielded_wallet_meta` — nullifier sync tracking.
- **Asset locks** (partially — persister now owns this, but legacy paths may linger):
  - Grep needed.
- **Contact helpers** (mostly `#[cfg(test)]` already after `131`'s consolidation):
  - `save_contact_request`, `save_dashpay_contact`, `load_contact_request_crypto_rows` — all `#[cfg(test)]`. Good.

### Direct SQL reads from UI (ARCH-2 fallout)

`db.load_dashpay_contacts` is called from 7+ UI/backend sites (enumerated in the ARCH-2 list above). These read SQL directly instead of iterating the in-memory `ManagedIdentity.established_contacts` populated by the persister's `load()` + `apply_changeset` flow. Creates a two-clock problem; the persister-written `dashpay_contacts` rows lack the display fields (`username`, `display_name`, `avatar_url`) that direct-SQL-written rows had — so readers see different data depending on origin.

### Legacy seed_hash references

Only remaining legitimate uses:
- `LockedWalletInfo.seed_hash` — migration bridge; needed until the v33→v34 migration is fully deployed and the `seed_hash` column has been dropped from the schema.
- `ensure_wallet_id_column_and_backfill` — the migration itself.
- `set_wallet_id_for_locked_wallet` — the `WalletMigrationScreen` UPDATE helper.
- `Database::compute_seed_hash` on `ClosedKeyItem` — still needed for the migration screen.
- Pre-v33 migration ladder code (v5 through v32).

All other `seed_hash` references have been renamed to `wallet_id` in the current branch.

---

## Target architecture

**Single rule: the persister is the sole writer AND the ultimate source of truth for all wallet-scoped state. UI and backend tasks read from in-memory models populated by the persister's `load()`, not from SQL directly.**

### Persister owns (write AND read)

All state keyed by `WalletId`:
- **Core wallet state**: chain height, UTXOs, address-pool watermarks, per-account transaction records, account pool state.
- **Asset locks**: full lifecycle with proofs, chain-lock state, funding type, identity-index binding.
- **Identities** — complete:
  - Identity metadata (wallet_index, DPNS names, top-ups, status, key storage).
  - DashPay profile (existing).
  - DashPay payment history (existing).
  - **`identity.data` blob (bincoded `QualifiedIdentity`)** — currently dropped, moves in as part of this refactor. Masternode case handled explicitly (see below).
  - **Identity removal** — `IdentityChangeSet.removed` acted on: persister DELETEs the row (CASCADEs clean up children via FK).
  - **Primary-identity tracking** — moves to a wallet-scoped settings row the persister writes.
  - **Last scanned HD index** — persisted either as a column on the wallet row or as a dedicated per-wallet table.
- **Contacts**: sent requests, incoming requests, established. Already done on write; readers should consume in-memory state.
- **DashPay payments**: already done on write; readers too.
- **Platform address balances**: **decision needed** — see "Open design questions" below.
- **Token balances**: **decision needed**.
- **Shielded notes**: moves into the persister. Today owned by backend tasks (with the SF-C1 silent-drop bug).

### Persister does NOT own (legitimate exceptions)

- **`settings` table** — application preferences, network selection, UI state. Not wallet-scoped.
- **`single_key_wallet` table** — a different crypto domain (masternode operator keys, etc.). Has its own lifecycle. Does NOT flow through `PlatformWalletChangeSet` (which is keyed by `WalletId`).
- **Schema migrations** — `Database::initialize` and its consolidated v33→v34 migration run outside the persister.
- **Debug / reporting queries** — read-only views for diagnostics can query SQL directly, but must be clearly marked as debug-only and not drive production UI.

### Masternode identity handling (task #130)

The blocker on #130 was: masternode identities are registered via single-key wallets, not HD wallets, and so don't have a `WalletId`. The per-wallet changeset flow doesn't cover them.

**Resolution**: two-tier identity flow:
- **HD-wallet identities**: flow through the `WalletId`-keyed `IdentityEntry.identity_data` field. Persister writes blob via UPSERT keyed by `identity_id`.
- **Masternode identities**: flow through a **separate** `MasternodeIdentityChangeSet` (or similar) that is keyed by `(operator_key_hash, identity_id)` — NOT `WalletId`. Persister handles both via dispatch in `flush_inner`.

This preserves the single-source-of-truth property (all writes through persister) without forcing masternode identities into an ill-fitting per-HD-wallet flow.

### Read-path contract

- **Single boot-time load**: `PlatformWallet::load_persisted` calls `persister.load(wallet_id)` → returns `PlatformWalletChangeSet` → `apply_changeset` populates in-memory `ManagedWalletInfo` / `ManagedIdentity`.
- **Mid-session writes** go through the persister, and `apply_changeset` is invoked in the same transaction so in-memory state is always a reflection of flushed state.
- **Readers** (UI, backend tasks) iterate in-memory state, never SQL.
- **Re-load** on screen transitions is unnecessary; the in-memory state is authoritative until the next flush.
- **Debug accessors** that need SQL are explicitly named (e.g. `db.raw_dashpay_contacts_for_debug`) and never used in production UI.

### Error-handling contract

- **Every write path** returns `Result<_, TypedError>`. No `let _ =` on DB operations outside `#[cfg(test)]`.
- **Silent drops forbidden**. If the persister receives a field it doesn't know how to persist, `flush()` returns `Err(OutOfScope)` at that field's emission point — the caller chose wrong, or the persister needs to grow.
- **Error chain preserved** via `#[source]` on every `thiserror` variant. No stringly-typed errors in persister or migration code.
- **Mutex poisoning** returns a distinguished variant; callers can retry or fail gracefully.
- **Tests lock the contract**: every "drop becomes error" transition has a test that emits the now-forbidden field and asserts the error variant. Regression immediately visible.

---

## Open design questions (resolve before Stage 1)

1. **Platform address balances** — persister-owned or stay direct?
   - Arg for persister: consistency; they're wallet-scoped; balance is state.
   - Arg against: balance is a derived query result from Platform RPC, not wallet-derived state. Writes come from `PlatformPollTask`, which is a backend task, not a mutation the wallet emits.
   - **Recommendation**: persister-owned for reads (cached platform query results are state), but writes come via a dedicated `PlatformAddressChangeSet` that the backend task emits — removing the current silent drop.

2. **Token balances** — same question.
   - **Recommendation**: same as above. Backend task emits a `TokenBalanceChangeSet`; persister persists; UI reads in-memory state.

3. **Primary-identity tracking** — where does it live?
   - Option A: column on the `wallet` table (one primary per wallet). Simple.
   - Option B: a `wallet_metadata` table. More extensible.
   - **Recommendation**: A for now, widen to B if we need more wallet-scoped metadata.

4. **Last scanned HD index** — same question.
   - Option A: column on the `wallet` table.
   - Option B: per-account granularity needed? If so, use the existing `wallet_account_pool_state` table.
   - **Recommendation**: B — add a `last_scanned_index` column to `wallet_account_pool_state` (it already has the right grain).

5. **Shielded notes** — persister-owned?
   - **Recommendation**: yes. The SF-C1 silent-fund-loss bug is the pre-existing consequence of the split. A new `ShieldedChangeSet` sub-changeset carries note insertions + nullifier marks. Persister writes them, UI reads in-memory `ShieldedWallet::notes`.

6. **Should UI-to-platform-wallet accessors live on `PlatformWallet` or on `ManagedWalletInfo`?**
   - `PlatformWallet` is the wallet facade UI interacts with. Add `pw.read_established_contacts(identity_id)`, `pw.read_dashpay_profile(identity_id)`, etc.
   - **Recommendation**: on `PlatformWallet`. UI should never need to reach into `ManagedWalletInfo` or `ManagedIdentity` directly.

7. **Transaction boundaries** — does one `flush()` call commit everything or split?
   - Current: one `rusqlite::Transaction` per `flush()` call covers core + identity subset + asset locks + contacts.
   - With the expanded scope (identity blob, shielded, platform balances, tokens), this becomes a larger transaction. Risk: longer lock holds, more FK checks.
   - **Recommendation**: one transaction still. The atomicity property is too valuable to break. Measure transaction duration under sync-storm load before considering batching.

8. **What about SPV high-volume emission?** (Architecture agent's Q4)
   - Today the persister flushes inline on every `store`. For SPV initial sync (hundreds of thousands of blocks), this could dominate wall time.
   - **Recommendation**: measure first. If the benchmark says it matters, add a bounded batch window (250ms or N changesets). This is a separate follow-up, not part of this refactor.

---

## Staged execution plan

Each stage is a self-contained unit that leaves `main` green and the test suite passing. Stages are ordered by dependency; later stages assume earlier ones landed.

### Stage 0 — alignment + doc (this document)

- Agree on the target architecture, especially the open design questions above.
- Land this document in `docs/ai-design/2026-04-15-persister-unification/`.
- Update `src/changeset/sqlite.rs` module docs to reflect the target rather than the current scope.
- **Effort: 2 hours (after alignment).**

### Stage 1 — eliminate silent drops, case-by-case

For each of the 7 drop cases in `flush_inner`:

1. **`dashpay_profiles` overlay write-path emission** → hard error. The field exists ONLY for load; populating it on write is a caller bug. Add `SqlitePersistError::OverlayFieldOnWritePath { field }`.
2. **`dashpay_payments_overlay`** → same.
3. **`PlatformAddressChangeSet`** → per design question 1: implement it in the persister. Write path + load path.
4. **`TokenBalanceChangeSet`** → per design question 2: implement.
5. **`IdentityChangeSet.removed`** → implement. `flush_inner` DELETEs the `identity` row (CASCADEs child rows).
6. **`IdentityChangeSet.primary_identity`** → implement per design question 3.
7. **`IdentityChangeSet.last_scanned_index`** → implement per design question 4.

For each: add a round-trip test (the TC-H1-style "emit field, flush, load, assert present").

Remove the `debug_assert!` at `sqlite.rs:1061-1069` — after this stage, no top-level fields are silently dropped, so the guard is obsolete.

**Effort: 3 days.**
**Risk: medium.** Finding and migrating direct-write callers of these fields is the hard part.

### Stage 2 — identity blob into persister (#130)

Closes the biggest remaining gap. HD-wallet identities flow through `IdentityEntry.identity_data`; masternode identities flow through a new `MasternodeIdentityChangeSet`.

- Add `identity_data: Option<Vec<u8>>` to `IdentityEntry` (in `platform-wallet`).
- Add `MasternodeIdentityChangeSet` to `PlatformWalletChangeSet` (in `platform-wallet`).
- Persister writes both; load reconstructs both.
- Migrate ~15 callers of `Database::insert_local_qualified_identity` / `update_local_qualified_identity` to emit changesets instead.
- Delete or `#[cfg(test)]`-gate the direct-write helpers.

**Effort: 4 days.**
**Risk: high.** Touches identity creation flow in many backend tasks.

### Stage 3 — shielded notes into persister

Closes SF-C1 (the shielded-fund-loss silent drop).

- Add `ShieldedChangeSet` (note insertions, nullifier spends, sync metadata) to `PlatformWalletChangeSet`.
- Persister writes shielded_notes + shielded_wallet_meta.
- Migrate `backend_task/shielded/sync.rs` to emit changesets.
- Delete `Database::insert_shielded_note` etc. direct writers (or `#[cfg(test)]`).

**Effort: 2 days.**
**Risk: medium.** Shielded sync is a concentrated surface.

### Stage 4 — DashPay profile + payment direct-writer elimination

Closes SF-C2 + SF-M5.

- All `Database::save_dashpay_profile` callers → emit `IdentityEntry.dashpay_profile` changesets.
- All `Database::save_payment` / `update_payment_status` callers → emit `IdentityEntry.dashpay_payments` changesets.
- `save_dashpay_profile_avatar_bytes` same.
- Delete or `#[cfg(test)]`-gate the direct writers.

**Effort: 2 days.**
**Risk: low-medium.** The persister already writes these tables; we're just moving the call sites.

### Stage 5 — UI read consolidation (ARCH-2)

Closes ARCH-2.

- Add `PlatformWallet` accessors: `read_established_contacts`, `read_dashpay_profile`, `read_dashpay_payments`, `read_contact_requests_sent`, `read_contact_requests_incoming`.
- Migrate the 7 UI/backend read sites listed above.
- Delete `Database::load_dashpay_contacts` (or `#[cfg(test)]`).

**Effort: 1-2 days.**
**Risk: low.** Pure read-path migration; behavior equivalent.

### Stage 6 — fix pre-existing silent failures (SF-C4, SF-H1..5, SF-M1..5)

These are `v1.0-dev` bugs but tie into the same hygiene push.

- `get_wallet_identity_indices` → `rusqlite::Result<HashSet<u32>>` + fix callers.
- Lock-contention `if let Ok(...)` silent skips in `wallet_lifecycle.rs` → proper error paths.
- `save_dashpay_profile_avatar_bytes` → propagate error.
- `get_shielded_balance` drop `unwrap_or(0)`.
- Others per the silent-failure-hunter list.

**Effort: 2-3 days.**
**Risk: low-medium.** Some changes ripple into UI error handling.

### Stage 7 — type-design polish (TYPE-3, TYPE-4, TYPE-7)

- `MigrationError` → typed enum (TYPE-3).
- `SqlitePersistError::Encode(String)` → `Encode(#[source] bincode::EncodeError)` (TYPE-4).
- `PlatformWalletPersistence::Error` associated type (TYPE-7).

**Effort: 1 day.**
**Risk: low.**

### Stage 8 — remaining test coverage (TC-H1..H5, TC-M1..M5)

All remaining open test gaps from the test-coverage agent.

**Effort: 2-3 days.**
**Risk: very low.**

### Stage 9 — legacy seed_hash removal (post-release)

After the v33→v34 migration is confirmed deployed in the field for N release cycles:

- Remove `LockedWalletInfo.seed_hash` field (rename or delete).
- Remove `ensure_wallet_id_column_and_backfill` (no longer needed).
- Remove `set_wallet_id_for_locked_wallet`.
- Remove `Database::compute_seed_hash` + v33-era migration helpers.
- Delete the `WalletMigrationScreen` entirely.

**Effort: 1 day.**
**Risk: low — cleanup only.**

### Stage 10 — WalletId newtype (task #142)

Now that all cross-cutting paths flow through the persister, add type-safety on top:

- `pub struct WalletId([u8; 32])` with `ToSql`/`FromSql`/`AsRef<[u8]>`/`Display` impls.
- Migrate ~50 files across 3 repos via compiler-driven refactor.

**Effort: 1-2 days.**
**Risk: medium (blast radius) but caught entirely by the compiler.**

---

## Total size and PR strategy

- **Sum: ~18-22 days of focused work.**
- **Strategy**: one PR per stage on top of `feat/platform-wallet2`. Stage 1 lands first; later stages cherry-pick off its tip.
- **Merge cadence**: stages 1-5 gate on each other and should land before v34 ships to users. Stages 6-10 can trail into a follow-up minor release.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Large blast radius overwhelms review | One stage per PR; each stage compiles + tests green in isolation |
| Stage 2 (identity blob) breaks identity creation | Masternode case designed before execution; integration test coverage per backend-task flow |
| Stage 5 (UI consolidation) hides rendering bugs | Render-path diff screenshots before/after; cover every migrated site |
| Team members add new direct-DB writers during refactor | Post-Stage 1 CI check: grep for `Database::` patterns outside the persister, `#[cfg(test)]`, and allowlisted accessors |
| Performance regression from larger transactions | Benchmark transaction duration per stage; abort if > 50 ms on fixture workloads |

## Open items tracked as tasks

- `#108`, `#136`, `#137`, `#138`, `#139`, `#140` — P0 fixes, all completed.
- `#141` — ARCH-1 `AppLifecycle`. Deferred per user decision.
- `#142` — WalletId newtype. Scheduled as Stage 10.
- `#130` — identity blob in persister. Scheduled as Stage 2 with masternode rethink.

New tasks to open when this plan is approved:
- Stage 1 work items (7 sub-tasks, one per dropped field).
- Stage 2 / #130 masternode design sub-task.
- Stage 3 shielded changeset design sub-task.
- Stages 4-9 top-level trackers.
