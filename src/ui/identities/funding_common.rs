use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use eframe::epaint::{Color32, ColorImage};
use egui::Vec2;
use image::Luma;
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
use qrcode::QrCode;

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
}
