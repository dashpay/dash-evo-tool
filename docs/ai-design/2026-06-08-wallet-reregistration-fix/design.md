# Wallet Re-Registration Fix — Restoring the Upstream Persistor Write Path

Repairs a HIGH-severity regression introduced by `e6c6c017` (PROJ-010 seedless
load): after the `SeedReregistrationLoader` was deleted, **no DET code writes
the upstream `platform-wallet.sqlite` persistor**. The seedless
`load_from_persistor` only ever READS it, so when that persistor is empty
(fresh install, post-reset, migration, sidecar-only wallets) the wallet is
never registered with the upstream SPV manager, the watch set is empty, and
received Core funds are invisible at 100% sync.

> All upstream `file:line` citations are at the live pin
> `9e1248cb` (`platform`), `eb889af1` (`rust-dashcore` / `key-wallet`).
> READ-ONLY design — no production code in this document.

---

## 0. Headline findings (read first)

1. **The bug is a missing WRITE, not a broken READ.** `load_from_persistor`
   (`packages/rs-platform-wallet/src/manager/load.rs:55`) faithfully rebuilds
   every wallet the persistor knows about, watch-only and seedless. The defect
   is that DET never *populates* that persistor. Grep confirms zero call sites
   for `create_wallet_from_seed_bytes` / `persister.store` / changeset
   construction anywhere in `src/` or `tests/`.
2. **There is NO upstream seedless watch-only WRITE/persist API.** The complete
   public manager write surface is two methods, both seed-bearing:
   `create_wallet_from_mnemonic` and `create_wallet_from_seed_bytes`
   (`wallet_lifecycle.rs:73/100`). Both funnel into the private
   `register_wallet`, the sole writer to `persister.store` (`:281`). There is
   no `register_watch_only`, no `add_watch_only_wallet`, no public path that
   feeds an xpub manifest into `store`.
3. **The building blocks for a seedless register exist upstream but are not
   wired into a writer.** `build_watch_only_wallet(network, wallet_id,
   manifest)` (`rehydrate.rs:61`, `pub(super)`), `Wallet::new_watch_only`,
   `Account::from_xpub`, and the public `PlatformWalletChangeSet` /
   `AccountRegistrationEntry` / `WalletMetadataEntry` types are all present.
   What is absent is a **manager method that takes a keyless manifest and both
   registers the watch-only wallet AND calls `persister.store`.** The load path
   reads such manifests; nothing writes them except the seed path.
4. **DET cannot compute the upstream `WalletId` seedlessly.** Upstream
   `WalletId = SHA256(root_public_key.serialize() ‖ root_chain_code)` over the
   **master `m`** xpub (`key-wallet wallet/mod.rs:99`). DET's sidecar
   `xpub_encoded` holds the **`m/44'/coin'/0'` account** xpub
   (`model/wallet/mod.rs:404`, `meta.rs:45`). BIP44 hardens every level above
   the account, so the account xpub cannot yield the root xpub. **DET therefore
   cannot construct a valid `(WalletId, changeset)` pair without the seed.**
   This kills the "DET writes the persistor directly" option as a *seedless*
   strategy — DET can only write the persistor at a moment it holds the seed.
5. **Birth height is the silent-funds trap.** A wallet's SPV compact-filter
   scan window starts at its persisted `birth_height`
   (`wallet_lifecycle.rs:135`, `load.rs:84`). A fresh registration with
   `birth_height_override = None` resolves to the **current SPV tip**, so any
   deposit before that tip (the real-world 1.0 DASH at block 1492173) is never
   matched. The fix MUST register with `birth_height = 0` (full historical
   scan) — or a per-wallet floor — for any wallet that may have pre-existing
   deposits. There is no separate rescan API; the persisted `birth_height` is
   the only lever, and it is only settable through the seed-bearing
   `create_wallet_from_*` call.

---

## 1. Problem statement and confirmed root cause

The product model (PROJ-008) is **watch-only at launch, password only at
secret/sign time**. The seedless `load_from_persistor` read path serves that
model correctly — *if the persistor is populated*. It is not. The deleted
`SeedReregistrationLoader` was the only thing that, at every cold boot,
re-derived each wallet from its seed and drove the upstream create path which
internally calls `persister.store`. Removing it left the persistor write side
with no caller.

Failure chain:

```
register_wallet (context)  → writes DET sidecars + legacy data.db + in-mem addrs
                           ✗ never calls upstream create_wallet_from_seed_bytes
                           ✗ never calls persister.store
        ↓ (cold boot)
load_from_persistor()      → persister.load() returns EMPTY
                           → zero wallets rebuilt → empty SPV watch set
        ↓
SPV reaches 100%           → no addresses watched → deposit at 1492173 invisible
```

The backend-e2e "Timed out waiting for wallet to register with the upstream
backend" timeout is the same root cause: nothing registers the wallet upstream,
so `is_wallet_registered` never flips true.

---

## 2. The upstream-API finding (watch-only write path)

**Absent.** Verified exhaustively:

| Capability | Upstream surface | Seedless? | Writes persistor? |
|---|---|---|---|
| `load_from_persistor` | `manager/load.rs:55` | yes | no (read) |
| `create_wallet_from_mnemonic` | `wallet_lifecycle.rs:73` | **no (seed)** | yes |
| `create_wallet_from_seed_bytes` | `wallet_lifecycle.rs:100` | **no (seed)** | yes |
| `remove_wallet` | `wallet_lifecycle.rs:386` | n/a | yes (delete) |
| `build_watch_only_wallet` | `rehydrate.rs:61` | yes | **no — `pub(super)`, read-side only** |
| `persister.store(WalletId, ChangeSet)` | `changeset/traits.rs:199` | yes (public) | yes |

The data model fully supports watch-only wallets (the load path builds them).
What is missing is a **public, seedless writer** — a method that accepts an
xpub manifest + `WalletId` + birth height and performs `insert_wallet` +
`persister.store`. `register_wallet` does exactly this but is private and only
reachable with a fully-built (seed-derived) `Wallet`, and `WalletId` is not
seedlessly derivable by DET (§0.4).

**Conclusion: this parallels PROJ-017 — a missing upstream registrar.** A pure
seedless fix is not available with the current upstream surface.

---

## 3. Chosen design — write the persistor when DET holds the seed

Because no seedless write path exists and DET cannot compute `WalletId`
seedlessly, the fix populates the upstream persistor **at the two moments DET
legitimately holds the seed**, and leaves the seedless read path untouched for
warm boots. This preserves watch-only-at-launch: after the persistor is
populated once, every subsequent cold boot is seedless via the existing
`load_from_persistor`.

### 3.1 The seed-vs-watch-only decision, made explicit

The design crux asked whether to repopulate the watch set from sidecar xpubs
without the seed. **That is not possible** at the upstream layer: the only
writer needs a seed-derived `Wallet`, and the `WalletId` the persistor is keyed
by needs the root xpub DET does not hold. Therefore:

- **At launch (cold boot): stay seedless.** No change to the boot path. If the
  persistor is already populated, `load_from_persistor` rebuilds watch-only
  with no prompt — the PROJ-008 contract holds.
- **At the seed-bearing moments: write the persistor.** Registration with the
  upstream manager happens where DET already has the plaintext seed in hand —
  it does *not* introduce any new password prompt the product didn't already
  require:
  - **(W1) Create / import** (`context::register_wallet`, called from
    `add_new_wallet_screen` / `import_mnemonic_screen` with `&seed`): the user
    just typed or generated the phrase. Register upstream here.
  - **(W2) First unlock** (the JIT chokepoint, where a protected wallet's seed
    is decrypted for the first signing/derivation, and where an unprotected
    wallet's seed resolves prompt-free): for any wallet **present in DET
    sidecars but absent from the upstream persistor** (migrated installs,
    wallets created before this fix, post-reset), register upstream the first
    time the seed becomes available. An *unprotected* wallet resolves through
    the chokepoint's no-prompt fast path, so its balance reappears at startup
    with no user action; a *protected* wallet reappears on the unlock gesture
    the user performs anyway.

This is the minimal correct model: it never prompts solely to view balances,
and it never silently shows a wallet whose seed DET cannot prove it holds.

### 3.2 What gets written, and the birth-height lever

Registration uses the existing seed-bearing upstream call:

```
pwm.create_wallet_from_seed_bytes(network, seed_bytes,
                                  WalletAccountCreationOptions::Default,
                                  birth_height_override)
```

- `WalletAccountCreationOptions::Default` reproduces the SAME BIP44 account
  manifest the seedless gate already matches against (locked by the existing
  `bridge_account_xpub_matches_upstream_for_same_seed` test) — so the
  account-xpub fund-routing gate keeps working unchanged.
- `birth_height_override` is the birth-height lever (§4).
- Internally this writes `WalletMetadataEntry { network, birth_height }` +
  per-account `AccountRegistrationEntry` xpubs + address-pool snapshots via one
  atomic `persister.store` — exactly the manifest `load_from_persistor`
  rebuilds from on the next boot.

### 3.3 Idempotency

`create_wallet_from_seed_bytes` returns `WalletAlreadyExists` (via
`insert_wallet`) when the wallet is already registered. Registration is
therefore guarded: before calling, check the upstream manager
(`is_wallet_registered` / `pwm.get_wallet(wallet_id)`), and treat
`WalletAlreadyExists` as success. W1 and W2 are both idempotent and may both
fire for the same wallet across a session without double-watching.

### 3.4 Rejected alternatives

- **(a) Pure seedless re-registration from sidecar xpubs.** *Rejected:
  infeasible.* No upstream seedless writer exists, and DET holds only the
  hardened account xpub, not the root xpub `WalletId` requires (§0.4). Would
  require an upstream change first (see §6).
- **(b) DET writes persistor rows directly via the storage crate.** *Rejected.*
  Even setting aside seam risk, DET still cannot compute `WalletId` seedlessly,
  so this is not actually seedless — it needs the seed anyway. And reaching into
  the upstream changeset/schema to hand-roll `persister.store` calls duplicates
  `register_wallet`'s 200-line snapshot logic (account specs, address pools,
  metadata), bypasses `insert_wallet`'s in-memory registration, and violates
  **M-PLATFORM-WALLET-FIRST-PARTY** intent (DET consuming the persister, not
  reimplementing the registrar). If we ever want a *truly* seedless path, the
  correct home is an upstream API (option a / §6), not a DET-side schema poke.
- **(c) Re-introduce seed-at-boot (the deleted loader).** *Rejected.* Forces a
  password prompt at launch for protected wallets just to view balances —
  directly violates PROJ-008. The chosen W2 (first-unlock) is the same
  mechanism minus the launch-time prompt.
- **(d) Defer ALL registration to first unlock (W2 only, no W1).** *Rejected as
  the sole strategy.* A freshly created/imported wallet would not be registered
  until the user later signs — its balance would not appear until then. W1 is
  cheap (the seed is already in hand) and makes new wallets work immediately, so
  both writes ship together.

---

## 4. Birth-height / rescan strategy

### 4.1 The trap

`register_wallet` resolves birth height as: explicit override wins; else SPV
confirmed header tip; else `0` (`wallet_lifecycle.rs:135-143`). A wallet
registered with `None` while SPV is synced gets `birth_height = tip`, and its
compact-filter scan window is `[tip, ∞)` — **pre-existing deposits are
invisible.** This is precisely the 1492173 symptom.

### 4.2 The rule

Birth height depends on whether the wallet could already hold funds before this
registration:

- **W1 fresh-created wallet** (brand new phrase, never funded): no prior
  deposits possible. `birth_height_override = None` is correct and cheap — scan
  only from now forward.
- **W1 imported wallet** (existing recovery phrase) and **W2 every wallet**
  (recovered/migrated/pre-fix): deposits may predate registration.
  `birth_height_override = Some(0)` — **full historical scan from genesis** — is
  the safe default. This is the only setting that guarantees the 1492173 case
  is found.

### 4.3 Where the birth height comes from

DET persists no per-wallet creation height (verified — only the upstream
sidecar `WalletMeta` mentions `birth_height`, and DET never writes a real
value). Three tiers, in preference order:

1. **Known funding block** — not available today; DET has no record. Future:
   if a wallet records its first-deposit height, pin to it.
2. **Conservative network floor** — a per-network constant just below the
   earliest height DET could have produced an address (e.g. a DET-release-era
   height). Cheaper than genesis, still safe for any DET-created wallet.
   *Optional optimisation; not required for correctness.*
3. **Genesis (`Some(0)`)** — always correct, never misses funds. The shipping
   default for imported/recovered/migrated wallets.

Recommendation: ship **genesis (`Some(0)`)** for imported/W2 wallets,
`None` for W1-fresh. Treat the network-floor optimisation as a follow-up only if
genesis rescan cost proves painful.

### 4.4 Cost / UX of a genesis rescan

The SPV client already runs `with_start_height(0)` (`mod.rs:1510`) — it
downloads compact filters from genesis regardless. The per-wallet `birth_height`
only governs which downloaded filters a wallet is *matched* against, not how
many filters are fetched. So `Some(0)` adds **filter-matching** work over the
wallet's address set across the full chain, not a second network download. On
testnet this is seconds-to-minutes of CPU; on mainnet it is heavier but bounded
by the already-downloaded filter set. UX: balances populate progressively as
the match pass advances; surface this as the normal "syncing" state, not a
distinct rescan mode. No new banner required beyond existing sync indication.

---

## 5. Idempotency, placement, and seam compliance

### 5.1 Placement

- **W1 (create/import):** inside `context::register_wallet`
  (`src/context/wallet_lifecycle.rs:197`), after the sidecar/db writes and
  in-memory insert, add a backend registration step that calls a new
  `WalletBackend::register_wallet_from_seed(seed_hash, &seed,
  birth_height_override)`. The seed is already a borrowed parameter there.
- **W2 (first unlock / cold-boot reconciliation):** a new
  `WalletBackend::ensure_upstream_registered(seed_hash, plaintext_seed,
  birth_height_override)` invoked from the JIT chokepoint's seed-resolution
  scope (`SecretAccess::with_secret_session` / `with_secret`), and from
  `bootstrap_wallet_addresses_jit` (`context/wallet_lifecycle.rs:377`) which
  already runs at cold boot for prompt-free-resolvable wallets. This reuses the
  existing "seed is open right now" chokepoints — the same pattern
  `ensure_identity_funding_accounts` already uses to do seed-dependent work
  lazily (`mod.rs:1489`).
- **Read path:** `load_from_persistor_seedless` (`mod.rs:424`) is unchanged. It
  remains the warm-boot fast path; W1/W2 simply guarantee its input is
  populated.

### 5.2 Idempotency and partial state

The new register methods are per-wallet and idempotent:
1. Compute the upstream `WalletId` from the seed (the existing seed-bearing
   `wallet_id` derivation — DET already does this inside `create_wallet_from_*`
   via `compute_wallet_id`).
2. If `pwm.get_wallet(wallet_id)` is `Some`, return `Ok` (already registered) —
   no second watch.
3. Else call `create_wallet_from_seed_bytes(.., birth_height_override)`; map
   `WalletAlreadyExists` to `Ok` (race-safe).
4. On success, resolve into the DET `id_map` / `wallets` / `snapshots` via the
   SAME account-xpub fund-routing gate the seedless loader uses — preserving the
   published-xpub == scanned-xpub invariant. A wallet already in the persistor
   and already in `id_map` is a no-op.

Wallets already registered (warm boot) are never disturbed; wallets missing from
the persistor are filled exactly once each.

### 5.3 Seam compliance (M-DONT-LEAK-TYPES / M-PLATFORM-WALLET-FIRST-PARTY)

- All upstream types (`WalletId`, `PlatformWallet`, `WalletAccountCreationOptions`,
  `PlatformWalletError`) stay inside `src/wallet_backend/`. The new methods take
  DET-opaque inputs (`WalletSeedHash`, `&[u8; 64]`, `Option<u32>`) and return
  `Result<(), TaskError>`.
- The seed `&[u8; 64]` is borrowed for the duration of the upstream call and
  never parked — consistent with R3 (an open wallet parks no seed) and the JIT
  secret model. W2 obtains it inside an existing `with_secret_session` scope so
  it is zeroized when the scope ends.
- No upstream type touches DET's SQLite schema, MCP schemas, or user-facing
  strings; upstream errors go to `TaskError` `#[source]` / `with_details`.

---

## 6. Upstream (platform) change — optional, called out for the lead

The chosen design needs **no upstream change** to fix the regression: it reuses
the existing seed-bearing `create_wallet_from_seed_bytes`.

A *truly seedless* watch-only registrar would require an upstream addition and
is **not** on the critical path. If the lead wants to eliminate the
seed-at-register requirement entirely (so even W1/W2 become seedless), land in
platform **PR #3692** (which we own) a public manager method roughly:

```
pub async fn register_watch_only_wallet(
    &self,
    wallet_id: WalletId,
    network: Network,
    account_manifest: Vec<AccountRegistrationEntry>,
    birth_height: u32,
) -> Result<Arc<PlatformWallet>, PlatformWalletError>;
```

…which would `build_watch_only_wallet` + `insert_wallet` + emit the
registration changeset via `persister.store`. **But** DET would still need the
root-xpub `WalletId`, which it does not persist today — so a seedless upstream
registrar is only useful if DET *also* starts persisting the root xpub (a
sidecar schema addition). Recommendation: **defer the upstream registrar.** Ship
the seed-bearing fix now; revisit a seedless registrar + root-xpub sidecar as a
separate hardening item if the first-unlock latency on large migrated installs
becomes a real complaint.

---

## 7. Migration angle

`finish_unwire` (`src/backend_task/migration/finish_unwire.rs`) copies legacy
encrypted seed envelopes and `WalletMeta` into the DET sidecars but **never
registers wallets with the upstream persistor** — confirmed by inspection. So a
migrated install lands in exactly the empty-persistor state that triggers the
bug.

**Recommendation: rely on the W2 cold-boot bridge, do NOT add seed registration
to `finish_unwire`.** Rationale:

- `finish_unwire` runs without the seed in hand for *protected* wallets (the
  envelope is encrypted; the password is not available at migration time).
  Forcing registration there would either require a password prompt mid-migration
  (violates PROJ-008) or skip protected wallets (incomplete).
- W2 already covers migrated wallets: unprotected ones register prompt-free at
  the next cold boot; protected ones register on first unlock. The cold-boot
  bridge is the single, uniform mechanism — adding a partial second one in
  `finish_unwire` would split the logic and create a protected/unprotected
  asymmetry.
- `flush_persister` (`mod.rs:898`) already exists for migration durability; once
  W1/W2 populate the persistor it flushes real rows, no change needed.

If the lead wants migrated *unprotected* wallets to register during migration
rather than at the next boot (marginally faster first-funds visibility), that is
a one-line W2 call over the just-migrated unprotected set at the end of
`finish_unwire::run` — but it is an optimisation, not a correctness requirement.

---

## 8. Task breakdown (for Bilby)

Ordered; each task scoped to one developer agent. **(S)** = needs Smythe
security/funds-safety review. **(M)** = Marvin validates against the 1492173
repro.

- **T1 — `WalletBackend::register_wallet_from_seed` (W1 writer) (S).**
  Add `pub(crate) async fn register_wallet_from_seed(&self, seed_hash:
  &WalletSeedHash, seed: &[u8; 64], birth_height_override: Option<u32>) ->
  Result<(), TaskError>` in `src/wallet_backend/mod.rs`. Body: derive upstream
  `WalletId` from the seed; if already registered, `Ok`; else
  `create_wallet_from_seed_bytes(network, *seed, Default, birth_height_override)`,
  mapping `WalletAlreadyExists`→`Ok`; resolve into `id_map`/`wallets`/`snapshots`
  via the existing account-xpub gate. Keep all upstream types inside the method.
  ~120 lines. Dep: none. (S): seed borrowed, not parked; gate preserved.

- **T2 — Wire W1 into `context::register_wallet` (S).** In
  `src/context/wallet_lifecycle.rs:197`, after the in-memory insert, call
  `backend.register_wallet_from_seed(&seed_hash, seed, birth_height)`. Birth
  height: `None` for a freshly-generated wallet, `Some(0)` for an imported
  one — thread a `WalletOrigin { Fresh, Imported }` (or a `bool imported`) from
  the two call sites (`add_new_wallet_screen`, `import_mnemonic_screen`). Skip
  (log) when the backend is not yet wired — W2 covers it at next boot. ~70 lines.
  Dep: T1.

- **T3 — `WalletBackend::ensure_upstream_registered` (W2 writer) + chokepoint
  wiring (S).** Add the first-unlock/cold-boot variant that takes a held
  plaintext seed (inside a `with_secret_session` scope) and registers any wallet
  present in sidecars but absent from the persistor, with
  `birth_height_override = Some(0)`. Call it from `bootstrap_wallet_addresses_jit`
  (`context/wallet_lifecycle.rs:377`, cold-boot prompt-free path) and from the
  unlock gesture's seed-resolution scope. Idempotent (re-check
  `is_wallet_registered`). ~120 lines. Dep: T1. (S): seed obtained JIT, zeroized
  with the scope; no launch-time prompt for protected wallets.

- **T4 — Birth-height policy constant + plumbing.** Centralise the birth-height
  decision (`None` fresh / `Some(0)` imported-or-recovered) as a small typed
  helper so W1 and W2 agree. Document the genesis-rescan cost tradeoff in the
  helper's rustdoc. Optional network-floor constant left as a `TODO`. ~40 lines.
  Dep: T1.

- **T5 — Typed errors.** Add/confirm `TaskError` variants for the new failure
  modes (upstream register failure wrapping `Box<PlatformWalletError>`; the
  already-registered case is `Ok`, not an error). Reuse the existing
  `WalletBackend { source }` variant where it fits. ~30 lines. Dep: T1.

- **T6 — Tests (M).** (a) Unit: `register_wallet_from_seed` is idempotent
  (second call is a no-op, no double-watch). (b) Unit: birth-height policy maps
  fresh→`None`, imported→`Some(0)`. (c) Backend-e2e cold-boot: create/import a
  wallet, drop the backend, reconstruct, assert `is_wallet_registered` AND the
  watch set is non-empty, then fund an address *below* the registration tip and
  assert the balance appears (the 1492173 repro in miniature). (d) Migration:
  migrate a legacy install with an unprotected wallet, cold-boot, assert W2
  registers it prompt-free. ~220 lines. Dep: T2, T3, T4.

- **T7 — Docs + gap audit.** Note the persistor-write requirement in the
  wallet-backend architecture doc; flip the PROJ-010 regression entry in the gap
  audit; update `docs/user-stories.md` if a "funds visible after cold boot"
  story exists. Dep: T6.

### QA / review matrix

- **Smythe (funds-safety):** T1, T3 — confirm the account-xpub gate still
  rejects unmatched wallets; confirm the seed is borrowed/zeroized and never
  parked; confirm `Some(0)` birth height for any wallet that could pre-hold
  funds (no silent-miss).
- **Marvin (repro validation):** T6c against the real 1492173 deposit (or an
  equivalent below-tip testnet deposit) — the canonical regression gate.

---

## Candy tally (findings by severity)

- **HIGH (funds-correctness) — 2:** (1) no upstream seedless watch-only WRITE
  path exists; the persistor must be populated at a seed-bearing moment (§2/§3).
  (2) birth height MUST be `Some(0)` for imported/recovered/migrated wallets or
  pre-existing deposits (1492173) stay invisible (§4).
- **MEDIUM (correctness/seam) — 3:** (1) DET cannot compute the upstream
  `WalletId` seedlessly — it holds only the hardened account xpub, not the root
  (§0.4), foreclosing the pure-seedless and direct-persistor-write options.
  (2) registration must be idempotent and route through the existing account-xpub
  fund-routing gate (§5.2). (3) migration must NOT register in `finish_unwire`;
  the W2 cold-boot bridge is the single uniform mechanism (§7).
- **LOW (housekeeping/clarity) — 2:** (1) DET persists no birth height of its
  own; genesis is the safe default with a network-floor optimisation deferred
  (§4.3). (2) a seedless upstream registrar (PR #3692) is possible but would
  also need a root-xpub sidecar; defer (§6).

**Total: 7 findings (2 HIGH, 3 MEDIUM, 2 LOW).**
