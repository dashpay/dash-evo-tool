# Masternodes Page — UX Specification

**Repo:** `dash-evo-tool` · **Date:** 2026-07-09 · **Author:** Diziet
Companion: `01-requirements.md`, `wireframes.html`. Design tokens: `src/ui/theme.rs`, `docs/ux-design-patterns.md`.

All wireframes are annotated with the **existing component / token** they reuse. Nothing here invents a new
widget where an existing one serves.

---

## 1. Information Architecture

```
Left nav (root screens)
├── Dashpay
├── Identities
├── Identity Hub
├── Masternodes        ◀── NEW root tab (this design)
├── Contracts
└── … (Dash / network)
```

The Masternodes tab is a **sibling root screen** (persists in `AppState.main_screens`, survives network
switch). **Corrected 2026-07-09 (supersedes the original NFR-7):** it IS gated behind **Expert Mode**
(`is_developer_mode()`, user-facing label "Expert mode") — nav item and route both absent when Expert Mode
is off. Node-operator work is a distinct audience (Priya), and Expert Mode is DET's existing mechanism for
separating that audience from Alex (Everyday User) — see FR-1.

Internal screen stack within Masternodes:
```
Masternodes (list / empty state)  ──"Load a masternode"──▶  Load form  ──success──▶ back to list
        │
        └──click a card──▶  Masternode detail / voting  ──"Remove"──▶ confirm ──▶ back to list
```
Navigation uses the standard breadcrumb header + `PushScreen`/`PopScreen` for the load form and detail view.

---

## 2. User Journeys

### 2.1 First-time (empty → load a node)
Persona: Priya. Entry: clicks **Masternodes** in the left nav for the first time.
1. Sees the **empty state** card (§ wireframe A): what a masternode identity is for + a primary
   **Load a masternode** button.
2. Clicks it → **Load form** (wireframe C): enters ProTxHash, picks Masternode/Evonode, pastes
   Voting/Owner/Payout keys (manual paste only — see §4c, auto-derive does not apply to these key roles).
   Reads the plaintext-at-rest note.
3. Clicks **Load masternode** → returns to the list, now showing **one card** (wireframe B).
Success state: a card representing her node, with voter readiness + key status visible.
Failure paths: empty ProTxHash → button disabled with tooltip; load error → `MessageBanner` (Error) with a
user-friendly message and technical detail attached (never a raw error string).

### 2.2 Returning (list → open → vote)
Persona: Priya, on a later session with nodes already loaded.
1. Opens **Masternodes** → **card list** (wireframe B). Scans type badges, voter readiness, and
   "N contests to vote on".
2. Clicks a card → **detail/voting view** (wireframe D). Reviews keys summary + voter identity.
3. In the DPNS voting section, for each open contested name chooses **Abstain / Lock / a candidate**; vote is
   dispatched via the existing DPNS voting backend.
Success state: vote recorded (Success `MessageBanner`, auto-dismiss).
Edge case: node has no voter identity → voting section shows an actionable message ("Add its voting private
key to cast votes"), not a raw `NoVotingIdentity` error.

### 2.3 Housekeeping (remove a node)
1. Detail view → **Remove masternode** (danger button) → confirmation dialog (specific verb, Cancel left /
   Remove right, Escape cancels).
2. Confirm → node **and its associated voter identity** are forgotten; back to the list.

---

## 3. Interaction Patterns

- **Cards** — reuse `IdentityPickerCard` visual language (`src/ui/identity/identity_picker_card.rs`): rounded
  `surface` card (`RADIUS_LG`=16), monogram, type-badge pill, single click target with hover elevation and
  `WidgetInfo::labeled` a11y. Extend content to masternode-specific rows (voter readiness / key status /
  voting status) — same frame, different body.
- **Type badge** — reuse `draw_type_badge`: Masternode → `PLATFORM_PURPLE`, Evonode → `DASH_BLUE`, white text.
- **Node-type toggle** (load form) — segmented control using `unselected_fill(dark_mode)` for the inactive
  segment and `DASH_BLUE` for the active, matching the existing "Identity ID & private key / From my wallet /
  My username" tab styling in `04-load-masternode-keys.png`.
- **Key inputs** — reuse the existing private-key input widget (WIF-or-hex placeholder, reveal control) already
  used by the advanced arm. Password-input reveal rules per `docs/ux-design-patterns.md` §5.
- **Buttons** — `ComponentStyles`: `add_primary_button` (Dash-blue) for Load; `add_secondary_button` (outline)
  for Cancel; `add_danger_button` for Remove. `add_primary_button_enabled(false, …)` + `disabled_tooltip` for
  the disabled Load state. Top-right **Refresh** uses `add_toolbar_button` on the network accent (as in
  `01-dashpay.png`).
- **Collapsing sections** — the DPNS voting section on the detail view is an egui **collapsing header**,
  **collapsed by default**, with the open contest **count in the header** (`▸ DPNS name contests to vote on (3)`)
  so operators still see there's something to act on without expanding. Expanding (`▾`) reveals the voting table +
  Cast-votes button. Use the standard egui `CollapsingHeader` pattern.
- **Status** — identity status dot uses the `IdentityStatus → Color32` mapping (green/gray/orange/red) always
  paired with a text label (never colour-only — NFR-6).
- **Global-nav pills** — the existing `BreadcrumbPillMode` already provides the exact three renderings this model
  needs: `Interactive` (caret + dropdown, for a consumed selection), `Subdued` (dimmed, no caret + hover tooltip —
  the disabled fallback for unwired pills; no visible text tag), `Placeholder` (no value yet, e.g. the empty-state
  masternode pill). On Masternodes **both** the wallet pill and the masternode pill are `Interactive` and two-way
  bound. A page declares which selections it consumes; consumed pills bind two-way, others render `Subdued` with a
  how-to-change tooltip.
- **Messages** — `MessageBanner::set_global`; errors persistent + `.with_details(e)`, success auto-dismiss.
- **Confirmation** — `ConfirmationDialog` with `danger_mode(true)` for Remove.

---

## 4. Navigation / Breadcrumb — global wallet/identity switcher

**Change (FR-GLOBAL-NAV):** the top island's left region now hosts the **global wallet/identity switcher** on
every root page, not just the Identity Hub. It is the exact three-segment breadcrumb switcher from IDH-003
(`breadcrumb_switcher.rs`) — `Identities › 💼 wallet › 👤 identity` — rendered via
`top_panel::add_top_panel_with_breadcrumb`. The connection dot sits to its left; Refresh/action buttons stay
top-right. The switcher looks and behaves identically to the Identity Hub (`05-identity-hub-landing.png`).

- **Which page am I on?** Conveyed by the **left-nav rail highlight** (Masternodes active), not by the header.
- **Sub-screen navigation** (load form, detail) uses a **content-panel back row** (`‹ All masternodes`), keeping
  the global switcher single-line and unchanged across navigation (design-spec §A.3).
- The status/connection dot reflects network colour (orange on Testnet, as in the reference screenshots) and
  reuses `top_panel::add_connection_indicator`.

Header on every Masternodes screen: `[●dot]  Masternodes › 💼 Main Wallet ▾ › Ⓜ mn-east-01 ▾` — **page-aware**
leftmost crumb (confirmed Q1). On Masternodes **both pills are interactive** (caret ▾): the **wallet pill**
(funds Top up on this node, two-way bound — see §4c, NOT a key-derivation source) **and** the **masternode pill**
(the node in view — its dropdown lists loaded masternodes/evonodes, two-way bound with the card grid + detail
view; see §4b). The third pill is **page-scoped**: it is the masternode-in-view here, and the app-global User
identity on everyday-user pages.

## 4b. Global nav — design question & resolution

**Question (from the coordinator):** on non-identity pages (Masternodes, Contracts), what does the wallet/identity
selector scope to?
- **(a)** It always reflects the same app-global wallet + (User) identity context, independent of page.
- **(b)** On the Masternodes page the identity pill instead reflects the selected **masternode/evonode** identity
  (page-aware), since masternode identities are `IdentityType != User`.

**Resolution → the third pill is a page-scoped, page-aware object** (updated; supersedes the earlier
"Option (a) / read-only on Masternodes"). The third segment shows the identity/object the **current page**
operates on:
- **Everyday-user pages** (Dashpay / Identities / Identity Hub): the app-global **User** identity.
- **Masternodes page**: the **masternode/evonode in view** (`Ⓜ mn-east-01 ▾`), interactive and two-way bound with
  the card grid and the detail view. Its dropdown lists the loaded masternodes/evonodes.

**Why this does NOT violate FR-6 (the boundary).** FR-6 governs the **everyday-user Identity Hub / Identities
picker** — it must not list MN/Evonode identities. The masternode-in-view here is a **separate, page-scoped
selection**, distinct from the app-global User-identity selection. Choosing a masternode in the Masternodes
switcher does not touch the app-global user-identity selection, so it **never** appears in the identity pill on
user pages nor in the Hub picker. Two selections, two scopes — the Masternodes page's own switcher is simply not
the everyday-user picker FR-6 constrains. (The masternode pill's dropdown is also *not* the Hub picker; it is the
operator surface, which is exactly where MN/Evonode identities belong.)

**Two-way binding on Masternodes.** The masternode pill mirrors the page's own selection: opening a card sets the
pill; picking a node from the pill opens that node's detail. The detail view additionally carries a
`‹ All masternodes` content-panel back row to return to the grid.

### Authoritative selection interaction model (resolved)

The global nav follows four rules on every page (full text in requirements FR-GLOBAL-NAV-2):

1. **Silent context change** — selecting an object updates the app-global selection; **no forced navigation**.
2. **Two-way binding where the page consumes the selection** — nav pill and page stay in sync both ways.
3. **Blast-radius control** — a pill is interactive only on pages already wired to consume it; elsewhere it is
   **dimmed, no caret, no visible tag**, with a **hover tooltip telling the user how to change that selection**
   (+ a `TODO` in code). Interactivity rolls out page-by-page.
4. **Per-page composition** — show only the pills that make sense (a Wallet page shows only the wallet pill).

**On Masternodes:** **both pills are interactive** — wallet pill = two-way bound (funds Top up on this node, §4c);
masternode pill = the node in view, two-way bound with the card grid + detail (its dropdown lists loaded
masternodes/evonodes). The third pill is page-scoped, so it never leaks a masternode into the user pages (FR-6).

**Two-way-binding example to carry into implementation (Send/Transfer-from-wallet):** changing the wallet in the
top-nav changes the page's *source wallet*; changing the *source wallet* on the page updates the top-nav wallet
pill. This is the canonical shape for any page that consumes a selection — on Masternodes the source wallet feeds
**Top up** (FR-9), not key derivation (§4c).

**Confirmed Q1:** leftmost crumb is **page-aware** (`Masternodes › …`), linking to the active tab's root.

## 4c. Auto-derive finding — corrected (investigation, 2026-07-09)

**Auto-deriving Voting/Owner/Payout private keys from a loaded wallet does NOT work and was never wired for
Masternode/Evonode identities.** Verified against code: `backend_task/identity/load_identity.rs` gates
`derive_keys_from_wallets` to `IdentityType::User` only; for Masternode/Evonode the three key fields are always
manual paste, verified (not discovered) against the identity's on-chain public keys. This is architectural, not a
missing feature: masternode owner/voting/payout keys are Core-side keys tied to the node's ProRegTx, not part of
any wallet's identity-auth HD derivation tree. Consequence:
- **Load form (wireframe C):** the "Try to derive these keys from a loaded wallet" checkbox is **removed** — it
  would be misleading UI chrome (a no-op) on a page that is masternode-only (no User option here).
- **Wallet pill rationale corrected:** the wallet pill on the Masternodes page is NOT a key-derivation source.
  Its real, verified purpose is as the **funding source for Top up** (FR-9) — Top up moves DASH from the active
  wallet to the node's identity balance, a genuine use of "active wallet" context. The pill stays interactive and
  two-way bound for that reason.
- Superseded: **US-6 ("auto-derive parity")** is retired — see 01-requirements.md backlog note.

---

## 5. Accessibility (WCAG 2.1 AA)

- Card is one labelled click target (`WidgetInfo::labeled(Button, …, "Open {node}")`); Enter activates.
- Focus order: header actions → cards (reading order) / form fields top-to-bottom → primary action last.
- Focus indicator: `BORDER_WIDTH_THICK`, ≥3:1 contrast (theme default).
- No colour-only status: every status dot and badge carries a text label.
- Disabled Load button uses `disabled_tooltip` (NotAllowed cursor) explaining the blocker.
- Contrast: Dash-blue `#008de4` on white for primary buttons; secondary text `#64788c` meets AA on white.
- Known constraint (documented): egui offers no screen-reader annotations beyond `WidgetInfo`.

---

## 6. Responsive Behavior

- Card grid: `minmax(260px, 1fr)` columns (matches `CARD_MIN_WIDTH`=260), wrapping to 1 column on narrow
  widths via `ui.available_width()`; `ScrollArea` for overflow. Empty-state and forms sit inside
  `island_central_panel()` responsive margins.
- Load form: single-column, labels above inputs on narrow widths.

---

## 7. ASCII Wireframes

> **Erratum (PROJ-013, 2026-07-09):** `wireframes.html` still draws the legacy **two** Fill-Random buttons
> ("Fill Random HPMN" / "Fill Random Masternode") and omits the missing-voter "Add voting key" affordance.
> FR-12/§7 (one button, label follows the Node-type toggle) and wireframe D below are canonical; the HTML mock is
> stale for these two details only.

Legend: `[●]` status dot · `[ Button ]` primary · `( Button )` secondary/outline · `‹ Button ›` danger ·
`{MN}`/`{EVO}` type badge pill.

### (A) Masternodes tab — empty state
Reuses: **global switcher** header (`breadcrumb_switcher.rs` via `add_top_panel_with_breadcrumb`, styled per
`05-identity-hub-landing.png`), empty-state card pattern (`03-identities-empty.png`), `add_primary_button`,
`island_central_panel`. Nav rail highlights **Masternodes**.

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ [●] Masternodes › 💼 Main Wallet ▾ › Ⓜ (no masternode yet)         [ Refresh ] │  ← both pills interactive; MN pill is a placeholder (none loaded yet)
├───────────┬───────────────────────────────────────────────────────────────────┤
│  ▽ Dash   │                                                                     │
│  ⦿ Ident. │        ┌───────────────────────────────────────────────────┐      │
│  ⦿ IdHub  │        │            No masternodes loaded                    │      │  ← empty-state card (surface, RADIUS_LG)
│ ▶Masterno.│        ├───────────────────────────────────────────────────┤      │
│  ⦿ Contr. │        │  Load a masternode or evonode to vote on DPNS name  │      │
│  ~Dash~   │        │  contests and manage its owner and payout keys.     │      │
│           │        │                                                     │      │
│           │        │                 [ Load a masternode ]               │      │  ← primary (DASH_BLUE)
│           │        │                                                     │      │
│           │        │  Have your node's ProTxHash to hand. Keys are      │      │  ← canonical wording, §7
│           │        │  optional — a node loads read-only without them.   │      │
│           │        └───────────────────────────────────────────────────┘      │
│           │                                                                     │
└───────────┴───────────────────────────────────────────────────────────────────┘
```
Note: **both pills are interactive** on Masternodes. The **wallet pill** (▾) opens the wallet dropdown (two-way
bound — funds Top up on this node, §4c; not a key-derivation source). The **masternode pill** (▾) is the
page-scoped node-in-view selector; here it is a
placeholder because none is loaded yet. This third pill is page-scoped — it shows the app-global User identity on
everyday-user pages, never a masternode there (§4b, FR-6 boundary).

### (B) Masternodes tab — card list (2–3 cards)
Reuses: `IdentityPickerCard` frame + `draw_type_badge` + monogram + hover elevation; `IdentityStatus` colour
dot; responsive `minmax(260,1fr)` grid.

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ [●] Masternodes › 💼 Main Wallet ▾ › Ⓜ Choose a masternode ▾  [ + Load ][⟳]    │  ← both pills interactive; MN pill dropdown mirrors the cards below
├───────────┬───────────────────────────────────────────────────────────────────┤
│  ▽ Dash   │  ┌────────────────────────┐  ┌────────────────────────┐            │
│  ⦿ Ident. │  │ (M)            {MN}     │  │ (E)            {EVO}    │            │  ← monogram + type badge
│  ⦿ IdHub  │  │  mn-east-01            │  │  6f2a…c19b             │            │  ← alias (or shortened ProTxHash)
│ ▶Masterno.│  │  9a3f…d7e2  ·ProTxHash │  │  Evonode               │            │  ← sub-line
│  ⦿ Contr. │  │                        │  │                        │            │
│  ~Dash~   │  │  ● Voting ready        │  │  ▲ No voting key       │            │  ← voter readiness (green / warning)
│           │  │  Keys: V O P           │  │  Keys: · O ·           │            │  ← key status (present emphasised)
│           │  │  3 contests to vote on │  │  No open contests      │            │  ← DPNS voting status
│           │  │  ● Active              │  │  ● Unknown             │            │  ← IdentityStatus dot + label
│           │  └────────────────────────┘  └────────────────────────┘            │
│           │  ┌────────────────────────┐                                        │
│           │  │ (M)            {MN}     │                                        │
│           │  │  mn-west-02            │                                        │
│           │  │  b71c…40aa  ·ProTxHash │                                        │
│           │  │  ● Voting ready        │                                        │
│           │  │  Keys: V O ·           │                                        │
│           │  │  Vote scheduled        │                                        │
│           │  │  ● Active              │                                        │
│           │  └────────────────────────┘                                        │
└───────────┴───────────────────────────────────────────────────────────────────┘
```

### (C) Load a masternode form
Reuses: global switcher header; segmented toggle styled like existing tab row (`04-load-masternode-keys.png`);
existing private-key input widget; the password-input pattern (`wallet_unlock.rs` hold-to-reveal, see
`docs/ux-design-patterns.md` §5) for the optional encryption password; `add_primary_button_enabled` +
`disabled_tooltip`; Warning-tone inline note. **New (FR-8):** the optional encryption-password field needs new
plumbing (password threaded through `IdentityInputToLoad` → `load_identity` → existing `store_protected`).
**FR-12 (new, investigated 2026-07-09):** carries forward the "Fill Random Masternode" / "Fill Random HPMN"
dev convenience from `add_existing_identity_screen.rs:203-208` (`fill_random_masternode()` / `fill_random_hpmn()`,
lines 961-993) — picks a random REAL testnet node from a local `.testnet_nodes.yml` fixture and autofills
ProTxHash + keys; it does not fabricate a synthetic node. One button, labelled to match the Node-type toggle
above ("Fill Random Masternode" / "Fill Random Evonode"). **Fixture facts:** `.testnet_nodes.yml` is gitignored,
not tracked, and does not exist in this repo — it must be supplied locally and **does contain real private keys
in plaintext**. **Visibility is conditional, not disabled-state:** the button renders only when
`load_testnet_nodes_from_yml(...)` returns `Some(_)` on Testnet; otherwise it is absent entirely, no placeholder.
Gating is simpler than first assumed: the whole Masternodes tab now requires **Expert Mode** (FR-1), so this
button's own remaining condition is just Testnet + fixture-present — no separate `developer_mode` re-check
needed for normal navigation (Nagatha may still add one at the call-site as defense-in-depth; judgment call).

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ [●] Masternodes › 💼 Main Wallet ▾ › Ⓜ Choose a masternode ▾                    │  ← both pills interactive; unchanged during sub-nav
├───────────┬───────────────────────────────────────────────────────────────────┤
│  nav …    │  ‹ All masternodes                                                  │  ← content-panel back row (§4b), not in header
│           │  Load a masternode                                                  │
│           │  Load a masternode or evonode that already exists on the network.   │
│           │                                                                     │
│           │  Node type:   [ Masternode ]  ( Evonode )                           │  ← segmented toggle (active=DASH_BLUE)
│           │                                                                     │
│           │  ( 🎲 Fill Random Masternode )  Testnet-only dev convenience —      │  ← FR-12, testnet-only; carried over from
│           │    fills a real test node's ProTxHash and keys below.              │    add_existing_identity_screen.rs
│           │                                                                     │
│           │  ProTxHash:            [___________________________________]  (i)   │  ← required
│           │  Alias (optional):     [___________________________________]  (i)   │
│           │  Voting private key:   [ Private key (WIF or hex)         ] 👁 ⊘    │
│           │  Owner private key:    [ Private key (WIF or hex)         ] 👁 ⊘    │
│           │  Payout addr. key:     [ Private key (WIF or hex)         ] 👁 ⊘    │
│           │  Encryption password   [ Password to encrypt these keys   ] 👁      │  ← OPTIONAL (FR-8); reuses password-input pattern
│           │    (optional):         Set a password to encrypt these keys on this │  ← helper line
│           │                        device. Leave it blank to store them         │
│           │                        unencrypted and add protection later.        │
│           │                                                                     │
│           │  ⚠ Set an optional password to encrypt these keys on this device.   │  ← Warning-tone, non-blocking, ACCURATE (FR-8/NFR-4)
│           │    Without one, they are stored unencrypted and you can add          │
│           │    protection later from the key screen.                            │
│           │                                                                     │
│           │  [ Load masternode ]   ( Cancel )                                   │  ← primary disabled until ProTxHash set
│           │    Enter a ProTxHash to continue.                                   │  ← disabled tooltip
└───────────┴───────────────────────────────────────────────────────────────────┘
```

### (D) Masternode detail / voting view
Reuses: breadcrumb header; type badge; `shorten_id` + copy affordance; DPNS voting backend (surfaced, not
re-implemented); `add_danger_button` + `ConfirmationDialog`.

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ [●] Masternodes › 💼 Main Wallet ▾ › Ⓜ mn-east-01 ▾                  [ Refresh ]│  ← MN pill shows the node in view; two-way bound with the list + this detail
├───────────┬───────────────────────────────────────────────────────────────────┤
│  nav …    │  ‹ All masternodes                                                  │  ← content-panel back row (§4b)
│           │  mn-east-01   {MN}   9a3f…d7e2 ⧉        ● Active                    │  ← alias + badge + ProTxHash(copy) + status
│           │  ───────────────────────────────────────────────────────────────   │
│           │  Actions:  [ Withdraw ]  [ Top up ]  [ Transfer ]                    │  ← reuse withdraw / top_up / transfer screens, scoped to this node
│           │  ⓔ Evonode only:  ( Claim token rewards › )                          │  ← Evonode-only cross-link → ClaimTokensScreen (hidden for Masternode)
│           │  ───────────────────────────────────────────────────────────────   │
│           │  Keys                              Keys: unprotected  ( Add password │
│           │   Voting: loaded ✓  Owner: loaded ✓  Payout: loaded ✓    protection…)│  ← surfaces existing IdentityTask::ProtectIdentityKeys
│           │   Voter identity: 4c8e…1b70 ⧉                    ( Manage keys › )    │  ← opens existing KeyInfoScreen (view WIF, sign, add/remove key)
│           │  ───────────────────────────────────────────────────────────────   │
│           │  ▸ DPNS name contests to vote on (3)                                │  ← COLLAPSIBLE, collapsed by default; count stays visible
│           │  ───────────────────────────────────────────────────────────────   │
│           │  … expanded (▾), the same header reveals the voting table:          │
│           │   ┌─────────────────────────────────────────────────────────────┐ │
│           │   │ alice        ○ Abstain   ○ Lock   ○ Vote for: [candidate ▾] │ │
│           │   │ cooltoken    ○ Abstain   ○ Lock   ○ Vote for: [candidate ▾] │ │
│           │   │ dashfan      ○ Abstain   ○ Lock   ○ Vote for: [candidate ▾] │ │
│           │   └─────────────────────────────────────────────────────────────┘ │
│           │                                              [ Cast votes ]         │
│           │  ───────────────────────────────────────────────────────────────   │
│           │  ‹ Remove masternode ›                                              │  ← danger + confirmation dialog
└───────────┴───────────────────────────────────────────────────────────────────┘
```
**Evonode detail differs by:** the `( Claim token rewards › )` cross-link is shown **only** for Evonode
identities (routes to the existing `ClaimTokensScreen`); a plain Masternode hides it. Everything else is identical
across the two node types.

**Withdraw destination rule (FR-9):** with the owner key the destination is forced to the node's registered Core
payout address; with the transfer/payout key it is a free address. **Add-key rule (FR-10):** the purpose selector
excludes OWNER/VOTING for all identity types.

Empty voting state (no voter identity): the contests block is replaced by —
`This node has no voting key loaded. Add its voting private key to cast votes.` with a `( Add voting key )`
secondary action routing back to the load form pre-filled with this ProTxHash.

---

## 8. Component Reuse Summary

| Screen element | Existing asset reused |
|---|---|
| Global wallet/identity switcher (header) | `breadcrumb_switcher.rs` + `breadcrumb_pill.rs` (`Interactive`/`Subdued`/`Placeholder` modes already exist) + `identity_pill.rs`, via `top_panel::add_top_panel_with_breadcrumb`; app-scoped + page-scoped selection. **On Masternodes:** wallet pill `Interactive` (two-way) **and** masternode pill `Interactive` (two-way with card grid + detail). **New:** a page-scoped masternode selection distinct from the app-global user-identity selection (Nagatha) |
| Root-tab header + status dot + toolbar action | `top_panel::render_top_island` + `add_connection_indicator` + `add_toolbar_button` |
| Sub-screen back row (content panel) | lightweight label/link inside `island_central_panel` (no new widget) |
| Left nav rail (icon + label) | existing nav rail |
| Empty-state card | `03-identities-empty.png` pattern, `surface`/`RADIUS_LG`/`Shadow::medium` |
| Node cards | `IdentityPickerCard` frame, monogram, hover elevation, `WidgetInfo::labeled` |
| Type badge pill | `draw_type_badge` (PLATFORM_PURPLE / DASH_BLUE) |
| Node-type toggle | existing segmented tab styling |
| Key inputs | existing WIF-or-hex private-key input widget |
| Optional encryption password (load form) | password-input pattern (`wallet_unlock.rs` hold-to-reveal). **Backend NEW:** thread password through `IdentityInputToLoad` → `load_identity` → existing `store_protected`/`put_secret_protected` (Nagatha's scope) |
| Buttons | `ComponentStyles` primary / secondary / danger / toolbar |
| Status dot | `IdentityStatus → Color32` mapping (+ text label) |
| Messages / errors | `MessageBanner::set_global` + `.with_details` |
| Add-password-protection action | existing `IdentityTask::ProtectIdentityKeys` (Key Info screen's "Add password protection…"); Masternodes detail view reuses it — no new crypto |
| Credit actions (Withdraw / Top up / Transfer) | existing `withdraw_screen` / `top_up_identity_screen` / `transfer_screen`, scoped to the node's `QualifiedIdentity` (FR-9). Only the entry points are new |
| Manage keys drill-in | existing `KeyInfoScreen` scoped to the node (FR-10): view WIF, sign, add/remove key. Add-key purpose selector excludes OWNER/VOTING |
| Evonode token rewards | existing `ClaimTokensScreen` cross-link, Evonode-only (FR-11). Route, not rebuild |
| Remove confirmation | `ConfirmationDialog` `danger_mode(true)` |
| DPNS voting | existing `contested_names/vote_on_dpns_name.rs` backend + DPNS root screens |

---

## Locked decisions (accepted 2026-07-09)

Human accepted the wireframes; all four answers match the mock as-drawn, so `wireframes.html` is unchanged.
Binding for implementation:

1. **Voting depth = INLINE** — wireframe (D) stands: cast votes in the detail view via the DPNS-contest table
   + **Cast votes** button. No deep-link to the DPNS Active Contests root screen.
2. **Filter scope = HUB PICKER ONLY** — remove MN/Evonode from the Identity Hub picker only. The legacy
   Identities table keeps them; stripping it is a **deferred follow-up PR** (out of scope, not a regression).
3. **Nav placement = BELOW IDENTITY HUB** — order Dashpay / Identities / Identity Hub / Masternodes / Contracts
   / Dash, with a distinct node/server glyph (as drawn in the wireframes' rail).
4. ~~**Auto-derive = all three key roles** (Voting / Owner / Payout), matching current behaviour.~~ —
   **SUPERSEDED 2026-07-09** (post-acceptance investigation; matches `01-requirements.md` Locked-decisions #4). The
   load form has **no auto-derive affordance**: `derive_keys_from_wallets` is hard-gated to `IdentityType::User`, so
   masternode Voting/Owner/Payout keys are always pasted manually. See §4c. US-6 retired.

---

**Correction folded in (2026-07-09, per CLAUDE.md update):** MN/Evonode keys are not permanently plaintext — they
load Tier-1 (unprotected, no password field by design) and can be sealed to Tier-2 per-identity afterward via the
existing `IdentityTask::ProtectIdentityKeys`. The load-form note is now actionable ("you can protect this node's
keys after loading it") and the detail view surfaces the protection tier + an "Add password protection…" action
(reuse, no new crypto).

🍬 **Findings tally (UX)** — **1** usability improvement confirmed (Info): the missing-voter-identity path
must present an actionable "Add voting key" affordance instead of surfacing the raw `NoVotingIdentity` error
(carried into wireframe D empty state and US-3 acceptance criteria).
