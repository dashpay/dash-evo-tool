use arboard::Clipboard;
use eframe::epaint::{Color32, ColorImage};
use egui::Vec2;
use image::Luma;
use qrcode::QrCode;
use std::sync::{Arc, RwLock};

use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::dashcore::{OutPoint, TxOut};

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

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

pub fn capture_qr_funding_utxo_if_available(
    step: &Arc<RwLock<WalletFundedScreenStep>>,
    wallet: Option<&Arc<RwLock<Wallet>>>,
    funding_address: Option<&Address>,
) -> Option<(OutPoint, TxOut, Address)> {
    if !matches!(
        *step.read().expect("wallet funding step lock poisoned"),
        WalletFundedScreenStep::WaitingOnFunds
    ) {
        return None;
    }

    let address = funding_address.cloned()?;
    let wallet_arc = wallet?;

    let guard = wallet_arc.read().ok()?;
    let pw = guard.platform_wallet.as_ref()?;
    let info = pw.state_blocking();

    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let candidate_utxo = info
        .core_wallet.get_spendable_utxos()
        .iter()
        .filter(|utxo| utxo.address == address && utxo.value() > 0)
        .max_by_key(|utxo: &&&dash_sdk::dpp::key_wallet::Utxo| utxo.value())
        .map(|utxo| (utxo.outpoint, utxo.txout.clone()));

    if let Some((outpoint, tx_out)) = candidate_utxo {
        let mut step = step
            .write()
            .expect("wallet funding step write lock poisoned");
        *step = WalletFundedScreenStep::FundsReceived;
        Some((outpoint, tx_out, address))
    } else {
        None
    }
}
