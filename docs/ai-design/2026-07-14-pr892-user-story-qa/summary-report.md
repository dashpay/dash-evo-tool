# PR892 User-Story QA — Summary Report

**Status: IN PROGRESS.** This report is updated as the campaign proceeds; see `progress.md`
for the live per-story checklist.

Build under test: PR892 (`fix(wallets): show transaction history that predates the current
session`) @ commit `57195d54`, built from worktree
`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build`.
Binary: `/data/target/debug/dash-evo-tool`. Data dir (isolated):
`/data/tmp/det-qa-pr892-data`. Network: Testnet.

## PR892 regression fix — CONFIRMED FIXED

This is the single most important check in the campaign. Full repro in `scenarios/WAL.md`
under WAL-016.

**Test:** funded a wallet with 3 real testnet transactions, confirmed they rendered in the
live in-app Transaction History, then **fully quit the app** (`kill -TERM`, clean process
exit) and **cold-boot relaunched** the identical binary against the identical data
directory — not just navigating away and back in-app.

**Result:** all 3 transactions rendered correctly after the cold boot, with the same
amounts, timestamps, txids, and ChainLock heights as before the restart. Balance was also
correctly restored (3 DASH) immediately on startup, even before SPV re-sync completed.

**Conclusion: PR892's fix works as intended.** Persisted `core_transactions` rows are
correctly hydrated into the in-memory snapshot store at wallet load. The original bug
(transaction history rendering empty after restart despite correct balance) does not
reproduce on this build.

Evidence: `scenarios/screenshots/WAL-016-1-tx-history-live-before-restart.png`,
`scenarios/screenshots/WAL-016-2-tx-history-after-cold-boot-PASS.png`.

## Progress so far

Categories fully or partially covered: WAL (spot-checked: 001, 004, 010, 011, 016, 023, 024
— PASS; full sweep of remaining WAL stories still pending), SND (001, 003 — 001 PASS/
003 FAIL), NET (001 — PASS). All other categories not yet started.

See `progress.md` for the authoritative live checklist.

## Findings so far

### FAIL: SND-003 — Receive Dash with QR code
**Severity: Medium.** Clicking "Receive" on the Wallet screen (Expert view) does nothing —
no modal, no QR code, no navigation, no log entry. Reproduced 3x consistently. Workaround
exists (the address table exposes the receive address as copyable text, and it was used
successfully to fund this campaign's wallet), so this is not a total feature outage, but the
documented QR-code receive flow does not work at all as tested. Not yet verified whether
Default view behaves differently. Full repro: `scenarios/SND.md`.

## UX observations (non-blocking)

- **Sidebar navigation overflow**: in Expert view, at the app's default 800×600 window size,
  the sidebar (Identities/Masternodes/Contracts/Tokens/Wallets/Tools/Settings/Expert-toggle/
  Dash-logo) does not fit vertically — "Settings" is pushed below the fold and only
  reachable by scrolling the sidebar itself. Easy to miss on first use. See `scenarios/NET.md`.
- **Dash logo external link**: the Dash logo at the bottom of the sidebar opens a full
  external browser window to `dash.org`, positioned directly above/near "Settings" — easy to
  click by accident. See `scenarios/NET.md`.
- **Wallets are strictly per-network**: a wallet created while on Mainnet is invisible when
  switched to Testnet (and vice versa) — expected/correct behavior, but the "No wallets yet"
  empty state after a network switch could read as data loss to a user who forgot they
  created a wallet on the other network. Not a defect, just worth flagging as a potential
  first-time-user confusion point.

## Discrepancy vs. the campaign brief

The task brief referenced 152 stories across categories WAL/SND/ALK/IDN/DPN/DPY/TOK/DOC/
DEV/NET/MCP/UX/IDH/MN. The actual `docs/user-stories.md` at the PR892 base (`v1.0-dev`)
contains 123 stories (112 `[Implemented]`, 11 `[Gap]`) across only WAL/SND/ALK/IDN/DPN/DPY/
TOK/DOC/DEV/NET/MCP — no UX, IDH, or MN category exists in this document version.
Masternode/evonode aspects that would fall under "MN" are covered by IDN-003 (Load evonode/
masternode identity) and DEV-006 (View masternode list diff), both in-scope and tracked
under their actual categories. Proceeding with the document as it actually exists rather
than the brief's stale story count.

## Next steps

Continue working through remaining WAL stories, then SND, ALK, IDN, DPN, DPY, TOK, DOC, DEV,
NET, MCP in that order per the campaign plan. NET-011/019/020 (destructive) deferred to the
end as instructed.
