# Key placement resolution

How Dash Evo Tool decides which key store an identity key's private half lives
in, and why that is asked of the store rather than derived from the key.

Issue #889 follow-up. Supersedes the reconciliation-migration approach that was
designed for this problem and then withdrawn — see §6.

## 1. The defect

An identity's private halves live in a `BTreeMap` keyed by
`(PrivateKeyTarget, KeyID)`, serialized into the on-disk `QualifiedIdentity`
blob. The target used to be derived from the key's `Purpose`:

```rust
Purpose::VOTING => PrivateKeyOnVoterIdentity,
_               => PrivateKeyOnMainIdentity,
```

That derivation cannot express a `Purpose::VOTING` key filed on the **main**
identity, which is a supported shape: `masternode_key_presence` reads it as
voting readiness on its own, and `load_identity` — the authoritative loader —
files a main-identity key under `PrivateKeyOnMainIdentity` whatever its purpose.

For such a key, `sign` and `can_sign_with` looked in a store it was never filed
under. The app accepted the key, saved it, listed it as held, and no signing path
could find it. Two further consequences followed from the same derivation:

* A delete could land on a **different key**. The voter and main key-id spaces
  overlap, so id 0 names two keys on a masternode; removing one could remove the
  other's private half.
* A held check could pass on the strength of an unrelated key, because a probe
  keyed on the id alone cannot tell two keys apart.

## 2. Two questions, two functions

The derivation conflated two questions that have different answers.

| Question | Answer | Used by |
|---|---|---|
| Where **is** this key's private half filed? | `KeyStorage::candidates` — probes each store at the key's id, keeping only entries whose stored public-key data matches | every read and delete |
| Where **should** a new private half go, and which list names this key? | `QualifiedIdentity::placement_of` — reads the identity's own on-chain key lists | the Key Info paste path, and role naming |

`candidates` is three `BTreeMap` probes, not a scan, in a fixed
[`PROBE_ORDER`] — so resolution never depends on map iteration order. It accepts
an entry only when `same_key` does: every field of the public half except
`disabled_at`, which is the one field Platform lets move after a key is added, so
a key disabled on chain since it was saved still matches its stored snapshot.
Matching on key *material* alone would not be enough — a main identity's voting
key and a linked voter identity's key can carry identical `data` under the same
`id`, leaving `purpose` as the only thing telling them apart, and conflating them
would hand out or delete material the requested key does not own.

`placement_of` returns `Resolved` / `Ambiguous` / `Unknown`. `Unknown` is a real
state, not a failure: `add_key_to_identity` inserts a key before broadcasting the
transition that publishes it, so a key on no list is the steady state there.

`add_key_to_identity` is also the reason the backend write path does not consult
`placement_of` at all: it mints the key at `max_id + 1` on the main identity, so
`PrivateKeyOnMainIdentity` is the only list that can publish it. The slot is
still guarded: `max_id` comes from the freshly published record while the store
is local, so an entry saved here but never broadcast can occupy `max_id + 1`
with a different key, and the write refuses rather than overwrites. The paste
path in Key Info is the one writer that has to choose, because the key it is
handed already exists somewhere.

Synchronous callers get one shared approximation of §3's rule:
`KeyStorage::first_live_candidate` — the first candidate whose bytes are
resident, else the first candidate at all. A UI frame cannot await the honest
bytes-yielding resolution, so every screen that names a placement or shows held
material routes through it (or `held_private_key_data`, built on it). One rule
means the placement a screen names and the material it displays can never come
from two different stores; it approximates liveness without opening the vault by
trusting an `InVault` placeholder only when no resident sibling exists.

## 3. Resolve to bytes, not to a match

`resolve_private_key_bytes` takes the public key and returns the first placement
that **yields bytes** — not the first that matches.

The difference matters for one specific shape: a `PrivateKeyData::InVault`
placeholder whose vault secret is gone, sitting beside a live entry for the same
key under another store. A resolver that stopped at the first match would report
that key unusable with its bytes one probe away. Falling through makes the dead
entry self-healing at read time, with nothing deleted.

With nothing to fall through to, the first failure is returned rather than
`Ok(None)`, so "the vault is not open" never degrades into "you never had that
key".

The walk is resident-first: placements whose bytes are resident resolve with no
chokepoint access and are tried before vault-backed or wallet-derived ones — the
async mirror of `first_live_candidate`'s rule, so the two resolvers agree about
which copy of a dual-filed key answers first. Within each group, probe order
decides.

One exception ends the walk early: a cancelled password prompt
(`TaskError::SecretPromptCancelled`) is returned as-is, outranking any earlier
placement's mechanical failure. It is the user's answer about the key — falling
through to a sibling placement would re-ask for what was just declined, one
dialog per store, and reporting a prior placement's failure instead would deny
that anything was asked. Because resident placements walk first, the carve-out
can only fire when no prompt-free copy existed to serve.

## 4. The map key and the vault label are one address

A vault-backed key's bytes live under the label
`identity_key_priv.<m|v|o>.<key_id>`, derived from the map key and scoped to the
main identity id. Map key and label are therefore halves of one composite
address: naming a store the blob does not agree with names a label the bytes were
never stored under.

`resolve_private_key_bytes` discovers the placement instead of accepting one, and
builds the vault scope from what it found. With that signature a caller **cannot**
pass a mismatched target. This is why the function takes an `IdentityPublicKey`
rather than a `(target, key_id)` pair.

## 5. Placement is not derivation

Material matching fixes *where a key is filed*. It does not validate that a
`PrivateKeyData::AtWalletDerivationPath` entry's stored path still derives the
right key — an entry can be correctly matched here and still carry a stale path.
That is what the `ECDSA_HASH160` recovery scan in `sign` exists for, and the two
mechanisms are independent. Neither subsumes the other.

## 6. Why no migration

Both conventions exist on disk: `load_identity` and `add_key_to_identity` have
always written structurally, while the Key Info paste path wrote purpose-derived.
The obvious repair is a reconciliation pass that collapses to one convention.

It was designed and rejected. With the material lookup permanent, the resolver
does all of the correctness work, and moving entries buys only one convention on
disk — hygiene. Against that:

* Moving a vault-backed key is a **decrypt/re-encrypt**, because the AEAD binds
  its AAD to `wallet_id ‖ label`. A Tier-2 (password-protected) key therefore
  cannot be moved at boot at all, requiring a deferred password-gated flow.
* The bytes are irreplaceable. An imported masternode voting or owner key has no
  seed to regenerate from; a delete-after-copy that picks the wrong winner
  destroys the only copy. `Cargo.toml`'s `bincode` pin reasons the same way about
  the blob format.
* The eventual move to keying by `(Identifier, KeyID)` changes the label anyway,
  so collapsing now pays a destructive pass over secrets twice.

Leaving both conventions in place costs one three-probe lookup and moves nothing.
A crash cannot strand a key because nothing is ever in motion.

## 7. Consequences

* `impl From<Purpose> for PrivateKeyTarget` is **deleted**. It compiled away with
  no fallout — evidence the derivation was fully contained — and its absence is
  what stops a second derivation being reintroduced.
* `KeyInfoScreen` no longer carries a `target` field. It resolves on demand, so
  there is no state to thread through constructors and nothing for the
  `ScreenType` round trip to drop.
* `KeyStorage.private_keys` is **private**, and the *write-side* accessors that
  name a placement (`insert_at`, `entry_at`, `remove_at`, `insert_if_absent`,
  `has`) are `pub(crate)`. With correctness concentrated in the resolver, a
  caller reaching past it can silently miss a key that is present; keeping the
  placement-naming surface inside the crate keeps the set of such callers
  enumerable. They exist for the two that legitimately know a placement — the
  loader folding a previously-loaded record into a fresh one, and legacy
  recovery, which is *about* the placements an old blob recorded and must not
  be routed through a target-blind resolver. `insert_at` and `entry_at` turned
  out to have no production caller left and are `#[cfg(test)]`.
  The narrowing is not complete: several `pub` methods still take a
  caller-named placement (`get_resolve_local`, `get_resolve_with_seed`,
  `get_cloned_private_key_data_and_wallet_info`, `mark_in_vault`, `is_in_vault`,
  `public_key_for`, `wallet_seed_hash_for`), and `mark_in_vault` zeroizes the
  slot's occupant with no `same_key` guard — so `pub` alone does not yet mean a
  caller cannot file or destroy at a placement of its choosing. The residual is
  tracked in §8 and marked in the code as `TODO(placement-named-pub-surface)`.

## 8. What is not covered

* ~~**A deleted key's vault secret is not removed**~~ — **closed.** The
  remove-private-key path now deletes the vault secrets of the placements
  `candidates` resolved, through `AppContext::delete_identity_key_secrets`, and
  does so *before* the map entries go: the map is what makes a vault label
  enumerable, so the reverse order strands the bytes beyond every later path,
  the whole-identity sweep included. A vault failure aborts with map and blob
  untouched.
* **Keying by `(Identifier, KeyID)`** — the right end state, since it removes the
  role enum entirely. Mechanical once the store is known-consistent.
* ~~Validating the signing path's vault-resolved key against the requested
  public key~~ — **closed.** `with_identity_secret_key` — the chokepoint both
  `SignMessageWithIdentityKey` and `DeriveIdentityKeyForDisplay` read through —
  matches the vault's bytes against the public key the stored identity records
  at the requested placement before the closure runs, and refuses a disagreement
  with `TaskError::IdentityKeyMismatch`. This was the last place trusting a
  caller-supplied placement; the callers still carry `(target, key_id)` fields,
  but they are now checked rather than believed. A key type this build cannot
  derive a public half for skips the check, as `key_exclusion` does. The
  chokepoint also carries §3's fallthrough: the caller names its placement from
  the synchronous approximation, which cannot see a dead vault label, so the
  fetch serves the first placement of the same key whose label is live — a dead
  placeholder cannot shadow a live sibling on the Show/Sign path.
* **Several `pub` `KeyStorage` accessors still take a caller-named placement**
  (marked `TODO(placement-named-pub-surface)` on the struct):
  `get_resolve_local`, `get_resolve_with_seed`,
  `get_cloned_private_key_data_and_wallet_info`, `mark_in_vault`, `is_in_vault`,
  `public_key_for`, `wallet_seed_hash_for`. `mark_in_vault` is the sharp edge —
  it zeroizes whatever occupies the slot and repoints it at a vault label with
  no `same_key` guard; its single production caller is safe only because
  `insert_non_encrypted` refused a foreign occupant earlier in the same flow.
  Narrow them to `pub(crate)` or guard them, then restore §7's stronger claim.
* **Proof generation cannot use a locally-added, not-yet-broadcast key**
  (`backend_task/grovestark.rs`, marked `TODO(grovestark-unpublished-key)`): the
  requested key id is resolved against the identity's published keys before the
  resolver runs, so a key in the normal unpublished state (§2) fails
  indistinguishably from "no such key".

## 9. Test coverage

The layer column tells near-homonyms apart: the *resolver* rows exercise
`candidates` / `resolve_private_key_bytes` (`model/qualified_identity/mod.rs`),
the *naming* rows `placement_of` (same file), the *key store* rows `KeyStorage`'s
own guards and helpers (`encrypted_key_storage.rs`), the *vault* rows the
vault-secret-lifecycle chokepoints (`AppContext::delete_identity_key_secrets` in
`context/identity_db.rs`, `with_identity_secret_key` in `backend_task/wallet/mod.rs`),
and the UI rows the screens that consume them.

| Test | Layer | Pins |
|---|---|---|
| `a_held_voting_key_on_the_main_identity_is_signable_under_either_placement` | resolver | the regression lock — a saved key must be usable under **both** placements, so writer and reader can never drift apart again |
| `voting_key_on_the_main_identity_is_found_where_the_loader_files_it` | resolver | the defect: the shape the authoritative loader writes |
| `a_voting_key_an_older_build_filed_under_voter_stays_findable` | resolver | the no-migration constraint |
| `an_authentication_key_on_the_voter_identity_is_found` | resolver | the mirror defect |
| `two_different_keys_sharing_an_id_are_never_confused` | resolver | the id collision |
| `two_keys_sharing_id_and_material_are_told_apart_by_purpose` | resolver | the collision `data` alone cannot resolve |
| `two_keys_sharing_id_and_material_are_placed_by_purpose` | naming | the same collision on the *naming* question — `placement_of` tells the twins apart too |
| `a_key_disabled_since_it_was_saved_is_still_found` | resolver | `disabled_at` is the one field that legitimately moves |
| `a_key_disabled_since_it_was_saved_still_has_a_placement` | naming | naming survives on-chain disabling as reading does |
| `removing_one_key_leaves_a_different_key_sharing_its_id_alone` | Key Info screen | the delete path: confirmed RED against the purpose-derived removal, which left the key the user asked to delete in place and removed another |
| `a_dead_vault_placeholder_falls_through_to_a_live_placement` | resolver | the fallthrough rule (§3) |
| `a_lone_dead_placement_surfaces_its_error_rather_than_absence` | resolver | the other half of it |
| `a_key_with_no_placement_resolves_to_absence` | resolver | absence is `Ok(None)` — never an error, never another key's material |
| `cancelling_the_prompt_stops_asking_for_the_same_key` | resolver | §3's cancellation carve-out: one refusal, one dialog |
| `a_cancellation_outranks_an_earlier_placements_failure` | resolver | the cancellation is what surfaces, not a prior placement's mechanical failure |
| `a_resident_sibling_resolves_without_prompting_for_a_sealed_copy` | resolver | the resident-first walk: a sealed copy cannot put a prompt in front of bytes held in the clear |
| `duplicate_placements_are_returned_in_probe_order` | resolver | determinism, not iteration order |
| `an_entry_whose_material_disagrees_is_not_a_candidate` | resolver | the assumption material matching rests on |
| `an_operator_filed_key_from_a_legacy_blob_stays_reachable` | resolver | `PrivateKeyOnOperatorIdentity` has no live writer but is legacy-reachable |
| `both_keys_of_a_real_v093_blob_resolve_to_their_own_material` | migration | a **real** v0.9.3 blob's keys are reachable, not merely decodable |
| `a_different_key_cannot_take_an_occupied_slot` | key store | the write guard: a foreign occupant refuses the write and survives it unchanged |
| `the_same_key_still_overwrites_itself` | key store | re-entering a saved key corrects it rather than duplicating it |
| `the_occupied_slot_refusal_names_a_performable_remedy` | key store | the refusal's message names removal — the remedy that exists — not a refresh |
| `same_key_ignores_a_key_being_disabled` | key store | the `same_key` carve-out: `disabled_at` may move after a key is saved |
| `same_key_rejects_a_disagreement_anywhere_else` | key store | any other field disagreeing means a different key |
| `a_resident_placement_wins_over_a_vault_placeholder` | key store | `first_live_candidate` prefers resident bytes (§2's synchronous approximation) |
| `a_lone_placement_is_named_whatever_it_holds` | key store | with a single candidate there is nothing to prefer |
| `an_unheld_key_has_no_placement_and_no_material` | key store | an unheld key answers `None` to both placement questions |
| `wallet_derived_at_looks_past_a_placement_that_is_not_derived` | key store | the wallet probe is typed, not liveness-based — a duplicate's first placement need not be the derived one |
| `a_wallet_is_found_under_a_later_placement_too` | wallet lookup | `get_selected_wallet` finds a key's wallet under any placement it is filed at |
| `a_key_that_cannot_be_placed_is_not_reported_as_held` | Key Info screen | a paste with no store to file under is refused and never shown as held |
| `a_key_the_persist_refuses_is_not_reported_as_held` | Key Info screen | a persist refusal rolls the paste back — on screen and in the in-memory record |
| `a_removal_that_cannot_persist_keeps_the_key_held` | Key Info screen | a removal that could not persist leaves the key held on screen |
| `a_write_behind_the_screen_survives_a_paste` | Key Info screen | the paste is a locked read-modify-write — no concurrent writer's key is written away |
| `a_write_behind_the_screen_survives_a_removal` | Key Info screen | the removal edits the record as stored now, same lost-update guard |
| `a_show_request_that_cannot_resolve_speaks_through_the_typed_error` | Key Info screen | Show/Sign failures surface each variant's own remedy |
| `the_placement_errors_advise_an_action_both_paths_can_perform` | Key Info screen | the placement errors' remedies fit the Show/Sign read path as well as the paste path |
| `a_held_key_published_on_no_list_still_gets_a_row` | keys list | a held-but-unpublished key is reachable, so the occupied-slot remedy is performable from the add-key flow too |
| `the_keys_popup_finds_a_main_key_an_older_build_filed_under_voter` | identities list | the Keys popup is placement-blind for main-identity keys |
| `the_keys_popup_finds_a_voter_key_an_older_build_filed_under_main` | identities list | its voter-list mirror |
| `removing_a_key_also_removes_its_vault_secret` | vault | §8's second bullet: confirmed RED against the map-only removal, which left the bytes on disk unreachable |
| `delete_identity_key_secrets_drops_only_the_named_placement` | vault | the per-key delete is not the whole-identity sweep, and repeating it is harmless |
| `a_vault_secret_that_is_not_the_recorded_key_is_refused` | vault | §8's third bullet: RED against the unchecked chokepoint, which signed with the planted key |
| `a_placement_the_identity_does_not_record_is_refused` | vault | the other half of it — an orphaned label is not a key of this identity |
| `a_secret_matching_its_recorded_key_still_resolves` | vault | the check costs a healthy install nothing |
| `a_dead_placeholder_at_the_named_placement_falls_through_to_a_live_sibling` | vault | §3's fallthrough at the named-target chokepoint: Show/Sign reach a live sibling behind a dead placeholder |

[`PROBE_ORDER`]: ../../../src/model/qualified_identity/key_placement.rs
