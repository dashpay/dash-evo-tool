# rs-platform-wallet: own and expose the per-address platform nonce (advance-on-submit + local reader)

> Filed upstream as [dashpay/platform#3825](https://github.com/dashpay/platform/issues/3825) (2026-06-09).
>
> Citations are pinned to platform rev `9e1248cb0ae46c6fbfc5bd9d92540bec8c6d00e8` and key-wallet (rust-dashcore) rev `eb889af13f667ed39c35e8e8a0830eeedf523476`.

### User Story

As a **Platform Developer / wallet integrator** (Dash Evo Tool, the iOS
wallet, and any future consumer of `rs-platform-wallet`), I want the
platform-wallet crate to **own** the per-address platform nonce — keep it
current as I spend, and let me **read it locally** — so that spending does not
leak my IP to DAPI nodes on every transaction (today each spend re-fetches the
nonce over the network), and so that each integrator does not have to
reimplement the same fragile nonce custody (an optimistic cache, a
bump-after-send step, and a mismatch-retry) on top of the crate. Today every
integrator independently re-derives that bookkeeping; the wallet crate already
holds the address state and should be the single owner of the nonce alongside
the balance.

### Problem / current behavior

`PlatformPaymentAddressProvider` already stores the per-address nonce next to
the balance — `PerAccountPlatformAddressState.found: BTreeMap<PlatformP2PKHAddress, AddressFunds>`
(`packages/rs-platform-wallet/src/wallet/platform_addresses/provider.rs:75`),
where `AddressFunds { nonce, balance }` carries both
(`packages/rs-sdk/src/platform/address_sync/types.rs:53`).

But that nonce is **only ever a snapshot from the last completed sync** — no
send path advances it:

- **The only writers of `found` are sync and rehydration.** Sync flows through
  `on_address_found` → in-sync scratch → `sync_finished` flush
  (`provider.rs:584`, `provider.rs:692`), driven by
  `PlatformAddressWallet::sync_balances`
  (`packages/rs-platform-wallet/src/wallet/platform_addresses/sync.rs:29`).
  Rehydration seeds it via `insert_persisted_entry`
  (`provider.rs:116`) from `initialize_from_persisted`
  (`packages/rs-platform-wallet/src/wallet/platform_addresses/wallet.rs:155`).
  The nonce written here is the GroveDB-proven confirmed on-chain value
  (decoded at `packages/rs-sdk/src/platform/address_sync/mod.rs:869`).

- **Every send path takes `provider.read()` only** and never writes the nonce
  back into the live provider:
  - transfer: `let guard = self.provider.read().await;`
    (`packages/rs-platform-wallet/src/wallet/platform_addresses/transfer.rs:254`)
  - withdrawal: `self.provider.read().await`
    (`packages/rs-platform-wallet/src/wallet/platform_addresses/withdrawal.rs:112`)
  - fund-from-asset-lock: `self.provider.read().await`
    (`packages/rs-platform-wallet/src/wallet/platform_addresses/fund_from_asset_lock.rs:361`)

- **The post-transition nonce reaches sqlite but not the live provider.** The
  auto-fetch send returns the post-transition `AddressInfos` to the caller; the
  send path pushes it into a persistence changeset
  (`transfer.rs:304`, `transfer.rs:314`; `withdrawal.rs:153`;
  `fund_from_asset_lock.rs:430`, `fund_from_asset_lock.rs:300`) and pushes
  **balance only** onto the managed account via `set_address_credit_balance`
  (`transfer.rs:299`, `withdrawal.rs:141`, `fund_from_asset_lock.rs:429`). The
  in-memory `provider.found` nonce is left untouched until the next sync
  re-observes the address.

- **The crate explicitly delegates nonce custody to the caller.** When the
  changeset is applied to the in-memory managed account, the nonce is dropped
  on purpose:

  > `// Nonce isn't stored on ManagedPlatformAccount; callers that need it`
  > `// persist it via their own store (see evo-tool's platform_address_balances`
  > `// table which writes both balance and nonce from the changeset).`

  — `packages/rs-platform-wallet/src/wallet/apply.rs:195`. (Confirmed at the
  key-wallet layer: `ManagedPlatformAccount` stores
  `address_balances: BTreeMap<PlatformP2PKHAddress, u64>` — balance only, no
  nonce field —
  `key-wallet/src/managed_account/managed_platform_account.rs:47`.)

Net: the provider's nonce means *"the confirmed on-chain nonce as of the last
completed sync that observed this address."* It is **not** the next-to-use
value, and it is **stale between a send and the next sync**.

### Why a reader alone is insufficient

Exposing a public reader over today's `found` nonce would hand integrators a
value that is correct only until the first send, then silently stale until the
next sync re-observes the address. A second address-based transition built in
that window reuses the old nonce and consensus rejects it with
`AddressInvalidNonceError { provided_nonce: N, expected_nonce: N+1 }`. So the
reader must be paired with **advance-on-submit** for the owned nonce to be
trustworthy as a next-to-use source. (This is the same `AddressInvalidNonceError`
surface tracked from the SDK side in #3407 and #3611 — see *Related*.)

There is also **no `AddressInvalidNonceError` handler anywhere** in `rs-sdk`,
`rs-platform-wallet`, or `swift-sdk` (established by the merged review on #3784).
So fetch-per-send is the *only* line of defense — if a fetched nonce is ever
stale (e.g. the rapid same-address send window, or replica lag per #3611), the
transition simply fails with no retry safety net. Owning and advancing the nonce
locally is more robust precisely because there is no upstream recovery path to
fall back on.

### Privacy impact (primary motivation)

The decisive cost of fetch-per-send is privacy, on the hot path, every time the
user spends:

- Every address spend re-fetches the nonce from the network
  (`fetch_inputs_with_nonce` → `AddressInfo::fetch_many`). Because the SDK
  resolves a *proved* query by querying for quorum agreement, a single nonce
  fetch contacts **two DAPI nodes**. So each spend exposes the user's IP to two
  DAPI nodes solely to obtain a nonce — doubling the network-metadata exposure
  versus a design that does not fetch per spend.
- This is an inherent, repeated leak: the more a user transacts, the more DAPI
  operators can correlate their IP with their platform addresses and spending
  activity. The exposure scales linearly with spend count and is unavoidable as
  long as the nonce lives on the server side of each send.
- A locally-owned nonce — read from platform-wallet's own state — eliminates the
  per-spend nonce fetch entirely: **zero incremental IP/metadata exposure** for
  obtaining the nonce. This is the core reason to own the value locally rather
  than re-fetch it on every transaction.

This trade-off is exactly what #3784 codifies: its merged rationale is that the
synced provider nonce is "cosmetic" *because* every spend re-fetches the
authoritative value at build time. In other words, the current design's
correctness deliberately depends on the per-spend network round-trip that
creates this privacy cost. We are asking to revisit that trade-off — make the
locally-owned nonce authoritative so the round-trip (and its IP exposure) is no
longer required on the spend path.

### Proposed change — two parts

#### (a) Advance + persist the nonce on submit

In the `rs-platform-wallet` send paths, after the transition is built/submitted,
take `provider.write()` and update each input address's `AddressFunds.nonce` in
`found` to the just-used value, in lockstep with the changeset that is already
written to sqlite:

- transfer — around `transfer.rs:254`–`transfer.rs:314`
- withdrawal — around `withdrawal.rs:112`–`withdrawal.rs:153`
- fund-from-asset-lock — around `fund_from_asset_lock.rs:361`–`fund_from_asset_lock.rs:430`
- the shield equivalent (the `ShieldFunds` path)

The value is already in hand: these paths receive the post-transition
`AddressInfos` and persist it to sqlite via the changeset
(`transfer.rs:304`/`:314`) — they simply do not write it back into the live
provider. This change closes that gap so the in-memory provider stays
consistent with the sqlite row.

**Submit-time-optimistic vs confirmation-time.** Writing the advance at submit
time (optimistically, before chain confirmation) is what covers rapid,
back-to-back sends from the same address within one sync window — the dominant
real-world case. A confirmation-time write leaves the same stale window open
between two quick sends. We recommend **optimistic advance on submit**, with the
next sync reconciling against the proven value (so a failed/abandoned submit
self-heals on the following sync).

#### (b) Public local reader on `PlatformAddressWallet`

`PlatformAddressWallet` is already publicly reachable via
`manager.get_wallet(id).platform()`
(`packages/rs-platform-wallet/src/manager/accessors.rs:260` →
`packages/rs-platform-wallet/src/wallet/platform_wallet.rs:129`). Add a
by-address reader that clones out of `provider.read()`:

```rust
/// Confirmed-and-locally-advanced funds (balance + nonce) for one
/// platform address, or `None` if the wallet has never observed it.
pub async fn get_address_funds(
    &self,
    address: &PlatformAddress,
) -> Option<AddressFunds>;
```

**No type cascade is required.** Every type in the signature is already public —
`PlatformAddress` (`pub enum`,
`packages/rs-dpp/src/address_funds/platform_address.rs:39`),
`PlatformP2PKHAddress` (`pub struct`,
`key-wallet/src/managed_account/platform_address.rs:27`), and `AddressFunds`
with `pub nonce`/`pub balance`
(`packages/rs-sdk/src/platform/address_sync/types.rs:53`). The accessor returns
owned/cloned values, so `PlatformPaymentAddressProvider` can stay `pub(crate)`
(`provider.rs:176`) — no internals are exposed.

**Prefer by-address over enumeration.** A `get_address_funds(&addr)` reader
indexes `found` directly (keyed by address), sidestepping the bijection hazard
that an enumerating `addresses_with_funds()` would hit (see QA-002 below). An
enumerating variant can be offered too, but only after the bijection issue is
fixed.

### Edge cases & residual gaps

- **Absent / brand-new address.** `found` has no entry for an address that has
  never been synced, so the reader returns `None`. Integrators should treat
  `None` as "seed nonce 0" for a never-used address. The `Option` return makes
  "unknown" distinguishable from "known, nonce 0" — preferable to a bare
  `AddressFunds` that conflates the two.

- **QA-002: bijection eviction on rehydration.** The `platform_addresses` table
  primary key is `(wallet_id, address)`
  (`packages/rs-platform-wallet-storage/migrations/V001__initial.rs:209`), not
  `(account_index, address_index)`, so two addresses in one account can share an
  `address_index`. `insert_persisted_entry` does
  `self.addresses.insert(index, address)` (`provider.rs:115`) on a
  `BiBTreeMap` (`bimap 0.6.3`), which evicts the prior pair on left-key
  collision; the nonce survives in `found` (keyed by address) but the evicted
  address is silently dropped from `current_balances`, which filters via
  `addresses.get_by_right(p2pkh)?` (`provider.rs:709`). A by-address reader is
  immune (it reads `found` directly); an enumerating reader is not. We recommend
  shipping the by-address reader **and** fixing the bijection — either key the
  load/PK on `(account_index, address_index)` or make `insert_persisted_entry`
  reject/repair collisions instead of silently evicting.

- **Crash between submit and persist.** If the optimistic advance is written to
  the provider and sqlite but the process dies before the changeset commits, the
  advance is lost and a post-restart read reverts to the prior confirmed nonce.
  This is acceptable only if submit → persist is ordered before broadcast (or
  the two writes are atomic). Worth calling out in the implementation so the
  ordering is deliberate.

- **Async reader → off-thread only.** The provider is
  `Arc<RwLock<Option<PlatformPaymentAddressProvider>>>` behind a tokio **async**
  `RwLock` (`wallet.rs:31`), so `get_address_funds` must be `async`. Consumers
  must call it off any UI/frame thread (for DET, via its task system) to avoid
  blocking inside the runtime.

- **Reorg / full rescan.** `prepare_for_sync` preserves `found` and clears only
  `absent` (`provider.rs:381`, `provider.rs:432`); a full rescan re-proves and
  overwrites with the freshly-proven nonce. If the chain itself rolled back
  below a transition, the proven nonce legitimately drops — that mirrors chain
  state, not corruption. No special handling needed; the optimistic advance from
  (a) is reconciled by the next sync.

- **u32 overflow.** `AddressNonce` is `u32` and `nonce_inc` does an unchecked
  `nonce + 1` (`packages/rs-sdk/src/platform/transition/address_inputs.rs:45`).
  Overflow only at 2^32 transitions from a single address — theoretical, debug-
  panic only; noted for completeness.

### Scope estimate

- **(b) reader** — ~15 LOC, additive, no type changes; clones under the lock.
- **(a) advance-on-submit** — the substantive change: a `provider.write()` +
  nonce write in each send path (transfer / withdrawal / fund-from-asset-lock /
  shield), reusing the post-transition `AddressInfos` the paths already receive.

Together these let integrators **delete their own per-address nonce tracking**
(optimistic cache, bump-after-send, mismatch-retry) and read the
crate-owned value each time — the platform-wallet crate becomes the single owner
of the nonce alongside the balance.

### Alternatives considered (and why they don't fit)

Hard constraint for this design: read the per-address nonce from
platform-wallet's **local** state on every spend — not from the server/chain on
the hot path, and not reimplemented separately by each integrator. Each
mechanism below is adjacent but fails one of those two requirements.

- **Chain reader `AddressInfo::fetch_many`** (`packages/rs-drive-proof-verifier/src/types.rs:126`,
  `packages/rs-sdk/src/platform/fetch_many.rs:542`). Proof-fetches `(balance, nonce)`
  fresh from chain. Correct and authoritative, but it is a network round-trip per
  read and reads *from the server* — exactly the dependency this design removes.
- **Auto-fetch send APIs** — `Sdk::transfer_address_funds` /
  `withdraw_address_funds` (`packages/rs-sdk/src/platform/transition/{transfer_address_funds,address_credit_withdrawal}.rs`),
  `Identity::top_up_from_addresses` / `put_with_address_funding_fetching_nonces`
  (`.../transition/{top_up_identity_from_addresses,put_identity}.rs`). Internally
  `fetch_inputs_with_nonce` + `nonce_inc`, fetching the nonce from chain on every
  send. Robust (this is the iOS pattern) but keeps a per-send network dependency
  on the hot path and yields no locally-owned value to read between sends.
- **iOS / FFI approach** — `rs-platform-wallet-ffi`
  (`identity_registration.rs:21`: *"Nonces are intentionally **not** part of this
  struct — the SDK's … fetches at submit"*); `PersistentPlatformAddress.nonce` is a
  one-way display mirror (`persistence.rs:671`). iOS leans entirely on the
  server auto-fetch above. Same rejection: server-sourced, no local owner.
- **Existing public balance reader `addresses_with_balances()`**
  (`packages/rs-platform-wallet/src/wallet/platform_addresses/wallet.rs:302`) —
  public, local, but **balance only**, no nonce. This is the closest existing
  shape; proposal (b) is its nonce-carrying sibling.
- **`sync_watermark()`** (`wallet.rs:325`) — public and local, but returns
  `last_known_recent_block`, not a per-address nonce. Wrong granularity and
  shape; included only because it is the precedent for the reader pattern (see
  #3723 below).
- **Add the nonce to key-wallet `ManagedPlatformAccount`**
  (`key-wallet/src/managed_account/managed_platform_account.rs:47`, balance-only
  today). A possible home, but the provider already holds the nonce in `found`;
  adding a second copy on the managed account duplicates state and re-opens the
  "who is authoritative" problem. Keep one owner — the provider.
- **DET's current optimistic cache + `ShieldedNonceMismatch` retry.** The status
  quo this proposal exists to **delete** — the per-integrator nonce custody the
  User Story calls out. Not an alternative to keep; the thing being removed.
- **#3407's SDK fetch-cache + auto-retry.** Symptom mitigation at the fetch
  layer; still re-fetches from the server and establishes no locally-owned
  nonce. Complementary, not a substitute.

### Related

- **#3723** (merged 2026-05-21) — `feat(platform-wallet): expose sync_watermark()
  on PlatformAddressWallet`. **Direct precedent for proposal (b):** added a
  `pub async fn sync_watermark()` on `PlatformAddressWallet` backed by a
  `pub(crate)` provider mirror, no type cascade, "pure additive surface." Our
  `get_address_funds` is the same mechanism applied to the per-address nonce
  rather than the watermark. Not a duplicate (it exposes the watermark, not the
  nonce).
- **#3784** (merged 2026-06-07) — `docs: clarify address-sync catch-up nonce`.
  **Important design context, not a dup.** A maintainer review states the synced
  provider nonce is "cosmetic … every address-spend path re-fetches the
  authoritative nonce at build time (`fetch_inputs_with_nonce` + `nonce_inc`), so
  the synced value … never lands in a broadcast transition." That is the current
  fetch-per-send stance this proposal asks to revisit: making the provider nonce
  *authoritative and locally readable* is precisely the change, so the "cosmetic"
  framing is the status quo we want to upgrade.
- **#3650** (open) — `fix(sdk): address-sync no longer silently discards balance changes for post-snapshot addresses (Found-025)`. The sync-layer (rs-sdk) fix that ensures freshly-derived addresses and their balance changes actually land in `provider.found` (previously a post-snapshot address was silently dropped, never reaching `result.found`/`on_address_found`). It is the **foundation this proposal builds on**: a locally-owned, advanced nonce is only correct if `found` is completely and correctly populated by sync. It is also the PR whose review introduced the "cosmetic nonce / fetch-per-send" rationale — clarified in its stacked follow-up #3784 — that this proposal revisits. (Comment context: the "callers own the nonce" delegation at `apply.rs:196` was authored by the same maintainer in `4809f802` on 2026-04-17.)
- **#3739** (open, rs-sdk) — `address_sync` persists `nonce=0` for addresses
  first surfaced via incremental RPC. Sync-layer *data quality*; complementary
  — a correct owned nonce depends on sync writing the right value in the first
  place. Distinct from provider ownership/reader.
- **#3611** (open, dapi) — stale nonce/balance read after a confirmed broadcast
  due to DAPI replica lag. A read-after-write consistency problem in the
  *fetch* path; orthogonal to owning the in-memory provider nonce.
- **#3407** (closed) — SDK auto-retry / stale-mark on `AddressInvalidNonceError`.
  Mitigates the *symptom* at the SDK fetch-cache layer and notes address nonces
  are "fetched fresh each time via `fetch_inputs_with_nonce()`" with no cache —
  this issue proposes the crate *own* the value instead, removing the need to
  re-fetch or retry in the common case.
- **#3737** (closed) — migrate `AddressPool` from
  `platform_payment_managed_account` to `PlatformPaymentAddressProvider`.
  Precedent: owned per-address state is being consolidated onto the provider;
  the nonce is the natural next field to own there.
- **#3625** (merged 2026-06-09) — `feat(platform-wallet)!: add
  platform-wallet-storage crate (sqlite persister)`. Establishes the
  `platform_addresses` table (with the `nonce` column) and the rehydration path
  this proposal builds on. Already present in the pinned tree (`9e1248c`).
- **#3692** (open) — `feat(platform-wallet): watch-only rehydration from
  persistor (seedless load)`. The cold-start load path that seeds the provider's
  `found` (including the nonce) from sqlite; the reader in (b) surfaces what this
  rehydrates.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
