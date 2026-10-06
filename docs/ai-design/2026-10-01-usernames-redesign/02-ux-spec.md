# Usernames and Name Votes — UX Specification (developer edition)

Two streams with separate homes:
- **Stream U**: identity usernames, in the Identities hub. Requirements and copy
  are in `01-requirements.md`.
- **Stream V**: voting, in Masternodes ▸ Votes. Requirements are in
  `../2026-07-16-dpns-voting-experience/`.

Wireframes: `wireframes.html` (U1–U7, V0–V6). Platform rules:
`platform-facts.md`.

## Personas and jobs
| Persona | Jobs | Home |
|---|---|---|
| Alex (Everyday) | get a payable name; see/share it; understand vote, cost (not refunded), risk; follow the request without a voting tool; recover after losing | Identities hub |
| Priya, owner (Power) | names per identity; pick the main name; add a name paid from identity balance | Identities hub |
| Priya, operator (Power, 1–40 nodes, shared voting keys) | know from anywhere that a vote is needed; one decision → all chosen nodes; see weight and changes left; vote now or before the deadline; trust no duplicates; per-node results | Masternodes ▸ Votes |
| Jordan (Developer, testnet) | fast test names (digit 2–9 avoids votes); 90-minute contests | both |

## IA
```text
Top bar: breadcrumb · [N names need your vote] (V) · network
Identities hub (U)
  header subtitle: @main | "@name · Waiting for vote"
  Home: request card · one-time outcome banners · checklist
  Profile ▸ Usernames: rows (••• Copy · QR · Show as main) · pending → Request status
                       [Get another username] → Choose → (Consent) → Confirm & pay → Result
Masternodes (V, Power gate): [Votes | Nodes]; Votes = To decide · Voted · Scheduled · History
Tools: DPNS removed.
```

## Decisions (former open questions)
| Topic | Decision | Why |
|---|---|---|
| Where voting lives | Masternodes ▸ Votes (first segment), plus the top-bar chip | Operator convenience; keys and weights live there |
| Contest browser for non-operators | None; own contests via Request status | No listed job; less surface |
| Fee | Live from `sdk.version()` (0.2 DASH before protocol 14, 0.1 DASH from 14); "isn't returned"; never "deposit" | PF §1 |
| Delete / transfer | Delete never offered; transfer/sale deferred to phase 2 | PF §2; irreversible flow with no backend op |
| Lost-request retention | 30 days in Profile | Noticeable after a break; Platform keeps the poll record |
| Refresh cadence | U: hub arrival (> 5 min) + 15 min while pending (testnet 2 min). V: 30 min (testnet 3 min) while voting nodes are loaded | Fresh enough for 14-day and 90-minute contests |

## Vocabulary
| Identity side (U) | Operator side (V) | Never in UI copy |
|---|---|---|
| username, `@name`, Waiting for vote, Open for other requests, community vote, community vote fee (not returned), Show as main, Name on this device | name contest, decision, node set, votes (weighted), changes left, Vote with this node | DPNS, contested, alias, deposit, state transition, nonce |

## Interaction states (U)
- Availability row: exactly one state (USR-FR-032), 400 ms debounce. Format
  checks update per keystroke.
- Disabled controls always carry a tooltip reason. View-only identity → `Add a
  key`. Not synced → `Available after sync finishes.`
- Blocking progress overlay only during registration. Request-status refresh shows inline
  `Last updated {time}`, no overlay.
- Outcome banners: once each, dismissible. Profile rows carry the state for
  30 days.

## Interaction states (V)
See `../2026-07-16-dpns-voting-experience/02-ux-spec.md`. In short:
- the node-set chip is remembered;
- cards are sorted by time left;
- the confirm is an aggregate;
- progress is a non-blocking drawer;
- keyboard J/K, 1–9, L, A, Space, Enter.

## Accessibility and responsive
- Status is text plus icon. Tallies are non-focusable labels. Single-key
  shortcuts act only in list focus (WCAG 2.1.4). Modals trap and return focus.
- Narrow layout: Profile columns stack, with the Usernames card first when it
  has a pending row. Contest card tally stacks above choices. The drawer becomes
  a bottom sheet.

## Backend gaps (summary)
- U: `CheckUsernameAvailability`, `RefreshMyUsernameRequests`, `Vec` pending
  API, contest fee and contested rule in `model/`, contest durations helper,
  local main-name and outcome-seen settings.
- V: attention summary and background refresh, node-set preference,
  changes-left journal count (upstream ask: vote count in proved votes),
  masternode-list membership per node.
