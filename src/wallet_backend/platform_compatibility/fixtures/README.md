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

The production bridge compares these exact histories and materialized
schemas before translating data. The destination itself is created by the
current public `SqlitePersister::open`, not by the fixture SQL. Updating
either schema revision requires reviewing and regenerating these guards.
The selected dependency pin `63cf57f40d0000bf3b2b26026c8fa1c71162852d`
has identical migration code and schema to `e3cd7cf`, so it uses the same
destination guard.

`public-rows.sql` contains only public synthetic rows from that old revision's
`packages/rs-platform-wallet-storage/tests/fixtures/populated_v001.db`, after
applying its V002–V014 SQL. It selects the registered A1 wallet and its rows;
the source fixture's separate empty B2 wallet has no account registration.
The rows include a public account key, an identity, contact, transaction,
UTXO and sync state. No seeds, private keys or real profiles are included.
