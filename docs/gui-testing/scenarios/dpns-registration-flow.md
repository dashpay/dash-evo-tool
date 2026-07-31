# Scenario: DPNS registration messaging and social-profile save feedback

**Verifies:** DPNS registration correctly distinguishes "registered outright"
from "submitted for a community vote" (contested name), a pending
registration shows a clear indicator/tooltip instead of looking unrequested,
the onboarding checklist counts a pending request toward "Pick a username,"
and — from the same underlying PR — the DashPay social-profile save flow
gives real progress/success/error feedback without a stuck progress banner
or a stale result misattributed after switching identities mid-save. (Risk
area: #918.)

**Tier justification:** Needs a real contested-name registration against a
live testnet name-contest window, real onboarding-checklist state derived
from actual identity data, and real async save timing (a save in flight
when the user switches identity) — none of which a no-display harness can
drive with a live contest clock and real background-task scheduling.

**Run against BOTH builds with this identical procedure** (baseline
`v1.0.0-weekly.20260721`, then HEAD `origin/v1.0-dev`).

## Prerequisites

- Network: testnet
- Environment variables (names only):
  - `E2E_WALLET_MNEMONIC` — funded testnet wallet
- A DPNS name likely to be contested (short, generic, or currently popular)
  to register during an active contest window
- At least two identities on the same wallet (for the identity-switch-
  mid-save step)
- Verify exact tooltip/button text during execution rather than assuming
  the wording below is literal

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or HEAD worktree binary>
test -x "$BIN"
LOG="$DATADIR/dpns-registration.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

## Procedure

1. **Register a contested name.** On an identity with no username yet,
   register a name likely to be contested. Record the exact completion
   message shown — does it say "registered" or does it indicate a pending
   community vote?
2. **Identities list indicator.** Record whether/how the Identities list
   shows this identity's pending request differently from "no username."
3. **Identity Home indicator.** Record what the Identity Home hero card
   shows for this identity — the requested name, a status indicator, or the
   generic "pick a username" prompt.
4. **Indicator tooltip.** If a status indicator/pill is present, open its
   tooltip and record what it says (who decides, any estimated timing).
5. **Onboarding checklist.** Record whether the onboarding checklist treats
   the submitted request as satisfying "Pick a username," and what wording
   it uses.
6. **Uncontested name, for comparison.** On a second identity, register a
   name unlikely to be contested and record the completion messaging —
   confirm it's still distinguishable from the contested case.
7. **Contest resolves.** If the contest window is short enough to wait out,
   revisit the identity after resolution and record what the indicator does
   (disappears / name displays normally on a win; reverts to "pick a
   username" on a loss).
8. **Social profile save, normal case.** Open Contacts → set up/edit social
   profile, change the display name or bio, save. Record what feedback
   appears during and after the save (progress indicator, then a
   success/error banner) and whether the progress indicator clears when the
   outcome appears.
9. **Social profile save, forced failure.** If you can force a failure
   (briefly disconnect network mid-save), record the same as step 8 for the
   failure path.
10. **Switch identity mid-save.** Start a profile save on identity A, and
    before it completes, switch to identity B. Once the save's result
    arrives in the background, record which identity (if any) shows a
    result banner, and whether it's ever attributed to the wrong
    (now-active) identity.
11. **Contacts setup CTA.** On an identity with no DashPay profile, open
    the Contacts tab and record the setup card's CTA wording and whether any
    "Why?"/explanation control actually opens an explanation.

## Safety constraints specific to this scenario

- None beyond the standing rules — DPNS registration fees are small and
  testnet-only here.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, then apply this
rule:

- **BLOCKING** only if HEAD is worse than baseline in the happy flow (e.g.
  a contested registration falsely claims "Registered!" on HEAD when
  baseline correctly said "pending"; a save's progress banner gets
  permanently stuck on HEAD where baseline cleared it; a save result gets
  attributed to the wrong identity on HEAD where baseline didn't), or a
  **data-loss** outcome (a profile edit silently lost) on either build.
- **NOT blocking**: a timing-sensitive race that needs several attempts to
  reproduce and only sometimes shows on either build; wording differences
  with no functional consequence; an issue reproducing identically on both
  builds.
- If baseline doesn't have a pending-registration indicator at all (a new
  feature on HEAD), record that as a feature-presence difference, not a
  step failure on baseline.

## Known gotchas

- Contest resolution timing depends on the live masternode voting schedule
  on testnet — step 7 may need to be deferred to a later session rather than
  blocking the rest of this scenario.
- The identity-switch race (step 10) is timing-sensitive; several attempts
  with the save action and the identity switch performed in quick succession
  may be needed to land inside the race window on either build.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
