# Tokens in the Identity Hub (2026-10-28)

Design record: show the tokens an identity holds inside **Identities › hub**,
where the identity's owner already is, instead of only in the token-first
**Tokens › My Tokens** table. Holder jobs only — every administrative token
action (mint, burn, freeze/unfreeze, destroy frozen funds, pause/resume, set
price, update config, token creator, distributions) stays in the Tokens screen
untouched.

Companion wireframes: [`wireframes.html`](wireframes.html) (T0–T9, light and
dark). Status: **implemented** (`src/ui/identity/tokens_tab.rs`,
`token_detail.rs`, `token_copy.rs`, `src/ui/state/identity_tokens.rs`); where
the code departs from this note, the checklist's "Accepted deviations and
additions" section records it. The user's answers to the
open questions are recorded in §11 (Decisions) and, item by item, in
[`implementation-checklist.md`](implementation-checklist.md) — **the
checklist is the authoritative list of what gets built**; where this note and
the checklist disagree, the checklist wins.

Source of truth reviewed: `src/ui/identity/{hub_screen,tabs,identity_hub_tab_bar,home}.rs`,
`src/ui/tokens/tokens_screen/{structs,my_tokens}.rs`,
`src/ui/tokens/{claim_tokens_screen,transfer_tokens_screen,view_token_claims_screen,direct_token_purchase_screen,add_token_by_id_screen}.rs`,
`src/backend_task/tokens/{mod,query_my_token_balances}.rs`,
`src/context/contract_token_db.rs`, `src/wallet_backend/token_balance.rs`,
`src/ui/masternodes/detail_screen.rs` (MN-009), `docs/personas/*`,
`docs/user-stories.md` (TOK-001…020, MN-009), `docs/ux-design-patterns.md`,
`src/ui/components/README.md`, and the prior hub designs listed in §9.

---

## 1. Executive summary

**Problem.** Alex opens his identity and sees Dash balance, contacts and
profile — but not the DashCoffee tokens a client just sent him. To find them he
must know that a separate *Tokens* section exists, that the table there is
grouped by token rather than by identity, and that the "Transfer / Claim" row
sits next to "Mint / Burn / Freeze" he can never use. Priya, with four
identities, has to click into every token to see which identity holds what.

**Actors.** Alex (Everyday, one identity, mainnet), Priya (Power, several
identities, sometimes a token issuer), Jordan (Developer, testnet, creates
tokens and claims airdrops to test them).

**Direction.** Add an identity-first **Tokens** tab to the hub
(Home · Contacts · **Tokens** · Profile) that lists this identity's tracked
tokens with balance, a plain-words status line, and exactly two inline actions —
**Send** and **Claim** — plus a quiet menu (check now, details, copy ID, stop
tracking, and — only when this identity holds an administrative permission on
that token — a link to the Tokens screen). Reuse the existing Transfer and
Claim screens pre-scoped to the identity; the Transfer screen and My Tokens'
"Transfer" button adopt **Send** wording (D2). A "Hide tokens with zero
balance" checkbox sits above the list from day one (D4). Phase 2 adds a Home
card (only when there is something to show), a token detail page and
filter/sort controls. The balance snapshot, registry, dismissals and refresh
task already existed and are identity-keyed; the backend grew three small
pieces: `TokenTask::TrackTokenBalancesAgain(pairs)` (one request re-tracking
every dismissed pair of an identity), the `AppContext::{set,take}_pending_token_selection`
courier that opens Tokens › My Tokens on a token, and `IdentityTokensViewPrefs`
persisted at the k/v key `det:identity_tokens_view:v1`.

## 2. Personas and jobs

| Persona | Jobs regarding tokens of *this* identity | Where today | Gap |
|---|---|---|---|
| Alex (Everyday) | see what I hold and how much; tell tokens apart; collect rewards/airdrops; send some to a person; know how to receive; understand why a button is grey | Tokens › My Tokens, token-first table | not in the identity's own place; issuer actions in the way; "Check" and "Transfer" are bare verbs; empty state says "No Tracked Tokens" |
| Priya (Power, holder) | the same, per identity; stop tracking noise tokens; see "issued by me" at a glance; jump to admin when she *is* the issuer | same | per-identity view needs a click per token; no issuer badge from the holder side |
| Jordan (Developer) | claim test airdrops fast, verify a transfer landed, see raw IDs | same | fine functionally; benefits from identity-first list on testnet |

Progressive disclosure (`docs/personas/README.md`): everyone sees the tab,
rows, Send/Claim, Add a token, the hide-zero checkbox and the "Issued by you"
badge. The "Manage in Tokens screen" menu item is **not** role-gated: it
appears for whichever identity actually holds at least one administrative
permission on that token (D3) — in practice Priya and Jordan as issuers, but
Alex too if a token's rules grant him one. Raw credit/fee context stays on the
pushed screens where it already lives.

## 3. User needs, prioritised

| # | Need | Persona | Priority | Phase | Covered by |
|---|---|---|---|---|---|
| N1 | See every tracked token of the selected identity with its balance, formatted in the token's decimals and symbol | all | P0 | MVP | T1 |
| N2 | Recognise a token: name, symbol, short ID, "Issued by you", "Can't be sent" | all | P0 | MVP | T1, T1b |
| N3 | Claim rewards / once-per-identity airdrop from the row | Alex, Jordan | P0 | MVP | T1, T6 |
| N4 | Send tokens to another identity from the row | Alex, Priya | P0 | MVP | T1, T6 |
| N5 | Empty, first-load, stale-after-error states that explain and offer a way forward | Alex | P0 | MVP | T2, T3 |
| N6 | Refresh on arrival and on demand; never confuse "not checked" with 0 | all | P0 | MVP | T1, T1b |
| N7 | Add a token to track (by name search or by ID) without leaving the hub mentally | Alex | P0 | MVP | T2 |
| N8 | Know how to receive tokens | Alex | P1 | MVP | T7 |
| N9 | Stop tracking a token for this identity; get it back | Priya | P1 | MVP | menu, T2 right |
| N10 | Glance from Home when something is held or claimable | Alex | P1 | P2 | T8 |
| N11 | Token detail page: facts in plain words, copyable IDs | Alex, Priya | P1 | P2 | T5 |
| N12a | Hide tokens with a zero balance (checkbox always visible, remembered per device; never hides "Checking balance…" rows) | Alex, Priya | P1 | MVP | T1, T4 |
| N12b | Scale to many tokens: filter, sort | Priya, Jordan | P1 | P2 | T4 |
| N13 | Discreet link to admin flows when the identity holds an admin permission on the token | Priya, Jordan | P1 | MVP (menu), P2 (detail) | menu, T5 |
| N14 | Token activity (sends, receipts, claims) | Alex | P2 | P3 | T5 placeholder |
| N15 | Buy a priced token from the row | Priya | P2 | P3 | — |
| N16 | Show the estimated reward amount inline before claiming | Jordan | P3 | P3 | — |

Non-goals: any admin action in the hub; a per-token "address"; moving or
removing anything from the Tokens screen.

## 4. Placement decision

**Both a tab and a Home card, phased: the tab is the MVP, the card is Phase 2
and conditional.**

Why a tab, not only a card: a token list with per-row actions, five row states
and up to dozens of rows needs a surface of its own; Home is the glance surface
and already carries actions, checklist, notices and payments (its two-column
layout wraps under 760 px). Why also a card: Home is where Alex lands; "2 tokens
have something to claim" is exactly the kind of signal Home exists for. Why
conditional: an empty "Tokens" card would advertise a secondary feature to
people who have none — the tab label alone is enough discoverability.

Tab order `Home · Contacts · Tokens · Profile` keeps Profile (settings-like)
last. The tab bar shows a count badge only for pending claims (no badge at
zero), never the token count (a count that is not actionable is noise) — D1.
The Home card appears when any balance is above zero *or* a claim is pending
— D5.

Alternatives rejected:
- *Profile tab card* — Profile is public-facing settings; balances do not belong there.
- *Linking out to My Tokens filtered by identity* — keeps the token-first grouping and the admin buttons in Alex's path; also the My Tokens screen has no identity filter today.
- *Reviving the hidden Activity tab for tokens* — Activity is a unified timeline by design (§A.2 of 2026-04-22); tokens would hijack it.

## 5. Information architecture

```text
Identities ▸ hub ▸ [ Home | Contacts | Tokens | Profile ]
  Home
    └ Tokens card (P2; shown only when balance > 0 or a claim waits)
  Tokens (MVP)
    header: "Tokens" · Refresh · Add a token ▾ (Find by name · Add by ID)
    state bar: Last updated {time} · How do I receive tokens? → Receive dialog
    controls: [ ] Hide tokens with zero balance   (always visible; per device; never hides "Checking balance…")
              ≥ 8 rows (P2): filter · sort
    list, one row per tracked token of the selected identity
       row: avatar · name [+badge] · symbol · short ID │ balance · status │ Send · Claim │ •••
       •••: Check balance now · Token details · Copy token ID · Stop tracking
            · ─ · Manage in Tokens screen   (only if this identity holds an admin permission on the token)
       row click / Enter → Token detail (P2)
Pushed screens (existing): TransferTokensScreen (Send wording) · ClaimTokensScreen · AddTokenByIdScreen
Root screens (existing): Tokens › Search (find by name) · Tokens › My Tokens (manage; "Transfer" button → "Send")
```

Vocabulary: **Send** everywhere a holder moves tokens — the hub rows, the
pushed `TransferTokensScreen` (heading, buttons, confirmation, progress and
success messages) and My Tokens' row button; only internal type and function
names keep "transfer" (D2). **Claim**, **Stop tracking** (matches TOK-018),
**Add a token**. Never in hub copy: transfer, distribution type, perpetual,
pre-programmed, contract owner, state transition, nonce.

## 6. Data source (investigated, not guessed)

Everything the MVP reads already exists and is identity-keyed:

| Need | Existing seam | Notes |
|---|---|---|
| Rows | `AppContext::get_all_known_tokens()` (registry, global per device) minus `untracked_token_balances()` for `(identity, token)` | registry is device-wide; dismissals are per pair |
| Balance | `AppContext::identity_token_balances()` → `IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>`; underneath `WalletBackend::token_balances()` (`UpstreamTokenBalances`, lock-free, frame-safe) | `None` = identity not synced / token not in its synced set → "Checking balance…"; `Some(0)` = real zero (TB2–TB4 in `token_balance.rs`) |
| Actions | `IdentityTokenBalance::into_with_actions(identity, contract, in_dev_mode, pricing)` → `can_transfer`, `can_claim`, `can_maybe_purchase` for the row buttons | needs `get_contract_by_token_id`. The admin flags feed **only** the "Manage in Tokens screen" link: the hub computes the actions once with `in_dev_mode = false` and asks `IdentityTokenAvailableActions::has_admin_permission()` — never pass `in_dev_mode = true` to decide the link |
| Once-per-identity | `model::token::once_per_identity_amount(config)`, `AppContext::once_per_identity_claimed_at(token, identity)` | drives "You can claim {amount} once." / "Claimed on {date}." |
| Issuer badge | `contract.owner_id() == identity.id()` | My Tokens colours the ID dark green; the hub uses a text badge |
| Refresh | `TokenTask::QueryMyTokenBalances` (all identities, single-flight; `TokenBalanceRefreshInProgress`, `TokenBalanceRefreshTimeout`), `TokenTask::QueryIdentityTokenBalance(pair)` (also re-tracks), `TokenTask::StopTrackingTokenBalance(pair)`, `TokenTask::TrackTokenBalancesAgain(pairs)` (re-tracks every dismissed pair of an identity in one request; takes the refresh guard before touching the dismissals) | a fetch sends `TaskResult::CacheUpdated` (`CacheDomain::Tokens`) before its typed result; every open screen gets it through `cached_data_changed`, and the hub only re-reads its token rows (`IdentityTokensState::cached_data_changed`), keeping the profile and contacts on screen; in-flight flag `AppContext::token_balance_refresh_in_flight` (crate-visible) |
| Manage link | `AppContext::set_pending_token_selection(token_id)` then `AppAction::SetMainScreen(RootScreenMyTokenBalances)`; `TokensScreen::refresh_on_arrival` takes the courier on every Tokens subscreen and selects the token only on My Tokens | mirrors `pending_identity_selection` |
| List controls | `IdentityTokensViewPrefs` (hide-zero, sort, filter ≤ 64 chars) at k/v `det:identity_tokens_view:v1`, global scope | hide-zero and sort are saved as they change; the filter when its box loses focus, when the tab is left or when the identity changes |
| Token names | `model::token::token_display_name(name, token_id)` — `identity_name::clean_display_text` (controls, bidi directives and every invisible format character removed; ZWNJ/ZWJ kept), falling back to the short ID | on-chain names are untrusted |
| Send | `TransferTokensScreen::new(IdentityTokenBalance, ctx)` | identity fixed by the balance record |
| Claim | `ClaimTokensScreen::new(IdentityTokenBasicInfo, QualifiedContract, TokenConfiguration, ctx)` — the MN-009 pattern in `masternodes/detail_screen.rs` | contract missing → info banner + fallback |
| Add | `ScreenType::AddTokenById`; `RootScreenType::RootScreenTokenSearch` | after import, `clear_untracked_token` re-tracks for all identities |
| Post-action balance | `insert_token_identity_balance` applies the proof-derived balance to the snapshot immediately | row updates before the next upstream pass |
| Receive | Home's Receive modal (`home::render_receive`) | one extra sentence |

Constraints found:
- The balance snapshot is **per identity, not per token**: an identity absent
  from it is "syncing" for every token. The first visit after start-up shows
  skeletons until `QueryMyTokenBalances` completes (T3 left).
- Platform has no "all tokens held by identity" query; tracking is explicit.
  The empty state and the Receive dialog say so plainly (T2, T7). *To confirm
  against the current SDK before copy is final.*
- `estimated_unclaimed_rewards` is `None` from the snapshot; amounts appear only
  after an explicit estimate task. Hence "Rewards may be waiting for you." (not
  an amount) in the MVP; N16 is Phase 3.
- Proof-verifying reads must wait for `connection_status().masternodes_ready()`
  (same gate Home uses for contacts/payments). Until then: rows render from the
  last snapshot, actions disabled with "Available after sync finishes." The
  token-balance tasks enforce the gate themselves
  (`TaskError::MasternodeListSyncPending`), so My Tokens, the identity detail
  Check and the post-mutation refreshes are covered too; the hub treats that
  refusal like "already refreshing" and sends the request again later.
- A forced balance refresh waits out an upstream background pass that is
  already running (it snapshotted the watch lists before DET registered its
  own), then runs its own pass, so a refresh never settles without fetching.
- Refresh is single-flight across the app: the hub and My Tokens share the
  guard. The hub treats `TokenBalanceRefreshInProgress` as "already
  refreshing", never as an error banner; a check or "Track tokens again"
  refused for that reason is sent again once the running refresh ends.
- Row checks, stop-tracking requests and the refresh each have their own
  pending-request bookkeeping (`PendingRequest` / `PendingRequests`), so two
  quick checks on different rows are both still the tab's when they answer.

## 7. Flows

```
Flow: See my tokens | Entry: hub tab bar → Tokens
Steps: 1. Tokens tab (rows from snapshot) -> arrival refresh if last sync > 2 min or never -> rows update in place
       2. Row -> hover/focus shows actions; Enter/click -> Token detail (P2)
Success: balances with "Last updated {time}" | Errors: inline banner, stale rows kept, Try again | Edge: identity unsynced → skeletons; token unsynced → "Checking balance…"; all dismissed → T2 right
```

```
Flow: Claim | Entry: row Claim (or Home card Claim when exactly one claim is pending, P2; several pending claims show a count and "See all tokens")
Steps: 1. Push ClaimTokensScreen pre-scoped to (identity, token) -> 2. confirm -> blocking progress -> 3. back to hub, success banner, row balance updated
Errors: contract not loadable → "This token's details aren't available yet. Refresh and try again."; network refusal explained by the existing screen (TOK-019)
Edge: once-per-identity already taken → row shows "Claimed on {date}", Claim hidden; view-only identity → Claim disabled with reason
```

```
Flow: Send | Entry: row Send
Steps: 1. Push TransferTokensScreen(IdentityTokenBalance), heading "Send {token}" -> 2. recipient (username or ID) + amount -> 3. confirm "Send" -> progress "Sending {amount}…" -> back, "Sent {amount} to {recipient}." banner, balance updated
Errors: non-transferable → Send disabled "This token cannot be sent to other identities."; zero balance → Send hidden
```

```
Flow: Add a token | Entry: header "Add a token ▾" or empty-state buttons
Steps: Find by name -> Tokens › Search (root) -> add -> return to hub (Back) -> row appears "Checking balance…" -> refresh
       Add by ID -> AddTokenByIdScreen (pushed) -> import -> pop -> same
Edge: token previously dismissed for this identity → import re-tracks it (existing behaviour)
```

```
Flow: Stop tracking | Entry: ••• Stop tracking (row or detail)
Steps: ConfirmationDialog "Stop tracking {token} for {identity}? You can add it again at any time." -> StopTrackingTokenBalance(pair) -> row leaves
Edge: last row → T2 right empty state with "Track tokens again" (TrackTokenBalancesAgain, one request)
```

## 8. Component and interaction specs

```
Component: IdentityTokensTab (ui/identity/tokens_tab.rs) | Purpose: render the tab from view-state
Variants: populated · empty-untracked · empty-all-dismissed · first-load · stale-error · many (P2)
States: rows default/hover/focus (a fetched zero balance is a normal row, not muted); Refresh default/in-flight(disabled+spinner)/disabled-not-synced;
  hide-zero checkbox on/off (always rendered above the list; hides rows with a fetched 0, never "Checking balance…" rows)
Props: &AppContext, &QualifiedIdentity, &mut IdentityTokensState, ProfileCache not needed
Accessibility: rows are focusable (Tab), Enter opens detail, ••• is a real button with aria-label naming the token;
  status is text; disabled buttons carry disabled_tooltip and the same sentence appears in detail; count badge has a title
Responsive: grid 40|1.4fr|1fr|auto|30; below 640 px the balance+status drop under the name (implemented); actions stay as buttons — folding them into ••• was not built
```

```
State: IdentityTokensState (ui/state/identity_tokens.rs) | renders nothing
Holds: rows (`Arc<[TokenRow]>`, re-read every 5 s or on invalidation), the owning identity, pending refresh / row checks / stop-tracking / track-again requests, retries owed after an in-flight refusal, last refresh time and failure, failed row checks, view prefs (hide-zero, sort, filter; persisted per device), the open detail page and stop-tracking confirmation
Returns: BackendTask for arrival/manual refresh via the hub's single dispatch slot (same rule as home::load_action)
```

Loading per `ux-design-patterns` §8: spinner + text for the first sync;
in-row "Checking balance…" per pair; no blocking overlay for reads. Writes keep
the pushed screens' existing overlays.

Errors per AGENTS.md: what happened + what to do, no jargon, details via
`with_details`. Full copy sheet: wireframes T9.

Accessibility (WCAG 2.2 AA): text-first status, 4.5:1 on all badge text in both
themes, focus ring `BORDER_WIDTH_THICK`, every disabled control has a reason,
menu is a real menu with Escape to close and focus return to the ••• button.
The single-key `/` filter shortcut was not built. The sort offers Claimable
first · Balance · Name; the wireframe's "Recently updated" is not offered (no
per-pair update time exists).

Design tokens: `Spacing::MD` row padding, `Shape::RADIUS_MD` list and card,
`DashColors::surface/border/text_secondary`, `DashColors::SUCCESS` for claim
status text; avatar hue derived from token ID (deterministic), initials from
alias. Two pill tints were added for badges that must read at 4.5:1 on their
own 12 % fill in both themes: `DashColors::pill_accent` (count badge, "Issued
by you") and `DashColors::pill_neutral` ("Can't be sent").

## 9. Superseded and related docs

- **Supersedes** the three-tab strip (`Home · Contacts · Profile`) in
  `2026-10-01-usernames-redesign/wireframes.html` U1, U2, U3 and
  `02-ux-spec.md` §IA — the strip is now four tabs. Nothing else in those
  frames changes.
- **Extends** `2026-04-22-identity-dashpay-redesign/design-spec.md` §A.2 tab
  catalog with row 5 "Tokens"; §A.1 left-nav is unchanged (Tokens root stays).
- **Consistent with** `2026-07-24-identity-home-actions/README.md` (one
  primary per row; label→title alignment — applied here as D2).
- Leaves `2026-04-23-identity-hub-impl` dev plan intact; this is a new phase.

## 10. Implementation plan

**Scope.** Phases 0, 1 and 2 are in scope. **Phase 3 is explicitly out of
scope** (token activity, Buy from row, inline reward estimate) and is listed
only so nobody designs around it by accident. The item-by-item checklist the
developer works from — with tests per item — is
[`implementation-checklist.md`](implementation-checklist.md); it is the
authoritative source for what ships, and this section is the narrative behind
it.

**Phase 0 — prep (≈0.5 day).** Move `IdentityTokenIdentifier`,
`IdentityTokenBalance*`, `IdentityTokenBasicInfo`,
`IdentityTokenAvailableActions`, `get_available_token_actions_for_identity`
from `ui/tokens/tokens_screen/structs.rs` to `model/token/` (mechanical; they
are shared data types, and `tokens/` already imports from `identity/` — a
reverse import would couple the two UI modules). Keep `pub use` shims in
`tokens_screen`. Add a helper on the actions type answering "does this identity
hold any admin permission on this token" that ignores the dev-mode display
override (D3; checklist P0.2).

**Phase 1 — MVP tab (≈2–3 days).**
1. `IdentityHubTab::Tokens` + label + accessible description; `ALL` becomes 4
   (update `tabs.rs` tests `labels_in_display_order`, `all_in_display_order`;
   kittest `identity_hub_switcher.rs` HOME_ONLY_MARKER "Profile" still holds).
2. `ui/state/identity_tokens.rs` view-state; `ui/identity/tokens_tab.rs`
   renderer (`render`, `load_action`, matching the Home/Contacts pattern that tab
   renderers do not themselves return data loads).
3. Hub wiring: render arm, arrival flag in `refresh_on_arrival`, reset on
   identity change, `display_task_result` for `FetchedTokenBalances`,
   `display_task_error` swallowing
   `TokenBalanceRefreshInProgress` and recording per-tab error.
4. Row actions → existing screens; ••• via `action_menu::show_action_menu`
   (Check balance now · Token details · Copy token ID · Stop tracking · Manage
   in Tokens screen under the D3 rule); Stop tracking via `ConfirmationDialog`.
   "Issued by you" badge and "Can't be sent" status on rows.
5. Empty states + Add a token menu; Receive dialog extra sentence
   (`home::render_receive` gains a `tokens_hint: bool`).
6. "Hide tokens with zero balance" checkbox above the list, always visible,
   persisted per device; never hides "Checking balance…" rows (D4). Zero rows
   render as normal rows.
7. Send wording (D2): `TransferTokensScreen` heading, buttons, confirmation,
   progress and success messages; My Tokens' row button "Transfer" → "Send".
   Internal type/fn names unchanged. Affected tests/kittests updated.
8. Tests: unit (row sorting, status derivation, copy constants end with a
   period), kittest (tab renders rows from a seeded snapshot; empty; error
   banner keeps rows; Claim pushes `ClaimTokensScreen`).
9. `docs/user-stories.md`: TOK-021 `[Gap]` → `[Implemented]`.

**Phase 2 (≈2 days).** Home Tokens card (shown iff any balance > 0 or a claim
is pending, D5; Claim reachable from the card when exactly one claim is pending), Token detail page (Send/Claim,
Manage under the D3 rule, Stop tracking), many-token controls (filter and
sort, remembered per device together with hide-zero).

**Phase 3 — OUT OF SCOPE (≈3+ days, needs backend).** Token activity on
detail (token history contract queries), Buy from row when priced
(`QueryTokenPricing` per token), inline reward estimate. Not part of this
design's delivery; T5's "Recent activity" and "Tracked since" are omitted.

Risks:
- Snapshot granularity (per identity) means the whole tab skeletons while any
  identity syncs for the first time; acceptable for MVP, revisit if upstream
  exposes per-pair sync state.
- Shared single-flight refresh guard: a refresh started from My Tokens makes the
  hub's Refresh disabled for its duration — intended, but the tooltip must say
  "Balances are being checked." not "Refresh".
- Send rename (D2) touches strings asserted by existing My Tokens /
  TransferTokensScreen tests; update them in the same change.
- Manage link (D3): the existing `in_dev_mode` path of `into_with_actions`
  flips every admin flag on for Power+ roles; deriving the link from those
  flags would show it to every Power user. Compute with `in_dev_mode = false`
  and ask `has_admin_permission()`.
- 800 px window height (known hub constraint): the list must scroll inside the
  island; header and controls stay fixed (T4).

## 11. Decisions

Answers from the user (2026-10-28) to the questions this note raised; the
same decisions are recorded in `implementation-checklist.md` § "User
decisions", which wins on any discrepancy.

- **D1 Tab order and count badge.** `Home · Contacts · Tokens · Profile`.
  The tab badge shows **only** the number of pending claims; no badge when
  zero. *(Was Q1.)*
- **D2 Verb: Send everywhere.** Holder transfers are called **Send** in the
  hub, in the pushed `TransferTokensScreen` (heading "Send {token}", confirm
  dialog, primary button, progress "Sending…" and success "Sent…" messages,
  breadcrumb) and in Tokens › My Tokens, whose row button "Transfer" becomes
  "Send". Internal type and function names (`TransferTokensScreen`,
  `TokenTask::TransferTokens`, …) stay as they are. *(Was Q2.)*
- **D3 Manage link by permission, not by role.** "Manage in Tokens screen"
  stays in the ••• menu and on the detail page, shown **only when the selected
  identity actually holds at least one administrative permission on that
  token** (mint, burn, freeze, unfreeze, destroy frozen funds, emergency
  action, set price, update config) as authorised by the token's configuration
  rules. Not tied to user role, not tied to being the issuer, and **not**
  derived from the developer-mode override that lights up every button.
  *(Was Q3.)*
- **D4 Zero balances: normal rows, opt-in hiding.** A fetched 0 renders as a
  normal row (no muted styling). A **"Hide tokens with zero balance"** checkbox
  is always visible above the list — not only at ≥ 8 rows — and is remembered
  per device. It never hides "Checking balance…" rows, because an unknown
  balance is not a zero. *(Was Q4.)*
- **D5 Home card threshold.** The Home Tokens card is shown when **any balance
  > 0 or a claim is pending**; otherwise it is hidden. *(Was Q5.)*

## 12. Findings in the existing surface (candy tally)

Confirmed in code while reviewing; fix opportunistically, not blockers for this
design.

| ID | Sev | Finding |
|---|---|---|
| UX-001 | Medium | `my_tokens.rs` `render_no_owned_tokens`: empty state "No Tracked Tokens — You don't have any tokens yet." conflates *tracked* with *owned*; a user who was just sent tokens reads it as "you have nothing". |
| UX-002 | Medium | `my_tokens.rs` `render_token_details`: bare "Check" button for an unknown balance — no object, no explanation; Everyday persona cannot tell it from a checkbox. |
| UX-003 | Low | `my_tokens.rs` `render_actions`: "Token contract not found" — states a cause with no action; violates error rule 2. |
| UX-004 | Low | Resolved: the disabled Send reason in My Tokens now names the cause (`send_disabled_reason` — not transferable, balance still being checked, or nothing held) instead of the circular "not available". |
| UX-005 | Low | `my_tokens.rs` `render_token_details`: contract-owner marker is a hardcoded dark-green `Color32::from_rgb(0,100,0)` on the ID text — colour-only signal, not theme-aware, outside `DashColors`. |
| UX-006 | Low | Holder and issuer actions share one row (`render_actions`): Send/Claim next to Mint/Burn/Freeze/Destroy for the same user; progressive disclosure not applied. |
| UX-007 | Info | Resolved for tokens by D2: Home, the hub, the pushed send screen and My Tokens all say "Send"; Masternodes still says "Claim token rewards ›". |

Tally: 0 Critical · 0 High · 2 Medium · 4 Low · 1 Info — 7 candies, 2 resolved.
