# Platform development pin upgrade

Reviewed on 2026-09-10. The four Platform dependencies move together from
`67d4ef3f6340a1e983229b6870ef60cf7573602a` (`4.2.0-dev.2`, PR #3968)
to `63cf57f40d0000bf3b2b26026c8fa1c71162852d` (`4.2.0-dev.8`, `v4.2-dev`).
The transitive rust-dashcore revision moves from `3d13d9838c80fb5e67cf1f62cf5f3f4477bd5b9a`
to `93260bf39bac5d9d09e89bfb45e9ea3ff7fdcbcd`.

The selected revision includes [#4649](https://github.com/dashpay/platform/pull/4649),
which serializes pending contact-crypto persistence with identity removal.
Its changes relative to `e3cd7cf` leave public APIs, dependency manifests,
migration history and the database schema unchanged; the existing `e3cd7cf.sql`
schema guard therefore also applies to `63cf57f`.

## What survived the PR split

| Area | Result at the new pin |
| --- | --- |
| SQLite persistence and seedless rehydration | Included through [#3968](https://github.com/dashpay/platform/pull/3968), with a different migration lineage. |
| Typed persistence errors | Included through [#4586](https://github.com/dashpay/platform/pull/4586). Store retry eligibility is now an explicit backend contract. |
| Guarded editable secrets and password envelopes | Retained, including `SecretString::replace_range`; Argon2 working memory wiping improves. |
| Large operating-system memory pages | Secret storage now refuses page sizes above 16 KiB. Hosts using 64 KiB pages are not covered by the local Linux checks. |
| Secret deserialization/schema features | `secret-serde` becomes `serde`; schemas are included with `secrets`. |
| Provider-key reconstruction | Retained; the proposed FFI deduplication is not required by DET. |
| Contact-account scan coverage | [#4587](https://github.com/dashpay/platform/pull/4587) remains open. Upstream inserts contact accounts without invalidating `account_generation` and prior filter-scan coverage. DET does invalidate these for new accounts created at bootstrap/unlock; it cannot retroactively cover an account already inserted by recurring upstream sync. End-to-end contact-payment consequences still need network testing. |
| FFI asset-lock proof size gate | [#4585](https://github.com/dashpay/platform/pull/4585) remains open and the old gate is absent. DET consumes Rust wallet APIs, not this FFI entry point. |

## Compatibility work

- Document queries explicitly use an empty sub-query list, retaining their
  existing single-query behavior.
- Persistence failures retain their typed source and use the new store-failure
  classification; exhaustive wallet error handling includes new variants.
- Swept transactions are removed from DET's displayed transaction history while
  retaining unrelated transactions and other wallets' history.
- The existing single-UTXO Max-send regression now passes; its test is enabled
  in the ordinary test suite (DET #909 / rust-dashcore #911).
- [#4496](https://github.com/dashpay/platform/pull/4496) replaces identity
  tombstones with hard deletion and metadata cascades. Ownership reconciliation
  now preserves a still-listed identity until its wallet takes ownership,
  including when that promotion must wait for a later reconciliation.
- Existing PR-pin databases use a compatibility bridge: their V001 checksum is
  different and their V003 unified schema precedes the new branch's V009.
  Rewriting migration checksums alone is invalid because the materialized schemas
  also differ.

## Existing data

Both `det-app.sqlite` and the per-network wallet database use the bridge. It
recognizes the exact old migration history and materialized schema, retains a
SQLite backup beside the original (`<original-name>.platform-67d4ef3-backup-*.sqlite`), and
validates the converted data with the new storage reader before rebuilding the
original in one transaction. Unknown history, unexpected schema changes, or
unreadable data stop the upgrade rather than guessing at a conversion.

Shared wallet columns and opaque DET metadata are copied and compared byte for byte.
Version-domain aliases retain the largest sequence on a collision. A tombstoned
identity still present in DET's active roster remains live; genuinely retired
typed identity rows remain in the backup, while their opaque local metadata is
preserved. An ambiguous or malformed saved identity roster fails closed.
The encrypted seed vault is not migrated or rewritten.

Keep the retained backups. Downgrading does not automatically reverse the
database conversion; recovery requires the corresponding backup and the prior
application version. Validation uses synthetic upstream fixtures, not a user's
real profile.

## Validation and limits

- `cargo fmt --all`: completed.
- `cargo clippy --locked --all-features --all-targets -- -D warnings`: passed with the
  exact CI flags.
- `cargo test --locked --lib --all-features`: 2520 passed, none failed or ignored.
  This includes old app-preference and populated wallet upgrades, persisted
  balances/identities, backup preservation, interrupted-process rollback,
  WAL snapshots, writer exclusion, unknown-schema rejection and repeated open.
- Focused regressions reproduced and fixed stale swept-transaction history and
  identity metadata deletion during ownership transfer. The single-UTXO Max-send
  regression is included in the normal passing suite.
- Network-dependent backend E2E and GUI tests were not run. The checks used Linux
  and synthetic data; they do not establish contact-payment behavior on a live
  network or support for hosts with larger operating-system memory pages.

`cargo audit` reports the same five advisories against both lockfiles:
RUSTSEC-2026-0204 (`crossbeam-epoch`), RUSTSEC-2026-0258 (`h2`),
RUSTSEC-2026-0194 and RUSTSEC-2026-0195 (`quick-xml`), and RUSTSEC-2026-0257
(`webbrowser`). This upgrade introduces none of those five, but the audit is
not clean. Unmaintained/unsound/yanked warnings also remain. This is a bounded
upgrade review, not a whole-Platform security audit.

<sub>Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
