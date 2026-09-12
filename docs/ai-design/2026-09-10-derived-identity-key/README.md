# Add wallet-derived identity keys

The existing Add Key screen starts with “Derive from wallet” selected. It shows
an index chooser; unchecking shows the manual private-key input and random-key
button. Switching modes clears manual input. The first free index is selected,
including after refreshing an identity or adding another key.

## Derivation and storage

- Support ECDSA_SECP256K1 and ECDSA_HASH160 using the existing canonical ECDSA
  identity-authentication derivation path. Other key types require manual input.
- Resolve the wallet and identity index from unambiguous canonical paths already
  attached to that identity. An unrelated selected wallet is not a substitute.
- Warm the public-key cache asynchronously through the existing secret-access
  mechanism. Only public information reaches the index chooser.
- Consider persisted paths and public-key hashes occupied, including disabled
  keys and equivalent secp256k1/HASH160 representations. A missing cache blocks
  selection until loaded; a failed load offers a retry.
- Offer indices below `max_identity_key_id + 6`, capped at 4096 to bound work.
  The range fits the existing seed-recovery scan. It grows as key IDs grow.
- Send key attributes and the derivation index to the backend. Reload the local
  identity and validate its wallet, type, index and occupancy there; recheck the
  chosen key hash against a newly fetched network identity before broadcasting.
- Store `AtWalletDerivationPath`, not a copied private key. Registration and
  subsequent signing resolve the seed through the existing secret chokepoint.
  Protected imported keys retain their current password preflight and sealing.
- Keep contract bounds, key purpose, security level, fees and the existing master
  key authorization flow. Derivation does not itself add HASH160 support to
  platform-wallet's DashPay profile API.

## Verification

Offline tests cover index holes, disabled HASH160 aliases, wallet/network/index
mismatch, derivation-path serialization, signing after persistence/reload, stale
network duplicates, occupied local slots, checkbox visibility and clearing,
disabled index selection, and submission without private-key bytes.

Manual funded testnet follow-up: load a wallet identity, add a derived HIGH
secp256k1 key, use it to sign a permitted operation, then restore the wallet in
an isolated data directory and verify that the added key is rediscovered. Repeat
with HASH160 for an operation whose backend supports that type. Never record
recovery phrases or private keys in test artifacts.
