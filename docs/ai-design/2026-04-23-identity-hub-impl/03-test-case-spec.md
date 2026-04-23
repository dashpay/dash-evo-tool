# Phase 1c — Test Case Specification

Specifications only. Code lives in `tests/kittest/` and inline `#[cfg(test)]` modules.

## Unit tests (per component)

### UT-BPILL-01 — `BreadcrumbPill::new` stores props

**Preconditions**: construct with `BreadcrumbPill::new("Main Wallet")`.
**Steps**: read `label()` accessor.
**Expected**: returns the exact label string.

### UT-BPILL-02 — `BreadcrumbPill::subdued` flag

**Preconditions**: construct, call `.subdued(true)`.
**Steps**: inspect `is_subdued()`.
**Expected**: returns `true`. Render path does not draw a chevron in this mode.

### UT-BPILL-03 — `BreadcrumbPill::placeholder` renders italic

**Preconditions**: construct with `BreadcrumbPill::placeholder("(no wallet yet)")`.
**Steps**: inspect response after render.
**Expected**: `response().is_interactive == false`, `response().placeholder == true`.

### UT-BSWITCH-01 — `BreadcrumbSwitcher` composition

**Preconditions**: build switcher with three segments: plain link, wallet pill, identity
pill.
**Steps**: call `.show(ui)`.
**Expected**: response struct reports which segment was clicked (if any).

### UT-IDPILL-01 — Identity pill label priority

**Preconditions**: identity has `local_nickname=Some("dev")`, `dpns_handle=Some("alex.dash")`,
`identity_id="Fx1Kj…9Tt"`.
**Steps**: compute display label via `IdentityPill::display_label`.
**Expected**: returns `"dev"`. (Local nickname wins.)

### UT-IDPILL-02 — Identity pill label priority — no nickname

**Preconditions**: identity has `local_nickname=None`, `dpns_handle=Some("alex.dash")`,
`identity_id="..."`.
**Expected**: returns `"alex.dash"`.

### UT-IDPILL-03 — Identity pill label priority — raw ID fallback

**Preconditions**: identity has `local_nickname=None`, `dpns_handle=None`, `identity_id="Fx1Kj…9Tt"`.
**Expected**: returns `"Fx1Kj…9Tt"` (shortened, monospace).

### UT-PICKER-01 — `IdentityPickerCard::heading` priority matches pill

**Preconditions**: identity has `display_name=Some("Alex")`, `dpns_handle=Some("alex.dash")`.
**Expected**: heading = `"Alex"`; sub-line = `"@alex.dash"`.

### UT-PICKER-02 — `IdentityPickerCard` no display name

**Preconditions**: `display_name=None`, `dpns_handle=Some("mn-east-01.dash")`.
**Expected**: heading = `"mn-east-01.dash"`; sub-line = the identity-type label.

### UT-PICKER-03 — `IdentityPickerAddCard` has dashed border

**Preconditions**: default construction.
**Steps**: inspect render settings.
**Expected**: border style reports dashed; hover switches to solid Dash-blue.

### UT-TABS-01 — `IdentityHubTabBar` selection

**Preconditions**: tab bar with all four tabs, selected = Home.
**Steps**: click the Contacts tab via kittest.
**Expected**: response returns `Some(IdentityHubTab::Contacts)`; internal selection
updated.

### UT-CHECKLIST-01 — Onboarding checklist completion

**Preconditions**: checklist with three steps, `Pick a username` marked complete.
**Steps**: render.
**Expected**: first step rendered with check mark; remaining two with empty circle.

### UT-CHECKLIST-02 — Dismiss persists

**Preconditions**: checklist rendered; user clicks dismiss button.
**Expected**: response reports `dismissed == true`; caller must persist via settings.

### UT-ACTIVITY-ROW-01 — Failed row has retry

**Preconditions**: `ActivityRow::new` with status `Failed`.
**Expected**: render includes a `Retry` small button; row border color = danger.

### UT-REQUEST-CARD-01 — Received vs Sent styling

**Preconditions**: two cards, `RequestCard::received` and `RequestCard::sent`.
**Expected**: received has amber left-border + Accept/Decline buttons. Sent has blue
left-border + Pending pill + Cancel request button.

### UT-CONTACT-ROW-01 — Clickable surface

**Preconditions**: row with handle and display name.
**Steps**: click the row body.
**Expected**: response `clicked == true` with the contact id carried in the response.

### UT-GATE-01 — No-social-profile gate card

**Preconditions**: gate card rendered with `@{handle}` placeholder.
**Expected**: interpolates the handle correctly; primary button = `Add a display name`.

### UT-HERO-01 — Identity hero, social profile set

**Preconditions**: identity with display name + handle + balance.
**Expected**: render emits the avatar with initials fallback when no image; `text_secondary`
for handle line; tabular numerals for balance.

### UT-HERO-02 — Identity hero, no social profile

**Preconditions**: same identity with `display_name=None`.
**Expected**: render emits type-glyph monogram instead of avatar; no display-name line.

## Integration tests (kittest, one per tab + onboarding)

### IT-ONBOARD-01 — onboarding empty state renders

**File**: `tests/kittest/identity_hub_onboarding.rs`.
**Preconditions**: `AppContext` with zero loaded identities on Testnet.
**Steps**: mount `IdentityHubScreen::new(&app_context)`, run one frame.
**Expected**:
- Heading text `Welcome to Identities.` is present.
- Both primary buttons present: `Create my first identity`, `I already have an identity
  — load it`.
- Developer Mode footer absent (Alex persona, developer mode off).

### IT-HOME-01 — Home tab renders with one identity

**File**: `tests/kittest/identity_hub_home.rs`.
**Preconditions**: `AppContext` with one loaded User identity (fake in-memory test doubles).
**Steps**: mount `IdentityHubScreen`, run one frame.
**Expected**:
- Breadcrumb `Identities` link + wallet pill + identity pill present.
- Tab bar with exactly four tab labels: Home, Contacts, Activity, Settings.
- Home tab selected by default.
- Quick-actions row has three buttons: Send, Receive, Add contact.
- Secondary-actions row has three ghost buttons: Add funds, Send to wallet, Send to
  another identity.

### IT-CONTACTS-01 — Contacts tab gated when no social profile

**File**: `tests/kittest/identity_hub_contacts.rs`.
**Preconditions**: `AppContext` with one identity, no social profile.
**Steps**: mount hub, switch to Contacts tab, run one frame.
**Expected**:
- Heading `Set up a social profile first.` present.
- Primary button `Add a display name` present.
- No request cards or active contacts list rendered.

### IT-ACTIVITY-01 — Activity tab shell renders

**File**: `tests/kittest/identity_hub_activity.rs`.
**Preconditions**: one identity, `identity_hub_activity_feed` flag off.
**Steps**: mount hub, switch to Activity tab.
**Expected**:
- Filter chips present: All, Payments, Funding.
- Gated message present: `Unified activity is coming soon.`

### IT-SETTINGS-01 — Settings tab renders sections

**File**: `tests/kittest/identity_hub_settings.rs`.
**Preconditions**: one identity, social profile set.
**Steps**: mount hub, switch to Settings tab.
**Expected**:
- Section heading `Social profile` present.
- Section heading `Username` present.
- Section heading `Aliases` present.
- Advanced expander present.

### IT-NAV-01 — new left-nav entry is present and coexists with old entries

**File**: `tests/kittest/identity_hub_nav.rs`.
**Preconditions**: default app mount.
**Steps**: inspect left panel nav buttons.
**Expected**:
- Old `Identities` nav entry present (legacy).
- Old `Dashpay` nav entry present (legacy).
- New `Identities` hub entry present (distinct variant).

## Traceability

| Requirement | Tests |
|---|---|
| FR-1 (new nav entry, coexists) | IT-NAV-01 |
| FR-2 (dispatch by identity count) | IT-ONBOARD-01, IT-HOME-01 |
| FR-3 (breadcrumb switcher) | UT-BPILL-*, UT-BSWITCH-01, UT-IDPILL-*, IT-HOME-01 |
| FR-4 (four tabs) | IT-HOME-01, IT-CONTACTS-01, IT-ACTIVITY-01, IT-SETTINGS-01, UT-TABS-01 |
| FR-5 (onboarding CTAs + dev footer) | IT-ONBOARD-01 |
| FR-6 (picker grid) | UT-PICKER-*, (manual visual — separate kittest added once picker lands) |
| FR-7 (i18n-ready strings) | enforced by review, not test |
| FR-8 (tooltips wired) | verified by `InfoPopup` / `ResponseExt` unit tests |
| FR-9 (nav tooltip) | verified by reading `add_left_panel` diff |
| NFR-1 (no backend mods) | verified by reviewing git diff against `src/backend_task/` |
| NFR-2 (feature gating) | per-flag cfg checks in module headers |
| NFR-5 (tests present) | this document |
| NFR-6 (lint clean) | CI |
| NFR-7 (progressive disclosure) | UT-TABS-01 persona matrix (future), visual review |

---

Revision: 1
Authored: 2026-04-23
