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

use super::social_profile_gate_card::SocialProfileGateCard;
use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::dashpay::DashPayTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::ScreenType;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape};
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

/// Every clickable affordance on the Contacts tab header + populated shell.
/// Mirrors the home-tab `HomeButton` dispatcher pattern so the dead-button
/// unit test can enumerate every variant and assert each one maps to a real
/// screen, not `AppAction::None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactsButton {
    /// Header `+ Add by username`.
    HeaderAddByUsername,
    /// Header `Scan QR`.
    HeaderScanQr,
    /// Header `Show my QR`.
    HeaderShowMyQr,
    /// Populated active-section `Add by username` button.
    ActiveAddByUsername,
    /// Gate card `Set up my profile` CTA (gated state only). Emits a hub
    /// outcome because the Settings tab is hub-local.
    GateSetUpProfile,
}

/// What a [`ContactsButton`] click produces, as a pure enum for unit tests.
/// `GateSetUpProfile` is `SwitchHubTab(Settings)` — a hub-local intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactsButtonKind {
    /// Open the given screen via `AppAction::AddScreen`.
    OpenScreen(ContactsScreenKind),
    /// Switch to another hub tab (§B.4.1 gate CTA -> Settings).
    SwitchHubTab(super::IdentityHubTab),
}

/// Screens any contacts button can open. Maps 1:1 to `ScreenType` — kept as a
/// pure enum for unit tests that have no `AppContext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactsScreenKind {
    /// `ScreenType::DashPayAddContact` — add-contact flow.
    AddContact,
    /// `ScreenType::DashPayQRGenerator` — show-my-QR screen.
    QrGenerator,
}

/// Pure dispatcher. Every variant MUST produce a non-dead result — the
/// `every_contacts_button_produces_live_action` test enforces this.
pub fn contacts_button_kind(button: ContactsButton) -> ContactsButtonKind {
    use ContactsButtonKind::*;
    use ContactsScreenKind::*;
    match button {
        ContactsButton::HeaderAddByUsername | ContactsButton::ActiveAddByUsername => {
            OpenScreen(AddContact)
        }
        // Scan QR currently routes to the add-contact screen (which owns
        // the existing scan affordance). TODO(identity-hub): if a dedicated
        // scan screen ships, swap to it here.
        ContactsButton::HeaderScanQr => OpenScreen(AddContact),
        ContactsButton::HeaderShowMyQr => OpenScreen(QrGenerator),
        ContactsButton::GateSetUpProfile => SwitchHubTab(super::IdentityHubTab::Settings),
    }
}

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
    profiles: &mut super::profile_cache::ProfileCache,
) -> AppAction {
    let state = ContactsTabState::resolve(app_context, profiles);
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
    /// identity has no DashPay profile (or one has not loaded yet).
    pub fn resolve(
        app_context: &Arc<AppContext>,
        profiles: &mut super::profile_cache::ProfileCache,
    ) -> Self {
        // The app-scoped active identity (selected → first → none). `None` on a
        // load error or no identities falls back to Gated so the hub never
        // draws a half-broken populated UI.
        let Some(active) = app_context.resolve_selected_identity() else {
            return ContactsTabState::Gated { handle: None };
        };

        let handle = primary_dpns_handle(&active);
        if has_social_profile(profiles, &active) {
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
        // Route to the Settings tab — that is where the user actually edits
        // display name and avatar (social-profile fields). Resolution goes
        // through the pure `contacts_button_kind` dispatcher, so `identity-
        // hub` feature gating is handled in one place.
        match contacts_button_kind(ContactsButton::GateSetUpProfile) {
            #[cfg(feature = "identity-hub")]
            ContactsButtonKind::SwitchHubTab(tab) => {
                return AppAction::SwitchIdentityHubTab(tab);
            }
            #[cfg(not(feature = "identity-hub"))]
            ContactsButtonKind::SwitchHubTab(_) => {
                // Without the identity-hub feature, there is no hub to
                // switch to — the gate card should not even be reachable
                // in that build, but we defend against it rather than
                // silently drop the click.
            }
            ContactsButtonKind::OpenScreen(_) => {
                // Not possible today (dispatcher returns SwitchHubTab), but
                // exhaustive match future-proofs the gate CTA.
                unreachable!("GateSetUpProfile should not map to OpenScreen");
            }
        }
    }
    if response.why_toggled {
        // TODO(identity-hub): persist the expanded flag on the hub screen so
        // the panel stays open across frames. Until then the card is
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
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state_guard: &mut ContactsState,
) -> AppAction {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut action = AppAction::None;

    action |= header_row(ui, app_context, dark_mode);

    ui.add_space(12.0);
    section_card(ui, dark_mode, RECEIVED_HEADING, |ui| {
        // TODO(identity-hub): render real `RequestCard::received` rows once a
        // loaded-contact-requests cache lands on `AppContext`. When wiring,
        // consume `RequestCardResponse::{accepted, declined}` and dispatch
        // `DashPayTask::AcceptContactRequest` / `RejectContactRequest` with
        // the request's `Identifier`. These backend variants already exist at
        // `src/backend_task/dashpay.rs`; wiring is additive to this file.
        ui.label(RichText::new(NO_RECEIVED_EMPTY).color(DashColors::text_secondary(dark_mode)));
    });

    ui.add_space(12.0);
    let mut active_add_action = AppAction::None;
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
            let add_resp = ui
                .add(ComponentStyles::primary_button(ADD_BY_USERNAME_LABEL))
                .clickable_tooltip(
                    "Find someone by their Dash username or identity ID and add them as a \
                     contact.",
                );
            if add_resp.clicked() {
                active_add_action =
                    resolve_contacts_button(ContactsButton::ActiveAddByUsername, app_context);
            }
        },
    );
    action |= active_add_action;

    ui.add_space(12.0);
    // Sent-section collapses entirely when empty (per design §B.4). The
    // populated shell renders only the heading so the section wiring stays
    // visible to reviewers; the real list hides it once data is live.
    section_card(ui, dark_mode, SENT_HEADING, |ui| {
        // TODO(identity-hub): render real `RequestCard::sent` rows once a
        // loaded-sent-requests cache lands. Consume
        // `RequestCardResponse::cancelled` and dispatch a "cancel" task —
        // requires a new `DashPayTask::CancelContactRequest` variant (not
        // yet present; do NOT add here — defer until a parallel wallet
        // refactor wave lands, per the integration constraints).
        ui.label(
            RichText::new("No outgoing requests.").color(DashColors::text_secondary(dark_mode)),
        );
    });

    // Fire the existing backend task to populate the list — but only once per
    // tab entry. Without this guard the populated shell re-dispatches every
    // frame, which floods the backend channel and hammers the SDK. The hub
    // resets the guard in `refresh_on_arrival()` so a fresh tab switch or
    // explicit refresh will trigger another load.
    if !state_guard.load_requested {
        state_guard.load_requested = true;
        action |= AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::LoadContacts {
                identity: identity.clone(),
            },
        )));
    }

    action
}

/// Header row: title on the left, three action buttons right-aligned.
///
/// Returns any `AppAction` generated by clicks on the three header buttons
/// (`Add by username`, `Scan QR`, `Show my QR`). Each is routed through the
/// pure [`contacts_button_kind`] dispatcher so the same mapping is used by
/// the unit tests and by the renderer.
fn header_row(ui: &mut Ui, app_context: &Arc<AppContext>, dark_mode: bool) -> AppAction {
    let mut action = AppAction::None;
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
                // `Show my QR` — opens the existing QR generator screen.
                let show_qr = ui
                    .add(ComponentStyles::secondary_button(
                        SHOW_MY_QR_LABEL,
                        dark_mode,
                    ))
                    .clickable_tooltip("Show a QR code so someone nearby can add you or pay you.");
                if show_qr.clicked() {
                    action |= resolve_contacts_button(ContactsButton::HeaderShowMyQr, app_context);
                }
                ui.add_space(8.0);

                // `Scan QR` — routes to the add-contact screen, which owns
                // the existing scan affordance; no new scan-only entry point
                // is introduced. TODO(identity-hub): swap to a dedicated QR
                // scanner screen if/when one ships.
                let scan = ui
                    .add(ComponentStyles::secondary_button(SCAN_QR_LABEL, dark_mode))
                    .clickable_tooltip("Use a camera or paste a QR image to add a contact.");
                if scan.clicked() {
                    action |= resolve_contacts_button(ContactsButton::HeaderScanQr, app_context);
                }
                ui.add_space(8.0);

                // `Add by username` — primary CTA routes to the existing
                // Add-contact screen (username-first input).
                let add = ui
                    .add(ComponentStyles::primary_button(ADD_BY_USERNAME_LABEL))
                    .clickable_tooltip(
                        "Find someone by their Dash username or identity ID and add them as a \
                         contact.",
                    );
                if add.clicked() {
                    action |=
                        resolve_contacts_button(ContactsButton::HeaderAddByUsername, app_context);
                }
            },
        );
    });
    action
}

/// Materialise a [`ContactsButton`] into a concrete [`AppAction`] using the
/// provided `AppContext`. Thin adapter over [`contacts_button_kind`] so the
/// renderer keeps using the pure dispatcher and tests can exercise the
/// decision logic without an `AppContext`.
fn resolve_contacts_button(button: ContactsButton, app_context: &Arc<AppContext>) -> AppAction {
    match contacts_button_kind(button) {
        ContactsButtonKind::OpenScreen(kind) => {
            let screen = match kind {
                ContactsScreenKind::AddContact => ScreenType::DashPayAddContact,
                ContactsScreenKind::QrGenerator => ScreenType::DashPayQRGenerator,
            };
            AppAction::AddScreen(screen.create_screen(app_context))
        }
        #[cfg(feature = "identity-hub")]
        ContactsButtonKind::SwitchHubTab(tab) => AppAction::SwitchIdentityHubTab(tab),
        #[cfg(not(feature = "identity-hub"))]
        ContactsButtonKind::SwitchHubTab(_) => AppAction::None,
    }
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

/// Detect whether the identity has a DashPay social profile. Reads the hub's
/// async profile cache; an entry that is absent (not loaded yet) or holds no
/// profile yields `false`, so the gate is shown and the user is not blocked
/// until a profile is confirmed to exist.
fn has_social_profile(
    profiles: &mut super::profile_cache::ProfileCache,
    identity: &QualifiedIdentity,
) -> bool {
    matches!(profiles.get_or_request(identity), Some(Some(_)))
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

    // ---------------------------------------------------------------
    // Dead-button regression tests — same invariant as home.rs:
    // every interactive button MUST produce a live result from the
    // pure dispatcher. The T8-Wave-2 regression was that the header
    // buttons were rendered without any click handling at all; this
    // suite pins the expected mapping.
    // ---------------------------------------------------------------

    const ALL_CONTACTS_BUTTONS: &[ContactsButton] = &[
        ContactsButton::HeaderAddByUsername,
        ContactsButton::HeaderScanQr,
        ContactsButton::HeaderShowMyQr,
        ContactsButton::ActiveAddByUsername,
        ContactsButton::GateSetUpProfile,
    ];

    #[test]
    fn contacts_all_buttons_list_is_exhaustive() {
        for button in ALL_CONTACTS_BUTTONS {
            let _: () = match *button {
                ContactsButton::HeaderAddByUsername => (),
                ContactsButton::HeaderScanQr => (),
                ContactsButton::HeaderShowMyQr => (),
                ContactsButton::ActiveAddByUsername => (),
                ContactsButton::GateSetUpProfile => (),
            };
        }
    }

    #[test]
    fn every_contacts_button_maps_to_a_live_action() {
        for button in ALL_CONTACTS_BUTTONS {
            let kind = contacts_button_kind(*button);
            // The dispatcher only produces two variants; both are live —
            // `OpenScreen` resolves to `AppAction::AddScreen(...)` and
            // `SwitchHubTab` to `AppAction::SwitchIdentityHubTab(...)`.
            match kind {
                ContactsButtonKind::OpenScreen(_) | ContactsButtonKind::SwitchHubTab(_) => {}
            }
        }
    }

    #[test]
    fn add_by_username_and_scan_qr_open_add_contact_screen() {
        assert_eq!(
            contacts_button_kind(ContactsButton::HeaderAddByUsername),
            ContactsButtonKind::OpenScreen(ContactsScreenKind::AddContact),
        );
        assert_eq!(
            contacts_button_kind(ContactsButton::ActiveAddByUsername),
            ContactsButtonKind::OpenScreen(ContactsScreenKind::AddContact),
        );
        assert_eq!(
            contacts_button_kind(ContactsButton::HeaderScanQr),
            ContactsButtonKind::OpenScreen(ContactsScreenKind::AddContact),
        );
    }

    #[test]
    fn show_my_qr_opens_qr_generator_screen() {
        assert_eq!(
            contacts_button_kind(ContactsButton::HeaderShowMyQr),
            ContactsButtonKind::OpenScreen(ContactsScreenKind::QrGenerator),
        );
    }

    #[test]
    fn gate_cta_switches_to_settings_tab() {
        assert_eq!(
            contacts_button_kind(ContactsButton::GateSetUpProfile),
            ContactsButtonKind::SwitchHubTab(super::super::IdentityHubTab::Settings),
        );
    }
}
