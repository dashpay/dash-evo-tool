# Profile: `wallet-only`

The floor profile. Proves that wallets — including a password-protected one —
survive an upgrade with their aliases, derived addresses and at-rest protection
intact. No Platform identity is involved, so this profile can be captured
without any on-chain registration and without waiting on Platform state.

Use it for versions where a full `wallet-identity-dpns` capture is blocked
(testnet reset, no funded fixture wallet, no identity available). It is a
weaker fixture, not a substitute: the baseline matrix asserts against
[`wallet-identity-dpns`](wallet-identity-dpns.md).

## Required content

A captured data directory satisfies this profile when **all** of the following
are true at the moment the app is quit.

### Wallets

- [ ] Exactly two recovery-phrase wallets exist.
- [ ] **Wallet U** (unprotected) is imported from `MIGRATION_FIXTURE_MNEMONIC`,
      has a non-default alias, and is the main/selected wallet.
- [ ] **Wallet P** (protected) is imported from
      `MIGRATION_FIXTURE_PROTECTED_MNEMONIC` with the password
      `correct horse battery staple`, and has its own distinct non-default
      alias. A password *hint* is set only where the era's import screen offers
      the field — v0.9.3's does not, and always stores `password_hint` as
      `NULL`, so a fixture from that tag must not be failed for its absence.
- [ ] The two wallets have different seeds. Importing the same phrase twice
      produces one wallet, not two, and silently reduces this profile to a
      single-wallet fixture.
- [ ] Wallet U is *not* password-protected. Its whole purpose is to catch an
      upgrade that hands an unprotected wallet a password the user cannot
      supply.

### Addresses

- [ ] At least one receive address is derived and visible for each wallet, so
      the fixture carries derivation state and not just wallet metadata.
- [ ] Addresses are recorded outside the archive too — in the capture report —
      so a post-upgrade run can assert that the *same* addresses come back
      rather than merely that some addresses exist.

### Settings

- [ ] The network selector is on **testnet** and that choice is persisted (it
      is the network the upgraded build must come back on).
- [ ] No mainnet wallet, identity or setting is present anywhere in the
      directory.

### Balance

- [ ] Dust only. This profile needs no funds at all; if the wallet holds a
      balance from an earlier capture, that is tolerated but must stay at the
      minimum. See the threat model in [`../README.md`](../README.md).

## Verification

**Era ≤ v0.9.3** — single `data.db`, schema version 11:

```sql
-- read-only, always; a plain open can checkpoint the WAL and mutate the fixture
-- sqlite3 -readonly data.db
PRAGMA user_version;                                   -- expect 11 at v0.9.3
SELECT alias, is_main, uses_password, network FROM wallet;
SELECT COUNT(*) FROM wallet_addresses GROUP BY seed_hash;
```

**Current era** — `det-app.sqlite` plus `det-<network>.sqlite` plus
`secrets/det-secrets.pwsvault`. Wallet metadata lives in the k/v store as
serialized values rather than typed columns, so prefer the app's own read paths
(det-cli `core-wallets-list`) over hand-written SQL.

## Post-upgrade assertions this profile enables

The harness owns the assertions; they are listed here so a capture that cannot
support one is recognised as incomplete before it is packed.

- Both wallets are present after the upgrade, with their aliases unchanged.
- Wallet U opens with no password prompt.
- Wallet P still requires a password: the fixed literal opens it, and a wrong
  password is rejected. At-rest protection survived — it was not downgraded to
  an unprotected seed in passing.
- Every recorded address comes back identical.
- The upgraded app starts on testnet.
- A second boot changes nothing further: migration is idempotent, and no
  migration banner reappears.

## Known limitations

- v0.9.3 has no SPV stack; balances and UTXOs came only from a local Dash Core
  node over RPC/ZMQ. A capture made without such a node shows a zero balance
  and an empty `utxos` table. That is expected and does not invalidate the
  fixture — this profile asserts on wallets, addresses and protection, never on
  balance.
