# DPNS Voting Experience — Requirements

## Status

Planning specification. No implementation is authorized by this document.

## Problem statement

DET exposes DPNS voting through two disconnected experiences:

- a quick per-node section on the Masternodes detail page; and
- a legacy DPNS bulk dialog for multiple contests, nodes, and schedules.

Both call the same backend, but neither owns a complete, authoritative model of
the operation. The result is unsafe ambiguity: a vote can be accepted while DET
shows no confirmation, the same choice can immediately be offered again, bulk
results cannot identify the affected node, and a second click can submit another
state transition while the first is unresolved.

Voting consumes Platform credits and each node may vote only five times on a
contest in total (the initial vote plus up to four changes). Duplicate or
ambiguous submissions therefore have a real cost.

## Primary persona

Priya, the power-user masternode operator, manages one or more nodes and expects:

- current vote state to be accurate;
- quick actions for one node;
- bulk and scheduled operations for many nodes;
- progress that survives navigation and restart;
- exact per-node results; and
- safe recovery when Platform accepted a request but DET could not confirm it.

The voting workspace remains hidden below Detailed interface mode. It is not an
Everyday User workflow.

## Current-state audit

| Finding | Source behavior | User impact |
|---|---|---|
| Vote ownership is never populated | Contest cache reconstructs `ContestedName::my_votes` as empty and no task fills it | A successful vote is offered again after refresh |
| Existing votes are treated as closed | `is_open_for_voter` excludes any contest where the node already voted | Operators cannot inspect or change an existing vote |
| Same-node bulk execution races | `VoteOnDPNSNames` runs contests with `join_all`; each task independently fetches the same voter nonce | Multiple votes for one node can conflict |
| Bulk results lose node identity | `DPNSVoteResults` contains only name, choice, and result | Partial results cannot say which node succeeded |
| Scheduled casts can report false success | Scheduled paths treat an outer `Ok(DPNSVoteResults)` as success without inspecting inner failures | A failed or unconfirmed scheduled vote is marked executed |
| Progress is screen-owned | App results are routed to the currently visible screen | Navigation can strand controls in progress or deliver feedback to the wrong page |
| Duplicate prevention is local or absent | Quick and bulk submit buttons do not share an operation lock | Repeated clicks or cross-screen actions can submit duplicates |
| Post-broadcast wait failure is ambiguous | A cause-less `StateTransitionBroadcastError` can follow successful broadcast | DET must not label it rejected or invite immediate retry |
| Legacy bulk workflow is disconnected | Bulk and scheduling live under DPNS while node management lives under Masternodes | Operators can miss existing capabilities or assume they were removed |

## Product decisions

1. Masternodes is the primary home for operator voting.
2. DPNS remains the home for name registration, contest discovery, and contest
   history, with a route into the shared voting workspace.
3. Quick voting and bulk/scheduled voting use one shared composer and one shared
   operation coordinator.
4. Current vote state is authoritative Platform data, not UI-local memory.
5. A vote row remains visible after voting and shows the current choice. Voting
   again is presented as a change, not a new missing vote.
6. Operation state is global, correlated, and durable enough to survive
   navigation and process restart.
7. Exact targets, not whole screens, are locked. Unrelated nodes and contests
   remain usable.

## Functional requirements

### Information architecture

- **VOTE-FR-001** — Masternodes provides `Nodes`, `Voting`, and `Scheduled`
  views under one operator-focused root.
- **VOTE-FR-002** — The DPNS Active Contests page links to the same Voting view;
  it does not maintain a second bulk implementation.
- **VOTE-FR-003** — A node detail page offers quick voting and an `Open Voting
  Center` action pre-filtered to that node.

### Authoritative state

- **VOTE-FR-010** — DET queries each loaded node's proved Platform votes and
  joins them with active contests.
- **VOTE-FR-011** — Each active contest shows the node's current choice, or
  `Not voted`.
- **VOTE-FR-012** — Existing votes remain actionable while the contest is
  votable, allowing a deliberate vote change.
- **VOTE-FR-013** — Selecting the already-current choice is a no-op and cannot
  create a state transition.
- **VOTE-FR-014** — Refresh updates contests, tallies, and node vote state as one
  coherent snapshot.
- **VOTE-FR-015** — A change to an existing vote is labeled as a limited vote
  change in Review. DET does not claim to know the remaining count.
- **VOTE-FR-016** — If current vote state cannot be proved, DET shows it as
  unavailable and disables submission for that node instead of assuming
  `Not voted`.
- **VOTE-FR-017** — Node summaries distinguish active contests from contests
  where the node has not voted yet.

### Vote composition

- **VOTE-FR-020** — Quick voting supports one node across one or more contests.
- **VOTE-FR-021** — Bulk voting supports one or more contests across one or more
  nodes.
- **VOTE-FR-022** — The operator can apply one timing choice to all selected
  nodes and override individual nodes.
- **VOTE-FR-023** — Timing choices are `Cast now`, `Schedule`, and `Do not use
  this node`.
- **VOTE-FR-024** — Before submission, a review step lists every target as
  node × contest, including current choice, requested choice, and timing.
- **VOTE-FR-025** — The review step removes no-op targets and explains why.

### Operation lifecycle

- **VOTE-FR-030** — Every submitted batch has a stable operation ID.
- **VOTE-FR-031** — Every target result includes operation ID, node ID, contest
  ID/name, requested choice, and typed status.
- **VOTE-FR-032** — Target statuses are `Scheduled`, `Queued`, `Submitting`,
  `Confirming`, `Confirmed`, `Unconfirmed`, `Rejected`, and
  `Failed before submission`, plus `Not applied` after definitive
  post-broadcast reconciliation.
- **VOTE-FR-033** — Same-node targets execute sequentially to preserve nonce
  order. Different nodes may execute concurrently with a fixed bound.
- **VOTE-FR-034** — A target lock prevents a second operation for the same
  network + node + contest while the first is unresolved.
- **VOTE-FR-035** — Button state derives from the shared coordinator. A click
  disables the affected action immediately and shows progress text.
- **VOTE-FR-036** — Navigation does not cancel an operation or lose its state.
- **VOTE-FR-037** — Restart restores scheduled and unresolved operations before
  enabling conflicting actions.

### Confirmation and recovery

- **VOTE-FR-040** — Structured Platform consensus causes are treated as
  confirmed rejection.
- **VOTE-FR-041** — A cause-less post-broadcast wait failure is treated as
  `Unconfirmed`, never as rejection.
- **VOTE-FR-042** — Unconfirmed targets are reconciled against the proved
  current vote and, when available, retried by transition hash through the
  Platform SDK.
- **VOTE-FR-043** — A target becomes `Confirmed` when authoritative state
  matches the requested choice.
- **VOTE-FR-044** — DET never offers `Submit again` while the result remains
  ambiguous. It offers `Check again`.
- **VOTE-FR-045** — A retry becomes available only after authoritative
  reconciliation proves the requested change was not applied.

### Scheduled votes

- **VOTE-FR-050** — Scheduled votes use the same target model, result model,
  locking, execution order, and reconciliation as immediate votes.
- **VOTE-FR-051** — A scheduled target is marked executed only after confirmed
  application.
- **VOTE-FR-052** — Rejected and failed-before-submission targets remain visible
  with an actionable status.
- **VOTE-FR-053** — Unconfirmed scheduled targets are not automatically
  rebroadcast.
- **VOTE-FR-054** — Existing scheduled-vote records migrate without losing node,
  contest, choice, time, or executed state.
- **VOTE-FR-055** — A scheduled target can be edited or cancelled until
  execution begins. Once submitting, it follows normal operation locking.

### Feedback

- **VOTE-FR-060** — One confirmed target shows a concise success banner.
- **VOTE-FR-061** — Batch feedback summarizes confirmed, unconfirmed, rejected,
  and failed counts and links to per-target details.
- **VOTE-FR-062** — Messages name node aliases and contested names where useful.
- **VOTE-FR-063** — Technical errors stay in banner details.
- **VOTE-FR-064** — Unconfirmed copy explicitly says DET will keep checking and
  warns against resubmission.

## Non-functional requirements

- **VOTE-NFR-001 Safety** — No UI path can bypass target locking.
- **VOTE-NFR-002 Correctness** — Same-voter transitions are serialized.
- **VOTE-NFR-003 Durability** — A crash after broadcast cannot erase the only
  record that the outcome is unresolved.
- **VOTE-NFR-004 Proofs** — Current vote state is obtained through proved SDK
  queries.
- **VOTE-NFR-005 Accessibility** — Disabled actions explain why; progress is not
  color-only; keyboard focus follows the composer step order.
- **VOTE-NFR-006 Localization** — User-facing strings are complete translation
  units with no parsed error text.
- **VOTE-NFR-007 Performance** — Refresh queries votes once per node, not once
  per contest, and bounds cross-node concurrency.
- **VOTE-NFR-008 Network isolation** — Drafts, schedules, operations, locks, and
  results are network-scoped.
- **VOTE-NFR-009 Secret handling** — The coordinator stores identifiers and
  choices, never private keys.

## Platform dependency

The preferred recovery contract is
[dashpay/platform#4137](https://github.com/dashpay/platform/issues/4137), which
tracks phase-specific, retryable post-broadcast wait errors. The SDK should
expose the transition hash after broadcast so DET can persist it and resume
waiting.

DET must still support fallback reconciliation by fetching the node's proved
vote for the target poll. Until either method proves the result, the operation
remains unconfirmed and locked against duplicate submission.

## Out of scope

- Changing Platform's five-vote protocol limit.
- Showing a remaining-change count before Platform exposes it in the proved
  identity-votes response.
- Embedding Platform Explorer.
- Allowing arbitrary cancellation after broadcast.
