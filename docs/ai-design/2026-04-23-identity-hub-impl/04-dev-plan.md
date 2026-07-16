# Phase 1d — Development Plan

Derived from 01-requirements.md, 02-ux-plan.md, 03-test-case-spec.md.

## Architecture

### Layers

1. **`src/ui/identity/`** — new submodule. Root screen `IdentityHubScreen`, four tab
   submodules (`home.rs`, `contacts.rs`, `activity.rs`, `settings.rs`), onboarding
   (`onboarding.rs`), picker (`picker.rs`), and a local `mod.rs` with the
   `IdentityHubTab` enum + shared tab state.

2. **`src/ui/components/`** — new shared widgets (flat placement):
   - `breadcrumb_pill.rs` — label + icon + optional chevron; `subdued` / `interactive` /
     `placeholder` modes.
   - `breadcrumb_switcher.rs` — composes plain link + two `BreadcrumbPill`s into the
     `Identities › wallet › identity` row.
   - `identity_pill.rs` — thin wrapper over `BreadcrumbPill` with the label priority rule.
   - `identity_hub_tab_bar.rs` — horizontal tab bar specific to the hub (does NOT replace
     the existing `*_subscreen_chooser_panel.rs` family — those stay).
   - `identity_picker_card.rs` + `identity_picker_add_card.rs`.
   - `identity_hero_card.rs` — gradient hero with avatar + handle + balance.
   - `onboarding_checklist.rs` — three-step list with check marks + dismiss.
   - `activity_row.rs` — 48px compact row; has a `Failed` variant with Retry.
   - `request_card.rs` — Received / Sent variants.
   - `contact_row.rs` — avatar + handle + Send button + overflow.
   - `social_profile_gate_card.rs` — the no-profile gate card used on Contacts tab.

3. **`src/model/identity_hub.rs`** — new small module holding UI-layer value types:
   `IdentityHubTab` enum, `SocialProfileState`, `HubLanding`, persistence for
   `start_tab_on_hub` user preference (optional).

4. **`src/ui/RootScreenType`** — add `RootScreenIdentityHub` variant.
   `ScreenType::IdentityHub`. `Screen::IdentityHubScreen`. `ScreenType::create_screen`
   dispatch. All three existing enums extended.

5. **`src/app.rs`** — register the new screen in `AppState::new()` `main_screens` BTreeMap.

6. **`src/ui/mod.rs`** — extend `RootScreenType::from_int` / `to_int` mapping
   (next free integer). No schema change. `src/database/settings.rs` consumes the
   mapping but does not own it — only add tests there if persistence behaviour changes.

7. **`src/ui/components/left_panel.rs`** — add a third nav button `Identities · Hub` (new
   label distinct from the legacy `Identities`) using the `identity.png` icon for now.
   Gate: always visible (no `FeatureGate`).

### Tech choices

- No new crate dependencies. Everything built from existing `egui`, `egui_extras`,
  `dash-sdk`, and the project's theme module.
- Kittest: follow existing patterns in `tests/kittest/*`.
- Async: tabs dispatch existing backend tasks (e.g. `DashPayTask::LoadContacts`) — the hub
  does not introduce new backend variants in the default scope.

## Task breakdown

Batched for one-agent serial execution. Each task ends with a commit and `cargo build` +
`cargo clippy` + `cargo test --lib` green.

### T1 — Planning artifacts (DONE)

Phase 1 documents committed under `docs/ai-design/2026-04-23-identity-hub-impl/`.

### T2 — Feature flag + RootScreenType variant

- Add `identity-hub` feature to `Cargo.toml` (default-enabled so the hub is visible
  by default; can be disabled for quick compile).
- Add `identity-hub-activity-feed` feature (default off) — gates the unified activity
  aggregator (stub tab content when off).
- Extend `RootScreenType` with `RootScreenIdentityHub` (to_int / from_int mapping uses
  integer 27 — next free).
- Add `ScreenType::IdentityHub` variant + `PartialEq` + `create_screen` arm that returns
  a placeholder stub screen.
- Add `Screen::IdentityHubScreen(IdentityHubScreen)` variant.

Unit tests: `database::settings` round-trip test covering the new integer.

Deliverable: `cargo build --all-features` green.

Commit: `feat(identity-hub): add feature flag, RootScreenType variant, screen enum wiring`

### T3 — Scaffold `src/ui/identity/`

- `src/ui/identity/mod.rs` — module root, re-exports.
- `src/ui/identity/hub_screen.rs` — `IdentityHubScreen` struct implementing `ScreenLike`
  with an empty body rendered inside `island_central_panel`. Holds tab state.
- `src/ui/identity/tabs.rs` — `IdentityHubTab` enum (Home / Contacts / Activity / Settings).
- `src/ui/identity/landing.rs` — `HubLanding` state machine: `Onboarding | Home | Picker`
  computed from loaded-identity count.

Unit tests: state-machine transitions for HubLanding.

Commit: `feat(identity-hub): scaffold hub screen module and tab state`

### T4 — Breadcrumb switcher + pill components

- `src/ui/components/breadcrumb_pill.rs` — `BreadcrumbPill` + `BreadcrumbPillResponse`.
  Builder methods: `.with_icon(...)`, `.subdued(bool)`, `.interactive(bool)`, `.placeholder()`.
- `src/ui/components/identity_pill.rs` — `IdentityPill` with label priority
  (Local nickname → DPNS → shortened ID).
- `src/ui/components/breadcrumb_switcher.rs` — composes plain-text `Identities` link +
  wallet pill + identity pill. `BreadcrumbSwitcherResponse` reports which segment was
  activated.

Unit tests: UT-BPILL-01..03, UT-IDPILL-01..03, UT-BSWITCH-01.

Commit: `feat(identity-hub): add breadcrumb switcher and pill components`

### T5 — Tab bar component + onboarding

- `src/ui/components/identity_hub_tab_bar.rs` — horizontal bar, four tab buttons, selected
  state uses existing theme tokens.
- `src/ui/identity/onboarding.rs` — onboarding empty state UI.
- Wire `HubLanding::Onboarding` in `IdentityHubScreen`.

Unit tests: UT-TABS-01. Kittest: IT-ONBOARD-01.

Commit: `feat(identity-hub): add tab bar and onboarding empty state`

### T6 — Left-nav entry + AppState wiring

- Add `identity_hub.png` icon reference (reuse `identity.png` temporarily with a TODO to
  create a distinct people-silhouette asset).
- Extend `left_panel.rs` buttons array with the new entry.
- Extend `app.rs` `main_screens` construction.
- Verify legacy `Identities` and `Dashpay` entries still work.

Kittest: IT-NAV-01.

Commit: `feat(identity-hub): wire left-nav entry and AppState registration`

### T7 — Identity picker

- `src/ui/components/identity_picker_card.rs` + `identity_picker_add_card.rs`.
- `src/ui/identity/picker.rs` — grid rendering, click handling, "Add a new identity" routes
  to existing `AddNewIdentityScreen`.

Unit tests: UT-PICKER-01..03.

Commit: `feat(identity-hub): add identity picker grid`

### T8 — Identity hero + onboarding checklist + Home tab

- `src/ui/components/identity_hero_card.rs`.
- `src/ui/components/onboarding_checklist.rs`.
- `src/ui/identity/home.rs` — full Home tab with hero, quick actions, secondary actions,
  checklist, recent activity preview (stubbed to existing backend data), advanced expander.

Unit tests: UT-HERO-01..02, UT-CHECKLIST-01..02.
Kittest: IT-HOME-01.

Commit: `feat(identity-hub): add Home tab with hero and onboarding checklist`

### T9 — Contacts tab (gated + populated shells)

- `src/ui/components/social_profile_gate_card.rs`.
- `src/ui/components/request_card.rs`.
- `src/ui/components/contact_row.rs`.
- `src/ui/identity/contacts.rs` — gated and populated states. Populated state delegates
  to existing DashPay backend task for contacts list; no new backend work.

Unit tests: UT-GATE-01, UT-REQUEST-CARD-01, UT-CONTACT-ROW-01.
Kittest: IT-CONTACTS-01.

Commit: `feat(identity-hub): add Contacts tab with gated + populated states`

### T10 — Activity tab shell

- `src/ui/components/activity_row.rs`.
- `src/ui/identity/activity.rs` — filter chip row + gated empty state + Retry wiring.
  Guards the full timeline behind `identity_hub_activity_feed` feature.

Unit tests: UT-ACTIVITY-ROW-01.
Kittest: IT-ACTIVITY-01.

Commit: `feat(identity-hub): add Activity tab shell with filter chips`

### T11 — Settings tab

- `src/ui/identity/settings.rs` — two-column layout with social profile (left), username +
  aliases (right), advanced expander, danger zone confirmation.
- Delegates edit / save actions to existing backend tasks (`DashPayTask::UpdateProfile`,
  `IdentityTask::AddAlias`, etc.) — additive nothing.

Kittest: IT-SETTINGS-01.

Commit: `feat(identity-hub): add Settings tab`

### T12 — docs + PR prep

- Update `src/ui/components/README.md` with new components.
- Update `docs/user-stories.md` with US-IDH-001..006.
- Write PR body.

Commit: `docs(identity-hub): update components reference and user stories`

### T13 — Polish + QA pass

- `cargo +nightly fmt --all`.
- `cargo clippy --all-features --all-targets -- -D warnings` — fix all.
- `cargo test --all-features --workspace` — fix regressions.
- Self-review against test-case spec.

Commit: `chore(identity-hub): formatting and clippy cleanup` (as needed).

### T14 — Push + PR + ci-dance

- Push feature branch.
- Open draft PR against `v1.0-dev`.
- Run `claudius:ci-dance` until green or the retry budget is exhausted.

## Task → Requirement traceability

| Task | Requirements satisfied |
|------|------------------------|
| T2   | FR-1, NFR-1 (additive only) |
| T3   | FR-2, FR-4 |
| T4   | FR-3 |
| T5   | FR-4, FR-5 |
| T6   | FR-1 (coexistence) |
| T7   | FR-6 |
| T8   | FR-4 Home, FR-7 (i18n strings) |
| T9   | FR-4 Contacts, FR-8 (gated-state tooltip) |
| T10  | FR-4 Activity, NFR-2 (feature flag) |
| T11  | FR-4 Settings |
| T12  | AC-7 |
| T13  | AC-2, AC-3, AC-4, NFR-6 |
| T14  | N/A — delivery |

## Feature-flag inventory

| Flag | Kind | Default | Gates |
|------|------|---------|-------|
| `identity-hub` | Cargo feature | on | left-nav `Identity Hub` entry + hub registration in `main_screens` |
| `identity-hub-activity-feed` | Cargo feature | off | unified activity aggregator rendering |
| `developer_mode` | runtime (`AppContext::is_developer_mode`) | off | dev footer, throwaway identity, multi-identity create |

Runtime gates use existing `FeatureGate` predicates where applicable; introduce new
predicate variants only if a gate is reused across two or more components.

## Risk register

| # | Risk | Mitigation |
|---|------|------------|
| R1 | Enum exhaustiveness — adding `RootScreenType` variant breaks every `match` with `_ => ...` OR requires updating hand-enumerated arms | Search `rg "RootScreenType::"` before committing; add explicit arms where the compiler complains; use `#[allow(clippy::enum_variant_names)]` already in place |
| R2 | `Screen` enum is already large; `large_enum_variant` lint | Box the new variant body if needed |
| R3 | Unit tests that need an `AppContext` | Use the existing test fixtures in `tests/kittest/*` — reuse mount pattern from `identities_screen.rs` test |
| R4 | kittest lifetime — existing test mount has specific setup | Copy-adapt `tests/kittest/identities_screen.rs` |
| R5 | Feature flag accidentally disables something users rely on | Default-on for the new-feature flag; default-off for the activity aggregator (explicitly experimental) |

## Model selection

- Single-agent execution, serial. No spawning.

---

Revision: 1
Authored: 2026-04-23
