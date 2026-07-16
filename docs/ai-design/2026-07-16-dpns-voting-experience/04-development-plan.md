# DPNS Voting Experience — Development Plan

## Architecture

```text
Platform proved queries
        │
        ▼
DPNS vote-state store ────────┐
                              │
Draft / shared composer ──> Vote operation coordinator
                              │
                              ├─ durable operation journal
                              ├─ target lock registry
                              ├─ scheduled dispatcher
                              └─ immediate executor
                                      │
                     group by node ───┤
                     sequential/node  │
                     bounded nodes    ▼
                              Platform broadcast + wait
                                      │
                                      ▼
                              reconciliation service
                                      │
                                      ▼
                         shared operation/result views
```

Screens never own the authoritative in-progress flag. They render coordinator
state and submit typed drafts.

## Domain model

Add `src/model/dpns_voting.rs` with pure, serializable types:

```rust
struct DpnsVoteTargetKey {
    network: Network,
    voter_id: Identifier,
    vote_poll_id: Identifier,
}

struct DpnsVoteTarget {
    key: DpnsVoteTargetKey,
    contested_name: String,
    requested_choice: ResourceVoteChoice,
    current_choice: Option<ResourceVoteChoice>,
    timing: VoteTiming,
}

struct DpnsVoteOperationId([u8; 16]);

enum VoteTiming {
    Now,
    Scheduled(TimestampMillis),
}

enum DpnsVoteTargetStatus {
    Scheduled,
    Queued,
    Submitting,
    Confirming,
    Confirmed,
    Unconfirmed,
    Rejected,
    FailedBeforeSubmission,
    NotApplied,
}

struct DpnsVoteOutcome {
    operation_id: DpnsVoteOperationId,
    target: DpnsVoteTarget,
    status: DpnsVoteTargetStatus,
    transition_hash: Option<[u8; 32]>,
    failure: Option<DpnsVoteFailure>,
}
```

Generate `DpnsVoteOperationId` with the project's existing random-number
dependency; no new UUID dependency is required.

`DpnsVoteFailure` is a pure domain enum, not a wrapper around `TaskError`.
Backend errors are mapped into it structurally. The stored form uses
serde-friendly representations and never serializes `TaskError` or secrets.
Full errors remain in task diagnostics and logs.

## Data ownership

### Authoritative current votes

Add `src/context/dpns_vote_state.rs`.

- Query `ResourceVote::fetch_many` once per loaded node using its ProTxHash.
- Index results by node + vote-poll ID.
- Persist the latest proved snapshot in the node's identity scope.
- Join vote state with global contest data when building UI view models.
- Stop using `ContestedName::my_votes` as an implied persistent source. Remove
  it or populate it only in an explicitly transient joined view.

This fixes the current false `Not voted` state and supports deliberate changes.

### Operation journal and locks

Add `src/context/dpns_vote_operations.rs`.

- Persist an operation before the first broadcast.
- Maintain target locks keyed by network + node + poll.
- Restore unresolved operations and locks on startup.
- Expose read-only snapshots to every UI surface.
- Release a lock only on Confirmed, Rejected, FailedBeforeSubmission,
  NotApplied, or explicit cancellation of a not-yet-submitting schedule.
- Keep Unconfirmed targets locked.

Use per-object KV records and a network-scoped index, following current DET KV
patterns.

## Backend tasks

Replace tuple-heavy vote tasks/results with structured variants:

```rust
ContestedResourceTask::SubmitDpnsVoteOperation(DpnsVoteOperation)
ContestedResourceTask::ReconcileDpnsVoteOperation(DpnsVoteOperationId)
ContestedResourceTask::DispatchDueDpnsVotes

BackendTaskContext::DpnsVoteOperation(DpnsVoteOperationId)
BackendTaskSuccessResult::DpnsVoteOperationUpdated(DpnsVoteOperationId)
```

The backend persists each target update in the shared coordinator. Task results
carry only the operation ID needed for AppState to request repaint and show a
summary. AppState never delivers raw vote outcomes to whichever screen happens
to be visible.

## Execution algorithm

1. Validate the draft against current proved state.
2. Remove exact no-ops.
3. Persist the operation and acquire target locks atomically.
4. Group immediate targets by voter/node.
5. Run different voter groups with a small semaphore.
6. Within each voter group, execute targets sequentially:
   - read the current nonce;
   - construct and validate the transition;
   - persist transition hash when the SDK exposes it;
   - broadcast;
   - wait for result;
   - classify the typed outcome.
7. On cause-less post-broadcast wait failure, mark Unconfirmed and enqueue
   reconciliation. Do not rebroadcast.
8. Refresh authoritative vote state after each terminal outcome.

This replaces the current `join_all` by contest, which can race the same voter
nonce.

## Reconciliation

Preferred path:

1. Resume `waitForStateTransitionResult` by persisted transition hash after
   [dashpay/platform#4137](https://github.com/dashpay/platform/issues/4137)
   exposes a phase-specific retryable wait error.
2. Independently fetch the proved vote using `VoteQuery` or the per-node votes
   query.
3. Confirm when the proved choice matches the request.
4. Keep the target Unconfirmed while neither path is definitive.
5. Permit resubmission only after the Platform contract defines a definitive
   negative result. Do not infer safety from a transient query failure.

The existing generic `PlatformResultUnconfirmed` classification remains useful,
but vote operations convert it into target-level coordinator state instead of a
screen-local banner.

## Scheduling

Migrate `ScheduledDPNSVote` into the shared target model:

- scheduled targets are persisted operations with `VoteTiming::Scheduled`;
- edit and cancel mutate the scheduled target atomically while it still holds
  its target lock;
- the due dispatcher moves them to Queued and uses the same executor;
- executed state means Confirmed, not merely outer task success;
- rejected, failed, and unconfirmed outcomes remain inspectable;
- the migration is idempotent and preserves legacy records.

Do not run immediate casting and schedule persistence as unrelated concurrent
backend tasks. One operation owns both kinds of targets.

## UI state and components

### Non-rendering state

Add `src/ui/state/dpns_vote_workspace.rs` for:

- draft contest choices;
- selected nodes and timing;
- current composer step;
- validation and no-op explanations;
- conversion to a typed operation request.

### Shared rendering

Add `src/ui/components/dpns_vote_composer.rs` implementing the three steps from
the UX specification.

The compact node-detail controls use the same draft/view-model logic and open
the shared review step. They do not implement a separate submit path.

### Masternodes views

Extend the Masternodes root state with:

- Nodes
- Voting
- Scheduled
- Operation detail

The root observes coordinator snapshots, so progress survives sub-view changes.

### DPNS integration

Replace the legacy bulk popup with `Vote with masternodes`, routing selected
contests into the shared Voting workspace. Keep Active, Past, and My Usernames
contest/name browsing in DPNS.

## Message handling

Create one vote-specific formatter over typed target outcomes. It produces:

- banner summary;
- per-target plain-language status;
- technical details attachment;
- recovery action (`Check again`, `Review again`, or none).

Never parse error strings. Unconfirmed outcomes never offer an immediate retry.

## Implementation sequence

### PR A — Authoritative state and typed models

- Add domain types and vote-state store.
- Query proved votes per node.
- Fix active-contest view models to show current vote and allow changes.
- Update user stories: current vote visibility and vote changes.
- Covers VOTE-TC-001 through VOTE-TC-008.

### PR B — Coordinator and safe executor

- Add operation journal, locks, structured task context/results.
- Serialize targets per node; bound concurrency across nodes.
- Add post-broadcast unconfirmed classification and reconciliation seam.
- Fix scheduled false-success behavior at the executor boundary.
- Covers VOTE-TC-030 through VOTE-TC-056.

### PR C — Shared Voting Center

- Add shared composer and Masternodes Voting view.
- Integrate quick node flow.
- Add operation detail/progress.
- Route DPNS Active Contests into the shared workspace.
- Covers VOTE-TC-010 through VOTE-TC-025 and VOTE-TC-070 through VOTE-TC-076.

### PR D — Scheduled consolidation and migration

- Migrate existing schedules into operation targets.
- Replace legacy scheduled execution/status UI.
- Remove obsolete popup/state code after migration coverage passes.
- Covers VOTE-TC-060 through VOTE-TC-065.

Each PR is independently testable and must not expose two active submit
implementations for the same stage.

## Verification

- Unit tests for draft expansion, no-op removal, status transitions, locks, and
  storage migration.
- Backend tests with fake SDK seams for nonce ordering and all result classes.
- Kittest coverage for quick, bulk, navigation, disabled-state, and result UX.
- Backend E2E on Testnet for one-node multi-contest and two-node same-contest
  batches.
- Restart test with a persisted Unconfirmed operation.
- Formatter and clippy per repository policy.

## Documentation updates

- Revise DPN-005, DPN-006, DPN-007, and MN-003 acceptance criteria.
- Correct the protocol note to five votes total: initial vote plus four changes.
- Add a user story for operation recovery across navigation/restart.
- Replace the previous Masternodes design decision that made scheduled voting
  undiscoverable from the operator page.
