# Scenario: Masternodes ▸ Votes hub — browse, stage, confirm, schedule, node detail

**Verifies:** the screens a masternode operator uses to see and decide name
contests: the Masternodes tab segments, contest cards, staging a choice,
the confirmation step, scheduling options, node detail, and layout at a
narrow window. (Risk area: #901, #1052, #1053.)

**Tier justification:** Needs live contest data for loaded masternodes, the
real background refresh, real window resizing and the real light/dark theme —
beyond the in-process `kittest` harness.

**Run against BOTH builds with this identical procedure** — the baseline and
development binaries selected for the current campaign (record their exact
SHAs in that campaign's own artifacts, not here). Describe what you observe
on each build without assuming which one is "correct", and note where a
control or screen exists on only one build (a feature-presence difference,
not a step failure).

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Prerequisites

- Network: testnet (contests last about 90 minutes, so they can be observed
  without waiting days)
- Environment variables (names only):
  - `E2E_MN_PROTX_HASH` — a currently registered masternode/evonode
  - `E2E_MN_VOTING_KEY` — its voting key (optional; without it the Votes
    view shows its "missing voting key" empty state, which is itself worth
    recording)
  - `E2E_MN_OWNER_KEY` — only if a step needs the owner key
- Expert mode (or the highest interface level) turned on in Settings
- At least one live contested name on the selected network, if available
- Load the node through the Masternodes tab's load form

## Setup

```bash
DATADIR=$(mktemp -d)
cp .env.example "$DATADIR/.env"
pgrep -af dash-evo-tool

BIN=<path to the build under test — baseline or development worktree binary>
test -x "$BIN"
LOG="$DATADIR/masternodes-votes-hub.log"
DASH_EVO_DATA_DIR="$DATADIR" nohup "$BIN" >"$LOG" 2>&1 &
```

Resize the window (see the README's "Known UI/environment quirks") before
judging layout. Check `det-stderr.log` / `det.log` in `$DATADIR` for panics
after each run.

## Procedure

1. **Locate voting.** Record which left-nav entries and Tools sub-entries
   exist. Record whether Tools offers any contested-names/DPNS entry (older
   builds keep these under Tools ▸ DPNS: Active, Past, My usernames, Scheduled
   votes), and where (if anywhere) you can see contested names, scheduled
   votes and past contests.
2. **Masternodes tab layout.** Open Masternodes with no node loaded; record
   the empty state and primary action. Load the node; record the card
   contents (type, voter readiness, key status, voting-status line) and any
   segment/tab header (names and counts in labels; a newer build shows
   Votes / Nodes segments).
3. **Contests view.** Open the contest view for the loaded node (sub-views
   such as "To decide", "Voted", "Scheduled", "History"). A freshly created
   contest's row may need a manual Refresh before it shows; contest names are
   displayed in normalized form (for example `0` for `o`). Record the
   sub-views/filters offered, the contest cards (tally, deadline, node set,
   changes left) and any attention indicator in the top bar or navigation.
4. **Stage without sending.** Pick a choice on a contest card (for a
   requester, lock, or abstain). Record how the choices are presented (one
   row each or inline), the selection mark, the keyboard hints, and any
   bulk-action bar when several cards are selected. Do not press the final
   cast control yet.
5. **Confirmation step.** Open the confirmation. Record its title, the
   choice list (names and contender identity text; choices read like "Vote
   for <name> (<id>)", "Lock name", "Abstain"), the "Vote with:" node-set
   control (All / Evonodes only / Masternodes only / Custom, "Save as my
   default"), node counts, timing
   options and their defaults, the time-zone wording on any date field, and
   the exact label of the final button (for example "Cast 1 vote" or
   "Schedule 1 vote"). The window is titled "Confirm votes" and is non-modal;
   record where it is anchored and whether the screen behind stays usable.
   Then **cancel** out of it.
6. **Schedule options.** In the confirmation open the "specific time" option.
   Record the pre-filled date/time relative to now (and relative to the
   contest deadline), the zone label, and any warning shown. Cancel out.
7. **Cast (MUTATES: votes on-chain).** Only with a per-build equivalent
   contest/node fixture: confirm one decision for one node and record the
   progress display, the result text, and the node's remaining changes.
   Skip and say so if no equivalent fixture exists.
8. **Node detail.** Open the loaded node's detail view. Record the sections
   present, any votes table, any "vote with this node" style action, and
   whether voting controls are embedded in the detail view. Use that action
   (if present) and record the resulting screen and node-set selection.
9. **Narrow window.** Resize the window to about 800 px wide with a contest
   card that shows a full contender identity. Record whether the card, the
   Cast/Clear controls and the bottom summary bar stay fully inside the window
   and clickable.
10. **Light and dark theme.** Repeat steps 3 and 5 in both light and dark
    theme. Record any text invisible against its background (headings, bottom
    bar summary, button labels).
11. **Restart persistence.** Restart the app. Record whether staged/scheduled
    decisions and the node set are remembered.
12. **Old schedules after an upgrade (potentially MUTATES: an older build
    may cast the schedules on launch).** Only if a data directory from an
    earlier version holding unexecuted scheduled votes is available (copy it
    to a fresh directory first; do not use a real user's): start each build on
    it. Record any startup notice (wording, whether it lists votes, whether it
    returns on the next launch) and whether the old schedules appear or run.

## Safety constraints specific to this scenario

- Step 7 is the only step that casts by design; cast a single decision with a
  single testnet node.
- Step 12 can also change voting state: a build that predates #901
  automatically casts due schedules on launch, and copying the data directory
  isolates local storage, not the node's live votes. Run it only against a
  controlled testnet fixture with equivalent, independent voting state per
  build and a bounded, known number of executable schedules (or with the
  voting keys removed), and check the schedules before each launch. The
  current build only reports old schedules and never casts them.
- Never print or screenshot the key-entry fields while populated.

## Expected outcome / pass criteria

Record the observed behavior for each build at every step, with screenshots
for steps 3, 5, 9 and 10. Then judge each difference by the contract below.

- A cast that records the wrong choice, a lost scheduled decision after
  restart, or a control unreachable at the narrow width, on one build only, is
  a difference to classify under the contract.

Apply the [A/B build comparison contract](../README.md#ab-build-comparison-contract):
its blocker rule decides what blocks, and its equivalent-fixture rule applies
to every step marked **MUTATES** below.

## Known gotchas

- When both builds must vote on one contest with the same node, the network
  may refuse the second identical vote; plan a second node or contest.
- Testnet contests may not exist at the time of the run; record that and run
  the node-detail, empty-state and theme steps anyway.
- Remaining-vote-changes counts are tracked per device; a freshly seeded data
  directory may legitimately show "Unknown" on both builds.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
