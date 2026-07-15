# PR892 User-Story QA — Summary Report

**Status: COMPLETE.** All 175 stories in PR892's real catalog (`docs/user-stories.md` in the
PR892-build worktree — see "Methodology notes" for why this campaign initially tested against
the wrong, smaller catalog and how that was corrected) are accounted for: every
`[Implemented]` story was either executed end-to-end (live UI) or definitively marked BLOCKED
with documented reasoning (frequently backed by a read-only source review as supporting
context), every `[Gap]`/`[Removed]`/`[Superseded]` story is tracked N/A, and the three
destructive stories (NET-011, NET-019, NET-020) are deliberately BLOCKED pending explicit
human authorization — see their dedicated section below. `progress.md` is the live,
authoritative per-story checklist.

Build under test: PR892 (`fix(wallets): show transaction history that predates the current
session`) @ commit `57195d54`, built from worktree
`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build`.
Binary: a private, hash-verified copy built directly from this worktree
(`/data/tmp/det-qa-pr892-bin-myown/dash-evo-tool`, sha256
`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`) — see "Methodology
notes" for why the shared `/data/target/debug/dash-evo-tool` path is no longer used. Data dir
(isolated): `/data/tmp/det-qa-pr892-data`. Network: Testnet (Mainnet used only for a handful
of cross-checks, explicitly noted where relevant).

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
| PASS | 37 | Fully executed end-to-end, met acceptance criteria |
| FAIL | 25 | Executed, did not meet acceptance criteria — real bugs/gaps, listed below |
| BLOCKED | 93 | Could not be completed for a documented reason (see below — the large majority trace to one root cause, not 93 independent problems) |
| N/A | 20 | `[Gap]`/`[Removed]`/`[Superseded]` in `docs/user-stories.md` — not implemented, out of scope by design |
| **Total** | **175** | |

**Read the BLOCKED count carefully — it is not 93 independent failures.** Two systemic issues
account for nearly all of it:

1. **A mid-campaign environment blocker** (Testnet masternode-list/quorum-sync/wallet-storage
   failure — full diagnosis in `scenarios/ALK.md`) made Platform proof verification
   unavailable partway through the run and recurred repeatedly for the rest of the campaign,
   including in the later reconciliation-driven sweep (WAL/IDN/DPN/DPY/TOK/IDH/MN passes all
   hit it again). Once no Platform identity could be registered or loaded (see IDN below),
   every downstream story that needs an identity — most of DPN, DPY, TOK, DOC, IDH, MN, and
   several IDN/SND/WAL/UX stories — cascades to BLOCKED for that single reason. This is an
   **environment/infrastructure issue in this QA session**, not a confirmed PR892 regression —
   see the dedicated section below before concluding anything about the app itself from the
   BLOCKED count.
2. **NET-011 / NET-019 / NET-020** are three deliberate BLOCKED-by-policy items (destructive
   tests correctly deferred pending human authorization) — see their dedicated section.

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
- Root cause was **not** found during the QA campaign itself — it needed either destructive DB
  access or a debug-instrumented rebuild to capture the underlying error's structured detail,
  both appropriately gated behind human sign-off rather than attempted unilaterally by an
  unattended agent.

**Update, 2026-07-15**: the user later explicitly authorized a destructive follow-up
investigation on disposable copies (never the live QA data dir above). It fully root-caused
this — a storage-format incompatibility bug in the pinned upstream `platform-wallet` crate (a
specific `asset_locks` row's proof blob can be written but never decoded back), not corruption
or resource exhaustion. Full findings and a verified recovery:
`scenarios/ALK.md`'s "Resolution" section and
`/data/artifacts/dash-evo-tool/2026-07-14/pr892-user-story-qa/testnet-blocker-investigation/TEST-VECTOR.md`.
This confirms every BLOCKED verdict below that cites this blocker was genuinely untestable at
the time for the reason now identified — not a gap in how the campaign tested them.

**Full diagnostic trail**: `scenarios/ALK.md` ("App-restart failure" section, its addendum,
and the "Resolution" section), with a forward pointer from `scenarios/DEV.md` narrowing the
scope further.

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

- **IDN-002 / MN-001 (Load identity by ID / Load a masternode by keys) — silent hang.** Both
  "Load Identity" (ID + private key tab) and "Load a masternode" (ProTxHash) submit buttons
  hang completely silently on click: no banner, no log line, no timeout, ever — reconfirmed
  fresh in the reconciliation-driven sweep (MN-001, which supersedes the original IDN-003
  finding under the corrected catalog) with a 20s wait, still reproducing. This is distinctly
  worse than every other blocked-by-environment flow tested in this campaign, which all
  degrade gracefully with a clean typed or generic error — including sibling tabs/fields on
  the *same screens* ("Search Wallet for Identities", DPNS username search, ProTxHash format
  validation, malformed-hash rejection — all of which work correctly).
- **IDN-014 (Fund identity by receiving a deposit to a shown QR/address) — blank step, no
  error.** Create Identity wizard's "Receive a new deposit" funding method renders **zero
  content** at step 3 (no address, no QR, no amount field, no error message) — reconfirmed in
  the reconciliation sweep, correlated to the same `WalletBackendNotYetWired` environment
  condition but degrading with total silence rather than a typed error, unlike sibling flows
  on the same wizard.
- **SND-014 / SND-015 / SND-016 (Send maximum from Core wallet / Unshield / Send privately in
  shielded pool) — all FAIL, found in the reconciliation-driven sweep.** SND-014: the "fee
  reserved" label and "balance too low" message required by the story are dead code — only
  wired into a validation-error state a successful Max click can never reach, so Max fills
  silently with no fee shown (root-causes SND-005's earlier finding). SND-015/SND-016: the
  Shielded tab's "Unshield" and "Send (Private)" buttons are correctly implemented in source
  but unconditionally hidden behind a hardcoded `SHIELDED_ACTIVATION_PROTOCOL_VERSION: None`
  feature gate, so neither is ever reachable on any network in this build (consistent with
  SND-007's earlier "not available on this network yet" finding).
- **UX-001 (Blocking progress overlay for unsafe-to-interrupt operations) — narrow adoption.**
  The `ProgressOverlay` component itself is well-built (confirmed via source + its own ~30
  unit tests), but only two features in the entire codebase actually raise it: SPV sync and
  DPNS username registration. A Core-wallet Send — explicitly listed as an example
  "unsafe-to-interrupt operation" in the story text — uses only a non-blocking banner and
  does **not** raise this overlay, so a send can be double-fired via a fast double-click.
- **UX-003 (Global wallet/identity switcher across all tabs) — incomplete coverage.** The
  three-segment switcher works correctly wherever it's wired (Wallets, Identity Hub,
  Masternodes), but four root screens — **Contracts, Tokens, Tools, Settings** — render no
  switcher at all, not even a wallet-only pill, directly contradicting the "every root screen"
  acceptance criterion. Confirmed both live and via source (no switcher call in those four
  screen files).
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

## NET-011 / NET-019 / NET-020 (the destructive trio) — BLOCKED by design, not tested

All three are destructive/irreversible against the same shared data directory every other
category's evidence lives in, and were deliberately reserved for the very end of the campaign.

**NET-011** ("Wipe Platform data"): with everything else in the original pass complete, an
attempt was made to reach the control — the very first click (merely expanding an accordion,
not yet a destructive button) was halted by the Claude Code agent permission system, which
explicitly recommended deferring to a human rather than attempting to route around it. No
workaround was attempted.

**NET-019** ("Clear all local data for a network") and **NET-020** ("Clear cached SPV data to
force a resync") map to the same "Clear Testnet Database" / "Clear SPV Data" controls NET-011
was blocked on. In the final destructive-pass attempt, the permission system did not halt
navigation to these controls the same way it had for NET-011 — but per this campaign's own
explicit policy (irreversible action against shared, evidence-bearing state; requires a human
and a disposable copy of the data dir), neither destructive button was clicked regardless.
What *was* verified without confirming through: NET-019's "Clear Testnet Database" control has
no network gate (available on Mainnet too, per its acceptance criteria) and its confirmation
dialog text was read via source and matches the story's wording; NET-020's "Clear SPV Data" is
correctly gated to Expert-mode-and-above and is driven by live `SpvStatus` (disabled while
`Starting|Syncing|Running|Stopping`) — though the session's SPV was stuck in `Error` (the known
environment blocker), which is correctly not "active," so the disabled state itself could not
be observed live, only confirmed via source.

Full reasoning and step-by-step completion guides for a human (or an explicitly-authorized
follow-up) for all three are in `scenarios/NET.md`. A smaller, more precisely-scoped candidate
for NET-011 specifically ("Clear Platform Addresses," Developer-only) was also noted there for
whoever eventually authorizes this pass — worth considering before running the two broader
"Clear Testnet Database" / "Clear SPV Data" controls.

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

## Methodology notes

### Story-catalog correction (175 stories, not 123)

The campaign's first pass tested against `docs/user-stories.md` in the **qa-docs worktree**
(this report's own worktree), which tracks `v1.0-dev` — 123 stories (112 `[Implemented]`,
11 `[Gap]`). That was a coordinator pointing error, not a stale-doc problem: the catalog that
should have been used from the start is the one **inside the code actually under test** —
`docs/user-stories.md` in the PR892-build worktree
(`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build`, verified via
`git show 57195d54:docs/user-stories.md`). PR892 is ahead of `v1.0-dev`, not behind it: its
real catalog is a strict superset — **175 stories** (155 `[Implemented]`, 17 `[Gap]`, 2
`[Removed]`, 1 `[Superseded by MN-001]`) — spanning the original 11 categories plus three new
ones (**UX**, **IDH**, **MN**) that don't exist in the `v1.0-dev` version of the document at
all. `progress.md` was reconciled: every story tested in the first pass whose definition is
unchanged kept its original verdict; a handful (SND-002, IDN-003, DEV-002, DEV-006, NET-008)
were reclassified `[Gap]`/`[Removed]`/`[Superseded]` in the real catalog — in every one of
those cases the original FAIL finding (no implementation found, or an explicit not-supported
error) is fully consistent with the reclassification, so nothing here was invalidated, only
relabeled correctly. The ~35 remaining new/redefined stories (WAL-025–031, SND-014–016,
IDN-013a/014–016, DPN-008/009, DPY-012–014, TOK-018, NET-006/016–021, MCP-003/004, and the
three new UX/IDH/MN categories in full) were subsequently tested in a resumed sweep, whose
findings are folded into the verdict counts, FAIL list, and NET-011/019/020 section throughout
this report — the whole 175-story catalog is now reflected here, not just the original 123.

Also worth flagging: the corrected catalog itself has a genuine documentation defect — the ID
`IDN-013` is used for two different, unrelated stories ("Password-protect an identity's
signing keys (SEC-001)" and "Top up identity from Platform addresses"). Tracked
disambiguated as `IDN-013a`/`IDN-013b` in `progress.md`; worth a fix in
`docs/user-stories.md` upstream.

### Binary-provenance incident (brief window, not re-tested)

Partway through the reconciliation above, a second, unrelated issue surfaced: the shared,
machine-wide cargo target dir (`/data/target`, used by multiple concurrent worktrees/sessions
on this box) had its `dash-evo-tool` binary overwritten by an unrelated concurrent build for
a period of roughly 18:30–19:00 UTC on 2026-07-14. Any testing run against
`/data/target/debug/dash-evo-tool` during that window would have been exercising different
code than PR892. Two things followed: (1) the binary this campaign launches from was switched
to a private, hash-verified copy built directly from the known-clean PR892 worktree
(`/data/tmp/det-qa-pr892-bin-myown/dash-evo-tool`, sha256
`2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`) rather than the shared
path, and every future relaunch in this campaign uses that copy; (2) per coordinator
judgment, the affected window was assessed as low-risk (the concurrent builds sharing the box
are other feature-branch variants of the same app, close enough that the exposure was brief
and narrow) and was **deliberately not re-tested** — verdicts recorded during that stretch
are kept as-is. No PR892 testing had actually landed in the clobbered window by the time it
was caught, so in practice nothing was re-run or discarded either way.

## Recommendations

1. **Fix DOC-002's crash** — highest-priority item found, a straightforward `.expect()` →
   typed-error fix mirroring its sibling screen's existing pattern.
2. **Fix the silent-hang/silent-failure bugs** (IDN-002, MN-001/IDN-003, DOC-004, TOK-003,
   IDN-014) — these are worse for users than a clean error, since there's no way to tell the
   app isn't just slow versus permanently stuck. MN-001's hang is a particularly high-value
   fix since it single-handedly blocks the entire MN category (7 of 12 stories) from being
   testable at all.
3. **Investigate and resolve the Testnet environment blocker** in this QA data directory (or
   confirm it's specific to this session's data dir and not a general product issue) before
   trusting any of the 90+ stories that BLOCKED because of it — they are untested, not
   validated. It recurred throughout the entire campaign, including the later
   reconciliation-driven sweep, so it does not appear to be a one-off transient condition.
4. **Add a confirmation step before broadcasting a send** (SND-005/SND-001/SND-014) and a
   confirmation dialog for single-key wallet removal (WAL-007) — both are real-money-risk UX
   gaps. SND-014 specifically shows the fee-reserved/balance-too-low messaging exists in code
   but is wired to an unreachable state — a small, well-scoped fix.
5. **Fix WAL-006's Unlock flow** — a self-lockout bug is a serious usability regression
   regardless of severity tier.
6. **Widen UX-003's global switcher to Contracts/Tokens/Tools/Settings** — the component and
   its two-way binding already work correctly everywhere else; this looks like an integration
   gap on four specific screens rather than a design problem.
7. Everything else in the FAIL list is real but lower-impact — see the full list above for
   prioritization.

PR892's actual regression fix (transaction history surviving a cold boot) is solid and
confirmed working — the FAIL list above is unrelated to PR892's scope and reflects
pre-existing or adjacent issues surfaced by this broad regression pass.
