# Profile: `wallet-identity-dpns`

The baseline profile the migration matrix asserts against. Everything in
[`wallet-only`](wallet-only.md), plus a Platform identity carrying a resolved
DPNS name — the state a real user actually has, and the state whose loss after
an upgrade would be most visible and least recoverable.

## Required content

Everything required by [`wallet-only`](wallet-only.md) still applies. In
addition, at the moment the app is quit:

### Identity

- [ ] Exactly one identity is present, of type **User**, on testnet.
- [ ] It was registered **beforehand, with the current build**, from
      `MIGRATION_FIXTURE_MNEMONIC` — the same recovery phrase as Wallet U — and
      only *loaded by ID* in the capture itself. See "Why registration happens
      elsewhere" below.
- [ ] It is loaded on Wallet U (the unprotected wallet), not Wallet P. A
      protected wallet would make every post-upgrade identity assertion depend
      on the unlock flow as well, and confound the two failures.
- [ ] It has a non-default alias, so an upgrade that keeps the identity but
      drops the local metadata around it is still caught.
- [ ] Its identity ID is recorded in the capture report, outside the archive.

### DPNS

- [ ] Exactly one DPNS name is registered to that identity and is fully
      resolved — visible next to the identity in the app before quitting.
- [ ] The name is recorded in the capture report, outside the archive.
- [ ] The name is **not** in an active contest. A contested name's displayed
      state changes on its own as the contest resolves, which would make a
      post-upgrade comparison non-deterministic.
- [ ] Name resolution has actually completed. Quitting while the fetch is still
      in flight produces an identity with an empty name list, which then
      "survives" the upgrade perfectly while proving nothing.

## Why registration happens elsewhere

v0.9.3 does ship identity-registration and DPNS-registration screens, so the
constraint is not a missing feature. It is that v0.9.3 predates the SPV stack
entirely (there is no `src/spv/` at that tag): wallet funds and UTXOs are
visible only through a local Dash Core testnet node reached over RPC and ZMQ
(`TESTNET_core_host`, `TESTNET_core_rpc_*`, `TESTNET_core_zmq_endpoint` in that
era's `.env.example`). Without such a node the binary sees a zero balance and
cannot build the asset lock a registration requires.

Loading an identity by ID does not touch that path — it queries Platform over
`TESTNET_dapi_addresses` — and it fetches the identity's DPNS names as part of
the load, storing them with the identity. So registration is done ahead of time
with the current build, and the old binary only has to *adopt* the result.

A capture host that does run a synced Dash Core testnet node may register from
the old binary directly. Record that in the fixture's `notes`; it changes what
the fixture proves, so it must not be silently substituted.

## Verification

**Era ≤ v0.9.3** — the identity row is in `data.db`:

```sql
-- sqlite3 -readonly data.db
SELECT hex(id), alias, identity_type, is_local, network,
       wallet IS NOT NULL AS linked_to_wallet
FROM identity;
```

**The DPNS name is not SQL-checkable at this era.** `identity.data` is an
opaque bincode blob holding the `QualifiedIdentity`, and the fetched names live
inside it. Verify the name in the UI, on the identity's row, and capture a
screenshot for the report. Do not attempt to decode the blob out-of-band; do
not treat "the row exists" as proof the name is there.

**Current era** — read through the app's own paths (det-cli, or the identity
list) rather than SQL, for the same reason: identity metadata is serialized
into the k/v store.

## Post-upgrade assertions this profile enables

- Every `wallet-only` assertion.
- The identity is present after the upgrade, with the same ID, the same alias,
  the same type, and still linked to Wallet U.
- Its DPNS name is still shown, without needing a manual refresh or a re-load
  by ID. A name that returns only after the user re-fetches it is a partial
  failure, not a pass.
- The identity's keys are still usable — an upgrade that keeps the identity but
  loses its keys leaves the user unable to sign.
- No duplicate identity appears: the legacy drain runs once and adopts the
  existing row rather than adding a second one.
- A second boot changes nothing further.

## Preconditions to re-check before every capture

Shared testnet is mutable and the fixture's on-chain state is months old by the
time it matters. Before spending time on the old binary, confirm with the
**current** build that:

- [ ] The identity ID still resolves on testnet and still has a positive credit
      balance.
- [ ] The DPNS name still resolves to that identity (a network reset can drop
      it, and the name can then be taken by someone else).
- [ ] The fixture wallet still holds dust-level funds and nothing more.

If any of these fail, fix the on-chain state first, or capture
[`wallet-only`](wallet-only.md) and record why in the fixture's `notes`. A
fixture captured against stale on-chain state fails the matrix later for a
reason that has nothing to do with migration.
