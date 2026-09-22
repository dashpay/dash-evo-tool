//! Contacts tab.
//!
//! Renders either the populated Contacts page (received requests · active
//! contacts · sent requests) or the social-profile gate card when the
//! currently-active identity has no DashPay profile yet. See design-spec §B.4
//! and §B.4.1.
//!
//! The three sections feed off [`DashPayTask::LoadContacts`] and
//! [`DashPayTask::LoadContactRequests`], dispatched once per tab entry. Results
//! are stored by `hub_screen::display_task_result` into the
//! [`ContactsState`] view-model, which this module only reads.
//!
//! Row actions dispatch the DashPay backend tasks directly: Accept and Decline
//! on a received request, Cancel on a sent one, and Pay on an established
//! contact (which opens the existing send-payment screen), and View Profile
//! (which opens the existing contact-profile viewer).

use super::request_card::{RequestAction, RequestCard};
use super::social_profile_gate_card::SocialProfileGateCard;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::dashpay::{ContactData, DashPayTask};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::dashpay::{ContactInfoUpdate, UnreadableContactInfoPolicy};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::ScreenType;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::state::contacts_view::contact_label;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use std::sync::Arc;

pub use crate::ui::state::contacts_view::{ContactRequestEntry, ContactsState};

/// Copy constants, kept public so tests and sibling callsites share a single
/// source of truth. Complete sentences with no positional assumptions so
/// future i18n extraction is one line per string.
pub const ADD_BY_USERNAME_LABEL: &str = "Add by username";
pub const SCAN_QR_LABEL: &str = "Scan QR";
pub const SHOW_MY_QR_LABEL: &str = "Show my QR";
pub const PAY_LABEL: &str = "Pay";
pub const VIEW_PROFILE_LABEL: &str = "View Profile";
pub const RECEIVED_HEADING: &str = "Received requests";
pub const ACTIVE_HEADING_PREFIX: &str = "Active contacts";
pub const SENT_HEADING: &str = "Sent requests";
pub const NO_RECEIVED_EMPTY: &str = "No pending requests.";
pub const NO_ACTIVE_EMPTY: &str = "You have no contacts yet.";
pub const NO_SENT_EMPTY: &str = "No outgoing requests.";
pub const NO_SEARCH_MATCH: &str = "No contact matches your search.";
pub const SEARCH_PLACEHOLDER: &str = "Search your contacts";
pub const UNHIDE_LABEL: &str = "Unhide";
pub const UNHIDE_TOOLTIP: &str = "Show this contact in your contact list again.";

/// Hidden-section toggle. Shown only when at least one contact is hidden, so the
/// count tells the user there is something to recover.
pub const SHOW_HIDDEN_LABEL: &str = "Show hidden contacts";

/// Why a contact the user never hid by hand can still be in this section:
/// declining or cancelling a request hides that person, and a contact that is
/// later established with them stays hidden until it is unhidden here.
pub const HIDDEN_EXPLAINER: &str = "Hidden contacts are kept out of your list. Declining or cancelling a request also hides that \
     person. Unhide anyone you want back.";

/// Sent-section caption. A DashPay contact request cannot be deleted from the
/// network once sent, so the copy states plainly what Cancel really does
/// instead of promising a withdrawal the protocol cannot deliver.
pub const CANCEL_EXPLAINER: &str = "Cancelling a request hides it and tells the other person you are no longer waiting. The \
     original request stays on the network.";

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
/// `every_contacts_button_maps_to_a_live_action` test enforces this.
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

/// Render one request row and report which request the user acted on.
///
/// The received and sent sections differ only in the card they hand in — the
/// row itself (render, spacing, click reporting) is the same on both, so it
/// lives here once. Dispatch is [`dispatch_request`]'s job, which needs the
/// state this loop only reads.
fn request_row(
    ui: &mut Ui,
    card: RequestCard,
    entry: &ContactRequestEntry,
) -> Option<(Identifier, RequestAction)> {
    let response = card.show(ui);
    ui.add_space(4.0);

    response.action().map(|action| (entry.request_id, action))
}

/// Pair each request with whether its action is already running, so the row loop
/// no longer needs the state it would otherwise have to hold borrowed.
fn request_rows<'a>(
    entries: &'a [ContactRequestEntry],
    state: &ContactsState,
    app_context: &Arc<AppContext>,
) -> Vec<(&'a ContactRequestEntry, bool)> {
    entries
        .iter()
        .map(|entry| {
            (
                entry,
                state.is_in_flight(&entry.request_id)
                    || app_context.contact_request_action_is_in_flight(&entry.request_id),
            )
        })
        .collect()
}

/// Dispatch the backend task for the request row the user clicked, if any.
///
/// Accept, Decline, and Cancel each sign and pay for a state transition, so the
/// request is marked in flight and a further click on it dispatches nothing
/// until its result lands. The row's buttons are disabled meanwhile; this guard
/// is what makes that true rather than merely visible.
fn dispatch_request(
    state: &mut ContactsState,
    identity: &QualifiedIdentity,
    clicked: Option<(Identifier, RequestAction)>,
) -> AppAction {
    let Some((request_id, action)) = clicked else {
        return AppAction::None;
    };
    if !state.begin_request(request_id) {
        return AppAction::None;
    }
    AppAction::BackendTask(BackendTask::DashPayTask(Box::new(request_task(
        action,
        identity.clone(),
        request_id,
    ))))
}

fn dispatch_request_shared(
    app_context: &Arc<AppContext>,
    state: &mut ContactsState,
    identity: &QualifiedIdentity,
    clicked: Option<(Identifier, RequestAction)>,
) -> AppAction {
    if clicked
        .is_some_and(|(request_id, _)| app_context.contact_request_action_is_in_flight(&request_id))
    {
        return AppAction::None;
    }
    dispatch_request(state, identity, clicked)
}

/// One-shot hydration of the tab: the contact list and the request lists.
///
/// Both loads travel as a single [`AppAction::BackendTasks`]. Two separate
/// `AppAction::BackendTask`s would not: `AppAction`'s `|=` is last-writer-wins,
/// so the first load would be dropped on the floor and the active section would
/// never fill.
fn load_action(identity: &QualifiedIdentity, state: &mut ContactsState) -> AppAction {
    if !state.claim_load() {
        return AppAction::None;
    }
    AppAction::BackendTasks(
        vec![
            BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
                identity: identity.clone(),
            })),
            BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
                identity: identity.clone(),
            })),
        ],
        BackendTasksExecutionMode::Concurrent,
    )
}

/// Backend task for a request-card action. Accept and Decline act on a received
/// request; Cancel withdraws a sent one.
pub fn request_task(
    action: RequestAction,
    identity: QualifiedIdentity,
    request_id: Identifier,
) -> DashPayTask {
    match action {
        RequestAction::Accepted => DashPayTask::AcceptContactRequest {
            identity,
            request_id,
        },
        RequestAction::Declined => DashPayTask::RejectContactRequest {
            identity,
            request_id,
            unreadable: UnreadableContactInfoPolicy::Abort,
        },
        RequestAction::Cancelled => DashPayTask::CancelContactRequest {
            identity,
            request_id,
            unreadable: UnreadableContactInfoPolicy::Abort,
        },
    }
}

/// Backend task that makes a hidden contact visible again.
///
/// Unhiding is a `contactInfo` broadcast with `display_hidden` cleared — the
/// same document the decline / cancel paths set it on. The whole document is
/// rewritten, so everything unhiding has no opinion about rides along untouched:
/// the nickname and note are carried over, and the accepted accounts are
/// preserved rather than replaced with an empty list.
pub fn unhide_task(identity: QualifiedIdentity, contact: &ContactData) -> DashPayTask {
    DashPayTask::UpdateContactInfo {
        identity,
        contact_id: contact.identity_id,
        update: ContactInfoUpdate::visibility(false),
    }
}

/// Public entry point invoked by `hub_screen` when the Contacts tab is active.
///
/// Resolves the "current" identity as the app-scoped active identity. When no
/// identity is loaded, or the active identity has no DashPay profile, the gated
/// state is rendered.
///
/// The caller owns a [`ContactsState`] so the populated shell only dispatches
/// its backend tasks once per tab entry — not once per paint.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut ContactsState,
    profiles: &mut super::profile_cache::ProfileCache,
) -> AppAction {
    let tab_state = ContactsTabState::resolve(app_context, profiles);
    render_state(ui, app_context, &tab_state, state)
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
    tab_state: &ContactsTabState,
    state: &mut ContactsState,
) -> AppAction {
    match tab_state {
        ContactsTabState::Gated { handle } => render_gated(ui, handle.as_deref()),
        ContactsTabState::Populated { identity } => {
            render_populated(ui, app_context, identity, state)
        }
    }
}

/// Centered gate card shown when the active identity has no social profile.
///
/// Exposed to integration tests so IT-CONTACTS-01 can mount the gated view
/// without constructing a full `AppContext`.
pub fn render_gated(ui: &mut Ui, handle: Option<&str>) -> AppAction {
    let card = SocialProfileGateCard::new(handle);
    let response = card.show(ui);
    if response.primary_clicked {
        // Route to the Settings tab — that is where the user actually edits
        // display name and avatar (social-profile fields). Resolution goes
        // through the pure `contacts_button_kind` dispatcher so routing lives
        // in one place.
        match contacts_button_kind(ContactsButton::GateSetUpProfile) {
            ContactsButtonKind::SwitchHubTab(tab) => {
                return AppAction::SwitchIdentityHubTab(tab);
            }
            ContactsButtonKind::OpenScreen(_) => {
                // Not possible today (dispatcher returns SwitchHubTab), but
                // exhaustive match future-proofs the gate CTA.
                unreachable!("GateSetUpProfile should not map to OpenScreen");
            }
        }
    }
    AppAction::None
}

/// Populated-state shell — three sections, each rendering live rows, plus a
/// one-shot dispatch of [`DashPayTask::LoadContacts`] +
/// [`DashPayTask::LoadContactRequests`].
fn render_populated(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
) -> AppAction {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut action = AppAction::None;

    action |= header_row(ui, app_context, dark_mode);

    action |= received_section(ui, app_context, identity, state, dark_mode);
    action |= active_section(ui, app_context, identity, state, dark_mode);
    action |= sent_section(ui, app_context, identity, state, dark_mode);

    // Fire LoadContacts + LoadContactRequests once per tab entry. The hub
    // resets the guard in `refresh_on_arrival()` so a tab switch or explicit
    // refresh triggers another load.
    //
    // Only when nothing else claimed this frame: `|=` is last-writer-wins, so a
    // load dispatched alongside a click would swallow one of the two. The load
    // guard is untouched until it actually dispatches, so it simply goes out on
    // the next paint.
    if matches!(action, AppAction::None) {
        action = load_action(identity, state);
    }

    action
}

/// Received requests — one [`RequestCard`] per incoming request, with Accept
/// and Decline wired to their backend tasks.
fn received_section(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    dark_mode: bool,
) -> AppAction {
    ui.add_space(12.0);

    let rows = request_rows(state.incoming(), state, app_context);
    let heading = if rows.is_empty() {
        RECEIVED_HEADING.to_string()
    } else {
        format!("{RECEIVED_HEADING} · {}", rows.len())
    };

    // Collected inside the row loop and acted on after it, so dispatching can
    // take `state` mutably once the loop's read borrow has ended.
    let mut clicked = None;
    section_card(ui, dark_mode, &heading, |ui| {
        if rows.is_empty() {
            ui.label(RichText::new(NO_RECEIVED_EMPTY).color(DashColors::text_secondary(dark_mode)));
            return;
        }
        for (entry, busy) in rows {
            let handle = entry.counterpart_id.to_string(Encoding::Base58);
            let card = RequestCard::received(
                shorten_id(&handle),
                &handle,
                entry.relative_time.as_deref().unwrap_or(""),
            )
            .with_busy(busy);
            clicked = request_row(ui, card, entry).or(clicked);
        }
    });

    dispatch_request_shared(app_context, state, identity, clicked)
}

/// Active contacts — searchable list of established contacts, each row offering
/// the existing contact-profile viewer and, when enabled, the send-payment
/// screen. Pay is an experimental DashPay feature, classified identically at all
/// four entry points into [`ScreenType::DashPaySendPayment`].
fn active_section(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    dark_mode: bool,
) -> AppAction {
    let mut action = AppAction::None;
    let pay_available = FeatureGate::DashPayOperations.is_available(app_context);
    ui.add_space(12.0);

    let heading = format!("{ACTIVE_HEADING_PREFIX} · {}", state.contacts_len());
    let has_contacts = state.contacts_len() > 0;

    section_card(ui, dark_mode, &heading, |ui| {
        // The search box only earns its place once there is something to search.
        if has_contacts {
            ui.add(
                eframe::egui::TextEdit::singleline(state.search_mut())
                    .hint_text(SEARCH_PLACEHOLDER)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(8.0);
        }

        let matches = state.filtered_contacts();
        if !has_contacts {
            ui.label(RichText::new(NO_ACTIVE_EMPTY).color(DashColors::text_secondary(dark_mode)));
        } else if matches.is_empty() {
            ui.label(RichText::new(NO_SEARCH_MATCH).color(DashColors::text_secondary(dark_mode)));
        } else {
            for contact in matches {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(contact_label(contact))
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            if pay_available {
                                let pay = ui
                                    .add(ComponentStyles::secondary_button(PAY_LABEL, dark_mode))
                                    .clickable_tooltip("Send Dash to this contact.");
                                if pay.clicked() {
                                    action = AppAction::AddScreen(
                                        ScreenType::DashPaySendPayment(
                                            identity.clone(),
                                            contact.identity_id,
                                        )
                                        .create_screen(app_context),
                                    );
                                }
                            }

                            let view_profile = ui
                                .add(ComponentStyles::secondary_button(
                                    VIEW_PROFILE_LABEL,
                                    dark_mode,
                                ))
                                .clickable_tooltip("View this contact's public profile.");
                            if view_profile.clicked() {
                                action = AppAction::AddScreen(
                                    contact_profile_screen_type(
                                        identity.clone(),
                                        contact.identity_id,
                                    )
                                    .create_screen(app_context),
                                );
                            }
                        },
                    );
                });
                ui.add_space(4.0);
            }
        }

        ui.add_space(8.0);
        let add = ui
            .add(ComponentStyles::primary_button(ADD_BY_USERNAME_LABEL))
            .clickable_tooltip(
                "Find someone by their Dash username or identity ID and add them as a contact.",
            );
        if add.clicked() {
            action = resolve_contacts_button(ContactsButton::ActiveAddByUsername, app_context);
        }

        action |= hidden_section(ui, identity, state, dark_mode);
    });

    action
}

/// Hidden contacts — the way back for anyone the user hid, declined, or
/// cancelled on. Collapsed behind a toggle, and drawn only when there is
/// something to recover, so it stays out of the way in the common case.
fn hidden_section(
    ui: &mut Ui,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    dark_mode: bool,
) -> AppAction {
    let mut action = AppAction::None;
    if state.hidden_contacts().is_empty() {
        return action;
    }

    ui.add_space(8.0);
    let label = format!("{SHOW_HIDDEN_LABEL} · {}", state.hidden_contacts().len());
    ui.checkbox(state.show_hidden_mut(), label);
    if !state.show_hidden() {
        return action;
    }

    ui.add_space(4.0);
    ui.label(
        RichText::new(HIDDEN_EXPLAINER)
            .small()
            .color(DashColors::text_secondary(dark_mode)),
    );
    ui.add_space(8.0);

    // The clicked contact is cloned out of the list so the unhide can mutate
    // `state` once the read borrow the row loop holds has ended.
    let mut clicked: Option<ContactData> = None;
    for contact in state.hidden_contacts() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(contact_label(contact)).color(DashColors::text_secondary(dark_mode)),
            );
            ui.with_layout(
                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                |ui| {
                    let unhide = ui
                        .add(ComponentStyles::secondary_button(UNHIDE_LABEL, dark_mode))
                        .clickable_tooltip(UNHIDE_TOOLTIP);
                    if unhide.clicked() {
                        clicked = Some(contact.clone());
                    }
                },
            );
        });
        ui.add_space(4.0);
    }

    if let Some(contact) = clicked {
        // Move the row now; the authoritative reload lands when the broadcast
        // confirms.
        state.unhide_contact(&contact.identity_id);
        action = AppAction::BackendTask(BackendTask::DashPayTask(Box::new(unhide_task(
            identity.clone(),
            &contact,
        ))));
    }

    action
}

/// Sent requests — one [`RequestCard`] per pending outgoing request, with
/// Cancel wired to [`DashPayTask::CancelContactRequest`].
fn sent_section(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    dark_mode: bool,
) -> AppAction {
    ui.add_space(12.0);

    let rows = request_rows(state.outgoing(), state, app_context);
    let heading = if rows.is_empty() {
        SENT_HEADING.to_string()
    } else {
        format!("{SENT_HEADING} · {}", rows.len())
    };

    let mut clicked = None;
    section_card(ui, dark_mode, &heading, |ui| {
        if rows.is_empty() {
            ui.label(RichText::new(NO_SENT_EMPTY).color(DashColors::text_secondary(dark_mode)));
            return;
        }

        ui.label(
            RichText::new(CANCEL_EXPLAINER)
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(8.0);

        for (entry, busy) in rows {
            let handle = entry.counterpart_id.to_string(Encoding::Base58);
            let card = RequestCard::sent(shorten_id(&handle), &handle).with_busy(busy);
            clicked = request_row(ui, card, entry).or(clicked);
        }
    });

    dispatch_request_shared(app_context, state, identity, clicked)
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
        ContactsButtonKind::SwitchHubTab(tab) => AppAction::SwitchIdentityHubTab(tab),
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

fn contact_profile_screen_type(identity: QualifiedIdentity, contact_id: Identifier) -> ScreenType {
    ScreenType::DashPayContactProfileViewer(identity, contact_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::collections::BTreeMap;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    fn qualified_identity(identifier: Identifier) -> QualifiedIdentity {
        let identity = Identity::create_basic_identity(identifier, PlatformVersion::latest())
            .expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

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
    fn populated_heading_format_matches_design() {
        // Design-spec §B.4: active-contacts header reads `Active contacts · {n}`.
        let mut state = ContactsState::default();
        assert_eq!(
            format!("{ACTIVE_HEADING_PREFIX} · {}", state.contacts_len()),
            "Active contacts · 0"
        );
        state.record_contacts(vec![crate::backend_task::dashpay::ContactData {
            identity_id: id(5),
            nickname: Some("Bao".into()),
            note: None,
            is_hidden: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }]);
        assert_eq!(
            format!("{ACTIVE_HEADING_PREFIX} · {}", state.contacts_len()),
            "Active contacts · 1",
            "the heading count must reflect the loaded contacts, not a hardcoded zero"
        );
    }

    #[test]
    fn copy_constants_are_complete_sentences() {
        for line in [
            NO_RECEIVED_EMPTY,
            NO_ACTIVE_EMPTY,
            NO_SENT_EMPTY,
            NO_SEARCH_MATCH,
            CANCEL_EXPLAINER,
            HIDDEN_EXPLAINER,
            UNHIDE_TOOLTIP,
        ] {
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
    // Dead-button regression tests — every interactive control MUST
    // produce a live result from a pure dispatcher.
    // ---------------------------------------------------------------

    const ALL_CONTACTS_BUTTONS: &[ContactsButton] = &[
        ContactsButton::HeaderAddByUsername,
        ContactsButton::HeaderScanQr,
        ContactsButton::HeaderShowMyQr,
        ContactsButton::ActiveAddByUsername,
        ContactsButton::GateSetUpProfile,
    ];

    #[test]
    fn every_contacts_button_maps_to_a_live_action() {
        for button in ALL_CONTACTS_BUTTONS {
            let kind = contacts_button_kind(*button);
            // Both variants are live — `OpenScreen` resolves to
            // `AppAction::AddScreen(...)` and `SwitchHubTab` to
            // `AppAction::SwitchIdentityHubTab(...)`.
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

    #[test]
    fn contact_profile_action_targets_the_working_viewer_and_selected_contact() {
        let identity = qualified_identity(id(1));

        assert!(matches!(
            contact_profile_screen_type(identity, id(2)),
            ScreenType::DashPayContactProfileViewer(owner, contact_id)
                if owner.identity.id() == id(1) && contact_id == id(2)
        ));
    }

    // ---------------------------------------------------------------
    // Request-row actions — the dead-button regression these rows had:
    // Accept/Decline were TODO stubs and Cancel had no task at all.
    // ---------------------------------------------------------------

    #[test]
    fn accept_maps_to_the_accept_task_for_that_request() {
        let identity = qualified_identity(id(1));
        let task = request_task(RequestAction::Accepted, identity, id(2));
        match task {
            DashPayTask::AcceptContactRequest {
                identity,
                request_id,
            } => {
                assert_eq!(identity.identity.id(), id(1));
                assert_eq!(
                    request_id,
                    id(2),
                    "the clicked row's request must be acted on"
                );
            }
            other => panic!("Accept must dispatch AcceptContactRequest, got {other:?}"),
        }
    }

    #[test]
    fn decline_maps_to_the_reject_task() {
        let task = request_task(RequestAction::Declined, qualified_identity(id(1)), id(2));
        assert!(
            matches!(task, DashPayTask::RejectContactRequest { request_id, .. } if request_id == id(2)),
            "Decline must dispatch RejectContactRequest for the clicked request"
        );
    }

    #[test]
    fn cancel_maps_to_the_cancel_task() {
        let task = request_task(RequestAction::Cancelled, qualified_identity(id(1)), id(2));
        assert!(
            matches!(task, DashPayTask::CancelContactRequest { request_id, .. } if request_id == id(2)),
            "Cancel must dispatch CancelContactRequest for the clicked request"
        );
    }

    // ---------------------------------------------------------------
    // Unhide — the recovery path for a contact that a decline, a cancel,
    // or a manual hide took off the list.
    // ---------------------------------------------------------------

    fn hidden_contact(nickname: Option<&str>, note: Option<&str>) -> ContactData {
        ContactData {
            identity_id: id(5),
            nickname: nickname.map(str::to_string),
            note: note.map(str::to_string),
            is_hidden: true,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }
    }

    #[test]
    fn unhide_clears_the_hidden_flag_for_that_contact() {
        let task = unhide_task(qualified_identity(id(1)), &hidden_contact(None, None));
        match task {
            DashPayTask::UpdateContactInfo {
                identity,
                contact_id,
                update,
                ..
            } => {
                assert_eq!(identity.identity.id(), id(1));
                assert_eq!(contact_id, id(5), "the clicked contact must be unhidden");
                assert!(
                    !update.display_hidden,
                    "unhiding must broadcast contactInfo with the hidden flag cleared"
                );
            }
            other => panic!("Unhide must dispatch UpdateContactInfo, got {other:?}"),
        }
    }

    #[test]
    fn unhide_preserves_the_nickname_and_note() {
        let task = unhide_task(
            qualified_identity(id(1)),
            &hidden_contact(Some("Bao"), Some("Met at the meetup")),
        );
        match task {
            DashPayTask::UpdateContactInfo { update, .. } => {
                assert_eq!(
                    update.nickname,
                    crate::model::dashpay::ContactInfoField::Preserve
                );
                assert_eq!(
                    update.note,
                    crate::model::dashpay::ContactInfoField::Preserve
                );
            }
            other => panic!("expected UpdateContactInfo, got {other:?}"),
        }
    }

    #[test]
    fn unhide_leaves_the_accepted_accounts_alone() {
        // The write replaces the whole contactInfo document. Unhiding says
        // nothing about which accounts the user accepted, so it must not
        // volunteer an empty list — that would erase every one of them.
        let task = unhide_task(qualified_identity(id(1)), &hidden_contact(None, None));
        match task {
            DashPayTask::UpdateContactInfo { update, .. } => assert_eq!(
                update.accepted_accounts,
                crate::model::dashpay::AcceptedAccounts::Preserve,
                "unhiding must preserve the contact's accepted accounts, not overwrite them"
            ),
            other => panic!("expected UpdateContactInfo, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Hydration — both loads must survive the trip to `AppState`.
    // `AppAction`'s `|=` is last-writer-wins, so two separate
    // `BackendTask` actions in one frame mean the first is silently lost.
    // ---------------------------------------------------------------

    #[test]
    fn hydration_dispatches_both_loads_in_one_action() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();

        let AppAction::BackendTasks(tasks, mode) = load_action(&identity, &mut state) else {
            panic!("hydration must dispatch its loads as one multi-task action");
        };

        assert_eq!(mode, BackendTasksExecutionMode::Concurrent);
        assert!(
            tasks.iter().any(|t| matches!(
                t,
                BackendTask::DashPayTask(task) if matches!(**task, DashPayTask::LoadContacts { .. })
            )),
            "the contact list must be loaded — without it the active section stays empty"
        );
        assert!(
            tasks.iter().any(|t| matches!(
                t,
                BackendTask::DashPayTask(task)
                    if matches!(**task, DashPayTask::LoadContactRequests { .. })
            )),
            "the request lists must be loaded"
        );
    }

    #[test]
    fn hydration_fires_once_per_tab_entry() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();

        assert_ne!(load_action(&identity, &mut state), AppAction::None);
        assert_eq!(
            load_action(&identity, &mut state),
            AppAction::None,
            "a second paint must not re-dispatch the loads"
        );
    }

    // ---------------------------------------------------------------
    // In-flight guard — Accept / Decline / Cancel each sign and pay for a
    // state transition, so a second click while the first is running must
    // not buy a second one.
    // ---------------------------------------------------------------

    #[test]
    fn a_second_click_while_the_request_is_in_flight_dispatches_nothing() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();
        let clicked = Some((id(2), RequestAction::Accepted));

        assert_ne!(
            dispatch_request(&mut state, &identity, clicked),
            AppAction::None,
            "the first click must dispatch"
        );
        assert!(
            state.is_in_flight(&id(2)),
            "the row must be marked in flight"
        );
        assert_eq!(
            dispatch_request(&mut state, &identity, clicked),
            AppAction::None,
            "a second click must not pay for a second state transition"
        );
    }

    #[test]
    fn another_request_stays_clickable_while_one_is_in_flight() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();

        dispatch_request(
            &mut state,
            &identity,
            Some((id(2), RequestAction::Accepted)),
        );

        assert_ne!(
            dispatch_request(
                &mut state,
                &identity,
                Some((id(3), RequestAction::Declined))
            ),
            AppAction::None,
            "the guard is per request — an unrelated row must still act"
        );
    }

    #[test]
    fn a_failed_request_becomes_clickable_again() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();
        let clicked = Some((id(2), RequestAction::Cancelled));

        dispatch_request(&mut state, &identity, clicked);
        state.release_request(&id(2));

        assert_ne!(
            dispatch_request(&mut state, &identity, clicked),
            AppAction::None,
            "a failed action must leave the row actionable, not stuck forever"
        );
    }

    #[test]
    fn a_resolved_request_releases_its_guard() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();

        dispatch_request(
            &mut state,
            &identity,
            Some((id(2), RequestAction::Accepted)),
        );
        state.remove_request(&id(2));

        assert!(
            !state.is_in_flight(&id(2)),
            "a resolved request must not hold its guard"
        );
    }

    #[test]
    fn every_request_action_maps_to_a_distinct_task() {
        let tasks: Vec<DashPayTask> = [
            RequestAction::Accepted,
            RequestAction::Declined,
            RequestAction::Cancelled,
        ]
        .into_iter()
        .map(|a| request_task(a, qualified_identity(id(1)), id(2)))
        .collect();

        for (i, a) in tasks.iter().enumerate() {
            for b in tasks.iter().skip(i + 1) {
                assert_ne!(
                    a, b,
                    "each request action must dispatch its own task, never a shared one"
                );
            }
        }
    }
}
