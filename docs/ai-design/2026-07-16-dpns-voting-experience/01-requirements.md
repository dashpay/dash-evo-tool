# DPNS Voting Experience — Requirements

## Status

Implemented in PR [#901](https://github.com/dashpay/dash-evo-tool/pull/901)
(`feat(dpns): unify safe masternode voting operations`). **Revised 2026-10-01**
by the usernames/voting redesign (`docs/ai-design/2026-10-01-usernames-redesign/`):
operator convenience is the top priority; the safety model is unchanged.

Markers: **[Changed 2026-10-01]**, **[New 2026-10-01]**, **[Superseded 2026-10-01 → ID]**.
Superseded text is kept struck through for traceability; do not implement it.

Implementation stream: everything here is **Stream V** (voting). Identity-side
usernames are Stream U (`2026-10-01-usernames-redesign/01-requirements.md`).
Platform rules: `2026-10-01-usernames-redesign/platform-facts.md` (PF §n).

## Problem statement

DET previously exposed DPNS voting through disconnected per-node and bulk
experiences, both calling the same backend without a complete, authoritative
operation model: a vote could be accepted while DET showed no confirmation, the
same choice could be offered again, bulk results could not identify the node,
and a second click could submit another state transition while the first was
unresolved.

Voting is free for the voter (paid from the contest fund, PF §4), but each node
may vote only five times per contest (initial vote plus four changes), and each
vote is one state transition per node — there is no batching, even with a shared
voting key (PF §4). Duplicate, ambiguous, or wasted votes therefore have a real
cost, and operators with many nodes fan out to many transactions.

**[New 2026-10-01]** The #901 surface was safe but slow at scale: DPNS lived
under Tools, every batch blocked the whole window, review listed every
node × contest row, the node choice was not remembered, and nothing outside the
voting screen signalled that a vote was needed.

## Primary persona

Priya, the power-user masternode operator, runs 1–40 nodes (often several on
one voting key) and expects:

- current vote state to be accurate;
- to notice from anywhere in the app that a vote is needed **[New]**;
- one decision per contest applied to all chosen nodes **[New]**;
- to see her weight (evonode = 4) and each node's remaining changes **[New]**;
- bulk and scheduled operations for many nodes;
- progress that survives navigation and restart, without blocking the app **[Changed]**;
- exact per-node results; and
- safe recovery when Platform accepted a request but DET could not confirm it.

The voting workspace remains hidden below Detailed interface mode (Masternodes
gate). It is not an Everyday User workflow.

Validation floor: 24 nodes on one voting key, 6 open contests, 2 ending today.

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
| Voting entry points were disconnected | Per-node and bulk flows used different UI ownership | Operators could miss capabilities or assume they were removed |
| **[New]** Voting buried under Tools ▸ DPNS, mixed with "My usernames" | `tools_subscreen_chooser_panel.rs`, `dpns_subscreen_chooser_panel.rs` | Operators must remember to look; owners enter a governance tool |
| **[New]** Whole-window overlay per batch | #901 FR-035 | A 50-transaction fan-out blocks the app for minutes with no safety gain over target locks |

## Product decisions

1. ~~DPNS Active contests is the single home for immediate, bulk, and scheduled
   vote composition.~~ **[Changed 2026-10-01]** **Masternodes ▸ Votes** is the
   single home for immediate, bulk, and scheduled vote composition.
2. ~~Masternode detail links plainly to Active contests without carrying a node
   filter or draft.~~ **[Changed 2026-10-01]** Masternode detail offers
   `Vote with this node`, which opens Votes with the node set set to that node,
   visibly. It never carries a draft.
3. Single and bulk/scheduled voting use one confirm step and one shared
   operation coordinator. **[Changed: "review sheet" → aggregate confirm, VOTE-FR-080]**
4. Current vote state is authoritative Platform data, not UI-local memory.
5. A vote row remains visible after voting and shows the current choice. Voting
   again is presented as a change, not a new missing vote.
6. Operation state is global, correlated, and durable enough to survive
   navigation and process restart.
7. Exact targets, not whole screens, are locked. Unrelated nodes and contests
   remain usable. **[Reinforced: no whole-window blocking, VOTE-FR-083]**
8. **[New]** One decision per contest is applied to every node in the
   persistent node set; DET fans it out into one transaction per node.
9. **[New]** "Needs a vote" is signalled outside the voting page.

## Functional requirements

### Information architecture

- ~~**VOTE-FR-001** — Masternodes provides only the node list and node detail.~~
  **[Superseded 2026-10-01 → VOTE-FR-070]**
- **VOTE-FR-002** **[Changed 2026-10-01]** — Masternodes ▸ Votes owns the single
  vote composer, the confirm step, and the progress drawer.
- ~~**VOTE-FR-003** — A node detail page offers one `DPNS Voting` action that
  navigates plainly to Active contests.~~ **[Superseded 2026-10-01 → VOTE-FR-076]**
- **VOTE-FR-070** **[New]** — Masternodes has a segmented header `Votes | Nodes`.
  Votes is listed first and opens by default when VOTE-FR-073's count > 0,
  otherwise the last-used segment opens. Tools ▸ DPNS is removed. Persisted
  `RootScreenDPNSActiveContests/PastContests/ScheduledVotes` values route to
  Votes (To decide / History / Scheduled). The `My usernames` sub-screen leaves
  voting (owned by Stream U); `RootScreenDPNSOwnedNames` routes to the Identity
  hub.
- **VOTE-FR-071** **[New]** — Votes sub-views: `To decide` · `Voted` ·
  `Scheduled` · `History`. To decide is sorted by time left and includes
  contests where only some node-set nodes have voted (`Voted with {n} of {total}
  nodes`). Contests no node-set node can vote on go to a collapsed
  `Can't vote with your nodes` group with the reason.
- **VOTE-FR-072** **[New]** — Top-bar attention chip on every screen (Power role,
  ≥ 1 node with a voting key): `{count} names need your vote`. It is amber with
  `first ends in {time}` when the soonest deadline is within the urgency window
  (VOTE-FR-085), otherwise calm. A second chip `{count} vote is still being
  checked` appears while targets are Unconfirmed. Either chip opens Votes.
- **VOTE-FR-073** **[New]** — The Masternodes nav item shows a badge = contests
  needing a decision from ≥ 1 node-set node + unresolved targets.
- **VOTE-FR-074** **[New]** — A background refresh of contests and node vote state
  feeds VOTE-FR-072/073 while voting nodes are loaded: every 30 min on mainnet,
  every 3 min on testnet/devnet, and immediately on Votes arrival. A refresh
  that fails, or leaves any node's vote state unchecked, does not count as
  completed and is retried after a sixth of the interval. Proved vote
  state stays valid for display and for VOTE-FR-072/073 for two refresh
  intervals, so the signal does not blank between refreshes. Submission never
  relies on it: preflight fetches its own proof and accepts it for 120 seconds.
  The same limit applies to deciding that nothing needs sending: the confirm
  step skips a node as already voted only on proof within 120 seconds. On older
  proof the vote is sent to preflight, which confirms it without broadcasting
  when the choice is in place, or asks for another review when it changed.
  The tray and the confirm step present such a vote as one to check again
  (VOTE-FR-080/086): never as `Not voted yet`, and never as a transaction.
  A node with no changes left (VOTE-FR-078) is the exception: it can send
  nothing whatever a fresh check finds, so on older proof it is skipped with
  the `no changes left` reason instead of being checked again, and the tray
  does not offer it.
- **VOTE-FR-076** **[New]** — Node detail shows `This node's votes` (name, choice,
  changes left, deadline), its masternode-list status, and `Vote with this node`
  (VOTE-FR-075 node set = that node).

### Authoritative state

- **VOTE-FR-010** — DET queries each loaded node's proved Platform votes and
  joins them with active contests. Use `ResourceVote::fetch_many` +
  `ContestedResourceVotesGivenByIdentityQuery` keyed by proTxHash; never the
  `get_contested_dpns_identity_votes` stub (PF §5d).
- **VOTE-FR-011** — Each active contest shows the node's current choice, or
  `Not voted`.
- **VOTE-FR-012** — Existing votes remain actionable while the contest is
  votable, allowing a deliberate vote change.
- **VOTE-FR-013** — Selecting the already-current choice is a no-op and cannot
  create a state transition.
- **VOTE-FR-014** — Refresh updates contests, tallies, and node vote state as one
  coherent snapshot.
- **VOTE-FR-015** **[Changed 2026-10-01]** — A change to an existing vote is
  labelled as a change once per batch in the confirm step. Changes left per node
  follow VOTE-FR-078.
- **VOTE-FR-016** — If current vote state cannot be proved, DET shows it as
  unavailable and excludes that node from submission instead of assuming
  `Not voted`. Other nodes remain usable.
- **VOTE-FR-017** — Node summaries distinguish active contests from contests
  where the node has not voted yet.
- **VOTE-FR-077** **[New]** — Cards show weighted tallies (Platform tallies are
  weighted, PF §3) with the hint `Evonodes count as 4 votes.` and an influence
  line built from node-set weight: `{leader} leads by {margin} votes. Your
  {weight} votes can change who leads.` Show it only when weight ≥ margin. Ties
  read `If still tied at the end, the most recent request wins.` (PF §3).
- **VOTE-FR-078** **[New]** — Changes left per node per contest: with n =
  confirmed votes for that proTxHash × contest in the journal, it is 4 when n = 0,
  otherwise 5 − n. Label it `counted on this device`. If proved state shows a vote not present in the
  journal, show `Changes left unknown. This device has no record of this node's
  earlier votes.`
  A node with 0 left is skipped with the reason `no changes left (4 of 4 changes
  used)`.
- **VOTE-FR-079** **[New]** — A node not in the current masternode list is
  excluded from the node set, with `Not in the masternode list. Its votes don't
  count.` (PF §4 vote expiry).

### Vote composition

- **VOTE-FR-020** — Voting supports one node across one or more contests.
- **VOTE-FR-021** — Voting supports one or more contests across one or more
  nodes.
- **VOTE-FR-022** **[Changed 2026-10-01]** — A single timing applies to the whole
  batch, and individual nodes can be overridden under `Adjust nodes` in the
  confirm step. A node loaded after decisions were staged starts on
  `Don't use this node` every time the confirm step opens for those decisions,
  until the operator picks another option for it under `Adjust nodes`.
  Decisions staged after all earlier ones were cast or cleared use every
  loaded node again.
- **VOTE-FR-023** **[Changed 2026-10-01]** — Timing choices: `Now`,
  `When voting is about to end` (VOTE-FR-081), `At a specific time`, and per node
  `Don't use this node`.
- **VOTE-FR-024** **[Changed 2026-10-01]** — Before submission, the confirm step
  shows an aggregate (VOTE-FR-080). The full node × contest list (current →
  requested choice, timing, changes left) is available under `Adjust nodes`.
  A node already shown on the requested choice whose vote is only checked again
  (VOTE-FR-074) reads `Current choice: {choice}, as requested. It will be
  checked again before a vote is sent.`
- **VOTE-FR-025** — The confirm step removes no-op targets and explains why
  (counted under `Skipped`).
- **VOTE-FR-075** **[New]** — Persistent node-set selector per network: `All my
  nodes` · `Evonodes only` · `Masternodes only` · `Custom` (checklist), plus
  `Save as my default`. The chip shows `{n} nodes · {weight} votes`. Nodes
  without a voting key, out of changes, or not in the list are shown with the
  reason and cannot be selected.
- **VOTE-FR-080** **[New]** — The confirm popover shows:
  - title `Cast {d} decisions with {n} nodes`;
  - a decision list (name, choice, nodes, deadline);
  - `{t} transactions, one per node and name. Voting is free for your nodes.`;
    `{t}` leaves out votes that are only checked again (VOTE-FR-074), and the
    line is not shown when `{t}` is 0;
  - when there are such votes: `{r} votes are already shown as cast. Dash Evo
    Tool checks them again first and sends nothing for nodes that still hold
    their choice.`;
  - the change warning, once: `{c} of these change an earlier vote. Each node can
    change its vote 4 times per name.`;
  - `Skipped: {s} votes` with reasons;
  - the timing selector and `Adjust nodes`;
  - primary `Cast {t} votes` / `Schedule {t} votes`; when every vote is only
    checked again, `Check {r} votes` / `Schedule {r} checks`.
  - Enter confirms and Esc cancels.
  Relative schedule display presets are stored in one network-scoped metadata record per vote operation, without changing the vote journal binary format. Changing the absolute scheduled time clears the preset; cancellation and identity removal retain metadata only with its journal owner under the same retention rules. Network-data clearing retains journal history and uncertain locks under existing rules, and display metadata follows those owners. Reads never create or delete metadata.
- **VOTE-FR-082** **[New]** — Keyboard (only when focus is in the contest list):
  J/K or ↓/↑ move card focus · 1–9 vote for contender n · L lock · A abstain ·
  0 clear · Space select · Enter open confirm. Selecting ≥ 2 cards shows a bulk
  bar: `Lock name` · `Abstain` · `Clear`. A `Shortcuts` popover lists the keys.
- **VOTE-FR-086** **[New]** — The tray reads `{d} decisions ready · {t}
  transactions, one per node and name` with `Clear` and `Cast`. Votes that are
  only checked again (VOTE-FR-074) are left out of `{t}` and shown beside it as
  `{r} votes already shown as cast will be checked again`; `Cast` stays
  available for them. When `{t}` is 0 the tray reads `{d} decisions ready`
  without the transaction half.

### Operation lifecycle

- **VOTE-FR-030** — Every submitted batch has a stable operation ID.
- **VOTE-FR-031** — Every target result includes operation ID, node ID, contest
  ID/name, requested choice, and typed status.
- **VOTE-FR-032** — Target statuses are `Scheduled`, `Queued`, `Submitting`,
  `Confirming`, `Confirmed`, `Unconfirmed`, `Rejected`, and
  `Failed before submission`, plus `Cancelled` when the user cancels a
  scheduled target before it is submitted (see VOTE-FR-055). `Not applied`
  (definitive post-broadcast reconciliation) is **not implemented — blocked on
  [dashpay/platform#4137](https://github.com/dashpay/platform/issues/4137)**:
  the status is reserved in the journal encoding, and nothing produces it.
- **VOTE-FR-033** — Same-node targets execute sequentially to preserve nonce
  order. Different nodes may execute concurrently with a fixed bound.
- **VOTE-FR-034** — A target lock prevents a second operation for the same
  network + node + contest while the first is unresolved.
- **VOTE-FR-035** **[Changed 2026-10-01]** — Button state derives from the shared
  coordinator. A click disables the affected targets immediately. Progress shows
  in the non-blocking drawer (VOTE-FR-083), never in a full-window overlay.
- **VOTE-FR-036** — Navigation does not cancel an operation or lose its state.
- **VOTE-FR-037** — Restart restores scheduled and unresolved operations before
  enabling conflicting actions. A vote that was still queued (never claimed, so
  never broadcast) when the app stopped is sent after restart only while it is
  fresh: within 2 minutes of the review for an immediate vote, or of its time
  for an admitted schedule. Older ones are not sent by themselves — an
  immediate vote becomes `Not submitted` with `Review again`, a schedule is
  shown as missed. An immediate vote that recovery stops or sends again is
  dated by that recovery, so the progress drawer and `Needs attention` of that
  session show its outcome even though it was reviewed in an earlier one. A recovered immediate vote that was reviewed as a first
  vote is likewise not sent when Platform now shows a different vote for the
  node.
- **VOTE-FR-083** **[New]** — The progress drawer (bottom-right, collapsible,
  persists across screens) has the header `Casting {t} votes · {done} done ·
  {sending} sending · {checking} being checked`, a segmented progress bar, and
  per node × name rows with typed status and the valid action (`Check again`,
  `Review again`, `Add voting key`). `Review again` opens the confirm step for
  that one node × name only; decisions staged in the tray are not part of it
  and are still staged when it is confirmed or cancelled. Cards whose targets
  are in flight show
  `Sending with {n} nodes…` and lock their choices. Everything else stays
  interactive.
- **VOTE-FR-084** **[New]** — A `Needs attention` row at the top of Votes appears
  when any target is Unconfirmed, Rejected, Failed before submission, or a missed
  schedule. It summarizes and links to the drawer (`Show progress`) or Scheduled
  (`Open Scheduled`). Unconfirmed targets and missed schedules always count. A
  failure counts while it is the latest outcome for its target, has not been
  dismissed in the drawer, and was sent in this session — for a scheduled vote
  that is its scheduled time, so a vote scheduled days ago that fails today is
  shown.
- **VOTE-FR-087** **[New]** — Targets not yet submitted when their contest's
  voting ends become `Failed before submission` with the reason `Not submitted.
  Voting ended.`

### Confirmation and recovery

- **VOTE-FR-040** — Structured Platform consensus causes are treated as
  confirmed rejection. A rejection reported by the broadcast step is checked
  once against the proved current vote first, because that step retries and
  reports only its last attempt: when Platform already shows the requested
  choice, the target is `Confirmed`. Any other read, or a failed read, keeps
  the rejection.
- **VOTE-FR-041** — A cause-less post-broadcast wait failure is treated as
  `Unconfirmed`, never as rejection.
- **VOTE-FR-042** — Unconfirmed targets are reconciled against the proved
  current vote: an exact match confirms the target, and a missing or different
  vote leaves it Unconfirmed and locked. The scheduled-vote sweep re-checks them
  after it has executed the due votes — on the three sweeps after startup or
  vote activity, then every 10 minutes — and reports a result only when a vote
  was confirmed. Targets of a decided contest are not queried, because Platform
  has dropped their vote references. `Check again` re-checks on demand. Retry
  by transition hash is **not implemented — blocked on
  [dashpay/platform#4137](https://github.com/dashpay/platform/issues/4137)**.
- **VOTE-FR-043** — A target becomes `Confirmed` when authoritative state
  matches the requested choice.
- **VOTE-FR-044** — DET never offers `Submit again` while the result remains
  ambiguous. It offers `Check again`.
- **VOTE-FR-045** — **Not implemented — blocked on
  [dashpay/platform#4137](https://github.com/dashpay/platform/issues/4137).**
  A retry would become available only after authoritative reconciliation proves
  the requested change was not applied. No reconciliation result proves that
  today, so an Unconfirmed target keeps its lock.
- **VOTE-FR-046** — If saved voting-operation progress cannot be read, DET
  shows a persistent notice that the displayed history may be incomplete,
  offers `Retry loading`, and keeps cached progress and existing target
  locks intact until a refresh succeeds.

### Scheduled votes

- **VOTE-FR-050** — Scheduled votes use the same target model, result model,
  locking, execution order, and reconciliation as immediate votes.
- **VOTE-FR-051** — A scheduled target is marked executed only after confirmed
  application.
- **VOTE-FR-052** — Rejected and failed-before-submission targets remain visible
  with an actionable status.
- **VOTE-FR-053** — Unconfirmed scheduled targets are not automatically
  rebroadcast.
- **VOTE-FR-054** — Schedules use only the voting journal. Startup reports
  unexecuted schedules in previous SQLite storage and asks the user to schedule
  them again; it neither imports, lists, nor executes them. The notice stops
  one contest duration after it first appeared, when no such contest can still
  be open.
- **VOTE-FR-055** — A scheduled target can be edited or cancelled until
  execution begins. Once submitting, it follows normal operation locking.
- **VOTE-FR-056** — A target still `Scheduled` more than 120 seconds past its
  due time is explained in Scheduled as a missed automatic vote, with
  `Cast now`, `Edit`, and `Remove` actions.
- **VOTE-FR-081** **[New]** — `When voting is about to end` resolves to
  end_time − preset (default 6 h on mainnet, 10 min on testnet/devnet, editable).
  Store the absolute UTC time as today, and display both: `6 hours before the end
  · {abs UTC}`. A contest already inside the lead time is voted now instead of
  rejecting the batch; the confirm step says so with a count: `{n} of these
  votes will be sent now because voting ends soon.` `{n}` leaves out votes that
  are only checked again (VOTE-FR-074), and the line is hidden when none is
  left. Every other contest is
  scheduled as above. A contest whose deadline has not been read yet cannot be
  placed: its votes are listed under Skipped (`The end of voting is not known
  yet for these names. …`) and the rest of the batch goes ahead; a node set to
  `Now` under `Adjust nodes` still votes on it. Deadlines are read from the
  contest list when the confirm step is built, not when a choice was staged.
  Absolute times (`At a specific time`) at or after the deadline are still
  rejected (existing rule).
- **VOTE-FR-088** **[New]** — The Scheduled view groups rows by decision (name ×
  choice × time) with an expandable node list (`24 nodes ▾`). Per-node status
  shows inside the expansion. Groups are listed by name, then time; the view
  has no sort control and does not follow the History sort.

### Feedback

- **VOTE-FR-060** — One confirmed target shows a concise success banner.
- **VOTE-FR-061** **[Changed 2026-10-01]** — Batch feedback is the drawer plus one
  final banner summarizing counts: `{n} nodes voted on {d} names.`, or, while
  some are unconfirmed, `Participating nodes: {n}. Names with confirmed votes:
  {d}. Votes still being checked: {k}. Dash Evo Tool will keep checking. Do not
  submit the pending votes again.` A batch with several different outcomes
  lists one `{Outcome}: {count}.` unit per non-empty outcome. A batch in which
  nothing was submitted names the cause: a connection problem, voting ended, or
  a voting key that is not loaded.
- **VOTE-FR-062** — Messages name node aliases and contested names where useful.
- **VOTE-FR-063** — Technical errors stay in banner details.
- **VOTE-FR-064** — Unconfirmed copy explicitly says DET will keep checking and
  warns against resubmission.
- **VOTE-FR-065** — Loading a voting private key that does not match the
  selected node's voter identity is rejected with a key-specific error,
  without merging or altering the node's existing stored keys.
- **VOTE-FR-085** **[New]** — Durations and urgency come from the network's
  platform version (PF §3): contest 14 days / joinable 7 days on mainnet,
  90 min / 45 min on testnet/devnet. Urgency window: 24 h mainnet, 30 min
  otherwise. Never hard-code "two weeks". Source: shared
  `model/dpns.rs` helper (owned by Stream U).

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
  units with no parsed error text; plural pairs for counts.
- **VOTE-NFR-007 Performance** — Refresh queries votes once per node, not once
  per contest, and bounds cross-node concurrency. The background refresh
  (VOTE-FR-074) reuses the same path.
- **VOTE-NFR-008 Network isolation** — Drafts, schedules, operations, locks,
  results, and node-set preferences are network-scoped.
- **VOTE-NFR-009 Secret handling** — The coordinator stores identifiers and
  choices, never private keys.
- **VOTE-NFR-010** **[New]** **Shortcuts** — Single-key shortcuts act only while
  focus is in the contest list and never in text fields. Every shortcut has a
  visible control equivalent (WCAG 2.1.4).

## Platform dependency

The preferred recovery contract is
[dashpay/platform#4137](https://github.com/dashpay/platform/issues/4137) (retryable
post-broadcast wait errors; transition hash after broadcast). DET still supports
fallback reconciliation via the node's proved vote. Until either proves the
result, the operation remains unconfirmed and locked.

**[New]** Upstream ask: expose the per-node vote count in the proved
identity-votes response so VOTE-FR-078 can stop relying on the local journal.

## Out of scope

- Changing Platform's five-vote protocol limit.
- ~~Showing a remaining-change count before Platform exposes it in the proved
  identity-votes response.~~ **[Superseded 2026-10-01 → VOTE-FR-078]** (local
  count, labelled as such).
- Embedding Platform Explorer.
- Allowing arbitrary cancellation after broadcast.
- Unlocking locked names (not implemented in Platform, PF §1).
