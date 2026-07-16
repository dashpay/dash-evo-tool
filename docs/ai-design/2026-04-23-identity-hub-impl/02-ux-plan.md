# Phase 1b — UX Plan

Derived from Phase 1a (Requirements) and the authoritative design spec at
`docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md`. Nothing here overrides
the design spec; this document is the implementation-side interpretation.

## Journey 1 — First-time user (Alex, no identity yet)

1. App opens, `Identities` nav clicked.
2. `IdentityHubScreen` loads, inspects `AppContext.qualified_identities()` for the active
   network: empty → onboarding state.
3. Breadcrumb renders `(no wallet yet) › (no identity yet)` when no wallet is loaded; if a
   wallet is loaded but no identity, renders `[wallet pill] › (no identity yet)`.
4. Central island shows avatar silhouette + heading + two buttons + developer-mode footer.
5. `Create my first identity` routes to the existing `AddNewIdentityScreen`. `I already
   have an identity — load it` routes to the existing `AddExistingIdentityScreen`.
6. Returning to the hub after successful identity creation lands on Identity Home
   (journey 2).

**Interaction patterns**: two vertically-stacked primary buttons, info tooltip on the nav
entry (tt-1), developer-mode chip footer hidden for Alex / Priya, shown for Jordan.

## Journey 2 — Identity Home (Alex, one identity, social profile set)

1. `IdentityHubScreen` inspects identity count: 1 → Identity Home directly.
2. Active identity is the only one; breadcrumb identity pill is interactive but dropdown
   has just that one identity + `+ Add another identity` footer.
3. Tab bar rendered at the top of the island: Home · Contacts · Activity · Settings.
4. Home layout zones render top-to-bottom: hero card · quick-actions row · secondary-
   actions row · onboarding checklist (if not dismissed) · recent activity preview ·
   advanced expander (collapsed for Alex).

**Interaction patterns**:
- Hero card avatar is a `StyledCard` with gradient background overriding the default
  surface.
- Quick-actions row uses three `StyledButton::primary` instances with equal width.
- Secondary-actions row uses three `StyledButton::ghost` instances.
- Onboarding checklist dismiss button uses tt-20.
- Recent activity preview: each row is a new `ActivityRow` component with compact 48px
  height; last row has footer link `See all activity` that switches to the Activity tab.

## Journey 3 — Multi-identity switcher (Priya, three identities)

1. Hub detects ≥ 2 identities → picker grid (§B.14).
2. User clicks a card → selects that identity in breadcrumb + navigates to Home tab.
3. On any tab, user clicks the identity pill → listbox dropdown of all identities in the
   active wallet + grouped section for imported-by-ID identities + footer
   `+ Add another identity`.
4. Selecting a different identity updates breadcrumb + refreshes the current tab.

**Interaction patterns**:
- Picker grid uses egui layout equivalent of CSS `repeat(auto-fill, minmax(260px, 1fr))`
  — computed dynamically from the available width.
- Identity card is a new `IdentityPickerCard` component. `role=button` / focusable.
- `Add a new identity` card uses the same dimensions with dashed-border styling.

## Journey 4 — Contacts (populated, social profile set)

1. User on Home → clicks Contacts tab.
2. `ContactsTab` queries existing `ContactsList` backend path (no new backend task).
3. Layout: tab header row (Contacts title + `+ Add by username` / `Scan QR` / `Show my QR`
   buttons), then three sections: Received requests (amber left-border), Active contacts
   (with search input + row list), Sent requests (blue left-border, muted).
4. Clicking an active contact row opens a right-side detail drawer (Frame 4 detail panel).

**Interaction patterns**:
- `RequestCard` component with `kind: Received | Sent` variant (drives color + buttons).
- `ContactRow` component — avatar + handle + last-payment hint + Send button + `•••`
  overflow.
- Search input uses existing `egui::TextEdit::singleline` with `search` placeholder copy
  from tooltip catalog.
- Detail drawer is a right-anchored egui `SidePanel::right` inside the central panel with
  `RADIUS_LG` rounded corners.

## Journey 5 — Contacts (gated, no social profile)

1. `ContactsTab` renders a gated-state card when `current_identity.social_profile()` is
   `None`.
2. Heading + body copy from §B.4.1 verbatim.
3. Primary `Add a display name` button switches to Settings tab + scrolls to social profile
   section. Secondary `Why?` toggles an inline explanation.

## Journey 6 — Activity

1. Filter chips: All (default on) · Payments · Funding · Platform. Alex sees Payments and
   Funding; Platform collapsed under `More`. Priya / Jordan see all three.
2. Timeline renders up to N rows paginated; each row expandable.
3. Failed row (red left-border) shows `Retry` small button with tt-44.
4. Empty state: `No activity yet. Your payments, additions, and identity changes will
   appear here.`

**MVP constraint**: the unified aggregator over DashPay payments + funding + platform ops
does not exist today. The tab ships with filter chips and a gated-state body saying
"Unified activity is coming soon. For now, view activity on the existing DashPay Payments
screen." Cargo feature flag: `identity-hub-activity-feed`, off by default.

## Journey 7 — Settings (Priya, multi-wallet)

1. Settings tab: two-column at ≥ 1024px width (left: social profile, right: username +
   aliases). Single-column fallback.
2. Advanced expander below (open by default for Priya / Jordan, collapsed for Alex).
3. Danger zone at the bottom of Advanced — confirmation dialog on `Unload this identity
   from this device`.

## Accessibility notes

- Every icon-only button has `WidgetInfo::selected(WidgetType::Button, enabled, selected,
  accessible_name)` set (pattern already used in `left_panel.rs`).
- Breadcrumb nav uses `aria-current="page"` on the active identity pill — in egui, this
  maps to `WidgetInfo::selected(WidgetType::Link, ..., true, ...)`.
- Focus rings: rely on egui's built-in `visuals.widgets.active.bg_stroke`; for the picker
  card, use a 3px Dash-blue outline on `response.has_focus()`.
- Color contrast: all pill-on-gradient combinations reviewed against WCAG 2.2 AA using the
  tokens already in `theme.rs`.

## DX notes

- Every component lives in its own file under `src/ui/components/` and is re-exported from
  `src/ui/components/mod.rs`.
- Each component has a `new()` constructor with required args only. All optional
  configuration goes through builder methods.
- Each component exposes a `Response` struct implementing `ComponentResponse`. Consumers
  never touch component internals.
- Components render correctly in both light and dark mode — a single `dark_mode: bool`
  computed from `ctx.style().visuals.dark_mode` drives color token selection.

## Out-of-scope for this PR

- Shadow-alpha realignment in `theme.rs` (design-spec §E). Tracked as a separate follow-up
  — would affect every screen, not just the new hub.
- Real unified activity aggregator. Tab shell only.
- Auto-accept contact requests proof generation. UI toggle added, backend wiring deferred.
- Pick-a-username contest detection flow (§B.13). Hub routes to the existing
  `RegisterDpnsNameScreen` which already handles this.
- Contested name browse-alternatives suggestion. Deferred.

---

Revision: 1
Authored: 2026-04-23
