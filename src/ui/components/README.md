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
| `AddressInput` | `address_input.rs` | `ValidatedAddress` | Unified address with autocomplete, type detection (Core/Platform/Shielded/Identity), DPNS resolution. GitHub-style tag search (`type:core\|platform\|…`, `wallet:name`; unrecognized tokens are free text). Rows render `[wallet pill] address (name) [type pill] balance`; when no explicit hint is set the placeholder is a live `type:…|… wallet:…|…` legend |
| `PasswordInput` | `password_input.rs` | N/A (security) | Masked input with hold-to-reveal, zeroizes on drop. NOT ComponentResponse |
| `IdentitySelector` | `identity_selector.rs` | N/A (Widget) | ComboBox dropdown for identity selection |

## Breadcrumb Components

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `BreadcrumbPill` | `breadcrumb_pill.rs` | `String` | Label + optional icon + chevron. Three modes: Interactive / Subdued / Placeholder. Reusable anywhere a breadcrumb pill is needed (Identities hub breadcrumb, future wallet breadcrumbs). |
| `global_nav_switcher::render()` | `global_nav_switcher.rs` | `GlobalNavEffect` | Page-aware three-segment switcher (`segment-1 › 💼 wallet › 👤 identity/object`) via `top_panel::add_top_panel_with_global_nav[_capturing]` / the Hub's own `breadcrumb_switcher` shim. Live on Identities, DashPay, DPNS, Wallets, Identity Hub, and Masternodes — interactive on Hub, Wallets and Masternodes (Masternodes also carries a page-scoped node pill, and its wallet segment is a read-only "not in a wallet" indicator since a node is wallet-less), subdued (read-only) on Identities, DashPay and DPNS; remaining root screens (Contracts, Tokens, Tools, Network Chooser, Withdraws, …) still render the plain breadcrumb (FR-GLOBAL-NAV rollout in progress). Composes per page from a `PageNavSpec` (`ui/state/global_nav.rs`) and reuses `BreadcrumbPill`/`IdentityPill`. |

## Display Components

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `Avatar` | `avatar.rs` | N/A (display) | DashPay contact/profile avatar from a URL. Renders image / spinner / `👤` fallback, decoding + uploading the texture on the UI thread. Backed by `ui/state/avatar_cache.rs` (`AvatarCache`), which fetches off-frame via `DashPayTask::FetchAvatar`. `show(ui, &mut AvatarCache)` returns `AvatarResponse { fetch, clicked }`; the caller dispatches `fetch`. Builders: `corner_radius`, `clickable(tooltip)`. |

## Placement Rule

`src/ui/components/` holds **reusable** components only — widgets that plausibly
have a second consumer outside their originating screen (Wallets, Tokens,
Contracts, Tools, Settings, etc.).

Identity Hub-specific widgets (`IdentityHubTabBar`, `IdentityHeroCard`,
`OnboardingChecklist`, `IdentityPickerCard`, `IdentityPickerAddCard`,
`SocialProfileGateCard`, `RequestCard`, `IdentityPill`) live in
`src/ui/identity/` alongside the tab modules that consume them. If one of
those widgets gains a second consumer outside the hub, promote it into this
directory.

## Dialog Components

| Component | File | DomainType | Description |
|-----------|------|------------|-------------|
| `ConfirmationDialog` | `confirmation_dialog.rs` | `ConfirmationStatus` | Modal confirm/cancel with danger mode |
| `SelectionDialog` | `selection_dialog.rs` | `SelectionStatus` | Modal with ComboBox selection |
| `InfoPopup` | `info_popup.rs` | N/A | Info popup with optional markdown |
| `WalletUnlockPopup` | `wallet_unlock_popup.rs` | `WalletUnlockResult` | Password-based wallet unlock (renders via shared `passphrase_modal`) |
| `passphrase_modal()` | `passphrase_modal.rs` | `PassphraseModalOutcome` | Shared passphrase-entry chrome: overlay, centered window, `PasswordInput`, error line, optional extra body (e.g. remember checkbox). Cancellable prompts map Cancel/Esc/X/click-outside to Cancel; blocking prompts expose only their configured actions. |
| `EguiSecretPromptHost` | `secret_prompt_host.rs` | N/A (`SecretPrompt`) | egui host for just-in-time secret prompts; enqueues requests for `AppState` to render and answers via one-shot. `ActivePrompt` owns the live modal |

## Feedback Components

| Component | File | Description |
|-----------|------|-------------|
| `MessageBanner` | `message_banner.rs` | Global error/warning/success/info banners. `set_global()`, `with_details()`, auto-dismiss. Extensions: `OptionBannerExt`, `OptionBannerShowExt`, `ResultBannerExt` |
| `ProgressOverlay` | `progress_overlay.rs` | Full-screen blocking progress overlay: spinner, optional step counter, generic buttons (`with_action(label, action_id)`, mirrors `MessageBanner::with_action`), 120s watchdog. A hard block is never keyboard-activatable except via `with_keyboard_escape(action_id)`, which designates one focus-pinned button as a keyboard-reachable escape (Enter/Space) for unbounded blocks (the SPV-sync block). Global path: `set_global()` (raise, mirrors `MessageBanner::set_global`) / `render_global()`, claims input each frame. Companions: `OverlayConfig`, `OverlayHandle`, `OptionOverlayExt` (`raise` — the banner's `replace`, renamed to dodge inherent `Option::replace`), `ProgressOverlayResponse` |

## Styled Components (`styled.rs`)

| Component | Description |
|-----------|-------------|
| `StyledButton` | Primary button following Dash design guidelines |
| `StyledCard` | Card with padding and border |
| `StyledCheckbox` | Themed checkbox |
| `GradientButton` | Animated gradient button |

## Layout

| Function/Module | File | Description |
|-----------------|------|-------------|
| `island_central_panel()` | `styled.rs` | Responsive central panel, renders global MessageBanners |
| `add_location_view()` | `top_panel.rs` | Breadcrumb navigation + connection status |
| `add_top_panel_with_global_nav()` | `top_panel.rs` | Top panel wired to `global_nav_switcher::render()` for a page that only *reads* the selection — subdued pills (`subdued_everyday_spec`); used by Identities, DashPay and DPNS. Every root screen not listed here still calls the plain `add_top_panel()` |
| `add_top_panel_with_global_nav_capturing()` | `top_panel.rs` | Same, but also returns the (already-applied) `GlobalNavEffect` so a page that *consumes* a selection can mirror it into its own state (two-way binding). Used by Wallets (`wallet_only_spec` → mirrors `SwitchWallet` into the page's cached wallet) and Masternodes (`masternodes_page_nav_spec` → mirrors `SelectPageObject` by opening that node) |
| `add_left_panel()` | `left_panel.rs` | Main icon navigation sidebar |
| `load_icon()` / `load_svg_icon()` | `icons.rs` | Load & cache embedded raster/SVG icons from `icons/` |
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
