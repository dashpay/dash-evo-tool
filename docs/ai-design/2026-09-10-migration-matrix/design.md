# Migration matrix: proving DET data survives an upgrade

A CI matrix that upgrades a data directory written by a **released** DET build
with the current build and asserts nothing was lost. The required baseline
today is `v0.9.3 → current`; from the next release forward, every new release
adds the pair `previous → next` on its own.

Companion to `docs/user-stories.md` WAL-033, to the capture recipe in
`docs/gui-testing/scenarios/migration-fixture-capture.md`, and to the fixture
contract in `tests/migration-fixtures/`. Complementary to — not a replacement
for — the compatibility bridge documented in
`docs/ai-design/2026-09-10-platform-pin/upgrade-notes.md`.

## 1. Context

DET's local state is the user's wallet: seeds, derived addresses, identities
and their keys, aliases, settings. Every release moves that state through
several independent migration mechanisms, and no test today runs a *real*
released binary's on-disk output through a *real* current binary. The nearest
coverage is unit-level and synthetic on both ends.

Two additional facts shaped this design:

- **`platform-wallet-storage` was pinned to a commit outside `v4.2-dev`.** The
  two lineages materialize genuinely different schemas — not merely different
  migration numbering — so rewriting checksums upstream would not have fixed
  it. This is already solved; see §3.
- **Only `v0.9.3` is a required baseline today.** The obligation "every new
  version gets tested" starts with the next release. `v1.0.0-weekly.20260908`
  is deliberately out of scope.

Standing assumption for everything below: **only migrations that work
correctly and unattended are supported.** A migration that cannot complete
safely is a defect to fix, not a case to document a manual recovery procedure
around. A harness assertion therefore fails; it never falls back.

## 2. The three migration systems

| # | System | Where | Notes |
|---|---|---|---|
| 1 | `data.db` schema ladder | `src/database/initialization.rs` | Legacy DET SQLite; `DEFAULT_DB_VERSION` is 11 at v0.9.3. Since the platform-wallet rewrite (#860) it only builds a fresh file: every boot path opens an existing `data.db` read-only |
| 2 | The "unwire" drain | `src/backend_task/migration/`, gated by `MIN_DIRECT_MIGRATION_VERSION` (11) / `MAX_DIRECT_MIGRATION_VERSION` (40) in `src/model/data_migration.rs` | One-shot move of legacy `data.db` rows into the `platform-wallet-storage` k/v store; idempotent through sentinels |
| 3 | `refinery` migrations inside `platform-wallet-storage` | upstream, applied to both `det-app.sqlite` and `det-<network>.sqlite` | The layer the lineage divergence hit |

A fixture drives all three in the order a user meets them, which is the point:
each system is individually tested, their composition is not.

## 3. PR #981's bridge, and the gap it names

[PR #981](https://github.com/dashpay/dash-evo-tool/pull/981) repins the four
Platform dependencies onto `v4.2-dev` and adds a DET-side compatibility bridge,
`src/wallet_backend/platform_compatibility/`. Its behaviour:

- Recognises the exact old migration history *and* materialized schema before
  translating anything.
- Keeps a backup beside the original
  (`<original-name>.platform-67d4ef3-backup-*.sqlite`), validates the converted
  data with the new reader, and rebuilds the original in one transaction.
- **Stops the upgrade** on unknown history, unexpected schema, or unreadable
  data, rather than guessing a conversion.
- Copies DET blobs and shared wallet columns byte for byte, so it never has to
  understand a bincode payload's internals.
- Covers both `det-app.sqlite` and the per-network database with the same
  mechanism.

Its own stated limits are exactly this design's mandate: *"Validation uses
synthetic upstream fixtures, not a user's real profile"* and *"Network-dependent
backend E2E and GUI tests were not run."* The matrix supplies real captured
profiles and a real process boot; it does not re-test what the bridge's unit
tests already cover.

One consequence to accept: the bridge is specific to one recognised old history
(`67d4ef3`). A future divergence needs another dedicated bridge — that is
inherent to "stop rather than guess", and the guard in D0.2 exists to catch the
next one early rather than to make bridges unnecessary.

**v0.9.3 is structurally immune to all of this.** That build predates the
`platform-wallet` rewrite: it has no `det-<network>.sqlite` at all, so its
upgrade always runs path (2) and creates the persister from scratch. That is
why the single required baseline is also the safest one.

## 4. Fixtures

A fixture is a data directory captured from a released binary, packed, and
replayed later. The contract lives in `tests/migration-fixtures/README.md`; the
summary:

- **Capture with the old binary, never regenerate with the current one.**
  Regenerating would test nothing.
- **v0.9.3 needs the GUI.** No det-cli, no MCP, no SPV at that tag; isolation is
  via `XDG_CONFIG_HOME` (no `DASH_EVO_DATA_DIR` yet) and the era's own
  `.env.example`. Its identity is registered beforehand by the current build
  and only loaded by ID during capture, because without SPV that binary can see
  funds only through a local Dash Core node over RPC/ZMQ.
- **From the next release forward, capture headless** through det-cli; the GUI
  stays required only for a password-protected wallet import (no password
  parameter on `core_wallet_import`) and for reacting to a testnet reset.
- **Storage: GitHub Actions build artifacts**, not Releases. Artifacts expire,
  so a baseline needs explicit long retention and/or a refresh job;
  `manifest.json` in the repo is the only durable index, and a missing or
  expired artifact fails the job loudly.
- **The fixture wallet is public by design**: dedicated, testnet-only,
  dust-only, never the backend-E2E framework wallet, with a fixed literal
  password for the protected wallet. The recovery phrase still never enters the
  repository.

## 5. Verification harness

The harness runs a **real compiled binary as a subprocess** against the
extracted fixture directory — det-cli headless where the version supports it,
the GUI under Xvfb for v0.9.3. An in-process `egui_kittest` boot is explicitly
rejected: it does not reproduce the production startup path, which is a large
part of what migration correctness means here. The environment is fully reset
between fixtures.

Assertions are **conditional, never unconditional** — "the migration ran" and
"the data was already current" are different passes, and a test that cannot
tell them apart contradicts existing coverage:

| Property | Assertion |
|---|---|
| Boot | No panic, no `WalletDataIncompatible`. A migration that cannot complete is a failure, never a fallback |
| `data.db` | Never written: every boot path opens an existing file read-only (#860), so it stays byte-identical whatever its version. A fixture without one gets a fresh file at `DEFAULT_DB_VERSION` |
| Sentinels | An already-completed import keeps its original marker |
| Wallets | Aliases, addresses and wallet count preserved |
| Protected wallet | Right password opens it, wrong password is rejected, at-rest protection survived |
| Identity | Same ID, alias, type and wallet link; DPNS name still shown without a manual re-fetch |
| Idempotence | A second boot changes nothing further |

Read-only MCP tools (`identity_list`, `app_storage_status`) provide the
post-upgrade view without a GUI.

## 6. Phases

- **Phase 0 — validate the bridge on real data, and prevent a repeat.**
  (D0.1) Feed a captured fixture to `platform_compatibility` as an extra case,
  closing the "synthetic fixtures only" gap. (D0.2) A CI guard that flags a new
  `platform-wallet-storage` pin whose migration history diverges from the
  previous pin — ancestry *and* a fingerprint of the rendered migrations, since
  ancestry alone misses an SQL change under an unchanged version number.
  (D0.3) Document `platform_compatibility` as the established pattern for the
  next divergence. Phase 0 blocks nothing: the required baseline is v0.9.3,
  which is immune by construction.
- **Phase 1 — capture.** v0.9.3 first (GUI); tooling and manifest under
  `tests/migration-fixtures/`. Optionally capture one of the diverged weeklies
  (`20260818` / `20260825`) as real regression material for D0.1.
- **Phase 2 — harness.** As in §5.
- **Phase 3 — CI (`migration-matrix.yml`).** Gate matched to the release
  candidate's exact commit; matrix-pruning policy sized after the first real
  timing measurement; explicit failure on missing or expired inputs.
- **Phase 4 — automation, from the next release forward.** Triggered by
  chaining from the weekly publish run itself, not the `release: published`
  event (which a `GITHUB_TOKEN` normally will not fan out from). Headless
  capture → artifact upload → manifest PR. Steps that cannot be automated
  (re-registration after a testnet reset, the protected wallet, milestone
  promotion) stay documented as manual.
- **Phase 5 — documentation.** This directory, the capture scenario,
  `docs/user-stories.md` WAL-033, the `docs/kv-keys.md` filename correction
  (`platform-wallet.sqlite` → `det-<network>.sqlite`), and `CHANGELOG.md`.

## 7. Sequencing

1. PR #981 — independent; merges on its own track.
2. **PR-A**: fixture tooling, storage/retention, and the v0.9.3 capture.
   Independent of #981, may run in parallel.
3. **PR-B**: real-binary harness, read-only MCP tools, `migration-matrix.yml`,
   and the weekly-build gate. Requires only `v0.9.3 → current`.
4. **PR-C** (after #981 merges): D0.1, D0.2, D0.3.
5. **PR-D**: capture automation for future releases.

After the first release cut following PR-B, that release becomes baseline #2
and the `N → N+1` chain extends by itself.

## 8. Accepted risks

- The bridge covers exactly one recognised old history; a future divergence
  needs another one. D0.2 detects, it does not prevent.
- Artifact retention is a configuration decision, not a property — unlike a
  Release, an artifact disappears if nobody keeps it alive.
- Capture jobs need runner egress to testnet P2P/DAPI, with the same
  flakiness class that keeps `backend-e2e` out of `tests.yml`.
- Phase 4's automatic PRs depend on a GitHub App token; the fallback is a human
  opening the PR after a notification.
- Linux-only in v1. Windows and macOS are an open decision, not an oversight.
