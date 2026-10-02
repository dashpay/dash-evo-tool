# Mainnet identity migration fixture

`data-with-imported-key.db` is the exact 180,224-byte database supplied by
the operator on 2026-10-02 as `/tmp/data.db.mainnet`. Its SHA-256 is
`23fc06e82df316dca56cb6ae338aaa470790e6e1cdf19f35cdcd777d1d9485d3`.
The exact capturing release is unknown; the stored schema version is 11 and
all network rows use the historical mainnet spelling `dash`.

Key 4 is ECDSA_SECP256K1 AUTHENTICATION/HIGH.
Unlike keys 0–3, this key was imported as plaintext rather than derived from
the wallet. The operator also explicitly authorized its private key as a
public test constant (`IMPORTED_PRIVATE_KEY` in the regression test).

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
  AUTHENTICATION/HIGH (2), TRANSFER/CRITICAL (3), plus the imported
  ECDSA_SECP256K1 AUTHENTICATION/HIGH key (4).
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

The test also checks that key 4 becomes an `InVault` reference and resolves to
the exact private key supplied by the operator after every boot and after recovery. Its
signatures are verified against the original full public key.

The final signing copy also loses its wallet seed and identity key inventory
deliberately. `RestoreFromPreviousVersion` must recover the seed and all five
keys from the preserved database, and every signature must verify in the same
session. The imported key's actual vault entry is also deleted before restoration,
so a surviving secret cannot mask a recovery failure. Missing wallets remain
an explicit restore action, as in #1043;
startup does not resurrect wallets the user may have removed deliberately.

Run:

```sh
cargo test --locked --test migration-matrix --all-features mainnet_ -- --nocapture
```
