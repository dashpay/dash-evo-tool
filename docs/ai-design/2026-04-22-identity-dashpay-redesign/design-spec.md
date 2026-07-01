# Identity + DashPay Redesign — UX Specification

**Target**: Dash Evo Tool 2, `v1.0-dev`
**Date**: 2026-04-22
**Author**: Trillian (Technical Writer)
**Status**: Approved — implementation reference

---

## Orientation

This spec collapses the current two left-nav entries — **Dashpay** and **Identities** — into
one unified section called **Identities**.

**Critical distinction carried throughout this document:**

- **Identity** is the primary on-chain Dash Platform object. It owns keys, DPNS usernames,
  a credit balance, and optional documents. Every operation on Dash Platform requires an
  identity. A user can own multiple identities across multiple wallets, or import identities
  without a wallet.
- **Social profile** is *optional extended metadata* attached to an identity via a DashPay
  Profile document — display name, bio, avatar. Many identities will never have a social
  profile (masternodes, evonodes, DPNS-only users, developers).
- **Wallet** is the container for signing keys. A wallet can own zero, one, or many
  identities. An identity may also have no wallet on this device (imported by ID and private
  key).

Every place a previous draft said "Profile" when referring to the on-chain object, this
document says "Identity." Every place the previous draft said "DashPay profile" or "profile"
as optional extended metadata, this document says "social profile."

Nothing currently possible disappears. Feature parity is preserved.

---

## A. Information Architecture

### A.1 Left-nav entry

**Label: `Identities`** — unchanged from the current codebase label to minimize churn for
existing users. Always plural. No pluralization logic needed.

```
Wallets
Identities     ← collapses today's Identities + Dashpay
Contracts
Tokens
Tools
Settings
```

Nav icon: people-silhouette glyph (replaces today's card icon, subtly distinguishes from
Wallets). Network color stripe behavior is unchanged.

**Info tooltip on the nav entry** (info, All):
> Your identities on Dash Platform. Manage usernames, balances, keys, and — if you set up a
> social profile — DashPay contacts and payments.

### A.2 Tabs inside Identities

| # | Tab | Purpose | Maps to today |
|---|-----|---------|---------------|
| 1 | **Home** | Identity hero (DPNS handle, type badge, balance, primary actions). If a social profile exists, its avatar + display name + bio render at the top of the hero. If not, an inline "Set up your social profile" card is shown. Onboarding checklist, recent activity preview. | Identities row summary + Dashpay Profile view |
| 2 | **Contacts** | Requests strip · search + filters · contact list · right-side detail drawer · Add by username · Scan QR · Show my QR. Disabled / gated when the current identity has no social profile. | Dashpay Contacts + Profile Search + Add Contact + Contact Details |
| 3 | **Activity** | Unified timeline merging DashPay payments, funding (Add funds / Send to wallet / Send to another identity), and platform ops (DPNS, key changes). Filter chips: Payments · Funding · Platform. Expandable detail per row. | Dashpay Payment History + identity credit movements |
| 4 | **Settings** | Identity essentials: DPNS username + aliases, keys table, raw Identity ID, identity type, refresh / diagnostics, danger zone. Social profile subsection — create / edit display name, bio, avatar, delete social profile. | Dashpay Profile edit + identity Keys / Add Key + Alias + DPNS registration |

### A.3 Wallet + Identity switching — breadcrumb as switcher

The breadcrumb IS the wallet and identity switcher. It is always visible in the topbar of
every tab. Three segments, left to right:

```
Identities  ›  [💼 Main Wallet]  ›  [👤 @alex.dash ▾]
```

```
<nav aria-label="Location">
  <ol>
    <li><a>Identities</a></li>
    <li aria-hidden="true">›</li>
    <li>[wallet pill]</li>
    <li aria-hidden="true">›</li>
    <li aria-current="page">[identity pill]</li>
  </ol>
</nav>
```

**First segment — "Identities"**: plain text link. Navigates back to the Identity Picker
(§B.14) or to the section root. Not a pill.

**Second segment — wallet pill** (`.breadcrumb-pill`): icon + wallet alias. Style and
interactive behavior vary by persona:
- Alex (single wallet): `.subdued` modifier — no chevron, non-interactive, transparent
  background. Info tooltip `tt-3` unchanged.
- Priya / Jordan (multiple wallets): `.switcher-interactive` — hover border, chevron,
  `aria-haspopup="listbox"`. Dropdown lists every loaded wallet on the current network plus
  footer "Set up another wallet".

**Third segment — identity pill** (`.breadcrumb-pill.switcher-interactive`): avatar (or
type-glyph monogram) + DPNS handle (or short Identity ID) + chevron. Always interactive
where an identity is active. `aria-haspopup="listbox"`. Dropdown is scoped to the selected
wallet. A grouped section "Identities without a wallet on this device" lists identities
imported by raw ID. Footer "Add another identity" opens a chooser (Create new · Load
existing · Dev Mode only: Create multiple test identities).

**Placeholder rules** (when a segment has no value yet):

| Situation | Second segment | Third segment |
|---|---|---|
| No wallet, no identity (onboarding) | `(no wallet yet)` — italic, `text-secondary`, `aria-disabled="true"`, `role="presentation"` | `(no identity yet)` — same treatment |
| Wallet selected, no identity chosen (picker page) | Wallet pill (subdued or interactive per persona) | `(choose an identity)` — italic placeholder |
| All tabs when identity is active | Wallet pill | Identity pill with handle/name |

**Persona behavior**:

- Alex (one wallet, one identity): wallet pill is `.subdued` (non-interactive) with info
  tooltip. Identity pill is interactive.
- Priya (many wallets, many identities): both pills fully interactive with chevrons and
  dropdowns.
- Jordan (Dev Mode): identity dropdown footer offers "New throwaway wallet + identity" that
  chains wallet creation → funding → identity registration.

**Network awareness**: switching network filters both dropdowns to wallets and identities
that exist on the new network.

**Identity pill dropdown — ordering rule**: items are sorted `Local nickname → DPNS username
→ Identity ID (shortened)`. Inline search appears once the wallet contains 7 or more
identities. Drag-to-reorder is intentionally deferred to a later iteration (see §G).

**Jordan dev-mode dropdown footer**: the identity pill dropdown contains a `+ New throwaway
wallet + identity` footer entry in Developer Mode (catalog §D entry #6). This Jordan-only
path chains wallet creation → funding → identity registration in one step.

**CSS**: `.breadcrumb-pill` uses reduced vertical padding (`padding: 2px var(--sp-sm)`) so
the topbar stays single-line. The `.breadcrumb-ol` container is `display:flex; align-items:
center; gap:var(--sp-xs); flex-wrap:nowrap`. Focus ring and dropdown affordances are
unchanged from the previous standalone pill styling.

### A.4 Default landing for the Identities nav

When the user clicks **Identities** in the left nav the app decides what to show based on
how many identities are loaded for the active network:

| Loaded identities | Landing |
|---|---|
| 0 | Onboarding empty state (F1) |
| 1 | Identity Home for that identity directly (F3 — covers both social-profile-set and no-profile states) |
| ≥ 2 | Identity Picker grid (F2) |

**Navigation from the picker**: clicking a card selects that identity in the breadcrumb
switcher and navigates to Identity Home. The identity pill on Home becomes the route back
to the picker — clicking it opens the same identity list as a dropdown. Navigating to the
`Identities` breadcrumb link also returns to the picker. Both affordances use the same
behaviour so there is only one mental model.

---

## B. Screen-by-screen Design

All strings are complete sentences with named placeholders per the project i18n rule.
No concatenation.

### B.1 Onboarding empty state (Frame 1)

Shown when the user opens Identities on a network where they have no loaded identities.

**Layout**: island central panel, centered content, max-width 640 px. Abstract avatar
silhouette against a soft Dash-blue radial gradient. Heading + body + two primary actions
stacked vertically. Muted footer band in Developer Mode.

**Exact strings**:

- Heading: `Welcome to Identities.`
- Body paragraph 1: `An identity is your account on Dash Platform. With one you can pick a
  username, send and receive Dash by name, and — if you choose — connect with people through
  DashPay.`
- Body paragraph 2: `You only need a small amount of Dash from your wallet to get started.`
- Primary button: `Create my first identity`
- Secondary button (ghost): `I already have an identity — load it`
- Developer Mode footer: `Developer tools:` `[Create multiple test identities]` `·`
  `[Load identity by ID]`

**Validation / failure banners**:
- Insufficient wallet balance: `Your wallet does not have enough Dash to create an identity
  yet. Add at least {amount} to continue.` [Go to Receive]
- No wallet: `You need a wallet before you can create an identity.` [Set up a wallet]

### B.2 Identity Home (Frame 3)

The default tab landing once at least one identity exists. The canonical wireframe render
uses the Alex / social-profile-set state. See §B.3 for the no-social-profile variant, which
is annotated inside the same frame.

**Layout zones** (vertical stack inside the island panel):

1. Chrome strip — breadcrumb (with wallet + identity pills), tab bar.
2. Hero identity card (~240 px tall, full width, gradient surface).
3. Quick-actions row (Send / Receive / Add contact).
3a. Secondary actions row (Add funds / Send to wallet / Send to another identity) — all three
   visible for all personas. See §PROJ-008 for entry-point rationale.
4. Onboarding checklist strip (conditional — until all three steps complete). The three
   steps, in order:
   1. `Pick a username`
   2. `Set a display name` — hidden if the user has previously dismissed the social profile
      card (treated as a deliberate skip; do not re-prompt).
   3. `Add your first contact`
5. Recent activity preview (latest 5 rows + See all link).

**Hero identity card content** (social profile set):

Left cluster: 96 px avatar circle (social profile image or initials fallback) + display name
(heading_large) + `@{handle}` below (body, text_secondary).

Right cluster: `{amount} DASH` (heading_medium) + fiat equivalent (body, text_secondary) +
Identity-type badge pill + Network pill.

If no DPNS name: `No username yet` (italic, text_secondary) with link `Pick a username`.

**Quick-actions row**:

| Button | Label | Tooltip |
|---|---|---|
| Primary | `Send` | `Send Dash to a contact, username, or address.` |
| Primary | `Receive` | `Show a QR code or your username so someone can pay you.` |
| Secondary | `Add contact` | `Find someone by username and add them to your contacts.` |

**Recent activity preview** — up to 5 rows. Example strings:
- `Received {amount} DASH from {counterparty_name}`
- `Sent {amount} DASH to {counterparty_name}`
- `Added {amount} DASH to your identity`
- `Sent {amount} DASH to your wallet`
- `Registered the username {handle}`

Footer link: `See all activity` → activates Activity tab.

Empty state: `No activity yet. When you send or receive Dash, it will show up here.`

**Progressive disclosure on Home** (Priya / Jordan only):

Advanced expander below activity preview (collapsed for Alex, open for Priya / Jordan):
- Label: `Advanced details`
- Contents: raw Identity ID (copyable, monospace, RADIUS_SM), revision number, last
  updated, keys summary.

**Secondary actions row** (below quick-actions row, all personas):

| Button | Label | Tooltip |
|---|---|---|
| Ghost | `Add funds` | `Move Dash from your wallet into this identity.` |
| Ghost | `Send to wallet` | `Convert your identity balance back to spendable Dash in your wallet.` |
| Ghost | `Send to another identity` | `Transfer Dash directly from this identity to another identity.` |

These three buttons enter the Add funds wizard (§B.9), the Send to wallet flow, and the
Send sheet (§B.7) with pre-configured recipient mode respectively. All are visible for all
personas; no `.adv` gating.

### B.3 Identity Home — no social profile state

The same frame as §B.2 (Frame 3) covers this state via an annotation callout. No separate
frame exists for the no-profile state.

**Hero identity card** (no social profile): type-glyph monogram in place of avatar (person /
masternode / evonode glyph in a Dash-Blue ring). DPNS handle + identity-type badge +
balance. No display name shown.

**Inline social profile card** (rendered below the quick-actions row, above onboarding
checklist):

- Heading: `Set up your social profile`
- Body: `Add a display name, bio, and avatar so people can find you on DashPay. This is
  optional — you can still use every other feature without it.`
- Primary button: `Add a display name`
- Ghost link: `Skip — I use this identity only for {reason}`

**Tooltip on the social profile card** (info, All):
> Add a display name, bio, and avatar so people can find you on DashPay. This is optional
> — you can still use every other feature without it.

**Wireframe annotation** (inside Frame 3):
> When the selected identity has no social profile, the avatar shows the type-glyph monogram
> and the hero body renders an inline "Set up your social profile" card. See design-spec §B.3.

### B.4 Contacts (Frame 4)

Populated contacts page shown when the identity has a social profile. Three sections rendered
in priority order.

**Layout zones**:
1. Tab header: `Contacts` title + right-aligned action buttons: `+ Add by username`,
   `Scan QR`, `Show my QR` (catalog tooltips #26–28).
2. Section 1: Received requests — awaiting your approval (amber left-border, amber `2 new`
   badge, rendered first so they surface immediately).
3. Section 2: Active contacts (the bulk of the page — heading `Active contacts · {n}` +
   search input right-aligned).
4. Section 3: Sent requests — waiting for acceptance (muted, blue left-border, rendered at
   bottom).

**Received requests section** — horizontal row of request cards (`.request-card
.request-card--received`, amber `3px` left-border). Each card:
- 40 px avatar, display name, `@handle`, relative timestamp.
- Accept button (catalog tt-29) and Decline button (catalog tt-30). Both have `aria-describedby`.

**Active contacts section** — list rows (`.list-row`). Each row:
- 40 px avatar, display name (body_large), `@{handle}` + last-payment hint (body_small,
  text_secondary).
- Compact `Send` primary-small button (catalog tt-32) and `•••` overflow icon-button (catalog tt-33).
- Row is clickable and opens the contact detail drawer (right-hand slide-in, 480 px):
  avatar, display name, `@{handle}`, four action buttons (Send Dash · Copy handle · Edit
  private label · Remove contact). Collapsible sections: About · Private notes · Payment
  history · Advanced.

**Sent requests section** — request cards (`.request-card .request-card--sent`, blue left-
border, `opacity: 0.85`). Each card:
- Avatar, handle, display name.
- `Pending` pill (catalog tt-31) on the pill.
- `Cancel request` ghost button (catalog tt-29c): *Cancel the request. {counterparty_name}
  will not be notified.*

**Search input** (inside the active contacts section header, right-aligned):
- `type="search"`, `placeholder="Search your contacts"`, `aria-label="Search your contacts"`.

**Empty states**:
- No received requests: the section collapses to a single muted line `No pending requests.`
- No active contacts: section shows `You have no contacts yet.` with the primary `Add by
  username` CTA.
- No sent requests: section is hidden entirely (no empty state).

**Add by username** (Contacts tab header button): the input field accepts `@username` or a
raw Base58 Identity ID — both resolve to the same lookup path. Tooltip copy: "Find someone
by their Dash username or identity ID and add them as a contact."

### B.4.1 No-social-profile state

When the current identity has no social profile, the Contacts tab does not show the three
sections above. Instead, the main content area renders a centered gate card:

- Heading: `Set up a social profile first.`
- Body: `Contacts use your display name and avatar to let people find you. Your username
  @{handle} already works for payments — a social profile only unlocks contacts. Without a
  social profile, you cannot add contacts or receive contact requests.`
- Primary button: `Add a display name`
- Secondary button: `Why?` — expands an inline explanation panel.

The setup card lives on Identity Home (§B.3). Once the social profile is set up, the
Contacts tab transitions to the populated state (§B.4).

**Tooltip on the Contacts tab when gated** (info, All):
> Set up a social profile first. Contacts need a display name and avatar so people can find
> you.

### B.6 Activity tab (Frame 5)

Unified timeline.

**Filter chips** (multi-select): All (default) · Payments · Funding · Platform (collapsed
under More for Alex; fully visible for Priya / Jordan).

**Timeline row (collapsed)**: 48 px. Left: colored icon badge. Center: action sentence
(body_large) + counterparty + method (body_small). Right: timestamp + expand chevron.

**Timeline row (expanded)**: detail panel slides down. Two-column for Priya, single-column
for Alex. Contents: Summary · Counterparty · Details (memo, fee, status: Confirmed /
Pending / Failed) · Advanced (Priya: raw TxID, state-transition hash, block height) ·
Dev mode JSON dump.

**Failed activity row**: red left-border accent.
- Row text: `Could not send {amount} DASH to {counterparty_name}`
- Right: `Retry` small button
- Expanded banner: `The network did not accept this payment. Your balance is unchanged.
  Check your connection and try again, or try a smaller amount.`

Empty state: `No activity yet. Your payments, additions, and identity changes will appear
here.`

### B.7 Send sheet (Frame 6)

Modal sheet, RADIUS_LG, elevated shadow, modal_overlay backdrop. Width 560 px desktop.
Escape = cancel.

**Step 1 — Compose**:
- Heading: `Send Dash`
- Sub-heading (body_small, text_secondary): `Send from your identity {display_name}.`
- Recipient field label: `To`
- Placeholder: `Username, contact, Dash address, or identity ID`
- Validation strings:
  - `Looking up @{handle}…` (info color)
  - `{handle} not found.` (error color)
  - `This address is not valid.` (error color)
  - `This is your own identity. Pick someone else.` (warning color)
- Amount field label: `Amount`
- `Use {fiat_code}` toggle (link style): `Enter the amount in {fiat_code}. We convert it to
  DASH for you.`
- Quick amount pills: `{amount_a}` · `{amount_b}` · Max
- Memo expander label: `Add a note`
- Memo placeholder: `Private to you and the recipient.`

**Fee and total preview card** (calm grey, RADIUS_MD):
- `You send` `{amount} DASH`
- `Network fee` `{fee_amount} DASH`
- `Total from your identity` `{total_amount} DASH`
- Priya / Jordan only: `Credits used` `{credit_amount} credits`

Buttons: `Cancel` (ghost) · `Review` (primary). Disabled tooltip: `Enter a valid recipient
and amount to continue.`

**Step 2 — Review**: recipient card + summary rows + memo preview + `Send {amount} DASH`
primary + `Back` ghost.

**Step 3 — Sent**: `Payment sent`. Body: `{counterparty_name} will see this in their
activity. Your identity balance is now {new_balance} DASH.` Post-success suggestion card if
the recipient is not yet a contact: `Would you like to add {counterparty_name} as a contact
so future payments are one click away?` [Add as contact] [Not now]

**Step 3 — Failed**: Heading `Payment could not be sent`. Body: `Your balance is unchanged.
Check your connection and try again, or try a smaller amount.` Actions: Back · Try again.

### B.8 Settings tab — Priya, Advanced expanded (Frame 7)

**Layout**: two-column on >= 1024 px; single column otherwise. Left: social profile section.
Right: username and aliases section. Full-width Advanced expander below.

**Left column — Social profile section** (renamed from "Public profile" in earlier draft):

- Section heading: `Social profile`
- Helper: `This information is visible to everyone on Dash Platform.`
- Avatar editor: 128 px circle + `Change photo` ghost button.
- Display name input, label: `Display name`, placeholder: `How should people see your name?`
- Bio textarea (4 rows), label: `About`, placeholder: `A short description, up to {max}
  characters.`
- Save button: `Save social profile`
  - Disabled (no changes): `There are no changes to save.`
  - Disabled (invalid): `Fix the highlighted fields before saving.`
- Danger link (danger color): `Delete social profile`
  - Confirmation dialog: `Remove the display name, bio, and avatar from DashPay. Your
    identity, usernames, and balance stay intact. Are you sure?`

**Right column — Username and aliases**:
- Section heading: `Username`
- Current username row: monospace `@{handle}` + Copy button + `Primary` pill.
- No primary username: CTA card `Pick a username` with `Register a username` primary button.
- Section heading: `Aliases`
- Helper: `Extra usernames that also point to your identity.`
- List rows: `@{alias}` + `Make primary` + `Remove`.
- `Add an alias` ghost button.

**Advanced expander** (collapsed by default for Alex; expanded for Priya / Jordan):
- Label: `Advanced`
- Sub-label: `Keys, raw identifiers, and identity type.`

Contents:

1. **Identity type and raw ID**:
   - Identity-type badge (User identity / Masternode identity / Evonode identity).
   - Raw Identity ID (monospace, RADIUS_SM surface, Copy icon-button).
   - Masternode / Evonode only: `Masternode ID` (ProTxHash, monospace, Copy button).

2. **Keys**:
   - Sub-heading: `Keys`
   - Helper: `Keys let this identity sign actions. Most people never need to manage these
     directly.`
   - Table: columns Purpose · Type · Status · Added · Actions.
   - Per-row actions: View details · Disable (where applicable).
   - Below table: `Add a new key` primary-small button.

3. **Refresh and diagnostics**:
   - `Refresh identity data` ghost button.
   - Priya only: `Refresh mode` selector (Core only / Platform only / Both).

4. **Danger zone** (red-bordered card at bottom of Advanced expander):
   - Sub-heading: `Danger zone`
   - Action: `Unload this identity from this device`
     - Confirmation dialog: `This removes the identity from this device. It remains on Dash
       Platform — you can load it again later.`

5. **Voter identity keys** (Masternode / Evonode only — rendered inside Advanced, after the
   main Keys table):
   - Sub-heading: `Voting keys` (Alex-facing label for `PrivateKeyOnVoterIdentity`).
   - Helper: `These keys belong to the separate voter identity tied to your masternode. Most
     operators manage them via the CLI, not this screen.`
   - Alex-facing label tooltip: "The keys your masternode uses to vote on username contests."
   - Table: same columns as main Keys table (Purpose · Type · Status · Added · Actions).
   - This section is only rendered for `Masternode identity` and `Evonode identity` types.
     It is `.adv`-gated.

6. **Local nickname** (under the Display name section, below the `Aliases` heading):
   - Field label: `Local nickname`
   - Placeholder: `A label only you see on this device.`
   - Helper: `This nickname is never published to Dash Platform. It is useful if you manage
     several identities and want a shorthand beyond your DPNS username.`
   - Displayed in the identity pill dropdown: priority order is Local nickname → DPNS
     username → shortened Identity ID (see §A.3).
   - Wording audit entry: `alias (local QualifiedIdentity.alias)` → `Local nickname` (distinct
     from DPNS Alias, which refers to on-chain secondary usernames).

7. **Auto-accept contact requests** (under Social profile section):
   - Toggle: `Auto-accept contact requests`
   - Helper: `Generate a proof that automatically accepts inbound contact requests without
     your approval. Useful for public-facing accounts.`
   - Account-index selector (Priya / Jordan only): `Account index`
   - Validity-period selector: `Valid for` (options: 1 week · 1 month · 3 months · 1 year)
   - Catalog §D addition (All personas, info tooltip):
     > Automatically accept contact requests for this identity using an HD-derived proof.
     > The proof works for the selected validity period, then expires.

### B.9 Add funds wizard

Entry points: secondary actions row on Home (Add funds button), Advanced expander on Home.

**Funding method chooser** (step 1) — four methods, persona-gated:

| Method | Alex-facing label | Visibility |
|--------|-------------------|------------|
| `UseWalletBalance` | `From your wallet` (recommended) | Alex, Priya, Jordan |
| `AddressWithQRCode` | `Send to an address` | Alex, Priya, Jordan |
| `UsePlatformAddress` | `Use a Platform address` | Priya, Jordan |
| `UseUnusedAssetLock` | `Recover an unfinished funding` | Priya, Jordan |

**Recover an unfinished funding** (`UseUnusedAssetLock`) is the orphan-recovery flow for
users whose identity creation failed mid-stream. Alex-facing label deliberately avoids "Asset
lock" jargon. Priya / Jordan see the technical name in a secondary gloss.

**From your wallet** (`UseWalletBalance`) is the primary path and should be pre-selected.
After method selection, step 2 shows an amount input with the wallet balance shown inline,
fee preview, and a `Confirm` primary button.

### B.10 Create identity wizard

Triggered by `Create my first identity` on the onboarding screen, or `+ Add another identity
→ Create new` from the identity pill dropdown.

1. **Fund the identity** — runs the Add funds wizard (§B.9) inline. Recommended method:
   `UseWalletBalance`. Minimum required shown dynamically.
2. **Pick a username** — optional at creation time but encouraged. Contested name detection
   runs here (see §B.13). User may skip; username can be registered later from Settings.
3. **Done** — brief success state; identity is now active on Home.

### B.11 Load existing identity

Triggered by `I already have an identity — load it` on the onboarding screen, or `+ Add
another identity → Load existing` from the identity pill dropdown.

**Mode chooser** (three modes):

| Mode | Alex-facing label | Visibility |
|------|-------------------|------------|
| By Identity ID + private key | `Enter the identity ID and private key` | Alex, Priya, Jordan |
| By DPNS username | `Enter my username` | Alex, Priya, Jordan |
| By wallet derivation | `Derive from my wallet` | Priya, Jordan (Advanced) |

- **By Identity ID + private key**: two inputs — Identity ID (Base58) and private key. After
  import, the identity is visible immediately on Home.
- **By DPNS username**: resolves the username to an Identity ID via the network, then
  prompts for the private key for the resolved identity.
- **By wallet derivation**: scans the wallet's derivation path for registered identities.
  Priya / Jordan only, hidden behind an Advanced expander for Alex.

### B.12 — (reserved)

### B.13 Pick a username

Accessible from: Home hero `Pick a username` link (when no DPNS name), Settings right column
`Register a username` button, and step 2 of the Create identity wizard (§B.10).

**Step 1 — Enter a username**:
- Input field with live availability check (debounced, 300 ms).
- Success state: `@{handle} is available.` (success color).
- Unavailable state: `@{handle} is taken.` with a `Browse alternatives` ghost link.
- Contested state: `@{handle} is contested. Registering it starts a masternode vote.`
  - Contested fee preview card: shows the higher fee (e.g. `0.2 DASH contest fee`) alongside
    the standard registration fee.
  - Explanation banner: `Contested names are put to a vote by masternodes. If enough
    masternodes vote against your registration, the name goes to the next applicant. The
    voting period lasts approximately 2 weeks.`
  - Alex sees the plain-language banner; Priya / Jordan also see the lock period in blocks.

**Step 2 — Fee preview and confirm**:
- Standard: `You pay {fee_amount} DASH to register @{handle}.`
- Contested: `You pay {fee_amount} DASH (including the {contest_amount} DASH contest
  deposit). If the vote succeeds, the deposit is burned.`
- Primary button: `Register @{handle}`

**Step 3 — Registered / Failed**:
- Success: `@{handle} is yours. It is now your primary username.`
- Failed (contested, vote lost): `The vote on @{handle} did not go your way. Your contest
  fee was returned. You can try a different username or wait and try again.`

### B.14 Identity picker (Frame 2)

Shown when the user clicks Identities in the left nav and two or more identities are loaded
on the current network (see §A.4 for the routing rules).

**Layout**: island central panel. No switcher row — the picker itself is the selector.

- Page heading: `Pick an identity` (heading_large / text-xxl)
- Sub-heading (body_small, text_secondary): `Each identity has its own balance, keys, and
  optional social profile. Choose one to open it, or add a new identity.`
- CSS grid: `grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: var(--sp-md);`

**Identity card** (`.card .identity-card`, `RADIUS_LG`):

- Whole card is a `role="button"` / `tabindex="0"` target. Focus ring on `:focus-visible`.
  Hover elevates shadow from `--shadow-small` to `--shadow-medium`.
- 72×72 circular avatar at the top of the card. If a social profile exists: Dash-blue
  fill with the first letter of the display name. If no social profile: monogram glyph
  (User = person silhouette, Masternode = diamond, Evonode = diamond) in
  `--bg-dark` / `--text-secondary`.
- Identity-type badge pill (`badge--user` / `badge--masternode` / `badge--evonode`) anchored
  to the top-right corner of the card.
- Display name (heading_small / fw-600). When no display name: DPNS handle. When neither:
  shortened Identity ID in monospace (e.g. `Fx1Kj…9Tt`).
- Sub-line (body_small, text_secondary): DPNS handle if not already the heading, otherwise
  the identity-type label string (`User identity` / `Masternode identity` / `Evonode identity`).
- Balance (body_small, fw-600, tabular numerals): `{amount} DASH`. Fiat equivalent
  (text_xs, text_secondary) on the line below when available.
- Static wireframe callout (not a real implementation artifact): `Opens Identity Home →`
  rendered in a small info-colored chip at the card bottom.
- `aria-label="Open {display_name_or_handle}"`.
- Tooltip (tt-78x): `Open this identity. You can switch between identities anytime from
  the pill under the breadcrumb.`

**"Add a new identity" card** (`.identity-card.identity-card--add`):

- Same grid cell, same dimensions. Visual treatment: dashed border (`2px dashed var(--border)`)
  with hover switching to solid Dash Blue.
- 72×72 circle with `+` glyph (Dash Blue on transparent background).
- Heading: `Add a new identity`. Sub-line: `Create a new identity or load one you already own.`
- `role="button"`, `tabindex="0"`, `aria-label="Add a new identity"`.
- Tooltip (tt-78y): `Create a new identity or load one you already own.`

**Card set rendered in the wireframe** (Priya / multi-identity context):

| # | Card | Type | DPNS | Balance |
|---|---|---|---|---|
| 1 | Alex Torres (social profile) | User | @alex.dash | 0.75 DASH ≈ 45.25 USD |
| 2 | Priya Nakamura (social profile) | User | @priya.dash | 12.5 DASH |
| 3 | No social profile — monogram | Masternode | mn-east-01.dash | 0.10 DASH |
| 4 | Testing — no DPNS (`.dev` gated) | User | *(none)* — shows `Fx1Kj…9Tt` | 0.01 DASH |
| 5 | Add a new identity | — | — | — |

Card 4 carries the `.dev` class and is hidden for Alex and Priya — Jordan only.

**Card states**:

- Default: `--shadow-small` border `1px solid var(--border)`.
- Hover: `--shadow-medium` (no border color change on standard cards).
- Focus-visible: 3 px Dash-blue outline with 2 px offset.
- Currently-selected identity (future real implementation): `border-color: var(--dash-blue)`
  with a small `Selected` indicator — not shown in the static wireframe.

**Empty state**: not applicable — the picker is only shown when ≥ 2 identities exist. When
0 or 1 identity exist the routing rule in §A.4 applies instead.

---

## C. Wording Audit

Authoritative replacement table. All strings are complete sentences with named placeholders.
No concatenation. Column 1 = current codebase / UI string. Column 2 = Alex-facing
replacement. Column 3 = Power / Dev tooltip gloss (shown on hover for Priya / Jordan).

| Current | Alex-facing | Power/Dev tooltip |
|---|---|---|
| Top up | Add funds | Move Dash from your wallet into this identity as Platform credits. |
| Withdraw | Send to wallet | Convert Platform credits back to spendable Dash on the Core chain. Takes one or more blocks to settle. |
| Transfer | Send to another identity | Transfer Platform credits from this identity directly to another identity without leaving Platform. |
| Identity / Identities | Identity / Identities (kept) | A Dash Platform identity — the on-chain object that owns usernames, keys, and documents. |
| DashPay profile | Social profile | Optional display name, bio, and avatar linked to this identity for DashPay. |
| UserId / Identity ID | Identity ID | Base58-encoded identity ID on Dash Platform. |
| ProTxHash | Masternode ID | The ProTxHash that binds this identity to a masternode on Dash Core. |
| User / Masternode / Evonode (type) | User identity / Masternode identity / Evonode identity | Identity type: basic user, masternode-bound, or evonode-bound. |
| Credits | Balance (in identity context) | The raw Platform credit balance for this identity. |
| Credits spent | Network fee | Platform credits consumed by this action. |
| Asset lock | (hidden from Alex) | An unspent output on Core locked as funding for Platform operations. Created automatically when you add funds. |
| Dashpay (nav label) | (removed — subsumed into Identities) | — |
| DashPay (described in copy) | your social payments network on Dash Platform | Marketing label for the Platform-based social payments protocol. |
| Contact request | Add as contact / Connect | A DashPay contact request state transition that, once mutual, creates a shared payment channel. |
| Incoming contact requests | Requests from others | Pending inbound contact requests awaiting your response. |
| Outgoing contact requests | Requests you sent | Pending outbound contact requests. |
| Nickname (contact) | Private label | A local-only label for this contact. Never shared. |
| Note (contact) | Private note | Local-only note visible only on this device. |
| Hide contact | Hide from list | Exclude this contact from the default list view. They are not removed or notified. |
| Register DPNS name | Pick a username | Register a DPNS name against this identity. May be contested. |
| Update alias | Add or change usernames | Add a secondary DPNS alias or change which alias is primary. |
| Load identity | Load an existing identity | Import an existing identity by ID and owner private key. |
| Create identity | Create a new identity | Register a new identity on Dash Platform. |
| Create identities in bulk | Create multiple test identities | Bulk-register N identities for testing. Only available in Developer Mode. |
| Refresh identity | Refresh identity data | Re-query identity state from the network. |
| Add key | Add a new key | Add a key to the identity via an UpdateIdentity state transition. |
| Key purpose | Purpose | Authentication, Encryption, Decryption, Transfer, Voting, or System. |
| Key security level | Security level | High, Medium, Critical, or Master security level. |
| Scan QR | Scan QR | Scan a DashPay contact or payment QR code. |
| Generate QR | Show my QR | Generate a QR code for this identity. |
| Auto-accept contact request | Auto-accept contact requests | Generate an HD-derived proof that automatically accepts inbound contact requests without manual approval. |
| Platform credits | Identity balance | Platform credits owned by this identity. |
| Duffs | Satoshis (Dash) | 1/100,000,000 of a DASH; the Core-chain smallest unit. |
| State transition | Platform action | A signed payload submitted to Dash Platform to mutate state. |
| State transition result | Action result | The success/error envelope returned by Platform for the submitted state transition. |
| Broadcast | Send | Broadcast the transaction to the network. |
| Nonce | Sequence number | Monotonically increasing per-identity sequence number used to order state transitions. |
| Revision | Version | Identity revision counter, incremented on each change. |
| Proof | (hidden from Alex) | GroveSTARK inclusion proof for the queried Platform data. |
| Mnemonic | Recovery phrase | BIP39 mnemonic phrase; 12/24 words used to derive the HD wallet. |
| BIP44 account | Main account | BIP44 external chain for this wallet. |

**DashPay brand note**: "DashPay" is retained as a descriptor — in onboarding copy, in
contact-related empty states, in Developer Mode tooltips. It is removed only as a top-level
navigation entry.

---

## D. Tooltip Catalog

All tooltips: complete sentences, named placeholders, no concatenation. Variant maps to
`ResponseExt` methods: `info` = `info_tooltip`, `clickable` = `clickable_tooltip`,
`disabled` = `disabled_tooltip`.

| # | Element | Tooltip text | Variant | Persona |
|---|---|---|---|---|
| 1 | Left-nav entry `Identities` | Your identities on Dash Platform. Manage usernames, balances, keys, and — if you set up a social profile — DashPay contacts and payments. | info | All |
| 2 | Wallet pill (interactive, Priya / Jordan) | Switch between your wallets. Each wallet can own several identities. | clickable | Priya, Jordan |
| 3 | Wallet pill (single-wallet label, Alex) | This identity is funded by {wallet_name}. Set up another wallet on the Wallets screen to unlock switching. | info | All |
| 4 | Identity pill | Switch between identities in {wallet_name} or add a new one. | clickable | All |
| 5 | Identity pill dropdown group `Identities without a wallet on this device` | These identities were imported by ID and are not tied to any wallet on this device. | info | Priya, Jordan |
| 6 | `+ New throwaway wallet + identity` (Dev Mode) | Create a temporary wallet and a new identity in one step. Handy for testing. | clickable | Jordan |
| 7 | `Create my first identity` button | Start the short setup: pick a username, fund the identity from your wallet, and confirm. | clickable | All |
| 8 | `I already have an identity — load it` | Enter the identity ID and private key to import an existing identity into this device. | clickable | All |
| 9 | `Create multiple test identities` | Create a batch of identities for testing. Each one is funded and registered automatically. | clickable | Jordan |
| 10 | Hero balance (DASH amount) | This is the Dash held by your identity. Your wallet balance is shown on the Wallet screen. | info | All |
| 11 | Hero fiat equivalent | An estimated value in {fiat_code}. Rates can change. | info | Alex, Priya |
| 12 | Identity-type badge `User identity` | A regular identity used for payments, DPNS, and DashPay. | info | All |
| 13 | Identity-type badge `Masternode identity` | An identity tied to a Dash masternode. It can vote on name contests. | info | All |
| 14 | Identity-type badge `Evonode identity` | An identity tied to a Dash evonode. It can vote and validate Platform transactions. | info | All |
| 15 | Network pill | You are on {network_name}. Identities and balances are separate per network. | info | All |
| 16 | Quick action `Send` | Send Dash to a contact, username, or address. | clickable | All |
| 17 | Quick action `Receive` | Show a QR code or your username so someone can pay you. | clickable | All |
| 18 | Quick action `Add contact` | Find someone by username and add them to your contacts. | clickable | All |
| 19 | `Set up your social profile` card | Add a display name, bio, and avatar so people can find you on DashPay. This is optional — you can still use every other feature without it. | info | All |
| 20 | Onboarding checklist dismiss | Hide the setup checklist. You can find these actions on Settings and Contacts anytime. | clickable | All |
| 21 | Insight chip `@handle works like an address` | Share your username with anyone who wants to pay you. It works even if the sender is on a different Dash wallet. | info | Alex |
| 22 | Advanced expander on Home | Show technical details like raw IDs, keys, and revision numbers. | clickable | All |
| 23 | Raw Identity ID copy button | Copy the full identity ID to your clipboard. | clickable | Priya, Jordan |
| 24 | ProTxHash / Masternode ID copy button | Copy the masternode ID to your clipboard. | clickable | Priya, Jordan |
| 25 | Contacts tab (gated, no social profile) | Set up a social profile first. Contacts need a display name and avatar so people can find you. | info | All |
| 26 | Contacts tab header `Add by username` | Find someone by their Dash username or identity ID and add them as a contact. | clickable | All |
| 27 | Contacts tab header `Scan QR` | Use a camera or paste a QR image to add a contact. | clickable | All |
| 28 | Contacts tab header `Show my QR` | Show a QR code so someone nearby can add you or pay you. | clickable | All |
| 29 | Incoming request `Accept` | Accept this contact request. You will appear in each other's contact list. | clickable | All |
| 30 | Incoming request `Decline` | Decline this request. The other person will not be notified. | clickable | All |
| 31 | Outgoing request `Pending` pill | Waiting for {counterparty_name} to respond. | info | All |
| tt-29c | Sent request `Cancel request` button | Cancel the request. {counterparty_name} will not be notified. | clickable | All |
| 32 | Contact list row `Send` button | Send Dash to {counterparty_name}. | clickable | All |
| 33 | Contact list row `•••` overflow | More actions for this contact. | clickable | All |
| 34 | Contact overflow `Edit private label` | Change the local-only label for this contact. Only you see it. | clickable | All |
| 35 | Contact overflow `Hide from list` | Exclude this contact from your default list. They are not notified. | clickable | All |
| 36 | Contact overflow `Remove contact` (disabled) | Removing contacts is not yet available. It will arrive in a future update. | disabled | All |
| 37 | Contact detail `Copy handle` | Copy @{handle} to your clipboard. | clickable | All |
| 38 | Contact detail `Private notes` hint | Only you can see this. It is never shared with the contact. | info | All |
| 39 | Activity filter chip `Payments` | Shows money you sent or received. | info | All |
| 40 | Activity filter chip `Funding` | Shows when you added Dash to your identity, sent Dash back to your wallet, or moved Dash between identities. | info | All |
| 41 | Activity filter chip `Platform` | Shows identity changes like usernames, keys, and contracts. | info | Priya, Jordan |
| 42 | Activity row expand chevron | Show details for this activity. | clickable | All |
| 43 | Activity export button | Save your activity as a CSV file. | clickable | Priya, Jordan |
| 44 | Activity failed row `Retry` button | Try sending this payment again. Your balance has not been touched. | clickable | All |
| 45 | Activity `Request {amount} from {counterparty_name}` (disabled) | Payment requests are coming soon. | disabled | All |
| 46 | Settings: social profile `Change photo` | Upload a square image. Other apps will see this avatar. | clickable | All |
| 47 | Settings: `Save social profile` (disabled, no changes) | There are no changes to save. | disabled | All |
| 48 | Settings: `Save social profile` (disabled, invalid) | Fix the highlighted fields before saving. | disabled | All |
| 49 | Settings: `Delete social profile` | Remove the display name, bio, and avatar from DashPay. Your identity, usernames, and balance stay. | clickable | All |
| 50 | Username `Primary` pill | Your primary username is what people see by default. | info | All |
| 51 | `Make primary` alias action | Use this username as your main one. Your old primary will become an alias. | clickable | All |
| 52 | `Remove` alias action | Remove this alias. You will keep your other usernames. | clickable | All |
| 53 | `Add an alias` button | Register another DPNS name that points to this identity. | clickable | All |
| 54 | Keys table column `Purpose` | What this key is allowed to do (authenticate, transfer, decrypt, vote). | info | Priya, Jordan |
| 55 | Keys table column `Type` | The cryptographic algorithm for this key. | info | Priya, Jordan |
| 56 | Keys table column `Status` | Whether this key is active, disabled, or revoked. | info | Priya, Jordan |
| 57 | Keys `Add a new key` | Register a new key for this identity. You will choose its purpose and type. | clickable | Priya, Jordan |
| 58 | `Refresh identity data` button | Fetch the latest state of this identity from the network. | clickable | All |
| 59 | Danger zone `Unload this identity from this device` | Remove this identity from this device. It remains on Dash Platform — you can load it again later. | clickable | All |
| 60 | ProTxHash row info | The masternode identifier on the Dash Core chain. | info | Priya, Jordan |
| 61 | Send sheet `To` label | Paste a username, Dash address, or identity ID. You can also pick from your contacts. | info | All |
| 62 | Send sheet `Amount` label | How much Dash to send. We will show the network fee before you confirm. | info | All |
| 63 | Send sheet `Use {fiat_code}` toggle | Enter the amount in {fiat_code}. We convert it to DASH for you. | clickable | All |
| 64 | Send sheet `Max` quick-amount | Send your entire identity balance minus the network fee. | clickable | All |
| 65 | Send sheet memo `Add a note` expander | Attach a private note that only you and the recipient can see. | clickable | All |
| 66 | Send sheet fee row `Network fee` | Paid to the network to process this payment. Not paid to anyone you know. | info | All |
| 67 | Send sheet `Credits used` row (Priya / Jordan) | The Platform credits this action will spend. | info | Priya, Jordan |
| 68 | Send sheet `Review` (enabled) | Double-check before sending. | clickable | All |
| 69 | Send sheet `Review` (disabled) | Enter a valid recipient and amount to continue. | disabled | All |
| 70 | Send success `Send again` | Open the send sheet again to {counterparty_name}. | clickable | All |
| 71 | Send success `Add as contact` | Save {counterparty_name} to your contacts so future payments are one click away. | clickable | All |
| 72 | Receive `Copy username` | Copy @{handle} to your clipboard. | clickable | All |
| 73 | Receive `Share via…` | Share your username through another app. | clickable | All |
| 74 | Receive QR code | This QR code contains a Dash address for a one-time payment to this identity. | info | All |
| 75 | Receive `Copy address` | Copy the address to your clipboard. | clickable | All |
| 76 | Receive `Show full address list` | Open the address table for this wallet. Useful for advanced setups. | clickable | Priya |
| 77 | Connection indicator (connected) | You are connected and up to date. | info | All |
| 78 | Connection indicator (syncing) | You are catching up with the network. Balances may update shortly. | info | All |
| 79 | Connection indicator (offline) | You are offline. Payments cannot be sent until you reconnect. | info | All |
| 80 | Developer Mode chip (when on) | Developer Mode shows advanced fields and testnet tools. Turn it off in Settings. | info | Jordan |
| 81 | Settings: `Refresh mode` selector | Choose whether to refresh only Core chain data, only Platform data, or both at once. | info | Priya |
| 82 | Identity pill dropdown `Add another identity` footer | Create a new identity or load one you already own. | clickable | All |
| 83 | Home secondary action `Add funds` | Move Dash from your wallet into this identity. | clickable | All |
| 84 | Home secondary action `Send to wallet` | Convert your identity balance back to spendable Dash in your wallet. | clickable | All |
| 85 | Home secondary action `Send to another identity` | Transfer Dash directly from this identity to another identity. | clickable | All |
| 86 | Topbar `Refresh identity data` icon-button | Fetch the latest identity data from the network. | clickable | All |
| tt-78x | Identity picker card (each identity card) | Open this identity. You can switch between identities anytime from the breadcrumb. | clickable | All |
| tt-78y | Identity picker "Add a new identity" card | Create a new identity or load one you already own. | clickable | All |

---

## E. Visual Direction

Reuses all existing tokens from `src/ui/theme.rs`. No new color constants invented.

**Shadow alpha intentional deviation**: the wireframe uses CSS shadow alphas
`0.08 / 0.12 / 0.15 / 0.18 / 0.30` (for `--shadow-small` through `--shadow-glow`). These
are the **visual target**. The current `theme.rs` `Shadow::*` constants store egui alpha
bytes `8 / 12 / 15 / 18 / 30` (out of 255), which decode to `0.031 / 0.047 / 0.059 /
0.071 / 0.118` — noticeably fainter than the wireframe intent. The implementation PR should
update `Shadow::*` alpha bytes in `theme.rs` to match the wireframe values
(8→20, 12→31, 15→38, 18→46, 30→76 in 255-scale). This is a deliberate mini-deviation
flagged here so reviewers know the wireframe is not wrong.

- **Island central panel**: `RADIUS_LG` (16 px) + `Shadow::elevated()`.
- **Identity Home hero**: gradient `DashColors::DASH_BLUE` (#008de4) → `DashColors::PLATFORM_PURPLE` (#8250dc) at 14 % opacity over `DashColors::surface(dark_mode)`. Radius `RADIUS_XL` (20 px).
- **Other cards**: `DashColors::surface(dark_mode)` + `Shadow::medium()`, `RADIUS_MD` (12 px).
- **Pill badges**: `RADIUS_FULL` (255 px). Identity-type pill colors:
  - User identity = `DashColors::DASH_BLUE` (#008de4) fill at 12 % opacity, 1 px stroke.
  - Masternode identity = `DashColors::PLATFORM_PURPLE` (#8250dc) fill at 12 % opacity, 1 px stroke.
  - Evonode identity = `DashColors::HIGHLIGHT_GOLD` (#9b870c) fill at 12 % opacity, 1 px stroke.
- **Avatars (social profile set)**: 96 px circle, 2 px `DashColors::DASH_BLUE` ring at 20 % opacity. Fallback = `DashColors::DASH_BLUE` fill with first letter of display name in `DashColors::WHITE`.
- **Identities without a social profile**: type-glyph monogram in same ring (person glyph for User, masternode glyph for Masternode, evonode glyph for Evonode).
- **Monospace**: only on copyable identifiers (Identity ID, addresses, TxIDs, Masternode ID) and in Developer Mode JSON dumps.
- **Spacing**: `Spacing::XXL` (48 px) between major sections; `Spacing::MD` (16 px) inside cards; `Spacing::SM` (8 px) between label / value pairs.
- **Send / Receive sheets**: `Shadow::elevated()` with `DashColors::modal_overlay()` (rgba 0,0,0,120) backdrop. Send primary button focus state uses `Shadow::glow()` (rgba 0,141,228,30).

**Type-glyph monogram design decision**: user identities display a single-person silhouette
glyph (Unicode U+1F464 or SVG equivalent); masternode identities display an abstract node /
server glyph to signal infrastructure; evonode identities display a diamond glyph matching
the existing evonode icon in the codebase. All three render at 40 px within a 96 px circle.

---

## F. Wireframe Reference

`wireframe.html` renders 8 sequential frames on one scrolling page:

| Frame | Caption | Subtitle |
|---|---|---|
| 1 | Onboarding empty state | First-time welcome with two primary CTAs and a Developer Mode footer band. Breadcrumb shows `(no wallet yet)` and `(no identity yet)` placeholders. |
| 2 | Identity picker | Grid of identity cards shown when ≥ 2 identities are loaded. Four identity cards plus an "Add a new identity" card. Breadcrumb shows wallet pill + `(choose an identity)` placeholder. |
| 3 | Identity Home | Hero with avatar and display name, quick actions, onboarding checklist, recent activity. Canonical render: Alex with social profile. Annotation callout documents the no-profile state (see §B.3). |
| 4 | Contacts | Populated contacts page: 2 received requests (amber accent), 5 active contacts, 2 sent requests (blue accent). |
| 5 | Activity | Unified timeline: one expanded row, one failed row, ten normal rows across all three filter categories. |
| 6 | Send sheet | Username resolution in progress, amount input, memo expander, fee preview card. Rendered over a dimmed Identity Home backdrop. |
| 7 | Settings | Social profile section, aliases, keys table, danger zone. Priya context with interactive breadcrumb (multi-wallet). |
| 8 | App chrome reference | Component reference: left nav, breadcrumb switcher variants A (Alex, subdued wallet pill), B (Priya, both pills interactive), C (onboarding placeholders). Moved to end — readers encounter real screens first. |

CSS custom properties in `wireframe.html` mirror `src/ui/theme.rs` line-for-line. Every
variable maps to its Rust constant in comments. Exception: shadow alpha values are
intentionally brighter in the wireframe than in `theme.rs` — see §E for the rationale.

---

## G. Open Questions — Closed

| # | Question | Decision |
|---|---|---|
| G1 | Nav label: `My Profile` vs `Identities`? | **`Identities`** — existing codebase label preserved; minimizes churn for existing users. Approved by user this session. |
| G2 | DashPay brand prominence? | **Descriptor only** — kept in onboarding copy, power-user tooltips, and contact-related empty states. Removed only as nav label. |
| G3 | Bulk identity creation (IDN-011) placement? | **Quiet tertiary link** in Add-another-identity chooser modal, Developer Mode only. |
| G4 | Memo storage on sends? | **Split**: DashPay-routed payments store memo as part of the DashPay payment document; raw-address sends store memo locally. |
| G5 | Unload vs. delete identity in danger zone? | **Unload only** exposed. No "delete permanently" note shown — Platform does not support it today and surfacing a note implying it will exist is premature. |
| G6 | Identity pill dropdown drag-reorder? | **Deferred** to a later iteration. Default ordering: Local nickname → DPNS username → shortened Identity ID. Inline search at 7+ identities. |
| G7 | Local identity alias (`QualifiedIdentity.alias`) vs. DPNS aliases? | **Preserved as `Local nickname`** in §B.8 Settings. The field is not deprecated; it is renamed to disambiguate from DPNS on-chain aliases. Migration: existing `alias` values display as-is; no data loss. |
| G8 | `Request payment` (catalog #45) — UI placement? | **Future feature**. Catalog entry #45 is retained as a disabled-state tooltip ("Payment requests are coming soon.") for now. No active UI row in the wireframe. |
| G9 | Secondary Home actions visibility gating? | **All personas, no `.adv` gate**. Add funds / Send to wallet / Send to another identity are visible to Alex, Priya, and Jordan. Alex's funding path defaults to `UseWalletBalance` (§B.9); advanced funding methods are gated inside the wizard, not on the Home row. |
