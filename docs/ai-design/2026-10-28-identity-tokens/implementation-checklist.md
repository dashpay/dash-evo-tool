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
- [ ] P0.1 Move `IdentityTokenIdentifier`, `IdentityTokenBalance*`, `IdentityTokenBasicInfo`, `IdentityTokenAvailableActions`, `get_available_token_actions_for_identity` from `ui/tokens/tokens_screen/structs.rs` to `model/token/`; keep re-exports so existing callers compile. *(no new test; existing tests pass)*
- [ ] P0.2 A helper on the actions type answering "has any admin permission" that ignores the dev-mode display override (Q3).

### Phase 1 — Tokens tab (MVP)
- [ ] P1.1 `IdentityHubTab::Tokens` between Contacts and Profile; label + accessible description; `tabs.rs` tests updated.
- [ ] P1.2 Pending-claim count badge on the tab (Q1); none when 0.
- [ ] P1.3 View-state in `ui/state/` (renders nothing); renderer in `ui/identity/`; tab renderer does not itself return data loads (hub's single dispatch slot, same as Home/Contacts).
- [ ] P1.4 Rows from registry minus per-pair dismissals; balance in token decimals + symbol; name, symbol, short ID; "Issued by you" badge; "Can't be sent" status.
- [ ] P1.5 Row states: balance known, zero (normal styling, Q4), "Checking balance…" (unknown ≠ 0), claim available, once-per-identity "You can claim {amount} once." / "Claimed on {date}." (date via `model/datetime.rs`).
- [ ] P1.6 Refresh on arrival (stale > 2 min or never) and on demand; single-flight: `TokenBalanceRefreshInProgress` = "already refreshing", never an error banner; disabled Refresh tooltip "Balances are being checked."; "Last updated {time}".
- [ ] P1.7 Actions disabled until `masternodes_ready()` with reason "Available after sync finishes."
- [ ] P1.8 Send → `TransferTokensScreen` pre-scoped; hidden at zero balance; disabled with reason when not transferable.
- [ ] P1.9 Claim → `ClaimTokensScreen` (MN-009 pattern); contract missing → info banner with actionable copy; hidden after once-per-identity claim taken.
- [ ] P1.10 ••• menu: Check balance now · Token details · Copy token ID · Stop tracking · Manage in Tokens screen (Q3 rule only).
- [ ] P1.11 Stop tracking via `ConfirmationDialog`; dispatches `StopTrackingTokenBalance(pair)`.
- [ ] P1.12 Add a token menu: Find by name (Tokens › Search) · Add by ID (`AddTokenById`).
- [ ] P1.13 Empty states: no tokens tracked; all dismissed ("Track tokens again"); first load (spinner + text); stale after error (inline banner, rows kept, Try again).
- [ ] P1.14 "How do I receive tokens?" → Home's Receive dialog with the extra tokens sentence.
- [ ] P1.15 Hide-zero checkbox (Q4), persisted per device; does not hide unknown-balance rows.
- [ ] P1.16 Hub wiring: render arm, arrival flag, reset on identity change, `display_task_result` / error handling per README §10 P1.3.
- [ ] P1.17 Send rename (Q2) in `TransferTokensScreen` and My Tokens; update affected tests/kittests.
- [ ] P1.18 List scrolls inside the island at 800 px window height; header + controls stay fixed.

### Phase 2
- [ ] P2.1 Home Tokens card, shown iff any balance > 0 OR a claim is pending (Q5); Claim reachable from the card; opens the Tokens tab.
- [ ] P2.2 Token detail page (row click / Enter / "Token details"): facts in plain words, copyable IDs, Send/Claim, Manage (Q3 rule), Stop tracking.
- [ ] P2.3 Many-token controls: filter by name/symbol and sort; remembered per device together with hide-zero.

### Cross-cutting
- [ ] X.1 All copy from wireframes T9, with Q2/Q4 changes; i18n-ready complete sentences, no jargon (AGENTS.md error rules), technical detail only via `with_details`.
- [ ] X.2 No admin action rendered anywhere in the hub except the Manage link.
- [ ] X.3 Theme tokens only (`DashColors`, `Spacing`, `Shape`); light + dark; focus visible; disabled controls carry reasons.
- [ ] X.4 `docs/user-stories.md` TOK-021 → `[Implemented]` with acceptance criteria matching this checklist.
- [ ] X.5 `cargo fmt --all`; clippy clean for touched targets; narrow tests + `kittest` green.
