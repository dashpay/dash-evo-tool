# DPNS Voting Experience — Test Case Specification

## Authoritative state

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-001 | Current vote loads from Platform | Node has a proved Lock vote | Refresh Voting | Row shows `Current vote: Lock` | FR-010, FR-011 |
| VOTE-TC-002 | Existing vote remains visible | Node already voted; contest active | Open Active contests | Contest appears in Voted and change controls are available | FR-012 |
| VOTE-TC-003 | Current choice is a no-op | Current vote is Lock | Select Lock and review | Target is removed; nothing can be submitted | FR-013, FR-025 |
| VOTE-TC-004 | Coherent refresh | Contest tally and current vote both changed | Refresh | One snapshot shows both new values | FR-014 |
| VOTE-TC-005 | Vote query is per node | One node, 100 contests | Refresh | Identity-votes query runs once for the node, not 100 times | NFR-007 |
| VOTE-TC-006 | Vote change warning | Node has an existing vote | Select a different choice and review | Review says this uses a limited vote change without inventing a remaining count | FR-015 |
| VOTE-TC-007 | Vote query failure is not `Not voted` | Proved identity-votes query fails | Open voting | State is unavailable; affected submit controls are disabled | FR-016, NFR-004 |
| VOTE-TC-008 | Node summary distinguishes active and unvoted | Three active contests; node voted in two | View node card | Summary says three active and one needs a vote | FR-017 |

## Single-contest voting

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-010 | Single vote | Active contests, one draft choice | Review and submit | One target is created per selected loaded node for that contest | FR-020, FR-024 |
| VOTE-TC-011 | Multi-contest vote | Active contests, three draft choices | Review | Review shows the exact node × contest targets | FR-020, FR-031 |
| VOTE-TC-012 | Schedule one choice | Active contests, one draft | Choose Schedule in review | Targets appear in Scheduled with the chosen time | FR-023, FR-050 |
| VOTE-TC-013 | Missing voting key | No loaded node can vote | Open Active contests | Contest appears under Not votable and submit is unavailable | FR-003 |

## Bulk voting

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-020 | Multiple contests and nodes | Two contests, three nodes | Select all and review | Six exact targets are listed | FR-021, FR-024 |
| VOTE-TC-021 | Set all timing | Three selected nodes | Apply Schedule to all | All nodes receive the same schedule | FR-022, FR-023 |
| VOTE-TC-022 | Per-node override | Set all Cast now | Override one node to Schedule | Review reflects two Now and one Scheduled target | FR-022 |
| VOTE-TC-023 | Sticky review tray | Active contests visible | Select a choice | `Votes ready to cast: 1` appears and Review and cast opens in place | FR-002 |
| VOTE-TC-024 | Node navigation is plain | Node detail visible | Click `DPNS Voting` | Active contests opens without a node filter or carried draft | FR-003 |
| VOTE-TC-025 | Advanced node overrides | Three nodes loaded | Open Review and cast, expand advanced choices | All nodes default to Cast now and each can be overridden | FR-011, FR-021 |

## Execution correctness

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-030 | Same-node serialization | One node, three immediate targets | Submit | Nonce fetch/broadcast for target N+1 starts after N finishes submission | FR-033, NFR-002 |
| VOTE-TC-031 | Cross-node bounded concurrency | Four nodes, one target each | Submit | Different nodes run concurrently up to the configured bound | FR-033, NFR-007 |
| VOTE-TC-032 | Structured result correlation | Two nodes vote on same name; one fails | Complete operation | Result identifies the exact successful and failed node | FR-031 |
| VOTE-TC-033 | Scheduled inner error is not success | Scheduled backend returns an inner rejection | Execute | Target is Needs attention; record is not marked executed | FR-051, FR-052 |
| VOTE-TC-034 | Scheduled unconfirmed is not rebroadcast | Scheduled wait fails after broadcast | Run next sweep | Target remains Checking result; no second broadcast occurs | FR-053 |

## Duplicate prevention

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-040 | Double click | Submit enabled | Double-click Submit | Exactly one operation and one target broadcast are created | FR-034, FR-035 |
| VOTE-TC-041 | Cross-screen duplicate | Target is confirming | Return to Active contests | Same node × contest target is disabled with explanation | FR-034, FR-036 |
| VOTE-TC-042 | Unrelated target stays usable | One target confirming | Select another node or contest | Unrelated target remains enabled | Product decision 7 |
| VOTE-TC-043 | Navigation preserves lock | Submit, leave page, return before result | Inspect target | Progress and lock remain active | FR-036 |
| VOTE-TC-044 | Restart preserves lock | Persist unresolved target; restart | Open Voting | Target is restored and reconciled before resubmission is allowed | FR-037, NFR-003 |

## Confirmation and recovery

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-050 | Confirmed success | Broadcast and result wait succeed | Complete target | Status Confirmed; success banner shown | FR-043, FR-060 |
| VOTE-TC-051 | Structured rejection | Platform returns typed consensus cause | Complete target | Status Rejected with actionable typed message | FR-040 |
| VOTE-TC-052 | Cause-less wait failure | Broadcast succeeds; wait returns no cause | Complete target | Status Unconfirmed; warning forbids resubmission | FR-041, FR-044, FR-064 |
| VOTE-TC-053 | Reconcile to success | Unconfirmed target; proved vote matches request | Check again | Status changes to Confirmed without rebroadcast | FR-042, FR-043 |
| VOTE-TC-054 | Reconcile to safe retry | Unconfirmed target; definitive reconciliation proves absence | Check again | Status allows reviewed resubmission | FR-045 |
| VOTE-TC-055 | Reconciliation unavailable | DAPI remains unavailable | Check again | Target stays Unconfirmed and locked; no false failure/success | FR-044 |
| VOTE-TC-056 | Partial batch | Two confirmed, one unconfirmed, one rejected | Complete batch | Warning shows counts and details map every target | FR-061, FR-062 |

## Scheduling and migration

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-060 | Legacy schedule migration | Existing scheduled-vote records | Upgrade | Node, contest, choice, time, and executed state are preserved | FR-054 |
| VOTE-TC-061 | Due schedule uses shared coordinator | Scheduled target becomes due | Sweep | Same lock, result, and reconciliation model is used | FR-050 |
| VOTE-TC-062 | Failed schedule remains visible | Submission fails before broadcast | Open Scheduled | Needs attention row shows corrective action | FR-052 |
| VOTE-TC-063 | Scheduled target can be edited | Target is Scheduled, not due | Change time or choice | Updated target persists and keeps one lock | FR-055 |
| VOTE-TC-064 | Scheduled target can be cancelled | Target is Scheduled, not due | Cancel and confirm | Target is removed and its lock is released | FR-055 |
| VOTE-TC-065 | Submitting schedule cannot be edited | Target is Submitting | Inspect actions | Edit and Cancel are disabled with an explanation | FR-055 |

## UX, accessibility, and isolation

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-070 | Blocking submission feedback | Target submitting | Inspect screen | Full-window progress overlay remains visible until success or error | FR-035, NFR-005 |
| VOTE-TC-071 | Keyboard review | Review sheet open | Tab and activate controls | Focus order is logical and advanced node choices are reachable | NFR-005 |
| VOTE-TC-072 | Network isolation | Testnet target unresolved | Switch Mainnet | Mainnet has no Testnet locks or operation rows | NFR-008 |
| VOTE-TC-073 | No secret persistence | Operation stored | Inspect serialized operation | No private key or WIF bytes are present | NFR-009 |
| VOTE-TC-074 | Complete message units | All new copy | Localization audit | Strings are complete and do not parse technical errors | NFR-006 |
| VOTE-TC-075 | All entry points share one submit path | Quick, bulk, and due-scheduled drafts | Dispatch each | Every path creates a coordinator operation; no legacy direct-broadcast path remains | NFR-001 |
| VOTE-TC-076 | Technical error stays in details | Target rejected or fails | Inspect banner | Primary copy is plain language; typed diagnostic is attached as details | FR-063 |
