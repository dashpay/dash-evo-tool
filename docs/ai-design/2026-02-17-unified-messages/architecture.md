# MessageBanner Component -- Technical Architecture

## 0. Current State Analysis

### Overview

The codebase has **~50 screens** (structs implementing `ScreenLike`) with no unified message rendering component. There are **4 distinct rendering patterns**, **8+ different error colors**, and **two competing `MessageType` enums**. Each screen independently manages its own message fields and rendering.

### Rendering Style Taxonomy

| Pattern | Description | Used By |
|---------|-------------|---------|
| **A - Framed Banner** | `Frame` with `fill(color.gamma_multiply(0.1))`, border stroke, RichText label, "Dismiss" button | ~8 screens |
| **B - Timed Badge** | `(String, MessageType, DateTime<Utc>)` tuple, rendered with countdown timer (auto-dismiss ~10s) | ~8 screens |
| **C - Status Enum** | Error stored in a status enum variant (e.g., `TransferCreditsStatus::ErrorMessage`), rendered inline via `colored_label` | ~8 screens |
| **D - Bare colored_label** | Simple `ui.colored_label(Color32::X, message)` inline, no frame, no dismiss | ~20+ screens |

### Error Color Inconsistency

| Color Value | Usage Count | Screens |
|---|---|---|
| `Color32::from_rgb(255, 100, 100)` | ~8 screens | AddNewIdentityScreen, TopUpIdentityScreen, WalletsBalancesScreen, WalletSendScreen, PlatformInfoScreen, AddressBalanceScreen, KeyInfoScreen, ImportMnemonicScreen |
| `Color32::DARK_RED` | ~10 screens | WithdrawalScreen, TransferScreen, RegisterDpnsNameScreen, IdentitiesScreen, DPNSScreen, ContactsList, ContactRequests, AddContactScreen, SendPaymentScreen, SetTokenPriceScreen |
| `Color32::RED` | ~5 screens | FreezeTokensScreen, UnfreezeTokensScreen, PauseTokensScreen, ResumeTokensScreen, ClaimTokensScreen |
| `DashColors::error_color(dark_mode)` | ~7 screens | MintTokensScreen, BurnTokensScreen, DestroyFrozenFundsScreen, ProfileSearchScreen, ContactProfileViewerScreen, QRCodeGeneratorScreen, QRScannerScreen |
| `DashColors::ERROR` (`from_rgb(235,87,87)`) | ~1 screen | ContactInfoEditorScreen |
| `Color32::from_rgb(220, 80, 80)` | ~1 screen | WalletUnlockPopup |

### Per-Screen Catalog

#### Identities

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `IdentitiesScreen` | `identities/identities_screen.rs` | `backend_message: Option<(String, MessageType, DateTime<Utc>)>` | Pattern B (timed, 10s auto-dismiss with countdown) | Custom: sets `backend_message` | Custom: `RefreshedIdentity` → Success | Shows spinner during refresh; Error=`DARK_RED`, Success=`DARK_GREEN` |
| `AddNewIdentityScreen` | `identities/add_new_identity_screen/mod.rs` | `error_message: Option<String>` | Pattern A (framed banner + Dismiss) | Custom: sets `error_message` | Default | Prefix: `"Error registering identity: {}"`. Color: `from_rgb(255,100,100)` |
| `AddExistingIdentityScreen` | `identities/add_existing_identity_screen.rs` | `error_message`, `backend_message`, `success_message` | Pattern A for errors | Custom: routes to 3 fields | Custom: `LoadedIdentity`, `Message` | **3 separate message fields**; `backend_message` = progress indicator |
| `TopUpIdentityScreen` | `identities/top_up_identity_screen/mod.rs` | `error_message: Option<String>` | Pattern A (framed banner + Dismiss) | Custom: sets `error_message`; resets step state on Error | Default | Also resets `WalletFundedScreenStep` on error |
| `WithdrawalScreen` | `identities/withdraw_screen.rs` | `error_message`, `withdraw_from_identity_status` enum | Pattern C (status enum) | Custom: sets status enum | Custom: `WithdrewFromIdentity` → Complete | Status enum is primary error carrier |
| `TransferScreen` | `identities/transfer_screen.rs` | `error_message`, `transfer_credits_status` enum | Pattern C (status enum) | Custom: sets **both** enum AND `error_message` | Custom: `TransferredCredits` → Complete | **Redundant dual-field storage** |
| `RegisterDpnsNameScreen` | `identities/register_dpns_name_screen.rs` | `error_message: Option<String>` | Pattern D (bare colored_label) | Custom: sets `error_message` | Default | Color: `DARK_RED` |

#### Identity Keys

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `KeysScreen` | `identities/keys/keys_screen.rs` | None | None | Default no-op | Default | Minimal screen, no messages |
| `KeyInfoScreen` | `identities/keys/key_info_screen.rs` | `error_message`, `sign_error_message` | Pattern A-like for `error_message`; Pattern D for `sign_error_message` | Default no-op | Default | **2 error fields** for different operations; no `display_message` override |
| `AddKeyScreen` | `identities/keys/add_key_screen.rs` | `error_message`, `add_key_status` enum | Pattern C (status enum) | Custom: sets `add_key_status` | Custom: `AddedKeyToIdentity` → Complete | |

#### DPNS

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `DPNSScreen` | `dpns/dpns_contested_names_screen.rs` | `message: Option<(String, MessageType, DateTime<Utc>)>`, `bulk_schedule_message` | Pattern B (timed) + bulk block | Custom: sets `message` | Custom: extensive vote handling | **2 message slots**; bulk message uses emoji icons (❌/🎉) |

#### Wallets

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `WalletsBalancesScreen` | `wallets/wallets_screen/mod.rs` | `message: Option<(String, MessageType, DateTime<Utc>)>`, `sk_error_message` | Pattern A (framed banner + Dismiss) | Custom: routes errors to fund dialog if processing | Custom: wallet ops | **Dialog interception**: fund-platform dialog captures errors |
| `AddNewWalletScreen` | `wallets/add_new_wallet_screen.rs` | `error: Option<String>` | **Modal Window** (`egui::Window::new("Error")`) | Default no-op | Default | Only screen using modal error popup |
| `ImportMnemonicScreen` | `wallets/import_mnemonic_screen.rs` | `error: Option<String>` | Pattern D (bare colored_label) | Default no-op | Default | No `display_message` override |
| `WalletSendScreen` | `wallets/send_screen.rs` | `send_status: SendStatus`, `error_message` | Pattern A (framed banner) + full success screen | Custom: sets `send_status` enum | Custom: `WalletPayment`, etc. | Status enum drives display; success shows full-page 🎉 view |
| `SingleKeyWalletSendScreen` | `wallets/single_key_send_screen.rs` | `message: Option<(String, MessageType, DateTime<Utc>)>`, `error_message` | Pattern B (timed banner) | Custom: sets `message` | Custom: `WalletPayment` | 2 fields |
| `CreateAssetLockScreen` | `wallets/create_asset_lock_screen.rs` | `message: Option<(String, MessageType, DateTime<Utc>)>`, `error_message` | Pattern B (timed banner) | Custom: sets `message` | Custom: asset lock types | Helper methods `set_error_message()`/`error_message()` |
| `AssetLockDetailScreen` | `wallets/asset_lock_detail_screen.rs` | `message: Option<(String, MessageType, DateTime<Utc>)>`, `error_message` | Pattern B (timed banner) | Custom: sets `message` | Custom: asset lock retrieval | Helper methods |

#### Contracts / Documents

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `DocumentQueryScreen` | `contracts_documents/contracts_documents_screen.rs` | `error_message: Option<(String, MessageType, DateTime<Utc>)>` | Pattern D (colored_label + timestamp expiry) | Custom: **only handles** `"Error fetching documents"` | Custom: Documents/PageDocuments | **Filters messages by text content** |
| `AddContractsScreen` | `contracts_documents/add_contracts_screen.rs` | `add_contracts_status` enum | Pattern C (status enum) | Custom: sets status enum | Custom: `FetchedContracts` | |
| `RegisterDataContractScreen` | `contracts_documents/register_contract_screen.rs` | `error_message`, `broadcast_status` enum | Pattern C + Pattern A | Custom: routes by text pattern | Custom: `RegisteredContract` | **Text-pattern branch** for proof error special case |
| `UpdateDataContractScreen` | `contracts_documents/update_contract_screen.rs` | `error_message`, `broadcast_status` enum | Pattern C + inline | Custom: same as Register | Custom: `UpdatedContract` | Same pattern as Register |
| `DocumentActionScreen` | `contracts_documents/document_action_screen.rs` | `backend_message: Option<String>` | Not rendered via banner | Custom: sets `backend_message`; **ignores message_type** | Custom: Broadcasted/Deleted | `_message_type` — type parameter unused |
| `GroupActionsScreen` | `contracts_documents/group_actions_screen.rs` | `fetch_group_actions_status` enum | Pattern C (status enum) | Custom: sets status enum | Custom: `ActiveGroupActions` | |

#### Tokens

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `TokensScreen` | `tokens/tokens_screen/mod.rs` | `backend_message: Option<(String, MessageType, DateTime<Utc>)>`, `token_creator_error_message` | Pattern B (timed banner) | Custom: smart routing (creator subpanel vs main) | Custom: extensive | **2 fields**; smart dispatch based on state |
| `MintTokensScreen` | `tokens/mint_tokens_screen.rs` | `error_message: Option<String>` | Pattern D (colored_label) | Custom: Error → `error_message` | Default | Color: `DashColors::error_color(dark)` |
| `BurnTokensScreen` | `tokens/burn_tokens_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `DashColors::error_color(dark)` |
| `TransferTokensScreen` | `tokens/transfer_tokens_screen.rs` | `transfer_tokens_status` enum | Pattern C (status enum) | Custom: sets status enum | Custom: `TransferredTokens` | No `error_message` field |
| `FreezeTokensScreen` | `tokens/freeze_tokens_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | **Color: `Color32::RED`** (not DARK_RED) |
| `UnfreezeTokensScreen` | `tokens/unfreeze_tokens_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `Color32::RED` |
| `PauseTokensScreen` | `tokens/pause_tokens_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `Color32::RED` |
| `ResumeTokensScreen` | `tokens/resume_tokens_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `Color32::RED` |
| `DestroyFrozenFundsScreen` | `tokens/destroy_frozen_funds_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `DashColors::error_color(dark)` |
| `ClaimTokensScreen` | `tokens/claim_tokens_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `Color32::RED` |
| `ViewTokenClaimsScreen` | `tokens/view_token_claims_screen.rs` | `message: Option<(String, MessageType, DateTime<Utc>)>` | Pattern D | Custom: all types → `message` | Custom: calls display_message if empty | |
| `AddTokenByIdScreen` | `tokens/add_token_by_id_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Success = refresh; Error = `error_message` | (inline) | `display_message` handles Success as task result |
| `PurchaseTokenScreen` | `tokens/direct_token_purchase_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | |
| `SetTokenPriceScreen` | `tokens/set_token_price_screen.rs` | `error_message: Option<String>` | Pattern D | Custom: Error → `error_message` | Default | Color: `DARK_RED`; also has amber validation labels |
| `UpdateTokenConfigScreen` | `tokens/update_token_config.rs` | `backend_message: Option<(String, MessageType, DateTime<Utc>)>`, `error_message` (unused) | Pattern B (timed banner) | Custom: Error/Info → `backend_message` | Custom: `UpdatedTokenConfig` | **`error_message` field is explicitly commented `// unused`** |

#### Tools

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `PlatformInfoScreen` | `tools/platform_info_screen.rs` | `error_message: Option<String>` | Pattern A (framed banner + Dismiss) | Custom: Error → `error_message` | Custom: PlatformInfo results | **Error blocks result display** (early return) |
| `GroveSTARKScreen` | `tools/grovestark_screen.rs` | `gen_error_message`, `verify_error_message` | Pattern D (with Dismiss in framed box) | Custom: routes to gen/verify based on mode | Custom: GeneratedProof, VerifiedProof | **2 mode-specific error fields** |
| `MasternodeListDiffScreen` | `tools/masternode_list_diff_screen.rs` | `ui_state.message: Option<(String, MessageType)>`, `ui_state.error` | Pattern A (framed banner) | Custom: Error → `ui_state.error`; Info → **silently discarded** | Custom: CoreItem, extensive | **Info messages explicitly dropped** |
| `ProofLogScreen` | `tools/proof_log_screen.rs` | None | None | Explicit no-op | Default | **Fully ignores all messages** |
| `ProofVisualizerScreen` | `tools/proof_visualizer_screen.rs` | None | None | Explicit no-op | Default | **Fully ignores all messages** |
| `TransitionVisualizerScreen` | `tools/transition_visualizer_screen.rs` | `broadcast_status` enum, `contract_fetch_message: Option<(String, Instant)>` | Pattern C (status enum) | Custom: routes by type | Custom: FetchedContract | `contract_fetch_message` is time-based |
| `AddressBalanceScreen` | `tools/address_balance_screen.rs` | `error_message: Option<String>` | Pattern A (framed banner + Dismiss) | Custom: Error → `error_message` | Custom: AddressBalance | Color: `from_rgb(255,100,100)` |

#### DashPay

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `DashPayScreen` | `dashpay/dashpay_screen.rs` | None (delegates) | Delegates to active sub-screen | Custom: dispatches to sub-screens | Default | Pure delegator |
| `ContactsList` | `dashpay/contacts_list.rs` | `message: Option<(String, MessageType)>` | Pattern D | Custom: sets `message` | Custom | Color: `DARK_RED`/`DARK_GREEN`/`LIGHT_BLUE` |
| `ContactRequests` | `dashpay/contact_requests.rs` | `message: Option<(String, MessageType)>` | Pattern D (bold for errors) | Custom: sets `message` | Custom | |
| `ContactDetailsScreen` | `dashpay/contact_details.rs` | `message: Option<(String, MessageType)>` | Pattern D | **Public + trait impl** (trait delegates to pub fn) | Custom | |
| `ContactInfoEditorScreen` | `dashpay/contact_info_editor.rs` | `message: Option<(String, MessageType)>` | Pattern D | **Public + trait impl** | Custom | Uses `DashColors::SUCCESS/ERROR/INFO` constants |
| `ProfileSearchScreen` | `dashpay/profile_search.rs` | `message: Option<(String, MessageType)>` | Pattern D | **Public + trait impl** | Custom | Color: `DashColors::error_color(dark)` |
| `ContactProfileViewerScreen` | `dashpay/contact_profile_viewer.rs` | `message: Option<(String, MessageType)>` | Pattern D | **Public + trait impl** | Custom | |
| `AddContactScreen` | `dashpay/add_contact_screen.rs` | `message: Option<(String, MessageType)>` | Pattern D | Custom: wraps `AddContactError` | Custom | |
| `QRCodeGeneratorScreen` | `dashpay/qr_code_generator.rs` | `message: Option<(String, MessageType)>` | Pattern D | **Public + trait impl** | Default | |
| `QRScannerScreen` | `dashpay/qr_scanner.rs` | `message: Option<(String, MessageType)>` | Pattern D | **Public + trait impl** | Custom | |
| `SendPaymentScreen` | `dashpay/send_payment.rs` | `message: Option<(String, MessageType)>` | Pattern D | Custom: sets `sending=false` + `message` | Custom: `DashPayPaymentSent` | |

#### Network / Other

| Screen | File | Message Fields | Rendering | `display_message` | `display_task_result` | Quirks |
|---|---|---|---|---|---|---|
| `NetworkChooserScreen` | `network_chooser_screen.rs` | `custom_dash_qt_error_message`, `spv_clear_message`, `db_clear_message` | Inline specialized fields | Default no-op | Default | **3 specialized local message fields**; no standard message pipeline |

### Key Issues Summary

1. **No unified rendering component** — 4 distinct visual patterns with no shared helper
2. **8+ different error colors** — ranges from `Color32::RED` to `Color32::DARK_RED` to `from_rgb(255,100,100)` to `DashColors::error_color(dark)`
3. **Screens silently ignoring messages** — `KeysScreen`, `ProofLogScreen`, `ProofVisualizerScreen` (no-op), `MasternodeListDiffScreen` (drops Info), `DocumentActionScreen` (ignores type)
4. **Redundant dual-field storage** — `TransferScreen` stores errors in both status enum and `error_message`
5. **Dead code** — `UpdateTokenConfigScreen.error_message` explicitly marked unused; `theme.rs::MessageType` is dead code
6. **6 screens with public+trait dual display_message** — DashPay sub-screens have both a `pub fn` and `ScreenLike` impl
7. **Inconsistent auto-dismiss** — 8 screens use `DateTime<Utc>` timestamps for timed messages; the rest persist until user action
8. **Success messages often silently lost** — Token action screens (Mint, Burn, Freeze, etc.) use default `display_task_result` which calls `display_message("Success", Success)`, but their `display_message` only handles Error

---

## 1. MessageType Enum Unification

### Current State

Two competing `MessageType` enums exist:

1. **`src/ui/mod.rs:830`** -- Active, 3 variants (Success, Info, Error), used by `ScreenLike` trait
2. **`src/ui/theme.rs:638`** -- Dead code (`#[allow(dead_code)]`), 4 variants (Success, Error, Warning, Info), has `color()` and `background_color()` methods

### Decision

**Extend** the active enum in `mod.rs` by adding a `Warning` variant. **Consolidate** the color methods from the dead-code `theme.rs::MessageType` into `DashColors` as dark-mode-aware methods (the dead enum only had light-mode colors). Then **delete** the dead-code enum in `theme.rs` (lines 636-664).

### Unified Enum (in `src/ui/mod.rs`)

```rust
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MessageType {
    Success,
    Info,
    Warning,
    Error,
}
```

This is the only change to the existing `MessageType`. The variant order follows severity (lowest to highest). Adding `Warning` is backward-compatible -- existing match arms that use `Success`, `Info`, `Error` will get a compiler error only in exhaustive matches, which is desirable to ensure all sites handle the new variant.

### New DashColors Methods (in `src/ui/theme.rs`)

Add `info_color(dark_mode)` and `message_background_color(message_type, dark_mode)` to `DashColors`, consolidating all message-type-aware colors in one place:

```rust
impl DashColors {
    /// Info severity color -- complements existing error_color/success_color/warning_color.
    pub fn info_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(100, 180, 255) // Lighter blue for dark mode
        } else {
            Self::DEEP_BLUE // Dark blue for light mode
        }
    }

    /// Returns the tinted background color for a message severity level.
    /// Uses low alpha (8% light, 12% dark) for subtle tinting.
    pub fn message_background_color(message_type: MessageType, dark_mode: bool) -> Color32 {
        let alpha = if dark_mode { 30 } else { 20 };
        match message_type {
            MessageType::Error => {
                let c = if dark_mode { (255, 100, 100) } else { (235, 87, 87) };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
            MessageType::Warning => {
                let c = if dark_mode { (255, 200, 100) } else { (241, 196, 15) };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
            MessageType::Success => {
                let c = if dark_mode { (80, 200, 120) } else { (39, 174, 96) };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
            MessageType::Info => {
                let c = if dark_mode { (100, 180, 255) } else { (52, 152, 219) };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
        }
    }

    /// Returns the foreground (text/border) color for a message severity level.
    /// Delegates to existing per-severity color methods.
    pub fn message_color(message_type: MessageType, dark_mode: bool) -> Color32 {
        match message_type {
            MessageType::Error => Self::error_color(dark_mode),
            MessageType::Warning => Self::warning_color(dark_mode),
            MessageType::Success => Self::success_color(dark_mode),
            MessageType::Info => Self::info_color(dark_mode),
        }
    }
}
```

This approach:
- Keeps all color definitions in `DashColors` (single source of truth for the design system)
- Reuses existing `error_color`/`success_color`/`warning_color` methods for foreground colors
- Fills the gap: `info_color(dark_mode)` was missing
- Replaces the dead `theme.rs::MessageType` color methods with dark-mode-aware equivalents

---

## 2. MessageBanner Component

### Alignment with Component Design Pattern

`MessageBanner` follows the conventions established in `doc/COMPONENT_DESIGN_PATTERN.md`:

| Convention | How MessageBanner Applies It |
|---|---|
| **Private fields only** | `state: Option<MessageState>` is private |
| **`new()` constructor** | `MessageBanner::new()` creates an empty banner |
| **Builder methods** | `with_auto_dismiss_duration()` for configurable timeout |
| **`show()` method** | `show(&mut self, ui: &mut Ui)` renders the banner |
| **Self-contained** | Handles rendering, dismiss, auto-dismiss, and color resolution internally |
| **Colors via design system** | Uses `DashColors::message_color()`, `DashColors::message_background_color()`, and `DashColors::text_secondary()` — no hardcoded colors |

**Why MessageBanner does NOT implement the `Component` trait:**

The `Component` trait (in `component_trait.rs`) is designed for **input** components that produce domain values — `AmountInput` produces `Amount`, `ConfirmationDialog` produces `ConfirmationStatus`. Each has `DomainType`, `ComponentResponse` with `has_changed()`/`changed_value()`/`update()`, and `current_value()`.

`MessageBanner` is a **display** component that **consumes** data (receives messages) rather than **producing** domain values. It has no meaningful `DomainType` to bind, no user-produced value to return, and no `update()` target. Forcing the `Component` trait would require artificial type parameters (`DomainType = ()`) that add complexity without value.

This is consistent with how other display-only widgets in the codebase work — e.g., `StyledButton`, `GradientHeading`, `InfoPopup` all have `show()` without implementing `Component`.

### File Location

`src/ui/components/message_banner.rs`

Register in `src/ui/components/mod.rs`:
```rust
pub mod message_banner;
pub use message_banner::MessageBanner;
```

### Struct Definition

```rust
use crate::ui::MessageType;
use crate::ui::theme::DashColors;
use std::time::{Duration, Instant};

const DEFAULT_AUTO_DISMISS_DURATION: Duration = Duration::from_secs(5);

/// A self-contained banner widget for displaying screen-level messages.
///
/// Each screen owns one instance. Call `show()` every frame inside the
/// screen's `ui()` method, before the `ScrollArea`. The banner renders
/// nothing when no message is set.
///
/// Follows the component conventions from `doc/COMPONENT_DESIGN_PATTERN.md`:
/// private fields, `new()` constructor, builder methods, `show()` rendering.
pub struct MessageBanner {
    /// Current message state: text, severity, and the instant it was set.
    /// `None` means no message is displayed.
    state: Option<MessageState>,
    /// Duration before Success/Info messages auto-dismiss.
    auto_dismiss_duration: Duration,
}

struct MessageState {
    text: String,
    message_type: MessageType,
    created_at: Instant,
}
```

The struct is intentionally minimal. `state` is `Option` so the banner occupies zero layout space when empty. `Instant` is used for auto-dismiss timing because it is monotonic and does not depend on wall clock.

### Public API

```rust
impl MessageBanner {
    /// Creates an empty banner (no message displayed).
    pub fn new() -> Self {
        Self {
            state: None,
            auto_dismiss_duration: DEFAULT_AUTO_DISMISS_DURATION,
        }
    }

    /// Builder: set custom auto-dismiss duration for Success/Info messages.
    pub fn with_auto_dismiss_duration(mut self, duration: Duration) -> Self {
        self.auto_dismiss_duration = duration;
        self
    }

    /// Sets or replaces the current message. Resets the auto-dismiss timer.
    /// An empty string is treated as a clear operation.
    pub fn set_message(&mut self, text: &str, message_type: MessageType) {
        if text.is_empty() {
            self.state = None;
            return;
        }
        self.state = Some(MessageState {
            text: text.to_string(),
            message_type,
            created_at: Instant::now(),
        });
    }

    /// Clears the current message immediately.
    pub fn clear(&mut self) {
        self.state = None;
    }

    /// Returns whether a message is currently displayed.
    pub fn has_message(&self) -> bool {
        self.state.is_some()
    }

    /// Renders the banner into the given `Ui`.
    /// Call this every frame before the ScrollArea.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Implementation below
    }
}

impl Default for MessageBanner {
    fn default() -> Self {
        Self::new()
    }
}
```

### Rendering Implementation (`show`)

All colors are resolved through `DashColors` methods — no hardcoded `Color32` values in the component.

```rust
pub fn show(&mut self, ui: &mut egui::Ui) {
    // 1. Check if there is a message to display
    let Some(state) = &self.state else { return };

    // 2. Auto-dismiss check for Success and Info
    let auto_dismiss = matches!(state.message_type, MessageType::Success | MessageType::Info);
    if auto_dismiss && state.created_at.elapsed() >= self.auto_dismiss_duration {
        self.state = None;
        return;
    }

    // 3. Resolve colors via DashColors (single source of truth)
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    let fg_color = DashColors::message_color(state.message_type, dark_mode);
    let bg_color = DashColors::message_background_color(state.message_type, dark_mode);
    let secondary_color = DashColors::text_secondary(dark_mode);

    // 4. Compute remaining seconds for countdown (only for auto-dismiss types)
    let remaining_secs = if auto_dismiss {
        let elapsed = state.created_at.elapsed();
        self.auto_dismiss_duration
            .checked_sub(elapsed)
            .map(|d| d.as_secs() + 1) // +1 so we show "1s" until it actually expires
    } else {
        None
    };

    // 5. Render using DashColors and theme constants
    let icon = icon_for_type(state.message_type);
    let text = state.text.clone(); // clone to release borrow on self
    let mut dismissed = false;

    egui::Frame::new()
        .fill(bg_color)
        .inner_margin(egui::Margin::symmetric(
            Spacing::SM_I8 + Spacing::XXS as i8,  // 10px
            Spacing::SM_I8,                         // 8px
        ))
        .corner_radius(Shape::RADIUS_SM as f32)     // 6px
        .stroke(egui::Stroke::new(Shape::BORDER_WIDTH, fg_color))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Icon
                ui.label(egui::RichText::new(icon).color(fg_color).strong());
                ui.add_space(Spacing::XS); // 4px

                // Message text (wrapping allowed via Label)
                ui.label(egui::RichText::new(&text).color(fg_color));

                // Flexible space
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Dismiss button (rightmost)
                    if ui.small_button("x").clicked() {
                        dismissed = true;
                    }

                    // Countdown (before dismiss button in RTL layout)
                    if let Some(secs) = remaining_secs {
                        ui.label(
                            egui::RichText::new(format!("({}s)", secs))
                                .font(Typography::body_small())
                                .color(secondary_color),
                        );
                    }
                });
            });
        });
    ui.add_space(Spacing::SM as f32); // 8px below banner

    if dismissed {
        self.state = None;
    }
}
```

### Color Resolution — All Through DashColors

The banner uses **zero hardcoded colors**. All color resolution goes through `DashColors`:

| Purpose | Method | Source |
|---|---|---|
| Text & border color | `DashColors::message_color(type, dark_mode)` | Delegates to `error_color()` / `success_color()` / `warning_color()` / `info_color()` |
| Background tint | `DashColors::message_background_color(type, dark_mode)` | Semantic backgrounds at 8% (light) / 12% (dark) alpha |
| Countdown text | `DashColors::text_secondary(dark_mode)` | Standard secondary text color |
| Spacing values | `Spacing::XS`, `Spacing::SM`, etc. | Theme constants |
| Corner radius | `Shape::RADIUS_SM` (6px) | Theme constants |
| Border width | `Shape::BORDER_WIDTH` (1px) | Theme constants |

This ensures that if the design system colors are updated globally, the banner automatically reflects the changes.

### Icon Selection

```rust
fn icon_for_type(message_type: MessageType) -> &'static str {
    match message_type {
        MessageType::Error => "\u{26A0}",   // ⚠ warning sign (visually distinct via color)
        MessageType::Warning => "\u{26A0}", // ⚠ same glyph, differentiated by color
        MessageType::Success => "\u{2713}", // ✓ check mark
        MessageType::Info => "\u{2139}",    // ℹ info
    }
}
```

Note: If the egui font does not render these Unicode characters, fall back to ASCII: `"!"`, `"!"`, `"v"`, `"i"`. The implementer should verify glyph availability at development time.

---

## 3. ScreenLike Trait Changes

### Current Signature (unchanged)

```rust
pub trait ScreenLike {
    fn refresh(&mut self) {}
    fn refresh_on_arrival(&mut self) { self.refresh() }
    fn ui(&mut self, ctx: &Context) -> AppAction;
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}
    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {
        self.display_message("Success", MessageType::Success)
    }
    fn pop_on_success(&mut self) {}
}
```

**No signature change is needed.** The `display_message` method signature already accepts `&str` and `MessageType`. Adding `Warning` to the `MessageType` enum is the only change, and it flows through the existing signature.

### Default Implementation Consideration

It would be tempting to add a default `display_message` implementation that delegates to a `banner` field. However, `ScreenLike` is a trait and has no access to struct fields, so no default implementation is possible without an additional accessor method.

**Option considered and rejected:** Adding a `fn banner(&mut self) -> &mut MessageBanner` method to ScreenLike. This would save boilerplate but forces every screen to have a field named in a specific way, and changes the trait contract for all 45+ implementors simultaneously. The cost of the two-line `display_message` implementation per screen is low.

**Recommendation:** Each screen implements `display_message` as a two-line method:

```rust
fn display_message(&mut self, message: &str, message_type: MessageType) {
    self.banner.set_message(message, message_type);
}
```

Some screens perform additional logic in `display_message` (e.g., `TopUpIdentityScreen` resets its step state on error). Those screens keep their custom logic and add the `banner.set_message()` call.

---

## 4. Migration Plan

### Phase 1: Create Component + Unify MessageType + Extend DashColors

**Scope:** 3 files changed, 1 file created

1. **Update `src/ui/mod.rs`** -- Add `Warning` variant to `MessageType` enum (line 830).
2. **Update `src/ui/theme.rs`** -- Add `info_color(dark_mode)`, `message_color(message_type, dark_mode)`, and `message_background_color(message_type, dark_mode)` to `DashColors`. Delete the dead-code `MessageType` enum and its `impl` block (lines 636-664).
3. **Create `src/ui/components/message_banner.rs`** -- Full `MessageBanner` implementation as described in Section 2, using only `DashColors` for colors and theme constants for spacing/shape.
4. **Update `src/ui/components/mod.rs`** -- Add `pub mod message_banner; pub use message_banner::MessageBanner;`
5. **Fix compile errors** -- Any exhaustive `match` on `MessageType` will need a `Warning` arm. These are likely limited to `app.rs` (the `TaskResult` routing) and any screen that matches on message type in `display_message`. Add `MessageType::Warning => { /* same as Error for now */ }` to each.

**Validation:** `cargo build` succeeds. No behavioral change yet.

### Phase 2: Migrate Screens Incrementally

Each screen migration follows the same mechanical pattern. Screens can be migrated one at a time in separate commits, enabling incremental review.

**Per-screen changes:**

1. Add `use crate::ui::components::MessageBanner;` (if not already via glob import)
2. Replace `error_message: Option<String>` (and similar fields) with `banner: MessageBanner`
3. In `new()` / constructor: replace `error_message: None` with `banner: MessageBanner::new()`
4. In `ui()`: replace the inline `Frame` error rendering block with `self.banner.show(ui);`
5. In `display_message()`: replace `self.error_message = Some(...)` with `self.banner.set_message(message, message_type);`
6. Remove any manual dismiss logic (the banner handles it)
7. If the screen sets `self.error_message = Some(...)` inline during `ui()`, replace with `self.banner.set_message(...)`
8. If the screen clears the error (e.g., `self.error_message = None`), replace with `self.banner.clear()`

**Estimated effort per screen:** ~5-15 lines changed per screen file. Mechanical, low risk.

**Migration order suggestion:**
1. Start with a simple screen (e.g., `transfer_screen.rs`) as a proof-of-concept
2. Then migrate screens with the exact duplicate pattern (the `Frame` + dismiss button block identical to `top_up_identity_screen`)
3. Then migrate screens with custom `display_message` logic
4. Finally migrate screens that use status enums with `ErrorMessage(String)` variants

### Phase 3: Remove Dead Code

1. Delete the `#[allow(dead_code)]` annotations from `theme.rs` if the dead `MessageType` was removed in Phase 1
2. Remove any helper functions that were only used for the old error rendering pattern (if any exist)
3. Run `cargo clippy` to find any remaining unused code related to the old pattern

---

## 5. What Stays Unchanged

| Concern | Status | Rationale |
|---------|--------|-----------|
| **Status enums** (Pattern 2, e.g., `TransferCreditsStatus::ErrorMessage(String)`) | Keep as-is | These track screen workflow state. The screen can use both a status enum for flow control and `MessageBanner` for display. When entering the `ErrorMessage` state, the screen calls `self.banner.set_message(...)`. |
| **`show_success_screen()` helper** (Pattern 6, `src/ui/helpers.rs`) | Keep as-is | This renders a full-page success view with detailed info, not a banner. Different purpose entirely. |
| **Inline validation** (Pattern 8, e.g., `AmountInput` error messages) | Keep as-is | These are field-level validation hints rendered next to the input. They are not screen-level messages. |
| **`ConfirmationDialog`** | Keep as-is | Modal dialog for confirming actions. Orthogonal to message banners. |
| **BackendTask / TaskResult / AppState** | Keep as-is | The task routing in `app.rs` calls `display_message()` on the visible screen. This works identically with `MessageBanner` -- the screen's `display_message` impl just delegates to `banner.set_message()`. |
| **`AppAction` enum** | Keep as-is | No new action variants needed. |
| **`BackendTaskSuccessResult`** | Keep as-is | No changes to result types. |
| **`component_trait.rs` (`Component` / `ComponentResponse`)** | Not implemented by `MessageBanner` | `Component` trait is for input widgets that produce domain values (`AmountInput` → `Amount`). `MessageBanner` is a display widget that consumes data. It follows the pattern's conventions (private fields, builder, `show()`, DashColors) without the trait. Same approach as `StyledButton`, `GradientHeading`, `InfoPopup`. |

---

## 6. Code Examples: Before/After Migration

### Example: `TopUpIdentityScreen`

#### Before (current code)

**Struct definition** (`src/ui/identities/top_up_identity_screen/mod.rs:44-66`):
```rust
pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    // ... other fields ...
    error_message: Option<String>,
    // ...
}
```

**Constructor:**
```rust
impl TopUpIdentityScreen {
    pub fn new(qualified_identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            // ...
            error_message: None,
            // ...
        }
    }
}
```

**Error rendering in `ui()`** (lines 531-552):
```rust
// Display error message at the top, outside of scroll area
if let Some(error_message) = self.error_message.clone() {
    let message_color = egui::Color32::from_rgb(255, 100, 100);

    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(message_color.gamma_multiply(0.1))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .stroke(egui::Stroke::new(1.0, message_color))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&error_message).color(message_color));
                    ui.add_space(10.0);
                    if ui.small_button("Dismiss").clicked() {
                        self.error_message = None;
                    }
                });
            });
    });
    ui.add_space(10.0);
}
```

**`display_message` impl** (lines 430-435):
```rust
fn display_message(&mut self, message: &str, message_type: MessageType) {
    if message_type == MessageType::Error {
        self.error_message = Some(format!("Error topping up identity: {}", message));
        // Reset step so UI is not stuck on waiting messages
        let mut step = self.step.write().unwrap();
        if *step == WalletFundedScreenStep::WaitingForPlatformAcceptance
            || *step == WalletFundedScreenStep::WaitingForAssetLock
        {
            *step = WalletFundedScreenStep::ChooseFundingMethod;
        }
    }
}
```

#### After (migrated)

**Struct definition:**
```rust
use crate::ui::components::MessageBanner;

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    // ... other fields ...
    banner: MessageBanner,  // replaces error_message: Option<String>
    // ...
}
```

**Constructor:**
```rust
Self {
    // ...
    banner: MessageBanner::new(),  // replaces error_message: None
    // ...
}
```

**Rendering in `ui()`** (replaces the 20-line block):
```rust
// Display message banner at the top, outside of scroll area
self.banner.show(ui);
```

**`display_message` impl** (keeps custom step-reset logic):
```rust
fn display_message(&mut self, message: &str, message_type: MessageType) {
    self.banner.set_message(message, message_type);
    if message_type == MessageType::Error {
        // Reset step so UI is not stuck on waiting messages
        let mut step = self.step.write().unwrap();
        if *step == WalletFundedScreenStep::WaitingForPlatformAcceptance
            || *step == WalletFundedScreenStep::WaitingForAssetLock
        {
            *step = WalletFundedScreenStep::ChooseFundingMethod;
        }
    }
}
```

Note that the error message prefix `"Error topping up identity: "` is dropped. The banner's visual styling (red color, error icon) already communicates that it is an error. The backend task error string itself is descriptive enough.

### Example: Simple Screen (no custom `display_message`)

Many screens have no custom `display_message` override and rely on the default no-op. After migration, these screens get a one-line `display_message`:

```rust
fn display_message(&mut self, message: &str, message_type: MessageType) {
    self.banner.set_message(message, message_type);
}
```

And a one-line render call in `ui()`:

```rust
self.banner.show(ui);
```

---

## 7. Architectural Decisions Summary

| Decision | Rationale |
|----------|-----------|
| Per-screen ownership (not global) | Matches egui immediate-mode model. No shared mutable state. Each screen is self-contained. |
| `Instant` for timing (not egui frame time) | Monotonic, works correctly even if frames are missed while screen is not visible. |
| Follows Component Design Pattern conventions, not `Component` trait | `MessageBanner` is a display widget (consumes data), not an input widget (produces domain values). It follows the pattern's conventions (private fields, builder, `show()`, DashColors) like `StyledButton`/`InfoPopup`. |
| All colors via `DashColors` methods | Zero hardcoded `Color32` values. New `DashColors::message_color()`, `message_background_color()`, and `info_color()` methods ensure single source of truth. If the design system is updated, the banner reflects changes automatically. |
| Delete dead `theme.rs::MessageType`, consolidate into `DashColors` | The dead enum had light-mode-only `color()`/`background_color()` methods. The replacement `DashColors::message_color(type, dark_mode)` and `DashColors::message_background_color(type, dark_mode)` are dark-mode-aware and reusable by any component. |
| No accessor method on `ScreenLike` | Avoids coupling the trait to a specific field name. The two-line `display_message` boilerplate is acceptable. |
| Incremental migration (one screen at a time) | Reduces risk. Each migration is a small, reviewable change. Old and new patterns coexist during migration. |

---

## 8. Alternatives Considered

### Alternative A: Global App-Level Banner (Rejected)

Render the banner in `AppState::update()` above all screens, eliminating per-screen ownership.

**Pros:** Zero migration effort for backend task errors — `AppState` would call `banner.set_message()` directly instead of routing through `display_message()`.

**Rejected because:** Screens render inside panels with different layouts (left sidebar, main content, modals). A global banner can't know the correct placement within each screen's layout. Also, screens that set validation errors during `ui()` would still need their own banner. Result: two systems instead of one.

### Alternative B: Implement `Component` Trait (Rejected)

Make `MessageBanner` implement `Component<DomainType = (), Response = MessageBannerResponse>`.

**Pros:** Perfectly uniform with `AmountInput` and `ConfirmationDialog`.

**Rejected because:** `Component` requires `DomainType` (the value the component produces), `ComponentResponse` with `has_changed()`/`changed_value()`/`update()`, and `current_value()`. A display widget has no meaningful domain value to produce. Using `DomainType = ()` and `changed_value() -> &Option<()>` would be semantically empty. The pattern doc itself describes components that "handle wallet selection" or "manage passwords" — input components. Display-only widgets (`StyledButton`, `GradientHeading`, `InfoPopup`) don't use the trait either.

### Alternative C: Add `fn banner() -> &mut MessageBanner` to ScreenLike (Rejected)

Add an accessor method to the trait so `AppState` could call `screen.banner().set_message()` directly.

**Pros:** Eliminates the two-line `display_message` boilerplate per screen.

**Rejected because:** Forces every `ScreenLike` implementor to have a `MessageBanner` field (including screens that legitimately don't need messages, like `ProofLogScreen`). Changing the trait contract for 50+ screens simultaneously increases migration risk.

### Alternative D: Use `DateTime<Utc>` for Timing (Rejected)

8 screens already use `DateTime<Utc>` for timed messages.

**Rejected because:** `Instant` is monotonic (immune to system clock changes), simpler (no chrono dependency needed), and purpose-built for duration measurement. `DateTime<Utc>` is correct for display timestamps but wrong for timeout logic.
