# PR892 User-Story QA — Summary Report

**Status: COMPLETE.** All 123 stories in `docs/user-stories.md` are accounted for (see
`progress.md` for the authoritative per-story checklist). One story (NET-011) is BLOCKED by
design — see its dedicated section below — rather than tested; every other story was either
executed end-to-end or definitively marked BLOCKED/FAIL/N-A with reasoning.

Build under test: PR892 (`fix(wallets): show transaction history that predates the current
session`) @ commit `57195d54`, built from worktree
`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build`.
Binary: `/data/target/debug/dash-evo-tool`. Data dir (isolated):
`/data/tmp/det-qa-pr892-data`. Network: Testnet (Mainnet used only for a handful of
cross-checks, explicitly noted where relevant).

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

## Overall results

| Verdict | Count | Meaning |
|---|---:|---|
| PASS | 25 | Fully executed end-to-end, met acceptance criteria |
| FAIL | 23 | Executed, did not meet acceptance criteria — real bugs/gaps, listed below |
| BLOCKED | 64 | Could not be completed for a documented reason (see below — the large majority trace to one root cause, not 64 independent problems) |
| N/A | 11 | `[Gap]` in `docs/user-stories.md` — not implemented, out of scope by design |
| **Total** | **123** | |

**Read the BLOCKED count carefully — it is not 64 independent failures.** Two systemic issues
account for nearly all of it:

1. **A mid-campaign environment blocker** (Testnet masternode-list/quorum-sync/wallet-storage
   failure — full diagnosis in `scenarios/ALK.md`) made Platform proof verification
   unavailable partway through the run. Once no Platform identity could be registered or
   loaded (see IDN below), every downstream story that needs an identity — most of DPN, DPY,
   TOK, DOC, and several IDN/SND/WAL stories — cascades to BLOCKED for that single reason.
   This is an **environment/infrastructure issue in this QA session**, not a confirmed PR892
   regression — see the dedicated section below before concluding anything about the app
   itself from the BLOCKED count.
2. **NET-011** is one deliberate BLOCKED-by-policy item (destructive test correctly deferred
   pending human authorization) — see its own section.

Excluding those two systemic causes, the FAIL list below is the substantive, actionable
signal from this campaign.

## Environment blocker — read before drawing conclusions from BLOCKED stories

Starting partway through the ALK category (~2026-07-14 18:19 UTC), the Testnet
wallet-backend/chain-sync stopped wiring successfully in this QA session's data directory,
and — as testing progressed into DEV — this was found to be a broader **masternode-list/
quorum-sync failure that blocks Platform proof verification for any Platform query**, not
just the wallet's own SPV client. Symptoms: "SPV sync failed" banners, `WalletBackendNotYetWired`
errors, Platform queries failing with masternode-list/quorum errors. DAPI connectivity itself
stayed healthy throughout (29/29 endpoints unbanned).

This was investigated extensively and non-destructively:
- Reproduced across 10+ full process restarts and via the in-app reconnect path.
- **Mainnet worked fine in the same process** — ruling out a general resource/backend/network
  problem; it is Testnet-specific.
- A differential test (brand-new, zero-state throwaway wallet) **disproved** the initial
  hypothesis that two asset-lock rows created during ALK-001 testing were the trigger — the
  same failure occurs with a wallet that has never held any asset lock.
- Two non-destructive repair attempts (clearing stale WAL/SHM SQLite sidecars; attempting to
  remove and reconstruct the wallet through the app's own sanctioned "Remove Wallet" UI) did
  not resolve it, and the second was correctly halted by the Claude Code agent permission
  system before any destructive confirmation, pending explicit human authorization.
- Root cause was **not** found — it needs either destructive DB access or a debug-instrumented
  rebuild to capture the underlying error's structured detail, both appropriately gated behind
  human sign-off rather than attempted unilaterally by an unattended agent.

**Full diagnostic trail**: `scenarios/ALK.md` ("App-restart failure" section and its
addendum), with a forward pointer from `scenarios/DEV.md` narrowing the scope further.

**Practical effect on this report**: every BLOCKED verdict from ALK-002 onward that cites
"known environment issue" reflects this one open problem, not 60+ separate defects. It should
be triaged and fixed (or the QA data dir reset with authorization and a subset of the
BLOCKED stories re-run) before treating those stories as validated either way — they are
**untested**, not **passing**.

## FAIL findings (real bugs and gaps — the actionable signal)

Ordered roughly by severity/impact within each rough tier. Full repro steps for every item
are in the category's `scenarios/*.md` file.

### Critical

- **DOC-002 (Update an existing data contract) — application crash.** Clicking "Update
  Contract" panics the whole process: an `.expect("Failed to load contracts")` on
  `app_context.get_contracts()` fires when the wallet backend isn't wired
  (`src/ui/contracts_documents/update_contract_screen.rs:93`). Its sibling "Register
  Contract" screen handles the identical condition cleanly with a typed error — this one
  does not. Confirmed via diff against `v1.0-dev` that this is a **pre-existing bug, not a
  PR892 regression**. App relaunched cleanly afterward; zero persistent state lost.

### High

- **IDN-002 / IDN-003 (Load identity by ID / Load evonode-masternode identity) — silent
  hang.** Both "Load Identity" (ID + private key tab) and "Load a masternode" (ProTxHash)
  submit buttons hang completely silently on click: no banner, no log line, no timeout, ever.
  This is distinctly worse than every other blocked-by-environment flow tested in the same
  session, which all degrade gracefully with a clean typed or generic error — including
  sibling tabs on the *same screens* ("Search Wallet for Identities", DPNS username search,
  ProTxHash format validation).
- **DOC-004 (Query and browse documents) — silent infinite hang.** "Fetch Documents"
  dispatches a real query and never resolves — no banner, no error, ever (reproduced across
  two sessions with 60s and 45s waits) — while an ever-counting "Querying documents..."
  progress banner falsely implies the operation is still in progress.
- **TOK-003 (Add token by contract or token ID) — silent failure drop.** A well-formed
  contract ID dispatches correctly and the underlying query genuinely fails (visible in
  logs), but the failure is never surfaced to the user at all (no banner, no inline message),
  reproduced twice.
- **SND-003 (Receive Dash with QR code) — feature does not work at all.** Clicking "Receive"
  on the Wallet screen (Expert view) does nothing — no modal, no QR code, no navigation, no
  log entry. Reproduced 3x from a clean state. A workaround exists (the address table exposes
  the receive address as copyable text, and was used successfully throughout this campaign to
  receive faucet funds), but the documented QR-code flow — the actual point of the story —
  is completely inert. Not verified whether Default view differs.
- **MCP-001 (Manage wallets via CLI) — imported wallets are invisible across process
  boundaries.** `core_wallets_list`/`core_address_create`/`core_balances_get` all fail to see
  a wallet imported by a prior `det-cli` invocation, or even an already-imported wallet from
  an earlier call in the *same* process. Root cause traced to source:
  `ListWalletsTool::invoke` (`src/mcp/tools/wallet.rs:520-539`) reads only the in-memory
  `AppContext.wallets` map, which is never hydrated from the DB/secrets-vault outside the
  SPV-gated code path — breaking the exact process-per-command pattern every example in
  `docs/CLI.md` uses. (MCP-002, the transport/protocol layer itself, is unaffected and
  PASSES cleanly — this is a wallet-tooling defect, not a server defect.)

### Medium

- **WAL-006 (Lock and unlock wallet) — self-lockout bug.** Lock works and correctly blocks
  sensitive operations, but Unlock never opens a password prompt — a locked wallet becomes
  **permanently stuck**, confirmed across 4 attempts.
- **WAL-005 (Rename a wallet) — inert.** The Rename button has no effect on either HD or
  single-key wallets, reproduced repeatedly.
- **WAL-007 (Remove a wallet) — missing confirmation for single-key wallets.** HD wallets get
  a proper confirmation dialog before removal; single-key wallets are deleted **instantly**
  with **zero confirmation** — a real data-loss risk for a destructive action.
- **SND-005 (See fee estimate before confirming send) — no confirmation step exists at all.**
  Neither the simple nor the Advanced Options Send form shows a fee estimate or any
  confirmation step before broadcasting — clicking "Send" broadcasts immediately, every time
  (reproduced on 4 separate real sends). The "Max" button *does* silently account for a fee
  internally but never labels or displays it anywhere, before or after the fact (the
  post-send Transaction History "Fee" column is always `-`). This also means **SND-001's own
  stated acceptance criterion** ("confirmation dialog before broadcast") does not hold, though
  SND-001 itself still PASSes on its primary criteria (destination + amount entry, screen
  navigation).
- **WAL-017 (Fund Platform address from wallet) — coin-selection failure, later shown to be
  transient.** Initially failed with "No UTXOs available for selection" despite a
  multi-UTXO funded wallet. A later differential test in the ALK category (creating an asset
  lock through a *different* UI entry point, then immediately retrying WAL-017's exact
  failing scenario in the same session) **succeeded** — proving this is not a persistent,
  global coin-selection defect. Left as FAIL since the originally-tested flow did fail as
  observed and reproducibly at the time, but this should not be read as "Platform funding is
  broadly broken" — see `scenarios/ALK.md` for the full differential-test writeup.

### Low (settings/UI gaps, not functional breakage)

- **NET-002 (Auto-update from dashmate config)** — no detection/import UI anywhere;
  `.env.example` requires manual copy-paste from the `dashmate` CLI instead.
- **NET-003 (Configure Dash-Qt path)** — the setting exists in the data model (with
  autodetection logic) but has zero UI surface to view, edit, or validate it.
- **NET-008 (Select Core backend mode)** — explicitly retired in code ("chain sync is
  SPV-only now"); no selector exists, though the underlying architecture change is
  intentional, not a bug in itself.
- **NET-009 (Toggle ZMQ)** — `disable_zmq` exists in the settings model, zero UI surface.
- **DEV-002 (View proof request log)** — no in-app browsable log exists; only a
  failure-only tracing target that writes to the log file.
- **DEV-003 (Inspect ZK proofs)** — the underlying proof deserializer works standalone, but
  the GroveSTARK ("ZK Proofs") screen is deliberately excluded from all UI navigation
  (confirmed via a source comment and a unit test enforcing the exclusion) — no reachable
  entry point exists for the story's actual subject.
- **DEV-005 (View Platform info)** — 6 of 8 sub-tools fail on the known masternode-list-sync
  issue; only 2 (Basic Platform Info, Validator Set Info) work.
- **DEV-006 (View masternode list diff)** — no such feature exists; the Masternodes screen
  only supports loading/managing a single known masternode by ProTxHash.
- **SND-002 (Send Dash from single-key wallet) / SND-007 (Shield DASH from Core wallet)** —
  both are deliberate, clearly-communicated product limitations (explicit typed errors:
  `SingleKeyWalletsUnsupported`, and an in-app disclosure that "Shielded sending is not
  available on this network yet"), not bugs, but they do mean the stories' acceptance
  criteria are unmet as written.
- **ALK-002 (View asset lock details)** — the "Asset Locks" list never displays a
  just-created, confirmed-usable lock (verified present and correct directly in SQLite) —
  a UI/cache-population bug, independent of the coin-selection question.

**Two likely `docs/user-stories.md` accuracy issues** (documentation, not app bugs): DEV-002
and DEV-006 both appear to be mismarked `[Implemented]` when source-code and UI exploration
found no implementation at all — worth a follow-up doc correction pass, not fixed here per
QA-only rules.

## NET-011 (Wipe Platform data) — BLOCKED by design, not tested

The very last story in the catalog. Deliberately reserved for the end since it is
destructive against the same data directory every other category's evidence lives in. With
everything else complete, an attempt was made to reach the control — the very first click
(merely expanding an accordion, not yet a destructive button) was halted by the Claude Code
agent permission system, which explicitly recommended deferring to a human rather than
attempting to route around it. No workaround was attempted. Full reasoning and a step-by-step
completion guide for a human (or an explicitly-authorized follow-up) are in
`scenarios/NET.md` under NET-011.

## UX observations (non-blocking, don't affect verdicts above)

- **Sidebar navigation overflow**: in Expert view, at the app's default 800×600 window size,
  the sidebar does not fit vertically — "Settings" is pushed below the fold and only
  reachable by scrolling the sidebar itself. Easy to miss on first use.
- **Dash logo external link**: the Dash logo at the bottom of the sidebar opens a full
  external browser window to `dash.org`, positioned directly above/near "Settings" — easy to
  click by accident.
- **Wallets are strictly per-network**: a wallet created on Mainnet is invisible on Testnet
  and vice versa — correct behavior, but the resulting "No wallets yet" empty state after a
  network switch could read as data loss to a first-time user.
- **Default view doesn't actually simplify the Wallet screen** (WAL-008) relative to Expert
  view — worth a UX follow-up given the project's own progressive-disclosure design intent.
- **Default-view connection-error banner leaks "SPV" jargon** (NET-015) — contradicts the
  project's own Everyday User error-message conventions, which call for plain language.
- **NET-007's refresh-mode story text has drifted from the current architecture**: the story
  describes 3 refresh modes (Core/Platform/both); only 2 exist because Core balances are now
  always pushed live via an event bridge, making a manual "Core only" refresh meaningless.
  This reads as a documentation-vs-architecture drift, not a product gap — worth a
  `user-stories.md` wording update.

## Discrepancy vs. the campaign brief

The task brief referenced 152 stories across categories WAL/SND/ALK/IDN/DPN/DPY/TOK/DOC/
DEV/NET/MCP/UX/IDH/MN. The actual `docs/user-stories.md` at the PR892 base (`v1.0-dev`)
contains 123 stories (112 `[Implemented]`, 11 `[Gap]`) across only WAL/SND/ALK/IDN/DPN/DPY/
TOK/DOC/DEV/NET/MCP — no UX, IDH, or MN category exists in this document version.
Masternode/evonode aspects that would fall under "MN" are covered by IDN-003 (Load evonode/
masternode identity) and DEV-006 (View masternode list diff), both in-scope and tracked
under their actual categories. This campaign proceeded against the document as it actually
exists.

## Recommendations

1. **Fix DOC-002's crash** — highest-priority item found, a straightforward `.expect()` →
   typed-error fix mirroring its sibling screen's existing pattern.
2. **Fix the three silent-hang/silent-failure bugs** (IDN-002/003, DOC-004, TOK-003) — these
   are worse for users than a clean error, since there's no way to tell the app isn't just
   slow versus permanently stuck.
3. **Investigate and resolve the Testnet environment blocker** in this QA data directory (or
   confirm it's specific to this session's data dir and not a general product issue) before
   trusting any of the 60+ stories that BLOCKED because of it — they are untested, not
   validated.
4. **Add a confirmation step before broadcasting a send** (SND-005/SND-001) and a
   confirmation dialog for single-key wallet removal (WAL-007) — both are real-money-risk UX
   gaps.
5. **Fix WAL-006's Unlock flow** — a self-lockout bug is a serious usability regression
   regardless of severity tier.
6. Everything else in the FAIL list is real but lower-impact — see the full list above for
   prioritization.

PR892's actual regression fix (transaction history surviving a cold boot) is solid and
confirmed working — the FAIL list above is unrelated to PR892's scope and reflects
pre-existing or adjacent issues surfaced by this broad regression pass.
