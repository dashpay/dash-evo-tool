# D4c — `identity_authentication_ecdsa_private_key` (mod.rs:1112) Redesign

**Author:** Nagatha (Architect)
**Date:** 2026-06-02
**Status:** Read-only design. No implementation in this pass.
**Branch:** `docs/platform-wallet-migration-design`
**Parent scope:** `docs/ai-design/2026-06-02-jit-secret-access/r3-completion-scope.md` (the §2 census
classified `identity_authentication_ecdsa_private_key` as the private sibling that "genuinely needs
the seed every call … converts via the JIT-signer track" — this document is that track).
**Sibling:** `docs/ai-design/2026-06-02-jit-secret-access/d4b-authxpub-persistence.md` (the **public**
identity-auth keys, served seed-free from `AuthPubkeyCache`). D4c builds on D4b: the chooser's public
keys come from D4b's cache + public derivation; D4c removes the chooser's *private* derivation.

---

## 0. The one-paragraph verdict (read this first)

I went looking for a hard problem and found that someone had already solved most of it. The
identity-create state transition is **already signed seed-free through the JIT chokepoint** — both
the identity-ownership proof (`QualifiedIdentity` as `Signer<IdentityPublicKey>`,
`model/qualified_identity/mod.rs:316`, which resolves keys via `secret_access.with_secret` +
`get_resolve_with_seed`) and the asset-lock witness (`DetSigner`, in
`WalletBackend::register_identity`, `wallet_backend/mod.rs:1287`). The private keys the chooser
derives at `mod.rs:1112` are **never used to sign the state transition**: `IdentityKeys::to_key_storage`
(`backend_task/identity/mod.rs:84`) *discards* the `PrivateKey` and stores only
`PrivateKeyData::AtWalletDerivationPath { wallet_seed_hash, derivation_path }`, which is re-derived
JIT at signing time. The chooser derives private keys for exactly one reason that survives scrutiny —
to compute the **public** key material (pubkey bytes / hash160) — and one cosmetic reason: to display
a WIF in **advanced** mode. **So the redesign is not "materialize private keys JIT at registration";
registration already does that. The redesign is "stop deriving private keys in the chooser at all,
and serve the chooser from public keys (D4b cache + public derivation)."** The only residual private
derivation is the advanced-mode WIF display, which becomes a backend task (the same
`WalletTask::DeriveKeyForDisplay` pattern D3 introduced for the wallets-screen key viewers). After
that, `identity_authentication_ecdsa_private_key`'s only callers are the e2e helper (converted to a
seed-param/public form) and — transiently — nothing in production. The parked-seed read at
`mod.rs:1112` is then deletable in D4.

---

## 1. What the private key is actually used for (the load-bearing trace)

### 1.1 The chooser builds `IdentityKeys` with real `PrivateKey`s

`AddNewIdentityScreen` holds `identity_keys: IdentityKeys` (`add_new_identity_screen/mod.rs:86`).
Three sync methods populate it, each calling `identity_authentication_ecdsa_private_key` (mod.rs:1112):

| Method | Sites | When |
|---|---|---|
| `ensure_correct_identity_keys` | `:212` (master), `:225` (5 default keys) | wallet selected / unlocked / index or funding-method change |
| `update_identity_key` | `:950` (master), `:966` (existing keys) | identity index changed |
| `add_identity_key` | `:1000` | advanced-mode "+ Add Key" |

All run **inside `ui()` render paths** (Class C — no async boundary). `IdentityKeys.keys_input`
carries `(PrivateKey, DerivationPath)` tuples.

### 1.2 At registration, the private keys are thrown away

`register_identity_clicked` (`:789`) packages `self.identity_keys.clone()` into
`IdentityRegistrationInfo` and dispatches `IdentityTask::RegisterIdentity`. In the backend:

- `to_public_keys_map()` (`backend_task/identity/mod.rs:180`) → the `BTreeMap<KeyID,
  IdentityPublicKey>` sent to Platform. **Uses `private_key.public_key(&secp)` only** — public
  output.
- `to_key_storage(wallet_seed_hash)` (`:84`) → the `KeyStorage` stored on the `QualifiedIdentity`.
  For every key it computes the public data (`pubkey_hash` / `to_bytes`) and then stores
  `PrivateKeyData::AtWalletDerivationPath { wallet_seed_hash, derivation_path.clone() }`. **The
  `PrivateKey` value is dropped.** The stored artifact is the *path*, not the key.

So the `PrivateKey` in `IdentityKeys` is consumed only to derive its own public key. The signing
material is reconstructed later from the path.

### 1.3 Signing the identity-create ST is already JIT

Two registration funding flows, both already seed-free:

**(a) Wallet / asset-lock funded** → `register_identity_via_wallet_backend`
(`register_identity.rs:137`) → `backend.register_identity(...)` (`wallet_backend/mod.rs:1287`):

```text
with_secret_session(HdSeed{seed_hash}) {
    let asset_lock_signer = DetSigner::from_held(session.plaintext(), network);  // JIT
    wallet.identity().register_identity_with_funding(
        funding, identity_index, keys_map,
        identity_signer:  &QualifiedIdentity,   // proves key ownership — JIT-resolves keys
        asset_lock_signer:&DetSigner,           // asset-lock witness — JIT
        settings)
}
```

`QualifiedIdentity::sign` (`mod.rs:316`) resolves each signing key via the pure
`wallet_seed_hash_for` probe → `with_secret(HdSeed{seed_hash})` → `get_resolve_with_seed(seed)`. No
parked-seed read. The keys being added sign to prove ownership through this path, JIT.

**(b) Platform-address funded** → `register_identity_from_platform_addresses`
(`register_identity.rs:237`): builds a `DetPlatformSigner::from_held(seed, …)` inside
`with_secret_session` and calls `identity.put_with_address_funding(..., &qualified_identity,
&signer, …)`. Identity-ownership signing is still `&qualified_identity` (JIT); the address-funding
witness is `DetPlatformSigner` (JIT, from D3).

**Conclusion.** Identity-create signing has *no* dependency on the chooser's private keys and *no*
parked-seed read. The JIT private-key provisioning the brief asks me to "hook into registration"
**already exists** and is the production path today. D4c's job is to stop the *chooser* from deriving
private keys it does not need.

---

## 2. Design — the chooser-public / registration-JIT split

### 2.1 Chooser uses PUBLIC keys (the core change)

`IdentityKeys` stops carrying `PrivateKey`. It carries the **public** material plus the derivation
path — which is all `to_public_keys_map` and `to_key_storage` actually consume. New shape:

```rust
// backend_task/identity/mod.rs
pub struct IdentityKeyEntry {
    pub public_key: PublicKey,            // 33-byte compressed secp256k1 pubkey
    pub derivation_path: DerivationPath,  // the wallet path; private key re-derived JIT at sign time
    pub key_type: KeyType,
    pub purpose: Purpose,
    pub security_level: SecurityLevel,
    pub contract_bounds: Option<ContractBounds>,
}

pub struct IdentityKeys {
    pub(crate) master: Option<IdentityKeyEntry>,   // purpose=AUTHENTICATION, security=MASTER
    pub(crate) others: Vec<IdentityKeyEntry>,
}
```

`to_public_keys_map` / `to_key_storage` change from `private_key.public_key(&secp)` to reading
`entry.public_key` directly — the pubkey-data branches (`ECDSA_HASH160` → `pubkey_hash`,
`ECDSA_SECP256K1` → `to_bytes`) are unchanged, they just take the already-derived pubkey. The
`WalletDerivationPath { wallet_seed_hash, derivation_path }` stored in `KeyStorage` is built from
`entry.derivation_path` exactly as today. **`to_key_storage` becomes purely public** — no `PrivateKey`
anywhere on the registration-prep path.

### 2.2 Where the chooser's public keys come from

The three chooser methods switch from `identity_authentication_ecdsa_private_key` to public
derivation, served by **D4b's `AuthPubkeyCache` + the public derivation path**:

```text
// chooser, seed-free:
let pk = wallet.identity_authentication_ecdsa_public_key_cached(&cache, network, idx, key_index)
            // D4b: cache hit -> 33 bytes -> PublicKey, ZERO seed access
```

The chooser needs the **derivation path** too (to store in `KeyStorage`). The path is a pure
function of `(network, identity_index, key_index)` —
`DerivationPath::identity_authentication_path(network, ECDSA, identity_index, key_index)` — and
needs **no seed**. So the chooser computes `entry = { public_key: <cached>, derivation_path:
<pure>, … }` entirely seed-free.

**Cold-cache caveat (inherited from D4b §4.5).** A cold `AuthPubkeyCache` cannot serve a never-seen
`(identity, key)` tuple without one JIT seed-derivation. The chooser runs in sync `ui()` and cannot
`await`. Two acceptable resolutions, in preference order:

1. **Warm-before-show (recommended).** When a wallet is selected/unlocked or the identity index
   changes, the screen dispatches an async `WalletTask::WarmIdentityAuthPubkeys { seed_hash,
   identity_index, key_count }` that resolves the needed pubkeys via D4b's cold-fill (one
   `with_secret_session`, one prompt for a protected wallet — but this *is* the unlock the user just
   performed) and populates the cache. The chooser then reads the warm cache synchronously on the
   next frame. Display shows "Preparing identity keys…" until the warm result lands. This mirrors the
   existing `refresh_banner` + `display_task_result` lifecycle every other screen uses.
2. **Lazy cache via D4b §4.4.** If `bootstrap_wallet_addresses_jit` already warms the identity-auth
   ranges at unlock (D4b-4), the cache is warm for index 0..N by the time the chooser opens, and
   step 1 is only needed for an *advanced-mode* high identity index the bootstrap didn't cover.

Either way, the chooser's `ensure_correct_identity_keys` / `update_identity_key` / `add_identity_key`
become **pure, seed-free, synchronous** cache reads (option 1's warm task is the only async hop, and
it is an ordinary backend task, not a derivation inside `ui()`).

### 2.3 The chooser methods, after

| Method | Before | After |
|---|---|---|
| `ensure_correct_identity_keys` | derives master + 5 private keys via `:1112` | reads master + 5 **public** keys from `AuthPubkeyCache` (+ pure paths); builds `IdentityKeyEntry`s. Returns a "cache cold — warming" signal if a tuple is missing, which the screen turns into a `WarmIdentityAuthPubkeys` task. |
| `update_identity_key` | re-derives private keys for new index | re-reads public keys for new index from cache (warm-if-cold). |
| `add_identity_key` | derives one private key for the next index | reads one public key for the next index from cache (warm-if-cold). |

### 2.4 Private keys materialized JIT at registration — already done, restated for the record

No change to the signing path is required. At registration:

- The chosen key set is `IdentityKeys` (public + paths). `to_public_keys_map` → the keys submitted.
  `to_key_storage` → `KeyStorage` with `AtWalletDerivationPath` entries keyed by the same paths.
- `QualifiedIdentity::sign` maps `key_id` → `(target, key_id)` → `wallet_seed_hash_for` →
  `with_secret(HdSeed{seed_hash})` → `get_resolve_with_seed(seed)` →
  `derive_private_key_in_arc_rw_lock_slice_with_seed(…, derivation_path, …)`. **This is where the
  private key for each chosen public key is materialized** — JIT, borrowed seed, derived at the
  stored path, used to sign, dropped. The mapping "chosen public key (chooser) → private key (sign
  time)" is the `derivation_path` carried in `IdentityKeyEntry` → `WalletDerivationPath`. Identical
  path in, identical key out (BIP-32 determinism); D4b §1 proves the public pre-image matches the
  private derivation byte-for-byte.

So the public key the user sees in the chooser and the private key that signs at registration are
the two faces of the same `derivation_path`. The chooser never holds the private face.

---

## 3. The signer — does identity-create go through `DetSigner`/the chokepoint?

**Yes, two signers, both already on the chokepoint. No new signer path is needed.**

| Signer | Trait | Role in identity-create | Seed source today |
|---|---|---|---|
| `QualifiedIdentity` | `Signer<IdentityPublicKey>` (`mod.rs:316`) | signs with each **identity key being added**, proving ownership | JIT: `with_secret` + `get_resolve_with_seed` (already seed-free) |
| `DetSigner` | upstream `key_wallet::signer::Signer` (path-indexed) | signs the **asset-lock credit-output** witness | JIT: `DetSigner::from_held(session.plaintext(), network)` in `register_identity` |
| `DetPlatformSigner` | `Signer<PlatformAddress>` | signs the **platform-address funding** witness (address-funded flow only) | JIT: `from_held(seed, …)` (D3) |

The identity-ownership signature — the one the brief flags ("the keys being added must sign to prove
ownership") — flows through `QualifiedIdentity::sign`, which is already the seed-free JIT path. The
"JIT private-key provision for it" the brief asks me to design is `get_resolve_with_seed`, which D2
already landed and which `QualifiedIdentity::sign` already calls. **D4c adds nothing here; it only
ensures the chooser stops pre-deriving the same keys eagerly.**

> A note on the alternative I rejected: one could imagine giving the chooser a `DetSigner`-style
> handle and deferring even the public derivation. Unnecessary — the public keys are cheap, cacheable
> (D4b), and the chooser genuinely must show/submit them. The private derivation is the only thing
> worth deferring, and it is already deferred to sign time.

---

## 4. The advanced-mode WIF display — the one true residual private read

`render_keys_input` (`add_new_identity_screen/mod.rs:608, 655`) shows `Secret::new(key.to_wif())`
for the master and each key, in **advanced** mode only. This is the sole place the chooser's private
keys were ever *displayed*. After §2 the chooser holds no private keys, so this must change.

**Design (mirror D3's `WalletTask::DeriveKeyForDisplay`).** The WIF column becomes a per-row "Show
WIF" affordance that dispatches `WalletTask::DeriveKeyForDisplay { seed_hash, derivation_path }`
(the task already exists from D3, `backend_task/wallet/derive_key_for_display.rs`). The backend
resolves the seed via `with_secret`, derives at the path, returns the WIF as a typed
`BackendTaskSuccessResult`; the screen renders it from `display_task_result` into a transient,
copyable field (zeroized on screen close). No seed crosses into `ui()`.

This is consistent with the existing trust boundary (the WIF was already shown on screen) and with
the merged D3 precedent. It also *improves* UX slightly: the WIFs are no longer derived eagerly for
every row on every frame; they are derived on demand.

> **Open design choice (Q1).** Advanced-mode pre-registration WIF display is a niche power-user
> feature (inspect the keys before creating the identity). An alternative is to **drop** the WIF
> column entirely from the *pre-registration* chooser and rely on the post-registration key viewer
> (key_info_screen, already a backend task) to show WIFs after the identity exists. That removes the
> last private-derivation entry point from this screen with zero new task wiring. I lean toward
> dropping it (simpler, and the keys are recoverable post-registration), but it is a product call.

---

## 5. e2e `build_identity_registration` handling

There are **two** helpers named `build_identity_registration`:

### 5.1 Production helper (`backend_task/identity/mod.rs:484`)

Sync, calls `:1112` six times to build `IdentityKeys`. Convert to the same public shape as the
chooser: derive the six **public** keys (cache or, for tests, a direct seed-param public derivation)
and the pure paths. Because tests run headless and may want determinism without a warm cache, give it
a **seed-param** form (mirroring the D2 `*_with_seed` pattern the brief calls out):

```rust
// public-only, seed-param — derives public keys + paths from a borrowed seed
pub(crate) fn build_identity_registration_with_seed(
    app_context, wallet_arc, seed: &[u8; 64], identity_index, funding_amount,
) -> Result<IdentityRegistrationInfo, TaskError>
```

The async caller resolves the seed once via `with_secret_session` and calls this. A thin async
wrapper `build_identity_registration` (no seed param) opens the scope and delegates — so existing
call sites that can `await` need only add `.await`.

### 5.2 Test-framework helper (`tests/backend-e2e/framework/identity_helpers.rs:24`)

Returns `(IdentityRegistrationInfo, Vec<u8> master_key_bytes)`. **The `master_key_bytes` is dead
weight:** every consumer binds it as `_master_key_bytes` / `_signing_key_bytes` or stores it inert
(`fixtures.rs:86`, `token_tasks.rs:65`); the *actual* signing in tests uses the **public**
`signing_key` from `find_authentication_public_key(&qi)`, never the raw bytes. So:

- **Drop the `Vec<u8>` from the return type** (or return an empty/placeholder for the smallest diff).
  Update the three struct fields (`SharedIdentity.signing_key_bytes`, `SharedToken`,
  `token_tasks` local) to not require it — they already don't use it to sign.
- Convert the helper to the seed-param public build (§5.1). Tests drive it with the test wallet's
  seed (which the harness already has, since it creates the funded wallet) — **no parked seed
  needed**. This is the "give it an async variant or a seed-param form so tests can drive it without
  the parked seed" the brief asks for.

This is a mechanical test refactor with no live-network behavior change: the registration task it
feeds is unchanged; only the *input construction* moves from private to public.

---

## 6. Removing `:1112`'s parked-seed read — the D4 gate

After §2–§5, the callers of `identity_authentication_ecdsa_private_key` are:

| Caller | Status after D4c |
|---|---|
| `add_new_identity_screen` ×5 | **Gone** — chooser uses public keys (§2); WIF display via backend task (§4). |
| `backend_task/identity/mod.rs:496,510` (`build_identity_registration`) | **Gone** — public seed-param build (§5.1). |
| `tests/.../identity_helpers.rs:37,47` | **Gone** — public seed-param build (§5.2). |

With zero remaining callers, `identity_authentication_ecdsa_private_key` itself becomes dead and is
**deleted** (along with its `register_address_from_private_key` side-effect call — note the public
counterpart `register_address_from_public_key` already exists at `mod.rs:1146` and is what the
public-key data-map path uses, so address registration is preserved seed-free). Its deletion removes
one `self.seed_bytes()?` read at `mod.rs:1112`. Combined with D4b (the public readers at 1045/1069)
and D1–D3, this drains the identity-key reader population — satisfying the parent's **R-5 compile-time
gate**: once `OpenWalletSeed.seed` is removed in D4, any surviving `seed_bytes()` caller fails to
compile, and there are none left here.

> **Scope boundary.** D4c does **not** itself delete `OpenWalletSeed.seed` — that is D4's job, gated
> on D1+D2+D3+D4b+D4c all draining their readers. D4c's deliverable is "the identity-key chooser and
> its helpers no longer read the parked seed for private keys," making `:1112` deletable.

---

## 7. UX delta — does the user see a new prompt at registration?

**No new prompt at the registration click.** The registration backend task already opens exactly one
`with_secret_session` (it has since D2/D3); the identity-ownership and asset-lock signatures share
that one scope. A protected wallet prompts once there today; that is unchanged.

**Where the prompt moves (slightly earlier, and only for protected wallets):**

- Today the chooser derives private keys eagerly in `ui()` — which requires the wallet to be
  *already open* (the chooser gates on `wallet.is_open()` and shows an "Unlock Wallet" button
  first). So a protected wallet is **already unlocked before the chooser populates keys**. The seed
  is in the session cache.
- After D4c, the chooser reads **public** keys (cache). If the cache is cold (§2.2), the
  warm-before-show task runs inside the *already-open* session — **no new prompt** (the unlock
  already happened; the chokepoint serves from the session cache, resolution order step 1, per the
  recorded `with_secret` semantics). For an *unprotected* wallet, no prompt ever.

**Net UX delta:** none visible for the common path. The eager per-frame private derivation (and its
implicit "wallet must be open" coupling) is replaced by a cache read plus, at worst, a one-frame
"Preparing identity keys…" banner on a cold cache. The single registration prompt is unchanged.
**This is acceptable and arguably better** — the chooser no longer holds private keys in memory
across the whole screen lifetime (a residency reduction), and key display is on-demand.

One honest caveat: if we adopt option-2 warming (bootstrap-at-unlock) and a power user picks an
identity index *beyond* the bootstrapped range in advanced mode, the cold-fill warm task runs then.
For a protected wallet whose session has since been forgotten (e.g. `RememberPolicy` expired), that
could surface a prompt mid-chooser. Mitigation: the chooser already requires `is_open()`; if the
session is gone it shows the unlock button first, so the prompt is the normal unlock, not a surprise.

---

## 8. Risk assessment — this touches identity registration

| # | Risk | Severity | Mitigation |
|---|---|:---:|---|
| RK-1 | **Public-key ↔ private-key path divergence.** If the chooser stores a `derivation_path` that doesn't match the public key it displays, `to_key_storage` would persist a path whose JIT-derived private key signs with a *different* key than the one submitted in `to_public_keys_map` → identity-create rejected (key ownership proof fails) or, worse, an identity created with keys the wallet can't sign with. | **High** | Derive `public_key` and `derivation_path` from the **same** `(network, identity_index, key_index)` in one place (a single `IdentityKeyEntry` constructor). D4b §1 guarantees public-from-cache == public-from-private-derivation byte-for-byte. Add a Smythe parity test: for a range of indices, `cache pubkey == public(get_resolve_with_seed(path))`. |
| RK-2 | **Cold-cache silent fallback to wrong/empty key.** A cache miss that returns `Default`/empty instead of warming would build an `IdentityKeys` with a zero/garbage pubkey. | **High** | The chooser must treat a miss as "not ready" (warm task), **never** as a usable key. `register_identity_clicked` already guards `master_private_key.is_some()`; the equivalent guard becomes "all entries present and non-placeholder." Fail closed: no warm cache ⇒ registration button disabled. |
| RK-3 | **e2e regression from dropping `master_key_bytes`.** A hidden consumer might actually sign with the raw bytes. | **Medium** | Verified: no non-underscore consumer signs with the bytes (all use the public `signing_key`). Land the e2e change behind a full `cargo test --test e2e` + one live-network identity-create run (the registration path is unchanged, so the risk is purely compile/plumbing). |
| RK-4 | **Advanced-mode WIF task leaks the key beyond the screen.** Routing a WIF through a `BackendTaskSuccessResult` widens its reach. | **Medium** | Reuse D3's `DeriveKeyForDisplay` exactly (it already round-trips a WIF for the wallets screen — same trust boundary, already reviewed). Wrap in the `Secret` newtype end-to-end; zeroize the transient field on screen close. Or adopt Q1 (drop pre-registration WIF entirely). |
| RK-5 | **Removing `:1112` before all callers drained** (esp. a missed test caller) → "wallet closed" at runtime once D4 lands. | **Medium** | Same R-5 compile-time gate as the parent: deleting `OpenWalletSeed.seed` makes `seed_bytes()` un-implementable; any surviving caller fails to compile. Sequence `:1112` deletion in D4c, but the *field* removal stays in D4. `grep identity_authentication_ecdsa_private_key` must return zero before D4. |
| RK-6 | **Bootstrap/address-registration side-effect lost.** `:1112` calls `register_address_from_private_key` (registers the p2pkh address into `known_addresses`/`watched_addresses`). Dropping it could lose an address registration the wallet relied on. | **Low** | The public data-map path (`identity_authentication_ecdsa_public_keys_data_map`, mod.rs:1051) already calls `register_address_from_public_key` for the same paths during identity load/discover (D4b's domain). The address derives identically from the public key (p2pkh). So registration is preserved on the public path; verify no flow depended *only* on the private-key call to register an identity-auth address (load/discover cover it). |
| RK-7 | **Warm task races the render.** The chooser reads the cache the same frame the warm task is still in flight. | **Low** | Standard `display_task_result` lifecycle: the screen shows "Preparing…" until the success result lands, then re-reads. egui re-renders on result arrival. No correctness risk — only a one-frame delay. |

**Overall:** the fund-critical surface (the actual signing) is **untouched** — it is already JIT and
stays exactly as-is. D4c's risk is concentrated in RK-1/RK-2 (public/path consistency on the
input-construction side), which a single parity test closes decisively. This is a *lower*-risk change
than D3 (which moved live signers) precisely because the signer paths don't move.

---

## 9. Task breakdown for Bilby

> D4c depends on **D4b** (the `AuthPubkeyCache` + public reader conversion) being available — the
> chooser's public keys come from that cache. D4c is otherwise independent of D1/D2/D3 and, like D4b,
> can land before D4's field removal. Sequence: D4b → D4c → (D1,D2,D3 in parallel) → D4.

| Task | Title | Files | Depends on | Smythe? | Live e2e? |
|---|---|---|---|:---:|:---:|
| **D4c-1** | Reshape `IdentityKeys` to public-only: `IdentityKeyEntry { public_key, derivation_path, key_type, purpose, security_level, contract_bounds }`; rewrite `to_public_keys_map` + `to_key_storage` to read `entry.public_key` (no `PrivateKey`); keep stored `WalletDerivationPath` identical | `backend_task/identity/mod.rs` (struct + the two builders) | D4b-1 (uses `PublicKey` cache type conventions) | **Yes** (key-set correctness; the submitted pubkeys + stored paths gate signing) | No |
| **D4c-2** | Chooser public-key sourcing: rewrite `ensure_correct_identity_keys` / `update_identity_key` / `add_identity_key` to read public keys from `AuthPubkeyCache` + pure paths; add `WalletTask::WarmIdentityAuthPubkeys` (warm-before-show) + screen `display_task_result` wiring + "Preparing identity keys…" banner; fail-closed registration guard on cold cache (RK-2) | `ui/identities/add_new_identity_screen/mod.rs`, `backend_task/wallet/mod.rs` (+warm task), `backend_task/wallet/*` (warm impl) | D4b-2, D4b-3, D4c-1 | **Yes** (cold-cache fail-closed + warm correctness) | No |
| **D4c-3** | Advanced-mode WIF: route the WIF column through `WalletTask::DeriveKeyForDisplay` (or, per Q1, drop the pre-registration WIF column) | `ui/identities/add_new_identity_screen/mod.rs` (`render_keys_input`) | D4c-1; reuses D3's `DeriveKeyForDisplay` | Advisory (reuses merged D3 path) | No |
| **D4c-4** | Production `build_identity_registration` → public seed-param form + async wrapper; update its callers | `backend_task/identity/mod.rs:484`, `register_identity.rs` dispatch (if it calls it) | D4c-1 | **Yes** (the canonical test/UI registration prep) | No |
| **D4c-5** | e2e helper: public seed-param `build_identity_registration`; drop dead `master_key_bytes`/`signing_key_bytes`; update `fixtures.rs`, `dashpay_helpers.rs`, `token_tasks.rs`, and the `_*` call sites | `tests/backend-e2e/framework/identity_helpers.rs`, `framework/fixtures.rs`, `framework/dashpay_helpers.rs`, `backend-e2e/token_tasks.rs`, `backend-e2e/{identity_create,identity_withdraw,register_dpns,identity_tasks}.rs` | D4c-4 | Advisory | **Yes** — one live identity-create run to confirm the public-input path registers + signs (signing path unchanged, but prove it end-to-end) |
| **D4c-6** | Delete `identity_authentication_ecdsa_private_key` (+ its now-unused `register_address_from_private_key` if no other caller); parity test `cache_pubkey == public(derive_with_seed(path))` over an index range (RK-1) | `model/wallet/mod.rs` (delete `:1092–1125`), `tests/` (parity) | D4c-1..5 (all callers drained) | **Yes** (the deletion + parity is the load-bearing correctness claim) | No |

**Task count: 6.** D4c-1 and D4c-6 are the Smythe-critical correctness pair (input shape + the parity
that guarantees public matches private). D4c-5 is the only one needing live network, and only to
confirm an unchanged signing path still works through the reshaped input.

---

## 10. Open questions for the user

1. **Advanced-mode WIF (RK-4 / §4).** Keep pre-registration WIF display via a `DeriveKeyForDisplay`
   task, or **drop** it and rely on the post-registration key viewer? I lean toward dropping (removes
   the last private-derivation entry from this screen; keys are recoverable after registration), but
   it removes a power-user convenience.
2. **Cold-cache strategy (§2.2).** Warm-before-show task (option 1, self-contained in D4c) vs. rely
   on D4b-4 bootstrap warming at unlock (option 2, smaller D4c but couples to D4b-4 landing). I
   recommend implementing option 1 regardless, since it also covers advanced-mode high indices the
   bootstrap range won't reach.
3. **e2e `master_key_bytes` removal (§5.2).** Confirm dropping the `Vec<u8>` return is acceptable
   (verified unused for signing). If you'd rather keep the tuple shape for diff-minimization, I can
   return a placeholder instead — but the cleaner change deletes it.
4. **`IdentityKeys` rename.** With no private keys, `IdentityKeys` is a misnomer (it's now identity
   *key specs* + public material). Rename to `IdentityKeySpecs` / `IdentityKeySet`, or keep the name
   to minimize churn? (Cosmetic; I lean to renaming for honesty, per M-CONCISE-NAMES, but it widens
   the diff.)

---

## 11. Security review (security-best-practices skill)

- **ASVS V11.7 (data-in-use) / V13.3 (secret management):** D4c **reduces** plaintext-private-key
  residency. Today the chooser holds derived `PrivateKey`s for the entire screen lifetime; after, it
  holds only public keys. Private material exists only inside `with_secret`/`with_secret_session`
  frames at sign time (and, if kept, a transient WIF behind `Secret`). Net: a residency *removal*,
  not addition. (Candy-worthy — the chooser stops being a long-lived private-key holder.)
- **No private key caching (R3 invariant, D4b §6):** D4c stores **public** keys (D4b cache) and
  **paths** only. The path is not secret (it is derivable structure); the private key is re-derived
  JIT and never persisted. Upheld.
- **A04 Insecure Design / least privilege:** the chooser is granted exactly what it needs (public
  keys to display + submit) and no more. The eager private derivation was over-privileged for a
  display/submit surface.
- **Network confinement (SEC-001 class):** the derivation path carries the per-network coin-type and
  the D4b cache is network-keyed; the public key shown and the private key signed share the path, so
  cross-network key reuse stays structurally excluded.
- **Integrity (RK-1):** the one real correctness hazard — public/path divergence — is closed by the
  single-constructor rule + the D4c-6 parity test. Treat that test as a release gate.

---

## 12. Candy tally (architecture findings by severity)

| Severity | Count | Findings |
|---|:---:|---|
| **High** (design-shaping / fund-adjacent) | **2** | (1) Identity-create signing is **already** seed-free JIT (`QualifiedIdentity::sign` + `DetSigner`/`DetPlatformSigner`) — the redesign is "stop deriving private keys in the chooser," not "add JIT at registration." (2) Public-key↔derivation-path consistency (RK-1) is the load-bearing correctness invariant; a single-constructor rule + parity test closes it. |
| **Medium** (structural) | **2** | `IdentityKeys` reshapes to public-only (`to_key_storage` already discards the `PrivateKey`); cold `AuthPubkeyCache` needs a warm-before-show task because the chooser is sync `ui()`. |
| **Low** (mechanical / safety) | **3** | e2e `master_key_bytes` is dead weight (no signing consumer) — droppable; advanced-mode WIF is the only residual private read — reuse D3's `DeriveKeyForDisplay`; `register_address_from_public_key` already exists so address registration stays seed-free. |

**Total: 7 findings (2 High, 2 Medium, 3 Low).**

---

*The hardest problems are sometimes the ones already solved by someone who came before — you only
notice once you trace the whole chain. The seed was never needed in the chooser; it was just being
asked for out of habit. Stop asking, and the chooser goes quiet, the prompt stays exactly where it
was, and the only key left is the one the chokepoint already holds.* — Nagatha
