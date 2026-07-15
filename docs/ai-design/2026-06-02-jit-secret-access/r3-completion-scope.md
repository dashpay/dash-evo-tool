# R3 Completion Scope — Retiring the `Wallet::Open` Whole-Session Plaintext Seed

**Author:** Nagatha (Architect)
**Date:** 2026-06-02
**Status:** Read-only scoping. No implementation in this pass.
**Baseline:** branch `docs/platform-wallet-migration-design` @ `db03053f`
(*"refactor(wallet): JIT secret access — retire R1/R2 eager residencies, route R3 backend readers through chokepoint"*).

---

## 1. Context

The JIT secret-access refactor moves every plaintext-seed read behind a single
async chokepoint: `SecretAccess::with_secret` / `with_secret_session`
(`src/wallet_backend/secret_access.rs`), keyed by
`SecretScope::{HdSeed, SingleKey}`, handing the closure a borrowed `Zeroizing`
plaintext (`SecretPlaintext` / `SecretSession`).

- **R1** (HD-seed map) and **R2** (single-key cache) are fully retired.
- **R3** — the session-long plaintext seed parked inside `WalletSeed::Open(OpenWalletSeed { seed, .. })` —
  is the last residency. Wave C rerouted the three enumerated *async* backend readers through the
  chokepoint (the canonical template is `contact_requests.rs`, §6). It deliberately did **not** reshape
  `WalletSeed::open`, because doing so requires converting **every** remaining reader first.

This document is the census of what remains, the conversion design for each class, the
`WalletSeed::open` reshape verdict, the UI ripple, and a Wave-D task breakdown.

The governing tension: `with_secret*` is **async** and lives in the `wallet_backend` seam. Several
remaining readers are **synchronous** (in-model derivation, the `Wallet` `Signer<PlatformAddress>`
impl the SDK invokes, and sync UI key viewers) and cannot `await`.

---

## 2. Census — every live reader, classified

Method: `rg -n "seed_bytes\(\)|WalletSeed::Open|\.open\b" src/`, then traced each call chain to its
nearest async boundary. Test-only reads, egui `Window::open`, `wallet_unlock_popup.open()`, and
the `seed_envelope` debug-redaction test are excluded as non-load-bearing.

Classification key:
- **(A) async-reroutable** — the read sits in (or one hop from) an `async fn`; wrap in
  `with_secret(_session)` exactly like Wave C did for `contact_requests.rs`.
- **(B) sync-model, seed-as-parameter** — a sync model fn reading `self.seed_bytes()`; change its
  signature to receive `seed: &Zeroizing<[u8; 64]>` from the async caller that already holds the scope.
- **(C) hard-blocker** — a sync path with no reachable async boundary at the read site.

### 2.1 Census table

| # | Site | Reader | Class | Async boundary / mechanism |
|---|------|--------|:---:|----|
| 1 | `backend_task/dashpay/contacts.rs:59` | `register_dashpay_addresses_for_identity` (async fn) reads `seed_bytes().to_vec()` | **A** | Already async. Reroute: `with_secret(HdSeed{seed_hash}, |pt| pt.expose_hd_seed()…)`; move the BIP-32 derivation into a `wallet_backend` helper (mirror `derive_contact_xpub_material`). |
| 2 | `backend_task/dashpay/contact_info.rs:146` | `derive_contact_info_keys` reads `seed_bytes().to_vec()` | **A** | Caller is async. Same reroute; lift `derive_contact_info_keys` body into a `wallet_backend` derivation helper taking the borrowed seed. |
| 3 | `backend_task/dashpay/incoming_payments.rs:92` | `register_dashpay_addresses_for_identity` reads `seed_bytes().to_vec()` | **A** | Async fn. Same reroute (DIP-15 receive-side address derivation helper). |
| 4 | `model/qualified_identity/mod.rs:395` | `QualifiedIdentity::sign` (impl `Signer`, **async**) reads `wallet_ref.seed_bytes()` in the ECDSA_HASH160 path-scan fallback | **A** | The method is `async`, but the read is sync inside a held `RwLockReadGuard`. Reroute: resolve the seed via `with_secret` *before* the lock scan, pass borrowed bytes into the scan loop. Fund-relevant (signing). |
| 5 | `model/wallet/mod.rs:826` | `derive_private_key_in_arc_rw_lock_slice` (sync) | **B/C** | Called from `encrypted_key_storage.rs:317` (sync, at identity load) and indirectly key viewers. Becomes seed-as-parameter; see #6/#13. |
| 6 | `model/qualified_identity/encrypted_key_storage.rs:317` | sync key-materialization at load reads slice-derivation (→ #5) | **B** | Reached from identity-load backend tasks (async). Thread seed in from the async caller, or resolve per-wallet via `with_secret` before materialization. |
| 7 | `model/wallet/mod.rs:835` | `private_key_at_derivation_path` (sync) | **B/C** | Two consumers: backend (B) and sync UI viewers (C, #11/#12). Becomes seed-as-parameter. |
| 8 | `model/wallet/mod.rs:846` | `private_key_for_address` (sync) | **B** | Sole consumer `fund_platform_address_from_asset_lock.rs:73` is **async**. Reroute the *caller* via `with_secret`, pass seed into a seed-param variant. Fund-critical. |
| 9 | `model/wallet/mod.rs:1179,1207,1252,1275,1299,1373,1408` + `1317/1324` | `bootstrap_*_addresses` family (bip32, coinjoin, identity reg/invitation/topup/not-bound, provider, platform-payment), all sync, each `let seed = *self.seed_bytes()?` | **B** | Entry point `bootstrap_known_addresses` (mod.rs:718) → `wallet_lifecycle.rs:296 bootstrap_wallet_addresses` (sync, but invoked from async backend register/unlock paths). Convert the family to take `seed: &[u8;64]`; have the async caller resolve once via `with_secret_session` and pass it down. |
| 10 | `model/wallet/mod.rs:1611` | (additional bootstrap helper in the same family) | **B** | Same as #9. |
| 11 | `ui/identities/keys/key_info_screen.rs:401` | sync UI calls `private_key_at_derivation_path` to **display** a WIF/hex | **C** | No async boundary in `ui()`. Must become a backend task (`WalletTask::DeriveKeyForDisplay`) returning the WIF/hex string; UI renders the result. |
| 12 | `ui/identities/keys/key_info_screen.rs:459` | same, second branch | **C** | Same backend-task conversion. |
| 13 | `ui/wallets/wallets_screen/dialogs.rs:1257` | `derive_private_key_wif` (sync) reads `private_key_at_derivation_path`; callers `mod.rs:2471`, `address_table.rs:466` | **C** | Sync UI WIF export. Same backend-task conversion. |
| 14 | `model/wallet/mod.rs:1806` | `get_platform_address_private_key` (sync) reads `seed_bytes()` | **C** | Invoked by the `Wallet` `Signer<PlatformAddress>` impl (#15). Deletable once #15 is rerouted. |
| 15 | `model/wallet/mod.rs:1820` | **`impl Signer<PlatformAddress> for Wallet`** — `sign` / `sign_create_witness` / `can_sign_with`, each reaching `get_platform_address_private_key` → `seed_bytes()` | **C (hard-blocker)** | **STILL LIVE.** Passed as `&wallet` to the SDK at four async sites: `fund_platform_address_from_asset_lock.rs:82`, `fund_platform_address_from_wallet_utxos.rs:100`, `transfer_platform_credits.rs:52`, `withdraw_from_platform_address.rs:52`. SDK bound is `S: Signer<PlatformAddress>`. See §4. |
| 16 | `model/wallet/mod.rs:1962` | `WalletAddressProvider::with_gap_limit` copies `*wallet.seed_bytes()` into a `seed: [u8;64]` **field** | **C** | Sole caller `fetch_platform_address_balances.rs:34` is async. The provider holds a long-lived seed *copy* (a residency in its own right). Rebuild it to derive inside a `with_secret_session` scope, or take a borrowed seed; drop the owned field. |
| 17 | `context/wallet_lifecycle.rs:444` | `wallet_seed_snapshot` reads `seed_bytes()` to promote the seed into the session cache on unlock | **B (benign)** | This is the *bridge* from the open wallet into the chokepoint's session cache. Once `WalletSeed::open` no longer parks plaintext (§5), this snapshot has nothing to read; unlock instead validates the passphrase and the chokepoint owns residency. Convert last. |

### 2.2 Counts by class

| Class | Count (distinct live sites) | Sites |
|---|:---:|---|
| **A** — async-reroutable | **4** | #1, #2, #3, #4 |
| **B** — sync-model seed-as-parameter | **~7 functions** (bootstrap family counts as one cluster of ~8 fns) | #5–#10, #17 |
| **C** — hard-blocker | **3 clusters** | platform Signer (#14+#15+#16, fund-critical) · sync UI key viewers (#11+#12+#13) · `WalletAddressProvider` owned-seed field (#16) |

> Test-only `seed_bytes()` reads at `mod.rs:2496/2497/2812` and `2800` assert wallet open/closed
> state and must be rewritten to the new shape, but carry no production residency.

---

## 3. The seed-as-parameter design (class B)

**Today:** sync model fns reach back into `self.wallet_seed` via `self.seed_bytes()`. This is what
forces `WalletSeed::Open` to park the plaintext for the whole session.

**Target:** the seed is *passed in* from the one async caller that already holds a `with_secret_session`
scope. The model never reads its own parked seed.

Signature change pattern (applies to the bootstrap family #9/#10, and #5/#7/#8):

```text
// before
fn bootstrap_bip32_addresses(&mut self, network, app_context) -> Result<(), String> {
    let seed = *self.seed_bytes()?;            // ← parked-seed read
    ...
}

// after
fn bootstrap_bip32_addresses(
    &mut self,
    seed: &[u8; 64],                            // ← borrowed, operation-scoped
    network, app_context,
) -> Result<(), String> { ... }
```

**Call-chain to the async boundary (the load-bearing part):**

```
async backend task (register wallet / unlock / discover identities)
  └─ holds  SecretAccess::with_secret_session(HdSeed{seed_hash}, async |session| {
       let seed = session.plaintext().expose_hd_seed()?;   // borrowed Zeroizing
       // bootstrap is a &mut Wallet mutation → take the write lock INSIDE the scope
       wallet.write()?.bootstrap_known_addresses(seed, &app_context);
     })
```

`bootstrap_known_addresses(&mut self, seed, app_context)` fans the borrowed `seed: &[u8;64]` down to
every `bootstrap_*` child (#9/#10) — none of which read `self.seed_bytes()` any more.

**Where the resolution happens.** `bootstrap_wallet_addresses` (`wallet_lifecycle.rs:282`) is the
sync funnel today. It must gain an async sibling (`bootstrap_wallet_addresses_jit`) that opens the
`with_secret_session` scope and calls the new seed-param `bootstrap_known_addresses`. The two
registration/unlock callers (`wallet_lifecycle.rs` register + `handle_wallet_unlocked`) switch to the
async sibling. This is the single most mechanical, highest-fan-out conversion in Wave D.

**For #5/#7/#8** (`derive_private_key_in_arc_rw_lock_slice`, `private_key_at_derivation_path`,
`private_key_for_address`): add `*_with_seed(seed: &[u8;64], …)` variants. The async backend caller
(`fund_platform_address_from_asset_lock.rs`, identity-key materialization) resolves the seed via
`with_secret` and calls the seed-param variant. The legacy `self.seed_bytes()` variants are deleted
once no caller remains (the UI callers move to backend tasks per §6).

---

## 4. Class C — platform `Signer<PlatformAddress>` (the real hard-blocker)

This is the one that cannot be solved by seed-as-parameter, because **the SDK** — not our code —
invokes `wallet.sign(platform_address, data)` synchronously-from-its-perspective, deep inside
`top_up` / `transfer_address_funds` / `withdraw`. The SDK's trait bound is:

```text
pub trait TopUpAddress<S: Signer<PlatformAddress>> { async fn top_up(&self, …, signer: &S, …) … }
```

(Confirmed in the locked SDK rev `ddfa66ed…`,
`packages/rs-sdk/src/platform/transition/top_up_address.rs:23`.)

**`DetSigner` does NOT satisfy this bound.** `DetSigner` (`wallet_backend/det_signer.rs`) implements
the *identity-key* `Signer` (indexed by `DerivationPath`, methods `sign_ecdsa` / `public_key`). It is
**not** `Signer<PlatformAddress>`. So the platform-funding path has no JIT signer today — it leans
entirely on the live `Wallet` `Signer<PlatformAddress>` impl, which reads the parked seed.

**Design — `DetPlatformSigner<'a>`:**

Introduce a new JIT signer in the `wallet_backend` seam that mirrors `DetSigner`'s borrow discipline
but implements the platform trait:

```text
// wallet_backend/det_signer.rs (or a sibling det_platform_signer.rs)
pub(crate) struct DetPlatformSigner<'a> {
    held:    &'a SecretPlaintext<'a>,   // borrowed HD seed, never owned/copied
    network: Network,
    // the wallet's watched platform-payment paths, needed to map address → path
    paths:   &'a PlatformPathIndex,
}

#[async_trait]
impl Signer<PlatformAddress> for DetPlatformSigner<'_> {
    async fn sign(&self, addr: &PlatformAddress, data: &[u8]) -> Result<BinaryData, ProtocolError> {
        // map addr → derivation_path from self.paths (pure, no secret)
        // derive priv from borrowed seed, sign, drop the derived key
    }
    async fn sign_create_witness(…) { … }
    fn can_sign_with(&self, addr) -> bool { self.paths.contains(addr) }   // pure
}
```

**Reroute the four SDK call sites** (all already async):
`outputs.top_up(…, &wallet, …)` → build the signer inside a `with_secret_session(HdSeed{seed_hash})`
scope and pass `&platform_signer`:

```text
secret_access.with_secret_session(&HdSeed{seed_hash}, async |session| {
    let signer = DetPlatformSigner::from_held(session.plaintext(), network, &paths);
    outputs.top_up(&sdk, asset_lock_proof, asset_lock_pk, fee_strategy, &signer, None).await
}).await
```

`can_sign_with` is a pure path-membership check (no secret) — so it stays cheap and prompt-free.
The address→path index (`paths`) is built from `wallet.watched_addresses` *before* entering the
scope; only the actual `sign` derives from the borrowed seed.

**Once #15 is rerouted:** `impl Signer<PlatformAddress> for Wallet` (#15) and
`get_platform_address_private_key` (#14) have no callers → **delete both**. That deletes two of the
parked-seed reads outright.

> **Is the `Wallet` `Signer` impl dead now?** **No.** It is still the *only* `Signer<PlatformAddress>`
> in the codebase and is live at four fund-moving SDK call sites. It becomes dead — and deletable —
> only after `DetPlatformSigner` lands and the four sites swap. That swap is the centerpiece of Wave-D
> task D3 and is fund-critical.

---

## 5. `WalletSeed::open` reshape — verify-not-park

**Verdict: YES, achievable — but strictly *after* all A/B/C readers are converted.** Until then,
removing the parked seed breaks every reader in §2.

**New shape.** `WalletSeed::open(password)` becomes a **passphrase *validator***, not a seed parker:

```text
// today: decrypts and STORES the plaintext in WalletSeed::Open(OpenWalletSeed{ seed, .. })
// target: decrypt to prove the password is correct, then DISCARD the plaintext.
pub fn open(&mut self, password: &str) -> Result<(), WalletSeedError> {
    let seed = self.closed()?.decrypt_seed(password)?;   // Zeroizing, local
    // … no assignment of `seed` into self; it drops (zeroized) at end of scope …
    self.mark_unlocked();    // flips a state flag only
    Ok(())                   // seed gone; chokepoint owns all future residency
}
```

**What `OpenWalletSeed` keeps:** nothing secret. The `seed: [u8; 64]` field is **removed**. What
remains of the "open" state is the *metadata* the UI/model still needs without the seed —
`ClosedKeyItem` (seed_hash, salt, nonce, encrypted_seed, password_hint) plus an `unlocked: bool`
intent flag. In practice `WalletSeed` likely collapses to a single struct carrying
`ClosedKeyItem` + an `unlocked` flag, since `Open` and `Closed` would then differ only by that flag.
`is_open()` reads the flag; `seed_hash()` / `salt()` / `nonce()` / `encrypted_seed_slice()` /
`password_hint()` all read from the retained `ClosedKeyItem` unchanged.

**The unlock-intent bridge.** Today `handle_wallet_unlocked` → `wallet_seed_snapshot` (#17) reads the
parked seed and promotes it into the session cache. Post-reshape there is no parked seed to read.
Instead, `open()` (now a validator) is followed by a one-shot `with_secret(_session)` warm call that
re-decrypts via the chokepoint with `RememberPolicy::UntilAppClose`, honoring the "keep unlocked"
gesture. The seed lives **only** in the chokepoint's session cache — a single, auditable residency
with a known lifetime — never in the `Wallet` value graph.

**Net residency after R3:** zero plaintext seed in `Wallet` / `OpenWalletSeed`. The only plaintext
seed in the process is the operation- or session-scoped `Zeroizing` inside a `with_secret*` frame.
This is the R3 goal. (ASVS V11.7 in-use-data, V13.3 secret management — minimize plaintext residency
and confine it to a single controlled boundary.)

---

## 6. Ripple assessment — does seed-as-parameter leak into the UI?

**Yes — but containably.** Three sync UI surfaces read the seed directly *to display or export a
private key*, and the UI cannot `await` the chokepoint:

| UI surface | File | What it does |
|---|---|---|
| Key info screen | `ui/identities/keys/key_info_screen.rs:401,459` | renders WIF + hex of a derived key |
| Receive/export dialog | `ui/wallets/wallets_screen/dialogs.rs:1257` (`derive_private_key_wif`), callers `mod.rs:2471`, `address_table.rs:466` | copies a WIF for an address |

These do **not** get a seed parameter. They are converted to the **task pattern**: the screen returns
`AppAction::BackendTask(WalletTask::DeriveKeyForDisplay { seed_hash, derivation_path })`; the backend
resolves the seed via `with_secret`, derives, and returns the WIF/hex as a typed
`BackendTaskSuccessResult`. The screen renders the result from `display_task_result`. No seed crosses
into the UI layer.

The **unlock popups** (`wallet_unlock_popup.rs:109`, `wallet_unlock.rs:76`,
`single_key_send_screen.rs:774`, `wallets_screen/mod.rs:2569`) call `wallet_seed.open(...)` /
`wallet.open(...)`. After the §5 reshape these still call `open()` — but `open()` is now a *validator*
that parks nothing, so they need no structural change beyond the new return type. They are touched,
not redesigned.

**Layer/file count touched:**
- `wallet_backend/` — +1 signer type (`DetPlatformSigner`), +2–3 derivation helpers, reshaped chokepoint warm-on-unlock.
- `model/wallet/mod.rs` — bootstrap family + 3 derivation fns reshaped; `Signer<PlatformAddress>` + `get_platform_address_private_key` + `WalletAddressProvider` owned-seed deleted; `WalletSeed`/`OpenWalletSeed` collapsed.
- `model/qualified_identity/` — 2 files (sign fallback + load materialization).
- `backend_task/dashpay/` — 3 files (A).
- `backend_task/wallet/` — 4 fund sites swap to `DetPlatformSigner`; +1 `DeriveKeyForDisplay` task.
- `context/wallet_lifecycle.rs` — bootstrap async sibling + unlock bridge.
- `ui/` — 2 key-viewer surfaces → backend task; 4 unlock popups → return-type touch only.

This is **roughly 18–22 files across 5 layers**. It is **not one Bilby wave.** It must split — both to
keep each task independently reviewable and because the fund-critical signer/open work must be isolated
for Smythe.

---

## 7. Wave-D task breakdown

> Ordering matters: D1/D2 are prerequisites that drain the reader population so D4's `open()` reshape
> doesn't break anything. D3 is independent of D1/D2 but shares the "delete the parked seed last" gate.

| Task | Title | Class | Files | Depends on | Smythe review? |
|---|---|:---:|---|---|:---:|
| **D1** | Reroute the 4 async DashPay/identity-sign readers through `with_secret`; lift derivations into `wallet_backend` helpers | A | `dashpay/contacts.rs`, `contact_info.rs`, `incoming_payments.rs`, `qualified_identity/mod.rs` | — | Advisory (matches merged Wave-C pattern) |
| **D2** | Seed-as-parameter for the in-model derivation family + bootstrap async sibling | B | `model/wallet/mod.rs` (bootstrap family, `private_key_*`, slice-derive), `qualified_identity/encrypted_key_storage.rs`, `context/wallet_lifecycle.rs` | — | **Yes** (derivation correctness; HD path integrity) |
| **D3** | `DetPlatformSigner<'a>: Signer<PlatformAddress>`; swap `&wallet` → `&signer` at the 4 SDK fund sites; convert sync UI key viewers to `WalletTask::DeriveKeyForDisplay`; rebuild `WalletAddressProvider` without an owned seed | C | `wallet_backend/det_signer.rs` (+sibling), 4 × `backend_task/wallet/*`, `model/wallet/mod.rs` (provider), `ui/identities/keys/key_info_screen.rs`, `ui/wallets/wallets_screen/{dialogs,mod,address_table}.rs` | — | **Yes — fund-critical** (platform signing + asset-lock funding) |
| **D4** | `WalletSeed::open` → verify-not-park; collapse `OpenWalletSeed` (drop `seed` field); delete dead readers (`Wallet` platform `Signer`, `get_platform_address_private_key`, `wallet_seed_snapshot`); rewire unlock warm-on-`with_secret`; fix tests | — | `model/wallet/mod.rs`, `model/wallet/seed_envelope.rs`, `context/wallet_lifecycle.rs`, 4 unlock-popup UI files | **D1, D2, D3** | **Yes — fund-critical** (last plaintext residency removal; unlock semantics) |

**Why four, not one.** Each task closes a distinct reader population and is independently compilable
and testable. D4 is the *only* task that may remove the parked seed, and it is gated on D1–D3 having
drained every reader. Bundling D3/D4 into one PR would make the fund-critical diff unreviewable.

**Minimum split if compressed:** D1+D2 could merge (both non-fund-critical, no signer changes), but
D3 and D4 must each stand alone for Smythe. So the floor is **three** waves; the recommended shape is
**four**.

---

## 8. Risk — fund-safety traps in handing the borrowed seed down

| # | Risk | Mitigation |
|---|---|---|
| R-1 | **Accidental copy.** A seed-param fn does `let seed = *seed_ref;` (deref-copy onto the stack), recreating a residency the borrow was meant to prevent (`WalletAddressProvider` does exactly this today, #16). | Pass `seed: &[u8; 64]` and **derive in place**. Forbid `*seed`. Where a copy is unavoidable, wrap in `Zeroizing`. Lint/grep for `*seed` in the converted family during review. |
| R-2 | **Lifetime escape.** `DetPlatformSigner` / `DetSigner` borrows the session plaintext; if the `&signer` outlives the `with_secret_session` scope (e.g. stored in a struct returned from the closure), it's use-after-free of zeroized memory — or worse, a dangling borrow the compiler should reject but a `'static`-coercion (`Box`/`Arc`) could smuggle past. | Keep the signer constructed **and consumed** entirely inside the async closure (the SDK call awaits inside the scope). Never `Box`/`Arc`/return the signer. The `'a` lifetime tie to `SecretPlaintext<'a>` enforces this at compile time — do not add `'static` bounds to satisfy the SDK; the SDK takes `&S`, which is fine. |
| R-3 | **Operation-scoped guarantee lost on hand-off.** R1/R2 were retired precisely so a secret lives only for one operation. Threading a `&[u8;64]` *down a deep sync call tree* widens the window the plaintext is reachable on the stack. | Resolve the seed as **late** and as **close** to the derivation as the call graph allows; for the bootstrap family, one `with_secret_session` per bootstrap run is acceptable (it is one logical operation), but do not hoist the scope to wrap unrelated work. |
| R-4 | **Wrong-network platform signing.** The current `Wallet` `Signer<PlatformAddress>` brute-forces all four networks (`mod.rs:1839-1845`). `DetPlatformSigner` must preserve correctness: derive for exactly the wallet's network. | `DetPlatformSigner` carries the single `network` (from the backend, which knows the active network) — no brute-force. The address→path index is network-specific. This is *safer* than today, not just equivalent. |
| R-5 | **`open()` reshape lands before readers drained.** If D4 removes the `seed` field while any §2 reader still calls `self.seed_bytes()`, those paths return "wallet closed" at runtime → silent loss of signing/derivation. | Compile-time gate: deleting `OpenWalletSeed.seed` makes `seed_bytes()` un-implementable, so the build fails until every reader is gone. Enforce by sequencing D4 last and treating any remaining `seed_bytes()` caller as a blocker. |
| R-6 | **`can_sign_with` accidentally prompts.** If `DetPlatformSigner::can_sign_with` resolved the secret, the SDK's pre-flight checks would trigger passphrase prompts. | `can_sign_with` is a **pure** path-membership check against the prebuilt index; it never enters `with_secret`. (Matches the existing `Wallet::can_sign_with` which only derives — here we make it strictly pure.) |

---

## 9. Open questions for the user

1. **Session granularity for bootstrap (D2).** Bootstrapping derives hundreds of addresses across
   seven path families. One `with_secret_session` for the whole bootstrap run is the natural unit
   (it is one logical operation), but it holds the borrowed seed on the stack for the duration. Acceptable,
   or do you want the scope narrowed further (per-family re-prompt is hostile UX, so I recommend the
   single-run scope)?
2. **`WalletSeed` collapse vs. keep-the-enum (D4).** Once `Open` carries no secret, `Open`/`Closed`
   differ only by an `unlocked` flag. Collapse to one struct (my recommendation), or retain the
   two-variant enum for blast-radius minimization in D4? The former is cleaner; the latter is a smaller diff.
3. **`DeriveKeyForDisplay` exposure.** Converting the UI key viewers to a backend task means a derived
   private key (WIF/hex) round-trips as a `BackendTaskSuccessResult`. Confirm this is acceptable for the
   "view private key" feature (it already displays the key on screen, so the trust boundary is unchanged),
   and that the result should be wrapped in the existing `Secret` newtype end-to-end.

---

## 10. Candy tally (architecture findings by severity)

| Severity | Count | Findings |
|---|:---:|---|
| **High** (fund-critical hard-blockers) | **3** | Live `Wallet` `Signer<PlatformAddress>` at 4 SDK fund sites (#15); `DetSigner` does not satisfy `Signer<PlatformAddress>` (needs `DetPlatformSigner`); `WalletAddressProvider` owns a copied seed field (#16). |
| **Medium** (structural conversions) | **3** | Bootstrap family seed-as-parameter + async sibling (#9/#10); sync UI key viewers must become backend tasks (#11–#13); `WalletSeed::open` verify-not-park reshape gated on full reader drain (#17/§5). |
| **Low** (mechanical / advisory) | **2** | 4 async DashPay/identity-sign readers reroute cleanly via the merged Wave-C template (#1–#4); unlock popups need return-type touch only after reshape. |

**Total: 8 findings (3 High, 3 Medium, 2 Low).**

---

*Designs are elegant because they refuse to be anything less. The seed has exactly one place left to
hide; D1–D4 close the door, and the chokepoint keeps the only key.* — Nagatha
