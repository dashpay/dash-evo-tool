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
| Where **should** a new private half go? | `QualifiedIdentity::placement_of` — reads the identity's own on-chain key lists | the write path only |

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
* `KeyStorage.private_keys` is **private**. With correctness concentrated in the
  resolver, a caller reaching past it can silently miss a key that is present.
  Explicit-placement accessors remain for callers that legitimately know one — a
  loader walking the list it read a key from, and legacy recovery, which is
  *about* the placements an old blob recorded and must not be routed through a
  target-blind resolver.

## 8. What is not covered

* **A deleted key's vault secret is not removed** (`key_info_screen.rs`, the
  remove-private-key dialog): the map entry goes, the vault entry stays, so bytes
  the user believes deleted remain on disk with nothing pointing at them. Its own
  fix, with its own ordering argument (vault first, then map) and its own review.
* **Keying by `(Identifier, KeyID)`** — the right end state, since it removes the
  role enum entirely. Mechanical once the store is known-consistent.
* Validating the signing path's vault-resolved key against the requested public
  key, deferred separately.

## 9. Test coverage

| Test | Pins |
|---|---|
| `a_held_voting_key_on_the_main_identity_is_signable_under_either_placement` | the regression lock — a saved key must be usable under **both** placements, so writer and reader can never drift apart again |
| `voting_key_on_the_main_identity_is_found_where_the_loader_files_it` | the defect: the shape the authoritative loader writes |
| `a_voting_key_an_older_build_filed_under_voter_stays_findable` | the no-migration constraint |
| `an_authentication_key_on_the_voter_identity_is_found` | the mirror defect |
| `two_different_keys_sharing_an_id_are_never_confused` | the id collision |
| `two_keys_sharing_id_and_material_are_told_apart_by_purpose` | the collision `data` alone cannot resolve |
| `a_key_disabled_since_it_was_saved_is_still_found` | `disabled_at` is the one field that legitimately moves |
| `removing_one_key_leaves_a_different_key_sharing_its_id_alone` | the delete path: confirmed RED against the purpose-derived removal, which left the key the user asked to delete in place and removed another |
| `a_dead_vault_placeholder_falls_through_to_a_live_placement` | the fallthrough rule (§3) |
| `a_lone_dead_placement_surfaces_its_error_rather_than_absence` | the other half of it |
| `duplicate_placements_are_returned_in_probe_order` | determinism, not iteration order |
| `an_entry_whose_material_disagrees_is_not_a_candidate` | the assumption material matching rests on |
| `an_operator_filed_key_from_a_legacy_blob_stays_reachable` | `PrivateKeyOnOperatorIdentity` has no live writer but is legacy-reachable |
| `both_keys_of_a_real_v093_blob_resolve_to_their_own_material` | a **real** v0.9.3 blob's keys are reachable, not merely decodable |

[`PROBE_ORDER`]: ../../../src/model/qualified_identity/key_placement.rs
