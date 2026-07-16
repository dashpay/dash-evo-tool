# PR860 QA Delta Campaign — Summary Report

Status: **COMPLETE** (2026-07-16, resumed after a mid-campaign agent kill — see progress.md's
session log for the resume/verification detail).

## Tally

- 1 new story tested: **WAL-032** — PASS.
- 34 baseline-FAIL stories re-tested — the large majority now PASS (fixed since the
  2026-07-14/15 baseline). Remaining confirmed-still-FAIL: **UX-003** (global switcher missing on
  4 screens — since fixed separately by today's `316dae26`/`af1621d9` commits, confirmed
  reproducing at the specific commit this delta pass started from, prior to that fix landing),
  **IDN-013a** (password-protect identity key — `KeyInfoScreen` has no live UI trigger),
  **IDN-009** (refresh identity data — no visible/logged effect), **ALK-002** (Asset Locks list
  never surfaces a created lock).
- PASS spot-checks: DPN-002, DPY-001, MN-002, DEV-005 (all category-representative, see
  progress.md), plus incidental IDN/WAL/SND/IDH coverage throughout.
- New findings this campaign: sticky token-creation banner (fixed), background
  identity-sync wallet-id-mismatch (triaged, upstream, memcan TODO filed).

## FAIL list detail

See `progress.md` for full per-story evidence. The two headline still-open items:

1. **ALK-002 / ALK-003 / WAL-018** (Asset-Locks-list UI/cache bug) — unchanged since baseline.
   Root cause not re-investigated this pass (baseline already confirmed via direct SQLite check
   that the underlying data persists correctly — this is a UI/cache display bug, not a
   persistence or coin-selection bug). Reconfirmed live via two independent methods this session.
2. **IDN-013a** (password-protect an identity's signing keys) — `KeyInfoScreen`'s "Add password
   protection…" flow, described in this repo's own secret-storage design doc, has zero live UI
   entry point; the one plausible trigger (clicking a row in the new "Manage keys" table) has no
   click handler.

## Fixes landed this campaign (resumed session)

- **Sticky "Creating token…" banner** — `src/ui/tokens/tokens_screen/mod.rs`, the
  `BackendTaskSuccessResult::RegisteredTokenContract` arm now calls
  `operation_banner.take_and_clear()`. New regression test
  `registered_token_contract_clears_creation_banner`, proven red before / green after. Dispatched
  via Codex Sol (gpt-5.6-sol, high effort), independently verified (targeted `cargo test` +
  `cargo clippy --all-features --all-targets -- -D warnings`, both clean). Commit `d59c004f` on
  branch `fix-token-creator-banner`, merged to `docs/platform-wallet-migration-design` as
  `0c8d4834`. **Not pushed** (per standing instruction — never push without explicit permission).

## Triaged, not fixed

- **Background wallet-id-mismatch in `platform_wallet::manager::identity_sync`** — confirmed
  upstream (git-dependency `platform-wallet`, pinned rev `d18020f5`), not fixable in this repo.
  Root-caused precisely: `identity_sync.rs::apply_fresh_balances` always flushes token-balance
  changesets under the all-zero `WalletId` sentinel; `rs-platform-wallet-storage`'s
  `assert_identities_belong_to_wallet` guard only tolerates that sentinel for identities whose
  own `wallet_id` column is NULL — any identity with a real wallet association trips it on every
  sync pass. Caught and logged only (no crash/UI impact); the on-disk `token_balances` table
  silently doesn't get updated for affected identities, a real but low-visibility correctness
  gap. Filed as memcan TODO `92e7dcad-bb89-4e27-8041-5d6add39bd3d` (project `dash-evo-tool`).
  Queued as a user decision — file upstream against `dashpay/platform`, or accept as known.

## Environment / methodology notes

- Reused the already-running QA app instance and its funded/synced fixture data dir
  (`/data/tmp/det-pr860-wal032-fixture-data`) rather than relaunching, per the resume guidance —
  binary sha256 reverified (`c91f7ce151d248762c50b3601b0ba2c7d796c3ed28054a12c1d007f64d8bf439`,
  unchanged).
- `fix-token-creator-banner` worktree at `/data/git-worktrees/
  home-ubuntu-git-dash-evo-tool-2-fix-token-creator-banner` verified clean/untouched before
  dispatch, per handoff.
- NET-011/NET-019 (destructive) deliberately not re-run, same precedent as prior campaigns.
- Final plain-language product report published to
  `/data/artifacts/dash-evo-tool/2026-07-16/pr860-qa-final-report.md`.
