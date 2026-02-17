pub mod add_contact_screen;
pub mod contact_details;
pub mod contact_info_editor;
pub mod contact_profile_viewer;
pub mod contact_requests;
pub mod contacts_list;
pub mod dashpay_screen;
pub mod profile_screen;
pub mod profile_search;
pub mod qr_code_generator;
pub mod qr_scanner;
pub mod send_payment;

pub use add_contact_screen::AddContactScreen;
pub use dashpay_screen::{DashPayScreen, DashPaySubscreen};
pub use profile_search::ProfileSearchScreen;

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::ScreenType;
use chrono::{LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;

/// Format a Unix timestamp (seconds or milliseconds) as a human-readable
/// relative time string (e.g. "3 hours ago", "just now").
pub(crate) fn format_relative_time(timestamp: u64) -> Option<String> {
    if timestamp == 0 {
        return None;
    }
    // Distinguish seconds vs milliseconds: timestamps after year ~2001 in millis
    // exceed 1_000_000_000_000, while second-based timestamps won't until year 33658.
    let dt_result = if timestamp > 1_000_000_000_000 {
        Utc.timestamp_millis_opt(timestamp as i64)
    } else {
        Utc.timestamp_opt(timestamp as i64, 0)
    };
    match dt_result {
        LocalResult::Single(dt) => {
            let human = HumanTime::from(dt).to_string();
            if human.contains("seconds") {
                Some("just now".to_string())
            } else {
                Some(human)
            }
        }
        _ => None,
    }
}
use egui::{Frame, Margin, RichText, Ui};
use std::sync::Arc;

/// Renders a styled "No Identities Loaded" card for DashPay screens.
/// Returns an AppAction if the user clicks the "Load Identity" button.
pub fn render_no_identities_card(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;

    Frame::group(ui.style())
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(5.0)
        .outer_margin(Margin::same(20))
        .shadow(ui.visuals().window_shadow)
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(5.0);
                ui.label(
                    RichText::new("No Identities Loaded")
                        .strong()
                        .size(25.0)
                        .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                );

                ui.add_space(5.0);
                ui.separator();
                ui.add_space(10.0);

                ui.label(
                    "To use DashPay features, you need to load or create an identity first.",
                );

                ui.add_space(10.0);

                ui.heading(
                    RichText::new("Here's what you can do:")
                        .strong()
                        .size(18.0)
                        .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                );
                ui.add_space(5.0);

                ui.label("• LOAD an existing identity by clicking the button below, or");
                ui.add_space(1.0);
                ui.label("• CREATE a new identity from the Identities screen after setting up a wallet.");

                ui.add_space(15.0);

                let button = egui::Button::new(
                    RichText::new("Load Identity")
                        .color(egui::Color32::WHITE)
                        .strong(),
                )
                .fill(crate::ui::theme::DashColors::DASH_BLUE)
                .min_size(egui::vec2(150.0, 36.0));

                if ui.add(button).clicked() {
                    return AppAction::AddScreen(
                        ScreenType::AddExistingIdentity.create_screen(app_context),
                    );
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.label(
                    "(Make sure Dash Core is running. You can check in the network tab on the left.)",
                );

                ui.add_space(5.0);

                AppAction::None
            })
            .inner
        })
        .inner
}
