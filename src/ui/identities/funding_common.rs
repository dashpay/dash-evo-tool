use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use eframe::epaint::{Color32, ColorImage};
use egui::Vec2;
use image::Luma;
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
use qrcode::QrCode;

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
}
