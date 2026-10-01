# DPNS Voting Experience — UX Specification

## Adopted direction (revised 2026-10-01)

This revision supersedes the 2026-07-21 "DPNS ▸ Active contests" placement. The
durable operation coordinator remains the safety model. What changed:

- Voting lives in **Masternodes ▸ Votes**, and a top-bar chip signals from every
  screen.
- One decision per contest is applied to a persistent node set.
- The confirm step is an aggregate.
- Progress is a non-blocking drawer.
- Each node shows its weight and changes left.
- Keyboard and bulk shortcuts are added.

Stream V. Wireframes: `../2026-10-01-usernames-redesign/wireframes.html`
frames V0–V6. Requirement IDs: `01-requirements.md`.

## Rationale

- Operators vote repeatedly and manage nodes occasionally, so voting sits next
  to the keys, node types (weight) and node fixes.
- Operators run many nodes and Platform has no vote batching. One decision
  fanned out to N transactions is the right unit; per-target rows are detail.
- Target locks already prevent duplicates. Blocking the whole window added time,
  not safety.
- Running out of changes is the costliest mistake, so changes left are shown,
  honestly labelled.
- Read-only tallies and selectable choices keep different shapes (unchanged
  from #901).

## Information architecture

```text
Top bar (every screen): breadcrumb · [N names need your vote] · [1 vote is still being checked] · network
Masternodes  (badge = needs-decision contests + unresolved targets)
├── [ Votes | Nodes ]            Votes first; opens on Votes when badge > 0
├── Votes
│   ├── To decide   (default; sorted by time left; partly voted included)
│   ├── Voted
│   ├── Scheduled
│   └── History
│   + node-set chip · filter · Shortcuts · Needs-attention row · tray · confirm · drawer
└── Nodes          list + detail (detail: This node's votes · Vote with this node)
Tools — DPNS entry removed. My usernames → Identity hub (Stream U).
```

## Votes ▸ To decide (frame V1)

Header row: sub-view chips · node-set chip `Vote with: All my nodes · 24 nodes ·
51 votes ▾` · `Find a name` (homoglyph tooltip: `Search ignores look-alike
characters the way usernames do, so o matches 0 and l matches 1.`).

Card (two columns):
- Left, read-only:
  - `alice.dash` and `Ends in 6 hours` (amber within the urgency window);
  - `2 requests · Voting ends {date} {time} UTC.` During the join window:
    `Others can join until {date} {time} UTC.`;
  - weighted tally bars with numbers (`Vote for` rows = contenders, then
    `Lock name`, `Abstain`);
  - influence line (VOTE-FR-077).
- Right: `Your decision` pills `Vote for {name}` (1–9) · `Lock name` (L) ·
  `Abstain` (A). The node line reads `Your nodes: 19 not voted · 4 voted Abstain
  · mn-07 has no changes left`. When the decision changes earlier votes:
  `Changing 4 earlier votes uses 1 of each node's 4 changes.` Unavailable state:
  `vote state unavailable for 3 nodes; they're left out.` with
  [Refresh voting].
- Checkbox at top-left for bulk selection. Keyboard focus ring on the focused
  card.

Below the cards, a collapsed `Can't vote with your nodes ({n})` group (dimmed,
reason per card).

Tray (outside scroll): `{d} decisions ready · {t} transactions, one per node and
name` [Clear] [Cast ⏎].

Needs-attention row (warning style, only when present): `Needs attention: 1 vote
from mn-12 on dashfan.dash is still being checked. Don't submit it again.
1 scheduled vote was missed.` [Show].

## Node set (frame V1b)

A popover with radios: All my nodes · Evonodes only · Masternodes only · Custom.
- Each radio shows `{n} nodes · {weight} votes ({e} evonodes × 4 + {m}
  masternodes)`.
- Custom shows a checklist with node, votes, voting-key status and note.
- Disabled rows: `No key` [Add voting key] · `Not in the masternode list` ·
  `No changes left`.
- `Save as my default` (per network) · [Done].

## Confirm (frame V2)

Popover per VOTE-FR-080. Timing segmented control: `Now` | `When voting is about
to end` | `At a specific time`.
- Helper for the relative option: `"When voting is about to end" casts each vote
  6 hours before its name's deadline. Keep Dash Evo Tool open and connected
  until then.` Names already closer to their deadline are voted now, and the
  confirm reads `{n} of these votes will be sent now because voting ends soon.`
- `Adjust nodes` table: Node · Name · Current → new · Changes left · Timing
  (Now / Before the end / Don't use this node).

Rules (unchanged from #901, now in the confirm step):
- remove exact no-ops;
- block targets held by an unresolved operation, with the reason;
- refuse nodes whose proved state is unavailable;
- if a first vote becomes a change during preflight, reopen the confirm with the
  change warning.

## Progress drawer (frame V2b)

As specified in VOTE-FR-083.
- Row copy by status: `Done` · `Sending` · `Being checked` + `This vote may
  already have been submitted. Don't submit it again.` [Check again] ·
  `Not applied` + `Platform shows a different vote.` [Review again] ·
  `Not cast` + reason.
- Collapsed state: a chip `Casting 44 votes · 33 done` above the network chip.
- The final banner is defined in VOTE-FR-061.

## Scheduled (frame V3)

Columns: Name · Nodes (`24 nodes ▾`) · Vote · When · Status · actions.
- When shows the relative label plus absolute UTC and a relative time.
- Statuses: `Scheduled` [Edit] [Remove] · `Missed` + `Dash Evo Tool wasn't
  running at that time. Voting is still open for {time}.` [Cast now] [Edit]
  [Remove] · `Not cast` + saved reason + valid action · `Cast with {n} nodes`.
- Bulk `Remove finished votes` asks for confirmation.
- Note: `Keep Dash Evo Tool open and connected so scheduled votes are cast on
  time.`

## History (frame V4)

Name · Ended · Outcome (`Went to {name} ({short_id})` [Copy ID] · `Locked for
good, no one can register it`) · `Your nodes voted` (`Vote for Zed (24 nodes,
51 votes)` / `Your nodes didn't vote.`).

## Empty and gate states (frame V5)

| State | Copy | Action |
|---|---|---|
| No nodes loaded | `Load a masternode with its voting key to vote on name contests.` (contests shown read-only; choice tooltip `Load a masternode with a voting key to vote.`) | Load a masternode |
| Nodes, no voting key | `None of your nodes has a voting key on this device. Add a voting key to vote. One key can serve several nodes.` | Add a voting key |
| No open contests | `There are no open name contests right now. New contests appear here automatically.` | Refresh |
| Journal unreadable | `Saved voting progress couldn't be read, so this history may be incomplete. Votes that are still being checked stay protected from resubmission.` | Retry loading |

## Node detail (frame V6)

Header: node alias, `Voting ready`, a type badge `Evonode · 4 votes`, and the
masternode-list status. Primary [Vote with this node]. The `This node's votes`
table shows Name · Vote · Changes left · Voting ends, with the footer `Changes
left are counted on this device.`

## Top-bar chip (frame V0)

Defined in VOTE-FR-072. Tooltip lists up to 3 names with deadlines. It is hidden
below the Power role or when no node has a voting key.

## Accessibility and responsive behavior

- Tally bars and numbers are labels with no focus. Choices are
  `selectable_label`s with the full phrase.
- Shortcut rules are in VOTE-NFR-010.
- Status is always text plus icon; color is supplementary.
- The drawer never steals focus.
- Narrow widths: tally stacks above choices; the drawer becomes a bottom sheet;
  the tray stays outside the scroll area.
