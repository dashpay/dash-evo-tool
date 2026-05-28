# DET data migration tool — TODO notes

**Status:** living document. Append-only — earlier entries set context for later ones.
**Audience:** the future migration-tool author.
**Background:** DET's pre-platform-wallet `data.db` (at `<app_data_dir>/data.db`) is being unwired
from the new code path in branch `refactor/platform-wallet-loose-seam` (PR #860). Existing users
will see an empty UI after the unwire ships; this migration tool, planned as a separate library,
will import their legacy `data.db` data into the new storage layout (upstream
`platform-wallet-storage` + DET's k/v abstraction).

---

## Defensive posture

- Take a `.premigration2` snapshot of every database (DET's `data.db` and every
  `<data_dir>/spv/<network>/platform-wallet.sqlite`) before any write. Mirrors the
  `e2f83466` Stage-B pattern.
- Mark "already migrated" with a sentinel file (`<data_dir>/.migrated_to_pws_v1`) or
  a `migrated_to_pws_v1` row in DET's `settings` table — author's choice.
- Migration must be idempotent: re-running after a partial failure must be safe.
- On hard failure: keep `data.db` untouched so re-run works; rollback new-storage
  writes via the `.premigration2` snapshot.

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
- **Status:** TODO

---

### `wallet_addresses` (DET source file: `src/database/wallet.rs`)

- **Source:** `wallet_addresses` in `data.db`
- **Destination:** `account_address_pools` + `core_derived_addresses` in `platform-wallet-storage`
- **Mapping:** Derived address rows split across two upstream tables by address role
- **Per-network split:** Yes
- **Gotchas:** DET's `total_received` column has no upstream equivalent. Either recompute from
  upstream UTXOs post-migration or drop it — SPV will repopulate balances on next sync.
- **Status:** TODO

---

### `wallet_transactions` (DET source file: `src/database/wallet.rs`)

- **Source:** `wallet_transactions` in `data.db`
- **Destination:** `core_transactions` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — migration may be skipped if acceptable to re-fetch via SPV on cold
  start. Decide based on expected cache rebuild cost vs migration complexity.
- **Status:** TODO

---

### `utxos` (DET source file: `src/database/utxo.rs`)

- **Source:** `utxos` in `data.db`
- **Destination:** `core_utxos` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — SPV re-fetch on cold start is an acceptable alternative. Same
  defer-vs-migrate decision as `wallet_transactions`.
- **Status:** TODO

---

### `asset_lock_transaction` (DET source file: `src/database/asset_lock_transaction.rs`)

- **Source:** `asset_lock_transaction` in `data.db`
- **Destination:** `asset_locks` in `platform-wallet-storage`
- **Mapping:** Row-for-row transfer
- **Per-network split:** Yes
- **Gotchas:** Per user direction, the DET table is being left as a dormant artifact in PR #860
  (the unwire PR) — it is not being dropped there. This migration tool is what eventually drains
  the table and drops it. Do not drop the source table until migration is confirmed complete and
  idempotency guard is set.
- **Status:** TODO

---

### `platform_address_balances` (DET source file: `src/database/`)

- **Source:** `platform_address_balances` in `data.db`
- **Destination:** `platform_addresses` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — Platform re-fetch acceptable as alternative to migration.
- **Status:** TODO

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
- **Status:** TODO

---

### `identity_token_balances` (DET source file: `src/database/tokens.rs`)

- **Source:** `identity_token_balances` in `data.db`
- **Destination:** `token_balances` in `platform-wallet-storage`
- **Mapping:** Direct row transfer; pure cache
- **Per-network split:** Yes
- **Gotchas:** Pure cache — Platform re-fetch acceptable.
- **Status:** TODO

---

### DashPay tables (DET source file: `src/database/dashpay.rs`)

Tables: `dashpay_profiles`, `dashpay_contacts`, `dashpay_contact_requests`,
`dashpay_payments`, `dashpay_contact_address_indices`, `dashpay_address_mappings`

- **Source:** Above tables in `data.db`
- **Destination:** `dashpay_profiles`, `contacts_*`, `dashpay_payments_overlay` in
  `platform-wallet-storage`
- **Mapping:** Pending column-by-column parity audit — do not implement until audit is complete
- **Per-network split:** Yes
- **Gotchas:** Some DET-only address-index tables (e.g., `dashpay_contact_address_indices`,
  `dashpay_address_mappings`) may have no upstream equivalent — confirm during audit. Do not
  assume 1:1 column parity; DET and upstream evolved independently.
- **Status:** BLOCKED — column-by-column parity audit required first

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
- **Status:** TODO

---

### `identity_order`, `token_order`

- **Source:** `identity_order`, `token_order` in `data.db`
- **Destination:** upstream k/v store (lightweight UI state)
- **Mapping:** Encode current ordering as a k/v entry per network
- **Per-network split:** Likely yes — DET stores these with a `network` column
- **Gotchas:** Low priority. UI order is cosmetic; missing it is not user-data loss.
- **Status:** TODO

---

### `contract` cache, `token` cache

- **Source:** `contract`, `token` tables in `data.db`
- **Destination:** LRU in-memory cache (not persisted)
- **Mapping:** Do not migrate — regenerable from Platform on cold start
- **Per-network split:** N/A
- **Gotchas:** None expected. Migration tool should explicitly skip these tables and document why.
- **Status:** N/A (table regenerable — do not migrate)

---

### `top_up` (DET source file: `src/database/top_ups.rs`)

- **Source:** `top_up` in `data.db`
- **Destination:** TBD — candidate is `dashpay_payments_overlay` or a new k/v entry
- **Mapping:** Pending schema decision
- **Per-network split:** Likely yes
- **Gotchas:** No clear upstream home yet. Schema decision required before implementation.
- **Status:** BLOCKED — schema destination decision required

---

### `contested_name`, `contestant` (DPNS)

- **Source:** `contested_name`, `contestant` in `data.db`
- **Destination:** DET-only domain — either remain in `data.db` long-term or move to DET k/v sidecar
- **Mapping:** Pending decision
- **Per-network split:** Yes
- **Gotchas:** DPNS contest data has no upstream analog in `platform-wallet-storage`. Migration
  tool author must decide whether to carry these to a DET-specific sidecar store or leave them
  in the old `data.db` under a "legacy section" with a clear ownership comment.
- **Status:** TODO — destination decision pending

---

### `scheduled_votes`

- **Source:** `scheduled_votes` in `data.db`
- **Destination:** DET k/v sidecar (masternode vote queuing — DET-only)
- **Mapping:** Encode as k/v entries keyed by vote identity + target
- **Per-network split:** Yes
- **Gotchas:** No upstream analog. DET-specific feature; stays in DET storage layer.
- **Status:** TODO

---

### `proof_log`

- **Source:** `proof_log` in `data.db`
- **Destination:** None
- **Mapping:** Do not migrate
- **Per-network split:** N/A
- **Gotchas:** Diagnostic-only table. Recommend deleting rows (not the schema) as part of cleanup
  post-migration. Do not carry to new storage.
- **Status:** N/A (diagnostic artifact — delete, do not migrate)

---

### `single_key_wallet`

- **Source:** `single_key_wallet` in `data.db`
- **Destination:** No upstream concept
- **Mapping:** Pending decision
- **Per-network split:** Unknown
- **Gotchas:** No upstream equivalent in `platform-wallet-storage`. Options: (a) keep as
  DET-only feature in a DET sidecar, (b) scope out non-HD wallet support entirely. Decision
  required before migration can be designed.
- **Status:** BLOCKED — feature scope decision required

---

### `shielded_notes`, `shielded_wallet_meta`

- **Source:** `shielded_notes`, `shielded_wallet_meta` in `data.db`
- **Destination:** upstream shielded subsystem
- **Mapping:** Pending parity check
- **Per-network split:** Likely yes
- **Gotchas:** Upstream shielded-storage parity audit not yet done. Do not implement until
  audit confirms column/schema parity between DET and upstream shielded tables.
- **Status:** BLOCKED — upstream shielded audit required

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
- `single_key_wallet` long-term plan: keep as DET sidecar, drop the feature, or push upstream?
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
