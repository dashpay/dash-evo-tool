use arboard::Clipboard;
use eframe::epaint::{Color32, ColorImage};
use egui::Vec2;
use image::Luma;
use qrcode::QrCode;
use std::sync::{Arc, RwLock};

use crate::lock_helper::RwLockExt;
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
        *step.read_or_recover(),
        WalletFundedScreenStep::WaitingOnFunds
    ) {
        return None;
    }

    let address = funding_address.cloned()?;

    let wallet_arc = wallet?;

    let candidate_utxo = {
        let wallet = wallet_arc.read_or_recover();
        wallet.utxos.get(&address).and_then(|utxos| {
            utxos
                .iter()
                .filter(|(_, tx_out)| tx_out.value > 0)
                .max_by_key(|(_, tx_out)| tx_out.value)
                .map(|(outpoint, tx_out)| (*outpoint, tx_out.clone()))
        })
    };

    if let Some((outpoint, tx_out)) = candidate_utxo {
        let mut step = step.write_or_recover();
        *step = WalletFundedScreenStep::FundsReceived;
        Some((outpoint, tx_out, address))
    } else {
        None
    }
}
