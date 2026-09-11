# Compatibility bridges for a diverged `platform-wallet-storage` pin

Reusable pattern, not a point-in-time record. For the specific incident that
established it, see `docs/ai-design/2026-09-10-platform-pin/upgrade-notes.md`
(PR #981) and `docs/ai-design/2026-09-10-migration-matrix/design.md` §3.
Reference implementation: `src/wallet_backend/platform_compatibility/`.

## 1. When this pattern applies

`scripts/migration-fixtures/check-pin-ancestry.sh` fails one of its two
checks on a `platform-wallet` / `platform-wallet-storage` repin:

- **Ancestry** — the new pin is not an ancestor of the target release branch
  (taken from a side branch, like `67d4ef3`).
- **Fingerprint** — the pin is on the target branch, but the hash over its
  rendered migration sources changed without the pin moving. Upstream edited
  an already-released migration's SQL in place.

Either failure means migration history has forked or been rewritten:
databases already written by the old pin and databases written by the new
code no longer share one linear `refinery` history. `SqlitePersister::open`
on an old database then fails with `WalletStorageError::Migration(_)`
instead of applying new migrations on top. That failure is the trigger to
build a new bridge module — do not silence the guard or force a checksum
match instead.

## 2. Why reconciling checksums/numbering is not a fix

The intuitive fix — renumber or re-hash the old migrations to line up with
the new history — only works if the *materialized schemas* are actually the
same. They may not be. In the incident that motivated this pattern (#981):

- The old pin's `V001` migration checksum differed from the new pin's `V001`.
- The old pin's `V003` was already a "unified" schema that the new pin only
  reaches at `V009`.

That is real schema divergence, not just a version-numbering mismatch.
Rewriting checksums would make refinery *believe* the histories match while
leaving column sets, tables, and constraints genuinely different — silent
data corruption on the next write, not a fixed migration. Before writing a
bridge, confirm by diffing rendered schemas (`sqlite_master`, as captured in
`engine.rs`'s `OLD_SCHEMA`/`TARGET_SCHEMA` fixtures) — never assume checksum
repair is sufficient without checking.

## 3. The bridge mechanism (design level)

A bridge (`engine.rs`) is deliberately narrow and fails closed at every step:

1. **Recognize the exact old shape.** Compare the source database's
   `sqlite_master` objects and `refinery_schema_history` byte-for-byte
   against a known old schema/history baked in as a fixture. Any deviation
   (including `PRAGMA application_id`) aborts with `Unrecognized` — no
   partial or fuzzy matching.
2. **Back up the original file first**, verified with `PRAGMA
   integrity_check` before anything is touched, kept beside the original
   regardless of outcome.
3. **Convert and validate against the new reader**, not just the new schema.
   Data is copied column-by-column into a fresh database built by the
   current `SqlitePersister::open`, verified row-for-row against the source
   (`equal_rows`), then opened and loaded through the real public API
   (`SqlitePersister::load()` and friends) before it is trusted.
4. **Rebuild the original file atomically, in one SQLite transaction.** A
   crash or power loss during rebuild leaves either the complete old schema
   or the complete new schema — never a half-migrated file.
5. **Unknown or unreadable data stops the upgrade.** Every ambiguous case
   (unexpected added/removed columns, non-empty tables that should be new,
   a malformed identity roster) returns a typed `UpgradeError` variant and
   leaves the original file untouched, rather than guessing at a conversion.
   `open()` in `mod.rs` falls back to surfacing the *original* open error
   when the bridge declines to act — it never masks a real incompatibility.

## 4. Building the test fixture for a new bridge

Synthetic SQL fixtures (empty schemas rendered from the old and new
migration sources, per `fixtures/README.md`) are the starting point, but
PR #981 explicitly flagged them as insufficient alone: they only prove the
bridge handles an *empty* database of the recognized shape, not the byte
encodings, blob payloads, and edge-case rows a real released binary
actually produced.

Prefer a real captured profile from an affected release, per D0.1 in
`docs/ai-design/2026-09-10-migration-matrix/design.md`:

- Capture with the affected binary itself — `tests/migration-fixtures/`
  documents the capture contract; use det-cli headless where the affected
  release supports it, GUI capture otherwise.
- Treat the captured fixture as an additional test case fed through the new
  bridge, alongside (not instead of) the synthetic empty-schema tests.
- Never regenerate a fixture with the current build — that tests nothing
  about the old shape.

## 5. Non-goals and limits

- **One bridge per divergence, not a general translator.** This is a
  per-incident bridge, not an N-way schema migration engine. It recognizes
  one exact old history and one exact new target; it does not attempt to
  cover multiple old lineages or guess at partial matches.
- **Do not extend an existing bridge module to "handle more cases."** A new
  divergence gets its own dedicated module (own fixtures, own recognition
  check, own `UpgradeError` review) rather than branching inside
  `platform_compatibility/` to recognize a second old shape. Keeping
  recognition exact and single-purpose is what makes "stop rather than
  guess" trustworthy.
- **The CI guard detects divergence; it does not prevent or fix it.**
  `check-pin-ancestry.sh` (D0.2) is the early-warning signal that a new
  bridge is needed. Passing the guard again after a repin does not retire
  an already-built bridge for data still on disk from the old lineage.
