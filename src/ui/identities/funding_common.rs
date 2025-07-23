use std::fmt::Display;

use arboard::Clipboard;
use eframe::epaint::{Color32, ColorImage};
use egui::Vec2;
use image::Luma;
use qrcode::QrCode;

#[derive(Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub enum WalletFundedScreenStep {
    ChooseFundingMethod,
    WaitingForAssetLock,
    WaitingForPlatformAcceptance,
    Success,
}

impl WalletFundedScreenStep {
    /// Returns true if the step indicates that the wallet is in progress of identity creation
    pub fn is_processing(&self) -> bool {
        matches!(
            self,
            WalletFundedScreenStep::WaitingForAssetLock
                | WalletFundedScreenStep::WaitingForPlatformAcceptance
        )
    }
}

impl Display for WalletFundedScreenStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalletFundedScreenStep::ChooseFundingMethod => write!(f, "Choose Funding Method"),
            WalletFundedScreenStep::WaitingForAssetLock => write!(f, "Waiting for Asset Lock"),
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                write!(f, "Waiting for Platform Acceptance")
            }
            WalletFundedScreenStep::Success => write!(f, "Success"),
        }
    }
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
