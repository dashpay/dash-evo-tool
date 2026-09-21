# Add wallet-derived identity keys

The existing Add Key screen starts with “Create from wallet” selected when the
identity has a canonical wallet path on this device; otherwise it opens on the
manual private-key input and explains why. “Create from wallet” shows a
wallet-key-slot chooser; turning it off shows the manual private-key input and
random-key button. Switching modes clears manual input.

## Slot selection: `key_index == key_id`

The default slot is the one equal to the key id the new key will get
(`local max key id + 1`), falling back to the lowest free slot only when that
one is taken or out of range. platform-wallet discovery
(`rs-platform-wallet` `discovery.rs`, `derive_key_breadcrumbs`) and DET's
HASH160 signing fallback (`sign_via_hash160_path_scan`) both assume
`key_index == key_id`; a key off that convention is recoverable by DET's
public-key-matching restore but may be watch-only in other wallets, and a
`key_index == key_id` client adding "the next key" could reuse its private key
in another slot. The chooser stays, so a user can still pick a different free
slot; when the selection differs from the matching free slot, the screen
recommends the matching one. After an add or a rejected slot, the chooser waits
for the identity to reload from the network before offering slots again.

## Derivation and storage

- Support ECDSA_SECP256K1 and ECDSA_HASH160 using the existing canonical ECDSA
  identity-authentication derivation path. Other key types require manual input.
- Resolve the wallet and identity index from unambiguous canonical paths already
  attached to that identity. An unrelated selected wallet is not a substitute.
- Warm the public-key cache asynchronously through the existing secret-access
  mechanism. Only public information reaches the slot chooser. The chooser's
  view (availability, slot load state, occupancy, selection) lives in
  `ui/state/derived_key_chooser.rs` and is recomputed on load, refresh and task
  results — never per frame. The slot load has its own state, apart from the
  submission status; a failed load (or one that finishes with keys still
  missing) waits for an explicit Retry rather than re-dispatching.
- Consider persisted paths and public-key hashes occupied, including disabled
  keys and equivalent secp256k1/HASH160 representations. A missing cache blocks
  selection until loaded; a failed load offers a retry.
- Offer slots below `recovery_scan_bound(max_identity_key_id)`
  (`max_identity_key_id + IDENTITY_KEY_RECOVERY_LOOKAHEAD`, lookahead 6), capped
  at `MAX_DERIVATION_INDEX_LIMIT` (4096) to bound derivation work. The load,
  load-from-wallet and discovery scans build their bounds on the same
  `recovery_scan_bound`, and a unit test fails if any of them stops covering the
  highest selectable slot.
- Send key attributes and the derivation index to the backend. Reload the local
  identity and validate its wallet, type, index and occupancy there; recheck the
  chosen key hash against a newly fetched network identity before broadcasting.
- Derive the public key for the chosen slot from the seed through the secret
  chokepoint before broadcast and compare it with the cached key the chooser
  used. The public-key cache is an unauthenticated sidecar and HASH160 keys have
  no proof of possession on Platform, so a mismatch repairs the cache entry and
  fails with `DerivedKeySeedMismatch` without broadcasting.
- Store `AtWalletDerivationPath`, not a copied private key. Registration and
  subsequent signing resolve the seed through the existing secret chokepoint.
  Protected imported keys retain their current password preflight and sealing.
  A key created from the wallet for a password-protected identity is protected
  by the wallet, not the identity password (mixed state); the screen says so.
- Keep contract bounds, key purpose, security level, fees and the existing master
  key authorization flow. Derivation does not itself add HASH160 support to
  platform-wallet's DashPay profile API.

## Verification

Offline tests cover index holes, the `key_index == key_id` default and its
fallback, the recovery-window invariant across all scan sites, disabled HASH160
aliases, wallet/network/index mismatch, derivation-path serialization, signing
after persistence/reload, stale network duplicates, occupied local slots, every
backend guard of the derived add (unsupported type, unknown identity, index at
or over the limit, occupied index, missing or ambiguous wallet, cached key the
seed does not derive, cold cache fill), checkbox default and manual fallback,
slot-load failure and retry apart from submission, no warm loop, rejected-slot
recovery, disabled slot selection, and submission without private-key bytes.

Manual funded testnet follow-up: load a wallet identity, add a derived HIGH
secp256k1 key, use it to sign a permitted operation, then restore the wallet in
an isolated data directory and verify that the added key is rediscovered. Repeat
with HASH160 for an operation whose backend supports that type. Never record
recovery phrases or private keys in test artifacts.
