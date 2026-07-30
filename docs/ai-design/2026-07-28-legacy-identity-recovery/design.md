# Legacy identity recovery flow (issue #889)

**Status:** implemented. The proposal below is the design as written; §10 records
where the implementation deliberately departs from it, and is authoritative
wherever the two disagree.
**Scope:** exactly GitHub issue #889: an explicit, opt-in, per-identity recovery
action that reads the preserved legacy `data.db` and restores private keys and
role associations stranded by the migration's skip-if-present rule.
**Base:** `v1.0-dev` @ `3e05d5f30`.
**Prior art consumed:** `docs/ai-design/2026-07-13-legacy-identity-migration/design.md`
(§7 known limitation), `src/backend_task/migration/finish_unwire.rs` (the shipped
importer), commit `08242ecd` (the reverted reconcile — read in full, diff and
tests), and the shipped `IdentityLoadMode::MergeIntoExisting` /
`ProtectIdentityKeys` machinery on `v1.0-dev`.

**Explicit non-goals:** identity unload/removal work; re-import of identities
that are *absent* from the modern store (unreadable-blob rows counted by
`UnreadableIdentitiesWarning` — a different case, noted in §9 as a natural
extension); wallet-link restoration; any migration-time or launch-time
automation.

---

## 1. Problem restatement

The v0.9.3 → v1.0 cold-start migration (`migrate_identities`, PR #885) imports
legacy `identity` rows skip-if-present: an identity id already in the modern
`det:identity:v1` store is never re-persisted, wholesale. That rule is correct —
three reconcile heuristics (unconditional merge, empty-key-map gate, the
bare-only reconcile reverted in `08242ecd`) each failed review because they
tried to infer *user intent* from *state shape*:

- Field absence is not evidence of anything. "Remove private key from DET"
  leaves no tombstone; a cleared alias persists as `None`; a record with an
  empty key map may be a ProTxHash-only bare load **or** an identity whose only
  key the user deliberately removed. The model carries no provenance to tell
  these apart, and no gate on record shape ever will.
- A plaintext legacy key merged into a password-protected (Tier-2) identity
  trips `encode_identity_blob_vault_first`'s fail-closed
  `IdentityKeyProtectionDowngrade` guard and fails the whole pass.
- Withholding the migration sentinel to "retry later" re-fires an unactionable
  warning every launch and, worse, resurrects identities the user has since
  deleted.

The consequence (design doc §7, the tracked limitation): a v0.9.3 identity that
was already **partially** loaded into the modern store before the upgrade — a
masternode brought in bare from its ProTxHash, or holding one of its
voting/owner/payout keys while the legacy blob holds the others — keeps its
remaining keys stranded in `data.db`. Nothing is destroyed (migration never
deletes source rows), but no current flow can reach them: the load form rejects
the duplicate ProTxHash (`RejectIfExists` → `DuplicateProTxHash`), the per-key
screen requires re-typing a WIF the user may no longer hold, and adding a
plaintext key to a protected identity trips the downgrade guard.

### What "genuinely missing" must mean operationally

Not "the field is `None`" — that is precisely the ambiguity that sank three
heuristics. This design decomposes it into two predicates, **neither of which
requires provenance**, plus one decision that only the user can make:

1. **Candidate** (computable): a legacy item not currently present in the
   modern record.
   - A key is identified by `(PrivateKeyTarget, KeyID)` — the exact map key of
     `KeyStorage::private_keys`. Candidate ⇔ the modern map has no entry at
     that key **and** the legacy entry carries recoverable material (§3.2).
   - A role association (`associated_voter_identity`,
     `associated_operator_identity`, `associated_owner_key_id`) is a candidate
     ⇔ modern is `None` and legacy is `Some`.
2. **Eligible to write** (structural): the merge is *additive-only*. Output is
   constructed by starting from the freshly re-read modern record and
   inserting candidate entries; nothing is ever removed or replaced; modern
   wins every collision by construction (§3.3).
3. **Wanted** (the user's call): whether a candidate should actually be
   restored. The absence-vs-removal ambiguity is not solved — it is
   **transferred to the only party who holds the provenance**. The UI shows
   exactly which items would be restored, per identity, and the approved set
   travels in the task payload as an item-level allowlist. A user who
   deliberately removed a key declines; one who approves has made a decision
   equivalent to re-typing the WIF, which was always legitimate.

"Genuinely missing" = candidate ∧ user-approved, applied additively. No shape
heuristic, no automation, no silent path.

One further distinction the detection must preserve (a known trap): a **bare
but valid** modern record (no keys — the expected partial-load shape this issue
exists for) is an eligible recovery target; a record or legacy row that fails
to **decode** is corrupt and takes a typed-error path (§3.5), never a "treat as
empty and merge anyway" path.

---

## 2. Entry point & UX flow

### 2.1 Where recovery is surfaced

Passive, contextual, per-identity — never a launch-time banner, never a nag.
Two surfaces, one shared component:

- **Masternode / evonode detail screen** (`ui/masternodes/detail_screen.rs`) —
  the canonical case. The screen already renders a `render_missing_voter`
  warning with an "Add voting key" WIF prompt; the recovery section renders in
  the same key-management area. When the candidate set includes the voting
  key, restore-from-backup becomes the primary remedy and re-typing the WIF the
  fallback — which is the user story verbatim ("without having to re-enter
  WIFs I no longer have on hand").
- **Key Info screen** (`ui/identities/keys/key_info_screen.rs`), reached from
  the Identities screen for `User` identities and via "Manage keys" for nodes —
  the issue's suggested location, covering the non-masternode partial-load
  variant.

Gating, cheap to strict:

1. The legacy `identity` table holds a row for this network
   (`AppContext::has_legacy_identities`) — fresh installs never see any of
   this. One `SELECT EXISTS` per context, cached thereafter. Not a
   file-existence check: a fresh install creates its own empty `data.db`, so
   the file is always there (§10.11).
2. On screen arrival (`refresh_on_arrival`), the screen dispatches
   `IdentityTask::CheckLegacyRecovery { identity_id }` — a backend task, since
   detection reads `data.db`. Result cached in screen state; no per-frame I/O.
3. The section renders only when the returned candidate set is non-empty.

Because detection is a pure function of (modern record, legacy row), the
section **self-extinguishes**: after a successful merge the candidate set is
empty and the affordance disappears. No dismissal state is required for v1; a
per-identity `det:legacy_recovery_dismissed:v1` key (own key per identity,
`DetScope::Identity` — never a shared collection) is a compatible follow-up if
users ask for "don't offer again".

### 2.2 What the user sees and confirms

The section copy is written for the Everyday User persona (no "migration",
"blob", "vault"):

> *Some keys for this identity from your previous Dash Evo Tool version
> haven't been brought across.*

followed by the item list in user terms — "Owner key", "Voting key", "Payout
key", "Voting identity link" (labels derived from `Purpose` / association kind;
key bytes and WIFs are never displayed, and never logged) — and a **Restore
keys…** button. The list *is* the preview; pressing Restore is the explicit
per-identity decision, and the listed items travel with the task as the
approved allowlist (§3.4). Items that cannot be restored by this flow (§3.2's
legacy `Encrypted` keys) are listed separately with an explanation and are
never part of the allowlist.

### 2.3 Protected-identity password flow

Reuses the shipped verify-then-seal machinery end to end — no new prompt UI,
no new crypto:

1. The backend task detects Tier-2 via
   `protected_identity_verify_scope(&modern)` — `Some(scope)` ⇔ the identity
   has at least one vault key with `SecretScheme::Protected`. (A bare identity
   has no keys, hence no protected keys, hence is never gated — the gate
   applies exactly when it must.)
2. `secret_access.verify_identity_object_password(&scope).await` fires the
   standard secret-prompt modal, verifies the typed password by unsealing an
   existing protected key of the same identity (one-password invariant
   preserved), re-asks on wrong password, and returns a
   `VerifiedIdentityPassword`.
   - **Cancel** → the prompt's cancel error propagates; the task aborts with
     **zero writes** (the prompt runs before any mutation).
   - **Wrong password** → the prompt re-asks (standard behaviour); the task
     never proceeds on an unverified password.
   - **Headless/MCP** → `SecretPromptUnavailable`, fail closed, zero writes.
3. Before any vault write, the Tier-2 branch also runs the existing
   `reject_resident_identity_plaintext` guard (from
   `protect_identity_keys.rs`, widened to `pub(super)`): a modern record still
   carrying resident plaintext from an incomplete load-path vault migration
   fails fast with the existing typed errors and their established remedies,
   rather than half-sealing and then tripping the downgrade guard at persist.
4. Approved plaintext legacy keys are sealed Tier-2 under the verified
   password via the existing `seal_merged_plaintext_keys` (which flips each to
   `InVault` and writes through `SecretAccess::seal_new_identity_key_with_password`
   → `SecretSeam::put_secret_protected`) **before** the record is persisted —
   so `encode_identity_blob_vault_first` never sees plaintext on a protected
   identity.

### 2.4 Partial success

**All-or-nothing at the persistence level, by construction.** The flow performs
exactly one record write (`update_local_qualified_identity`); either the merged
record lands or the modern record is unchanged. There is no per-key password
dimension (one password per identity — the shipped invariant), so "some keys
succeed, one is blocked by the password" cannot arise. Two nuances, both
already the codebase's accepted semantics:

- A crash between Tier-2 vault seals and the record write leaves sealed vault
  entries at their correct labels with the record not yet referencing them —
  the same recoverable intermediate as the shipped merge-load path. A re-run
  is idempotent (same-label upsert) and completes the job. No partially-merged
  *record* is ever observable.
- Items excluded up front (legacy `Encrypted` keys, stale approvals per §3.4)
  are **reported outcomes, not failures**: the success result enumerates
  merged, skipped-stale, and unrecoverable items, and the UI says so plainly.

---

## 3. Merge algorithm

### 3.1 Inputs

Computed twice — once for the preview (`CheckLegacyRecovery`), once inside the
executing task — from the same pure function, so preview and execution cannot
drift:

- `modern: QualifiedIdentity` — fresh `get_local_qualified_identity(&id)?`;
  `None` ⇒ `IdentityNotFoundLocally` (a deleted identity is simply not
  eligible; recovery never resurrects a deleted record).
- `legacy: LegacyIdentityRow` — a **single-row** read of `data.db` (§4.2),
  through the same per-row decode, id-consistency, and network-filter rules as
  the shipped `read_identities` (including the row-id-vs-blob-id divergence
  check and the `dash`/`mainnet` alias filter). Missing table, missing row,
  `is_local = 0`, or `data IS NULL` ⇒ "nothing to recover"; a decode failure ⇒
  typed error (§3.5), never treated as empty.

### 3.2 Candidate computation — `compute_recovery_plan(modern, legacy) -> RecoveryPlan`

Pure, in `model/legacy_recovery.rs`. For keys, per legacy entry
`((target, key_id), (public_key, key_data))`:

| Legacy `PrivateKeyData` | Modern has `(target, key_id)`? | Verdict |
|---|---|---|
| `Clear` / `AlwaysClear` | no | **candidate** (raw material present) |
| `AtWalletDerivationPath` | no | **candidate** (re-derivable reference; carries no plaintext, lands verbatim) |
| `Encrypted` (legacy per-key envelope, decode-only) | no | **excluded**, reason `LegacyEncryptedFormat` — merging ciphertext would poison every re-save and the protect opt-in (`IdentityKeyProtectionLegacyFormat`). Listed for the user with the existing remedy (re-load the identity with the key). **Confirmed unreachable in practice** (coordinator follow-up, 2026-07-28): full git history from this variant's introduction (`a5db23e6c8`, Nov 2024) through the real `v0.9.3` release tag shows no encrypt function ever constructing one — every consumer, then and now, treats it as a stub ("please enter password" / today's `LegacyEncryptedFormat` rejection). Distinct from the wallet's imported-single-key legacy password encryption (`single_key_wallet` table), which is real AES-GCM+Argon2id and already has a full decrypt→re-encrypt path (`backend_task/migration/single_key_restore.rs`, T-SK-03) — unrelated to this issue. User confirmed (2026-07-28): no known real-world instances; exclusion stands as designed, no decrypt path to build. |
| `InVault` | no | **excluded**, reason `NoMaterial` — impossible in a genuine v0.9.3 blob (the variant postdates it); tolerated defensively, never merged. |
| any | yes | **not a candidate** — modern wins, unconditionally. |

For associations: `associated_voter_identity`, `associated_operator_identity`,
`associated_owner_key_id` are each a candidate ⇔ modern `None` ∧ legacy
`Some`.

**Plan invariant (enforced in model code, unit-tested):** any candidate key
with `target == PrivateKeyOnVoterIdentity` groups the `VoterAssociation`
candidate with it — a voting key without its voter identity link is
unpresentable (`masternode_key_presence` and the signing paths key off the
association). The preview shows them as one item; approval of one is approval
of both.

**Never candidates, ever:** `alias` (user-editable; the exact resurrection
hazard from `08242ecd`), `identity` (the dpp identity — modern is the newer
on-chain revision), `dpns_names`, `status`, wallet link
(`wallet_hash`/`wallet_index` — preserved from the modern record by the
`update_local_qualified_identity` writer's existing contract), and all runtime
wiring (`associated_wallets`, `secret_access`, `top_ups`).

### 3.3 Application — `apply_recovery_plan(modern, legacy, approved) -> AppliedRecovery`

Pure, same module. Starting from a clone of the **fresh modern record**:

```
for each approved item, in plan order:
    Key(target, key_id):
        merged.private_keys.private_keys
              .entry((target, key_id))
              .or_insert(legacy entry)          // modern wins even intra-task
    VoterAssociation / OperatorAssociation:
        if merged.<assoc>.is_none() { merged.<assoc> = legacy.<assoc> }
    OwnerKeyAssociation:
        if merged.associated_owner_key_id.is_none() { … = legacy value }
```

Properties that hold **by construction**, not by discipline:

- The output key map is a superset of the input modern key map, and every
  modern entry survives byte-identical (`or_insert` cannot replace).
- Nothing outside `private_keys` and the three association fields is touched —
  the function has no code path that writes any other field.
- An approved item that is no longer a candidate at execution time is a
  no-op, counted `skipped_stale` (§3.4).
- The write path can never receive a *partial inventory as if it were
  complete* (the general provenance trap): the persisted argument is always
  `modern ∪ approved-candidates`, a superset of the current record — a partial
  legacy blob structurally cannot express a removal.

### 3.4 The approved-allowlist rule (TOCTOU containment)

The executing task merges exactly **recomputed-candidates ∩ approved**:

- Approved at preview, still missing → merged (the normal case).
- Approved at preview, no longer missing (user re-added it via the load form
  meanwhile, or a prior recovery run already landed it) → skipped, reported
  `skipped_stale`. Idempotent re-runs are this rule.
- Missing at execution but **not** approved (user removed a key between
  preview and execution) → **never merged** — the user never approved it. The
  outcome reports it so the UI re-offers a fresh preview.

"Nothing merges without an explicit user decision" is thereby literal at the
item level, not just the identity level.

### 3.5 Failure handling (typed, per repo rules)

New `TaskError` variants (no `String` payloads; `Display` written for the
Everyday User — what happened + what to do, details via `with_details`):

| Variant | When | User-facing direction |
|---|---|---|
| `LegacyIdentityUnreadable { identity_id }` | legacy row present but blob/columns fail decode | "The saved copy of this identity from the previous version could not be read. You can still add the key by entering it on the identity's key screen." |
| `LegacyRecoveryNothingApproved` | empty allowlist reaches the backend (UI bug / MCP misuse) | re-run the check and select items |
| existing `IdentityNotFoundLocally`, `IdentityLoadInProgress`, `SecretPromptUnavailable`, prompt-cancel, `IdentityKeyProtectionIncomplete` / `IdentityKeyProtectionLegacyFormat` | as today | existing copy |

A failure anywhere before the single record write leaves the modern record
untouched. `IdentityKeyProtectionDowngrade` is **unreachable** from this flow
(§3.6) — reaching it would be an implementation bug, and a test asserts the
branches that make it unreachable.

### 3.6 Why this cannot reproduce the three reverted failure modes

| Failure mode (from `08242ecd` and issue text) | Structural counter — not "more care" |
|---|---|
| **Unconditional merge** resurrects removals/renames | Apply is additive from the fresh modern base with `or_insert`; alias/status/identity/wallet-link have no write path in the apply function; and nothing executes without a per-item user allowlist in the task payload. Migration code is untouched — there is no automatic caller. |
| **Shape heuristics** (empty-key-map / bare-only gates) misread deliberate removals | There is no gate on record shape anywhere. Detection only *lists*; it never writes. The bare-vs-removed question is answered by the user, the sole holder of that provenance; declining is a first-class outcome. |
| **Protection-downgrade trip** fails the pass on Tier-2 identities | The flow branches on the *same predicate the guard evaluates* (`find_protected_identity_key_scope`, via `protected_identity_verify_scope`). Tier-2 branch: password verified up front, resident-plaintext preflight, then `seal_merged_plaintext_keys` marks every merged plaintext key `InVault` **before** persist — the guard's `has_plaintext_for_vault()` input is false. Tier-1 branch: no protected key exists by that same predicate, so the guard's other input is false. On both branches the guard's trigger condition is false by the branch condition itself. |
| *(fourth hazard from §7)* pending-sentinel re-fire nags every launch | No sentinel, no launch-time hook, no durable pending state at all. A passive on-screen section that recomputes from source data and disappears when empty. |

---

## 4. Data flow & module placement

Per the DET module placement policy; **no new secret-handling code is written
anywhere** — the flow composes existing chokepoint paths, the same argument
that carried the migration design's §8.

### 4.1 `model/legacy_recovery.rs` — new, pure

- `enum RecoveryItem { Key { target: PrivateKeyTarget, key_id: KeyID }, VoterAssociation, OperatorAssociation, OwnerKeyAssociation }`
  (+ a display descriptor carrying `Purpose` for UI labels — public data only).
- `enum ExclusionReason { LegacyEncryptedFormat, NoMaterial }`
- `struct RecoveryPlan { items: Vec<RecoveryItem>, excluded: Vec<(RecoveryItem, ExclusionReason)> }`
- `fn compute_recovery_plan(modern: &QualifiedIdentity, legacy: &QualifiedIdentity) -> RecoveryPlan`
- `fn apply_recovery_plan(modern: &QualifiedIdentity, legacy: QualifiedIdentity, approved: &[RecoveryItem]) -> AppliedRecovery`
  where `AppliedRecovery { merged: QualifiedIdentity, applied: Vec<RecoveryItem>, skipped_stale: Vec<RecoveryItem> }`.

No `AppContext`, no `Sdk`, no DB, no vault — the single source of truth for
"what counts as genuinely missing", unit-testable in isolation. UI and backend
both consume it; neither reimplements any part of it.

### 4.2 `database/legacy_import.rs` — one new reader

`pub(crate) fn read_identity_row(conn: &Connection, network: Network, id: &[u8; 32]) -> rusqlite::Result<LegacyIdentityLookup>`
with `enum LegacyIdentityLookup { Found(LegacyIdentityRow), Absent, Unreadable }`
— a single-row (`WHERE id = ?`) variant of `read_identities`, refactoring the
existing per-row decode (column classes, 32-byte checks, blob decode, row-id ≡
blob-id, status/alias column restoration) into a shared helper so the two
readers cannot diverge. Read-only by construction: the connection comes from
the existing `open_legacy_read_only` (`SQLITE_OPEN_READ_ONLY`; the private
copy in `finish_unwire.rs` is hoisted next to `database/mod.rs`'s existing
one). **No write to `data.db` exists anywhere in this design, and no new
DetKv key either** — the frozen-legacy-store constraint is satisfied by having
nothing to violate it with.

### 4.3 `backend_task/identity/recover_legacy_keys.rs` — new; the enforcement layer

Two `IdentityTask` variants:

- `CheckLegacyRecovery { identity_id }` → reads modern + legacy, returns
  `BackendTaskSuccessResult::LegacyRecoveryCandidates { identity_id, plan }`
  (descriptors only — no key bytes, nothing logged beyond the hex id).
- `RecoverLegacyIdentityData { identity_id, approved: Vec<RecoveryItem> }`:

```
claim   = begin_identity_load(identity_id, None)?          // excludes concurrent loads/merges
guard: migration_status().state().is_in_progress() ⇒ WalletStorageNotReady   // same rule as delete
modern  = get_local_qualified_identity(&id)?  else IdentityNotFoundLocally
legacy  = read_identity_row(open_legacy_read_only(db_file_path)?, network, id)
plan    = compute_recovery_plan(&modern, &legacy.qi)        // recomputed, never trusted from UI
password= match protected_identity_verify_scope(&modern)? {
              Some(scope) => { reject_resident_identity_plaintext(&modern.private_keys)?;
                               Some(verify_identity_object_password(&scope).await?) }
              None => None }
applied = apply_recovery_plan(&modern, legacy.qi, approved ∩ plan.items)
if applied.applied.is_empty() ⇒ Ok(LegacyRecoveryCompleted { nothing_to_recover })
if let Some(pw) = &password { seal_merged_plaintext_keys(&mut applied.merged, pw)? }  // Tier-2: InVault before persist
update_local_qualified_identity(&applied.merged)?           // ONE write; preserves wallet link
claim.loaded()
Ok(LegacyRecoveryCompleted { identity_id, applied, skipped_stale, excluded })
```

Entirely local — **no network fetch** (unlike `MergeIntoExisting`, which
re-fetches because the user is re-loading; here the modern record already
holds the newer on-chain identity, and legacy's older copy is never used).
Recovery therefore works offline and the claim is held for milliseconds.

### 4.4 Secret chokepoint accounting

Every secret byte moves through existing seams only:

| Path | Route |
|---|---|
| Tier-1 plaintext legacy key | `update_local_qualified_identity` → `encode_identity_blob_vault_first` → `IdentityKeyView::store_all` → `SecretSeam` (keyless vault; blob persists `InVault` placeholders only) |
| Tier-2 plaintext legacy key | `seal_merged_plaintext_keys` → `SecretAccess::seal_new_identity_key_with_password` → `SecretSeam::put_secret_protected` (Argon2id + XChaCha20-Poly1305, AAD-bound) |
| `AtWalletDerivationPath` | no secret bytes exist; the reference lands in the blob verbatim |

No logging of blobs, decoded identities, or `PrivateKeyData` at any level —
same rule as the migration (`Debug` is already redacting; log hex ids only).

### 4.5 UI

- `ui/components/legacy_recovery_section.rs` — new render-only component
  (component pattern: builder config, `show(ui) -> ComponentResponse`); takes
  the plan summary, emits the approved-items dispatch intent. It renders egui,
  so it is a component, not `ui/state/`.
- `ui/masternodes/detail_screen.rs` + `ui/identities/keys/key_info_screen.rs`
  — hold `Option<plan>` screen state, dispatch `CheckLegacyRecovery` on
  arrival (behind the legacy-rows gate of §2.1), render the section, handle both
  results in `display_task_result`, refresh on completion. Progress/success
  banners via the standard `BannerHandle` lifecycle; failure banners carry
  details via `with_details`.

### 4.6 Deployment

No new crate dependencies (verified: everything reuses shipped machinery — no
registry lookups needed), no schema change, no new DetKv key, no migration
ordering constraint. The feature is dormant on any install without a legacy
`data.db`. Ships in the normal binary for all platform targets; standard CI
gates apply. `docs/kv-keys.md` needs no change (nothing durable is added);
`docs/user-stories.md` gains one `[Implemented]` story on landing.

---

## 5. Idempotency & concurrency

**No persisted recovery state — that is the design, not an omission.**
Detection is a pure function of two existing stores; eligibility recomputes on
every screen arrival. Consequences:

- **Idempotent by recomputation:** after a successful merge the candidate set
  is empty; a re-run returns `NothingToRecover` and writes nothing; the UI
  affordance disappears on its own. Repeating recovery after a *partial*
  approval merges only the still-missing remainder. `data.db` is never
  modified, so recovery is repeatable indefinitely (AC-5).
- **Nothing to race:** the shared-collection DetKv hazard (get-then-put on a
  set key is not compare-and-swap) is avoided by having no such key. The only
  writes are the identity's own `det:identity:v1` under its own scope, plus
  the pre-existing idempotent `index_add_identity` no-op (the id is already
  indexed — recovery requires presence).
- **Per-identity exclusion:** the task holds the identity-load registry claim
  (`begin_identity_load`), the same mutex the load/merge paths use — a
  concurrent load, merge-load, or second recovery of the same identity gets
  `IdentityLoadInProgress`. The claim spans read → compute → seal → write.
- **Exclusion against every other writer:** the load claim covers only the
  paths that take it, which is neither the ordinary writers nor the protection
  tier migrations. The write section additionally holds
  `AppContext::identity_record_lock` — see §10.13.
- **TOCTOU across the preview gap** is contained by the allowlist-intersection
  rule (§3.4): re-additions become stale-skips, removals-after-preview are
  never merged (not approved), and the additive `or_insert` apply makes even a
  same-instant collision resolve modern-wins.
- **Migration interplay:** recovery refuses to run while a migration pass is
  in progress (same `is_in_progress` check the delete path uses). A deleted
  identity is ineligible (modern-absent), and the migration's own
  deletion-progress record is untouched — recovery neither reads nor writes
  migration state.

---

## 6. Test plan

TDD order: model tests first (several must go RED against a deliberately naive
merge before the real one lands), then backend integration on the offline
wired `AppContext` harness the protect tests already use, seeding legacy rows
with the existing `LegacyIdentityFixture` / `basic_legacy_identity_blob`
helpers. All tests isolated to temp dirs; no real user data.

### Model (`model/legacy_recovery.rs`)

| # | Scenario | Asserts |
|---|---|---|
| M1 | legacy-only `Clear` key | is a candidate |
| M2 | key present in both at same `(target, key_id)` | not a candidate; apply keeps the modern bytes (RED vs legacy-wins) |
| M3 | modern-only key | never in plan; survives apply byte-identical |
| M4 | each association, all four presence combinations | candidate only on modern-`None` ∧ legacy-`Some` |
| M5 | legacy `Encrypted` key | excluded, `LegacyEncryptedFormat`, never applied even if "approved" |
| M6 | voting key candidate | `VoterAssociation` grouped; applying the key applies the link |
| M7 | identical records | empty plan |
| M8 | bare modern record | full legacy key set + associations as candidates (the §1 bare-but-valid case) |
| M9 | apply superset property | for every fixture: output key map ⊇ modern's; alias/status/identity/wallet fields untouched (structural additive-only proof) |
| M10 | approved ∩ candidates rule | approved-but-present → `skipped_stale`; missing-but-unapproved → untouched (the removal-mid-flight case) |

### Backend (`recover_legacy_keys.rs`, offline `AppContext`)

| # | Scenario | Asserts |
|---|---|---|
| B1 | Tier-1 end-to-end: bare modern masternode + legacy owner/voting `Clear` keys | check reports candidates; recover merges; `masternode_key_presence` shows owner+voting; **stored `det:identity:v1` bytes contain no `Clear`/`AlwaysClear`** (asserted from stored bytes, §8-style); vault holds the keys; legacy rows still in `data.db` |
| B2 | Tier-2 with correct password | merged key `SecretScheme::Protected`, opens under the *same* identity password |
| B3 | Tier-2 wrong password (headless `NullSecretPrompt` variant too) | `SecretPromptUnavailable` / re-ask path; stored record byte-identical, zero vault writes |
| B4 | Tier-2 prompt cancel | typed cancel error, zero writes |
| B5 | Tier-2 with resident-plaintext modern key | fails fast with `IdentityKeyProtectionIncomplete` before any vault write |
| B6 | run recovery twice | second run `NothingToRecover`, store unchanged (byte compare) |
| B7 | modern record deleted first | `IdentityNotFoundLocally`; nothing recreated |
| B8 | undecodable legacy blob | `LegacyIdentityUnreadable`, zero writes |
| B9 | legacy row absent / `is_local=0` / NULL blob | check reports nothing-to-recover, not an error |
| B10 | concurrent load claim held | `IdentityLoadInProgress` |
| B11 | `User`-identity variant with one missing main-identity key | same flow, no masternode-specific assumptions |
| B12 | wallet link `Some` on modern, `None`-wallet legacy row | link preserved verbatim after recovery (writer contract) |

### UI (`tests/kittest`, light)

| # | Scenario |
|---|---|
| U1 | section absent when no `data.db` / empty plan; present with items labelled by purpose; disappears after a success result |

### AC traceability

M/B/U rows ↔ acceptance criteria: AC-1 → U1, B1(check); AC-2 → M2/M3/M9, B1;
AC-3 → B2–B5; AC-4 → M10, B-series (no test contains any automatic trigger —
and `finish_unwire` is untouched, its existing suite is the regression net);
AC-5 → B1 (legacy rows intact), B6.

---

## 7. Acceptance-criteria checklist

| Issue criterion | How the design satisfies it |
|---|---|
| Discoverable, opt-in recovery entry point on the Key Info / identity screen for a present-but-partial identity | §2.1: passive section on the masternode detail and Key Info screens, gated by an on-arrival backend detection task; renders only when recoverable candidates exist; self-extinguishes when none remain. |
| Reads the preserved legacy blob and merges only genuinely-missing keys and role associations, never overwriting anything currently held | §3: candidates are absence-keyed on `(PrivateKeyTarget, KeyID)` / `None` associations; apply is additive-only from the fresh modern base (`or_insert`), modern wins every collision by construction; alias/identity/status/wallet-link have no write path. |
| Protected identity: recovery gated behind the identity password; no plaintext write that trips `IdentityKeyProtectionDowngrade` | §2.3/§3.6: branch on the guard's own predicate; verify password up front via the shipped prompt (cancel/wrong/headless all fail closed with zero writes); seal merged keys Tier-2 (`InVault`) before the single persist — the guard's trigger is false on both branches. |
| Explicit per-identity decision; nothing reconciled silently at migration time | §2.2/§3.4: the previewed item list is the approved allowlist carried in the task payload; execution merges recomputed-candidates ∩ approved. `finish_unwire.rs` is not modified; skip-if-present stands. |
| No legacy source data deleted; recovery idempotent and safely repeatable | §4.2/§5: `data.db` opened `SQLITE_OPEN_READ_ONLY`, no write path exists; no durable recovery state; re-runs recompute to `NothingToRecover`; stale approvals skip. |

---

## 8. Implementation task decomposition

Ordered; each independently implementable and reviewable by a single developer.

| Task | Contents | Depends on |
|---|---|---|
| **T-889-01** | `model/legacy_recovery.rs`: `RecoveryItem`, `RecoveryPlan`, `compute_recovery_plan`, `apply_recovery_plan` + tests M1–M10 (M2/M9/M10 RED-first against a naive merge) | — |
| **T-889-02** | `database/legacy_import.rs`: `read_identity_row` + shared row-decode refactor with `read_identities`; hoist `open_legacy_read_only` | — |
| **T-889-03** | `backend_task/identity/recover_legacy_keys.rs`: both task variants, claim/guards/password flow, `TaskError` + `BackendTaskSuccessResult` variants; widen `reject_resident_identity_plaintext` to `pub(super)`; tests B1–B12 | 01, 02 |
| **T-889-04** | `ui/components/legacy_recovery_section.rs` + masternode `detail_screen.rs` wiring (detection dispatch, result handling, banners; restore-from-backup as the primary missing-voter remedy) | 03 |
| **T-889-05** | `ui/identities/keys/key_info_screen.rs` wiring for `User` identities (same component) | 03, 04 |
| **T-889-06** | kittest U1; `docs/user-stories.md` story; CHANGELOG; update design-record cross-reference in the migration doc §7 | 03–05 |

---

## 9. Future extensions (explicitly out of scope now)

- **Unreadable-row re-import**: `finish_unwire` comments already defer
  "recover undecodable rows after a decoder fix" to an explicit user gesture
  under this issue's umbrella. The `read_identity_row` + backend-task
  architecture is the natural host; it is a different eligibility class
  (modern-absent) and a different writer (`insert`, not `update`), so it is a
  separate design.
- **Per-identity dismissal** of the recovery call-out
  (`det:legacy_recovery_dismissed:v1`, own key per identity).
- **Wallet-link restoration** for identities whose modern record lost the
  link — interacts with funds flows and wallet presence; deliberately not
  bundled with key recovery.

---

## 10. Deviations from the proposal (as implemented)

Written after the code landed, and after the review round that followed. Where
this section and §1–§9 disagree, this section describes what shipped.

### 10.1 Detection is dispatched from the render latch, not `refresh_on_arrival`

§2.1 step 2 and §4.5 put the `CheckLegacyRecovery` dispatch on
`refresh_on_arrival`. Shipped: a dispatch-once latch inside the view's own
render (`ui/state/legacy_recovery.rs`, consumed by `detail_screen.rs` and
`key_info_screen.rs`). The masternode list screen rebuilds the detail view on
every task result, so an arrival-only hook would miss those rebuilds, while a
naive re-dispatch on every render would loop (result → rebuild → re-check →
result). The preview result is routed *into* the open view instead of
triggering a rebuild; a restore result does rebuild, which re-arms the check.

Arrival still matters, but for a different reason: a restore run from the Key
Info screen the node page pushed never reaches the node page, so
`refresh_on_arrival` re-reads the node and re-arms its check.

### 10.2 Progress is inline, not a banner

§4.5 said progress and success both go through the standard `BannerHandle`
lifecycle. Shipped: progress replaces the button inside the section with a
spinner, which also makes a double dispatch impossible; only the success banner
is global.

### 10.3 `ui/state/legacy_recovery.rs` holds the cross-screen fetch state

Neither §4.5 nor §8 lists it. The per-identity fetch state is a six-state
machine that renders nothing, so the module-placement policy's render/no-render
discriminator puts it in `ui/state/`, not `ui/components/`. It is also what
lets the Key Info surface reuse the masternode surface's work wholesale.
`ui/masternodes/list_screen.rs` is likewise part of the change: it carries the
result-routing rule §10.1 describes.

### 10.4 The Key Info offer is not gated to `User` identities

§2.1 and §8 T-889-05 frame that surface as the `User`-identity variant.
Shipped: the offer is identity-scoped and correct on any type, so a node
operator who reaches Key Info through "Manage keys" sees the same
self-extinguishing offer.

### 10.5 Candidates must still correspond to the identity

§3.2 admits a candidate on absence alone. Shipped: a key must also still
*correspond* to the identity — its saved private half derives the public half
it is stored with, that public half is still a live (not retired) key of the
identity the target names, and an `AtWalletDerivationPath` reference names a
wallet this install holds. The voter-identity link is a candidate only
alongside a voting key the node will actually hold.

The reason is §3.2's own blind spot: `masternode_key_presence` reports a role as
held from the record alone, so restoring a key the node rotated away from — or
a voter link with no key behind it — flips the role to "present", retires both
the recovery offer and the missing-voter remedy, and surfaces later as a
rejected transaction. Failing candidates go to `RecoveryPlan::excluded` with
their own reasons (`KeyNoLongerOnIdentity`, `VoterLinkWithoutVotingKey`), the
same treatment §3.2 already gave the legacy `Encrypted` format. This is the
check the manual "type the WIF" path (`verify_voting_key_exists_on_identity` and
friends) has always enforced; restoring should not be a way around it.

### 10.6 The migration guard is not held across the password prompt

§4.3's sketch holds `migration_run` for the whole task, prompt included, on the
grounds that the delete path takes the same guard. The comparison does not
hold: the delete path is fully synchronous and holds it for microseconds. As
written, an open Tier-2 recovery prompt made deleting an *unrelated* identity
fail with `WalletStorageNotReady` and could park `finish_unwire`'s awaiting
acquire indefinitely — both reproduced.

Shipped: an unguarded preflight (read, dry-run the merge, decide whether a
password is needed, prompt) followed by one fully synchronous critical section
that re-acquires the guard, re-checks the migration state, re-reads the record,
merges again and writes. §5's per-identity `begin_identity_load` claim still
spans the whole flow.

Re-reading also turns two properties from side effects into code. A concurrent
writer that lands during the prompt — refresh, DPNS registration, transfer,
none of which take the claim — is no longer reverted by the write of a
pre-prompt snapshot, and a delete that lands during the prompt surfaces as
`IdentityNotFoundLocally` instead of being undone by the upsert. §3.6's
downgrade-guard predicate is evaluated over the *merged* record, the one the
encoder sees, rather than over the pre-merge record; an identity that gains
password protection during the prompt therefore stops with the typed
`LegacyRecoveryIdentityChanged` rather than sealing under a password for a
state that no longer exists.

### 10.7 §7 row 4's "`finish_unwire.rs` is not modified"

The importer's *behaviour* is unchanged and its skip-if-present rule stands,
which is what that row asserts. The file itself is touched: its private
`open_legacy_read_only` was hoisted into `database/`, as §4.2 says. Mechanical,
and covered by the importer's existing suite.

### 10.8 Item labels live in the UI, not the model

§4.1 gave `RecoveryItemDescriptor` a display label. Shipped: the model carries
`(target, key_id, purpose)` and nothing else; the UI names items through the
shared `role_label_and_tip` vocabulary that already labels the "Manage keys"
buttons, and tells two rows in one role apart by key id. Two mappings meant two
names for one key, six lines apart, on the same screen.

### 10.9 Outcome buckets are disjoint

§3.4 left `skipped_stale` to mean "approved but no longer a candidate", which
swept in items that were never restorable at all. Shipped: `AppliedRecovery`
reports `excluded` alongside `applied` and `skipped_stale`, and the three are
disjoint — "cannot be restored" and "already back in place" are opposite
answers, and a caller reading the outcome has to be able to tell them apart.

### 10.10 Only the modern record may vouch for a key

§10.5's correspondence check resolved the identity a voter/operator-target key
belongs to as "the modern link, or the legacy one if there is no modern link".
That fallback made the check circular in exactly the case it exists for: the
legacy file supplied both the key and the identity snapshot that publishes it,
so any self-consistent pair passed — including a voting key the chain retired
months before the upgrade, which that snapshot still shows as live.

Shipped: [`reference_identity`] reads the modern record only. A key on a voter
or operator identity the modern record does not link to has no admissible
witness and is excluded as `LinkedIdentityUnverified`, with the same manual
remedy the other exclusions carry ("load this identity again and enter the
key").

Fetching the linked identity from Platform instead was rejected on two grounds.
It would put the network in the middle of an offline, read-only preview that
every screen arrival dispatches; and it would not establish the property
anyway, because a voter identity's id is *derived* from its voting key
(`Identifier::create_voter_identifier`). Rotating the voting key on-chain
creates a **different** voter identity rather than retiring a key on the
existing one, so fetching the legacy-named voter identity would confirm the
stale key against its own orphaned identity — verification theatre.

**Behaviour change worth noticing on its own:** a masternode loaded from its
ProTxHash alone no longer gets its voter-identity-target voting key back in one
click. The key is listed as unverifiable and the voter link goes with it
(`VoterLinkWithoutVotingKey`), so the node keeps reporting its voting role as
missing and `render_missing_voter`'s "Add voting key" prompt stays the remedy on
offer — which is the honest outcome, since nothing available offline can tell
that key from one the chain replaced. A record that *does* carry the voter link
(a re-load keeps it while rebuilding the key map) restores the key as before.

Grouping follows the same rule. `voter_association_is_grouped` now keys on the
voting *role* (`is_voting_role`) rather than on the voter-identity target — the
predicate that already decides whether the link is worth offering — so the two
cannot disagree about which candidate the link belongs to.

**§2.1's "restore first, type the WIF as fallback" did not ship, and cannot.**
The node page carried a second missing-voter message for the case where the plan
holds a voting-role candidate. That case requires the modern record to name the
voter identity, and the only production write that names it
(`load_identity.rs`) is the same branch that writes the voting key — so a record
that could show the message is never in the missing-voter state. The message,
its companion tooltip and the predicate behind them were removed as unreachable;
`render_missing_voter` shows one honest message and its in-place prompt.
Restoring such a key safely needs an on-chain check this design rejects offline
(above), tracked as issue #942.

### 10.12 The operator link is unverifiable in the same way its keys are

§10.10 stopped the legacy file vouching for a key held on a voter or operator
identity, but left `associated_operator_identity` itself offered with no gate.
Nothing in production writes that field fresh — `load_identity.rs` only carries
an existing value over — so an offered operator link was always the legacy
file's uncorroborated claim.

Restoring it is the same circularity one step further out: the claim lands in
the modern record, and the next preview reads it back as the outside witness
`reference_identity` demands, making every operator key in the same file a
candidate. Shipped: an operator link the modern record does not already carry is
excluded as `LinkedIdentityUnverified`, like the keys held on that identity. The
voter link keeps its own `VoterLinkWithoutVotingKey` gate, which answers a
different question (a link with no key behind it reports a node as able to vote
when it cannot).

### 10.11 The fresh-install gate is on rows, not on the file

§2.1 step 1 gated detection on `data.db` existing. Every install passes that
gate: `AppState::boot_inputs` opens an existing file read-only *or* creates one
and runs the fresh-install schema ladder, so `FetchState::Unavailable` was
unreachable in the shipped binary and every identity or node screen dispatched
a detection task against a table the ladder had just created empty. The kittest
that claimed otherwise only passed because the `testing` feature substitutes an
in-memory database with no path.

Shipped: `AppContext::has_legacy_identities` asks whether the legacy `identity`
table holds a local row for this network — one `SELECT EXISTS` over the same
filter `read_identities` uses, answered once per context and cached, `false`
when the table does not exist at all. A probe that errors arms the offer rather
than retiring it: the detection task surfaces its own typed error, while a
silent `false` would withdraw a recovery the user's data still supports.

The kittest was replaced by lib tests over a real file-backed `data.db`
(`only_a_legacy_identity_row_for_this_network_arms_the_check`), which assert the
premise directly — the file exists and the gate still says no — and cover the
network scoping a file check could never have.

### 10.13 The write section needs a guard the load claim does not provide

§10.6 leans on the load claim plus re-reading to make the write safe. Neither
covers the writers that actually contend for the record. The claim is taken by
the load paths and by recovery, and nothing else: a refresh, a top-up, a
transfer, a DPNS registration and a key edit all persist a whole
`QualifiedIdentity` snapshot through `update_local_qualified_identity` without
it. Re-reading closes the *prompt* window, but the merge's own read → seal →
write is still three steps, and another writer's snapshot landing inside them
erases the restored keys with no error anywhere. `migration_run` does not help:
the ordinary writers do not take it either.

Separately, `VerifiedIdentityPassword` proves what the vault held when the
prompt closed. The tier migrations (`ProtectIdentityKeys` /
`UnprotectIdentityKeys`) take neither the claim nor `migration_run`, so an
identity unprotected and re-protected under a *new* password while the prompt
sat open still reads as protected at write time — and §3.6's branch would seal
the recovered keys under the old one, leaving one identity needing two
passwords.

Shipped: `AppContext::identity_record_lock(identity_id)` — a per-identity guard
taken inside `insert_local_qualified_identity`,
`update_local_qualified_identity`, `set_identity_alias`,
`delete_local_qualified_identity` and both tier migrations, so coverage does not
depend on remembering it at each of ~20 call sites. `persist_legacy_recovery`
holds it for its whole read → merge → seal → write span and writes through
`write_local_qualified_identity_locked`. Under that guard it re-proves the
verified password against the protected scope *as it stands then*
(`SecretAccess::identity_object_password_still_opens`), refusing with
`LegacyRecoveryIdentityChanged` when it no longer opens.

Lock order is `migration_run` → record guard, matching the delete path; nothing
is held across an `.await`, and the password prompt still runs entirely outside
both, so a UI thread that waits on the guard can never be waiting on itself.
The one whole-record write that does *not* take it is `persist_identity_blob`,
the eager vault migration inside the read path — a caller already holding the
guard reaches it, so taking it there would self-deadlock. That write is
idempotent (it replaces plaintext with vault placeholders in the blob it just
read) and carries a TODO to fold it into the guarded path.

### 10.14 The Key Info offer needed a route before it was reachable at all

§2.1 named the Key Info screen as reachable "from the Identities screen for
`User` identities and via 'Manage keys' for nodes", and §7 row 1 called the
entry point discoverable. Both were false for `User` identities as shipped.

The legacy `IdentitiesScreen` per-key popup is the route §2.1 means, and
`ui/components/left_panel.rs` deliberately drops `RootScreenIdentities` from the
nav, so nothing navigates to it. The identity hub's replacement surface —
Settings → Advanced → "Manage keys" — opened a read-only key table with no way
into `KeyInfoScreen`. Every remaining route (transfer, withdraw, the token
screens) is gated on the identity already holding a key of the kind that action
needs, which is exactly false for the identities this flow exists to help. A
`User` identity with stranded keys could therefore reach the offer only in
Developer view, through a send-money screen.

§2.1 and §7 row 1 are both left as written, per this document's model — the
proposal stands as proposed and this section is the correction. A third surface
now carries the offer that §2.1 names two for, and the mechanism §7 row 1 calls
an "on-arrival" detection task is the `ensure_checked()` render-loop latch of
§10.1.

Shipped: `KeysScreen` carries the `QualifiedIdentity`, renders one row per key
in the §10.8 vocabulary with its held state in words, and opens
`KeyInfoScreen` for any key regardless of what the device holds. The offer
itself renders on that list, above the rows, because it is identity-scoped —
requiring a user to pick an arbitrary key to discover an identity-level offer
repeats the defect one level up. The offer stays on `KeyInfoScreen` too: it
self-extinguishes, and the masternode path lands there.
