pub mod account_summary;
pub mod add_new_wallet_screen;
pub mod asset_lock_detail_screen;
pub mod create_asset_lock_screen;
pub mod import_mnemonic_screen;
pub mod send_screen;
pub mod shield_screen;
pub mod shielded_send_screen;
pub mod shielded_tab;
pub mod single_key_send_screen;
pub mod unshield_credits_screen;
pub mod wallets_screen;

use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use eframe::egui::Ui;

/// Shared user-facing copy shown in every surface (tooltips, inline banners)
/// where a single-key wallet action that needs Core (sending, refreshing
/// balances) is unavailable because the app is running on the built-in SPV
/// backend. Centralised so the wording stays consistent and a single string
/// needs updating when translations land.
///
/// Intentionally action-specific: receiving and viewing an already-loaded
/// balance/UTXO list still work in SPV mode, so the copy must not imply the
/// whole wallet is unusable.
pub const SINGLE_KEY_REQUIRES_CORE_MESSAGE: &str = "Sending and refreshing balances for single-key wallets require a local Dash Core node. Open Settings, switch to Expert mode, and select Local Dash Core node to enable these actions. Receiving still works in SPV mode.";

/// Renders the persistent "single-key wallets require Dash Core" notice as a
/// [`MessageBanner`] anchored to the current surface. Unlike the global
/// banner, this is not dismissed automatically — the underlying app state
/// (SPV backend mode) is what drives its visibility.
///
/// Constructed fresh each frame on purpose: this is a persistent state notice
/// (bound to the SPV backend mode), not a transient task result, so we want
/// it visible the whole time the mode is active. A fresh instance every frame
/// means the auto-dismiss timer never fires and the banner is shown
/// consistently; rendering is cheap and egui handles the repainting.
pub fn render_single_key_requires_core_banner(ui: &mut Ui) {
    let mut banner = MessageBanner::new();
    banner.set_message(SINGLE_KEY_REQUIRES_CORE_MESSAGE, MessageType::Warning);
    banner.show(ui);
}
