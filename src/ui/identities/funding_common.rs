use crate::model::wallet::Wallet;
use crate::ui::state::TrackedAssetLockCache;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use eframe::epaint::{Color32, ColorImage};
use egui::{ComboBox, Ui, Vec2};
use image::Luma;
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
use qrcode::QrCode;
use std::fmt;
use std::sync::{Arc, RwLock};

/// How the user chooses to fund an identity operation. Shared by the
/// create-identity and top-up screens (both render the same chooser), so the
/// enum, its labels, and the default pre-selection live in one place.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum FundingMethod {
    NoSelection,
    UseUnusedAssetLock,
    UseWalletBalance,
    /// Use Platform Address credits.
    UsePlatformAddress,
}

impl fmt::Display for FundingMethod {
    /// Everyday-user labels. `UseUnusedAssetLock` deliberately avoids "asset
    /// lock" jargon — in the create context it reads as recovering an
    /// interrupted setup. Top-Up uses [`FundingMethod::top_up_label`] instead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = match self {
            FundingMethod::NoSelection => "Select how to fund",
            FundingMethod::UseWalletBalance => "From your wallet (recommended)",
            FundingMethod::UseUnusedAssetLock => "Recover an unfinished funding",
            FundingMethod::UsePlatformAddress => "Use a Platform address",
        };
        write!(f, "{}", output)
    }
}

impl FundingMethod {
    /// Top-Up-context label. Shares [`Display`]'s wording except for
    /// `UseUnusedAssetLock`: an existing identity being topped up was never
    /// mid-setup, so "recover an unfinished funding" doesn't fit — it just
    /// reuses an existing funding transaction.
    pub fn top_up_label(&self) -> &'static str {
        match self {
            FundingMethod::NoSelection => "Select how to fund",
            FundingMethod::UseWalletBalance => "From your wallet (recommended)",
            FundingMethod::UseUnusedAssetLock => "Use an existing funding transaction",
            FundingMethod::UsePlatformAddress => "Use a Platform address",
        }
    }
}

/// The funding-method chooser's starting state for a wallet that either does
/// or doesn't have spendable balance (§B.9: pre-select `UseWalletBalance`
/// only when it is actually available; otherwise start unselected rather
/// than land on a method the ComboBox itself wouldn't offer). A pure function
/// so the decision is testable without constructing a real wallet/balance
/// snapshot.
pub fn default_funding_state(wallet_has_balance: bool) -> (FundingMethod, WalletFundedScreenStep) {
    if wallet_has_balance {
        (
            FundingMethod::UseWalletBalance,
            WalletFundedScreenStep::ReadyToCreate,
        )
    } else {
        (
            FundingMethod::NoSelection,
            WalletFundedScreenStep::ChooseFundingMethod,
        )
    }
}

/// Resolve the funding method and step after the selected wallet changes.
///
/// While the user has not made an explicit choice, the screen re-applies its
/// default pre-selection for the new wallet, so the chooser always tracks a
/// wallet the current selection can actually be funded from. Once a method has
/// been chosen explicitly, a wallet switch preserves it untouched — a switch
/// must never silently change a funding method the user picked on purpose.
pub fn funding_method_after_switch(
    user_chose_method: bool,
    current: (FundingMethod, WalletFundedScreenStep),
    new_wallet_has_balance: bool,
) -> (FundingMethod, WalletFundedScreenStep) {
    if user_chose_method {
        current
    } else {
        default_funding_state(new_wallet_has_balance)
    }
}

/// Whether a wallet holding `spendable_duffs` can cover `minimum_credits` of
/// platform fees. Shared by the Create-Identity and Top-Up wallet-balance
/// funding gates so the duffs -> credits conversion has one source of truth.
pub fn spendable_covers_minimum(spendable_duffs: u64, minimum_credits: u64) -> bool {
    spendable_duffs.saturating_mul(CREDITS_PER_DUFF) >= minimum_credits
}

/// The largest amount, in credits, a "Max" button can safely offer from a
/// wallet holding `spendable_duffs`, after reserving `fee_credits` for the
/// platform fee. Built on `spendable_duffs` (not the wallet's `total`, which
/// also counts immature/locked funds coin selection cannot touch) so the
/// offered amount never exceeds what the wallet can actually send.
pub fn max_amount_after_fee_reserve(spendable_duffs: u64, fee_credits: u64) -> u64 {
    spendable_duffs
        .saturating_mul(CREDITS_PER_DUFF)
        .saturating_sub(fee_credits)
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub enum WalletFundedScreenStep {
    ChooseFundingMethod,
    WaitingOnFunds,
    FundsReceived,
    ReadyToCreate,
    WaitingForAssetLock,
    WaitingForPlatformAcceptance,
    Success,
}

/// A calm, jargon-free sentence describing where a funding asset lock is in its
/// lifecycle. Shown to the Everyday User when they pick an existing asset lock
/// to fund an identity, so they never see a raw `Debug` enum.
pub fn asset_lock_status_label(status: &AssetLockStatus) -> &'static str {
    match status {
        AssetLockStatus::Built => "Prepared, not yet sent to the network.",
        AssetLockStatus::Broadcast => "Sent to the network. Waiting for confirmation.",
        AssetLockStatus::InstantSendLocked => "Confirmed and ready to use.",
        AssetLockStatus::ChainLocked => "Confirmed and ready to use.",
        AssetLockStatus::Consumed => "Already used to fund an identity.",
    }
}

/// The Dash address that received the locked funds for this asset lock, derived
/// from the lock transaction's credit output. Returns `None` when the address
/// cannot be derived (e.g. a non-standard output). Lets the user tell two asset
/// locks apart by address as well as transaction id.
///
/// Mirrors the upstream recovery derivation, which reads the first credit output
/// of the asset-lock payload (asset locks built here carry a single credit
/// output).
pub fn asset_lock_address(lock: &TrackedAssetLock, network: Network) -> Option<Address> {
    let Some(TransactionPayload::AssetLockPayloadType(payload)) =
        &lock.transaction.special_transaction_payload
    else {
        return None;
    };
    let output = payload.credit_outputs.first()?;
    Address::from_script(&output.script_pubkey, network).ok()
}

/// Render a wallet-picker ComboBox and return the wallet the user clicked this
/// frame, if any. `label_fn` supplies each entry's text and the closed-box text
/// for the current selection; `enabled_fn` greys out wallets that cannot serve
/// the active funding method. Reset-on-change is the caller's responsibility —
/// act on the returned wallet. Shared by the create-identity, top-up, and
/// add-existing-identity screens so the picker scaffolding lives in one place.
pub fn wallet_selection_combo(
    ui: &mut Ui,
    id_salt: &str,
    wallets: &[Arc<RwLock<Wallet>>],
    selected: Option<&Arc<RwLock<Wallet>>>,
    mut label_fn: impl FnMut(&Arc<RwLock<Wallet>>) -> String,
    mut enabled_fn: impl FnMut(&Arc<RwLock<Wallet>>) -> bool,
) -> Option<Arc<RwLock<Wallet>>> {
    let selected_text = match selected {
        Some(wallet) => label_fn(wallet),
        None => "Select".to_string(),
    };

    let mut clicked = None;
    ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for wallet in wallets {
                let is_selected = selected.is_some_and(|s| Arc::ptr_eq(s, wallet));
                let label = label_fn(wallet);
                let enabled = enabled_fn(wallet);
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.selectable_label(is_selected, label).clicked() {
                        clicked = Some(wallet.clone());
                    }
                });
            }
        });
    clicked
}

/// Outcome of the shared unused-funding picker gate.
pub enum FundingAssetLockPicker {
    /// A gate message (no wallet / busy / load failed / loading / none found)
    /// was already rendered; there is nothing for the caller to show.
    Handled,
    /// Asset locks still usable to fund an identity, for the caller to render.
    Available(Vec<TrackedAssetLock>),
}

/// Render the shared load/gate states for the unused-funding picker and return
/// the actionable asset locks (Built / Broadcast / IS-Locked / Chain-Locked —
/// never Consumed) for the selected wallet. The create-identity and top-up
/// screens differ only in how each lock row looks, so this owns the identical
/// wallet-resolution, retry, loading, and empty states.
pub fn actionable_asset_locks(
    ui: &mut Ui,
    cache: &mut TrackedAssetLockCache,
    selected_wallet: Option<&Arc<RwLock<Wallet>>>,
) -> FundingAssetLockPicker {
    let Some(wallet) = selected_wallet else {
        ui.label("No wallet selected.");
        return FundingAssetLockPicker::Handled;
    };

    let seed_hash = match wallet.read() {
        Ok(w) => w.seed_hash(),
        Err(_) => {
            ui.label("Wallet is busy. Try again in a moment.");
            return FundingAssetLockPicker::Handled;
        }
    };

    if cache.is_failed(&seed_hash) {
        ui.label("Couldn't load your unfinished funding.");
        if ui.button("Retry").clicked() {
            cache.invalidate_one(&seed_hash);
        }
        return FundingAssetLockPicker::Handled;
    }

    let Some(all_tracked) = cache.get(&seed_hash) else {
        ui.label("Loading your unfinished funding…");
        return FundingAssetLockPicker::Handled;
    };

    // Consumed locks are tracked for history but cannot fund an identity.
    let tracked: Vec<TrackedAssetLock> = all_tracked
        .iter()
        .filter(|t| !matches!(t.status, AssetLockStatus::Consumed))
        .cloned()
        .collect();

    if tracked.is_empty() {
        ui.label("No unfinished funding was found.");
        return FundingAssetLockPicker::Handled;
    }

    FundingAssetLockPicker::Available(tracked)
}

// Function to generate a QR code image from the address
pub fn generate_qr_code_image(pay_uri: &str) -> Result<ColorImage, qrcode::types::QrError> {
    // Generate the QR code
    let code = QrCode::new(pay_uri.as_bytes())?;

    // Render the QR code into an image buffer
    let image = code.render::<Luma<u8>>().build();

    // Convert the image buffer to ColorImage
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let pixels: Vec<Color32> = pixels
        .into_iter()
        .map(|p| {
            let color = 255 - p; // Invert colors for better visibility
            Color32::from_rgba_unmultiplied(color, color, color, 255)
        })
        .collect();

    Ok(ColorImage {
        size,
        source_size: Vec2::new(size[0] as f32, size[1] as f32),
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_is_user_facing_for_every_variant() {
        // Exhaustive over the enum so a new variant forces a copy decision here
        // instead of silently falling back to a Debug render in the UI.
        for status in [
            AssetLockStatus::Built,
            AssetLockStatus::Broadcast,
            AssetLockStatus::InstantSendLocked,
            AssetLockStatus::ChainLocked,
            AssetLockStatus::Consumed,
        ] {
            let label = asset_lock_status_label(&status);
            assert!(label.ends_with('.'), "label should be a sentence: {label}");
            let debug = format!("{status:?}");
            assert_ne!(label, debug, "label must not be the Debug repr");
            assert!(
                !label.contains("AssetLockStatus") && !label.contains("InstantSendLocked"),
                "label must not leak enum jargon: {label}"
            );
        }
    }

    #[test]
    fn exact_balance_covers_minimum() {
        let minimum_credits = 10 * CREDITS_PER_DUFF;
        assert!(spendable_covers_minimum(10, minimum_credits));
    }

    #[test]
    fn one_credit_short_of_minimum_is_insufficient() {
        let minimum_credits = 10 * CREDITS_PER_DUFF + 1;
        assert!(!spendable_covers_minimum(10, minimum_credits));
    }

    #[test]
    fn one_credit_above_minimum_is_sufficient() {
        let minimum_credits = 10 * CREDITS_PER_DUFF - 1;
        assert!(spendable_covers_minimum(10, minimum_credits));
    }

    #[test]
    fn zero_spendable_never_covers_a_positive_minimum() {
        assert!(!spendable_covers_minimum(0, 1));
    }

    #[test]
    fn conversion_does_not_overflow_on_extreme_values() {
        assert!(spendable_covers_minimum(u64::MAX, u64::MAX));
    }

    #[test]
    fn max_amount_reserves_fee_from_spendable_duffs() {
        let spendable_duffs = 10;
        let fee_credits = 500;
        assert_eq!(
            max_amount_after_fee_reserve(spendable_duffs, fee_credits),
            spendable_duffs * CREDITS_PER_DUFF - fee_credits
        );
    }

    #[test]
    fn max_amount_saturates_to_zero_when_fee_exceeds_spendable() {
        assert_eq!(max_amount_after_fee_reserve(1, u64::MAX), 0);
    }

    #[test]
    fn max_amount_does_not_overflow_on_extreme_values() {
        assert_eq!(max_amount_after_fee_reserve(u64::MAX, 0), u64::MAX);
    }

    /// A wallet with spendable balance defaults to the recommended path,
    /// pre-selected and ready to go.
    #[test]
    fn default_funding_state_prefers_wallet_balance_when_available() {
        assert_eq!(
            default_funding_state(true),
            (
                FundingMethod::UseWalletBalance,
                WalletFundedScreenStep::ReadyToCreate
            )
        );
    }

    /// A wallet with nothing to fund from must not default to a method the
    /// ComboBox itself wouldn't offer — that was the fresh-wallet dead end. It
    /// starts unselected instead.
    #[test]
    fn default_funding_state_falls_back_to_no_selection_without_balance() {
        assert_eq!(
            default_funding_state(false),
            (
                FundingMethod::NoSelection,
                WalletFundedScreenStep::ChooseFundingMethod
            )
        );
    }

    /// Exhaustive over the enum so a new variant forces a copy decision here
    /// instead of silently falling back to a Debug render in the UI.
    #[test]
    fn display_is_jargon_free_for_every_variant() {
        for method in [
            FundingMethod::NoSelection,
            FundingMethod::UseUnusedAssetLock,
            FundingMethod::UseWalletBalance,
            FundingMethod::UsePlatformAddress,
        ] {
            let label = format!("{method}");
            let debug = format!("{method:?}");
            assert_ne!(label, debug, "label must not be the Debug repr");
            assert!(
                !label.contains("Asset Lock") && !label.contains("asset lock"),
                "label must not leak asset-lock jargon: {label}"
            );
        }
    }

    #[test]
    fn use_wallet_balance_is_the_recommended_primary_path() {
        assert_eq!(
            format!("{}", FundingMethod::UseWalletBalance),
            "From your wallet (recommended)"
        );
    }

    /// Top-Up's asset-lock label must not describe "recovering" a funding
    /// setup — an identity being topped up already exists and was never
    /// mid-creation. Every other variant keeps `Display`'s wording.
    #[test]
    fn top_up_label_differs_only_for_asset_lock() {
        assert_eq!(
            FundingMethod::UseUnusedAssetLock.top_up_label(),
            "Use an existing funding transaction"
        );
        assert_ne!(
            FundingMethod::UseUnusedAssetLock.top_up_label(),
            format!("{}", FundingMethod::UseUnusedAssetLock)
        );

        for method in [
            FundingMethod::NoSelection,
            FundingMethod::UseWalletBalance,
            FundingMethod::UsePlatformAddress,
        ] {
            assert_eq!(method.top_up_label(), format!("{method}"));
        }
    }

    /// F3 case 1: with no explicit choice yet, a wallet switch re-applies the
    /// screen's default pre-selection for the new wallet. Switching from a
    /// funded wallet to an unfunded one drops the stale `UseWalletBalance`
    /// pre-selection back to `NoSelection` instead of keeping a method the new
    /// wallet can't fund.
    #[test]
    fn switch_without_explicit_choice_recomputes_default() {
        let stale = (
            FundingMethod::UseWalletBalance,
            WalletFundedScreenStep::ReadyToCreate,
        );
        assert_eq!(
            funding_method_after_switch(false, stale, false),
            (
                FundingMethod::NoSelection,
                WalletFundedScreenStep::ChooseFundingMethod
            )
        );
        // And onto a funded wallet it recommends the wallet-balance path.
        assert_eq!(
            funding_method_after_switch(
                false,
                (
                    FundingMethod::NoSelection,
                    WalletFundedScreenStep::ChooseFundingMethod
                ),
                true
            ),
            (
                FundingMethod::UseWalletBalance,
                WalletFundedScreenStep::ReadyToCreate
            )
        );
    }

    /// F3 case 2: once the user has explicitly chosen a method, a wallet switch
    /// preserves it untouched — even when the new wallet's balance would have
    /// produced a different default.
    #[test]
    fn switch_after_explicit_choice_preserves_selection() {
        let chosen = (
            FundingMethod::UseUnusedAssetLock,
            WalletFundedScreenStep::ReadyToCreate,
        );
        assert_eq!(funding_method_after_switch(true, chosen, true), chosen);
        assert_eq!(funding_method_after_switch(true, chosen, false), chosen);
    }
}
