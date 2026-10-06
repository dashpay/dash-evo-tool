# Usernames Redesign — Test Case Specification

Level: U = unit, K = kittest (`tests/kittest/` or inline harness), B = backend test with fake SDK seam.
Stream V test cases live in `../2026-07-16-dpns-voting-experience/03-test-case-spec.md` (VOTE-TC-*). This file covers Stream U and the shared `model/` helpers.

## Shared model helpers (Stream U owns)

| ID | Description | Steps | Expected | Req | Lvl |
|---|---|---|---|---|---|
| USR-TC-001 | Contested rule | `is_contested_label` on `alice`, `al1ce`, `alice2`, `a`·20, `b0b-01`, `Alice` | true, true, false, false, true, true (homograph-normalized) | FR-040 | U |
| USR-TC-002 | Single source | grep for `is_contested_name` | No copy outside `model/dpns.rs` | FR-040 | U |
| USR-TC-003 | Durations | `contest_durations` mainnet / testnet / devnet | 14 d + 7 d; 90 min + 45 min; 90 min + 45 min | FR-041 | U |
| USR-TC-004 | Urgency window | `urgency_window` mainnet / testnet | 24 h / 30 min | FR-041 | U |
| USR-TC-005 | Contest fee | `contest_fee_credits` for PV13, PV14 | 20,000,000,000 / 10,000,000,000 credits | FR-042 | U |
| USR-TC-006 | No literal fee | grep `0.2006` | No match | FR-042 | U |
| USR-TC-007 | Suggestions avoid vote | `suggest_uncontested("alice")` | 3 labels, each valid and `is_contested_label == false` | FR-033 | U |

## Backend ops

| ID | Description | Preconditions | Expected | Req | Lvl |
|---|---|---|---|---|---|
| USR-TC-010 | Available | No domain, no contest; label non-contested | `Available` | FR-031 | B |
| USR-TC-011 | Needs vote | No domain, no contest; contested label | `NeedsVote` | FR-031 | B |
| USR-TC-012 | Joinable | Active contest, now < join_end, 2 contenders | `Joinable { contenders: 2, join_end }` | FR-031 | B |
| USR-TC-013 | Join closed | Active contest, now ≥ join_end | `JoinClosed` | FR-031 | B |
| USR-TC-014 | Taken | Awarded domain exists | `Taken` | FR-031 | B |
| USR-TC-015 | Locked | Vote state winner = Locked; `is_dpns_name_available` returns true | `Locked` (SDK helper not trusted) | FR-031 | B |
| USR-TC-016 | Multiple requests | Identity owns `@a`, contender in `b`, `c` | `RefreshMyUsernameRequests` returns 2 requests for it | FR-020 | B |
| USR-TC-017 | Outcomes | Finished contests: won, lost, locked, no winner | Phases Won / Lost / Locked / NoWinner with dates | FR-020 | B |
| USR-TC-018 | Re-check on pay | Check said NeedsVote; at pay time Taken, Locked, JoinClosed or AlreadyRequested | Nothing is broadcast; U4 shows the matching row | FR-037 | B+K |
| USR-TC-041 | Already requested | Running vote lists the asking identity | `AlreadyRequested`; Continue disabled | FR-031, FR-032 | U+B |
| USR-TC-042 | Awaiting result | Saved pending request, estimated end passed, no confirmed outcome | Phase `AwaitingOutcome`; never Won/Lost by the clock alone | FR-020 | U |
| USR-TC-043 | Unconfirmed request | Name request fails without a refusal; request not readable / readable | `UsernameRegistrationUnconfirmed`, Pay step not restored / registration continues as success | FR-038 | B+K |
| USR-TC-044 | Refusal after an applied send | Name request is refused; the vote shows / does not show this identity's request / cannot be read | Registration continues as success / the refusal keeps its own error / `UsernameRegistrationUnconfirmed` | FR-038 | B |
| USR-TC-045 | Won name leaves the identity | Saved win recorded in the identity's names; a later refresh no longer lists the name | Not listed as an owned username; no further read-back; request ages out with other outcomes | FR-020, FR-001 | U+B |

## UI (kittest)

| ID | Description | Preconditions | Steps | Expected | Req |
|---|---|---|---|---|---|
| USR-TC-020 | Card lists all states | Identity: 2 active, 1 joinable, 1 voting, 1 lost, 1 locked | Open Profile | Six rows in order; `Main` badge on first; plain status words | FR-001 |
| USR-TC-021 | Show as main | 2 active names | Row menu → Show as main on `@b` | Header subtitle `@b`; persists after restart; no backend task | FR-002, FR-043 |
| USR-TC-022 | No stubs | Any identity | Open Profile | No `Aliases`, `Make primary`, `Remove`, `Add an alias`, `View all usernames` | FR-006 |
| USR-TC-023 | Empty state | No names | Open Profile | Empty copy + `Get a username` enabled | FR-005 |
| USR-TC-024 | View-only | Identity without auth key | Open Profile | Button disabled with tooltip; `Add a key` link | FR-005 |
| USR-TC-025 | Header pending | No active name, 1 request | Open hub | Subtitle `@{name}` with the request's status: `Open for other requests` in the join window, `Waiting for vote` after it | FR-010 |
| USR-TC-026 | Home card copy | Leading / no rival / trailing | Open Home | Matching second sentence | FR-011 |
| USR-TC-027 | Banner once | Request becomes Won | Open hub twice | Success banner first time only | FR-012 |
| USR-TC-028 | Refresh cadence | Last refresh 2 min ago | Arrive at hub | No refresh dispatched; at 6 min, dispatched | FR-021 |
| USR-TC-029 | Request status rules | Pending request | Open status page | Timeline phases; tally with `Leading`/`Tied`; 4 rule sentences | FR-022 |
| USR-TC-030 | Voter link gating | Power + voting node / Everyday | Open status page | Link visible / absent | FR-022 |
| USR-TC-031 | No identity picker | Open Get another username from identity X | Render | Title names X; no identity selector | FR-030 |
| USR-TC-032 | Availability rows | Each typed result incl. network error | Type a label, wait 400 ms | Exactly one matching row; Continue enabled only for Available/NeedsVote/Joinable | FR-032 |
| USR-TC-033 | Plural copy | Joinable with 1 and 3 contenders | Render | Singular and plural variants | FR-032, NFR-001 |
| USR-TC-034 | Consent copy | NeedsVote, mainnet and testnet | Continue | Modal with network durations, fee from model, "isn't returned"; no "deposit" | FR-035, NFR-004 |
| USR-TC-035 | Confirm fee rows | Contested label, PV14 context | Open confirm | `Community vote fee (not returned)` = 0.1 DASH; total correct | FR-036 |
| USR-TC-036 | Low balance | Balance < total | Open confirm | Warning with missing amount; Pay disabled; Top up returns with name kept | FR-036 |
| USR-TC-037 | Not synced | SPV syncing | Open confirm | Pay disabled with `Available after sync finishes.` | FR-036 |
| USR-TC-038 | Result variants | Non-contested / contested success | Complete | `You're @name` / `Your request for @name is in` with lone-request sentence | FR-038 |
| USR-TC-039 | Rules disclosure | Open U4 | Expand rules | Four correct rules; no "case-sensitive" | FR-034 |
| USR-TC-040 | Owned-names route gone | Persisted `RootScreenDPNSOwnedNames` | Start app | Identity hub (route owned by Stream V; U asserts no owned-names UI remains in `ui/identity`) | FR-006 |
