# UI Components Reference

Concise catalog of all reusable UI components. Consult before creating new UI elements.

## Core Traits (`component_trait.rs`)

| Trait | Methods |
|-------|---------|
| `Component` | `show(&mut self, ui) -> InnerResponse<Response>`, `current_value() -> Option<DomainType>` |
| `ComponentResponse` | `has_changed()`, `is_valid()`, `changed_value()`, `error_message()`, `update(&mut Option<DomainType>)` |

## Input Components

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `AmountInput` | `amount_input.rs` | `Amount` | Decimal amount with validation, min/max, Max button, unit name |
| `AddressInput` | `address_input.rs` | `ValidatedAddress` | Unified address with autocomplete, type detection (Core/Platform/Shielded/Identity), DPNS resolution |
| `PasswordInput` | `password_input.rs` | N/A (security) | Masked input with hold-to-reveal, zeroizes on drop. NOT ComponentResponse |
| `IdentitySelector` | `identity_selector.rs` | N/A (Widget) | ComboBox dropdown for identity selection |

## Breadcrumb Components (Identities hub)

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `BreadcrumbPill` | `breadcrumb_pill.rs` | `String` | Label + optional icon + chevron. Three modes: Interactive / Subdued / Placeholder. Drives the wallet + identity switcher in the Identities hub breadcrumb. |
| `IdentityPill` | `identity_pill.rs` | `String` | Thin wrapper over `BreadcrumbPill` applying the priority rule Local nickname → DPNS handle → shortened Identity ID. `display_label` / `shorten_id` are pure functions reusable anywhere an identity label is rendered. |

## Identity Hub Components

All of these live flat in `src/ui/components/` and are consumed by the unified
Identities hub under `src/ui/identity/`. See the design spec at
`docs/ai-design/2026-04-22-identity-dashpay-redesign/` for visual context.

| Component | File | Description |
|-----------|------|-------------|
| `IdentityHubTabBar` | `identity_hub_tab_bar.rs` | Horizontal tab strip over the four hub tabs (Home · Contacts · Activity · Settings). Response struct surfaces the clicked tab; caller owns the selected-tab state so deep links can override it. |
| `IdentityHeroCard` | `identity_hero_card.rs` | Large hero card on the Home tab: identity avatar / pill, balance preview, quick actions. |
| `OnboardingChecklist` | `onboarding_checklist.rs` | Home-tab step list with dismiss + skip semantics. Response surfaces which step was acted on so the hub applies the right `HomeOutcome`. |
| `IdentityPickerCard` | `identity_picker_card.rs` | Picker-grid card representing one identity (avatar, name, id). Clicking selects that identity. |
| `IdentityPickerAddCard` | `identity_picker_add_card.rs` | Trailing picker-grid card that adds a new identity (onboarding-style CTA). |
| `SocialProfileGateCard` | `social_profile_gate_card.rs` | Contacts-tab gate shown when the active identity has no DashPay profile. Primary CTA emits `AppAction::SwitchIdentityHubTab(Settings)` so the user lands where profile editing happens. |
| `RequestCard` | `request_card.rs` | Contact-request row with received / sent variants. Response exposes `accepted` / `declined` / `cancelled` flags — callers dispatch the matching `DashPayTask` variant. |
| `ContactRow` | `contact_row.rs` | Active-contact row used in the contacts list. Consistent avatar/initials with `RequestCard`. |
| `ActivityRow` | `activity_row.rs` | Activity-tab row with icon, title, subtitle, timestamp, and optional amount. Used in the filterable timeline and the Home-tab recent-activity preview. |

## Dialog Components

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `ConfirmationDialog` | `confirmation_dialog.rs` | `ConfirmationStatus` | Modal confirm/cancel with danger mode |
| `SelectionDialog` | `selection_dialog.rs` | `SelectionStatus` | Modal with ComboBox selection |
| `InfoPopup` | `info_popup.rs` | N/A | Info popup with optional markdown |
| `WalletUnlockPopup` | `wallet_unlock_popup.rs` | `WalletUnlockResult` | Password-based wallet unlock (renders via shared `passphrase_modal`) |
| `passphrase_modal()` | `passphrase_modal.rs` | `PassphraseModalOutcome` | Shared passphrase-entry chrome: overlay, centered window, `PasswordInput`, error line, optional extra body (e.g. remember checkbox), Cancel/Esc/X/click-outside → Cancel |
| `EguiSecretPromptHost` | `secret_prompt_host.rs` | N/A (`SecretPrompt`) | egui host for just-in-time secret prompts; enqueues requests for `AppState` to render and answers via one-shot. `ActivePrompt` owns the live modal |

## Feedback Components

| Component | File | Description |
|-----------|------|-------------|
| `MessageBanner` | `message_banner.rs` | Global error/warning/success/info banners. `set_global()`, `with_details()`, auto-dismiss. Extensions: `OptionBannerExt`, `OptionBannerShowExt`, `ResultBannerExt` |
| `ProgressOverlay` | `progress_overlay.rs` | Full-screen blocking progress overlay: spinner, optional step counter, generic buttons (`with_action(label, action_id)`, mirrors `MessageBanner::with_action`), 120s watchdog. A hard block is never keyboard-activatable except via `with_keyboard_escape(action_id)`, which designates one focus-pinned button as a keyboard-reachable escape (Enter/Space) for unbounded blocks (the SPV-sync block). Global path: `set_global()` (raise, mirrors `MessageBanner::set_global`) / `render_global()`, claims input each frame. Companions: `OverlayConfig`, `OverlayHandle`, `OptionOverlayExt` (`raise` — the banner's `replace`, renamed to dodge inherent `Option::replace`), `ProgressOverlayResponse` |

## Styled Components (`styled.rs`)

| Component | Description |
|-----------|-------------|
| `StyledButton` | Primary/Secondary/Danger/Ghost variants, Small/Medium/Large |
| `StyledCard` | Card with padding and border |
| `StyledCheckbox` | Themed checkbox |
| `GradientButton` | Animated gradient with optional glow |
| `GlassCard` | Glass-morphism card |
| `HeroSection` | Large gradient header |
| `AnimatedIcon` | Configurable animated icon |
| `AnimatedGradientCard` | Card with animated gradient border |

## Layout

| Function/Module | File | Description |
|-----------------|------|-------------|
| `island_central_panel()` | `styled.rs` | Responsive central panel, renders global MessageBanners |
| `add_location_view()` | `top_panel.rs` | Breadcrumb navigation + connection status |
| `add_left_panel()` | `left_panel.rs` | Main icon navigation sidebar |
| `add_left_panel()` | `left_wallet_panel.rs` | Wallet/identity sidebar |
| Subscreen panels | `*_subscreen_chooser_panel.rs` | Tab navigation for DPNS, DashPay, Tokens, Tools |
| `ContractChooserState` | `contract_chooser_panel.rs` | Hierarchical contract tree view |

## Utility

| Component | File | Description |
|-----------|------|-------------|
| `U256EntropyGrid` | `entropy_grid.rs` | 32x8 interactive grid for 256-bit entropy generation |

> Non-widget UI state (per-screen view-models and async fetch-state caches that render no egui) lives in `src/ui/state/`, not here. For example `TrackedAssetLockCache` (`src/ui/state/tracked_asset_lock_cache.rs`) caches each wallet's tracked asset locks. See the module placement policy in `CLAUDE.md`.

## Usage Pattern

```rust
// Lazy init in screen struct
amount_input: Option<AmountInput>,
amount: Option<Amount>,

// In show():
let widget = self.amount_input.get_or_insert_with(|| {
    AmountInput::new(Amount::new_dash(0.0))
        .with_label("Amount (DASH):")
        .with_hint_text("Enter amount")
        .with_max_button(true)
});
let response = widget.show(ui);
response.inner.update(&mut self.amount);
```
