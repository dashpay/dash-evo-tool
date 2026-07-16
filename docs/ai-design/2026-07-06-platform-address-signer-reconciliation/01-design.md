# Platform-address signer reconciliation

## Symptom (live mainnet, real funds)

A user withdrew 20 DASH from a Platform address back to Core. Nothing was
broadcast (funds safe), but signing failed with:

```
Platform address P2pkh([…]) not found in wallet
```

The balance was visible in the withdraw picker, yet the signer refused it.

## Root cause: two maps that should stay in sync, but don't

`Wallet` (`src/model/wallet/mod.rs`) keeps two per-address maps:

- `platform_address_info: BTreeMap<Address, PlatformAddressInfo>` — balance/nonce
  per address. Feeds the balance/withdraw UI.
- `watched_addresses: BTreeMap<DerivationPath, AddressInfo>` — addresses the
  wallet has a derivation path for. Feeds `PlatformPathIndex::from_wallet`, i.e.
  the signer's view of "addresses I can sign for".

The signer (`DetPlatformSigner::sign_with_address`) resolves the target address
through `PlatformPathIndex`, which is built purely from `watched_addresses`. No
path → refuse to sign.

`WalletAddressProvider::apply_results_to_wallet` keeps the two maps in sync
correctly, because it derives every candidate itself and therefore knows each
address's index. But two other paths update `platform_address_info` from raw
`(hash160, balance, nonce)` triples pushed by the upstream `platform-wallet`
coordinator — which does its own account/gap-limit bookkeeping and never hands
DET a derivation index:

- `AppContext::apply_platform_address_push` (`src/context/mod.rs`) — every live
  coordinator push (~15 s).
- `AppContext::warm_start_platform_addresses` — cold boot.

So an address the coordinator discovers shows a real balance in the UI while the
signer has no path for it. That is exactly what the user hit.

## Fix: seedless reverse-derivation reconciliation

The DIP-17 final index is a **non-hardened** child, so a platform-payment
address derives from the account-level xpub alone (no seed). We exploit that:

1. **Cache the platform-payment account xpub on `Wallet`**
   (`platform_payment_account_xpub: Option<ExtendedPubKey>`), mirroring the
   existing `master_bip44_ecdsa_extended_public_key` cache. Populated eagerly in
   `Wallet::new_from_seed`; `Option` for backward compatibility with wallets
   persisted before the field existed (it is **not** persisted — re-derived JIT).

2. **`Wallet::ensure_platform_payment_account_xpub(&mut self, seed, network)`** —
   backfills the cache the next time the seed is borrowed through the JIT
   chokepoint. A cheap no-op once cached. Solves the seedless-backfill problem
   for existing wallets: the affected user's wallet self-heals on the next
   unlock without re-entering a recovery phrase.

3. **`Wallet::reconcile_platform_address(&mut self, address, network) -> bool`** —
   no-ops (true) if already known; returns false (debug log) if the xpub is not
   cached yet; otherwise reverse-derives candidates from the cached xpub over a
   bounded window and, on a match, registers `known_addresses` +
   `watched_addresses` exactly as `apply_results_to_wallet` does
   (`CLEAR_FUNDS` / `PlatformPayment`). A foreign/out-of-window address returns
   false with a **warn** (no silent caps).

### Where it runs

- `apply_platform_address_push` and `warm_start_platform_addresses` — reconcile
  each pushed address. The coordinator only ever pushes OWNED addresses
  (`event_bridge.rs`), so a match is found on the first push after the xpub is
  cached, then all later pushes short-circuit on `known_addresses`. No re-scan,
  no log spam in steady state.
- `fetch_platform_address_balances` — backfills the xpub while the seed is
  already borrowed, so the seedless push path can reconcile afterward.
- `withdraw_from_platform_address` — backfills **and** reconciles the actual
  input addresses, then rebuilds the path index from the reconciled wallet
  before signing. This makes the reported withdrawal self-heal on the first
  retry rather than depending on a later background push.

### Bounds

Search ceiling: `max(highest_registered_index, DEFAULT_GAP_LIMIT) + 500`
(25× the default gap limit). Owned addresses match within the first handful of
indices; the ceiling only bounds the pathological foreign-address case so the
search can never spin unbounded. Past the ceiling → warn + false.

## Fund-safety parity

Reverse-derivation registers the SAME address the seed path produces (the DIP-17
index is a non-hardened child of the account xpub), so the registered
derivation path is correct and the signer derives the correct key. Covered by
`reconcile_registers_platform_address_beyond_gap_limit` and
`reconciled_address_becomes_signable_backward_compat` (index 25, past the gap
limit, no seed at reconcile time).

Crucially, reconciliation can only register an address that actually derives
from the wallet's own xpub, so it can never mis-register a foreign/orphan
address for signing.
