//! Shared chrome for the app's centered modal dialogs.
//!
//! One place owns the dark backdrop and the bordered, centered `Window` used by the
//! passphrase modal and the confirmation / selection / info dialogs. Callers keep their
//! own dismissal policy (Escape / Enter / click-outside) since it differs per dialog.

use egui::{Context, Response, Ui, WidgetText};

use crate::ui::theme::DashColors;

/// The "hide this button" sentinel for dialog builders taking `Option<impl Into<WidgetText>>`.
pub const NOTHING: Option<&str> = None;

/// Layout and ordering knobs for one [`modal_chrome`] render.
pub struct ModalChromeConfig {
    /// Window title-bar text.
    pub title: WidgetText,
    /// Distinct id for the dark backdrop layer, so stacked modals don't share one.
    pub overlay_id: egui::Id,
    /// Paint order for the dark backdrop.
    pub overlay_order: egui::Order,
    /// Paint order for the window (above the backdrop).
    pub window_order: egui::Order,
    /// Whether the user can resize the window.
    pub resizable: bool,
    /// Inner padding of the window frame, in points.
    pub inner_margin: i8,
}

/// Outcome of a [`modal_chrome`] render.
pub struct ModalChrome<R> {
    /// The window's `Response` when it rendered; its `.rect` feeds `clicked_outside_window`.
    pub window_response: Option<Response>,
    /// True when the title-bar close (X) was clicked this frame.
    pub closed_via_x: bool,
    /// Value returned by `body` when the window rendered.
    pub inner: Option<R>,
}

/// Draws the dark backdrop and a centered, bordered modal window, running `body` inside.
///
/// The caller owns dismissal policy: inspect [`ModalChrome::window_response`]'s rect with
/// [`clicked_outside_window`](crate::ui::helpers::clicked_outside_window) and
/// [`ModalChrome::closed_via_x`], plus its own Escape/Enter handling.
pub fn modal_chrome<R>(
    ctx: &Context,
    config: ModalChromeConfig,
    body: impl FnOnce(&mut Ui) -> R,
) -> ModalChrome<R> {
    let screen_rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(config.overlay_order, config.overlay_id));
    painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

    let mut is_open = true;
    let window_response = egui::Window::new(config.title)
        .collapsible(false)
        .resizable(config.resizable)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(config.window_order)
        .open(&mut is_open)
        .frame(egui::Frame {
            inner_margin: egui::Margin::same(config.inner_margin),
            outer_margin: egui::Margin::same(0),
            corner_radius: egui::CornerRadius::same(8),
            shadow: egui::epaint::Shadow {
                offset: [0, 8],
                blur: 16,
                spread: 0,
                color: DashColors::popup_shadow(),
            },
            fill: ctx.global_style().visuals.window_fill,
            stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
        })
        .show(ctx, body);

    let (window_response, inner) = match window_response {
        Some(r) => (Some(r.response), r.inner),
        None => (None, None),
    };

    ModalChrome {
        window_response,
        closed_via_x: !is_open,
        inner,
    }
}
