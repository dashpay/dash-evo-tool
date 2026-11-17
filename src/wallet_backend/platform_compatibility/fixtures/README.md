# Platform database compatibility fixtures

`67d4ef3.sql` and `e3cd7cf.sql` are empty materialized schemas from
`dashpay/platform` revisions `67d4ef3f6340a1e983229b6870ef60cf7573602a`
and `e3cd7cf5a34dd69b59d633e532a1ac570870b03f` respectively. Their SQL was
rendered from `packages/rs-platform-wallet-storage/migrations/V*.rs` using
the matching revision's enum label constants, then applied in version order
to an empty SQLite database. The new schema also includes the V011 Rust
conversion hook's removal of the legacy `account_address_pools` and
`core_derived_addresses` tables. SQL alone does not reproduce that schema;
the integration tests compare it with the actual public migration runner.
The resulting `sqlite_master` definitions are
stored in table/index/trigger order. Refinery history uses its SipHasher13
hash over migration name, i32 version and rendered SQL. Timestamps and the
store-generation token are deterministic fixture values.

The production bridge compares the source history and schema in
`67d4ef3.sql`, and the intermediate guard `0c08eb6.sql` described below,
before translating data. No code reads `e3cd7cf.sql` any more: it is kept
as the reference for the V001–V018 migration names and checksums. The bridge
copies into that intermediate schema, then runs the current public
`SqlitePersister::open` on the populated stage before publishing it. The final
schema and migration checksums must match an independently opened empty
persister. Updating either historical schema requires reviewing and
regenerating its guard.
The intermediate guard is `0c08eb6.sql`, the schema of an empty database
opened by `SqlitePersister::open` at Platform revision
`0c08eb6c9f2f3ac313f86c88ff0925ee7bad55cc`. The V001–V018 migration names and
checksums match `e3cd7cf.sql`. V019 adds only the nullable `asset_locks.operation_id`
column, its unique index and immutability trigger (checksum `1537039181339941563`).
V020 adds only the nullable `asset_locks.shielded_recipient` column and its
immutability trigger (checksum `13071883127280964199`). The previous guard,
`033ecae.sql` (V001–V019), is this file without the V020 changes. The snapshot
contains no domain rows; history timestamps and the store-generation token are
normalized fixture values, and its schema objects and migration history equal
those of a V020 database the real runner creates. The bridge copies existing
asset-lock blobs unchanged and leaves both new columns NULL. A different
migration occupying version 19 is not an accepted source history.
Platform revision `a0d7b6ffec6f91e3818530b83d75adcd62480ebe` changes only doc
comments in `V007__utxo_sweep_winner_height.rs`; every migration's SQL and
checksum, and so the snapshot, are unchanged.
Revisions `132ae31a1356d035cd8326f27c1109685090a71d` (5.0.0-rc.8),
`063589bf0adbe1a8823a943cee95385b423b0e3f`,
`575d961fe0c54bd7e01a3cf3823494e58c6c4fdb`,
`0d91c133f03d4a9350b964586efa68b6e2f04791` and
`6323a430c731a14bbec1d7a18466ca28bb083316` leave the migrations untouched.


The current Platform pin is `f5195d311cf1dd5f6d30a423aecfd09f2462d9df`
on `v5.2-dev` (merge of Platform #5948). It adds V021
`core_transaction_accounting`, including a Rust backfill that indexes historical
inputs and repairs wallet-level transaction amounts while preserving original
records. V001–V020 are unchanged. Keep the intermediate guard at V020 so V021
runs **after** copying the historical rows; an empty V021 schema would skip the
backfill. The bridge publishes the resulting tables, indexes, history and data
atomically. Unlike the earlier accounting branch, this migration does not reset
Core synchronization or discard spend state.

Regenerate the migration-source fingerprint with
`scripts/migration-fixtures/check-pin-ancestry.sh --update`, then run the script
without arguments. The public-runner compatibility tests verify the final
persisted schema and accounting backfill; the V020 SQL guard remains unchanged.

`public-rows.sql` contains only public synthetic rows from that old revision's
`packages/rs-platform-wallet-storage/tests/fixtures/populated_v001.db`, after
applying its V002–V014 SQL. It selects the registered A1 wallet and its rows;
the source fixture's separate empty B2 wallet has no account registration.
The rows include a public account key, an identity, contact, transaction,
UTXO and sync state. No seeds, private keys or real profiles are included.

The public fixture's transaction uses a placeholder TXID that V021 now rejects.
Tests of successful public-runner upgrades materialize a matching transaction
body, TXID and UTXO outpoint at runtime. A separate refusal test keeps the
original bytes and verifies that a failed backfill leaves the source unchanged.
