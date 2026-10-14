//! Display-only QR code widget.
//!
//! Encodes a value into a QR code, keeps the uploaded texture keyed by that
//! value, and rebuilds it only when the value changes — so a screen can call
//! [`QrCodeImage::show`] every frame without re-encoding or re-uploading.

use eframe::epaint::{Color32, ColorImage};
use egui::{Context, Response, TextureHandle, TextureOptions, Ui, Vec2};
use image::Luma;
use qrcode::QrCode;

const DEFAULT_SIZE: f32 = 200.0;
const DEFAULT_FALLBACK: &str = "The QR code could not be created. Copy the value shown instead.";

/// Encoded value plus its texture (`None` when encoding failed, so a failure
/// is not retried every frame).
struct Cached {
    value: String,
    texture: Option<TextureHandle>,
}

/// QR code for a value. Hold it in the screen next to the value (lazily
/// created) and call [`show`](Self::show) each frame.
pub struct QrCodeImage {
    /// Edge length in points; `None` shows the image at its natural size.
    size: Option<f32>,
    filter: TextureOptions,
    fallback_text: &'static str,
    cached: Option<Cached>,
}

impl Default for QrCodeImage {
    fn default() -> Self {
        Self::new()
    }
}

impl QrCodeImage {
    pub fn new() -> Self {
        Self {
            size: Some(DEFAULT_SIZE),
            filter: TextureOptions::LINEAR,
            fallback_text: DEFAULT_FALLBACK,
            cached: None,
        }
    }

    /// Square edge length in points (default 200).
    pub fn with_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Show the image at the size the encoder produced instead of a fixed one.
    pub fn with_natural_size(mut self) -> Self {
        self.size = None;
        self
    }

    pub fn with_texture_options(mut self, filter: TextureOptions) -> Self {
        self.filter = filter;
        self
    }

    /// Sentence shown when the value cannot be encoded. Must tell the user to
    /// copy the value instead and carry no technical detail.
    pub fn with_fallback_text(mut self, text: &'static str) -> Self {
        self.fallback_text = text;
        self
    }

    /// The value last handed to [`show`](Self::show), whether or not it could
    /// be encoded.
    pub fn value(&self) -> Option<&str> {
        self.cached.as_ref().map(|c| c.value.as_str())
    }

    /// Draw the QR code for `value`, or the fallback sentence if it cannot be
    /// encoded.
    pub fn show(&mut self, ui: &mut Ui, value: &str) -> Response {
        let (size, fallback_text) = (self.size, self.fallback_text);
        match self.texture_for(ui.ctx(), value) {
            Some(texture) => match size {
                Some(edge) => ui.image((texture.id(), Vec2::splat(edge))),
                None => ui.image(texture),
            },
            None => ui.label(fallback_text),
        }
    }

    /// Texture for `value`; encodes and uploads only when `value` differs from
    /// the cached one.
    fn texture_for(&mut self, ctx: &Context, value: &str) -> Option<&TextureHandle> {
        if self.cached.as_ref().is_none_or(|c| c.value != value) {
            let texture = match encode(value) {
                Ok(image) => Some(ctx.load_texture("qr_code", image, self.filter)),
                Err(error) => {
                    // Diagnostics only; the user sees the fallback sentence.
                    tracing::warn!(?error, "QR code could not be created");
                    None
                }
            };
            self.cached = Some(Cached {
                value: value.to_owned(),
                texture,
            });
        }
        self.cached.as_ref().and_then(|c| c.texture.as_ref())
    }
}

fn encode(value: &str) -> Result<ColorImage, qrcode::types::QrError> {
    let code = QrCode::new(value.as_bytes())?;
    let image = code.render::<Luma<u8>>().build();

    let size = [image.width() as usize, image.height() as usize];
    let pixels = image
        .into_raw()
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
    fn same_value_reuses_texture() {
        let ctx = Context::default();
        let mut qr = QrCodeImage::new();
        let first = qr.texture_for(&ctx, "dash:Xabc").map(|t| t.id());
        let second = qr.texture_for(&ctx, "dash:Xabc").map(|t| t.id());
        assert!(first.is_some());
        assert_eq!(first, second);
        assert_eq!(qr.value(), Some("dash:Xabc"));
    }

    #[test]
    fn changed_value_rebuilds_texture() {
        let ctx = Context::default();
        let mut qr = QrCodeImage::new();
        let first = qr.texture_for(&ctx, "dash:Xabc").map(|t| t.id());
        let second = qr.texture_for(&ctx, "dash:Xdef").map(|t| t.id());
        assert!(second.is_some());
        assert_ne!(first, second);
    }

    #[test]
    fn unencodable_value_yields_no_texture_and_recovers() {
        let ctx = Context::default();
        let mut qr = QrCodeImage::new();
        // Larger than the biggest QR version can hold.
        let too_long = "x".repeat(4000);
        assert!(qr.texture_for(&ctx, &too_long).is_none());
        assert!(qr.texture_for(&ctx, &too_long).is_none());
        assert!(qr.texture_for(&ctx, "dash:Xabc").is_some());
    }

    #[test]
    fn show_renders_fallback_label_on_failure_without_panic() {
        let ctx = Context::default();
        let mut qr = QrCodeImage::new().with_fallback_text("Copy it instead.");
        let too_long = "x".repeat(4000);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            qr.show(ui, &too_long);
        });
        output.textures_delta.clear();
        assert!(qr.cached.is_some_and(|c| c.texture.is_none()));
    }
}
