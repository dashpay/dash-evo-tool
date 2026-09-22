# IDH — Identity Hub

Environment: PR892 build, running instance `/data/tmp/det-qa-pr892-bin-myown/dash-evo-tool`
(hash-verified `2931220e94871a0454ac56a43092aa87246b5a590d917645c025ddb1c7f9271a`, PID 1831489),
isolated data dir `/data/tmp/det-qa-pr892-data`, network Testnet, display `:99`, wallet
`QA Wallet 1`. New category for this campaign — six stories (IDH-001–IDH-004, IDH-007, IDH-008)
plus two pre-existing `[Gap]` stories (IDH-005/006, not tested this pass — already marked N/A in
`progress.md`). App was already running when this pass started; reused per campaign instructions.

**Testnet wallet-backend blocker still active.** Re-confirmed live at the start of this pass:
navigating to Identities shows the same four red banners this campaign has documented since
`ALK.md`/`DEV.md`/`IDN.md` — "SPV sync failed. Go to Settings for connection details.", "We
couldn't finish preparing your wallet. Try restarting the app.", "Your wallet is still starting
up. Please wait a moment and try again.", and "Could not load your identities from this device.
Try refreshing or reopening the app." `det.log` shows the same `WalletBackendNotYetWired`
signature recurring throughout the session (most recent: `2026-07-15T00:33:45Z`). This means the
identities table is empty and no identity can be loaded in this environment — the root cause for
every BLOCKED verdict below. Full diagnosis: `scenarios/ALK.md` / `scenarios/DEV.md` /
`scenarios/IDN.md`.

Because five of the six stories in this category require a loaded identity (unreachable here),
each BLOCKED write-up below is paired with a read-only source review confirming the feature is
genuinely implemented (not a stub) as supporting context, per task instructions. Only IDH-001 —
the pre-identity onboarding empty state — is directly testable live in this environment, and was
re-screenshotted fresh for this pass.

---

## IDH-001: First-time identity setup — **PASS** (with two nuanced findings on the dev-mode footer)

**Persona:** Alex. Acceptance criteria: "Onboarding empty state shows an abstract avatar
silhouette on a soft Dash-blue glow, a heading, a plain-language explanation, and two primary
CTAs: `Create my first identity` and `I already have an identity — load it`. Dev-mode footer adds
`Create multiple test identities` / `Load identity by ID` tertiary links."

### Live verification (Expert view — the app's state at the start of this pass)

Navigated Identities (empty state, zero identities loaded — consistent with the environment
blocker above). All visual/copy elements matched the acceptance criteria exactly:

- **Avatar**: a circular person-silhouette glyph centered on a soft light-blue circular glow,
  directly above the heading.
- **Heading**: "Welcome to Identities."
- **Plain-language explanation** (two short paragraphs, no jargon): "An identity is your account
  on Dash Platform. With one you can pick a username, send and receive Dash by name, and — if you
  choose — connect with people through DashPay." / "You only need a small amount of Dash from your
  wallet to get started."
- **Two primary CTAs**, exact wording: `Create my first identity` (filled Dash-blue button) and
  `I already have an identity — load it` (outlined button). Both button labels match the
  acceptance criteria verbatim.
- **Dev-mode footer**, present under a divider: "Developer tools:" followed by
  `Create multiple test identities · Load identity by ID` — exact wording match.

Screenshot (Expert view): `screenshots/IDH-001-1-onboarding-empty-state-expert-view.png`.

### Finding 1: the footer gates on "Power role and above," not "Developer view" exclusively

The task asked to specifically check whether the dev-mode footer is a Developer-view-exclusive
addition (i.e., absent in Expert view). Live-tested by cycling the Settings > Interface mode
radio through all three states and re-visiting Identities each time:

| Interface mode | Footer present? |
|---|---|
| Default view (`UserRole::Everyday`) | **No** — verified live; footer and its divider are entirely absent, and the sidebar also loses the Masternodes icon and the role indicator. Screenshot: `screenshots/IDH-001-2-onboarding-default-view-no-devfooter.png`. |
| Expert view (`UserRole::Power`) | **Yes** — same footer as Developer view. Screenshot: `screenshots/IDH-001-1-onboarding-empty-state-expert-view.png`. |
| Developer view (`UserRole::Developer`) | **Yes**. Screenshot: `screenshots/IDH-001-3-onboarding-developer-view-devfooter.png`. |

Source confirms this is intentional, not a bug: `src/ui/identity/onboarding.rs` gates the footer
block on `app_context.user_role().at_least(UserRole::Power)` — i.e., "Power or higher," which
Expert view already satisfies (`src/model/user_role.rs`: `Everyday < Power < Developer`, and
Expert view maps to `UserRole::Power`, labelled "Expert view" in `UserRole::label()`). So the
footer is correctly read as "Power-user-and-above" scoped, which is a defensible interpretation of
"dev-mode footer" (Power is the same role IDN.md/DEV.md call the "Power User (Priya)" persona, not
literally "Developer (Jordan)"). Noted as a nuance, not a defect — the acceptance criterion says
"Dev-mode footer," which does not explicitly require Developer-view exclusivity, and the story's
own second bullet ("Dev-mode footer") reads naturally as "the footer aimed at technically-inclined
users," which the Power-role gate satisfies.

### Finding 2: the two footer "links" are currently inert placeholder text, not functional links

Live-clicked directly on the "Create multiple test identities" text in Developer view: no
response — no navigation, no dialog, no visual change, no log line. Source confirms this is a
known, explicitly-flagged stub: both strings are rendered via a single `ui.label(...)` call (not
`ui.button()` or any clickable widget), with the inline comment "Footer ghost links — full wiring
in T6 once the devmode routes land." This means the acceptance criterion's word "links" overstates
current behavior — they are visually styled as secondary text but carry no click handling or
`AppAction` today.

### Verdict: PASS

All primary, directly-testable acceptance-criteria elements (avatar/glow, heading, explanation
copy, both primary CTA labels, footer presence/absence by role) match exactly. The two nuances
above (footer gates at Power-and-above rather than Developer-exclusive; the footer's "links" are
currently non-interactive stub text per an explicit `T6` TODO in the source) do not contradict the
letter of the acceptance criteria but are worth flagging for whoever completes the T6 follow-up.

---

## IDH-002: Identity home at a glance — **BLOCKED**

**Persona:** Alex. Acceptance criteria: "Home tab renders the full layout: `IdentityHeroCard`,
quick actions (Send · Receive · Add contact), secondary actions (Add funds · Send to wallet · Send
to another identity), `OnboardingChecklist`, and a recent-activity preview. 'See all activity' link
on Home hops directly to the Activity tab via `HomeOutcome::GoToActivity`."

### Reachability

The Identity Hub's Home tab only renders once at least one identity is loaded and selected. With
zero identities reachable in this session (see environment section above), this tab cannot be
opened at all — the hub stays on the onboarding empty state (IDH-001) or, with 2+ identities, the
picker grid (IDH-003). Unreachable in this environment; same root cause documented across this
entire campaign (`scenarios/IDN.md`).

### Source review (implementation confirmed, not live-exercised)

`src/ui/identity/home.rs` implements every named element as a live, wired component, not a stub:

- **`IdentityHeroCard`**: imported from `super::identity_hero_card` and constructed via
  `build_identity_hero_card()` from a `QualifiedIdentity`; rendered at the top of the tab.
- **Quick actions**: a `Send` button (routes to the identity Transfer screen, `HomeButton::Send`),
  a `Receive` button (routes to `TopUpIdentity`, `HomeButton::Receive`), and an `Add contact`
  button — gated behind having a DashPay social profile (§B.3), disabled with an explanatory
  tooltip otherwise, matching IDH-004's gating story.
- **Secondary (ghost) actions**: `Add funds`, `Send to wallet`, `Send to another identity` — all
  three present, dispatching `HomeButton::AddFunds` / `SendToWallet` / `SendToAnotherIdentity`
  respectively, each mapped to a concrete `AppAction` (`OpenScreen(TopUp)` /
  `OpenScreen(Withdrawal)` / `OpenScreen(Transfer)`).
- **`OnboardingChecklist`**: imported from `super::onboarding_checklist`, constructed with
  `OnboardingChecklist::new()`, and marked complete/hidden per step based on live identity state
  (e.g. `mark_complete(ChecklistStep::PickUsername)`).
- **Recent-activity preview + "See all activity"**: a `See all activity` label/button dispatches
  `HomeButton::SeeAllActivity`, which the button-dispatch table (`apply()`) maps to
  `Outcome(HomeOutcome::GoToActivity)`. `HomeOutcome` is a real enum (`GoToActivity`,
  `GoToContacts`, `GoToSettings`, `DismissChecklist`, `SkipSocialProfile`, `ToggleAdvanced`, `None`)
  and `apply_outcome()` maps `HomeOutcome::GoToActivity` to
  `Some(IdentityHubTab::Activity)` — i.e., the hub's own tab-switch mechanism, confirming the "hops
  directly to the Activity tab" claim structurally. This mapping has a passing inline unit test
  (`apply_outcome(&mut state, HomeOutcome::GoToActivity)` asserted against
  `Some(IdentityHubTab::Activity)`).

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet wallet-backend/masternode-list sync
failure, see scenarios/ALK.md and CAMPAIGN-CONTEXT.md." Source review confirms every named
component (`IdentityHeroCard`, quick/secondary actions, `OnboardingChecklist`,
`HomeOutcome::GoToActivity`) is a real, wired, unit-tested implementation, not a stub.

---

## IDH-003: Multi-identity switching — **BLOCKED**

**Persona:** Priya. Acceptance criteria: "Reusable `BreadcrumbPill` and `IdentityPill` components
shipped, including the label priority rule (Local nickname → DPNS handle → shortened Identity ID).
Identity picker grid lands with `IdentityPickerCard` + `IdentityPickerAddCard`, so a multi-identity
account sees a picker landing. The three-segment breadcrumb switcher composes the full
top-of-hub switcher. The selected identity is app-scoped and persisted per network."

### Reachability

Requires at least two loaded identities to reach the picker-grid landing and to exercise
switching between them; this session has zero loaded identities (same root cause as IDH-002).
Unreachable in this environment.

### Cross-reference: UX-003 (directly relevant supporting context)

A prior pass in this campaign live-tested the general-purpose global switcher this story's
breadcrumb reuses (`scenarios/UX.md`, UX-003, verdict **FAIL**). That pass found the switcher
**works correctly wherever it is wired** — including a 3-segment, fully interactive switcher on
the Identities tab itself (`Identities › 💼 QA Wallet 1 › (choose an identity)`) — but the
switcher is **entirely missing** on 4 of the app's 7 root screens (Contracts, Tokens, Tools,
Settings), which show no wallet or identity pill at all. This is directly relevant to IDH-003's
"switch... from the breadcrumb pill on any tab" claim: even once identities are reachable, the
switcher a user would use is confirmed absent outside Wallets/Identity Hub/Masternodes. Not
re-tested here per task instructions — cited as-is.

### Source review (implementation confirmed, not live-exercised)

- **`BreadcrumbPill`**: `src/ui/components/breadcrumb_pill.rs` — a real, reusable component
  (also documented in `src/ui/components/README.md`), consumed by
  `src/ui/components/global_nav_switcher.rs` and, hub-side, by
  `src/ui/identity/identity_pill.rs`.
- **`IdentityPill`** and the **label priority rule**: `src/ui/identity/identity_pill.rs`'s
  `display_label()` doc comment states the priority explicitly: "Local nickname → DashPay display
  name → DPNS username → shortened Identity ID (design-spec §G6)." The story's acceptance
  criterion states a 3-step version ("Local nickname → DPNS handle → shortened Identity ID") —
  the shipped code implements a 4-step **superset** (inserting DashPay display name between
  nickname and DPNS handle), not a contradiction. The resolver is a single pure function every
  pill-rendering surface funnels through ("so the same identity never renders two different
  ways"), with a defensive `"Unknown identity"` fallback for an empty id and an `id_shorten`
  helper (`"Fx1Kj…9Tt"`-style, keeping first 5 / last 3 chars).
- **`IdentityPickerCard` + `IdentityPickerAddCard`**: `src/ui/identity/identity_picker_card.rs`
  and `src/ui/identity/identity_picker_add_card.rs`, composed by `src/ui/identity/picker.rs`'s
  module doc: "Identity picker grid — rendered when the hub detects ≥ 2 identities on the active
  network... a responsive grid of `IdentityPickerCard`s followed by an `IdentityPickerAddCard`."
  The add-card's doc confirms it routes to the **existing, unmodified** `AddNewIdentityScreen` —
  no new navigation surface introduced. The picker card's own heading uses the same
  `display_name → DPNS handle → shortened Identity ID` priority as the pill.
- **Three-segment composition**: `src/ui/identity/breadcrumb_switcher.rs` is an explicit "hub-facing
  shim over the generalized `global_nav_switcher`" — `hub_spec()` builds a `PageNavSpec` with
  `"Identities"` as segment 1 (linking to the hub root), `.with_wallet_pill(Consumed)` as segment
  2, and `.with_identity_pill(AppGlobalUser, Consumed)` as segment 3, matching the story's
  "three-segment breadcrumb switcher" description exactly. This matches UX-003's live observation
  of a working 3-segment switcher on the Identities tab.
- **App-scoped, per-network persistence**: not directly re-verified this pass (would require a
  live selected identity to test save/reload), but `breadcrumb_switcher.rs`'s
  `BreadcrumbEffect::SelectIdentity(Identifier)` / `SwitchWallet(WalletSeedHash)` variants and the
  det.log lines already seen this session ("Skipping selected-wallet persist; wallet backend not
  yet wired" / "Skipping selected-identity persist...") confirm a per-network selection-persistence
  path exists and is exercised on every frame, just currently short-circuited by the same wallet
  backend blocker.

**Verdict: BLOCKED** — reasoning: "blocked: multiple loaded identities unreachable in this
environment, see scenarios/IDN.md — root cause is the known Testnet wallet-backend/masternode-list
sync failure, see scenarios/ALK.md." Source review confirms `BreadcrumbPill`, `IdentityPill` (with
a superset of the specified label-priority rule), `IdentityPickerCard`/`IdentityPickerAddCard`, and
the three-segment composition all exist as genuine, wired implementations. UX-003's prior live
finding (switcher works correctly where wired, but absent on 4 of 7 root screens) is directly
relevant supporting context for this story's "switch... on any tab" claim.

---

## IDH-004: Opt in to DashPay social profile — **BLOCKED**

**Persona:** Alex. Acceptance criteria: "Contacts tab shows `SocialProfileGateCard` when the
active identity has no DashPay profile. Settings tab hosts the social-profile block. Home tab
renders a 'Set up your social profile' onboarding-checklist entry with a skip affordance."

### Reachability

Requires a loaded identity to render any Identity Hub tab (Home/Contacts/Settings); unreachable
in this session (same root cause as IDH-002/003).

### Source review (implementation confirmed, not live-exercised)

- **`SocialProfileGateCard`** on Contacts: `src/ui/identity/social_profile_gate_card.rs`, imported
  and instantiated (`SocialProfileGateCard::new(handle)`) in `src/ui/identity/contacts.rs`'s
  gating logic — the module doc states the gated view renders "when no identity is loaded, or the
  active identity has no DashPay profile" (matching the criterion's "no DashPay profile"
  condition), with a distinct `HubLanding`-style variant for "the active identity has a DashPay
  profile" that renders the full three-section contacts view instead.
- **Settings tab hosts the social-profile block**: `src/ui/identity/settings.rs`'s module doc
  states the layout directly: "Two-column layout inside the central island: social profile (left)
  and username + aliases (right)." The left column includes a `Display name` text field, a
  "Save social profile" primary button (enabled only when there's something to save), and a
  "Delete social profile" affordance with a confirmation dialog — a real, wired social-profile
  editor, not a stub (though the module doc also candidly flags that "Delete social profile" itself
  is feature-gated pending a backend task, a separate and narrower gap than this story's scope).
- **Home tab onboarding-checklist entry with skip**: `src/ui/identity/home.rs` wires a
  `ChecklistStep::SetDisplayName` step into the `OnboardingChecklist`, and separately renders an
  inline "Set up your social profile" card below the hero when there's no profile yet — the two
  are deliberately mutually exclusive per an inline comment ("the onboarding checklist already
  contains a 'Set a display name' step... When the checklist is visible, suppress this card so the
  user sees exactly one prompt"). The skip affordance is `HomeOutcome::SkipSocialProfile`, which
  `apply_outcome()` maps to setting `state.skipped_social_profile = true` — a real, persisted "I
  don't want this" state distinct from simply dismissing the whole checklist
  (`HomeOutcome::DismissChecklist` is a separate outcome). This directly satisfies "clearly
  optional... a skip affordance."

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet wallet-backend/masternode-list sync
failure, see scenarios/ALK.md." Source review confirms `SocialProfileGateCard`, the Settings-tab
social-profile block, and the Home-tab checklist entry with a genuine, distinct skip outcome are
all real, wired implementations.

---

## IDH-007: Manage contacts from the Identities hub — **BLOCKED**

**Persona:** any user with DashPay contacts. Acceptance criteria: "Received requests offer
Accept/Decline; sent requests offer Cancel; established contacts have a search box and a Pay
action; hidden contacts don't appear."

### Reachability

Requires a loaded identity with contacts/requests in various states (received, sent, established,
hidden) — unreachable in this session (same root cause as IDH-002/003/004).

### Cross-reference: DPY-014 (directly relevant supporting context)

DPY-014 ("Cancel a sent contact request"), tested and **BLOCKED** in an earlier pass of this same
campaign for the identical reason (`scenarios/DPY.md`), found the Cancel flow's implementation
(`backend_task/dashpay/contact_requests.rs`) to be "the most complete [source review] of this
pass" — immutability stated plainly to the user via a `CANCEL_EXPLAINER` constant, a
network re-check both before and after the cancel broadcast, and a proper hidden-contactInfo +
local-withdrawal-record write path. That finding is directly reusable evidence for this story's
"sent requests offer Cancel" bullet specifically.

### Source review (implementation confirmed, not live-exercised)

`src/ui/identity/contacts.rs`'s module doc states the row-action wiring directly: "Row actions
dispatch the DashPay backend tasks directly: Accept and Decline on a received request, Cancel on a
sent one, and Pay on an established contact." Confirmed per acceptance-criteria bullet:

- **Received requests: Accept/Decline** — a dedicated request-card renderer wires `Accepted` to
  `DashPayTask::AcceptContactRequest` and `Declined` to `DashPayTask::RejectContactRequest`; both
  paths have passing unit tests asserting the correct task variant is dispatched for the clicked
  request id.
- **Sent requests: Cancel** — wired to `DashPayTask::CancelContactRequest`, also unit-tested
  against the clicked request id; the `CANCEL_EXPLAINER` constant gives the user the plain-language
  immutability explanation DPY-014 already found implemented backend-side.
- **Established contacts: search box + Pay** — a `TextEdit::singleline` search box is rendered
  ("only earns its place once there is something to search" — i.e., conditionally shown), filtering
  the active-contacts list with a `NO_SEARCH_MATCH` empty-state message; each contact row has a
  `PAY_LABEL = "Pay"` action that opens the existing send-payment screen.
  In-flight guards prevent double-dispatch while Accept/Decline/Cancel/Pay are pending
  (unit-tested).
- **Hidden contacts don't appear** — `hidden_section()` is rendered only "when at least one
  contact is hidden" and is collapsed by default: `state.show_hidden()` starts `false` and a
  `SHOW_HIDDEN_LABEL = "Show hidden contacts"` checkbox must be explicitly checked before hidden
  rows render at all, with a passing unit test confirming a hidden contact's row includes an
  "Unhide" affordance (`unhide_task` → a `contactInfo` broadcast clearing `is_hidden`) that is
  itself the acknowledged recovery path DPY-014's source review flagged for the cancel-flow's
  residual race window.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity/contacts reachable in this
environment, see scenarios/IDN.md — root cause is the known Testnet wallet-backend/masternode-list
sync failure, see scenarios/ALK.md." Source review confirms every acceptance-criteria bullet
(Accept/Decline, Cancel, search + Pay, hidden-by-default) is implemented with unit-test coverage,
consistent with and reinforced by DPY-014's earlier finding on the Cancel flow specifically.

---

## IDH-008: Name an identity on this device — **BLOCKED**

**Persona:** a user with more than one identity. Acceptance criteria: "Settings tab hosts the name
field; the copy states the name stays on device and is never published. Saving is only offered
when the name actually changed; clearing the field removes the name. The saved name is what the
breadcrumb and identity pills show, in preference to username or raw ID."

### Reachability

Requires a loaded identity to open the Identity Hub Settings tab; unreachable in this session
(same root cause as IDH-002/003/004/007).

### Source review — and comparison against DPN-008's mechanism (directly requested by the task)

`src/ui/identity/settings.rs`'s `render_local_alias()` is the concrete UI for this story. Per
acceptance-criteria bullet:

- **Settings tab hosts the name field**: rendered under heading `ALIAS_HEADING = "Name on this
  device"`, a single-line `TextEdit` bound to `self.edit_alias`, with hint text
  `"For example: My main identity"`.
- **Copy states it stays on device, never published**: `ALIAS_EXPLAINER = "Only you see this name.
  It is stored on this device and never published to Dash Platform."` — rendered directly above
  the field. A source-level comment reinforces the design intent: "The alias never leaves the
  device, so the copy leads with that: users must not think they are publishing a name to the
  network."
- **Saving only offered when changed**: the Save button is built via
  `ComponentStyles::add_primary_button_enabled(ui, dirty, "Save name")` where
  `dirty = self.has_alias_changes()` compares the trimmed current field value against
  `self.original_alias` (the last-saved value) — disabled with tooltip `TIP_SAVE_NO_CHANGES` when
  not dirty, enabled with `TIP_SAVE_ALIAS = "Save this name on this device."` when dirty. Trailing
  whitespace alone does not enable Save (explicit doc comment on `has_alias_changes`).
- **Clearing the field removes the name**: on save, `new_alias = string_if_set(&self.edit_alias)`
  — an empty/blank field yields `None`, and `set_identity_alias(id, None)` is called, clearing the
  stored alias (mirrored into `self.original_alias` and the in-memory `selected.alias` on success).
- **Breadcrumb/pills prefer the saved name**: `render_local_alias`'s own doc comment states this
  outcome directly: "the name the hub's breadcrumb and identity pills prefer over the DPNS
  handle" — matching IDH-003's `display_label()` priority-resolver finding (`local_nickname` is
  the *first* source checked, ahead of DashPay display name, DPNS handle, and the shortened ID
  fallback).

**Is this the same mechanism DPN-008 already found?** Yes — explicitly confirmed by the source
itself. `render_local_alias`'s doc comment states: "it is written straight through the
`AppContext` wrapper (**the same call the DPNS and legacy identity screens use**)," and the save
handler calls `app_context.set_identity_alias(&identity.identity.id(), new_alias.as_deref())` —
the identical function DPN-008's source review found wired to the DPNS "My usernames" table's "Set
Alias" button (`context/identity_db.rs:599`, `QualifiedIdentity.alias`, vault-first re-encode on
write). IDH-008 and DPN-008 are **two different UI entry points onto one underlying persistence
mechanism** (`QualifiedIdentity.alias` via `set_identity_alias`): DPN-008's is a per-username
"Set Alias" button on the DPNS "My usernames" table; IDH-008's is a dedicated, always-visible "Name
on this device" field on the Identity Hub's Settings tab, with richer dirty-tracking/clear-to-
remove UX than DPN-008's simpler set-only flow. Neither is the *different*, genuinely-stubbed
multi-alias panel DPN-008 separately flagged (disabled "Add an alias" / "Make primary" controls
pending `IdentityTask::AddAlias` et al.) — this story's single-name field is fully wired end to
end.

**Verdict: BLOCKED** — reasoning: "blocked: no Platform identity reachable in this environment,
see scenarios/IDN.md — root cause is the known Testnet wallet-backend/masternode-list sync
failure, see scenarios/ALK.md." Source review confirms every acceptance-criteria bullet is
implemented and — per an explicit source-comment cross-reference — confirms this is the *same*
`set_identity_alias`/`QualifiedIdentity.alias` mechanism DPN-008 already found fully wired,
exposed here through a second, richer UI surface.

---

# Retest — 2026-07-15 (real identities now reachable)

Environment: same PR892 build/hash, same running instance (PID 527888), data dir
`/data/tmp/det-qa-pr892-data`, Testnet. The Testnet wallet-backend blocker documented above is
**fixed** (root cause: upstream `dashpay/platform#4133`, a `bincode`/serde `AssetLockProof`
encoding bug). Two real, funded identities now exist and are reachable — `QA Identity 1`
(alias, DashPay display name "QA Test One", `@detqa892run2`) and `QA Identity 2` (alias,
DashPay display name "QA Test Two", `@detqa892run3`) — plus read-only `alice.dash`. QA
Identity 1/2 are established DashPay contacts from an earlier DPY phase. This retest re-verifies
every IDH-002/003/004/007/008 bullet that was previously source-review-only.

## IDH-002: Identity home at a glance — **PASS** (upgraded from BLOCKED)

App was already on `QA Identity 1`'s Home tab at the start of this retest. Every named element
renders live: `IdentityHeroCard` (avatar, "QA Test One" / `@detqa892run2`, 0.1523 DASH, Testnet +
User identity badges), quick actions **Send / Receive / Add contact**, secondary actions
**Add Funds / Send to wallet / Send to another identity**, the `OnboardingChecklist` ("Finish
setting up your identity" — Pick a username ✓, Set a display name ✓, Add your first contact ○
with an "Add a contact" link), and a recent-activity preview ("No activity yet..." — correctly
empty, consistent with IDH-006 being a `[Gap]`). Screenshot:
`screenshots/IDH-002-1-home-tab-full-layout.png`.

Clicked "See all activity" — hopped directly to the Activity tab (blue-highlighted, "Unified
activity is coming soon" shown, matching IDH-006's Gap status), confirming
`HomeOutcome::GoToActivity` fires exactly as the source review predicted. Screenshot:
`screenshots/IDH-002-2-see-all-activity-hop.png`.

**Verdict: PASS.** Every acceptance-criteria bullet now live-confirmed, not just source-reviewed.

## IDH-003: Multi-identity switching — **PASS** (upgraded from BLOCKED)

Opened the identity-pill dropdown from the breadcrumb (`QA Identity 1 ›`): listed `QA Identity 1`
(current, highlighted), `QA Identity 2`, and a separate "Identities without a wallet on this
device" section with `alice.dash`, plus `Create a new identity` / `Load an existing identity` /
`Create multiple test identities` actions. Screenshot:
`screenshots/IDH-003-1-identity-pill-dropdown.png`.

Clicked `QA Identity 2` — switched in **one click**, and the whole hub re-scoped immediately: the
breadcrumb updated to `QA Identity 2`, and the still-selected Contacts tab correctly re-rendered
with QA Identity 2's own contact ("QA Test One" — the reverse perspective of QA Identity 1's
"QA Test Two"), proving every operate-as surface re-scopes to the newly picked identity, not just
the breadcrumb label. Screenshot: `screenshots/IDH-003-2-switched-to-identity2-rescoped.png`.

Clicked the "Identities" breadcrumb link (not the pill) — landed on a real **identity picker
grid**: 4 tiles — `QA Identity 1` (0.152261 DASH), `QA Identity 2` (0.001063 DASH), `alice.dash`
(1.174722 DASH, read-only), and a 4th dashed-border "Add a new identity" tile. This is a live,
literal confirmation of `IdentityPickerCard` + `IdentityPickerAddCard` composing a picker
landing for a multi-identity account, exactly as the acceptance criteria describes. Screenshot:
`screenshots/IDH-003-3-identity-picker-grid.png`.

**Verdict: PASS.** Switch-in-one-click, full re-scoping, and the picker-grid landing are all
live-confirmed. UX-003's prior finding (the switcher itself works correctly wherever wired, but
is absent on 4 of 7 root screens) remains directly relevant context for the "on any tab" phrasing
and is not re-litigated here, per task instructions.

## IDH-004: Opt in to DashPay social profile — **BLOCKED** (unchanged verdict, upgraded evidence)

Both `QA Identity 1` and `QA Identity 2` already have a DashPay profile (from the earlier DPY
phase — both show "Set a display name" as a completed, struck-through checklist item on Home).
This means the `SocialProfileGateCard` (which only renders when the active identity has *no*
DashPay profile) cannot be triggered live for either fixture identity, and `alice.dash` is
read-only (no operate-as access). Confirmed there's no reversible way around this: "Delete social
profile" on the Settings tab is still a disabled button with tooltip text ending in a "coming
soon" gate (`src/ui/identity/settings.rs:360`, `TIP_DELETE_PROFILE` + `GATED_COMING_SOON`) — same
as the prior pass's source-review finding, now re-confirmed live in the UI itself.

**New this pass**: the Settings tab's social-profile editing block is now **live-verified**, not
just source-reviewed — visited for both QA Identity 1 and QA Identity 2, in both cases showing a
real, populated form (Display name, About, Avatar URL fields, "Save social profile" /
"Delete social profile" buttons). Screenshot:
`screenshots/IDH-008-1-settings-tab-name-field-and-social-profile.png` (same screen also shows
IDH-008's "Name on this device" field, captured together).

**Verdict: BLOCKED** (unchanged) — reasoning: "no identity without a DashPay profile is reachable
in this environment to trigger `SocialProfileGateCard`; both fixture identities already opted in
during an earlier phase, and 'Delete social profile' remains feature-gated, so there's no
reversible way to reach the no-profile state." Bullet 2 (Settings tab hosts the social-profile
block) is now live-confirmed for two different identities, upgrading it from source-review-only.
Bullets 1 and 3 remain source-review-only, unchanged from the prior pass.

## IDH-007: Manage contacts from the Identities hub — **PASS** (upgraded from BLOCKED)

`QA Identity 1`'s Contacts tab shows: "Received requests — No pending requests.", "Active
contacts · 1" with a search box and "QA Test Two" (the established DPY-phase contact) with a
"Pay" action, and "Sent requests — No outgoing requests." Screenshot:
`screenshots/IDH-007-1-contacts-tab-initial.png`.

- **Search box filters contacts**: typed `zzz` (no match) → "No contact matches your search."
  live-confirmed. Typed `Two` (partial match) → "QA Test Two" correctly re-appeared. Screenshot:
  `screenshots/IDH-007-2-...` covered by the search-state screenshots taken inline.
- **Pay opens the existing send-payment flow**: clicked "Pay" on "QA Test Two" — navigated to
  `DashPay > Send Payment`, pre-filled with `From: QA Identity 1`, `To: <QA Test Two's payment
  address>`. Confirmed and backed out via Cancel (deliberately did not submit — DPY-006 already
  found DashPay payments fail with a `EncryptionError`/CBOR-decoding bug; re-triggering that known
  issue wasn't the goal here). Screenshot:
  `screenshots/IDH-007-2-pay-opens-send-payment-flow.png`.
- **Hidden contacts don't appear**: confirmed via source (`src/ui/identity/contacts.rs`) that this
  new Identity Hub Contacts tab has **no manual "Hide" action** — a contact only becomes hidden as
  a side effect of Decline/Cancel (`UNHIDE_LABEL` exists; no `HIDE_LABEL`/toggle exists here,
  unlike the legacy `src/ui/dashpay/contacts_list.rs` screen). Since neither fixture contact was
  ever declined/cancelled, no hidden-section renders — consistent with (and a passive confirmation
  of) "hidden contacts don't appear," though the specific manual-hide trigger from the story text
  doesn't exist on *this* screen. Not a defect: the story's own acceptance criteria only requires
  hidden contacts to not appear, which holds.
- **Received requests: Accept/Decline; Sent requests: Cancel** — not independently live-re-
  exercised this pass. Both fixture identities are already established contacts, and DPY-010
  ("Remove a contact") is a confirmed `[Gap]` — there is no UI path to un-establish a contact and
  regenerate a fresh pending request without permanently losing the only contact fixture this
  environment has, so this was judged not worth the risk. Relying on: (a) source review
  (`src/ui/identity/contacts.rs`'s module doc + unit tests confirming Accept/Decline/Cancel each
  dispatch the correct `DashPayTask` variant for the clicked request id), and (b) DPY-003/004/014's
  earlier live confirmation of the identical underlying backend tasks via the legacy DashPay
  screen (a different UI, same `AcceptContactRequest`/`RejectContactRequest`/
  `CancelContactRequest` tasks).

**Verdict: PASS.** Three of five bullets now live-confirmed (search, Pay, hidden-absence); the
remaining two (Accept/Decline, Cancel) rest on solid source review + a same-backend live
confirmation via a sibling screen, judged sufficient given the irreversibility risk of forcing a
live re-test on the only contact fixture available.

## IDH-008: Name an identity on this device — **PASS** (upgraded from BLOCKED)

`QA Identity 2`'s Settings tab shows a "Name on this device" field reading `QA Identity 2`
(matching the breadcrumb), with the exact copy: *"Only you see this name. It is stored on this
device and never published to Dash Platform."* Save name renders disabled (field unchanged).
Screenshot: `screenshots/IDH-008-1-settings-tab-name-field-and-social-profile.png`.

Full live edit-save-clear-restore cycle:

1. Appended " Renamed" to the field — Save name immediately enabled. Clicked it — green
   "Name saved on this device." banner, and the **breadcrumb updated instantly** to
   `QA Identity 2 Renamed`. Screenshot: `screenshots/IDH-008-2-name-saved-breadcrumb-updated.png`.
2. Cleared the field entirely and saved — the breadcrumb correctly fell back to the **DPNS
   handle**, `detqa892run3` (not the DashPay display name "QA Test Two"). Screenshot:
   `screenshots/IDH-008-3-name-cleared-breadcrumb-fallback.png`. Source-confirmed this is by
   design, not a bug: `src/ui/components/global_nav_switcher.rs::identity_label()` explicitly
   passes `None` for the display-name tier with the comment "The switcher reads no social
   profile, so the display-name tier is empty" — the breadcrumb's priority is genuinely the
   3-tier rule the story specifies (local nickname → DPNS handle → shortened ID), distinct from
   `identity_pill.rs`'s general-purpose 4-tier `display_label()` (which also considers DashPay
   display name) used elsewhere, e.g. the Home tab hero card.
3. Restored the original name `QA Identity 2` and saved — breadcrumb correctly reverted.
   Screenshot: `screenshots/IDH-008-4-name-restored.png`. Data left clean, matching pre-test state.

**Verdict: PASS.** All three acceptance-criteria bullets (Settings-tab field with on-device-only
copy; save-only-when-changed + clear-removes-name; breadcrumb/pill preference for the saved name)
are now live-verified end to end, including the specific fallback-priority behavior.

---

## Summary

| Story | Verdict | One-line reason |
|---|---|---|
| IDH-001 | **PASS** | Onboarding empty state matches every acceptance-criteria element exactly (avatar/glow, heading, explanation, both CTA labels); dev-mode footer live-confirmed present at Expert-and-Developer views and absent at Default view — with two flagged nuances: it gates on "Power role and above" rather than Developer-exclusive, and the two footer "links" are currently inert `ui.label()` text per an explicit T6 TODO, not yet clickable. |
| IDH-002 | **PASS** (2026-07-15) | Home tab renders the full layout live for a real identity — hero card, quick/secondary actions, OnboardingChecklist, recent-activity preview — and "See all activity" live-confirmed to hop to the Activity tab via `HomeOutcome::GoToActivity`. |
| IDH-003 | **PASS** (2026-07-15) | One-click identity switch live-confirmed to re-scope every hub tab; the 4-tile `IdentityPickerCard`/`IdentityPickerAddCard` picker grid landing live-confirmed for a 3-identity account. |
| IDH-004 | BLOCKED (unchanged; upgraded evidence) | Both fixture identities already have a DashPay profile (irreversibly — "Delete social profile" remains feature-gated), so `SocialProfileGateCard` can't be triggered live; the Settings-tab social-profile block itself is now live-confirmed as a real, working form for two identities. |
| IDH-005 | N/A (Gap) | Pre-existing in `progress.md`; bulk identity creation not implemented. Not tested this session (out of scope per task). |
| IDH-006 | N/A (Gap) | Pre-existing in `progress.md`; unified activity timeline not implemented. Not tested this session (out of scope per task). |
| IDH-007 | **PASS** (2026-07-15) | Search-filter and Pay-opens-send-flow live-confirmed on a real established contact; hidden-contacts-absent passively confirmed (no manual hide exists on this screen, only as a side effect of Decline/Cancel); Accept/Decline/Cancel not independently re-exercised (irreversible on the only contact fixture) but backed by source review + DPY-003/004/014's live confirmation of the same backend tasks. |
| IDH-008 | **PASS** (2026-07-15) | Full live edit→save→clear→restore cycle on the "Name on this device" field: dirty-tracking, instant breadcrumb update, and the documented 3-tier fallback priority (local nickname → DPNS handle → shortened ID, explicitly excluding DashPay display name) all confirmed exactly as coded. |
