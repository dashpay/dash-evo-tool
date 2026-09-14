# DashPay profile writes through platform-wallet

The profile backend task uses the managing wallet's external-signer profile
API. `QualifiedIdentity` remains the signer, so imported and wallet-derived
private keys use the existing secret-access path. Platform-wallet owns key
selection, document construction, signing requests, broadcast, and profile
persistence. DET validates inputs, determines create versus update from a
network query, fetches optional avatar bytes, and maintains display timestamps.

Avatar download or decoding failures stop the task before any profile query or
write. Typed errors preserve the cause and tell the user how to correct the URL.
DET checks the managing wallet's identity against the profile authentication policy
before invoking the profile API; incompatible keys produce an actionable error.

Failed display-timestamp writes remain in a per-network, per-identity in-memory
repair queue. Profile reads retry only the local write and return the pending
values even if storage still fails. A newer save replaces the pending values;
a successful repair removes them. Pending repairs do not survive an application
restart, and do not resubmit a paid profile transition.
Fetched profiles initialize missing timestamps under the same lock as profile
saves, preserving stored dates and pending repairs. Failed timestamp reads do
not permit initialization. Downloaded avatar bytes also populate the view cache.

## Scope and limitations

- DET accepts active HIGH or CRITICAL authentication keys of type
  ECDSA_SECP256K1 or ECDSA_HASH160. The existing signer supports both types.
  HASH160 publication requires [Platform #4653](https://github.com/dashpay/platform/pull/4653),
  which widens upstream's selector for both create and replace. The Platform
  dependency revision is unchanged and does not include that fix, so HASH160
  writes still fail upstream until it is updated. Contact ECDH keys are unaffected.
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
wallet, avatar failures before network access, key compatibility, and timestamp
repair after transient storage failures. The narrow backend E2E module is enabled independently of the deferred
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

After updating Platform to a revision containing #4653, run the matching HASH160
case with the same flags and the filter
`profile_create_and_replace_with_high_hash160_derived_key`. It registers HASH160
MASTER and HIGH authentication keys, with no full-public-key signing alternative,
then verifies both profile creation and replacement through the same backend path.
