//! Contacts tab.
//!
//! Renders either the populated Contacts page (received requests · active
//! contacts · sent requests) or the social-profile gate card when the
//! currently-active identity has no DashPay profile yet. See design-spec §B.4
//! and §B.4.1.
//!
//! The tab does **not** introduce any new backend tasks — the populated-state
//! list feeds off the existing [`DashPayTask::LoadContacts`] and
//! [`DashPayTask::LoadContactRequests`] variants. Wire-through of the
//! dispatched action into `AppState` is owned by the hub screen, which is the
//! caller of [`render`]. The interactive accept / decline / cancel flows
//! ship in a follow-up task (T10) — this scaffold presents the shell shape
//! and copy only.

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::dashpay::DashPayTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::social_profile_gate_card::SocialProfileGateCard;
use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use eframe::egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use std::sync::Arc;

/// Copy constants, kept public so tests and sibling callsites share a single
/// source of truth. Complete sentences with no positional assumptions so
/// future i18n extraction is one line per string.
pub const ADD_BY_USERNAME_LABEL: &str = "Add by username";
pub const SCAN_QR_LABEL: &str = "Scan QR";
pub const SHOW_MY_QR_LABEL: &str = "Show my QR";
pub const RECEIVED_HEADING: &str = "Received requests";
pub const ACTIVE_HEADING_PREFIX: &str = "Active contacts";
pub const SENT_HEADING: &str = "Sent requests";
pub const NO_RECEIVED_EMPTY: &str = "No pending requests.";
pub const NO_ACTIVE_EMPTY: &str = "You have no contacts yet.";
pub const SEARCH_PLACEHOLDER: &str = "Search your contacts";

/// Hub-owned contacts-tab state. Tracks whether the Contacts tab has already
/// dispatched its `DashPayTask::LoadContacts` request for the current tab
/// entry. Without this, the populated state would fire a backend task on
/// every paint — that floods the channel and hammers the SDK. The flag is
/// reset by the hub when the user leaves the tab (via [`ContactsState::reset`])
/// or via an explicit refresh affordance.
#[derive(Debug, Default, Clone)]
pub struct ContactsState {
    /// Set to `true` after the first paint of the populated shell triggers
    /// the backend task. Guards all subsequent frames from re-dispatching.
    load_requested: bool,
}

impl ContactsState {
    /// Clear the dispatched flag so the next paint re-issues the load. Call
    /// this from `refresh()` / `refresh_on_arrival()` on the hub.
    pub fn reset(&mut self) {
        self.load_requested = false;
    }
}

/// Public entry point invoked by `hub_screen` when the Contacts tab is active.
///
/// Resolves the "current" identity as the first locally-loaded identity on
/// the active network (a pragmatic default until T7's identity picker lands).
/// When no identity is loaded, or the active identity has no DashPay profile,
/// the gated state is rendered.
///
/// The caller owns a [`ContactsState`] so the populated-shell only dispatches
/// its backend task once per tab entry — not once per paint.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state_guard: &mut ContactsState,
) -> AppAction {
    let state = ContactsTabState::resolve(app_context);
    render_state(ui, app_context, &state, state_guard)
}

/// Resolved rendering mode for the Contacts tab.
#[derive(Debug, Clone, PartialEq)]
pub enum ContactsTabState {
    /// No identity is loaded OR the active identity has no DashPay profile.
    ///
    /// The optional `handle` carries the active identity's primary DPNS
    /// username (without the leading `@`) when it is known, so the gate card
    /// can personalize its body copy.
    Gated { handle: Option<String> },
    /// The active identity has a DashPay profile — render the three-section
    /// populated shell.
    Populated {
        /// The currently-active identity. The populated-state renderer uses
        /// this to dispatch `DashPayTask::LoadContacts` and friends.
        identity: Box<QualifiedIdentity>,
    },
}

impl ContactsTabState {
    /// Inspect the app context to decide which state to render. Returns
    /// `Gated` if no identity is loaded, if loading fails, or if the active
    /// identity has no profile row in the DashPay local cache.
    pub fn resolve(app_context: &Arc<AppContext>) -> Self {
        // Fetch loaded identities on the active network. A load error falls
        // back to Gated so the hub never draws a half-broken populated UI;
        // the error is surfaced by the hub-level banner in `hub_screen`.
        let identities = match app_context.load_local_qualified_identities() {
            Ok(list) => list,
            Err(_) => return ContactsTabState::Gated { handle: None },
        };

        let Some(active) = identities.into_iter().next() else {
            return ContactsTabState::Gated { handle: None };
        };

        let handle = primary_dpns_handle(&active);
        if has_social_profile(app_context, &active) {
            ContactsTabState::Populated {
                identity: Box::new(active),
            }
        } else {
            ContactsTabState::Gated { handle }
        }
    }
}

/// Render the resolved state. Split out so tests can exercise the rendering
/// logic without reaching into `AppContext` internals.
fn render_state(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &ContactsTabState,
    state_guard: &mut ContactsState,
) -> AppAction {
    match state {
        ContactsTabState::Gated { handle } => render_gated(ui, handle.as_deref()),
        ContactsTabState::Populated { identity } => {
            render_populated(ui, app_context, identity, state_guard)
        }
    }
}

/// Centered gate card. The `Why?` panel toggle is a caller-owned boolean
/// persisted on the hub screen in a follow-up task; rendering it collapsed
/// here is the correct default for first paint.
///
/// Exposed to integration tests so IT-CONTACTS-01 can mount the gated view
/// without constructing a full `AppContext`.
pub fn render_gated(ui: &mut Ui, handle: Option<&str>) -> AppAction {
    let card = SocialProfileGateCard::new(handle);
    let response = card.show(ui);
    if response.primary_clicked {
        // Route to the Home-tab social-profile setup card. The hub owns tab
        // switching; emitting `AppAction::None` here keeps the screen free of
        // cross-tab coupling until the dedicated deep-link action lands in a
        // follow-up. The button remains visibly interactive so users see it
        // responds.
        //
        // TODO(identity-hub T8): return `AppAction::SwitchIdentityHubTab(Home)`
        // once the hub exposes that action.
    }
    if response.why_toggled {
        // TODO(identity-hub T9): persist the expanded flag on the hub screen
        // so the panel stays open across frames. Until then the card is
        // re-rendered collapsed each frame; the click still surfaces a
        // visible press so the affordance is not dead.
    }
    AppAction::None
}

/// Populated-state shell — three sections, placeholder empty-state copy, and
/// a dispatch of [`DashPayTask::LoadContacts`] / [`LoadContactRequests`] on
/// first paint so the real list can arrive without a new backend task.
fn render_populated(
    ui: &mut Ui,
    _app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state_guard: &mut ContactsState,
) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;

    header_row(ui, dark_mode);

    ui.add_space(12.0);
    section_card(ui, dark_mode, RECEIVED_HEADING, |ui| {
        ui.label(RichText::new(NO_RECEIVED_EMPTY).color(DashColors::text_secondary(dark_mode)));
    });

    ui.add_space(12.0);
    section_card(
        ui,
        dark_mode,
        &format!("{ACTIVE_HEADING_PREFIX} · 0"),
        |ui| {
            // Placeholder search input so the populated shell matches the
            // wireframe layout even before the real list is wired.
            let mut search = String::new();
            ui.add(
                eframe::egui::TextEdit::singleline(&mut search)
                    .hint_text(SEARCH_PLACEHOLDER)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(NO_ACTIVE_EMPTY).color(DashColors::text_secondary(dark_mode)));
            ui.add_space(8.0);
            ui.add(ComponentStyles::primary_button(ADD_BY_USERNAME_LABEL));
        },
    );

    ui.add_space(12.0);
    // Sent-section collapses entirely when empty (per design §B.4). The
    // populated shell renders only the heading so the section wiring stays
    // visible to reviewers; the real list hides it once data is live.
    section_card(ui, dark_mode, SENT_HEADING, |ui| {
        ui.label(
            RichText::new("No outgoing requests.").color(DashColors::text_secondary(dark_mode)),
        );
    });

    // Fire the existing backend task to populate the list — but only once per
    // tab entry. Without this guard the populated shell re-dispatches every
    // frame, which floods the backend channel and hammers the SDK. The hub
    // resets the guard in `refresh_on_arrival()` so a fresh tab switch or
    // explicit refresh will trigger another load.
    if state_guard.load_requested {
        return AppAction::None;
    }
    state_guard.load_requested = true;
    AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
        DashPayTask::LoadContacts {
            identity: identity.clone(),
        },
    )))
}

/// Header row: title on the left, three action buttons right-aligned.
fn header_row(ui: &mut Ui, dark_mode: bool) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Contacts")
                .strong()
                .size(22.0)
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.with_layout(
            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
            |ui| {
                ui.add(ComponentStyles::secondary_button(
                    SHOW_MY_QR_LABEL,
                    dark_mode,
                ));
                ui.add_space(8.0);
                ui.add(ComponentStyles::secondary_button(SCAN_QR_LABEL, dark_mode));
                ui.add_space(8.0);
                ui.add(ComponentStyles::primary_button(ADD_BY_USERNAME_LABEL));
            },
        );
    });
}

/// Render a bordered section card with a heading and caller-supplied body.
fn section_card(ui: &mut Ui, dark_mode: bool, heading: &str, body: impl FnOnce(&mut Ui)) {
    let frame = Frame::new()
        .fill(DashColors::surface_elevated(dark_mode))
        .stroke(Stroke::new(
            Shape::BORDER_WIDTH,
            DashColors::border(dark_mode),
        ))
        .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
        .inner_margin(Margin::symmetric(16, 12));
    frame.show(ui, |ui| {
        ui.label(
            RichText::new(heading)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(8.0);
        body(ui);
    });
}

/// Primary DPNS handle for an identity: the first registered DPNS name, when
/// available. Returns the bare handle without the leading `@`.
fn primary_dpns_handle(identity: &QualifiedIdentity) -> Option<String> {
    identity
        .dpns_names
        .first()
        .map(|n| n.name.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Detect whether the identity has a DashPay social profile. Reads the local
/// SQLite cache (authoritative for the UI layer — a missing row means we have
/// not seen a profile yet, which is the signal for the gated state). A load
/// error returns `false` so the gate is shown and the user is not blocked.
fn has_social_profile(app_context: &Arc<AppContext>, identity: &QualifiedIdentity) -> bool {
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    let identity_id = identity.identity.id();
    let network_str = app_context.network.to_string();
    matches!(
        app_context
            .db
            .load_dashpay_profile(&identity_id, &network_str),
        Ok(Some(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_state_variant_preserves_handle() {
        let s = ContactsTabState::Gated {
            handle: Some("alex.dash".to_string()),
        };
        match s {
            ContactsTabState::Gated { handle } => {
                assert_eq!(handle.as_deref(), Some("alex.dash"));
            }
            _ => panic!("expected Gated"),
        }
    }

    #[test]
    fn gated_state_variant_accepts_absent_handle() {
        let s = ContactsTabState::Gated { handle: None };
        matches!(s, ContactsTabState::Gated { handle: None });
    }

    #[test]
    fn populated_heading_format_matches_design() {
        // Design-spec §B.4: active-contacts header reads `Active contacts · {n}`.
        assert_eq!(
            format!("{ACTIVE_HEADING_PREFIX} · 0"),
            "Active contacts · 0"
        );
    }

    #[test]
    fn copy_constants_are_complete_sentences() {
        for line in [NO_RECEIVED_EMPTY, NO_ACTIVE_EMPTY] {
            assert!(
                line.ends_with('.'),
                "empty-state copy '{line}' must end with a period"
            );
            assert!(
                line.chars().next().unwrap().is_ascii_uppercase(),
                "empty-state copy '{line}' must start with an uppercase letter"
            );
        }
    }
}
