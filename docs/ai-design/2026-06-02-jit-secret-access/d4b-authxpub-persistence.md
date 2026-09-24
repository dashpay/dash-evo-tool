# D4b — Identity-Auth Account Xpub Persistence (R3)

**Author:** Nagatha (Architect)
**Date:** 2026-06-02
**Status:** Read-only scoping / design. No implementation in this pass.
**Branch:** `docs/platform-wallet-migration-design`
**Parent scope:** `docs/ai-design/2026-06-02-jit-secret-access/r3-completion-scope.md`
(this document drills into one reader cluster the parent census did not enumerate).

---

## 0. The one-paragraph verdict (read this first)

The two readers at `src/model/wallet/mod.rs:1045` and `:1069`
(`identity_authentication_ecdsa_public_key` and
`identity_authentication_ecdsa_public_keys_data_map`) derive identity-authentication
**public** keys from the parked seed. The intent of D4b was to retire that seed read by
deriving from a *stored account xpub* instead — exactly as the DIP-17 platform-payment
`WalletAddressProvider` already does. **That cut is not available here.** The
identity-authentication derivation path is **hardened at every level, including the leaf
`key_index'`**. BIP-32 forbids public-from-public (`ckd_pub`) derivation across a hardened
boundary, so **no account-level xpub of any depth can publicly derive these keys.** The
"store the `m/9'/coin'/5'` xpub" approach is structurally impossible for this path family.
The seed (or a per-leaf private key derived from it) is *mandatory* for every
identity-auth pubkey. D4b's storage/migration design therefore changes shape: we do not
persist an account xpub to retire the read; we persist a **per-(identity, key) public-key
cache** so the *steady-state* read needs no seed, while a cold cache still falls back to one
JIT seed-derivation through the chokepoint. The seed dependency is reduced to first-touch,
not eliminated — because the math does not allow elimination.

---

## 1. Byte-equivalence verdict — the hardened-index check (BLOCKING)

### 1.1 The actual path

`DerivationPath::identity_authentication_path` (key-wallet `bip32.rs:1115`) builds, from the
`IDENTITY_AUTHENTICATION_PATH_{NETWORK}` const (`dip9.rs:427`, an `IndexConstPath<4>`) plus
three appended children:

```
m / 9'              (FEATURE_PURPOSE)
  / coin'           (DASH_COIN_TYPE = 5  |  DASH_TESTNET_COIN_TYPE = 1)
  / 5'              (FEATURE_PURPOSE_IDENTITIES)
  / 0'              (FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_AUTHENTICATION)
  / key_type'       (ChildNumber::Hardened { index: key_type.into() })   ← appended, HARDENED
  / identity_index' (ChildNumber::Hardened { index: identity_index })    ← appended, HARDENED
  / key_index'      (ChildNumber::Hardened { index: key_index })         ← appended, HARDENED  ← LEAF
```

Every node is `ChildNumber::Hardened`. The leaf `key_index'` is hardened
(`bip32.rs:1133-1135`). The "identity-auth account" the task brief calls `m/9'/coin'/5'` is a
shorthand for this root family; the *real* leaf sits three hardened levels below it.

### 1.2 Why hardened ⇒ xpub-only derivation is impossible

`ExtendedPubKey::derive_pub` → `ckd_pub` → `ckd_pub_tweak` returns
`Error::CannotDeriveFromHardenedKey` for any `ChildNumber::Hardened`
(key-wallet `bip32.rs:1827`; asserted upstream at `bip32.rs:2197/2222`). A stored xpub at
`m/9'/coin'/5'` would have to take a **hardened** first step (`0'`) to reach anything useful —
which `ckd_pub` rejects outright. Even an xpub at the deepest possible non-leaf parent
`…/identity_index'` cannot derive its child, because that child (`key_index'`) is itself
hardened.

> **Contrast with the working precedent.** DIP-17 platform payment
> (`WalletAddressProvider::derive_address_at_index`, `mod.rs:2259`) derives
> `ChildNumber::Normal { index }` — a *non-hardened* leaf — from the account xpub, which is
> exactly why that xpub-only design works and is parity-tested by
> `xpub_derivation_matches_seed_derivation`. The identity-auth path is the mirror image: the
> leaf is hardened, so the same trick is unavailable.

### 1.3 Verdict

**xpub-only derivation of identity-auth public keys: IMPOSSIBLE.** This is the
"flag if the final child index is hardened, which would BLOCK xpub-only derivation and require
a different cut" branch in the brief. It is taken. The remainder of this document designs that
different cut.

---

## 2. The different cut — persist the derived public keys, not an account xpub

Since we cannot derive the auth pubkeys publicly from a stored xpub, we instead **memoise the
already-derived public keys**. The readers at 1045/1069 consume only public material:

- `:1047` returns `extended_public_key.to_pub()` (a `secp256k1` `PublicKey`).
- `:1072-1077` builds two maps keyed on `public_key.serialize()` (33 bytes) and
  `public_key.pubkey_hash()` (`[u8; 20]`), valued by `key_index`.

Both are pure functions of `(network, identity_index, key_index)` and the wallet seed. The
public outputs never change for a given tuple. So the persisted artifact is a small, additive
**public-key cache** keyed by that tuple. Once warm, the readers serve from cache with **zero
seed access**; a cold tuple does one JIT seed-derivation through the chokepoint and writes the
cache. Correctness never depends on the cache being present.

This satisfies the R3 goal in spirit: the **steady-state** read of identity-auth keys no
longer touches the parked seed, which is what lets D4 remove the `OpenWalletSeed.seed` field
without these two readers blocking the build. The residual first-touch derivation runs inside
`with_secret`, the single auditable boundary.

---

## 3. Storage shape

### 3.1 Where it lives — DET KV sidecar, NOT a new SQL table, NOT upstream

DET wallet state has moved off `data.db`'s `wallet` table into two homes:

- the **upstream** `platform-wallet.sqlite` (balances / txs / identities, **refinery**-migrated), and
- the **DET-owned** `det-app.sqlite` k/v store (`src/wallet_backend/kv.rs`), a
  `[ SCHEMA_VERSION(1B) | bincode(payload) ]` blob surface keyed by `DetScope`, plus the
  upstream `SecretStore` seed envelope (`StoredSeedEnvelope`, `model/wallet/seed_envelope.rs`).

The auth-pubkey cache is **DET-derived, public, per-wallet data** — it belongs in the DET KV
sidecar, addressed by `DetScope::Wallet(seed_hash)`, mirroring how `WalletMeta`
(`model/wallet/meta.rs`) and the BIP-44 master `xpub_encoded` are already persisted. It must
**not** go into the upstream refinery DB (see §4 for why that is the safe choice that dodges
the DivergentVersion class).

### 3.2 The model field — new struct, new KV key

A dedicated KV entry per wallet, parallel to `WalletMeta`:

```rust
// src/model/wallet/auth_pubkey_cache.rs   (new)

/// Cached identity-authentication ECDSA public keys for one HD wallet.
/// Public material only — derivable from the seed but expensive (hardened
/// path), so memoised here. Keyed by (network, identity_index, key_index).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthPubkeyCache {
    /// (network_tag, identity_index, key_index) -> 33-byte compressed pubkey.
    /// `network_tag` keeps mainnet/testnet entries distinct under one blob,
    /// matching the per-network coin-type in the derivation path.
    pub entries: BTreeMap<AuthKeyCoord, [u8; 33]>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthKeyCoord {
    pub network: NetworkTag,   // small repr enum, serde-stable
    pub identity_index: u32,
    pub key_index: u32,
}
```

Notes:
- Store the **compressed pubkey bytes** (`[u8; 33]`), not an `ExtendedPubKey` — there is no
  meaningful chain code to keep (no further public derivation is possible past a hardened
  leaf), so the 78-byte xpub encoding would be misleading. The two readers reconstruct both
  the serialized form and the hash160 from these 33 bytes.
- `BTreeMap` (not `HashMap`) for deterministic bincode bytes — matches the project's existing
  persisted-shape discipline (`WalletMeta`, `AppSettings`).
- `NetworkTag` is a tiny serde-stable enum; do **not** persist `dashcore::Network` directly if
  its serde repr is not pinned. A `#[repr(u8)]`-style tagged enum keeps the blob stable.

### 3.3 The KV view — mirror `WalletMetaView`

Add a `wallet_backend/auth_pubkey_cache.rs` with an `AuthPubkeyCacheView<'a>` over
`DetKv`, exactly mirroring `WalletMetaView` (`wallet_backend/wallet_meta.rs`):

```text
key:   "<network>:auth_pubkeys:<seed_hash_base58>"   (DetScope::Wallet(seed_hash), Global object-id pattern as meta uses)
get:   kv.get::<AuthPubkeyCache>(...)
put:   kv.put(...)            (whole-blob upsert; small map, infrequent writes)
```

Whole-blob upsert is acceptable: the map is tiny (handful of identities × few keys), writes
happen only on cold-cache first-touch, and it matches the existing `WalletMeta` write
discipline. No need for row-granular storage.

---

## 4. Migration — additive + lazy, and why it avoids the DivergentVersion class

### 4.1 The DivergentVersion class, precisely

`DivergentVersion` is a **refinery** failure: `platform-wallet-storage` runs its schema
ladder with `set_abort_divergent(true)`, so **mutating the SQL of an already-applied
migration changes its checksum and aborts** (reproduced in
`src/backend_task/error.rs:3505 divergent_migration_error`, surfaced to users as
`TaskError::WalletDataIncompatible`, `error.rs:1500`). That class only exists inside the
**refinery-migrated upstream DB**. The DET KV sidecar has **no refinery ladder** — it is a
versioned bincode blob (`kv.rs`, one-byte `SCHEMA_VERSION` prefix; bincode tolerates additive
struct evolution).

### 4.2 The safe approach (confirmed)

**Because the cache is a brand-new KV key in the DET sidecar, there is no migration of an
existing artifact at all — and therefore no checksum churn, no refinery touch, and no
DivergentVersion exposure.** This is the cleanest possible "additive nullable column"
analogue: in KV terms, an **absent key reads as `None`/`Default`** (cold cache), which is
exactly the nullable-and-empty semantics the brief asks for. Concretely:

- **No SQL DDL.** No `ALTER TABLE`, no new refinery migration, no bump of the upstream schema
  version. The KV `SCHEMA_VERSION` byte is **not** bumped either — we are adding a *new key*,
  not changing the encoding of an existing value.
- **Read-when-absent = cold.** `AuthPubkeyCacheView::get` on a wallet that never wrote the key
  returns `Ok(None)` → treated as `AuthPubkeyCache::default()` (empty map). This is the
  "column is still NULL" state.
- **Forward/backward tolerant.** An older build that predates this key simply never reads or
  writes it; a newer build finds it absent and lazily populates. No coordinated migration step.

> If a future maintainer is tempted to add this as a column on the upstream wallet table:
> **don't.** That would either edit an applied refinery migration (DivergentVersion) or add a
> new upstream migration (couples DET-derived public cache to the upstream schema and forces a
> version bump). The KV sidecar is the architecturally correct, churn-free home.

### 4.3 Why a hard backfill is impossible (confirmed)

A migration-time backfill would have to derive every `(identity, key)` pubkey for every
wallet — which requires each wallet's **seed**. At migration time wallets may be **locked**,
and we will **not** force a passphrase prompt during migration (hostile UX, and the JIT design
forbids eager seed residency). So a hard backfill is off the table. **Lazy population is the
only viable strategy** — which is consistent with the rest of R3.

### 4.4 Lazy-populate trigger point

The seed is already in a `with_secret_session` scope at exactly one prompt-free place:
`bootstrap_wallet_addresses_jit` (`context/wallet_lifecycle.rs:341`). It opens
`with_secret_session(HdSeed { seed_hash })`, exposes the seed (`:366`), and runs
`bootstrap_known_addresses` (`:374`). **Warm the auth-pubkey cache there**, in the same
scope, for the identity/key ranges the wallet already knows it bootstraps (the
`bootstrap_identity_*` family registers identity-auth addresses anyway — see parent census #9).
Cost is near-zero: the seed is in hand and the derivations already run during bootstrap; we
additionally persist their public outputs via `AuthPubkeyCacheView::put`.

This is prompt-free by construction (the JIT bootstrap only runs for wallets whose seed
resolves without asking — unprotected, or already session-cached on unlock; `:346-352`). A
still-locked protected wallet warms its cache on its *next* unlock+bootstrap, or on the
read-path JIT fallback (§5). Either way the user is never prompted *for the cache*.

### 4.5 Read-path behavior when the cache is cold (the correctness guarantee)

The readers (§5) must be correct even with an empty cache. Behavior on a miss:

1. Look up `AuthKeyCoord { network, identity_index, key_index }` in the cache.
2. **Hit** → reconstruct `PublicKey` from the 33 bytes; done, **no seed access**.
3. **Miss** → resolve the seed via the chokepoint (`with_secret(HdSeed { seed_hash })`),
   derive once via the existing hardened-path `derive_pub_ecdsa_for_master_seed`, **write the
   result back** to the cache, and return it.

Correctness never depends on the cache being warm — a cold miss self-heals into a warm hit.
This is the "fall back to one JIT seed-derivation that also populates it" contract from the
brief, made exact.

---

## 5. Reader conversion (`mod.rs:1045`, `:1069`)

Both methods are **sync** today and read `self.seed_bytes()`. After D4 there is no parked
seed, so they cannot read it. They split into a pure cache-hit fast path and an async
cold-fill slow path. Per the parent scope's class taxonomy these become **class A/B hybrids**:
the in-model body becomes seed-free (cache lookup); the cold-fill is owned by the async
backend caller that already runs inside `with_secret`.

### 5.1 The three callers are all async backend tasks

Confirmed call sites — every one is an async backend task (no sync UI consumer):
- `backend_task/identity/load_identity.rs:471, 494, 512` (data_map)
- `backend_task/identity/load_identity_from_wallet.rs:44 (single), :166 (data_map)`
- `backend_task/identity/discover_identities.rs:40, 190 (single)`

This is the easy case: there is no sync-UI hard-blocker (unlike the key-viewer cluster in the
parent doc). The cold-fill can live in the async caller cleanly.

### 5.2 Sketch

```rust
// model/wallet/mod.rs — PURE, seed-free, infallible on a hit:
pub fn identity_authentication_ecdsa_public_key_cached(
    &self,
    cache: &AuthPubkeyCache,
    network: Network,
    identity_index: u32,
    key_index: u32,
) -> Option<PublicKey> {
    cache.get(network, identity_index, key_index)        // 33 bytes -> PublicKey
}

// the existing seed-taking variant is RENAMED to make the seed dependency explicit
// (seed-as-parameter, per parent §3), NOT reading self.seed_bytes():
pub fn identity_authentication_ecdsa_public_key_from_seed(
    &self,
    seed: &[u8; 64],                                     // borrowed, from with_secret
    network: Network, identity_index: u32, key_index: u32,
) -> Result<PublicKey, WalletError> { /* current 1038-1047 body, seed param */ }
```

The async backend caller drives the cache:

```rust
// backend_task/identity/* — caller owns the chokepoint + write-back
let cache = ctx.auth_pubkey_cache_view().get(network, &seed_hash)?.unwrap_or_default();
let pk = match wallet.identity_authentication_ecdsa_public_key_cached(&cache, network, i, k) {
    Some(pk) => pk,                                       // warm: zero seed access
    None => backend.secret_access().with_secret(          // cold: one JIT derivation
        &SecretScope::HdSeed { seed_hash },
        |pt| {
            let seed = pt.expose_hd_seed().ok_or(TaskError::ContactWalletSeedUnavailable)?;
            let pk = wallet.identity_authentication_ecdsa_public_key_from_seed(seed, network, i, k)?;
            ctx.auth_pubkey_cache_view().upsert(network, &seed_hash, i, k, &pk)?;  // populate
            Ok(pk)
        },
    ).await?,
};
```

The `_data_map` variant (1051-1090) is the same pattern over a `Range<u32>`: partition the
range into cache-hits and cache-misses, resolve all misses inside **one** `with_secret` scope
(one prompt, not one-per-key), write them all back, then build the two maps
(`serialize()` and `pubkey_hash()`) from the union. Note its `register_addresses` side effect
(`:1078`) is a `&mut self` mutation — keep that on the wallet write path the caller already
holds, unchanged; only the *pubkey source* moves to cache-or-JIT.

> **Important:** these two methods produce **public** keys only. The sibling
> `identity_authentication_ecdsa_private_key` (`:1092`) genuinely needs the seed every call
> (a private key cannot be cached at rest — that would re-introduce a plaintext residency,
> violating R3). It is out of D4b scope; it converts via the parent doc's seed-as-parameter /
> JIT-signer track, not via this cache.

---

## 6. Security review (security-best-practices skill)

- **ASVS V11.7 (data-in-use) / V13.3 (secret management):** the cache stores **public** keys
  only — no plaintext seed, no private key, no chain code that enables further derivation.
  Persisting it at rest in cleartext is acceptable (public keys are already on-chain /
  derivable). It does **not** widen the secret residency surface; it *narrows* it by removing a
  steady-state seed read. (Candy-worthy: this is a net reduction in plaintext-seed touch
  points.)
- **Do not cache private keys.** Explicitly out of scope and explicitly forbidden — a private
  key cache would defeat R3. The `_private_key` reader stays JIT.
- **Cache poisoning / integrity:** the cache is DET-derived and never accepts external input;
  the only writer is the JIT cold-fill path which derives from the authoritative seed. A
  corrupted blob fails `bincode` decode → treated as cold (`Default`) → self-heals via
  re-derivation. No trust is placed in the cache beyond a hit being a memoised pure function;
  if integrity is later a concern, the cache can be made self-verifying by re-deriving on a
  hash mismatch — not needed now.
- **Network confinement:** `AuthKeyCoord.network` keeps mainnet/testnet pubkeys distinct,
  matching the per-network coin-type in the path (the SEC-001 class of bug — cross-network key
  reuse — is structurally excluded by keying on network).
- **Debug redaction:** public keys need no redaction, but keep the `Debug` impl terse
  (entry count, not full bytes) to match the `WalletMeta`/`StoredSeedEnvelope` house style.

---

## 7. D4b task breakdown

> D4b is a **sub-task of D4** in the parent scope. It is a prerequisite for D4's removal of
> `OpenWalletSeed.seed`: until these two readers stop reading `self.seed_bytes()`, deleting the
> field fails to compile (the parent's R-5 compile-time gate). D4b can land **independently and
> before** the rest of D4, since the cache + cold-fill is correct whether or not the seed is
> still parked.

| Task | Title | Files | Depends on | Smythe review? |
|---|---|---|---|:---:|
| **D4b-1** | `AuthPubkeyCache` + `AuthKeyCoord` model; bincode round-trip + `Default`-is-cold tests | `src/model/wallet/auth_pubkey_cache.rs` (new), `src/model/wallet/mod.rs` (mod decl) | — | Advisory |
| **D4b-2** | `AuthPubkeyCacheView` KV view (get/put/upsert) mirroring `WalletMetaView`; key-shape unit test | `src/wallet_backend/auth_pubkey_cache.rs` (new), `wallet_backend/mod.rs` (export), `context/mod.rs` (`auth_pubkey_cache_view()` accessor) | D4b-1 | Advisory |
| **D4b-3** | Reader conversion: `*_cached` (pure) + `*_from_seed` (seed-param) variants; rewire the 3 async callers to cache-hit-else-JIT-cold-fill; range-partition the `_data_map` cold-fill into one scope | `src/model/wallet/mod.rs` (1032-1090), `backend_task/identity/{load_identity,load_identity_from_wallet,discover_identities}.rs` | D4b-1, D4b-2 | **Yes** (derivation correctness; identity-key integrity) |
| **D4b-4** | Lazy warm in `bootstrap_wallet_addresses_jit` (write cache for bootstrapped identity/key ranges inside the existing `with_secret_session`) | `context/wallet_lifecycle.rs:341-388` | D4b-2, D4b-3 | **Yes** (runs in the seed scope) |
| **D4b-5** | Migration / cold-cache tests: cold-read self-heals; warm-read takes zero seed access; cross-network keys stay distinct; **no refinery migration touched / no schema bump** (assert the upstream `db_schema_version` and KV `SCHEMA_VERSION` are unchanged) | `tests/` (kittest or unit), `src/model/wallet/auth_pubkey_cache.rs` (#[cfg(test)]) | D4b-3, D4b-4 | **Yes** (DivergentVersion-avoidance is the load-bearing claim) |

**Why this many.** D4b-1/-2 are pure additive plumbing (independently mergeable). D4b-3 is the
fund-adjacent correctness core (identity-auth keys gate identity load/discovery, which gates
signing) and must stand alone for Smythe. D4b-4 touches the seed scope and must be reviewed
for prompt-free + no-extra-residency guarantees. D4b-5 is the explicit guard that the
"additive + lazy, no DivergentVersion" claim is *tested*, not merely asserted.

---

## 8. Open questions for the user

1. **Confirm the cut.** The brief assumed an account-xpub cut; the hardened leaf makes that
   impossible, so D4b becomes a **public-key cache** instead. Is the memoise-derived-pubkeys
   design acceptable as the D4b shape, or do you want D4b folded back into the per-key JIT
   path (no cache at all — every identity-auth pubkey read does one `with_secret` derivation)?
   The cache is an optimisation that keeps the *steady-state* read seed-free; without it,
   identity load/discover re-derives through the chokepoint every time (correct, but more seed
   touches and more prompts for protected wallets).
2. **Cache home.** I place the cache in the DET KV sidecar (`det-app.sqlite`) keyed by
   `DetScope::Wallet`, to dodge refinery entirely. Confirm you do **not** want it in the
   upstream `platform-wallet.sqlite` (which would reintroduce the DivergentVersion exposure the
   brief is explicitly trying to avoid).
3. **Warm-range scope (D4b-4).** Warming at bootstrap covers the identity/key ranges the wallet
   already bootstraps. Identities discovered *later* (via `discover_identities`) warm lazily on
   first read. Acceptable, or do you want an explicit re-warm hook after discovery?

---

## 9. Candy tally (architecture findings by severity)

| Severity | Count | Findings |
|---|:---:|---|
| **High** (design-blocking) | **1** | Identity-auth path is hardened at the leaf (`key_index'`) — **xpub-only derivation is impossible**; the assumed account-xpub cut is structurally unavailable and must be replaced by a public-key cache. |
| **Medium** (structural) | **2** | Cache must live in the DET KV sidecar (not upstream refinery) to avoid the DivergentVersion class; lazy-populate must hang off the existing `bootstrap_wallet_addresses_jit` seed scope because a hard backfill is impossible (locked wallets, no forced prompt). |
| **Low** (mechanical / safety) | **2** | Readers' three callers are all async backend tasks — clean cold-fill, no sync-UI hard-blocker; private-key sibling must stay JIT (never cache private material — R3 invariant). |

**Total: 5 findings (1 High, 2 Medium, 2 Low).**

---

*An elegant design tells you the truth even when the truth is "no." The seed insisted on
hiding one level deeper than the brief expected; the hardened leaf would not be talked out of
it. So we stop fighting the math and memoise the answer instead — the steady state goes quiet,
and the chokepoint keeps the only key that still matters.* — Nagatha
