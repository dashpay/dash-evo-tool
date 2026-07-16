# Phase 1a — Requirements

Source: `docs/ai-design/2026-04-22-identity-dashpay-redesign/` (README, design-spec, wireframe).

## Scope

Implement a brand-new `Identities` UI section inside Dash Evo Tool 2, coexisting with (not
replacing) the existing `Identities` and `Dashpay` sections. The new section is a four-tab
hub: Home · Contacts · Activity · Settings — with an onboarding empty state and an identity
picker for multi-identity wallets.

## Personas

Three personas drive visibility rules (reused from `docs/personas/`):

- **Alex Torres** — Everyday user. One wallet, one identity. Sees basic actions only.
- **Priya Nakamura** — Power user. Many wallets, many identities. Sees advanced expanders.
- **Jordan Kim** — Platform developer. Everything Priya sees plus Developer Mode tools.

Personas are runtime-resolvable via `AppContext::is_developer_mode()` for Jordan and via
the loaded-wallet / loaded-identity counts for Alex / Priya distinction. No explicit persona
dropdown in the product; the UI adapts.

## Functional requirements

FR-1. New left-nav entry `Identities` (plural, kept label) routes to the new hub. Old
`Identities` and `Dashpay` left-nav entries remain visible during the coexistence period.

FR-2. Hub entry point dispatches by loaded-identity count on the active network:
- 0 identities → onboarding empty state (§B.1).
- 1 identity → Identity Home for that identity (§B.2 / §B.3).
- ≥ 2 identities → Identity Picker grid (§B.14).

FR-3. Breadcrumb switcher is always visible in the topbar of every tab:
`Identities › [wallet pill] › [identity pill]`. Pills are live switchers where more than
one option exists, subdued (non-interactive) where there is only one. Placeholders render
when a segment is empty.

FR-4. Four tabs under the hub, rendered as a single tab-bar component:
- Home: identity hero, quick actions (Send · Receive · Add contact), secondary actions
  (Add funds · Send to wallet · Send to another identity), optional onboarding checklist,
  recent activity preview (top 5 rows), advanced expander with raw IDs.
- Contacts: populated state (received requests · active contacts · sent requests) OR
  gated-state card when identity has no social profile.
- Activity: unified timeline with filter chips (All · Payments · Funding · Platform),
  expandable rows, retry for failed rows.
- Settings: social profile (left column) · username + aliases (right column) · advanced
  expander · danger zone.

FR-5. Onboarding empty state renders two primary CTAs — `Create my first identity` and
`I already have an identity — load it` — plus a Developer Mode footer with
`Create multiple test identities` and `Load identity by ID`.

FR-6. Identity picker renders a CSS-grid of identity cards plus an `Add a new identity`
card. Cards show avatar or type-glyph monogram, display name / DPNS / shortened ID,
identity-type badge, balance, fiat equivalent where available.

FR-7. All user-facing strings are complete sentences with named placeholders per the i18n
rule in CLAUDE.md. No concatenation, no sentence-fragment joining.

FR-8. Every tooltip from design-spec §D is wired with the correct `ResponseExt` variant
(`info_tooltip` · `clickable_tooltip` · `disabled_tooltip`) and persona visibility.

FR-9. Nav-entry info tooltip: "Your identities on Dash Platform. Manage usernames,
balances, keys, and — if you set up a social profile — DashPay contacts and payments."

## Non-functional requirements

NFR-1. **Additive backend only**: No modifications to existing `BackendTask`, `WalletTask`,
`IdentityTask`, `DashPayTask`, `AppContext` methods, or database schemas. New additive
variants / methods are permitted only if unavoidable; default posture is zero backend
changes. (Locked decision #5.)

NFR-2. **Feature-gate unsupported capabilities**: For any UI affordance whose backend
dispatch does not exist today, hide the affordance behind a compile-time `cfg(feature = ...)`
flag or runtime predicate that defaults off. Document each gated capability in the PR
description.

NFR-3. **Theme reuse**: Every color, spacing, radius, shadow, and typography value pulled
from `src/ui/theme.rs`. No new token constants. The shadow-alpha realignment (design-spec §E)
is a separate concern tracked in the dev plan but is **out of scope** for this PR.

NFR-4. **Component reuse**: Every new widget must be placed flat inside
`src/ui/components/` alongside existing shared components, following
`docs/COMPONENT_DESIGN_PATTERN.md` (private fields, builder methods, `ComponentResponse`
trait, light+dark mode).

NFR-5. **Tests**: Each new component has unit tests covering validation, response struct
methods, and state transitions. Each tab plus the onboarding state has one `kittest`
integration test.

NFR-6. **Formatting and linting**: Zero clippy warnings with `--all-features --all-targets
-- -D warnings`. `cargo +nightly fmt --all` produces a clean diff.

NFR-7. **Progressive disclosure**: Advanced sections collapse by default. Developer Mode
tools only render when `app_context.developer_mode` is true. Persona-specific content uses
the `FeatureGate` predicate pattern established in
`src/model/feature_gate.rs` where applicable (memcan memory `a3628faa`).

NFR-8. **Graceful degradation**: When an identity has no social profile, the UI does not
crash — it falls back to monograms, shortened IDs, and the gated-state cards per §B.3 and
§B.4.1.

## Data needs & processing rules

- Loaded identities per network: `AppContext.qualified_identities()` (existing accessor).
- Active wallet + loaded wallets: `AppContext.wallets` (existing `RwLock<BTreeMap>`).
- Social profile presence: existing DashPay Profile query path — if none, render the
  no-profile state.
- Identity-type classification: existing `QualifiedIdentity.identity_type` enum (User /
  Masternode / Evonode) drives the type-glyph monogram and badge pill choice.
- DPNS primary username: existing accessor; render `No username yet` fallback when empty.
- Local nickname: `QualifiedIdentity.alias` (renamed in UI copy to `Local nickname` — see
  design-spec §G7). Stored field unchanged; label only changes.
- Recent activity: reuse existing activity-aggregation path if present; otherwise, render
  empty-state copy and gate the Activity tab behind a `recent_activity_feed` feature flag
  until a backend aggregator is added.

## User stories (to add to `docs/user-stories.md`)

US-IDH-001 `[Implemented]` **Alex — first-time setup**
  As Alex, I want to open the Identities section on a fresh device and be offered a clear
  single-step path to create my first identity, so I can start using Dash Platform without
  understanding what an identity is first.

US-IDH-002 `[Implemented]` **Alex — identity home at a glance**
  As Alex, when I have one identity, opening Identities shows me my balance, username, a
  big Send button, and my recent activity, without any jargon.

US-IDH-003 `[Implemented]` **Priya — switch between many identities**
  As Priya, with multiple wallets and identities, I can switch between them from the
  breadcrumb pill on any tab in under two clicks.

US-IDH-004 `[Implemented]` **Alex — opt in to DashPay**
  As Alex, setting up a social profile to unlock DashPay contacts is clearly optional and
  I can keep using payments and usernames without doing it.

US-IDH-005 `[Implemented]` **Jordan — bulk test identities**
  As Jordan in Developer Mode, I have a single entry point to create many test identities
  without leaving the Identities section.

US-IDH-006 `[Gap-follow-up]` **Unified activity timeline**
  As any persona, my payments, funding movements, and platform actions all live in one
  Activity tab with filters, not in separate screens. Full aggregation depends on a
  backend follow-up; the tab shell ships with filter chips and a gated-state message
  pointing to the existing identity-specific history views.

## Acceptance criteria

- AC-1. Running the app shows three `Identities`-group entries in the left nav (old
  `Identities`, old `Dashpay`, new hub). Clicking the new one opens the appropriate landing
  state per FR-2 without regressing either old screen.
- AC-2. `cargo build` is green on default features and on `--all-features`.
- AC-3. `cargo clippy --all-features --all-targets -- -D warnings` reports zero warnings.
- AC-4. `cargo test --all-features --workspace` is green.
- AC-5. At least one `kittest` test per tab plus onboarding asserts that the expected
  labels, buttons, and placeholders render.
- AC-6. No file under `src/ui/identities/`, `src/ui/dashpay/`, or `src/backend_task/` is
  modified by this PR (except to add additive backend variants if strictly required, which
  must be documented in the PR body).
- AC-7. `docs/user-stories.md` updated with stories US-IDH-001 .. US-IDH-006.

---

Revision: 1
Authored: 2026-04-23
Author: Claudius the Magnificent (single-agent execution)
