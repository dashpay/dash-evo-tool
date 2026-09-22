//! Avatar — a display-only widget that renders a DashPay contact/profile avatar
//! from a URL, backed by the [`AvatarCache`] fetch cache.
//!
//! The single avatar-rendering path for every DashPay screen. Given a URL and a
//! shared [`AvatarCache`], `show` renders one of:
//!
//! - the uploaded image once the bytes are fetched and decoded,
//! - a spinner while the fetch is in flight (also asking the caller to dispatch
//!   the fetch task the first time it is seen),
//! - a `👤` fallback glyph when there is no URL or the fetch/decode failed.
//!
//! Decode (center-crop to square) and GPU-texture upload happen here, on the UI
//! thread; the network fetch runs off-frame through the App Task System. See
//! [`AvatarCache`] for the state machine.
//!
//! [`AvatarCache`]: crate::ui::state::AvatarCache

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::ui::state::AvatarCache;
use crate::ui::theme::{DashColors, ResponseExt};
use egui::{ColorImage, RichText, TextureHandle, Ui};

/// A DashPay avatar widget. Configured per call site (size, corner radius,
/// clickability), rendered against a shared [`AvatarCache`].
pub struct Avatar<'a> {
    url: Option<&'a str>,
    size: f32,
    corner_radius: f32,
    clickable: bool,
    tooltip: Option<&'a str>,
}

/// Outcome of rendering an [`Avatar`].
#[derive(Default)]
pub struct AvatarResponse {
    /// A fetch task the caller must dispatch (the avatar was seen for the first
    /// time and is not yet cached). Combine into the screen's [`AppAction`] via
    /// [`AvatarResponse::into_action`].
    pub fetch: Option<BackendTask>,
    /// Whether a clickable avatar was clicked this frame.
    pub clicked: bool,
}

impl AvatarResponse {
    /// The fetch task as an [`AppAction`], or [`AppAction::None`] when nothing
    /// needs dispatching.
    pub fn into_action(self) -> AppAction {
        match self.fetch {
            Some(task) => AppAction::BackendTask(task),
            None => AppAction::None,
        }
    }
}

impl<'a> Avatar<'a> {
    /// A square avatar of `size` points for `url`. An empty or `None` URL renders
    /// the fallback glyph. Corner radius defaults to a full circle (`size / 2`).
    pub fn new(url: Option<&'a str>, size: f32) -> Self {
        Self {
            url,
            size,
            corner_radius: size / 2.0,
            clickable: false,
            tooltip: None,
        }
    }

    /// Override the corner radius (e.g. a rounded square instead of a circle).
    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    /// Make the rendered image clickable, showing `tooltip` on hover. The click
    /// is reported via [`AvatarResponse::clicked`].
    pub fn clickable(mut self, tooltip: &'a str) -> Self {
        self.clickable = true;
        self.tooltip = Some(tooltip);
        self
    }

    /// Render the avatar against `cache`, decoding and uploading its texture on
    /// the first frame after the bytes arrive.
    pub fn show(self, ui: &mut Ui, cache: &mut AvatarCache) -> AvatarResponse {
        let url = match self.url {
            Some(url) if !url.is_empty() => url,
            _ => {
                self.render_fallback(ui);
                return AvatarResponse::default();
            }
        };

        // Already decoded and uploaded — the steady state.
        if let Some(texture) = cache.ready_texture(url) {
            let clicked = self.render_image(ui, texture);
            return AvatarResponse {
                fetch: None,
                clicked,
            };
        }

        // Bytes fetched but not yet decoded: decode + upload once, then cache the
        // texture so later frames hit the branch above.
        match cache.fetched_bytes(url).map(decode_square_avatar) {
            Some(Some(image)) => {
                let texture = ui.ctx().load_texture(
                    format!("avatar:{url}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                cache.set_ready(url, texture.clone());
                let clicked = self.render_image(ui, &texture);
                return AvatarResponse {
                    fetch: None,
                    clicked,
                };
            }
            Some(None) => {
                // Fetched bytes did not decode as an image — fall back.
                cache.mark_failed(url);
                self.render_fallback(ui);
                return AvatarResponse::default();
            }
            None => {}
        }

        if cache.is_failed(url) {
            self.render_fallback(ui);
            return AvatarResponse::default();
        }

        // Loading or not yet requested: show a spinner and ask the caller to
        // dispatch the fetch (idempotent — `ensure_requested` dedups).
        self.render_spinner(ui);
        AvatarResponse {
            fetch: cache.ensure_requested(url),
            clicked: false,
        }
    }

    /// Render the uploaded avatar image, returning whether it was clicked.
    fn render_image(&self, ui: &mut Ui, texture: &TextureHandle) -> bool {
        let mut image = egui::Image::new(texture)
            .fit_to_exact_size(egui::vec2(self.size, self.size))
            .corner_radius(self.corner_radius);
        if self.clickable {
            image = image.sense(egui::Sense::click());
        }
        let response = ui.add(image);
        let response = match self.tooltip {
            Some(tooltip) => response.clickable_tooltip(tooltip),
            None => response,
        };
        self.clickable && response.clicked()
    }

    /// Render the `👤` fallback glyph (no URL, or fetch/decode failed).
    fn render_fallback(&self, ui: &mut Ui) {
        ui.label(
            RichText::new("👤")
                .size(self.size)
                .color(DashColors::DEEP_BLUE),
        );
    }

    /// Render the in-flight loading spinner.
    fn render_spinner(&self, ui: &mut Ui) {
        ui.add(
            egui::Spinner::new()
                .size(self.size)
                .color(DashColors::DASH_BLUE),
        );
    }
}

/// Decode `bytes` into a square [`ColorImage`], center-cropping to the shorter
/// side when the source is not already square. Returns `None` when the bytes are
/// not a decodable image.
fn decode_square_avatar(bytes: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());

    let cropped = if width != height {
        let side = width.min(height);
        let x_offset = (width - side) / 2;
        let y_offset = (height - side) / 2;
        image::imageops::crop_imm(&rgba, x_offset, y_offset, side, side).to_image()
    } else {
        rgba
    };

    let size = [cropped.width() as usize, cropped.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(
        size,
        &cropped.into_raw(),
    ))
}
