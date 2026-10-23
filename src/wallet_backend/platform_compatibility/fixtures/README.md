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

The production bridge recognizes only the exact `67d4ef3` history and schema.
It reconstructs the `e3cd7cf` V018 intermediate schema from the fixture,
copies and verifies the recognized legacy rows, then opens that populated
staging database with the current public `SqlitePersister::open`. This runs
upstream migrations on the data rather than copying old rows into an already
migrated empty database. The final schema and history must match an independent
fresh current persister before publication; both fixtures remain historical
recognition guards and must not be replaced with the latest schema.

At Platform `80c680affc6d122e64046bd53d49a0d4e7be4bd0`, V019 adds nullable
asset-lock operation IDs and V020 (`core_transaction_accounting`) rebuilds Core
history: it drops mined transactions, UTXOs and InstantLocks that an SPV rescan
redelivers, keeps unmined transactions and spent coins whose spender is not known
to be mined (stripping block details from the mined transactions funding them),
and rewinds each filter checkpoint to at most birth height minus one (minimum
zero). Loading may then drop pending spends whose authorization fails. The bridge
recognizes the migration by name, checks the checkpoint rewind, accepts any Core
rows that remain unchanged from the original (only stripped block details on mined
transactions), and requires every non-Core row to be preserved. The original
backup retains the pre-rescan Core data; the app rebuilds confirmed Core history
through SPV. A different V019 migration history, such as the unmerged `c0425f7`
accounting schema, is not recognized by this bridge.

`public-rows.sql` contains only public synthetic rows from that old revision's
`packages/rs-platform-wallet-storage/tests/fixtures/populated_v001.db`, after
applying its V002–V014 SQL. It selects the registered A1 wallet and its rows;
the source fixture's separate empty B2 wallet has no account registration.
The rows include a public account key, an identity, contact, transaction,
UTXO and sync state. No seeds, private keys or real profiles are included.
