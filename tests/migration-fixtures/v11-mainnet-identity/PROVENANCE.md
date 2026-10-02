# Mainnet identity migration fixture

`data.db` is the exact 180,224-byte database supplied by the operator on
2026-10-02 as `/tmp/data.db.mainnet`. Its SHA-256 is
`81309e1fb1f8158aa1b5bb0b61a7c662d5b0926115cda629bc863496d91590c9`.
The exact capturing release is unknown; the stored schema version is 11 and
all network rows use the historical mainnet spelling `dash`.

The operator explicitly authorized publication of the complete database,
including its unprotected wallet seed, for permanent public testing. This
wallet and identity are public test material: never send funds to them or
reuse their keys. This is an explicit exception to the usual testnet-only
fixture policy and prohibition on committing wallet seeds.

Contents:

- One unprotected wallet, `e2e-test-mainnet`, with seed hash
  `9fca92d3c5cd02c2502101331ef7ca7c40b64cacbceef700f9f14735f7f0d981`.
- One active User identity, `nuUKwvjw4wz8W7bstw5TMBgvsFNFxJiHb3DystC3RNu`,
  bound to wallet identity index 3, with four wallet-derived ECDSA_HASH160 keys:
  AUTHENTICATION/MASTER (0), AUTHENTICATION/CRITICAL (1),
  AUTHENTICATION/HIGH (2), TRANSFER/CRITICAL (3).
- Thirteen wallet addresses, two asset-lock records, one top-up record and
  no UTXOs. The identity's historical cached balance is 665,485,540 credits;
  a local snapshot does not establish its current on-chain balance.

The legacy private-key inventory stores compressed public keys in HASH160
key snapshots, while the identity's public-key map stores their hashes.
The regression test boots the real CLI, reopens the migrated storage, signs
an arbitrary local message with every key and verifies it against the
original public-key map. Three boots cover a fresh upgrade, repair of an
already-migrated profile with old key snapshots and completed drain markers,
and an unchanged repeat. Signing uses exact copies of each boot's resulting
profile to isolate in-process advisory locks. The source stays byte-identical.
The test does not broadcast transactions or require chain sync.

The final signing copy also loses its wallet seed and identity key inventory
deliberately. `RestoreFromPreviousVersion` must recover the seed and all four
keys from the preserved database, and all four signatures must verify in the
same session. Missing wallets remain an explicit restore action, as in #1043;
startup does not resurrect wallets the user may have removed deliberately.

Run:

```sh
cargo test --locked --test migration-matrix --all-features mainnet_identity -- --nocapture
```
