# DPNS Voting Experience — UX Specification

## Experience principle

Voting is one operator workflow with two entry speeds:

- **Quick vote** — one selected node, usually one or a few contests.
- **Voting Center** — many contests, many nodes, immediate and scheduled
  targets.

Both are views over the same draft, authoritative vote state, and operation
coordinator. They must never disagree about current votes or progress.

## Information architecture

```text
Masternodes
├── Nodes
│   ├── Node cards
│   └── Node detail
│       ├── Keys and actions
│       ├── Quick voting
│       └── Open Voting Center (filtered to this node)
├── Voting
│   ├── Active contests
│   ├── Vote composer
│   └── Recent operations
└── Scheduled
    ├── Upcoming
    ├── Needs attention
    └── Completed

DPNS
├── Active contests ── “Vote with masternodes” ──> Masternodes / Voting
├── Past contests
└── My usernames
```

The existing DPNS bulk popup is retired after the shared Voting Center is
available. DPNS contest browsing remains; operator actions route to
Masternodes.

## Shared concepts

### Current vote

Every node × contest row shows one of:

- `Not voted`
- `Current vote: Abstain`
- `Current vote: Lock`
- `Current vote: {candidate}`
- `Checking current vote…`
- `Current vote unavailable`

An existing vote does not remove the contest. Selecting a different choice is
labeled as a change.

`Current vote unavailable` disables that node's choice controls and offers
`Refresh vote state`. DET never treats an unavailable query as `Not voted`.

Node cards summarize both concepts, for example:

> 3 active contests · 1 needs a vote

When every active contest has a current vote:

> Votes cast in all active contests

### Target

A target is one requested action for one node on one contest. Batch progress
and results are always expressed in targets, never only in aggregate.

### Operation

An operation is the reviewed collection of targets submitted together. It owns
progress across screens and process restarts.

## Journey A — Quick vote from a node

1. Priya opens a node.
2. The DPNS section shows all active contests and the node's current choice.
3. Priya selects a new choice on one or more rows.
4. The primary action becomes `Review 1 vote` or `Review {n} votes`.
5. Review lists current → requested choice for this node.
6. Priya chooses `Cast now` or `Schedule instead`, then confirms.
7. Affected rows become read-only and show `Submitting…` or `Scheduled`.
8. Priya may navigate elsewhere. A global banner links to operation progress.
9. Confirmed rows update their current vote in place; they do not disappear.

```text
DPNS name contests (3)

alice.dash
Current vote: Abstain
( Abstain ) ( Lock ) ( Vote for alice )

dominguez.dash
Not voted
( Abstain ) ( Lock ) ( Vote for dominguez )

                           [ Review 1 vote ]
                           [ Open Voting Center ]
```

## Journey B — Bulk vote or schedule

The composer is a three-step full-page flow, not a transient popup. Nodes come
first so every later “current vote” summary has a defined node scope.

### Step 1: Nodes and timing

Priya selects nodes and chooses timing. `Set all` applies timing only; it does
not alter contest choices.

```text
Voting Center                    Step 1 of 3: Nodes and timing

Set all: [ Cast now v ]  [ Apply ]

[x] Eve Mainnet       Cast now
[x] Backup Evo        Schedule: 2026-07-20 18:00 UTC
[ ] Test Operator     Do not use this node

                               [ Next: Choose votes ]
```

### Step 2: Votes

Priya selects contests and a requested choice for each contest. Current-state
summaries cover only the nodes selected in Step 1.

```text
Step 2 of 3: Votes

[ ] alice.dash       Current across selected nodes: Mixed
    Abstain | Lock | Vote for alice

[x] dominguez.dash   Current across selected nodes: 1 not voted, 1 Lock
    Abstain | Lock | Vote for dominguez

                                  [ Back ] [ Review 2 targets ]
```

### Step 3: Review

The review expands the cartesian product into exact targets. No-op targets are
removed and explained.

```text
Step 3 of 3: Review

Node            Contest          Current       Requested      When
Eve Mainnet     dominguez.dash   Not voted     dominguez      Now
Backup Evo      dominguez.dash   Lock          dominguez      Jul 20

2 targets total. Each vote uses Platform credits.

                          [ Back ] [ Submit 2 targets ]
```

## Journey C — Operation progress

After submit, the review becomes an operation detail page.

```text
Submitting votes
1 confirmed · 1 checking

✓ Eve Mainnet / dominguez.dash
  Vote confirmed: dominguez

… Backup Evo / dominguez.dash
  The vote was submitted. DET is checking the result.

                                      [ Continue in background ]
```

Target rows expose technical details only through the standard expandable
details affordance. If a transition hash is available, developer mode may show
and copy it.

## Journey D — Unconfirmed result

1. Broadcast succeeds.
2. Waiting for the result fails without a structured consensus cause.
3. The target becomes `Unconfirmed`; it is not labeled failed.
4. The exact node × contest target remains locked.
5. DET retries the result wait and/or fetches the proved current vote.
6. If the requested choice appears, the target becomes `Confirmed`.
7. If authoritative reconciliation proves it was not applied, the target
   becomes `Not applied` and offers `Submit again`.
8. If Platform remains unavailable, the persistent action is `Check again`.

Banner copy:

> The vote was submitted, but DET could not confirm the result yet. DET will
> keep checking. Do not submit it again.

## Journey E — Scheduled vote

- Scheduled targets appear immediately in `Masternodes > Scheduled`.
- Before execution begins, `Edit schedule` and `Cancel scheduled vote` remain
  available.
- At execution time, status changes from `Scheduled` to `Submitting`.
- A confirmed result becomes `Completed`.
- A definite rejection becomes `Needs attention`.
- An ambiguous result becomes `Checking result`; it is never automatically
  rebroadcast.

## Control state

| Target state | Choice controls | Submit action | Other targets |
|---|---|---|---|
| Draft | Enabled | Enabled when draft has changes | Enabled |
| Current vote unavailable | Disabled for that node | `Refresh vote state` | Enabled |
| Scheduled | Read-only for that target | `Edit schedule` | Enabled |
| Queued / Submitting / Confirming | Disabled | Spinner + status | Enabled |
| Unconfirmed | Disabled | `Check again` | Enabled |
| Confirmed | Enabled for a deliberate change | No draft action | Enabled |
| Rejected / Failed before submission / Not applied | Enabled after correction | `Review again` | Enabled |

The disabled tooltip names the exact reason, for example:

> This node's vote for dominguez.dash is still being confirmed.

## Feedback matrix

| Outcome | Type | Primary copy |
|---|---|---|
| One confirmed | Success | `Vote cast successfully.` |
| All batch targets confirmed | Success | `{count} votes were cast successfully.` |
| Scheduled | Success | `{count} votes were scheduled.` |
| Partial | Warning | `{confirmed} of {total} votes were confirmed. Review the remaining {remaining}.` |
| Unconfirmed | Warning, persistent | `The vote was submitted, but DET could not confirm the result yet. DET will keep checking. Do not submit it again.` |
| Structured rejection | Error | Typed, user-actionable rejection message |
| Failed before broadcast | Error | `This vote was not submitted. {action}` |
| No-op | Info | `This node already has that vote. Nothing was submitted.` |

## Navigation and persistence

- Operation progress is not owned by a screen instance.
- Leaving Masternodes never clears an active operation.
- Returning to any voting entry point reads current operation state and locks
  affected targets.
- On startup, DET restores scheduled and unresolved targets before voting
  controls become available.
- Switching networks swaps to that network's independent voting workspace.

## Accessibility and interaction

- Use styled buttons and semantic status colors with text labels.
- Minimum click targets follow the shared button component.
- `Enter` advances or submits only on the review step.
- `Escape` closes a draft review but cannot cancel a submitted operation.
- Focus moves to the step heading after Back/Next.
- Progress text and spinner are both present; no status is color-only.
- Disabled controls use the standard disabled tooltip policy.

## Responsive behavior

- Desktop: contest table and node/timing table use the full island panel.
- Narrow width: target rows become stacked cards showing Node, Contest, Current,
  Requested, Timing, and Status.
- Operation progress remains usable without horizontal scrolling.
