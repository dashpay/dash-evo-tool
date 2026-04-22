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

## Dialog Components

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `ConfirmationDialog` | `confirmation_dialog.rs` | `ConfirmationStatus` | Modal confirm/cancel with danger mode |
| `SelectionDialog` | `selection_dialog.rs` | `SelectionStatus` | Modal with ComboBox selection |
| `InfoPopup` | `info_popup.rs` | N/A | Info popup with optional markdown |
| `WalletUnlockPopup` | `wallet_unlock_popup.rs` | `WalletUnlockResult` | Password-based wallet unlock |

## Feedback Components

| Component | File | Description |
|-----------|------|-------------|
| `MessageBanner` | `message_banner.rs` | Global error/warning/success/info banners. `set_global()`, `with_details()`, auto-dismiss. Extensions: `OptionBannerExt`, `OptionBannerShowExt`, `ResultBannerExt` |

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
| `ScreenWithWalletUnlock` | `wallet_unlock.rs` | Trait for screens needing wallet unlock |

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
