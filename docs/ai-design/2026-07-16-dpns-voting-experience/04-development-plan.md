# DPNS Voting Experience — Development Plan

Revised 2026-10-01. Sections up to "Message handling" describe what #901 built:
the safety model, which is **unchanged**. "2026-10-01 revision: Stream V work"
lists what changes. Identity usernames are Stream U
(`../2026-10-01-usernames-redesign/`). File ownership across streams:
"Shared backend and file ownership".

## Architecture

```text
Platform proved queries
        │
        ▼
DPNS vote-state store ────────┐
                              │
Draft / node set / composer ──> Vote operation coordinator
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
            shared operation/result views (Votes, drawer, chip, node detail)
```

Screens never own the authoritative in-progress flag. They render coordinator
state and submit typed drafts.

## Domain model (built in #901)

`src/model/dpns_voting.rs` holds the pure, serializable types:

- `DpnsVoteTargetKey { network, voter_id, vote_poll_id }`
- `DpnsVoteTarget { key, contested_name, requested_choice, current_choice, timing }`
- `DpnsVoteOperationId([u8; 16])`, generated with the existing RNG dependency
- `VoteTiming { Now, Scheduled(TimestampMillis) }`
- `DpnsVoteTargetStatus`: Scheduled, Queued, Submitting, Confirming, Confirmed,
  Unconfirmed, Rejected, FailedBeforeSubmission, NotApplied, Cancelled
- `DpnsVoteOutcome { operation_id, target, status, transition_hash, failure }`
- `DpnsVoteFailure`: a pure domain enum, mapped structurally from backend
  errors. It never serializes `TaskError` or secrets.

2026-10-01 additions (pure, same module):

- `NodeSet { All, EvonodesOnly, MasternodesOnly, Custom(BTreeSet<Identifier>) }`
  plus resolution against the loaded nodes, producing excluded nodes with typed
  reasons (`NoVotingKey`, `NoChangesLeft`, `NotInMasternodeList`,
  `VoteStateUnavailable`).
- `ChangesLeft { Known(u8), Unknown }`, computed from journal counts and proved
  state (VOTE-FR-078).
- `node_weight(node_type) -> u32` (1 or 4) and `influence(tally, node_set_weight)
  -> Option<Influence>` (VOTE-FR-077).
- `relative_schedule(end_time, preset) -> Result<TimestampMillis, …>`
  (VOTE-FR-081).
- `FailureReason::VotingEnded` for VOTE-FR-087.

## Data ownership (built in #901)

- `src/context/dpns_vote_state.rs`: proved votes per node via
  `ResourceVote::fetch_many` keyed by proTxHash (not the SDK stub). Indexed by
  node + poll and persisted per node scope.
- `src/context/dpns_vote_operations.rs`: journal and target locks. Persists
  before the first broadcast, restores on startup, and releases a lock only on a
  terminal state or on cancelling an unsubmitted schedule. Unconfirmed targets
  stay locked.

## Backend tasks (built in #901)

`ContestedResourceTask::SubmitDpnsVoteOperation`,
`ReconcileDpnsVoteOperation`, `CastDueScheduledVotes { … }`;
`BackendTaskSuccessResult::DpnsVoteOperationUpdated(id)`. The due-schedule sweep
runs on a timer of about 60 s in `AppState::update()`.

## Execution algorithm (unchanged)

1. Validate against proved state.
2. Remove no-ops.
3. Persist the operation and take locks atomically.
4. Group by node.
5. Bound concurrency across nodes, and run each node sequentially: nonce →
   build → persist hash → broadcast → wait → classify.
6. A cause-less wait failure becomes Unconfirmed and is reconciled, never
   rebroadcast.
7. Refresh vote state after each terminal outcome.

2026-10-01: before dispatching a queued target, re-check the contest end time.
If voting has ended, mark it `FailedBeforeSubmission(VotingEnded)`.

## Reconciliation (unchanged)

Resume by transition hash when dashpay/platform#4137 lands. Meanwhile use the
proved per-identity range query starting at the exact poll ID (#4138
workaround). Confirm only on an exact match, and stay Unconfirmed otherwise.

## Scheduling (built in #901)

Scheduled targets are journal operations with `VoteTiming::Scheduled`. Edit and
cancel are atomic under the lock. The dispatcher uses the shared executor. A
target counts as executed only when Confirmed. A schedule more than 120 s past
due and never admitted is marked missed.

## Message handling (unchanged)

One formatter over typed outcomes produces the summary, per-target copy,
details, and the recovery action. Never parse strings.

---

## 2026-10-01 revision: Stream V work

### Navigation and routing
- Masternodes root screen: add the `Votes | Nodes` segmented header and default
  segment logic (VOTE-FR-070). Votes renders the existing DPNS voting screen
  body: move `ui/dpns/` content under the Masternodes root, or embed
  `DPNSScreen` as the Votes segment. Either way there is one screen instance
  and one contest cache.
- Remove the DPNS item from the Tools chooser (`tools_subscreen_chooser_panel.rs`).
  Replace `dpns_subscreen_chooser_panel.rs` with the in-page sub-view chips
  `To decide · Voted · Scheduled · History`.
- Remove the `Owned` sub-screen and its table and "Set Alias" code from
  `ui/dpns/`.
- Routing in `ui/mod.rs`: `RootScreenDPNS{Active,Past,Scheduled}` map to the
  Masternodes root with the matching sub-view. `RootScreenDPNSOwnedNames` maps
  to `RootScreenIdentityHub`. Keep the enum values for persisted settings.
- `left_panel.rs`: Masternodes badge count from the attention summary.

### Attention signal
- New `src/context/dpns_vote_attention.rs`: an `AttentionSummary { needs_decision,
  soonest_end, unresolved }` derived from the contest cache, vote state, node
  set and journal. It is recomputed on refresh and on coordinator updates, never
  per frame.
- Background refresh (VOTE-FR-074): a timer in `AppState::update()` beside the
  due-schedule sweep. It dispatches the existing contest query plus the vote
  state refresh while ≥ 1 voting node is loaded. Cadence comes from network.
- `ui/components/top_panel.rs`: render the attention chip(s) next to the network
  chip (VOTE-FR-072), gated by `FeatureGate::Masternodes` and voting-node
  presence.

### Votes screen
- Cards: two-column layout with weighted tally (read-only), influence line,
  node line, checkbox, and choice pills with key hints. Sorted by time left.
  Groups per VOTE-FR-071.
- Node-set chip and popover (VOTE-FR-075). The preference persists per network
  in local KV (new key in the vote-state context or settings).
- Keyboard handling scoped to list focus, plus the bulk bar (VOTE-FR-082).
- Tray copy (VOTE-FR-086).
- Needs-attention row (VOTE-FR-084).

### Confirm and drawer
- Replace the review sheet with the aggregate confirm popover (VOTE-FR-080). The
  existing per-node matrix becomes the `Adjust nodes` table. The timing control
  gains `When voting is about to end`.
- Replace the full-window progress overlay for vote submissions with the
  progress drawer (VOTE-FR-083), driven by coordinator snapshots. It persists
  across screens and is rendered at AppState level like other global overlays.
- Final banner per VOTE-FR-061.

### Changes left and node status
- Journal query: count confirmed votes per (proTxHash, poll). Combine it with
  proved state to get `ChangesLeft` (VOTE-FR-078). Exclude nodes with
  `Known(0)` during draft expansion.
- Masternode-list membership per node (VOTE-FR-079): read from the masternode
  list already available via SPV/DAPI if present. Otherwise add a lightweight
  `in_list` refresh on the existing masternode refresh path (MN-011). **Needs
  investigation**; flag as a backend gap if no source exists.

### Node detail
- Replace the `DPNS Voting` button with `Vote with this node`, which sets
  NodeSet::Custom({node}) for the session, not as a saved default.
- Add the `This node's votes` table from the vote-state store and the journal
  (VOTE-FR-076).

### Scheduled and History
- Scheduled view: group by (poll, choice, scheduled time) with an expandable node
  list (VOTE-FR-088). Show the relative label when the schedule was created
  relative (store a `relative_preset: Option<Duration>` alongside the absolute
  time for display only).
- History: aggregate `Your nodes voted` with weight.

### Durations
Consume `model/dpns.rs` contest-duration and urgency helpers (owned by Stream U,
see below) for countdowns, presets and the chip.

## Shared backend and file ownership

| File / op | Owner | Consumer | Note |
|---|---|---|---|
| `model/dpns.rs`: contested-name rule (delegating to the contract's `field_matches`), contest/join durations per network, urgency window | **U** | V | Lands first. Replaces both `is_contested_name` copies. |
| `model/fee_estimation.rs`: contest fee from `sdk.version()` / `prefunded_voting_balance_for_document` | **U** | — | Repo rule: fee math only here |
| `model/dpns_voting.rs` (+ NodeSet, ChangesLeft, influence, relative schedule) | **V** | — | |
| `context/contested_names_db.rs`: pending usernames → `Vec`, outcomes | **U** | — | V doesn't edit it; V reads contests via the existing API |
| `context/dpns_vote_state.rs`, `dpns_vote_operations.rs`, new `dpns_vote_attention.rs` | **V** | — | |
| `backend_task/contested_names/*` | **V** | U reads `get_contested_dpns_vote_state` via its own new op | U adds no code here |
| New `backend_task/identity/dpns_usernames.rs` (`CheckUsernameAvailability`, `RefreshMyUsernameRequests`) | **U** | — | |
| `backend_task/mod.rs` (`BackendTaskSuccessResult` variants) | both | — | Each stream adds its variants in its own contiguous block (V first, U after). Rebase conflict expected and trivial. |
| `app.rs` timers | **V** only | — | U triggers its refresh from the hub screen, not app.rs |
| `ui/mod.rs` routing, `left_panel.rs`, `top_panel.rs`, `tools_subscreen_chooser_panel.rs`, `ui/dpns/*`, `ui/masternodes/*` | **V** | — | |
| `ui/identity/*` (register screen, settings/profile, home, hero card, checklist, request card) | **U** | — | U removes the "View all usernames" link; V removes the Owned sub-screen |
| `docs/user-stories.md` | both | — | Disjoint stories (V: DPN-003…007, 011, MN-003/011; U: DPN-001/002/008/010, new DPN-012+) |

## Implementation workstreams (one PR, #901)

The A–D workstreams from #901 are done. New:

### Workstream E — Stream V convenience (this revision)
- E1 Navigation, routing, Tools removal, owned-names exit. Tests VOTE-TC-080…084.
- E2 Attention summary, background refresh, chip, badge. Tests VOTE-TC-085…087,
  104.
- E3 Cards, sorting, groups, node set, influence, changes left, list
  exclusion. Tests VOTE-TC-002, 007, 088…097.
- E4 Aggregate confirm, relative scheduling, Adjust nodes. Tests VOTE-TC-006,
  010–025, 067–069.
- E5 Progress drawer, needs-attention row, voting-ended failure. Tests
  VOTE-TC-041, 056, 100–103.
- E6 Keyboard and bulk bar. Tests VOTE-TC-098, 099.
- E7 Node detail table and `Vote with this node`. Tests VOTE-TC-091, 094.

Stream U runs in parallel (plan: `../2026-10-01-usernames-redesign/01-requirements.md`
§Implementation plan). It must merge `model/dpns.rs` before E2/E4 consume the
duration helpers. Until then, V may stub them behind the same function
signatures.

## Verification
- Unit: NodeSet resolution, ChangesLeft, influence, relative schedule, attention
  summary, draft expansion with exclusions.
- Backend (fake SDK): voting-ended failure, out-of-changes skip, per-node query
  count.
- Kittest: segments and routing, chip, keyboard, bulk, confirm, drawer
  non-blocking, node detail.
- Testnet backend E2E (manual, `#[ignore]`): 2 nodes × 2 contests with a relative
  schedule in a 90-minute contest.
- `cargo fmt --all`; clippy scoped per AGENTS.md.

## Documentation updates
- `docs/user-stories.md`: revise DPN-003…007, DPN-011, MN-003, MN-011; add
  DPN-013 (attention chip), DPN-014 (node set + weight + changes left), DPN-015
  (non-blocking progress).
- Remove references to "DPNS ▸ Active contests" in docs and in-app copy.
