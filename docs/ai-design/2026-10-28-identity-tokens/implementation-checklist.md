# Identity Hub Tokens — Implementation Checklist (2026-10-28)

Source of truth for implementation and the completeness audit. Design: `README.md`
and `wireframes.html` in this directory. Where this checklist and the README
disagree, **this checklist wins** (it records the user's decisions on README §11).

Scope: README §10 Phase 0, Phase 1, Phase 2. **Phase 3 is out of scope**
(token activity, Buy from row, inline reward estimate).

## User decisions (README §11)

| Q | Decision |
|---|---|
| Q1 | Tab order `Home · Contacts · Tokens · Profile`. Tab count badge shows **only** the number of pending claims; no badge when zero. |
| Q2 | Use **Send** everywhere for holder token transfers. Rename the pushed `TransferTokensScreen` heading, buttons, confirmation text, progress and success messages to "Send" wording, and rename the "Transfer" button in Tokens › My Tokens to "Send". Internal type/fn names stay unchanged. |
| Q3 | "Manage in Tokens screen" menu item stays, shown **only when the selected identity actually holds at least one administrative permission on that token** (mint, burn, freeze, unfreeze, destroy frozen funds, emergency action, set price, update config), as authorised by the token's configuration rules. Not tied to user role, not tied to issuer-only, and **not** derived from any developer-mode override that shows every button. |
| Q4 | Tokens with a fetched zero balance are shown as **normal rows** (no muted styling). A **"Hide tokens with zero balance"** checkbox is always visible above the list (not only at ≥ 8 rows) and its value is remembered per device. "Checking balance…" rows (unknown balance) are never hidden by it. |
| Q5 | Home Tokens card is shown when **any balance > 0 OR a claim is pending**; otherwise hidden. |

## Checklist

Each item needs code + a test (unit or kittest) unless marked *(no test)*.

### Phase 0 — prep
- [x] P0.1 Move `IdentityTokenIdentifier`, `IdentityTokenBalance*`, `IdentityTokenBasicInfo`, `IdentityTokenAvailableActions`, `get_available_token_actions_for_identity` from `ui/tokens/tokens_screen/structs.rs` to `model/token/`; keep re-exports so existing callers compile. *(no new test; existing tests pass)*
- [x] P0.2 `IdentityTokenAvailableActions::has_admin_permission()` answers "has any admin permission" for actions computed with `in_dev_mode = false` (Q3); the hub computes its actions that way once per row.

### Phase 1 — Tokens tab (MVP)
- [x] P1.1 `IdentityHubTab::Tokens` between Contacts and Profile; label + accessible description; `tabs.rs` tests updated.
- [x] P1.2 Pending-claim count badge on the tab (Q1); none when 0.
- [x] P1.3 View-state in `ui/state/` (renders nothing); renderer in `ui/identity/`; tab renderer does not itself return data loads (hub's single dispatch slot, same as Home/Contacts).
- [x] P1.4 Rows from registry minus per-pair dismissals; balance in token decimals + symbol; name, symbol, short ID; "Issued by you" badge; "Can't be sent" status.
- [x] P1.5 Row states: balance known, zero (normal styling, Q4), "Checking balance…" (unknown ≠ 0), claim available, once-per-identity "You can claim {amount} once." / "Claimed on {date}." (date via `model/datetime.rs`).
- [x] P1.6 Refresh on arrival (stale > 2 min or never) and on demand; single-flight: `TokenBalanceRefreshInProgress` = "already refreshing", never an error banner; disabled Refresh tooltip "Balances are being checked."; "Last updated {time}".
- [x] P1.7 Actions disabled until `masternodes_ready()` with reason "Available after sync finishes."
- [x] P1.8 Send → `TransferTokensScreen` pre-scoped; hidden at zero balance; disabled with reason when not transferable.
- [x] P1.9 Claim → `ClaimTokensScreen` (MN-009 pattern); contract missing → info banner with actionable copy; hidden after once-per-identity claim taken.
- [x] P1.10 ••• menu: Check balance now · Token details · Copy token ID · Stop tracking · Manage in Tokens screen (Q3 rule only).
- [x] P1.11 Stop tracking via `ConfirmationDialog`; dispatches `StopTrackingTokenBalance(pair)`.
- [x] P1.12 Add a token menu: Find by name (Tokens › Search) · Add by ID (`AddTokenById`).
- [x] P1.13 Empty states: no tokens tracked; all dismissed ("Track tokens again"); first load (spinner + text); stale after error (inline banner, rows kept, Try again).
- [x] P1.14 "How do I receive tokens?" → Home's Receive dialog with the extra tokens sentence.
- [x] P1.15 Hide-zero checkbox (Q4), persisted per device; does not hide unknown-balance rows.
- [x] P1.16 Hub wiring: render arm, arrival flag, reset on identity change, `display_task_result` / error handling per README §10 P1.3.
- [x] P1.17 Send rename (Q2) in `TransferTokensScreen` and My Tokens; update affected tests/kittests.
- [x] P1.18 List scrolls inside the island at 800 px window height; header + controls stay fixed.

### Phase 2
- [x] P2.1 Home Tokens card, shown iff any balance > 0 OR a claim is pending (Q5); Claim reachable from the card when exactly one claim is pending (several show a count); "See all tokens" opens the Tokens tab.
- [x] P2.2 Token detail page (row click / Enter / "Token details"): facts in plain words, copyable IDs, Send/Claim, Manage (Q3 rule; opens Tokens › My Tokens on that token through the pending-token-selection courier), Stop tracking.
- [x] P2.3 Many-token controls: filter by name/symbol and sort; remembered per device together with hide-zero.

### Cross-cutting
- [x] X.1 All copy from wireframes T9, with Q2/Q4 changes; i18n-ready complete sentences, no jargon (AGENTS.md error rules), technical detail only via `with_details`.
- [x] X.2 No admin action rendered anywhere in the hub except the Manage link.
- [x] X.3 Theme tokens only (`DashColors`, `Spacing`, `Shape`); light + dark; focus visible; disabled controls carry reasons.
- [x] X.4 `docs/user-stories.md` TOK-021 → `[Implemented]` with acceptance criteria matching this checklist; TOK-004 and TOK-020 use "Send" wording.
- [x] X.5 `cargo fmt --all`; clippy clean for touched targets; narrow tests + `kittest` green.

## Accepted deviations and additions

What the code does where the README (or this checklist's first draft) said
otherwise. Each is intentional.

| # | Deviation / addition |
|---|---|
| D-1 | Backend additions beyond "no backend change": `TokenTask::TrackTokenBalancesAgain(Vec<IdentityTokenIdentifier>)` re-tracks every dismissed pair of an identity in one request and takes the refresh guard before clearing the dismissals, so a refusal leaves them untouched; `AppContext::{set,take}_pending_token_selection` is the courier behind "Manage in Tokens screen" (`TokensScreen::refresh_on_arrival` takes it on every Tokens subscreen and selects the token only on My Tokens, so a stale request cannot linger); `IdentityTokensViewPrefs` is persisted at the global k/v key `det:identity_tokens_view:v1`. |
| D-2 | P0.2's standalone helper was folded into `IdentityTokenAvailableActions::has_admin_permission()`; the hub computes every row's actions once with `in_dev_mode = false` and reads the flag, instead of computing the actions twice. |
| D-3 | Home card (P2.1): a single pending claim is taken from the card; two or more show "{count} tokens have something to claim." and send the user to the tab. |
| D-4 | Not built from README §8: folding row actions into ••• under 640 px (the balance and status do stack under the name), the single-key `/` filter shortcut, the "Recently updated" sort (no per-pair update time exists), and T5's "Tracked since" and "Recent activity" (P3). |
| D-5 | All hub copy lives in `src/ui/identity/token_copy.rs` with one complete-sentence / no-jargon test; "Contract" is allowed as the name of a copyable ID. The header count and the hidden count are complete sentences ("Tracking {count} tokens.", "{count} tokens are hidden."), the ▾ glyph is decoration outside the translated label, and "Issued by you" / "Issued by someone else" are whole values. |
| D-6 | New theme constants: `DashColors::pill_accent` and `pill_neutral` (the only tints that read at 4.5:1 on their own 12 % fill in both themes); the tab badge and the row badges use them instead of `DASH_BLUE` / `text_secondary`. |
| D-7 | On-chain token names are sanitized before display through `model::token::token_display_name` (shared `identity_name::clean_display_text`, which drops controls, bidi directives and every invisible format character — soft hyphen, U+180E, ZWSP, word joiner and invisible operators, BOM, tag characters — while keeping ZWNJ/ZWJ); a name with nothing left shows the short token ID. |
| D-8 | The filter is capped at `IdentityTokensViewPrefs::MAX_FILTER_CHARS` (64); it applies at once but is written to the database when its box loses focus (Enter included), when the Tokens tab is left, or when the identity changes — never per keystroke, never lost; hide-zero and sort are saved as they change. |
| D-9 | A failed full refresh marks every unknown balance "Couldn't check this balance." with Try again (not "Checking balance…"), and the first-load line shows only while a refresh actually runs. A check or "Track tokens again" refused because a refresh already runs is sent again once that refresh ends. |
| D-10 | Tokens › My Tokens shares the claim rule: `claim_is_offered_only_to_a_named_pre_programmed_recipient` (in `my_tokens.rs`) proves a pre-programmed schedule offers Claim only to the identities it names, and `a_disabled_send_never_calls_an_unknown_balance_zero` proves the disabled Send reason never calls an unchecked balance zero. |
| D-11 | The inline failure notice is a reusable component, `ui/components/inline_notice.rs` (`InlineNotice`), not a hand-built frame. |
| D-12 | The hub keeps its content on a screen-wide `refresh()` while any Tokens-tab request is outstanding (`expects_screen_refresh`), instead of counting nudges: a nudge delivered to a screen covering the hub, or two nudges app.rs merges into one refresh, cannot leave a stale expectation behind. |

