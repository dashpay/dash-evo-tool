# DET data migration tool — TODO notes

**Status:** living document. Append-only — earlier entries set context for later ones.
**Audience:** the future migration-tool author.
**Background:** DET's pre-platform-wallet `data.db` (at `<app_data_dir>/data.db`) is being unwired
from the new code path in branch `refactor/platform-wallet-loose-seam` (PR #860). Existing users
will see an empty UI after the unwire ships for the migrated domains; this migration tool, planned
as a separate library, will import their legacy `data.db` data into the new storage layout
(upstream `platform-wallet-storage` + DET's k/v abstraction).

---

## Reference commit SHAs (for the migration-tool author)

- **Last commit with all DET data.db read/write code paths**:
  `35eb07bf67b48a74f14de2f1cd2a907412cc0b9a` (PR #860's pre-unwire tip — bumps the platform
  dep to the k/v-aware SHA but still reads/writes every domain from `data.db`). Check this
  SHA out in a worktree to read the legacy code.
- **Pre-unwire DET v1.0-dev HEAD**: `87ba5b711839219f5e1c7aee8f9de36d038866e3`.
- **Upstream platform pin during unwire**: `17653ba8f9448edc569487b85bb35b27c5f6a14c`
  (`rs-platform-wallet-storage` with k/v store at
  `packages/rs-platform-wallet-storage/src/kv.rs` and SecretStore at
  `packages/rs-platform-wallet-storage/src/secrets/`).

---

## Domains deferred from PR #860 unwire

These two domains were investigated during PR #860 and intentionally left on `data.db`. They
are out of scope for the unwire and must be addressed in their own follow-up PRs before the
migration tool can fully drain `data.db`.

### Shielded (deferred in C8a)

`commitment_tree` is upstream grovedb-owned relational state spread across four tables on the
shared SQLite connection at `data.db`:

- `commitment_tree_shards`
- `commitment_tree_cap`
- `commitment_tree_checkpoints`
- `commitment_tree_checkpoint_marks_removed`

These are written by `dash-spv-shielded`'s grovedb backend, which expects ambient relational
SQL access. They cannot move to the per-wallet k/v store without an upstream grovedb API
change that exposes a single-blob or kv-shaped persistence interface. Until that upstream
work happens, `data.db` remains load-bearing for any user with shielded pool activity.

Other shielded artefacts investigated and resolved:

- **Spending-key derivation**: spending keys are derived in-memory via ZIP32 from the wallet
  seed; no migration needed because the seed is already in upstream SecretStore via Stage-B
  (commit `e2f83466`).
- **Shielded overlay table** (the small per-account index): can be migrated to the upstream
  k/v store, but is blocked behind `commitment_tree` because both must move together to be
  consistent.
- **Sync cursor**: already migrated as part of the per-wallet k/v cursor commit (`ed6ea588`).

### DashPay (deferred in C9, completed in D1–D4d)

**Status update (2026-05-29):** the DashPay deferral closed in commits S1 (shielded retire)
and D1–D4d (DashPay unwire) on branch `feat/unwire-deferred-domains` stacked on PR #860.
Upstream `ManagedIdentity` now owns contacts / requests / profiles / payments, and a
per-network DET k/v sidecar owns DashPay overlays (private memo, blocked / rejected
markers, timestamps, address index, address mapping). The DET tables
(`dashpay_profiles`, `dashpay_contacts`, `dashpay_contact_requests`,
`dashpay_payments`, `dashpay_contact_address_indices`, `dashpay_address_mappings`,
`contact_private_info`) are no longer created on fresh installs and have no live readers
or writers in DET code. Pre-D4d installs keep the dormant rows; the migration tool drains
them at its leisure.

The original C9 investigation notes (kept for the migration-tool author — DET DashPay state
is still readable at SHA `35eb07bf67b48a74f14de2f1cd2a907412cc0b9a`):

1. **SecretStore is not wired into AppContext.** DET does not currently consume `SecretStore`
   directly anywhere — Stage-B uses upstream `SqlitePersister`'s `secrets_backend` config, not
   a DET-held SecretStore handle. A "wire SecretStore into AppContext" commit is needed before
   private-memo migration (private memos are upstream's only encrypted-payload feature, and
   they require a SecretStore handle to read/write).
2. **State-machine vocabulary mismatch.** DET carries explicit lifecycle vocabulary that
   upstream does not:
   - `StoredContactRequest.status ∈ {pending, accepted, rejected, expired}`
   - `StoredContact.contact_status ∈ {pending, accepted, blocked}`
   - `StoredPayment.status ∈ {pending, confirmed, failed}`
   Upstream is presence-based: a contact is "accepted" iff it appears in
   `contacts_established`, "pending" iff it appears in `contact_requests_outgoing`, etc.
   Bridging requires a multi-screen UI redesign and corresponding backend-task changes.
3. **Schema shape mismatch.** DET's `contact_private_info` is
   `(nickname, notes, is_hidden, created_at, updated_at)`. Upstream's equivalent is
   `(encrypted_bytes, hidden_flag)`. Either DET adopts the upstream schema (lossy: loses the
   structured nickname/notes split) or upstream extends its schema (out of DET's control).
4. **Backend-task API redesign.** DET's `DashPayTask::AcceptContactRequest` and friends key
   by an i64 PK on the contact-request row. Upstream keys by `(owner_id, sender_id)`. Every
   DashPay backend task signature needs to change to the upstream key shape.
5. **Memo loss accepted.** Per user direction, the migration may drop DET's per-payment memos
   on the floor — no k/v memo overlay is required. This simplifies migration but does not
   simplify the UI re-platform.

Until these prerequisites land, every DashPay-related table on `data.db` stays load-bearing:

- `dashpay_profiles`
- `dashpay_contacts`
- `dashpay_contact_requests`
- `dashpay_contact_address_indices`
- `dashpay_address_mappings`
- `dashpay_payments`
- `contact_private_info`

### `asset_lock_transaction` (code deleted)

The `asset_lock_transaction` table had no live writers since commit `5cc6e893` (loose-seam
refactor). The entire `src/database/asset_lock_transaction.rs` module has now been deleted,
including the `CREATE TABLE` on fresh installs. Existing `data.db` rows on legacy installs
remain inert. The migration tool drains those rows by retrieving the deleted module via git
history at SHA `35eb07bf` (the last commit where the module compiled). No DROP TABLE is
emitted from DET — the legacy table is left in place until the migration tool ships and
idempotency is confirmed.

---

## Per-table migration list

### `wallet` (DET source file: `src/database/wallet.rs`)

- **Source:** `wallet` in `data.db`, columns: see `wallet.rs`
- **Destination:** `wallet_metadata` in `platform-wallet-storage`
- **Mapping:** One row per wallet; carry seed/keys and derivation metadata
- **Per-network split:** Yes — DET single-db with `network` column → upstream per-network persister
- **Gotchas:** Stage-B (`e2f83466`) already mirrors newly created wallets to upstream; migration
  tool may only need to handle wallets that pre-date Stage-B execution (i.e., wallets written
  before the user first launched post-Stage-B code). Detect by checking whether the upstream db
  already has a matching wallet entry before inserting.
- **Status:** DONE for new-install path — see commit `e2f83466` (Stage-B). Migration tool
  still needs to import pre-Stage-B installs.

---

### `wallet_addresses` (DET source file: `src/database/wallet.rs`)

- **Source:** `wallet_addresses` in `data.db`
- **Destination:** `account_address_pools` + `core_derived_addresses` in `platform-wallet-storage`
- **Mapping:** Derived address rows split across two upstream tables by address role
- **Per-network split:** Yes
- **Gotchas:** DET's `total_received` column has no upstream equivalent. Either recompute from
  upstream UTXOs post-migration or drop it — SPV will repopulate balances on next sync.
- **Status:** DONE for new-install path — see commit `09a7dfb7` (signer-driven flows on
  upstream wallet). Migration tool still needs to import legacy rows.

---

### `wallet_transactions` (DET source file: `src/database/wallet.rs`)

- **Source:** `wallet_transactions` in `data.db`
- **Destination:** `core_transactions` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — migration may be skipped if acceptable to re-fetch via SPV on cold
  start. Decide based on expected cache rebuild cost vs migration complexity.
- **Status:** DONE for new-install path — see commit `09a7dfb7`. Migration tool may skip
  (cache is regenerable from SPV).

---

### `utxos` (DET source file: `src/database/utxo.rs`)

- **Source:** `utxos` in `data.db`
- **Destination:** `core_utxos` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — SPV re-fetch on cold start is an acceptable alternative. Same
  defer-vs-migrate decision as `wallet_transactions`.
- **Status:** DONE for new-install path — see commit `09a7dfb7`. Migration tool may skip
  (cache is regenerable from SPV).

---

### `asset_lock_transaction` (DET source file: deleted; retrieve via git history)

- **Source:** `asset_lock_transaction` in `data.db` on legacy installs
- **Destination:** `asset_locks` in `platform-wallet-storage`
- **Mapping:** Row-for-row transfer
- **Per-network split:** Yes
- **Gotchas:** The DET module `src/database/asset_lock_transaction.rs` has been deleted as
  part of the unwire cleanup. Fresh installs no longer have the table. To read schema
  details and CRUD shape for the migration tool, retrieve the deleted module via git
  history at SHA `35eb07bf` (last compiling revision). Do not emit DROP TABLE from DET
  itself — the legacy table is left dormant until the migration tool ships and idempotency
  is confirmed.
- **Status:** DEFERRED — module deleted; rows inert on legacy installs.

---

### `platform_address_balances` (DET source file: `src/database/`)

- **Source:** `platform_address_balances` in `data.db`
- **Destination:** `platform_addresses` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — Platform re-fetch acceptable as alternative to migration.
- **Status:** DONE for new-install path — see commit `ed6ea588` (wallet platform-address-info
  + sync cursor → per-wallet k/v). Migration tool may skip (cache is regenerable from Platform).

---

### `identity` (DET source file: `src/database/identities.rs`)

- **Source:** `identity` in `data.db`
- **Destination:** `identities.entry_blob` (typed BLOB column) in `platform-wallet-storage`
- **Mapping:** Deserialize DET's stored identity representation → serialize as
  bincode-encoded `QualifiedIdentity` with a leading version byte prepended
- **Per-network split:** Yes
- **Gotchas:** Upstream schema uses a leading version byte in `entry_blob` for
  forward/backward compatibility — this byte must be present and set correctly, or upstream
  deserialization will silently produce garbage. Confirm the byte format with the
  platform-wallet-storage author before implementing. This is the highest-risk table in the
  migration.
- **Status:** DONE for new-install path — see commit `b14bf32c` (identities + tokens →
  per-network k/v). Migration tool still needs to import legacy rows with the version-byte
  contract correct.

---

### `identity_token_balances` (DET source file: `src/database/tokens.rs`)

- **Source:** `identity_token_balances` in `data.db`
- **Destination:** `token_balances` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — Platform re-fetch acceptable.
- **Status:** DONE for new-install path — see commit `b14bf32c`. Migration tool may skip
  (cache is regenerable from Platform).

---

### DashPay tables (DET source file: `src/database/dashpay.rs`)

Tables: `dashpay_profiles`, `dashpay_contacts`, `dashpay_contact_requests`,
`dashpay_payments`, `dashpay_contact_address_indices`, `dashpay_address_mappings`,
`contact_private_info`

- **Source:** Above tables in `data.db`
- **Destination:** `dashpay_profiles`, `contacts_*`, `dashpay_payments_overlay` in
  `platform-wallet-storage`
- **Mapping:** Pending column-by-column parity audit — do not implement until audit is complete
- **Per-network split:** Yes
- **Gotchas:** Some DET-only address-index tables (e.g., `dashpay_contact_address_indices`,
  `dashpay_address_mappings`) may have no upstream equivalent — confirm during audit. Do not
  assume 1:1 column parity; DET and upstream evolved independently.
- **Status:** DONE (D1–D4d unwire on `feat/unwire-deferred-domains`, stacked on PR #860).
  S1 retired the shielded data.db code path; D1 introduced the `DashpayView` adapter; D2
  wired sidecar reads/writes for DET-only overlays; D3 added blocked/rejected/timestamp
  markers; D4a–D4c migrated every DashPay read and write off the DET tables; D4d deletes
  `src/database/dashpay.rs` (894 LOC) and `src/database/contacts.rs` (356 LOC), drops all
  `CREATE TABLE` entries from `database/initialization.rs`, collapses 3 UI dual-writes to
  sidecar-only writes, and extends `AppContext::clear_network_database` with a
  `det:dashpay:` prefix sweep on the per-network k/v sidecar. The migration tool reads
  DET DashPay rows at SHA `35eb07bf67b48a74f14de2f1cd2a907412cc0b9a` (pre-unwire) and
  writes upstream-owned state into `ManagedIdentity` plus DET overlays into the
  `det:dashpay:*` k/v namespace.

---

### `settings` (DET source file: `src/database/settings.rs`)

- **Source:** `settings` in `data.db`
- **Destination:** upstream k/v store (k/v shipped in upstream PR #3625 at SHA
  `8c4a88a2cc7bead81e8441883afb7f69d3bf59cb`)
- **Mapping:** Each DET settings key → corresponding k/v entry; fields include network,
  fee multiplier, theme, developer mode, etc.
- **Per-network split:** Some settings are global (theme, dev mode); others may be per-network.
  Audit at implementation time.
- **Gotchas:** Mock the k/v shape until the upstream pin bump lands in DET. Do not hardcode
  key names — derive them from the same constant definitions the library exports.
- **Status:** DONE for new-install path — see commits `e4ff9621` (`AppSettings` user prefs
  → upstream k/v) and `02537507` (selected-wallet hashes → per-network k/v). The DET
  `settings` table now only carries the migration-runner version counter; migration tool
  still needs to import legacy preference rows from pre-C3 installs.

---

### `identity_order`, `token_order`

- **Source:** `identity_order`, `token_order` in `data.db`
- **Destination:** upstream k/v store (lightweight UI state)
- **Mapping:** Encode current ordering as a k/v entry per network
- **Per-network split:** Likely yes — DET stores these with a `network` column
- **Gotchas:** Low priority. UI order is cosmetic; missing it is not user-data loss.
- **Status:** DONE for new-install path — see commit `b14bf32c`. Migration tool may skip
  (cosmetic ordering is acceptable to lose).

---

### `contract` cache, `token` cache

- **Source:** `contract`, `token` tables in `data.db`
- **Destination:** LRU in-memory cache (not persisted)
- **Mapping:** Do not migrate — regenerable from Platform on cold start
- **Per-network split:** N/A
- **Gotchas:** None expected. Migration tool should explicitly skip these tables and document why.
- **Status:** DONE — see commits `e8bc5a6a` (`contract` → per-network k/v) and `b14bf32c`
  (`token` removed). Migration tool explicitly skips both per "regenerable cache" rule.

---

### `top_up` (DET source file: `src/database/top_ups.rs`)

- **Source:** `top_up` in `data.db`
- **Destination:** TBD — candidate is `dashpay_payments_overlay` or a new k/v entry
- **Mapping:** Pending schema decision
- **Per-network split:** Likely yes
- **Gotchas:** No clear upstream home yet. Schema decision required before implementation.
- **Status:** DONE for new-install path — see commit `7778eb64` (`top_ups` → k/v). Migration
  tool still needs to import legacy rows.

---

### `contested_name`, `contestant` (DPNS)

- **Source:** `contested_name`, `contestant` in `data.db`
- **Destination:** DET-only domain — either remain in `data.db` long-term or move to DET k/v sidecar
- **Mapping:** Pending decision
- **Per-network split:** Yes
- **Gotchas:** DPNS contest data has no upstream analog in `platform-wallet-storage`. Migration
  tool author must decide whether to carry these to a DET-specific sidecar store or leave them
  in the old `data.db` under a "legacy section" with a clear ownership comment.
- **Status:** DONE for new-install path — see commit `e8bc5a6a` (`contested_names` →
  per-network k/v). Migration tool still needs to import legacy rows.

---

### `scheduled_votes`

- **Source:** `scheduled_votes` in `data.db`
- **Destination:** DET k/v sidecar (masternode vote queuing — DET-only)
- **Mapping:** Encode as k/v entries keyed by vote identity + target
- **Per-network split:** Yes
- **Gotchas:** No upstream analog. DET-specific feature; stays in DET storage layer.
- **Status:** DONE for new-install path — see commit `7778eb64` (`scheduled_votes` → k/v).
  Migration tool still needs to import legacy rows.

---

### `proof_log`

- **Source:** `proof_log` in `data.db`
- **Destination:** None
- **Mapping:** Do not migrate
- **Per-network split:** N/A
- **Gotchas:** Diagnostic-only table. Recommend deleting rows (not the schema) as part of cleanup
  post-migration. Do not carry to new storage.
- **Status:** DONE — see commit `7778eb64` (proof_log → tracing). Migration tool explicitly
  skips and may drop legacy rows.

---

### `single_key_wallet`

- **Source:** `single_key_wallet` in `data.db`
- **Destination:** Two stores — private bytes to `SecretStore` label `single_key_priv.<addr>` scoped to `SINGLE_KEY_NAMESPACE_ID` (a fixed constant, not a per-HD-wallet `WalletId`); enumerable metadata (`address`, `alias`, `network`) to `det-app.sqlite` under `<network>:single_key_meta:<addr>`.
- **Mapping:** One row → one `SecretStore` entry + one `DetKv` entry. See `src/wallet_backend/single_key.rs`.
- **Per-network split:** Yes — the metadata sidecar key carries the `<network>:` prefix; `SecretStore` uses the fixed `SINGLE_KEY_NAMESPACE_ID` scope for all networks.
- **Gotchas:** `SINGLE_KEY_NAMESPACE_ID` is shared across all imported keys regardless of network — `<network>:` in the sidecar key is the partition. Do not confuse with per-HD-wallet `WalletId`. The `platform-wallet-storage` label allowlist rejects colons — the label uses a dot: `single_key_priv.<addr>`.
- **Status:** DONE for new-install path — D-2 decision in `docs/ai-design/2026-05-29-finish-unwire/notes.md`. See `src/wallet_backend/single_key.rs` (T-SK-01 + T-SK-02). Migration tool still needs to import legacy `single_key_wallet` rows into the two-store layout.

---

### `shielded_notes`, `shielded_wallet_meta`, `commitment_tree_*`

- **Source:** `shielded_notes`, `shielded_wallet_meta`, and the four `commitment_tree_*`
  tables in `data.db`
- **Destination:** upstream shielded subsystem (k/v overlay + future grovedb API)
- **Mapping:** Pending parity check
- **Per-network split:** Likely yes
- **Gotchas:** Upstream shielded-storage parity audit not yet done. `commitment_tree_*` is
  blocked behind an upstream grovedb API change (see "Domains deferred" section).
- **Status:** DEFERRED — see "Domains deferred" section above. Cannot migrate
  `commitment_tree_*` without upstream grovedb API change; the rest move together with it.

---

## Cross-cutting concern: per-network split

DET stores all networks in one `data.db` with a `network` column. Upstream
`platform-wallet-storage` is per-network: one database per network at
`<data_dir>/spv/<network>/platform-wallet.sqlite`.

Migration tool must route each row to the correct per-network persister. Write a helper that,
given a `network` string from DET, resolves the upstream persister instance. Test with all three
networks (mainnet, testnet, devnet) before shipping.

---

## Open questions for the migration-tool author

- DashPay column-by-column parity audit: incomplete. Required before any DashPay table
  can be migrated.
- Shielded-storage upstream parity: incomplete. Required before shielded tables can be migrated.
- Per-network vs single-instance scope for cross-network tables (`settings`,
  `identity_order`, `token_order`): confirm which keys are global vs per-network.
- DET k/v sidecar location: `<app_data_dir>/det-kv.sqlite` (single global) or
  `<app_data_dir>/spv/<net>/det-kv.sqlite` (per-network)? Pending user decision.
- `single_key_wallet` long-term plan: decided — keep via `SecretStore` + `det-app.sqlite` sidecar (D-2 in finish-unwire ADR). Drop-with-sunset is deferred pending user-population data.
- `top_up` schema destination: `dashpay_payments_overlay` or new k/v entry?
- `contested_name` / `contestant` permanent home: old `data.db` legacy section, DET k/v
  sidecar, or something else?
- Backup retention policy for `.premigration2` snapshots (see "Operational decisions" below).
- What constitutes a "successful launch" for purposes of snapshot pruning?

---

## Operational decisions still open

- Backup retention policy (`.premigration2` snapshots): keep indefinitely, prune after N
  successful launches, or user-configurable?
- Should migration run automatically on first launch after upgrade, or require user confirmation
  via a one-shot dialog?
- What does the UI show during the migration step? (Spinner with progress, modal blocker, banner?)
- Failure-mode UX: what does the user see if migration fails halfway? (Banner with guidance,
  auto-rollback message, or "old DB still intact, click to retry" prompt?)

---

## Change log

- 2026-05-28: Initial document created. Seeded all known tables and open questions from
  planning session. Branch `refactor/platform-wallet-loose-seam` (PR #860).
- 2026-05-29: Closed out PR #860. Annotated every per-table entry with the unwire commit SHA
  for the new-install path. Added "Reference commit SHAs" section pointing at the pre-unwire
  tip (`35eb07bf`), the pre-unwire `v1.0-dev` HEAD (`87ba5b71`), and the upstream platform
  pin (`17653ba8`). Added "Domains deferred" section explaining why shielded and DashPay
  stay on `data.db` after PR #860.
