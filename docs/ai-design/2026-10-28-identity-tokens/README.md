# Tokens in the Identity Hub (2026-10-28)

Design record: show the tokens an identity holds inside **Identities › hub**,
where the identity's owner already is, instead of only in the token-first
**Tokens › My Tokens** table. Holder jobs only — every administrative token
action (mint, burn, freeze/unfreeze, destroy frozen funds, pause/resume, set
price, update config, token creator, distributions) stays in the Tokens screen
untouched.

Companion wireframes: [`wireframes.html`](wireframes.html) (T0–T9, light and
dark). Design only; no Rust changed by this note.

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
tracking, and for Expert view a link to the Tokens screen). Reuse the existing
Transfer and Claim screens as-is, pre-scoped to the identity. Phase 2 adds a
Home card (only when there is something to show), a token detail page and
many-token controls. No data model or backend change is needed for the MVP: the
balance snapshot, registry, dismissals and refresh task already exist and are
identity-keyed.

## 2. Personas and jobs

| Persona | Jobs regarding tokens of *this* identity | Where today | Gap |
|---|---|---|---|
| Alex (Everyday) | see what I hold and how much; tell tokens apart; collect rewards/airdrops; send some to a person; know how to receive; understand why a button is grey | Tokens › My Tokens, token-first table | not in the identity's own place; issuer actions in the way; "Check" and "Transfer" are bare verbs; empty state says "No Tracked Tokens" |
| Priya (Power, holder) | the same, per identity; stop tracking noise tokens; see "issued by me" at a glance; jump to admin when she *is* the issuer | same | per-identity view needs a click per token; no issuer badge from the holder side |
| Jordan (Developer) | claim test airdrops fast, verify a transfer landed, see raw IDs | same | fine functionally; benefits from identity-first list on testnet |

Progressive disclosure (`docs/personas/README.md`): Alex sees the tab, rows,
Send/Claim, Add a token. Priya additionally sees "Issued by you" and the
"Manage in Tokens screen" menu item. Jordan sees everything Priya sees; raw
credit/fee context stays on the pushed screens where it already lives.

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
| N12 | Scale to many tokens: filter, sort, hide zeros | Priya, Jordan | P1 | P2 | T4 |
| N13 | Discreet link to admin flows when the identity is the issuer | Priya, Jordan | P2, optional | P2 | menu, T5 |
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
last. The tab bar shows a count badge only for pending claims, never the token
count (a count that is not actionable is noise).

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
    list, one row per tracked token of the selected identity
       row: avatar · name [+badge] · symbol · short ID │ balance · status │ Send · Claim │ •••
       •••: Check balance now · Token details · Copy token ID · Stop tracking · ─ · Manage in Tokens screen (Expert view)
       row click / Enter → Token detail (P2)
    ≥ 8 rows (P2): filter · sort · Hide zero balances
Pushed screens (existing): TransferTokensScreen · ClaimTokensScreen · AddTokenByIdScreen
Root screens (existing): Tokens › Search (find by name) · Tokens › My Tokens (manage)
```

Vocabulary: **Send** (matches Home's Send for Dash; the Tokens screen's
"Transfer" is the same operation — see Q2), **Claim**, **Stop tracking**
(matches TOK-018), **Add a token**. Never in hub copy: transfer, distribution
type, perpetual, pre-programmed, contract owner, state transition, nonce.

## 6. Data source (investigated, not guessed)

Everything the MVP reads already exists and is identity-keyed:

| Need | Existing seam | Notes |
|---|---|---|
| Rows | `AppContext::get_all_known_tokens()` (registry, global per device) minus `untracked_token_balances()` for `(identity, token)` | registry is device-wide; dismissals are per pair |
| Balance | `AppContext::identity_token_balances()` → `IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>`; underneath `WalletBackend::token_balances()` (`UpstreamTokenBalances`, lock-free, frame-safe) | `None` = identity not synced / token not in its synced set → "Checking balance…"; `Some(0)` = real zero (TB2–TB4 in `token_balance.rs`) |
| Actions | `IdentityTokenBalance::into_with_actions(identity, contract, in_dev_mode, pricing)` → `can_transfer`, `can_claim`, `can_maybe_purchase` (ignore the admin flags) | needs `get_contract_by_token_id`; `in_dev_mode = user_role().at_least(Power)` as in My Tokens |
| Once-per-identity | `model::token::once_per_identity_amount(config)`, `AppContext::once_per_identity_claimed_at(token, identity)` | drives "You can claim {amount} once." / "Claimed on {date}." |
| Issuer badge | `contract.owner_id() == identity.id()` | My Tokens colours the ID dark green; the hub uses a text badge |
| Refresh | `TokenTask::QueryMyTokenBalances` (all identities, single-flight; `TokenBalanceRefreshInProgress`, `TokenBalanceRefreshTimeout`), `TokenTask::QueryIdentityTokenBalance(pair)` (also re-tracks), `TokenTask::StopTrackingTokenBalance(pair)` | success sends `TaskResult::Refresh`; in-flight flag `AppContext::token_balance_refresh_in_flight` (crate-visible) |
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
  last snapshot, actions disabled with "Available after sync finishes."
- Refresh is single-flight across the app: the hub and My Tokens share the
  guard. The hub treats `TokenBalanceRefreshInProgress` as "already
  refreshing", never as an error banner.

## 7. Flows

```
Flow: See my tokens | Entry: hub tab bar → Tokens
Steps: 1. Tokens tab (rows from snapshot) -> arrival refresh if last sync > 2 min or never -> rows update in place
       2. Row -> hover/focus shows actions; Enter/click -> Token detail (P2)
Success: balances with "Last updated {time}" | Errors: inline banner, stale rows kept, Try again | Edge: identity unsynced → skeletons; token unsynced → "Checking balance…"; all dismissed → T2 right
```

```
Flow: Claim | Entry: row Claim (or Home card Claim, P2)
Steps: 1. Push ClaimTokensScreen pre-scoped to (identity, token) -> 2. confirm -> blocking progress -> 3. back to hub, success banner, row balance updated
Errors: contract not loadable → "This token's details aren't available yet. Refresh and try again."; network refusal explained by the existing screen (TOK-019)
Edge: once-per-identity already taken → row shows "Claimed on {date}", Claim hidden; view-only identity → Claim disabled with reason
```

```
Flow: Send | Entry: row Send
Steps: 1. Push TransferTokensScreen(IdentityTokenBalance) -> 2. recipient (username or ID) + amount -> 3. confirm -> back, banner, balance updated
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
Edge: last row → T2 right empty state with "Track tokens again"
```

## 8. Component and interaction specs

```
Component: IdentityTokensTab (ui/identity/tokens_tab.rs) | Purpose: render the tab from view-state
Variants: populated · empty-untracked · empty-all-dismissed · first-load · stale-error · many (P2)
States: rows default/hover/focus; Refresh default/in-flight(disabled+spinner)/disabled-not-synced
Props: &AppContext, &QualifiedIdentity, &mut IdentityTokensState, ProfileCache not needed
Accessibility: rows are focusable (Tab), Enter opens detail, ••• is a real button with aria-label naming the token;
  status is text; disabled buttons carry disabled_tooltip and the same sentence appears in detail; count badge has a title
Responsive: grid 40|1.4fr|1fr|auto|30; below 640 px the balance+status drop under the name and actions move into ••• 
```

```
State: IdentityTokensState (ui/state/identity_tokens.rs) | renders nothing
Holds: last_refresh_dispatched_at, arriving flag, per-pair last error, sort/filter/hide-zero (P2), pending stop-tracking confirmation
Returns: BackendTask for arrival/manual refresh via the hub's single dispatch slot (same rule as home::load_action)
```

Loading per `ux-design-patterns` §8: spinner + text for the first sync;
in-row "Checking balance…" per pair; no blocking overlay for reads. Writes keep
the pushed screens' existing overlays.

Errors per AGENTS.md: what happened + what to do, no jargon, details via
`with_details`. Full copy sheet: wireframes T9.

Accessibility (WCAG 2.2 AA): text-first status, 4.5:1 on all badge text in both
themes, focus ring `BORDER_WIDTH_THICK`, every disabled control has a reason,
menu is a real menu with Escape to close and focus return to the ••• button,
single-key `/` filter shortcut only when the list has focus (2.1.4).

Design tokens: `Spacing::MD` row padding, `Shape::RADIUS_MD` list and card,
`DashColors::surface/border/text_secondary`, `DashColors::SUCCESS` for claim
status text, `DashColors::DASH_BLUE` for the count badge; avatar hue derived
from token ID (deterministic), initials from alias. No new colour constants.

## 9. Superseded and related docs

- **Supersedes** the three-tab strip (`Home · Contacts · Profile`) in
  `2026-10-01-usernames-redesign/wireframes.html` U1, U2, U3 and
  `02-ux-spec.md` §IA — the strip is now four tabs. Nothing else in those
  frames changes.
- **Extends** `2026-04-22-identity-dashpay-redesign/design-spec.md` §A.2 tab
  catalog with row 5 "Tokens"; §A.1 left-nav is unchanged (Tokens root stays).
- **Consistent with** `2026-07-24-identity-home-actions/README.md` (one
  primary per row; label→title alignment — applied here as Q2).
- Leaves `2026-04-23-identity-hub-impl` dev plan intact; this is a new phase.

## 10. Implementation plan

**Phase 0 — prep (≈0.5 day).** Move `IdentityTokenIdentifier`,
`IdentityTokenBalance*`, `IdentityTokenBasicInfo`,
`get_available_token_actions_for_identity` from
`ui/tokens/tokens_screen/structs.rs` to `model/token/` (mechanical; they are
shared data types, and `tokens/` already imports from `identity/` — a reverse
import would couple the two UI modules). Keep `pub use` shims in `tokens_screen`.

**Phase 1 — MVP tab (≈2–3 days).**
1. `IdentityHubTab::Tokens` + label + accessible description; `ALL` becomes 4
   (update `tabs.rs` tests `labels_in_display_order`, `all_in_display_order`;
   kittest `identity_hub_switcher.rs` HOME_ONLY_MARKER "Profile" still holds).
2. `ui/state/identity_tokens.rs` view-state; `ui/identity/tokens_tab.rs`
   renderer (`render`, `load_action`, matching the Home/Contacts pattern that tab
   renderers do not themselves return data loads).
3. Hub wiring: render arm, arrival flag in `refresh_on_arrival`, reset on
   identity change, `display_task_result` for `FetchedTokenBalances` /
   `TokenBalanceRefreshAlreadyInFlight`, `display_task_error` swallowing
   `TokenBalanceRefreshInProgress` and recording per-tab error.
4. Row actions → existing screens; ••• via `action_menu::show_action_menu`;
   Stop tracking via `ConfirmationDialog`.
5. Empty states + Add a token menu; Receive dialog extra sentence
   (`home::render_receive` gains a `tokens_hint: bool`).
6. Tests: unit (row sorting, status derivation, copy constants end with a
   period), kittest (tab renders rows from a seeded snapshot; empty; error
   banner keeps rows; Claim pushes `ClaimTokensScreen`).
7. `docs/user-stories.md`: TOK-021 `[Gap]` → `[Implemented]`.

**Phase 2 (≈2 days).** Home Tokens card (conditional), Token detail page,
many-token controls (filter/sort/hide-zero, remembered per device), "Issued by
you" + Manage link (Expert view).

**Phase 3 (≈3+ days, needs backend).** Token activity on detail (token history
contract queries), Buy from row when priced (`QueryTokenPricing` per token),
inline reward estimate.

Risks:
- Snapshot granularity (per identity) means the whole tab skeletons while any
  identity syncs for the first time; acceptable for MVP, revisit if upstream
  exposes per-pair sync state.
- Shared single-flight refresh guard: a refresh started from My Tokens makes the
  hub's Refresh disabled for its duration — intended, but the tooltip must say
  "Balances are being checked." not "Refresh".
- Heading mismatch on pushed screens (Q2) if left as-is.
- 800 px window height (known hub constraint): the list must scroll inside the
  island; header and controls stay fixed (T4).

## 11. Open questions for the user

- **Q1 Tab order and count badge.** `Home · Contacts · Tokens · Profile` with a
  badge only for pending claims — agreed? (Alternative: no badge at all in MVP.)
- **Q2 Verb.** "Send" in the hub (matches Home) while the pushed screen is titled
  "Transfer Tokens". Rename the pushed headings to "Send {token}" / "Claim
  {token} rewards" in Phase 1 (small string change on existing screens), or use
  "Transfer" in the hub?
- **Q3 Expert-view link.** Keep the optional "Manage in Tokens screen" menu
  item for Power/Developer when the identity is the issuer, or drop any admin
  link from the hub entirely?
- **Q4 Zero-balance rows.** Show tracked tokens with a fetched 0 (sorted last,
  muted) or hide them by default behind the "Hide zero balances" toggle from
  day one? Proposal: show (Alex waiting for a payment needs to see the row).
- **Q5 Home card threshold.** Show the card when any balance > 0 *or* a claim
  waits (proposal), or only when a claim waits?

## 12. Findings in the existing surface (candy tally)

Confirmed in code while reviewing; fix opportunistically, not blockers for this
design.

| ID | Sev | Finding |
|---|---|---|
| UX-001 | Medium | `my_tokens.rs:265` empty state "No Tracked Tokens — You don't have any tokens yet." conflates *tracked* with *owned*; a user who was just sent tokens reads it as "you have nothing". |
| UX-002 | Medium | `my_tokens.rs:478` bare "Check" button for an unknown balance — no object, no explanation; Everyday persona cannot tell it from a checkbox. |
| UX-003 | Low | `my_tokens.rs:744` "Token contract not found" — states a cause with no action; violates error rule 2. |
| UX-004 | Low | `my_tokens.rs:711` disabled-Transfer reason "Transfer not available" is circular. |
| UX-005 | Low | `my_tokens.rs:466` contract-owner marker is a hardcoded dark-green `Color32::from_rgb(0,100,0)` on the ID text — colour-only signal, not theme-aware, outside `DashColors`. |
| UX-006 | Low | Holder and issuer actions share one row (`render_actions`): Transfer/Claim next to Mint/Burn/Freeze/Destroy for the same user; progressive disclosure not applied. |
| UX-007 | Info | Vocabulary split: Home says "Send", My Tokens says "Transfer", Masternodes says "Claim token rewards ›" — same operations, three verbs. |

Tally: 0 Critical · 0 High · 2 Medium · 4 Low · 1 Info — 7 candies.
