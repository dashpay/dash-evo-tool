# Usernames Redesign — Requirements (Stream U)

Scope: identity-side usernames. Voting is Stream V in
`../2026-07-16-dpns-voting-experience/` (shared ownership table in its
`04-development-plan.md`). Platform rules: `platform-facts.md` (PF §n). Frames:
`wireframes.html` U1–U7. UX detail: `02-ux-spec.md`.

## Principles
- Usernames belong to the identity. Show and manage them in the Identity hub
  only, never on a voting screen.
- The Everyday User (Alex) is the validation floor. Identity-side copy never
  uses "DPNS", "contest", "contested" or "alias".
- Never offer an action Platform rejects (delete), and never imply a refund.

## Problems fixed (code refs at time of writing)
| # | Problem | Ref |
|---|---|---|
| P1 | Own names live in Tools ▸ DPNS ▸ My usernames | `settings.rs` `usernames_screen_action` |
| P3 | No availability check; the SDK helper reports contested and locked names as available | `register_dpns_name_screen.rs:516-557`; PF §5a |
| P4 | Hard-coded `0.2006` fee, red "contested" copy, no consent | `register_dpns_name_screen.rs:531-540` |
| P5 | Only one pending request, and only when the identity owns no name; status depends on a cache refreshed by voting screens | `contested_names_db.rs:261-272` |
| P6/P7 | "Set Alias" writes a username into the device-only name; "alias" has three meanings | `dpns_contested_names_screen.rs:955-984`, `settings.rs:554` |
| P8 | Disabled stubs "Add an alias" / "Make primary" / "Remove" | `settings.rs:573-595` |
| P9/P10 | Self-contradicting rules panel; identity picker when opened from an identity; dead-end red errors | `register_dpns_name_screen.rs:435-505, 638-657` |
| P12/P13 | Success copy points to a label that may not update; contest rules misstated | `register_dpns_name_screen.rs:333, 651-657` |

## Functional requirements

### Usernames card (Profile tab) — U1, U1b
- **USR-FR-001** — Profile ▸ `Usernames` lists for the selected identity, in
  this order:
  - main name, then other active names;
  - pending requests: `Open for other requests` during the join window,
    `Waiting for vote` after it, and `Awaiting result` once the estimated end
    has passed but the network has not confirmed an outcome;
  - finished outcomes from the last 30 days: `Went to someone else`,
    `Locked for good`, `No one got it`.
  - **AC:**
    - Given an identity owning `@a` with a pending request `@b`, both rows
      render.
    - Given 2 pending requests, both render.
- **USR-FR-002** — Active row menu: `Copy username` · `Show QR code` · `Show as
  main` (hidden on the main row). `Main` badge only when the identity has > 1
  active name.
  - **AC:** after `Show as main` on `@b`, the header subtitle, pills and switcher
    show `@b`. No network call. The preference persists per identity and
    network.
- **USR-FR-003** — Pending rows show `View status` → U3. Outcome rows show
  `Dismiss`; lost rows also show `Choose another username` → U4.
- **USR-FR-004** — Footer button `Get another username` → U4, with the note
  `Each username is a separate payment from this identity's balance.`
- **USR-FR-005** — Empty state: `This identity has no username yet. A username
  lets people pay you as @name instead of a long address.` [Get a username].
  View-only identity (no auth key): button disabled, tooltip `Add a key to this
  identity to register usernames.` + `Add a key` link.
- **USR-FR-006** — Remove the "Aliases" block, the disabled `Make primary`,
  `Remove` and `Add an alias` controls, and the `View all usernames` jump. Never
  offer delete (PF §2). Transfer/sale is deferred; reserve no visible control.

### Header, Home, banners — U2
- **USR-FR-010** — Header subtitle: the main name, or, when the identity has no
  active name, `@{name}` with the request's current status (`Open for other
  requests`, `Waiting for vote` or `Awaiting result`).
- **USR-FR-011** — Home card for each pending request: `@{name} is waiting for a
  community vote. It ends around {date}.` plus one of `You're leading right
  now.` / `No one else has asked so far.` / `Another request is leading.`
  [View status].
- **USR-FR-012** — One-time outcome banners, each shown once per (identity,
  name, outcome) via a local "seen" flag:
  - won: Success `You're @{name}. People can now find and pay you by this name.`
  - lost: Info `@{name} went to someone else. You can choose a different
    username.` [Choose another username]
  - locked: Info `No one can register @{name} anymore. The community vote locked
    it.`
  - no winner: Info `The vote for @{name} ended without a winner. You can choose
    a different username.` [Choose another username]
- **USR-FR-013** — Onboarding checklist and hero card keep DPN-010 behaviour
  (pending counts as done), sourced from USR-FR-020.

### Own-request status — U3
- **USR-FR-020** — New backend op `RefreshMyUsernameRequests` (all loaded
  identities):
  - first re-reads each saved pending request with
    `get_contested_dpns_vote_state`, then discovers further requests with
    `get_contested_non_resolved_usernames`, keeping the contests the identity
    is a contender in;
  - returns per identity a `Vec<UsernameRequest { label, normalized_label,
    phase (Joinable|Voting|AwaitingOutcome|Won|Lost|Locked|NoWinner),
    requested_at, join_end, end, decided_at, tally (you, others[], lock,
    abstain), last_updated }>`;
  - `AwaitingOutcome` is a local reading of the clock only: the estimated end
    has passed and no outcome is confirmed. Elapsed time never awards or
    rejects a name;
  - persists it and replaces the single-`Option` pending API.
  - **AC:** an identity owning a name still reports its pending requests. A
    finished contest yields Won/Lost/Locked/NoWinner.
- **USR-FR-021** — Refresh cadence: on hub arrival when the last refresh is
  > 5 min old; while ≥ 1 request is pending, every 15 min (testnet/devnet:
  2 min); and on the `Refresh status` button. It is triggered from the hub
  screen (no `app.rs` timer).
- **USR-FR-022** — The Request status page (pushed; breadcrumb `Identities ›
  {identity} › @{name}`) shows:
  - a timeline: Requested · Open for other requests until {join_end} ·
    Community vote until {end} · Result;
  - a read-only weighted tally (You · Other request ({short_id}) · Lock, so no
    one gets it · Abstain) with the words `Leading` / `Tied`, and the note
    `Evonodes count as 4 votes. Votes can change until the vote ends. Last
    updated {time}.`;
  - `What happens next` with exactly these rules (PF §3):
    - lone/leading request wins at the end;
    - a tie goes to the most recent request;
    - lock > top request → nobody can ever register it;
    - the fee isn't returned in any case.
  - Secondary [Get a username without a vote]. Power role + a loaded node with a
    voting key: link `Your nodes can vote on this name` → Masternodes ▸ Votes
    filtered to the name (Stream V route; U only emits the navigation action).

### Registration flow — U4–U7
- **USR-FR-030** — `Get another username` is opened from an identity. There is
  no identity picker; the signing key is auto-picked (Power role: `Advanced`
  disclosure with the key selector). The first username for a new identity
  keeps the 2026-09-22 create flow.
- **USR-FR-031** — New backend op `CheckUsernameAvailability(label)` → typed
  `Available | NeedsVote | Joinable { contenders, join_end } | JoinClosed |
  Taken | Locked | AlreadyRequested` (the asking identity is already in the
  running vote; Platform rejects a second request). It combines the
  awarded-domain lookup with
  `get_contested_dpns_vote_state` and never relies on `is_dpns_name_available`
  alone (PF §5a). The UI debounces 400 ms. Network failure → `CantCheck` UI
  state (retry).
  - **AC:** a locked name → `Locked`; an active contest within the join window
    → `Joinable`; after it → `JoinClosed`.
- **USR-FR-032** — Availability copy, one row at a time:
  - `Checking if @{name} is available…`
  - `@{name} is available.`
  - `@{name} is available, but it needs a community vote.`
  - `{count} other people already asked for @{name}. You can join the vote until
    {time}.` (plural pair)
  - `@{name} is already taken. Try another name.`
  - `@{name} can't be registered. A community vote locked this name for good.`
  - `Others can no longer join the vote for @{name}. Try another name.`
  - `You already asked for @{name}. Its status is on your identity's Usernames
    list.`
  - `Availability can't be checked right now. Check your internet connection and
    try again.` [Try again]
  - Continue is enabled only for Available / NeedsVote / Joinable.
- **USR-FR-033** — Suggestions `Try one without a vote:` for NeedsVote, Joinable
  and Taken: three labels that fail the contested rule (e.g. appending a digit
  2–9). Generated by `model/dpns.rs`.
- **USR-FR-034** — `Username rules` disclosure (collapsed), one correct list:
  - 3–63 characters;
  - letters, numbers and hyphens, with no hyphen at the start or end;
  - capital letters are treated as lowercase;
  - names under 20 characters that use only letters, hyphens, 0 and 1 need a
    community vote, and adding a digit 2–9 avoids it.
- **USR-FR-035** — The consent modal for NeedsVote/Joinable shows:
  - `Anyone can also ask for @{name} during the first {join_duration}. After
    that, only Dash masternode votes decide.`;
  - `The vote ends after {duration}, even if no one else asks. You'll see the
    result on your identity's page.`;
  - `The community vote fee is {contest_fee} DASH. It isn't returned, whether
    you get the name or not.`;
  - `If more votes go to locking the name, no one can register it, including
    you.`;
  - `Until the vote ends, people can't find you by this name.`;
  - buttons [Choose another name] [I understand, continue].
  - Durations come from `model/dpns.rs`. Never use the word "deposit".
- **USR-FR-036** — The confirm step shows:
  - `Username` (+ `Community vote` badge);
  - `Registration fee` `about {fee} DASH`;
  - `Community vote fee (not returned)` `{contest_fee} DASH`, read via
    `model/fee_estimation.rs` from `sdk.version()`: 0.2 DASH before protocol
    14, 0.1 DASH from 14 (PF §1);
  - `Total`, and `Paid from {identity}'s balance · Available {balance} DASH`;
  - primary `Pay {total} DASH and get @{name}` / `… and request @{name}`.
  - Low balance: `This identity has {balance} DASH. Add at least {missing} DASH
    to continue.` [Top up], which returns here with the name kept.
  - Not synced: Pay disabled, tooltip `Available after sync finishes.`
- **USR-FR-037** — On Pay, re-run the availability check. If it changed to
  Taken/Locked/JoinClosed/AlreadyRequested, return to U4 with that row; nothing
  is spent.
- **USR-FR-038** — Progress: blocking overlay `Registering @{name}.` `Keep Dash
  Evo Tool open until this finishes.` Results:
  - registered: `You're @{name}`;
  - request: `Your request for @{name} is in` + `If no one else asks and no one
    votes to lock it, @{name} becomes yours then.` [View request status]
    [Go to my identity];
  - failure: errors per AGENTS.md (no jargon, an action, details via
    `with_details`);
  - unconfirmed: a failed send of the name request does not prove the fee was
    not spent, because the same signed request is re-sent and only the last
    attempt is reported. The app therefore first re-reads the vote (or the
    owned names) after any failed send, a refusal included, and continues as a
    success if the request is there. A refusal the network does not contradict
    keeps its own error. Otherwise it reports `The app could
    not confirm whether your request for this username went through. Do not
    pay again yet. Wait a few minutes, then check the Usernames list on your
    identity. If the name is not there, try again.` and returns to U4 with a
    fresh availability check, never to the Pay step.

### Model/shared (Stream U owns; Stream V consumes durations)
- **USR-FR-040** — `model/dpns.rs`: `is_contested_label(label)` delegating to
  the contract's contested index `field_matches` (or the equivalent pure regex
  derived from it). Delete `register_dpns_name_screen.rs::is_contested_name` and
  any other copy.
- **USR-FR-041** — `model/dpns.rs`: `contest_durations(network/platform_version)
  -> { total, join }` (mainnet 14 d / 7 d; testnet/devnet 90 / 45 min, PF §3)
  and `urgency_window(network)` (24 h / 30 min).
- **USR-FR-042** — `model/fee_estimation.rs`: `contest_fee_credits(platform_version)`;
  remove the `0.2006` literal.
- **USR-FR-043** — Local settings: main username per identity; outcome-seen
  flags (both network-scoped KV).

## Non-functional
- **USR-NFR-001** — Complete i18n units, plural pairs, durations formatted from
  values.
- **USR-NFR-002** — Validation placement per AGENTS.md: format and contested
  rules live in `model/`; availability is enforced in the backend op; the UI
  only renders.
- **USR-NFR-003** — Status is shown as text plus icon. Read-only tally bars are
  not focusable.
- **USR-NFR-004** — No UI path offers deletion or calls the fee a deposit.

## Implementation plan (Stream U)
1. `model/` helpers (USR-FR-040–042) — merge first; Stream V depends on 041.
2. Backend ops `CheckUsernameAvailability`, `RefreshMyUsernameRequests` in new
   `backend_task/identity/dpns_usernames.rs`; context API returning
   `Vec<UsernameRequest>` (`contested_names_db.rs`).
3. Registration flow rework of `register_dpns_name_screen.rs` (U4–U7).
4. Usernames card, request status page, home card, banners (`ui/identity/*`).
5. Remove alias stubs and the "View all usernames" jump. Update user stories
   DPN-001/002/008/010, add DPN-012/016/017.

## Out of scope
Transfer/sale of names (PF §2, deferred); deleting names (rejected on-chain);
unlocking locked names (not implemented in Platform).
