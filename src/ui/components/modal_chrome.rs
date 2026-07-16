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
    /// Whether the title bar shows a close button.
    pub show_close_button: bool,
    /// Whether input to everything behind the window is blocked. When set, the
    /// window's own layer is registered as egui's modal layer, so background
    /// widgets receive neither pointer nor keyboard input while the modal's own
    /// fields stay interactive.
    pub blocks_input: bool,
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

    if config.blocks_input {
        // Full-screen interactable sink that physically covers the whole
        // viewport. It supplies the pointer coverage the modal layer alone
        // cannot: `Context::layer_id_at` only redirects a below-modal click to
        // the modal layer when *some* interactable area covers that position, so
        // without a full-screen area every click landing outside the centered
        // window (which the window does not cover) would fall straight through to
        // the app beneath. With the sink present, `layer_id_at` returns the sink
        // or the window at every position — never a background widget's layer —
        // so nothing behind the modal receives pointer input.
        //
        // The sink is rendered at `Order::Middle`, strictly below the window's
        // `Order::Foreground`, so the window's own fields always resolve at/above
        // the modal layer (registered below) and stay focusable. The sink is NOT
        // the modal layer — that is the window itself (see the note after
        // `window.show`).
        let sink_id = config.overlay_id.with("input_sink");
        egui::Area::new(sink_id)
            .order(egui::Order::Middle)
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                ui.allocate_response(screen_rect.size(), egui::Sense::click_and_drag());
            });
    }

    let mut is_open = true;
    let mut window = egui::Window::new(config.title)
        .collapsible(false)
        .resizable(config.resizable)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(config.window_order)
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
        });
    if config.show_close_button {
        window = window.open(&mut is_open);
    }
    let window_response = window.show(ctx, body);

    // Register the window's OWN layer as egui's modal layer.
    //
    // egui gates BOTH keyboard focus (`Memory::allows_interaction`, via
    // `Context::create_widget`) and pointer hit-testing (`Context::layer_id_at`)
    // on the registered modal layer: a widget can focus / be clicked only if its
    // layer is at/above the modal layer. Registering the window's own layer makes
    // `compare_order(window, window)` `Equal`, so the modal's fields (e.g. the
    // passphrase input) always resolve at the modal layer and stay focusable,
    // while every lower layer — the app beneath and the `Order::Middle` sink — is
    // blocked.
    //
    // A prior version registered the full-screen sink as the modal layer instead.
    // Because the sink is a different layer than the window and (as a same-order
    // Area) did not reliably resolve below it, the window fell *below* the modal
    // layer and the password `TextEdit` was silently denied focus (NEW-005). The
    // sink is still rendered — it supplies the full-screen pointer coverage that
    // blocks the background — but it is no longer the modal layer.
    //
    // `set_modal_layer` takes effect next frame (egui consumes it into
    // `top_modal_layer` at end of pass); the opening-frame pointer click is
    // handled separately by the caller.
    if config.blocks_input
        && let Some(ref r) = window_response
    {
        let window_layer = r.response.layer_id;
        ctx.memory_mut(|memory| memory.set_modal_layer(window_layer));
    }

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
