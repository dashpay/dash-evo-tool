# Rehydration Re-Wire — `UpstreamFromPersisted` onto PR #3692 (seedless watch-only load)

Closes **PROJ-010** — the reserved `UpstreamFromPersisted` swap point in
`src/wallet_backend/loader.rs`. Re-wires DET's wallet-load logic onto the
upstream **seedless / watch-only** rehydration API added by dashpay/platform
PR #3692.

> Source-of-truth correction: the task brief cited head `9e2d2b0d` and an API
> with a `SeedProvider` port. The PR was **rebuilt 2026-05-25** (rebuild note in
> the PR body); the live head at design time was
> **`ddfa66ed373beaebdae9a5d919f896af43cbcd33`** and the API is **purely
> seedless** — the `SeedProvider` trait was deleted and `load_from_persistor()`
> takes **no resolver**. All upstream `file:line` citations in this document are
> at `ddfa66ed`.
>
> **⚠ Pin moved past `ddfa66ed` (read before chasing line numbers):** the
> shipped `Cargo.toml` pin is now `rev = 9e1248cb` (`platform 4.0.0-beta.2`,
> PR #3692 head). The seedless API *shape* this document targets is unchanged
> at `9e1248cb`, but the exact `file:line` citations below are at `ddfa66ed` and
> may have drifted. Re-pinning to a tagged release is the open release gate
> (PROJ-005 / F121 in the gap audit). Resolve symbols by name, not by line, when
> reading against the shipped pin.

> **⚠ Regression correction (2026-06-08):** this re-wire deleted DET's *only*
> persistor **writer** along with `SeedReregistrationLoader`. `load_from_persistor`
> is **read-only** — it brings back only what the persistor already holds. With no
> writer left, the persistor stayed empty on fresh / reset / migrated /
> sidecar-only installs, the SPV watch set was empty, and received funds were
> invisible at 100% sync (PROJ-010, HIGH). The seedless *read* path in this
> document is correct and unchanged; what was missing is a **write** path. The fix
> (`docs/ai-design/2026-06-08-wallet-reregistration-fix/design.md`) re-introduces
> the persistor write at the two seed-bearing moments —
> `WalletBackend::register_wallet_from_seed` (W1, create/import) and
> `WalletBackend::ensure_upstream_registered` (W2, cold-boot reconciliation) —
> with a genesis birth-height floor for imported/recovered/migrated wallets. Read
> the headline findings below with that in mind: "comes back at launch" holds only
> **once the persistor has been written** by W1/W2.

---

## 0. Headline findings (read first)

1. **The upstream API is seedless and watch-only.** `load()` no longer needs a
   seed and produces `Wallet::new_watch_only` per wallet. This is a *security
   upgrade* over `SeedReregistrationLoader`: balances/UTXOs/identities/contacts
   come back at launch with **no seed in memory**.
2. **The pin switch is SAFE, not risky.** `git compare ffdc28b8...ddfa66ed`
   ⇒ **`ahead_by: 59, behind_by: 0, status: ahead`**. The PR head is a clean
   *superset* of our exact current pin (`ffdc28b8`, the PR's own base branch).
   The brief's "ahead 53 / behind 63 / CONFLICTING" was measured against
   `v3.1-dev`, which the PR body confirms was *already merged in*. We are not
   missing the SPV-stop deadlock fix, the KvStore TOCTOU fix, or v3.1-dev —
   they are all in `ddfa66ed`.
3. **The `PersistedWalletLoader` trait shape MUST change.** Its current
   contract (`Vec<WalletRegistration>` carrying `seed_bytes`) is intrinsically
   seed-driven and feeds two things: the per-wallet
   `create_wallet_from_seed_bytes` loop *and* DET's `inner.seeds` signing cache.
   The seedless API populates wallets in **one** manager call and supplies **no**
   seeds. The loader must become a "load strategy" seam, not a "seed list" seam.
4. **DET keys everything by `WalletSeedHash = SHA256(seed)`; upstream returns
   `WalletId = SHA256(root_xpub‖chaincode)`.** The seedless path cannot produce
   `WalletSeedHash`. Bridge: DET already persists `xpub_encoded` (the 78-byte
   root xpub) in the wallet-meta + seed-envelope sidecars
   (`src/wallet_backend/hydration.rs`, `wallet_seed_store.rs`), so DET can
   compute `WalletId` from the xpub **without the seed** and build a
   `WalletId → WalletSeedHash` map at hydration time. This is the linchpin of
   the flip.

---

## 1. Upstream rehydration API surface (the exact types/functions DET calls)

All in crate `platform-wallet` at `ddfa66ed`.

### 1.1 Entry point — one call, seedless

`packages/rs-platform-wallet/src/manager/load.rs:48`
```rust
impl<P: PlatformWalletPersistence + 'static> PlatformWalletManager<P> {
    pub async fn load_from_persistor(&self) -> Result<LoadOutcome, PlatformWalletError>;
}
```
Internally: calls `self.persister.load()` → keyless `ClientStartState`; for each
persisted wallet builds a watch-only `Wallet` from its account manifest
(`rehydrate::build_watch_only_wallet`), applies the keyless core-state
projection (`rehydrate::apply_persisted_core_state`), layers public
contacts + identity keys, and registers the wallet into `self.wallets` /
`self.wallet_manager`. **No seed, no `create_wallet_from_seed_bytes`.**

Transactional: a whole-load failure rolls back every wallet inserted in the
pass; a per-row decode failure *skips* that wallet (continues the batch).

### 1.2 Result type — `LoadOutcome`

`packages/rs-platform-wallet/src/manager/load_outcome.rs` (re-exported
`platform_wallet::{LoadOutcome, SkipReason}`; `CorruptKind` at
`platform_wallet::manager::load_outcome::CorruptKind`):
```rust
pub struct LoadOutcome {
    pub loaded:  Vec<WalletId>,                  // WalletId = [u8; 32]
    pub skipped: Vec<(WalletId, SkipReason)>,    // non-empty skipped is SUCCESS
}
pub enum SkipReason { CorruptPersistedRow { kind: CorruptKind } }
pub enum CorruptKind { MissingManifest, MalformedXpub, DecodeError(String) }
```
`Ok(LoadOutcome)` with a non-empty `skipped` is success; `Err` is reserved for
whole-load failures (persister I/O, the no-silent-zero topology check).

### 1.3 Accessors used after load

`packages/rs-platform-wallet/src/manager/accessors.rs`
```rust
pub async fn get_wallet(&self, wallet_id: &WalletId) -> Option<Arc<PlatformWallet>>;
pub async fn wallet_ids(&self) -> Vec<WalletId>;
```

### 1.4 Skip event (additive, non-breaking)

`packages/rs-platform-wallet/src/events.rs:32` adds
`PlatformEvent::WalletSkippedOnLoad { wallet_id, reason }` and a
`PlatformEventHandler::on_platform_event(&self, _event)` method **with a default
no-op impl** — DET's `EventBridge` (`src/wallet_backend/event_bridge.rs:168`)
keeps compiling untouched; it *may* override it to surface skips.

### 1.5 Manager construction — unchanged

`packages/rs-platform-wallet/src/manager/mod.rs:99`
```rust
pub fn new(sdk: Arc<Sdk>, persister: Arc<P>, app_handler: Arc<dyn PlatformEventHandler>) -> Self;
```
Identical to today's `PlatformWalletManager::new(sdk, persister, bridge)` call at
`src/wallet_backend/mod.rs:263`. **No construction-signature change upstream.**

### 1.6 Host calling pattern (from the e2e RT suite)

`packages/rs-platform-wallet/tests/rehydration_load.rs` (item E) — the canonical
host flow:
```rust
let mgr = Arc::new(PlatformWalletManager::new(sdk, persister, handler));
let outcome = mgr.load_from_persistor().await?;   // seedless
for id in &outcome.loaded { assert!(mgr.get_wallet(id).await.is_some()); }
// outcome.skipped: corrupt rows, one PlatformEvent::WalletSkippedOnLoad each
```

---

## 2. `UpstreamFromPersisted` impl design

### 2.1 The trait must change shape

Today (`src/wallet_backend/loader.rs:57`):
```rust
pub trait PersistedWalletLoader: Send + Sync {
    fn wallets_to_register(&self, ctx: &Arc<AppContext>) -> Result<Vec<WalletRegistration>, TaskError>;
}
```
This returns seed-bearing descriptors and assumes the backend will call
`create_wallet_from_seed_bytes` per item. The seedless API breaks both
assumptions. The seam must move "up" one level — from *"give me the seeds to
re-register"* to *"perform the load and tell me which wallets came back"*.

**New trait shape (object-safe, async — mirrors the single-key seam):**
```rust
/// Outcome of a persisted-wallet load pass, mapped to DET-opaque types.
/// Carries NO upstream platform-wallet types (M-DONT-LEAK-TYPES).
pub struct LoadedWallets {
    /// Wallets now registered with the backend, keyed by DET's seed hash.
    pub loaded: Vec<WalletSeedHash>,
    /// Wallets present on disk but skipped (corrupt row). DET-opaque reason.
    pub skipped: Vec<(WalletSeedHash, PersistedLoadSkip)>,
}

/// DET-opaque skip reason — maps upstream CorruptKind without leaking it.
pub enum PersistedLoadSkip { MissingManifest, MalformedXpub, DecodeError }

#[async_trait::async_trait]               // backend is already async
pub trait PersistedWalletLoader: Send + Sync {
    /// Bring persisted wallets back into the running backend.
    async fn load(&self, ctx: &WalletBackendLoadCtx<'_>) -> Result<LoadedWallets, TaskError>;
}
```
`WalletBackendLoadCtx` is a small DET-internal struct the backend hands the
loader, exposing exactly what each impl needs **without leaking upstream types**:
- `&Arc<AppContext>`
- the `WalletId → WalletSeedHash` bridge builder (computed from sidecar xpubs)
- a callback to register a resolved `(WalletSeedHash, WalletId)` into the
  backend's `id_map` / `wallets` / `snapshots` (so the impl never touches those
  maps directly, preserving the seam).

> Rationale: keeping `wallets_to_register` and bolting the seedless call beside
> it would split load logic across two seams and force the seed-keyed bridge
> into `WalletBackend::new`. A single async `load(...)` keeps the strategy fully
> behind the trait — exactly what G2.4 promised ("only the loader impl + one
> construction line").

### 2.2 `UpstreamFromPersisted::load` algorithm

```
1. Build the WalletId → WalletSeedHash bridge from DET sidecars (seedless):
     for each (seed_hash, meta) in WalletMetaView.list(network):
         root_xpub = ExtendedPubKey::decode(meta.xpub_encoded)   // already persisted
         wallet_id = upstream_wallet_id(root_xpub)               // SHA256(xpub‖chaincode)
         bridge.insert(wallet_id, seed_hash)
   (Same xpub decode hydration.rs already does at lines 111-114, 273.)
2. outcome = pwm.load_from_persistor().await    // ONE seedless call
3. for wallet_id in outcome.loaded:
     seed_hash = bridge.get(wallet_id)           // resolve DET key
     pw        = pwm.get_wallet(&wallet_id).await
     ctx.register_resolved(seed_hash, wallet_id, pw)   // id_map/wallets/snapshots
4. map outcome.skipped[(wallet_id, SkipReason)] → (seed_hash, PersistedLoadSkip)
5. return LoadedWallets { loaded, skipped }
```
**No seed touched in any step.** Asset-lock signers and DashPay derivation are
*not* populated here — they remain seed-driven and lazy (§5).

### 2.3 Type mapping (kept inside `loader.rs` — M-DONT-LEAK-TYPES)

| Upstream (PR #3692)                       | DET-opaque                                 |
|-------------------------------------------|--------------------------------------------|
| `WalletId` (`[u8;32]`)                    | resolved to `WalletSeedHash` via bridge    |
| `LoadOutcome.loaded`                      | `Vec<WalletSeedHash>`                       |
| `SkipReason::CorruptPersistedRow{kind}`   | `PersistedLoadSkip`                         |
| `CorruptKind::{Missing/Malformed/Decode}` | `PersistedLoadSkip::{...}` (string dropped) |
| `PlatformEvent::WalletSkippedOnLoad`      | optional `EventBridge` → typed `TaskError`/log |

`WalletId`, `PlatformWallet`, `LoadOutcome` never escape `loader.rs` / `mod.rs`.

### 2.4 Seedless behavior vs seed behavior

- **Seedless (this impl):** runs at launch with the Keychain locked equivalent —
  no password prompt needed to *see* funds. Display surfaces (balance, UTXO,
  identity, contacts, asset-locks) populate from disk.
- **Seed still required for signing:** populated lazily into `inner.seeds` on
  the existing unlock path, not by the loader (§5). The loader's job ends at
  watch-only registration.

---

## 3. Construction flip

### 3.1 The one-line swap

`src/context/mod.rs:655` (inside `ensure_wallet_backend`):
```rust
- let loader = Arc::new(SeedReregistrationLoader::new());
+ let loader = Arc::new(UpstreamFromPersisted::new());
```
Plus the import at `src/context/mod.rs:25` and the re-export at
`src/wallet_backend/mod.rs:58` (swap the exported name).

### 3.2 `WalletBackend::new` consequence

`register_persisted_wallets` (`src/wallet_backend/mod.rs:348`) changes from
"iterate seed-bearing registrations and call `create_wallet_from_seed_bytes`" to
"call `loader.load(...)` once and consume `LoadedWallets`". The identity-funding
re-provision recurrence trap (a5538dc8, `mod.rs:428-443`) **stays** — it runs
per loaded `seed_hash` exactly as today, since the seedless `load()` still does
NOT reconstruct identity funding HD accounts (that gap is unchanged by #3692).

### 3.3 Does `SeedReregistrationLoader` stay?

**Remove it** (M-NO-TOMBSTONES), per G2.4 step 4 — once the upstream API is the
real path, the seed-re-registration mock is dead weight. **One caveat for the
user (§7 Q1):** the seedless path is watch-only; if any current launch-time
behavior *depends* on the seed being in `inner.seeds` immediately after load
(rather than after unlock), removing the seed-driven loader changes timing. Audit
shows the only `inner.seeds` consumers are signing paths (`signer_for`,
`derive_private_key`, `send_payment`, DashPay derivation) — all user-initiated,
post-unlock — so removal is safe. Keep the single-key reserved-impl mirror
pattern intact (the trait stays object-safe with one shipping impl).

---

## 4. Divergence & compile-risk assessment

### 4.1 Pin switch verdict: **SAFE**

`ffdc28b8 → ddfa66ed` is **ahead 59, behind 0** (verified via GitHub compare
API). The new pin is a strict superset of the current one. The PR adds 36 files,
**+2782/-338**, almost entirely *additive* readers and the new load path.

### 4.2 What the pin switch touches that DET consumes

| DET consumer | Upstream surface | In PR diff? | Impact |
|---|---|---|---|
| `DetKv` (`kv.rs`) → `KvStore`, `KvError`, `ObjectId` | `platform_wallet_storage` kv/object_id | **No** | none |
| `single_key.rs` → `secrets::{SecretStore, FileStoreError, SecretBytes, SecretString, WalletId}` | `platform_wallet_storage::secrets` | **No** | none |
| `WalletBackend::new` → `SqlitePersister`, `SqlitePersisterConfig` | `persister.rs` | **Yes (additive)** | `load()` now reconstructs `ClientStartState.wallets`; `LOAD_UNIMPLEMENTED` shrinks to `core::last_applied_chain_lock`. No signature change to `open`/`store`/`flush`. |
| `EventBridge` → `PlatformEventHandler` | `events.rs` | **Yes (additive)** | new trait method has a default impl ⇒ compiles unchanged |
| `pwm` construction | `manager/mod.rs::new` | unchanged sig | none |

> The brief's worry that "PR 3692 reworks `platform-wallet-storage` heavily
> (SecretStore/EncryptedFileStore/KeyringStore)" does **not** hold at `ddfa66ed`:
> the secrets rework shipped in the *base* (#3672/#3625), which is already at or
> below our pin. PR #3692's storage diff is confined to `persister.rs`,
> `schema/*` (new readers), `migrations/V001` (no V002), and tests — none of the
> `secrets` module DET imports.

### 4.3 Likely-to-need-adaptation beyond the loader

1. **`create_wallet_from_seed_bytes` call site** (`mod.rs:368-377`) — removed
   from the load path (moves entirely behind the new loader). The method still
   exists upstream for the *create-new-wallet* and *unlock-to-sign* paths; only
   the *load* usage moves.
2. **`inner.seeds` population timing** — no longer filled at load (seedless).
   Must be filled on unlock (§5). Audit who reads it pre-unlock: none today.
3. **`WalletId` bridge** — new seedless mapping in `loader.rs` (xpub→WalletId).
   `wallet_id_from_seed` (`mod.rs:975`) gets a sibling `wallet_id_from_xpub`.
4. **Skip surfacing** — `LoadedWallets.skipped` is new; UI should show a calm,
   actionable banner for corrupt rows (a new typed `TaskError` variant, §6 T4).

### 4.4 Cargo pin change

`Cargo.toml` lines 21/31/32/35 (and the `Cargo.lock` rev) flip
`ffdc28b8…` → `ddfa66ed373beaebdae9a5d919f896af43cbcd33` for `dash-sdk`,
`rs-sdk-trusted-context-provider`, `platform-wallet`, `platform-wallet-storage`.
PR #3692 is a **draft against an unreleased branch**; pin to the exact head SHA,
not a tag.

---

## 5. Fund-safety analysis (security-model delta of seedless load)

Threat model frame: A04 (insecure design — silent-zero balance), A02
(cryptographic/secret handling — seed in memory). ASVS V14.2 (data protection),
SECRETS.md.

### 5.1 What improves

- **No seed in memory at launch.** `SeedReregistrationLoader` decrypts and holds
  every wallet's 64-byte seed in `inner.seeds` from the moment the backend
  builds. `UpstreamFromPersisted` holds **zero** seed bytes to display balances.
  This shrinks the in-memory secret window from "whole session" to "only while a
  signing op is in flight" — a direct A02 improvement. The watch-only `Wallet`
  carries no key material (PR's AR-7 hygiene: only `WalletType::WatchOnly`).
- **No silent-zero balance.** Upstream `apply_persisted_core_state` fails closed
  (`RehydrationTopologyUnsupported`) rather than reconstruct a zero balance for a
  wallet with persisted UTXOs but no funds account (rehydrate.rs). DET surfaces
  this as a whole-load `Err` (calm banner), never a misleading "0 DASH".

### 5.2 What is preserved (invariants that must NOT regress)

- **Per-network coin-type derivation.** `ClientWalletStartState.network` is
  persisted and drives `build_watch_only_wallet(network, …)`. The bridge
  computes `WalletId` per-network from the per-network sidecar xpub. DET's
  `core_to_wallet_network` mapping is unchanged. No cross-network leakage.
- **published-xpub == scanned-xpub fund-routing.** The watch-only wallet is
  rebuilt from the **same** account `account_xpub` that was persisted at create
  time (the manifest), and `WalletId` is `SHA256(root_xpub‖chaincode)`. The
  bridge keys off the **same** `xpub_encoded` DET persisted. So the wallet DET
  displays/scans is provably the wallet whose xpub DET published — the routing
  invariant holds by construction. **Design gate:** the bridge MUST verify that
  the upstream `WalletId` returned in `outcome.loaded` matches a bridge entry; an
  unmatched `WalletId` (xpub mismatch) is a hard error, never a silent display of
  an unknown wallet (Smythe review item).
- **Seeds stay `Zeroizing`.** `WalletRegistration.seed_bytes` (the old
  seed-carrying type) is **deleted** with `SeedReregistrationLoader`. The new
  `LoadedWallets` carries no secret. `inner.seeds` continues to hold
  `Zeroizing<[u8;64]>`, populated only on unlock.

### 5.3 What still needs the seed (signing — unchanged)

The seed is required, exactly as today, for every operation that derives a
private key. These do **not** run at load; they run post-unlock, user-initiated:
- `signer_for` → `WalletAssetLockSigner` (`mod.rs:986`, `asset_lock_signer.rs:78`)
  — asset-lock creation, identity registration/top-up.
- `derive_private_key` (`mod.rs:1001`) — one-time credit-output keys for
  platform-address top-up / shielded deposit.
- `send_payment` (`mod.rs:1025`) — BIP-44 spend signing.
- DashPay derivation (`dashpay.rs:106-113`) — contact xpub / payment addresses.

**New requirement:** because the loader no longer fills `inner.seeds`, the
unlock flow must populate it. Cleanest placement: a small
`WalletBackend::provide_seed(seed_hash, seed_bytes)` called from the existing
wallet-unlock path (where DET already decrypts the seed). This keeps the secret
boundary in one method and the seed out of the load path entirely. If a signing
op is attempted while `inner.seeds` lacks the hash, return the existing typed
"wallet locked / unlock to sign" error (today's `WalletBackendNotYetWired` ⇒
should become a dedicated `WalletLocked`-style variant; §6 T4).

### 5.4 Net verdict

**Fund-safety: improved, no regression** — provided the §5.2 WalletId-match gate
and the §5.3 unlock-time seed provisioning are implemented. The watch-only model
is strictly less secret-exposing than seed-re-registration while preserving
coin-type and xpub-routing invariants.

---

## 6. Task breakdown (for Bilby)

Tasks are ordered; each ≥100 lines or batched. **(S)** = needs Smythe security
review.

- **T1 — Pin bump + compile baseline (batched).** Flip `Cargo.toml` lines
  21/31/32/35 to `ddfa66ed…`; refresh `Cargo.lock`; run
  `cargo clippy --all-features --all-targets -D warnings` and capture the *only*
  expected breakage (the load path). Fix nothing else yet — this isolates the
  divergence surface. Dependency: none.

- **T2 — Reshape `PersistedWalletLoader` + DET-opaque result types (S).**
  In `loader.rs`: replace `wallets_to_register` with async `load(...)`; add
  `LoadedWallets`, `PersistedLoadSkip`, `WalletBackendLoadCtx`; delete
  `WalletRegistration`'s seed field path. Keep the swap-boundary unit test
  (alternate-impl compiles). ~150 lines. (S): trait carries no upstream types;
  no secret in `LoadedWallets`. Dep: T1.

- **T3 — Implement `UpstreamFromPersisted` (S).** The §2.2 algorithm: seedless
  xpub→WalletId bridge (reuse `hydration.rs` xpub decode), one
  `load_from_persistor()` call, resolve `loaded`/`skipped` to `WalletSeedHash`,
  register via the ctx callback, **WalletId-match gate** (§5.2). Delete
  `SeedReregistrationLoader`. ~200 lines. (S): the match gate is the
  fund-routing guard — Smythe must confirm an unmatched WalletId hard-fails.
  Dep: T2.

- **T4 — Rewire `WalletBackend::new` / `register_persisted_wallets` + seed
  provisioning + typed errors (S).** Replace the per-wallet
  `create_wallet_from_seed_bytes` loop with the `loader.load(...)` call; keep the
  a5538dc8 identity-funding re-provision per loaded `seed_hash`; add
  `WalletBackend::provide_seed(...)` and call it from the unlock path; add typed
  `TaskError` variants `PersistedRowSkipped` and `WalletLocked` (replace the
  inverted `WalletBackendNotYetWired`-for-signing usage). ~180 lines. (S): seed
  enters memory only via `provide_seed`; load path stays seedless. Dep: T3.

- **T5 — Construction flip + cleanup (batched).** `context/mod.rs:25,655`
  import + one-line swap; `mod.rs:58` re-export; remove dead seed-loader
  references; delete the obsolete `wallet_id_from_seed` *load* usage (keep for
  create/sign). ~40 lines. Dep: T4.

- **T6 — Skip surfacing (UI/event) (batched).** Optionally override
  `EventBridge::on_platform_event` to log `WalletSkippedOnLoad`; surface
  `LoadedWallets.skipped` as a calm, actionable `MessageBanner` (Everyday-User
  wording — "One saved wallet couldn't be opened. Re-add it from its recovery
  phrase to restore it."). ~80 lines. Dep: T4.

- **T7 — Tests (batched).** Adapt the loader swap-boundary test; add a
  seedless-bridge unit test (xpub→WalletId matches `wallet_id_from_seed` for the
  same wallet); a backend-e2e cold-boot test: persist N wallets, drop backend,
  reconstruct via `UpstreamFromPersisted`, assert N balances visible with **no**
  seed in `inner.seeds`, then `provide_seed` + sign. ~200 lines. Dep: T5.

- **T8 — Docs + housekeeping (batched).** Update `g2-mock-boundary.md` §G2.5
  (G2 gate closed), flip PROJ-010 in the gap audit, `docs/user-stories.md` if a
  "see balances before unlock" story exists. Dep: T7.

---

## 7. Open questions / risks (need a user decision)

- **Q1 — Drop `SeedReregistrationLoader` entirely, or keep it as a fallback?**
  Recommendation: **drop** (G2.4, M-NO-TOMBSTONES). Keep only if you want a
  runtime escape hatch for a corrupt-persister recovery flow (re-derive from
  seed when `load_from_persistor` skips a row). My read: the skip→re-add-from-
  phrase UX (T6) covers that without a second loader. **Decision needed.**

- **Q2 — PR #3692 is a DRAFT against an unreleased branch
  (`feat/platform-wallet-sqlite-persistor`), milestone v4.0.0.** Pinning DET to a
  draft head means future force-pushes to that branch can move the SHA. We pin to
  the immutable commit `ddfa66ed`, so DET is stable — but we won't get the PR's
  later fixes until we re-pin. Acceptable? Or wait for #3692 to merge to
  `v3.1-dev` first? **Decision needed** (affects T1 timing).

- **Q3 — `last_applied_chain_lock` is the sole remaining `LOAD_UNIMPLEMENTED`.**
  It re-warms on the first post-restart SPV chainlock (no V001 column). Confirm
  DET has no launch-time consumer that needs it *before* first sync. Audit says
  no (DET reads chainlock state from `ConnectionStatus`, push-based), but flag for
  Smythe.

- **Q4 — Unlock-time seed provisioning placement.** §5.3 proposes
  `WalletBackend::provide_seed` called from the unlock path. Confirm the unlock
  path is a single chokepoint (it should be — the password flow), so seeds enter
  memory in exactly one place. If unlock is multi-site, the chokepoint must be
  established first (small extra task).

---

## Candy tally (findings by severity)

- **HIGH (fund-safety design gates) — 2:** (1) WalletId-match gate on
  `outcome.loaded` to preserve published-xpub==scanned-xpub routing (§5.2);
  (2) unlock-time `provide_seed` so signing keeps working after seedless load
  while keeping the seed off the load path (§5.3).
- **MEDIUM (correctness/API) — 3:** (1) `PersistedWalletLoader` trait must change
  shape, not just add an impl (§2.1); (2) `WalletSeedHash` ⇄ `WalletId` bridge
  via persisted xpub is mandatory (§0.4); (3) replace the inverted
  `WalletBackendNotYetWired`-for-locked-signing with a typed `WalletLocked`
  variant (§6 T4).
- **LOW (housekeeping/clarity) — 3:** (1) brief's head SHA / SeedProvider were
  stale — real head is seedless `ddfa66ed` (§0); (2) divergence is SAFE not
  risky — measured against the wrong base (§4.1); (3) drop
  `SeedReregistrationLoader` per M-NO-TOMBSTONES (§3.3).

**Total: 8 findings (2 HIGH, 3 MEDIUM, 3 LOW).**
