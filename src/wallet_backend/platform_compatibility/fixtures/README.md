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
`67d4ef3.sql`, and the destination guard `033ecae.sql` described below,
before translating data. No code reads `e3cd7cf.sql` any more: it is kept
as the reference for the V001–V018 migration names and checksums. The
destination itself is created by the current public `SqlitePersister::open`,
not by the fixture SQL. Updating either schema revision requires reviewing
and regenerating these guards.
The current destination guard is `033ecae.sql`, captured from an empty database
opened by `SqlitePersister::open` at Platform revision
`033ecaee2f3badc8e04ff8be664062f33516e3e4`. The V001–V018 migration names and
checksums match `e3cd7cf.sql`. V019 adds only the nullable `asset_locks.operation_id`
column, its unique index and immutability trigger (checksum `1537039181339941563`).
The snapshot contains no domain rows; history timestamps are normalized. The
bridge copies existing asset-lock blobs unchanged and leaves the new column NULL.
A different migration occupying version 19 is not an accepted source history.
Platform revision `a0d7b6ffec6f91e3818530b83d75adcd62480ebe` changes only doc
comments in `V007__utxo_sweep_winner_height.rs`; every migration's SQL and
checksum, and so the snapshot, are unchanged.
Revisions `132ae31a1356d035cd8326f27c1109685090a71d` (5.0.0-rc.8),
`063589bf0adbe1a8823a943cee95385b423b0e3f`,
`575d961fe0c54bd7e01a3cf3823494e58c6c4fdb` and
`0d91c133f03d4a9350b964586efa68b6e2f04791` leave the migrations untouched.


`public-rows.sql` contains only public synthetic rows from that old revision's
`packages/rs-platform-wallet-storage/tests/fixtures/populated_v001.db`, after
applying its V002–V014 SQL. It selects the registered A1 wallet and its rows;
the source fixture's separate empty B2 wallet has no account registration.
The rows include a public account key, an identity, contact, transaction,
UTXO and sync state. No seeds, private keys or real profiles are included.
