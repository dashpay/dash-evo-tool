# Legacy identity import (v0.9.3 → v1.0)

**Status:** implemented and shipped — PR #885 (`feat/legacy-identity-migration`),
with QA follow-ups in PR #891. All tasks in §9 (T-ID-01 … T-ID-06) landed. This
document is retained as the design record: it describes shipped behaviour, not a
pending proposal.
**Scope:** carry the legacy `data.db` `identity` rows into the modern
`StoredQualifiedIdentity` k/v store during the cold-start migration.
**Known limitation:** a partially-loaded identity strands its legacy-only keys
(§7); an opt-in recovery flow is tracked in
[issue #889](https://github.com/dashpay/dash-evo-tool/issues/889).

---

## 1. The gap

The schema ladder preserves the v0.9.3 `identity` table intact — `v093_upgrade.rs`
asserts exactly that: `alias`, `wallet`, `wallet_index`, `identity_type` and
`network` all survive from schema v11 to current. Nothing then reads it. No
production code path imports those rows into the per-network k/v store that
`AppContext::load_local_qualified_identities` reads from, so a v0.9.3 user who
upgrades finds an empty Identities screen and an empty Masternodes screen. A
masternode owner loses the owner/voting/payout keys they had loaded and must
re-import them by hand.

The `2026-05-28-migration-tool` notes already called `identity` "the highest-risk
table in the migration" and left it at "Migration tool still needs to import
legacy rows with the version-byte contract correct." This document settles what
that contract actually is, and it is not what the note assumed.

---

## 2. What v0.9.3 actually persisted

Source: `git show v0.9.3:src/database/identities.rs`, `…:src/database/initialization.rs`.

```sql
CREATE TABLE identity (
    id BLOB PRIMARY KEY,          -- 32-byte Identifier
    data BLOB,                    -- QualifiedIdentity::to_bytes()  (NULLABLE)
    status INTEGER NOT NULL DEFAULT 0,
    is_local INTEGER NOT NULL,    -- 1 = user-owned, 0 = observed-only
    alias TEXT,
    info TEXT,                    -- never written by v0.9.3
    wallet BLOB,                  -- WalletSeedHash ([u8; 32]), nullable
    wallet_index INTEGER,         -- u32, NOT NULL iff wallet IS NOT NULL
    identity_type TEXT,           -- format!("{:?}") of IdentityType
    network TEXT NOT NULL,
    CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL)
        OR (wallet IS NULL AND wallet_index IS NULL))
);
```

Three facts that decide the whole design:

1. **`data` is the entire identity, keys included.** v0.9.3's
   `insert_local_qualified_identity` writes `qualified_identity.to_bytes()` —
   bincode of the full `QualifiedIdentity`, whose `private_keys: KeyStorage`
   carries owner / voting / payout private-key material as `Clear`,
   `AlwaysClear`, `Encrypted`, or `AtWalletDerivationPath`. There is **no second
   table** holding identity keys in v0.9.3. Everything to migrate is in this one
   BLOB plus five scalar columns. Key material is therefore *not* re-derivable
   from the wallet seed in the general case: a masternode owner's owner/voting
   key is typically an imported WIF stored as `Clear`, not an HD path.
2. **`data` is nullable, and `is_local` is not always 1.** v0.9.3's
   `insert_identity_if_not_exists` writes observed (non-local) identities with
   `is_local = 0` and a possibly-`NULL` blob. Every v0.9.3 read path filters
   `WHERE is_local = 1 AND data IS NOT NULL`. The import must filter identically
   — a non-local row is a cache entry, not user data.
3. **`identity_type` is `Debug`-formatted**, yielding exactly `"User"` /
   `"Masternode"` / `"Evonode"` — byte-identical to the modern
   `IdentityType::as_tag()`. **All three variants can appear.** The test's
   masternode-only fixture is *not* representative: a v0.9.3 user with a DPNS
   name has a `User` identity, and an evonode operator has `Evonode`. The import
   must be type-agnostic (it is, if it round-trips the blob).

---

## 3. The target shape

`src/context/identity_db.rs`:

```rust
struct StoredQualifiedIdentity {
    qi_bytes: Vec<u8>,          // QualifiedIdentity::to_bytes()
    status: u8,
    identity_type: String,      // IdentityType::as_tag()
    wallet_hash: Option<[u8; 32]>,
    wallet_index: Option<u32>,
}
```

written at `DetScope::Identity(&id)` / key `det:identity:v1`, with the id also
registered in the Global roster `det:identity_index:v1` (there is no
cross-identity listing under `DetScope::Identity`, so an identity absent from
the roster is invisible to every load path).

The production writer is `AppContext::insert_local_qualified_identity(&qi, &Some((seed_hash, index)))`.
It already does, in order: vault-first key extraction → blob encode → roster
insert → `DetKv::put`. **The import must call it and nothing lower.**

---

## 4. The version-byte contract

The 2026-05-28 note said the destination was upstream's `identities.entry_blob`
with a leading version byte "confirm the byte format with the
platform-wallet-storage author before implementing." That is **stale**. The
destination moved (commit `b14bf32c`) to DET's own per-network k/v. There are now
two byte layers, and neither needs a new author agreement:

| Layer | Produced by | Format |
|---|---|---|
| **Outer** (k/v value) | `DetKv::put` — `src/wallet_backend/kv.rs:188` | `[ SCHEMA_VERSION (1B) = 1 ‖ bincode(StoredQualifiedIdentity) ]` |
| **Inner** (`qi_bytes`) | `QualifiedIdentity::to_bytes()` — `src/model/qualified_identity/mod.rs:510` | `bincode(QualifiedIdentity, config::standard())` — **no version byte** |

The outer byte is prepended automatically and validated on read
(`KvAdapterError::SchemaVersion` on mismatch). The importer gets it for free by
going through `insert_local_qualified_identity`. **Hand-rolling the blob bytes is
the one way to get this wrong; the design forbids it.**

The inner layer is where the real risk lives, and it is a *cross-version bincode
compatibility* question, not a version-byte question:

### 4.1 Evidence that the legacy blob decodes on HEAD

| Item | v0.9.3 | HEAD | Verdict |
|---|---|---|---|
| `QualifiedIdentity` manual `Encode` field order | `identity, associated_voter_identity, associated_operator_identity, associated_owner_key_id, identity_type, alias, private_keys, dpns_names` | identical | ✅ |
| `IdentityType` | `User, Masternode, Evonode` | identical | ✅ |
| `PrivateKeyTarget` | 3 variants, same order | identical | ✅ |
| `QualifiedIdentityPublicKey` | `{ identity_public_key, in_wallet_at_derivation_path }` | identical | ✅ |
| `KeyStorage` | `BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>` | identical | ✅ |
| `PrivateKeyData` | `AlwaysClear, Clear, Encrypted, AtWalletDerivationPath` (0–3) | same 0–3, `InVault` **appended** at 4 | ✅ legacy discriminants unchanged |
| `WalletSeedHash` | `[u8; 32]` | identical | ✅ |
| dpp `IdentityV0` / `IdentityPublicKeyV0` fields (platform `29f7492` vs `44c20e3`) | `{id, public_keys, balance, revision}` / `{id, purpose, security_level, contract_bounds, key_type, read_only, data, disabled_at}` | identical — the inter-rev diff is serde attributes and added tests only | ✅ |
| dpp `Purpose` / `SecurityLevel` / `KeyType` / `ContractBounds` | — | no variant added or reordered | ✅ |
| bincode | `=2.0.0-rc.3` | `=2.0.1` | ⚠️ **unverified** |

Everything structural checks out. The one open item is whether bincode's
`config::standard()` wire format is identical between `2.0.0-rc.3` and `2.0.1`.
It almost certainly is (varint + little-endian was frozen well before rc.3), but
"almost certainly" is not a contract for a blob that holds a masternode owner
key. **T-ID-06 (§9) closes it empirically with a golden blob.** Nothing else in
this plan depends on the answer; if the format *did* drift, the import would fail
loudly on decode (bincode errors, it does not silently produce garbage), the
sentinel would be withheld, and the legacy rows would still be there.

---

## 5. Where the step plugs in

A **sibling pass in `finish_unwire::run`**, after `drain_wallets` succeeds, under
its **own per-network sentinel**:

```
det:migration:identities:<net>:v1
```

```rust
pub async fn run(app_context: &Arc<AppContext>) -> Result<bool, TaskError> {
    // Held, NOT unwrapped: DET's own rows may not gate the funds path.
    let app_data = migrate_app_data(app_context);

    // Funds first. Nothing above may withhold a seed.
    let wallet_moved = drain_wallets(app_context).await?;

    // Held too — the two DET-owned passes must not gate each other.
    let identities = migrate_identities(app_context)?;

    if identities.unreadable > 0 {
        // `app_data` is judged HERE, never before: unwrapping it first would
        // return its (deterministic) error and mask the identity signal on this
        // launch AND every retry, stranding a masternode owner's keys over a
        // corrupt vote queue. Each arm publishes its own terminal state and
        // returns `Ok` — a returned `Err` would be re-published as a plain
        // `Failed`, dropping the identity count.
        match app_data {
            // SucceededWithUnreadableIdentities, or …AndVotes when the durable
            // vote-warning record reads back non-empty. A failing *read* costs
            // only the vote half — never the identity half, and never a `Failed`.
            Ok(outcome) => {
                let moved_data =
                    wallet_moved || outcome.moved_data() || identities.moved_data();
                …
                return Ok(moved_data);
            }
            // The one true FailedWithUnreadableIdentities: a hard app-data
            // failure. Both signals ride one retryable banner.
            Err(app_data_error) => {
                let moved_data = wallet_moved || identities.moved_data();
                …
                return Ok(moved_data);
            }
        }
    }

    // Every identity decoded, so app-data no longer masks anything: unwrap it.
    let app_data = app_data?;
    let moved_data = wallet_moved || app_data.moved_data() || identities.moved_data();
    …
}
```

**Held, then judged.** Each DET-owned pass runs unconditionally and its `Result`
is *held* — the order in which the two are **unwrapped** is the load-bearing part,
not the order in which they run. An app-data failure is deterministic (one
malformed vote-index blob is enough) and never writes its sentinel, so unwrapping
it ahead of the identity outcome would skip the identity import forever, not just
once.

**Why after `drain_wallets`, not inside it.** The import needs three things the
drain produces: the wallet backend wired (`det_kv()` is `wallet_backend()?.kv()`),
the secret vault reachable (key material goes there), and `ctx.wallets` hydrated
(`register_migrated_wallets` calls `hydrate_context_wallets`) so that
`AtWalletDerivationPath` keys land against a wallet that actually exists. Running
it inside the drain, before registration, would import identities whose wallet
link points at nothing yet.

**Why a new sentinel, not the drain's.** This is the same trap the app-data
sentinel was created to dodge, and the code says so at `migrate_app_data`: *"an
install that already completed the wallet drain under an earlier build (which had
no app-data import) still has those rows in `data.db`, and the wallet sentinel
would otherwise short-circuit the launch and strand them."* Every alpha/RC tester
who has already run the drain has `det:migration:finish_unwire:<net>:v1` written.
Reusing it would ship an identity importer that never runs for exactly the people
testing it. A third sentinel costs one constant.

**Failure isolation.** A failing identity import must not withhold the wallet
sentinel — funds access is already restored and sentinel-guarded by the time this
runs. Conversely the identity sentinel is withheld on any failure so the next
launch retries. `MigrationStep::Identities` is added for the progress UI.

---

## 6. Data mapping

| Legacy column | Destination | Notes |
|---|---|---|
| `data` (BLOB) | `QualifiedIdentity::from_bytes` → `qi.` everything | Includes **all key material**. |
| `status` (u8) | `qi.status = IdentityStatus::from_u8(..)` | The encoder **skips** `status`, so a decoded blob carries the default `Unknown`. Restoring it from the column is mandatory or every migrated identity reads back as "unknown, refresh required". |
| `alias` | `qi.alias = column` (unconditional) | The SQL column is authoritative; the blob's copy is stale. In v0.9.3 `set_identity_alias` updated **only** the column, and every loader decoded the blob then unconditionally overwrote `alias` with the column value. A rename or removal left the blob stale, so the column always won. Migrating with a blob-first fallback would resurrect a renamed-away alias or reverse a removal — so the column always wins here, including a NULL column clearing a stale blob alias. |
| `identity_type` | already inside `data` | Column is `format!("{:?}")` of the same value. `insert_local_qualified_identity` re-derives the wrapper's tag from `qi.identity_type`. Do not read the column. |
| `wallet` + `wallet_index` | `wallet_and_identity_id_info: Option<(WalletSeedHash, u32)>` | Both-or-neither (enforced by the legacy `CHECK`). |
| `network` | row filter `WHERE network IN (?1, ?2)` | Two-value filter via `mainnet_alias_for()` — a pre-v29 DB spells mainnet `dash`. |
| `is_local` | filter `= 1` | Non-local rows are observed-identity cache. Skip. |
| `info` | dropped | Never written by v0.9.3. |
| `top_up` join | already imported | `migrate_app_data` writes `det:top_ups:v1` under the same `DetScope::Identity`. Independent key; no interaction. |

`qi.network` is set by the read path (`decode_stored_identity`), not stored.
`qi.associated_wallets` / `qi.secret_access` / `qi.top_ups` are runtime wiring,
rehydrated on load. None of them need migrating.

---

## 7. Edge cases

| Case | Behaviour | Rationale |
|---|---|---|
| **Row already in the k/v store** | Skip the row wholesale (`skipped_existing`). The present record is never re-persisted, and legacy-only keys are **not** reconciled into it. **Known limitation** — see below. | Field *absence* cannot be told apart from a deliberate removal: an identity missing a key may have had it removed by the user ("Remove private key from DET", no tombstone), and a cleared alias persists as `None`; refilling from the stale legacy blob would resurrect either. A protected identity would additionally trip `encode_identity_blob_vault_first`'s `IdentityKeyProtectionDowngrade` guard if a plaintext legacy key were merged in, failing the whole pass. Provenance the model does not carry would be needed to reconcile safely, so the importer stays conservative and skips. |
| **Linked wallet failed to migrate / absent** | Import the identity anyway, **preserving `wallet_hash` + `wallet_index` verbatim**. Log at `warn`. | The link is what `load_local_qualified_identities_for_wallet` uses to re-attach the identity when the wallet is later restored or unlocked. Nulling it would orphan the identity permanently. A protected (locked) wallet is the *normal* case here — its seed is in the vault but it is not in `ctx.wallets` until the user unlocks. |
| **`data IS NULL` or `is_local = 0`** | Skip silently, do not count as failure. | Not user identity data; v0.9.3's own readers ignore these rows. |
| **`data` fails to decode** | Count `unreadable`, log at `warn` with the identity id, **withhold the sentinel**, publish a terminal warning state. Do **not** fail the pass. | Diverges from the scheduled-vote policy (which writes the sentinel anyway) on purpose: a corrupt vote row is unrecoverable, but an undecodable identity blob may be a *decoder* defect (bincode drift, §4.1) that a later build fixes. Withholding the sentinel keeps the retry door open at the cost of one cheap re-attempt per launch; the skip-if-present rule makes the retry a no-op for everything that already landed. Failing the pass instead would be wrong — it would gate nothing useful and shout at a user who cannot act. |
| **`id` not 32 bytes / `wallet` not 32 bytes** | Count `unreadable`, same as above. | Corruption. |
| **Identity type the fixture doesn't cover (`User`, `Evonode`)** | Handled with no extra code — the type lives in the blob and round-trips. | See §2.3. The *test* must cover it; the *code* need not branch on it. |
| **Second launch** | Sentinel short-circuits; nothing is read, nothing is written. | Mirrors `drain_wallets`. |
| **Legacy rows** | **Never deleted.** | Repo-wide migration rule: a migration that deletes its source can never be retried. |

### Known limitation — a partially-loaded identity strands its legacy-only keys

If a v0.9.3 identity was already loaded into the modern store *before* migration
but only partially — the canonical case is a masternode brought in from just its
ProTxHash, which persists a **bare** record (no private keys) plus, possibly,
missing owner/voting/payout associations — the importer skips it and does **not**
backfill the keys still sitting in the legacy blob. Those keys become
inaccessible through the current UI: the record shows as present but keyless.

No data is destroyed. The legacy `data.db` is preserved verbatim (rows are never
deleted), so a future recovery flow can read those keys back. The conservative
skip is deliberate — as the edge-case rationale explains, field absence cannot be
distinguished from a deliberate user removal without provenance the model does
not carry, and merging a plaintext legacy key into a protected identity would
trip the vault-first downgrade guard. Rather than a heuristic that risks
resurrecting removed keys or failing the whole pass, the safe behaviour is to
skip and defer recovery to a dedicated, provenance-aware flow.

**Resolved.** The recovery flow this limitation defers to has since shipped:
`docs/ai-design/2026-07-28-legacy-identity-recovery/design.md`
([issue #889](https://github.com/dashpay/dash-evo-tool/issues/889)). It is an
interactive, opt-in, per-identity action reached from the node detail page and
the Key Info screen. It answers the provenance question by asking the only
party who holds it: detection lists what the legacy blob has and the modern
record does not, and the user's approved list is what the merge writes. The
merge is additive from the fresh modern record, so nothing present is replaced
or removed; a protected identity verifies its password first and every restored
key is sealed before the record is written, so the downgrade guard cannot trip.

The importer itself is unchanged — skip-if-present still stands, `data.db` is
still opened read-only and never modified, and nothing about the recovery flow
runs at migration or launch time.

---

## 8. Security

This step moves identity private keys. The repo rule (`CLAUDE.md`, DET Module
Placement Policy) is that all wallet/identity secret bytes enter and leave the
vault through the single `wallet_backend/secret_seam.rs` chokepoint.

**The design satisfies this by not writing any secret-handling code at all.**
`AppContext::insert_local_qualified_identity` →
`encode_identity_blob_vault_first` → `IdentityKeyView::store_all` →
`SecretSeam`. The importer's only contact with key material is the
`QualifiedIdentity` value it holds between `from_bytes` and `insert_…`, which is
the same handling every existing load path already performs.

Consequences the implementer must respect:

- **Do not** construct `StoredQualifiedIdentity` directly (it is private to
  `identity_db.rs` — keep it that way).
- **Do not** write `qi_bytes` from the legacy blob verbatim. It would persist
  `Clear` / `AlwaysClear` plaintext keys into `det-app.sqlite`, bypassing the
  vault — precisely the leak `encode_identity_blob_vault_first` exists to close.
  The eager load-path migration would eventually repair it, but "eventually" is
  not a security property.
- **Do not** log the blob, the decoded `qi`, or any `PrivateKeyData`. `Debug` is
  already redacting on `StoredQualifiedIdentity` and `PrivateKeyData`; log the
  hex identity id only.
- A `PrivateKeyData::Encrypted(_)` key (legacy per-identity password envelope)
  round-trips untouched — `has_plaintext_for_vault` ignores it, so it is neither
  vaulted nor decrypted. That is correct: the migration has no password.

---

## 9. Implementation tasks

Ordered; each is independently reviewable. **All six shipped** — see the status
header.

- **T-ID-01 — `database/legacy_import.rs`: `read_identities`.**
  `pub(crate) fn read_identities(conn: &Connection, network: Network) -> rusqlite::Result<LegacyIdentities>`
  returning `{ identities: Vec<LegacyIdentityRow>, unreadable: u32 }` where
  `LegacyIdentityRow { id: [u8; 32], qi: QualifiedIdentity, wallet: Option<([u8; 32], u32)> }`.
  There is **no separate `status` field**: the reader restores `status` (and
  `alias`) from their columns straight onto `qi` before the row is yielded, so the
  caller receives one already-correct `QualifiedIdentity` and cannot forget to
  apply them (the blob encodes neither — see §6).
  SQL: `SELECT id, data, status, wallet, wallet_index, alias FROM identity WHERE is_local = 1 AND data IS NOT NULL AND network IN (?1, ?2)`,
  params `(network.to_string(), mainnet_alias_for(network))`. `alias` is selected
  because the column — not the blob's stale copy — is authoritative (§6). Missing
  table ⇒ empty. Per-row decode failure ⇒ `unreadable += 1`, warn, continue; a
  wrong SQLite storage class costs its own row, not the whole read. Mirrors
  `read_scheduled_votes` exactly.
  *Depends on:* nothing.

- **T-ID-02 — `MigrationStep::Identities`** in `context/migration_status.rs`,
  plus its progress-UI label.
  *Depends on:* nothing.

- **T-ID-03 — `finish_unwire::migrate_identities`.** Sentinel
  `identities_sentinel_key_for(network)` → `det:migration:identities:<net>:v1`.
  Pure body `migrate_identities_from_conn(conn, network, wallet_known, is_present, insert) -> Result<IdentityMigrationOutcome, MigrationError>`
  with closure seams (matching `migrate_app_data_from_conn`) so it unit-tests
  without an `AppContext`. `is_present` is the skip-if-already-imported check; a
  present identity is skipped wholesale (see the §7 known limitation). Counters:
  `imported`, `skipped_existing`, `unreadable`. Sentinel written **iff
  `unreadable == 0`**.
  *Depends on:* T-ID-01, T-ID-02.

- **T-ID-04 — Wire into `run()`** after `drain_wallets`, before the terminal
  state. Add `MigrationError::IdentityImportFailed { … }` for hard k/v-write
  failures and a `MigrationState::SucceededWithUnreadableIdentities { count }`
  terminal variant (sibling of the existing votes variant), or fold both counts
  into one warning state — implementer's call, but the user must be told *which*
  domain failed, since the remedies differ (re-schedule a vote vs. re-import a key).
  *Depends on:* T-ID-03.

- **T-ID-05 — Detection gate.** Add `"identity"` to `LEGACY_TABLES`. Note this
  gate belongs to `drain_wallets`; `migrate_identities` needs its own cheap
  "table has rows" probe so an identity-only install (a masternode voter with no
  HD wallet) is not skipped.
  *Depends on:* T-ID-03.

- **T-ID-06 — Golden-blob test (the bincode contract).** A checked-in hex
  constant: a `QualifiedIdentity::to_bytes()` produced by the **real v0.9.3
  binary** (see §11), asserted to decode on HEAD into the expected identity, keys
  and type. This is the only test that proves §4.1's one open row. Small, permanent,
  zero build cost after generation.
  *Depends on:* the Part-2 generator (§11).

---

## 10. Test plan (`v093_upgrade.rs`)

Extend the existing fixture; keep its mutation-tested shape.

**Fixture changes** — replace the placeholder `data = vec![0u8; 16]` blob (junk,
decodes to nothing) with real blobs, and widen coverage past masternode-only:

| Row | Type | `is_local` | Wallet link | Key shape | Locks |
|---|---|---|---|---|---|
| A | `Masternode` | 1 | unprotected wallet, idx 0 | `Clear` owner + `Clear` voting | the reported bug |
| B | `User` | 1 | unprotected wallet, idx 1 | `AtWalletDerivationPath` | non-masternode variant; wallet-derived key path |
| C | `Evonode` | 1 | **none** (`NULL`/`NULL`) | `Clear` | wallet-less identity (masternode loaded by ProTxHash) |
| D | `User` | **0** | — | — | observed-identity cache row must be skipped |
| E | `User` | 1 | protected wallet | `AtWalletDerivationPath` | identity on a **locked** wallet still imports, link preserved |
| F | any | 1 | — | `data = NULL` | null-blob row must be skipped, not counted as failure |

**Assertions in `v093_install_upgrades_…`** (replacing the current
"the identity row survives the ladder" block, which asserts the *precondition*
this design consumes — keep it, then add the outcome):

1. `ctx.load_local_qualified_identities()` returns exactly A, B, C, E — not D, not F.
2. A's `alias == "my-masternode"`, `identity_type == Masternode`,
   `wallet_index == Some(0)`, `status` equals the legacy column value **(not
   `Unknown`)** — the status-restore contract from §6.
3. A's `masternode_key_presence()` reports `owner` and `voting` — the user's keys
   came across, which is the whole point.
4. `ctx.load_local_masternode_identities()` returns A and C;
   `load_local_user_identities()` returns B and E — the type-filtered views the
   Masternodes and Identities screens actually call.
5. **No plaintext key on disk**: read the raw `det:identity:v1` value for A and
   assert its decoded `private_keys` contain no `Clear`/`AlwaysClear` — every key
   is `InVault`. This is the §8 security contract, and it must be asserted from
   the *stored bytes*, not the in-memory `qi`.
6. The vault holds A's keys: `IdentityKeyView::new(secret_store, A).scheme(target, key_id)` is not `Absent`.
7. E imports with `wallet_hash == Some(protected_seed_hash)` even though the
   protected wallet is locked and absent from `ctx.wallets`.
8. Top-up history still resolves for A — i.e. the identity blob and the
   already-migrated `det:top_ups:v1` entry share a scope without colliding.

**Assertions in `second_launch_after_a_v093_upgrade_changes_nothing`:**

9. Identity count is unchanged; `run()` returns `false`.
10. The legacy `identity` rows are still in `data.db` (count unchanged).

**Assertions in `a_retry_after_an_unreadable_identity_preserves_user_edits`:**

11. Edit A's alias post-migration (`ctx.set_identity_alias`), re-run
    `finish_unwire::run` ⇒ the alias survives. This is the skip-if-present rule
    (§7): an identity already in the store is skipped wholesale — never
    re-persisted with the stale legacy blob. It must go **RED** against a naive
    implementation that re-inserts unconditionally.

    This assertion lives on the **retry** path, not the clean second launch, because
    only the retry path actually reaches the check. A clean upgrade writes the
    identity sentinel, and on the next launch that sentinel short-circuits the pass
    before any row is examined — so `second_launch_after_a_v093_upgrade_changes_nothing`
    would pass even against an importer with no skip-if-present rule at all, proving
    nothing. Withholding the sentinel is what forces the re-run: the test seeds one
    undecodable blob (`unreadable > 0` ⇒ sentinel not written), so the following
    launch re-imports over identities that already landed — exactly the case where an
    unconditional INSERT-OR-REPLACE would silently overwrite the user's rename.

**Negative test (own `#[test]`, on `migrate_identities_from_conn`):** a row whose
`data` is garbage ⇒ `unreadable == 1`, the readable rows still import, and the
sentinel is **not** written.

---

## 11. Part 2 — the bincode / standalone-crate question

> **Settled.** This was a feasibility spike run during design; its verdict (§11.3)
> was adopted. The golden hex blob it recommended shipped as **T-ID-06** —
> `V093_MASTERNODE_BLOB_HEX` in `src/backend_task/migration/v093_upgrade.rs`, whose
> comment documents how to regenerate it. The section is kept for that rationale and
> for the regeneration recipe; the question itself is closed.

### 11.1 Is bincode really the only blocker?

**Yes.** Verified empirically. A standalone crate (`/data/tmp/v093-fixture-probe`,
own `Cargo.lock`, not a workspace member) depending only on

```toml
dash-evo-tool  = { git = "https://github.com/dashpay/dash-evo-tool", tag = "v0.9.3" }
rusqlite       = "0.37.0"
libsqlite3-sys = { version = "0.35.0", features = ["bundled"] }
```

resolves cleanly: 928 packages, `bincode 2.0.0-rc.3` + `bincode_derive 2.0.0-rc.3`
selected with no conflict. Outside the workspace there is no unification pressure,
so the rc.3-vs-2.0.1 clash simply does not arise. The only other snag is a
`links = "sqlite3"` collision if the scratch crate pins a different `rusqlite`
major than v0.9.3's — matching v0.9.3's `rusqlite 0.37` / `libsqlite3-sys 0.35`
resolves it. Nothing else objects.

It also **builds**: `cargo check --lib -p dash-evo-tool` against that lockfile
finishes green in 5m47s (warm shared cargo cache; a fully cold one is longer).

### 11.2 Can v0.9.3's write paths be driven from `pub` API?

Partly, and the parts that matter are reachable:

- ✅ `Database::new(&path)` + `Database::initialize(&path)` — `pub`, no
  `AppContext`. Produces the genuine v0.9.3 DDL and `database_version = 11`.
- ✅ `QualifiedIdentity { … }.to_bytes()` — `pub`, no `AppContext`. **This is the
  only thing genuinely worth extracting**: it is the one artefact the current
  fixture cannot honestly forge, and the one that closes §4.1.
- ✅ `model::wallet::encryption::encrypt_message` — `pub`. (Already used verbatim
  by the current fixture from HEAD's own code, and byte-identical.)
- ❌ `Database::insert_local_qualified_identity(&self, qi, wallet_info, app_context: &AppContext)`
  takes an `AppContext`, and v0.9.3's `AppContext::new` loads a `Config` from the
  environment, builds an SDK, loads system data contracts and spawns a
  `TaskManager`. Driving it is possible but heavy and fragile.

The `AppContext` requirement is **avoidable and should be avoided**: the generator
does not need v0.9.3's row-writing SQL, only its *schema* and its *blob encoder*.
Rows go in with plain `rusqlite` — which is exactly what `v093_upgrade.rs` already
does, and which is already verified against `git show v0.9.3:`.

### 11.3 Verdict — **qualified yes, scoped down**

**Do not** build a fixture-regenerating tool that produces a whole v0.9.3
`.sqlite` file, checked in or regenerated on demand. That is the rabbit hole:
the existing hand-rolled fixture is already source-verified line-by-line against
`git show v0.9.3:src/database/`, it reads clearly in the test, and a binary
`.sqlite` blob in the repo is opaque, unreviewable, and rots. It would replace a
good artefact with a worse one.

**Do** build a one-shot, throwaway generator whose only output is a **golden hex
blob**: a real v0.9.3 `QualifiedIdentity::to_bytes()` for a masternode identity
with owner + voting `Clear` keys, printed as hex, pasted into `v093_upgrade.rs`
as a `const` and asserted to decode on HEAD (**T-ID-06**). That converts §4.1's
one unverified row — "bincode rc.3 and 2.0.1 agree on the wire format" — from an
assumption into a test, which is the single highest-value thing this whole
investigation can buy. The generator itself is scratch; it is never checked in,
never built in CI, and never maintained. Document how to regenerate it in a
comment above the constant.

**Effort:**

| | |
|---|---|
| Scratch crate + `main.rs` (construct `QualifiedIdentity`, print hex) | ~20 min |
| Build of the v0.9.3 dep graph (928 crates) — **measured**, warm cache | ~6 min wall clock, unattended |
| Paste constant + write the decode assertion | ~15 min |
| **Total attended** | **< 1 hour** |

The build is long but hands-off, one-time, and off the critical path. The
artefact it produces is 3 lines in a test file with no ongoing cost. That is a
good trade. The full-`.sqlite`-fixture version of the same idea is not.

---

## 12. Design-review findings (historical)

> **Closed.** These are the defects and gaps the design investigation surfaced
> *before* implementation, kept as the record of why the shipped design looks the
> way it does. Every row below was addressed by the design in §§5–10 and shipped in
> PR #885. Nothing here is an open question or a live defect list.

| Severity | Count | Items |
|---|---|---|
| **Critical** | 1 | Legacy identities (and all their key material) are silently dropped on the v0.9.3 → v1.0 upgrade. |
| **High** | 2 | Reusing the `finish_unwire` sentinel would strand every install that already ran the drain. Unconditional re-insert on retry would overwrite user edits with stale legacy blobs. |
| **Medium** | 3 | The `identity` `status` column is not in the bincode blob and would silently read back as `Unknown`. `identity` is absent from `LEGACY_TABLES`, so an identity-only install never trips detection. The rc.3-vs-2.0.1 bincode wire contract is unverified. |
| **Low** | 2 | The 2026-05-28 "version-byte contract" note is stale (destination moved to DET k/v). The test fixture's masternode-only, junk-blob identity row is not representative of the real column shape. |
