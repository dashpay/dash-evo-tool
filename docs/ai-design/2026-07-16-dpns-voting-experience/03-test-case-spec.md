# DPNS Voting Experience — Test Case Specification

Stream V. Revised 2026-10-01. Markers as in `01-requirements.md`. Level: U = unit,
K = kittest, B = backend test with fake SDK seam.

## Authoritative state

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-001 | Current vote loads from Platform | Node has a proved Lock vote | Refresh voting | Node line shows the node voted Lock | FR-010, FR-011 |
| VOTE-TC-002 | Existing vote remains visible | Node already voted; contest active | Open Votes | Contest in Voted (or To decide if other node-set nodes have not voted); proved choice highlighted; change possible | FR-012, FR-071 |
| VOTE-TC-003 | Current choice is a no-op | Current vote is Lock | Select Lock, open confirm | Counted under Skipped; nothing submittable for that target | FR-013, FR-025 |
| VOTE-TC-004 | Coherent refresh | Tally and current vote changed | Refresh | One snapshot shows both | FR-014 |
| VOTE-TC-005 | Vote query is per node | One node, 100 contests | Refresh | One identity-votes query for the node | NFR-007 |
| VOTE-TC-006 **[Changed]** | Change warning once | 3 nodes with existing votes | Pick a different choice, open confirm | One line `3 of these change an earlier vote…`; no per-node repetition | FR-015, FR-080 |
| VOTE-TC-007 | Vote query failure is not `Not voted` | Proved query fails for 1 of 3 nodes | Open Votes | Card says unavailable for 1 node; that node excluded; others submittable | FR-016 |
| VOTE-TC-008 | Node summary distinguishes active and unvoted | 3 active; node voted in 2 | View node card | Summary: three active, one needs a vote | FR-017 |

## Composition

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-010 | Single decision | Node set = 3 nodes, one choice | Cast → confirm → Cast | One target per node for that contest | FR-020, FR-075 |
| VOTE-TC-011 **[Changed]** | Multi-contest aggregate | 3 choices, 3 nodes | Open confirm | Title `Cast 3 decisions with 3 nodes`; `9 transactions…`; Adjust nodes lists 9 rows | FR-024, FR-080 |
| VOTE-TC-012 | Schedule at a specific time | One draft | Choose At a specific time | Targets appear in Scheduled with that UTC time | FR-023, FR-050 |
| VOTE-TC-013 **[Changed]** | No voting key | Nodes loaded, none with voting key | Open Votes | Gate copy + `Add a voting key`; no submit | FR-075, V5 |
| VOTE-TC-014 | Voting key does not match node | Wrong voting key | Submit load | Key-specific rejection; existing keys unchanged | FR-065 |
| VOTE-TC-020 | Multiple contests and nodes | 2 contests, 3 nodes | Confirm | 6 targets in Adjust nodes | FR-021, FR-024 |
| VOTE-TC-021 **[Changed]** | Batch timing | 3 nodes | Choose `When voting is about to end` | All targets scheduled at end − preset | FR-022, FR-081 |
| VOTE-TC-022 | Per-node override | Batch Now | Set one node to Before the end in Adjust nodes | Two Now, one Scheduled target | FR-022 |
| VOTE-TC-023 **[Changed]** | Tray | 3 nodes | Pick a choice | Tray `1 decision ready · 3 transactions…` | FR-086 |
| ~~VOTE-TC-024~~ | ~~Node navigation is plain~~ | | | **[Superseded → VOTE-TC-091]** | |
| VOTE-TC-025 **[Changed]** | Adjust nodes defaults | 3 nodes | Expand Adjust nodes | All rows default to batch timing; each overridable incl. Don't use this node | FR-022, FR-023 |

## Execution correctness (unchanged)

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
| VOTE-TC-040 | Double click | Confirm open | Double-click Cast | One operation, one broadcast per target | FR-034, FR-035 |
| VOTE-TC-041 | Cross-screen duplicate | Target confirming | Return to Votes | Card shows `Sending with {n} nodes…`, choices locked for those targets | FR-034, FR-083 |
| VOTE-TC-042 | Unrelated target stays usable | One target confirming | Pick another contest | Enabled | Decision 7 |
| VOTE-TC-043 | Navigation preserves lock | Submit, navigate, return | Inspect | Drawer and lock persist | FR-036 |
| VOTE-TC-044 | Restart preserves lock | Unresolved target persisted | Restart | Restored and reconciled before resubmission | FR-037, NFR-003 |

## Confirmation and recovery

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-050 | Confirmed success | Broadcast and result wait succeed | Complete target | Status Confirmed; success banner shown | FR-043, FR-060 |
| VOTE-TC-051 | Structured rejection | Platform returns typed consensus cause | Complete target | Status Rejected with actionable typed message | FR-040 |
| VOTE-TC-052 | Cause-less wait failure | Broadcast succeeds; wait returns no cause | Complete target | Status Unconfirmed; warning forbids resubmission | FR-041, FR-044, FR-064 |
| VOTE-TC-053 | Reconcile to success | Unconfirmed target; proved vote matches request | Check again | Status changes to Confirmed without rebroadcast | FR-042, FR-043 |
| VOTE-TC-054 | Reconcile to safe retry | Unconfirmed target; definitive reconciliation proves absence | Check again | Status allows reviewed resubmission | FR-045 |
| VOTE-TC-055 | Reconciliation unavailable | DAPI remains unavailable | Check again | Target stays Unconfirmed and locked; no false failure/success | FR-044 |
| VOTE-TC-056 **[Changed]** | Partial batch | Two confirmed, one unconfirmed, one rejected | Complete batch | Drawer maps every target; final banner shows counts | FR-061, FR-062, FR-083 |
| VOTE-TC-057 | Journal read failure stays visible until retried | Saved vote-operation progress cannot be read | Open Votes ▸ Scheduled, then click `Retry loading` once the read succeeds | Persistent notice and `Retry loading` remain visible across renders until a successful refresh, then clear | FR-046 |

## Scheduling and migration

| ID | Description | Preconditions | Steps | Expected outcome | Requirements |
|---|---|---|---|---|---|
| VOTE-TC-060 | Legacy schedule migration | Existing scheduled-vote records | Upgrade | Node, contest, choice, time, and executed state are preserved | FR-054 |
| VOTE-TC-061 | Due schedule uses shared coordinator | Scheduled target becomes due | Sweep | Same lock, result, and reconciliation model is used | FR-050 |
| VOTE-TC-062 | Failed schedule remains visible | Submission fails before broadcast | Open Scheduled | Needs attention row shows corrective action | FR-052, FR-084 |
| VOTE-TC-063 | Scheduled target can be edited | Target is Scheduled, not due | Change time or choice | Updated target persists and keeps one lock | FR-055 |
| VOTE-TC-064 | Scheduled target can be cancelled | Target is Scheduled, not due | Cancel and confirm | Target is removed and its lock is released | FR-055 |
| VOTE-TC-065 | Submitting schedule cannot be edited | Target is Submitting | Inspect actions | Edit and Cancel are disabled with an explanation | FR-055 |
| VOTE-TC-066 | Missed automatic vote is explained | Due time > 120 s past, not executed | Reopen Scheduled | Row shows `Missed` with `Cast now`, `Edit`, `Remove` | FR-056 |
| VOTE-TC-067 **[New]** | Relative schedule resolves | Mainnet contest ends T | Schedule `When voting is about to end` | Stored time = T − 6 h; row shows `6 hours before the end · {abs UTC}` | FR-081 |
| VOTE-TC-068 **[New]** | Testnet preset | Testnet contest | Same | T − 10 min | FR-081, FR-085 |
| VOTE-TC-069 **[New]** | Grouped scheduled rows | One decision on 24 nodes | Open Scheduled | One row `24 nodes ▾`; expansion lists 24 statuses | FR-088 |

## Convenience (new)

| ID | Description | Preconditions | Steps | Expected outcome | Requirements | Lvl |
|---|---|---|---|---|---|---|
| VOTE-TC-080 | Votes first | Badge > 0 | Open Masternodes | Votes segment active | FR-070 | K |
| VOTE-TC-081 | Last used when nothing pending | Badge 0, last segment Nodes | Open Masternodes | Nodes active | FR-070 | K |
| VOTE-TC-082 | Legacy routes | Persisted `RootScreenDPNSScheduledVotes` | Start app | Votes ▸ Scheduled opens | FR-070 | K |
| VOTE-TC-083 | Owned-names route | Persisted `RootScreenDPNSOwnedNames` | Start app | Identity hub opens | FR-070 | K |
| VOTE-TC-084 | Tools has no DPNS | Any | Open Tools | No DPNS entry | FR-070 | K |
| VOTE-TC-085 | Attention chip urgent | Power, voting node, contest ends in 3 h (mainnet) | Any screen | Amber chip `1 name needs your vote · first ends in 3 hours`; click opens Votes | FR-072, FR-085 | K |
| VOTE-TC-086 | Chip hidden | Below Power role, or no voting key | Any screen | No chip | FR-072 | K |
| VOTE-TC-087 | Unresolved chip | One Unconfirmed target | Any screen | `1 vote is still being checked` chip | FR-072 | K |
| VOTE-TC-088 | Sort by time left | Contests ending in 4 d, 6 h, 12 min | To decide | Order 12 min, 6 h, 4 d | FR-071 | K |
| VOTE-TC-089 | Partly voted stays | 5 of 24 nodes voted | To decide | Card present with `Voted with 5 of 24 nodes` | FR-071 | K |
| VOTE-TC-090 | Node set persists per network | Choose Evonodes only, save | Restart; switch network | Evonodes only restored on that network only | FR-075, NFR-008 | U+K |
| VOTE-TC-091 | Vote with this node | Node detail | Click | Votes with node-set chip naming that node; no draft carried | FR-076 | K |
| VOTE-TC-092 | Weighted influence | Leader margin 6; node set weight 51 | Render card | Influence line shown; hidden when weight < margin | FR-077 | U |
| VOTE-TC-093 | Tie copy | Two contenders equal | Render | `If still tied at the end, the most recent request wins.` | FR-077 | K |
| VOTE-TC-094 | Changes left count | Journal: 3 confirmed votes for node × contest | Render node detail | `2 of 4` (5 − 3) | FR-078 | U |
| VOTE-TC-095 | Changes unknown | Proved vote exists, journal empty | Render | `Changes left unknown. This node voted outside Dash Evo Tool.` | FR-078 | U |
| VOTE-TC-096 | Out of changes skipped | Journal 5 votes | Confirm | Node under Skipped with `no changes left (4 of 4 changes used)`; not submitted | FR-078, FR-025 | B |
| VOTE-TC-097 | Not in list excluded | Node absent from masternode list | Node set | Row disabled with reason; not submitted | FR-079 | K |
| VOTE-TC-098 | Keyboard flow | 2 cards | J, 2, J, L, Enter | Choices set, confirm opens; shortcuts ignored while filter field focused | FR-082, NFR-010 | K |
| VOTE-TC-099 | Bulk bar | 3 cards selected | Click Abstain | All three drafts = Abstain | FR-082 | K |
| VOTE-TC-100 | Drawer non-blocking | Batch sending | Interact with another card, navigate | Other controls enabled; drawer persists; no full-window overlay | FR-035, FR-083 | K |
| VOTE-TC-101 | Drawer statuses | Mixed outcomes | Inspect | Each row has typed status and the single valid action; unconfirmed never offers resubmit | FR-083, FR-044 | K |
| VOTE-TC-102 | Needs-attention row | One Unconfirmed + one missed | Open Votes | Row summarises both; Show opens drawer / Scheduled | FR-084 | K |
| VOTE-TC-103 | Contest ends mid-batch | Queued targets past end | Execute | `Failed before submission` + `Not cast. Voting ended.` | FR-087 | B |
| VOTE-TC-104 | Background refresh cadence | Voting nodes loaded | Advance clock | Contest + vote-state refresh at 30 min mainnet / 3 min testnet; one query per node | FR-074, NFR-007 | U |
| VOTE-TC-105 | Votes stub not used | — | Code search / seam test | Vote state uses `ResourceVote::fetch_many` by proTxHash | FR-010 | B |

## UX, accessibility, and isolation

| ID | Description | Expected outcome | Requirements |
|---|---|---|---|
| ~~VOTE-TC-070~~ | ~~Blocking submission feedback~~ | **[Superseded → VOTE-TC-100]** | |
| VOTE-TC-071 **[Changed]** | Keyboard confirm | Tab order: decisions → timing → Adjust nodes → Cast; Enter confirms, Esc cancels | NFR-005, FR-080 |
| VOTE-TC-072 | Network isolation | No cross-network locks, rows, or node sets | NFR-008 |
| VOTE-TC-073 | No secret persistence | Serialized operation has no key bytes | NFR-009 |
| VOTE-TC-074 | Complete message units | Complete strings and plural pairs | NFR-006 |
| VOTE-TC-075 | One submit path | Every path (Votes, node-scoped, due schedule) creates a coordinator operation | NFR-001 |
| VOTE-TC-076 | Technical error in details | Plain copy; diagnostics attached | FR-063 |
