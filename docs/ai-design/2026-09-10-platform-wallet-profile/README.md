# DashPay profile writes through platform-wallet

The profile backend task uses the managing wallet's external-signer profile
API. `QualifiedIdentity` remains the signer, so imported and wallet-derived
private keys use the existing secret-access path. Platform-wallet owns key
selection, document construction, signing requests, broadcast, and profile
persistence. DET validates inputs, determines create versus update from a
network query, fetches optional avatar bytes, and maintains display timestamps.

## Scope and limitations

- HIGH and CRITICAL authentication keys are selected upstream. The pinned
  platform-wallet supports ECDSA_SECP256K1 only for profile writes. ECDSA_HASH160
  support belongs in dashpay/platform; this migration does not close #760 for
  HASH160-only identities. Related PRs: #762 and #978.
- An identity must be managed by a loaded wallet. An out-of-wallet identity
  cannot use this upstream API and receives an explicit error.
- Upstream `ProfileUpdate` treats omitted fields as unchanged. Clearing an
  existing field is rejected before broadcast, including removing an avatar,
  rather than reporting success with the old field still present.
- Upstream uses its own write settings; DET's custom state-transition creation
  options are not accepted by this API.
- Adding a wallet-derived key to an existing identity is separate work, tracked
  in MemCan TODO `d8c3a858-4de9-44a6-b59b-4a7c5439df03`.

## Validation

Unit coverage checks validation before network/signing, omission of blank input
fields, rejection of unsupported field removal, and failure without a managing
wallet. The narrow backend E2E module is enabled independently of the deferred
contact-flow suite:

```sh
cargo test --test backend-e2e --all-features \
  profile_create_and_replace_with_high_derived_key -- \
  --ignored --nocapture --test-threads=1
```

It requires a funded testnet `E2E_WALLET_MNEMONIC`. It creates a fresh identity
with a wallet-derived HIGH secp256k1 authentication key and no CRITICAL
authentication key, creates a profile, replaces it, and checks both the upstream
cache and the published document. No recovery phrase or private key belongs in
this document or test fixtures.
