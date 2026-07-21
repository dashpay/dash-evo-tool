# DPNS Voting Experience — UX Specification

## Adopted direction (2026-07-21)

This revision supersedes the earlier three-step Voting Center journeys in this
document. The durable operation coordinator remains the safety model, but the
separate Masternodes `Nodes / Voting / Scheduled` navigation and full-page
wizard are no longer part of the product. The earlier design remains available
in git history as the rejected v2 exploration.

The accepted direction keeps contest decisions where operators discover them:
DPNS Active contests. A single screen supports one vote, several contests,
several nodes, immediate casting, and scheduling. Masternode detail provides
only a plain `DPNS Voting` link to Active contests and never carries node or
contest preselection.

## Rationale

- Contest-first cards let an operator understand the decision before choosing
  nodes or timing.
- Read-only tally chips and selectable vote controls have different shapes and
  interaction states, removing the old ambiguity where a count looked like a
  vote button.
- The review surface starts with the common case: all loaded nodes and cast now.
- Per-node timing remains available without making every operator configure a
  matrix.
- Durable typed operations, exact target locks, and recovery messages remain
  visible without requiring a separate operation page.

## Information architecture

```text
Masternodes
├── node list
└── node detail
    └── DPNS Voting ──> DPNS / Active contests

DPNS
├── Active contests     (all vote composition and recent activity)
├── Past contests       (human-readable outcomes)
├── My usernames
└── Scheduled votes     (human-readable node, choice, time, status)
```

## Active contests

Contests render as cards, grouped in this order:

1. `Needs your vote`
2. `Voted`
3. `Not votable by your nodes`

Each group is collapsible. The first group opens by default; the last group is
dimmed and its vote controls are disabled. A filter remains available.

```text
┌─ Active contests ─────────────────────────────────────────────┐
│ Filter by name: [________________]                         │
│ Each node can change its vote up to four times after its   │
│ initial vote.                                              │
│                                                              │
│ ▾ Needs your vote (2)                                    │
│ ┌─ alice.dash                         Voting ends in 2d ─┐ │
│ │ [Lock name]  (12 votes)  [Abstain]  (3 votes)        │ │
│ │ [Vote for Alice] (20 votes)  3MN5mF…s8Qp              │ │
│ │ [Vote for Alyce]  (18 votes)  7Yk2aP…d1Rt              │ │
│ └──────────────────────────────────────────────────────┘ │
│ ▸ Voted (4)                                             │
│ ▸ Not votable by your nodes (1)                         │
├──────────────────────────────────────────────────────────────┤
│ Votes ready to cast: 1                  [Review and cast]   │
└──────────────────────────────────────────────────────────────┘
```

Parenthesized tally chips are labels, not buttons. `selectable_label` controls
contain an action phrase such as `Vote for Alice`, `Lock name`, or `Abstain`.
Selecting the active choice again removes it from the draft.

## Review and cast sheet

The sticky tray opens a sheet in the same Active-contests screen. The default
applies the chosen contests to all loaded voting nodes and casts now.

```text
┌─ Review and cast ───────────────────────────────────────┐
│ Selected votes                                               │
│ alice.dash  →  Vote for Alice  · voting ends in 2d         │
│                                                               │
│ All my nodes                                                 │
│ Cast timing: [Cast now v]                                    │
│                                                               │
│ ▸ Choose per node (advanced)                              │
│                                                               │
│ [Cancel]                                      [Submit votes] │
└───────────────────────────────────────────────────────────────┘
```

Timing choices are `Cast now`, `Schedule`, and `Do not use this node`.
Scheduling exposes the existing day/hour/minute controls and reminds the user
that DET must stay open and connected. The advanced disclosure exposes the
same timing choice for each node. Review removes exact no-op targets, blocks
targets already held by an unresolved operation, and refuses submission when
proved current state is unavailable.

## Submission and recovery

`SubmitDpnsVoteOperation` is the only submit path. Explicit submissions,
manual scheduled casting, and `Check again` show the full-window progress
overlay until a terminal task result or error arrives.

Recent voting activity appears below the contest groups and uses typed target
states:

```text
┌─ Recent voting activity ─────────────────────────────────┐
│ alice.dash — Vote for Alice — Confirmed                      │
│ example.dash — Lock name — Confirmation is still checked    │
│ This vote may already have been submitted. Do not submit it  │
│ again.                                          [Check again] │
│ other.dash — Abstain — Not applied            [Review again]│
└───────────────────────────────────────────────────────────────┘
```

`Confirmed`, `Confirming`, `Unconfirmed`, `Rejected`, `Not applied`, and
pre-submission failure are never inferred from strings. Unconfirmed targets
retain their lock and explicitly warn against resubmission.

## Scheduled votes and history

Scheduled votes remain a DPNS sub-screen. Rows show `{name}.dash`, node alias
or shortened Base58 identifier, a human-readable choice, absolute UTC time plus
relative time, typed status, and only valid actions. Past contests describe the
winner or locked outcome in words rather than exposing an unexplained raw ID.

## Accessibility and responsive behavior

- Tally chips have no click or keyboard behavior.
- Every selectable control contains the action and target in its label.
- Disabled controls explain what must change before voting is available.
- Cards wrap vote choices before truncating identifiers.
- The review tray remains outside the scrolling card region.
- The advanced matrix starts collapsed and remains keyboard reachable.
- Status is always expressed in text; color is supplementary.
