# Identity UI

Hub-local widgets live here, alongside the tab modules that consume them and the
standalone flow screens the hub links out to.
Promote a widget to `src/ui/components/` only if a second, non-hub consumer appears.

## Tab modules

| Module | Responsibility |
|---|---|
| `hub_screen.rs` | Root screen, tab selection, landing resolution |
| `home.rs` | Home tab (hero · quick actions · onboarding · recent activity · advanced) |
| `contacts.rs` | Contacts tab (gated / populated shell) |
| `activity.rs` | Activity tab (filter chips + legacy-payments link) |
| `settings.rs` | Settings tab (social profile · username · advanced) |
| `onboarding.rs` | Onboarding empty state |
| `picker.rs` | Identity picker grid (≥ 2 identities) |
| `landing.rs` | `HubLanding` state enum |
| `tabs.rs` | `IdentityHubTab` enum |

## Flow screens

Pushed onto the screen stack from the hub (or the left nav), not tabs.

| Module | Responsibility |
|---|---|
| `add_existing_identity_screen.rs` | Search a wallet for identities already registered from it |
| `add_new_identity_screen/` | Register a new identity (funded by address, deposit, asset lock, or balance) |
| `top_up_identity_screen/` | Top up an identity's credit balance, same four funding sources |
| `register_dpns_name_screen.rs` | Claim a DPNS username for an identity |
| `transfer_screen.rs` | Transfer credits to another identity |
| `withdraw_screen.rs` | Withdraw credits to a Core payment address |
| `keys/` | Key list, key detail, and add-key screens |
| `funding_common.rs` | Funding-method state machine and widgets, shared by the flows above and by `ui/wallets/` |

## Widgets

| Module | Used by | Notes |
|---|---|---|
| `identity_hub_tab_bar.rs` | `hub_screen` | Horizontal tab strip (Home · Contacts · Activity · Settings) |
| `identity_hero_card.rs` | `home` | Gradient hero card (Dash-Blue → Platform-Purple) |
| `onboarding_checklist.rs` | `home` | Pick username · set display name · add first contact |
| `identity_pill.rs` | `identity_picker_card` | Thin wrapper over `components::breadcrumb_pill::BreadcrumbPill` with the identity label priority rule (nickname → DPNS → shortened ID) |
| `identity_picker_card.rs` | `picker` | Per-identity card in the picker grid |
| `identity_picker_add_card.rs` | `picker` | Trailing "Add a new identity" CTA in the picker grid |
| `social_profile_gate_card.rs` | `contacts` | Gate shown when the active identity has no DashPay profile |
| `request_card.rs` | `contacts` (future) | Received / sent contact-request row |

## Button dispatcher pattern

Home, Contacts, and Activity each expose a pure `*_button_kind()` function
that maps a `*Button` enum variant to its `*ButtonKind` result (screen to
open, tab to switch, outcome to emit). The renderer calls through the
dispatcher at every click site; unit tests iterate every enum variant and
assert no button is dead. See the regression suite at the bottom of each
tab module.

The motivation is in `docs/ai-design/2026-04-23-identity-hub-impl/04-dev-plan.md`
under T8 — the original Wave 2 landed with every quick action returning
`AppAction::None` because the hub screen discarded the tab's action value.
The dispatcher pattern catches that class of bug in CI before the user
ever sees it.
