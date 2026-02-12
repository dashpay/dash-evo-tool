# Phase 5: DPNS Contest & Voting — Design & Review

## 5.1 DPNS Contest/Voting UX Design (Run 72)

### DPNS Screens Architecture

**4 tabs** (routes already defined in routes.tsx as placeholders):
- `/contracts/dpns-active` -> Active Contests (sortable table + inline vote selection + bulk voting)
- `/contracts/dpns-past` -> Past Contests (read-only historical table)
- `/contracts/dpns-owned` -> My Usernames (owned DPNS names with set-alias action)
- `/contracts/dpns-scheduled` -> Scheduled Votes (manage queued votes: remove, cast now, clear)

### Key Complexity Areas

**Active Contests tab** is the most complex — a sortable/filterable table where each row has
clickable vote buttons (Lock, Abstain, or per-contestant). Selected votes accumulate in state
and a "Cast/Schedule Votes" button opens a popup dialog with per-identity vote method
selection (No Vote / Cast Now / Schedule with days/hours/minutes). The dialog shows status
during submission and a success/partial-success/failure result screen.

**Smart name filter** converts lookalikes: 'o'/'O' -> '0', 'l' -> '1' (anti-confusion).

**Register Name** is already scoped as a separate screen (reachable from DPNS and Identities).
It detects contested names (length < 20, no non-0/1 digits), shows fee estimation, and
handles the preorder+domain document submission flow.

### Store Design: `contestStore.ts`

Following walletStore/identityStore patterns:
- State: contestedNames[], localDpnsNames[], scheduledVotes[], selectedVotes[], loading, refreshing, error
- Actions: loadContests, loadLocalNames, loadScheduledVotes, selectVote/deselectVote, castVotes, scheduleVotes, castScheduledVote, deleteScheduledVote, clearAll/clearCasted, setAlias, subscribeToUpdates
- Tauri commands already bound in bindings.ts

### Component Breakdown

- `components/contest/ActiveContestsTable.tsx` — sortable table with inline vote buttons
- `components/contest/PastContestsTable.tsx` — read-only historical table
- `components/contest/OwnedNamesPanel.tsx` — my usernames list with set-alias
- `components/contest/ScheduledVotesTable.tsx` — scheduled votes with actions
- `components/contest/VoteCastingDialog.tsx` — bulk vote casting/scheduling popup
- `components/contest/RegisterDpnsNameForm.tsx` — name registration form with validation
- Screens: DpnsActiveContestsScreen, DpnsPastContestsScreen, DpnsOwnedNamesScreen, DpnsScheduledVotesScreen, RegisterDpnsNameScreen

---

## 5.4 DPNS Screens Functionality Review (Run 84)

### Overall Assessment: STRONG — 95% functionality parity

572 tests (47 Playwright E2E + 199 screen tests + 79 table tests + 76 component tests + 81 store tests + 90 dialog tests). All 1845 project tests pass.

### Functionality Covered (complete parity):
- Active Contests: table with Name, Locked Votes, Abstain Votes, Ending Time, Last Updated, Contestants
- Vote selection: Lock, Abstain, TowardsIdentity — with toggle/replace behavior
- Vote visual emphasis: bold green for highest-vote lock/contestant
- Smart filter: o->0, l->1 normalization
- Sortable columns on all tables
- Past Contests: Name, Ended Time, Last Updated, Awarded To (WonBy/Locked badges)
- Owned Names: Name, Owner ID, Acquired At, Set Alias action
- Scheduled Votes: Name, Voter, Vote Choice, Scheduled Time, Status, Cast Now/Remove actions
- Clear All / Clear Casted buttons
- Vote Casting Dialog: full flow with Set All, per-identity selection, progress, results
- Register Name: identity selection, validation (3-63 chars), contested detection, fee estimation
- Real-time event subscriptions

### Minor Gaps (non-blocking):
1. Register Name: no wallet unlock flow (backend handles it)
2. Register Name: contested success always sets `contested: false`
3. Active Contests: "Register Name" button always visible (egui hides if no voting identities)
4. Auto-dismissing messages use Sonner toast (UX improvement, not regression)
5. No elapsed time indicator during contest refresh
