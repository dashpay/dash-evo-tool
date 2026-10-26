//! Contacts tab.
//!
//! Renders either the populated Contacts page (received requests · active
//! contacts · sent requests) or the social-profile gate card when the
//! currently-active identity has no DashPay profile yet. See design-spec §B.4
//! and §B.4.1.
//!
//! The three sections show the identity's DashPay snapshot, which
//! platform-wallet's background sync keeps current, through the
//! [`ContactsState`] view-model.
//!
//! Row actions dispatch the DashPay backend tasks directly: Accept and Decline
//! on a received request, Cancel on a sent one, and Pay on an established
//! contact (which opens the existing send-payment screen). Clicking an
//! established contact shows it in the detail pane beside the list (under it on
//! a narrow window), with its profile and a private note saved encrypted on
//! Dash Platform.

use super::request_card::{RequestAction, RequestCard};
use super::social_profile_gate_card::SocialProfileGateCard;
use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::dashpay::{ContactData, DashPayTask};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::dashpay::{
    ContactInfoUpdate, ContactRequestKey, MAX_CONTACT_NOTE_BYTES, validate_contact_note,
};
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::ScreenType;
use crate::ui::components::avatar::Avatar;
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::state::AvatarCache;
use crate::ui::state::contacts_view::contact_label;
use crate::ui::state::hub_profiles::HubProfiles;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape, Spacing};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{CornerRadius, Frame, Label, Margin, RichText, Stroke, TextEdit, Ui};
use std::sync::Arc;

pub use crate::ui::state::contacts_view::{ContactRequestEntry, ContactsState};

/// Copy constants, kept public so tests and sibling callsites share a single
/// source of truth. Complete sentences with no positional assumptions so
/// future i18n extraction is one line per string.
pub const ADD_BY_USERNAME_LABEL: &str = "Add by username";
pub const SCAN_QR_LABEL: &str = "Scan QR";
pub const SHOW_MY_QR_LABEL: &str = "Show my QR";
pub const PAY_LABEL: &str = "Pay";
pub const SELECT_CONTACT_TOOLTIP: &str = "Show this contact's details.";
pub const DETAIL_HEADING: &str = "Contact details";
pub const NO_SELECTION: &str = "Select a contact to see their details.";
pub const NOTE_HEADING: &str = "Private note";
pub const NOTE_HINT: &str = "Only you can see this note. It is stored encrypted on Dash Platform.";
pub const NOTE_PLACEHOLDER: &str = "Add a note that only you can see";
pub const SAVE_NOTE_LABEL: &str = "Save note";
pub const SAVING_NOTE_LABEL: &str = "Saving…";
pub const NOTE_UNCHANGED_TOOLTIP: &str = "There are no changes to save.";
pub const NOTE_SAVE_TOOLTIP: &str = "Save this note on Dash Platform. Saving costs a small fee.";
pub const NOTE_SAVED: &str = "Your note is saved.";
/// Shown when contact details are kept on this device only: DIP-0015 holds
/// them back until the identity has two contacts, and nothing publishes them
/// later on its own.
pub const DETAILS_SAVED_ON_THIS_DEVICE: &str = "Your contact details are saved on this device only. After this identity has two contacts, edit and save these details again to sync them to your other devices.";
const NOTE_SAVING_TOOLTIP: &str = "This contact's details are being saved.";
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

/// Hidden-section caption. Only an explicit hide puts a contact here;
/// declining or cancelling a request writes no contact details.
pub const HIDDEN_EXPLAINER: &str =
    "Hidden contacts are kept out of your list. Unhide anyone you want back.";

/// Sent-section caption. A DashPay contact request cannot be deleted from the
/// network once sent, and Cancel publishes nothing, so the copy states plainly
/// that it only hides the request on this device.
pub const CANCEL_EXPLAINER: &str = "Cancelling a request hides it on this device only. The other person is not notified, and the original request stays on the network.";

/// Below this tab width the detail pane stacks under the contact list. Home
/// and Profile switch to one column at the same width.
const TWO_COLUMN_MIN_WIDTH: f32 = 760.0;
/// Share of the two-column width the contact list takes.
const LIST_SHARE: f32 = 0.55;
const DETAIL_AVATAR_SIZE: f32 = 64.0;

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
) -> Option<(ContactRequestKey, RequestAction)> {
    let response = card.show(ui);
    ui.add_space(4.0);

    response.action().map(|action| (entry.request, action))
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
                state.is_in_flight(&entry.request)
                    || app_context.contact_request_action_is_in_flight(&entry.request),
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
    clicked: Option<(ContactRequestKey, RequestAction)>,
) -> AppAction {
    let Some((request, action)) = clicked else {
        return AppAction::None;
    };
    // A row of another identity, left on screen for a frame after a switch.
    if request.owner != identity.identity.id() || !state.begin_request(request) {
        return AppAction::None;
    }
    AppAction::BackendTask(BackendTask::DashPayTask(Box::new(request_task(
        action,
        identity.clone(),
        request.counterparty,
    ))))
}

fn dispatch_request_shared(
    app_context: &Arc<AppContext>,
    state: &mut ContactsState,
    identity: &QualifiedIdentity,
    clicked: Option<(ContactRequestKey, RequestAction)>,
) -> AppAction {
    if clicked.is_some_and(|(request, _)| app_context.contact_request_action_is_in_flight(&request))
    {
        return AppAction::None;
    }
    dispatch_request(state, identity, clicked)
}

/// Backend task for a request-card action. Accept and Decline act on the
/// request `counterparty_id` sent; Cancel withdraws the one sent to them.
pub fn request_task(
    action: RequestAction,
    identity: QualifiedIdentity,
    counterparty_id: Identifier,
) -> DashPayTask {
    match action {
        RequestAction::Accepted => DashPayTask::AcceptContactRequest {
            identity,
            counterparty_id,
        },
        RequestAction::Declined => DashPayTask::RejectContactRequest {
            identity,
            counterparty_id,
        },
        RequestAction::Cancelled => DashPayTask::CancelContactRequest {
            identity,
            counterparty_id,
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

/// Backend task that stores `note` as the private note on `contact`, keeping
/// its nickname, accepted accounts and hidden state.
pub fn save_note_task(
    identity: QualifiedIdentity,
    contact: &ContactData,
    note: Option<String>,
) -> DashPayTask {
    DashPayTask::UpdateContactInfo {
        identity,
        contact_id: contact.identity_id,
        update: ContactInfoUpdate::note(note),
    }
}

/// The fee line shown beside Save note.
pub fn note_fee_text(fee: &str) -> String {
    format!("Estimated fee for saving this note: up to {fee}.")
}

/// Dispatch the save of the selected contact's note draft, if it has a valid
/// change. A save already running for that contact dispatches nothing: each
/// save is a paid write.
fn dispatch_note_save(state: &mut ContactsState, identity: &QualifiedIdentity) -> AppAction {
    let Some(contact) = state.selected_contact().cloned() else {
        return AppAction::None;
    };
    let detail = state.detail_mut();
    let valid = detail
        .note_to_save(&contact.identity_id)
        .is_none_or(|note| validate_contact_note(&note).is_ok());
    if !valid || !detail.is_dirty(&contact.identity_id) {
        return AppAction::None;
    }
    let Some(note) = detail.begin_save(identity.identity.id(), contact.identity_id) else {
        return AppAction::None;
    };
    AppAction::BackendTask(BackendTask::DashPayTask(Box::new(save_note_task(
        identity.clone(),
        &contact,
        note,
    ))))
}

/// Dispatch the unhide of `contact`. Unhiding rewrites the contact's whole
/// `contactInfo`, so it waits for any other write to that contact to finish.
fn dispatch_unhide(
    state: &mut ContactsState,
    identity: &QualifiedIdentity,
    contact: &ContactData,
) -> AppAction {
    if !state
        .detail_mut()
        .begin_visibility_write(identity.identity.id(), contact.identity_id)
    {
        return AppAction::None;
    }
    // Move the row now; the authoritative reload lands when the broadcast
    // confirms.
    state.unhide_contact(&contact.identity_id);
    AppAction::BackendTask(BackendTask::DashPayTask(Box::new(unhide_task(
        identity.clone(),
        contact,
    ))))
}

/// Public entry point invoked by `hub_screen` when the Contacts tab is active.
///
/// Resolves the "current" identity as the app-scoped active identity. When no
/// identity is loaded, or the active identity has no DashPay profile, the gated
/// state is rendered.
///
/// The caller owns the [`ContactsState`].
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: Option<&QualifiedIdentity>,
    state: &mut ContactsState,
    profiles: &HubProfiles,
    avatars: &mut AvatarCache,
    pending_avatars: &mut Vec<BackendTask>,
) -> AppAction {
    let tab_state = ContactsTabState::resolve(app_context, identity, profiles);
    let mut avatars = DetailAvatars {
        cache: avatars,
        pending: pending_avatars,
    };
    render_state(ui, app_context, &tab_state, state, &mut avatars)
}

/// The hub's avatar cache and the fetches the detail pane asks it for.
struct DetailAvatars<'a> {
    cache: &'a mut AvatarCache,
    pending: &'a mut Vec<BackendTask>,
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
        /// The currently-active identity, whose lists the shell shows.
        identity: Box<QualifiedIdentity>,
    },
}

impl ContactsTabState {
    /// Inspect the app context to decide which state to render. Returns
    /// `Gated` if no identity is loaded, if loading fails, or if the active
    /// identity has no DashPay profile (or one has not loaded yet).
    pub fn resolve(
        app_context: &Arc<AppContext>,
        active: Option<&QualifiedIdentity>,
        profiles: &HubProfiles,
    ) -> Self {
        // `active` is the app-scoped active identity (selected → first → none).
        // `None` on a load error or no identities falls back to Gated so the
        // hub never draws a half-broken populated UI.
        let Some(active) = active else {
            return ContactsTabState::Gated { handle: None };
        };

        let handle = app_context.main_username(active);
        if has_social_profile(profiles, active) {
            ContactsTabState::Populated {
                identity: Box::new(active.clone()),
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
    avatars: &mut DetailAvatars<'_>,
) -> AppAction {
    match tab_state {
        ContactsTabState::Gated { handle } => render_gated(ui, handle.as_deref()),
        ContactsTabState::Populated { identity } => {
            render_populated(ui, app_context, identity, state, avatars)
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

/// Populated-state shell — the three sections and the selected contact's
/// detail pane: beside them on a wide tab, under the active contacts on a
/// narrow one.
fn render_populated(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    avatars: &mut DetailAvatars<'_>,
) -> AppAction {
    state.reconcile_owner(identity);
    state.apply_snapshot(app_context.dashpay_snapshot(&identity.identity.id()));
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut action = AppAction::None;

    action |= header_row(ui, app_context, dark_mode);

    // Everything scrolls below the header, so a long contact list stays
    // reachable in a short window. The two columns scroll on their own, so the
    // details stay in view while the list is scrolled.
    if ui.available_width() >= TWO_COLUMN_MIN_WIDTH {
        let content_width = ui.available_width() - Spacing::LG;
        let list_width = content_width * LIST_SHARE;
        let detail_width = content_width - list_width;
        // The column gap applies between the columns only, not inside them.
        let item_spacing = ui.spacing().item_spacing;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = Spacing::LG;
            let height = ui.available_height();
            ui.allocate_ui_with_layout(
                eframe::egui::vec2(list_width, height),
                eframe::egui::Layout::top_down(eframe::egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing = item_spacing;
                    ui.set_width(list_width);
                    eframe::egui::ScrollArea::vertical()
                        .id_salt("identity_hub_contacts_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            action |= received_section(ui, app_context, identity, state, dark_mode);
                            action |= active_section(ui, app_context, identity, state, dark_mode);
                            action |= sent_section(ui, app_context, identity, state, dark_mode);
                        });
                },
            );
            ui.allocate_ui_with_layout(
                eframe::egui::vec2(detail_width, height),
                eframe::egui::Layout::top_down(eframe::egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing = item_spacing;
                    ui.set_width(detail_width);
                    eframe::egui::ScrollArea::vertical()
                        .id_salt("identity_hub_contact_detail")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            action |=
                                detail_pane(ui, app_context, identity, state, avatars, dark_mode);
                        });
                },
            );
        });
    } else {
        eframe::egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                action |= received_section(ui, app_context, identity, state, dark_mode);
                action |= active_section(ui, app_context, identity, state, dark_mode);
                // Stacked, the empty pane would only push the sent requests down.
                if state.selected_contact().is_some() {
                    action |= detail_pane(ui, app_context, identity, state, avatars, dark_mode);
                }
                action |= sent_section(ui, app_context, identity, state, dark_mode);
            });
    }
    action
}

/// Open the send-payment screen for `contact_id`.
fn pay_action(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    contact_id: Identifier,
) -> AppAction {
    AppAction::AddScreen(
        ScreenType::DashPaySendPayment(identity.clone(), contact_id).create_screen(app_context),
    )
}

/// The selected contact's profile, its Pay action, and its private note.
fn detail_pane(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    avatars: &mut DetailAvatars<'_>,
    dark_mode: bool,
) -> AppAction {
    let mut action = AppAction::None;
    let mut save_clicked = false;
    ui.add_space(12.0);

    section_card(ui, dark_mode, DETAIL_HEADING, |ui| {
        let Some(contact) = state.selected_contact().cloned() else {
            ui.label(RichText::new(NO_SELECTION).color(DashColors::text_secondary(dark_mode)));
            return;
        };
        let label = contact_label(&contact);

        ui.horizontal(|ui| {
            match contact
                .avatar_url
                .as_deref()
                .filter(|url| !url.is_empty() && !avatars.cache.is_failed(url))
            {
                Some(url) => {
                    if let Some(task) = Avatar::new(Some(url), DETAIL_AVATAR_SIZE)
                        .show(ui, avatars.cache)
                        .fetch
                    {
                        avatars.pending.push(task);
                    }
                }
                None => {
                    super::avatar::paint_identity_monogram(
                        ui,
                        DETAIL_AVATAR_SIZE,
                        HeroIdentityKind::User,
                        label.chars().next(),
                        DashColors::DASH_BLUE,
                    );
                }
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.add(
                    Label::new(
                        RichText::new(&label)
                            .strong()
                            .size(18.0)
                            .color(DashColors::text_primary(dark_mode)),
                    )
                    .truncate(),
                );
                if let Some(handle) = contact_handle(&contact, &label) {
                    ui.add(
                        Label::new(
                            RichText::new(handle).color(DashColors::text_secondary(dark_mode)),
                        )
                        .truncate(),
                    );
                }
            });
        });

        if let Some(bio) = contact
            .bio
            .as_deref()
            .map(str::trim)
            .filter(|bio| !bio.is_empty())
        {
            ui.add_space(8.0);
            ui.label(RichText::new(bio).color(DashColors::text_primary(dark_mode)));
        }

        if FeatureGate::DashPayOperations.is_available(app_context) {
            ui.add_space(8.0);
            let pay = ComponentStyles::add_secondary_button(ui, PAY_LABEL, dark_mode)
                .clickable_tooltip("Send Dash to this contact.");
            if pay.clicked() {
                action = pay_action(app_context, identity, contact.identity_id);
            }
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        save_clicked = note_editor(ui, app_context, identity, state, &contact, dark_mode);
    });

    if save_clicked {
        action = dispatch_note_save(state, identity);
    }
    action
}

/// The private-note editor, its size and fee, and Save note. Returns whether
/// Save note was clicked while it could save.
fn note_editor(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    contact: &ContactData,
    dark_mode: bool,
) -> bool {
    let contact_id = contact.identity_id;
    let saving = state
        .detail()
        .is_writing(&identity.identity.id(), &contact_id);
    let detail = state.detail_mut();
    let Some(note) = detail.note_mut(&contact_id) else {
        return false;
    };

    ui.label(
        RichText::new(NOTE_HEADING)
            .strong()
            .color(DashColors::text_primary(dark_mode)),
    );
    ui.label(
        RichText::new(NOTE_HINT)
            .small()
            .color(DashColors::text_secondary(dark_mode)),
    );
    ui.add_space(6.0);
    ui.add_enabled(
        !saving,
        TextEdit::multiline(note)
            .hint_text(NOTE_PLACEHOLDER)
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );

    let to_save = detail.note_to_save(&contact_id);
    let note_bytes = to_save.as_deref().map_or(0, str::len);
    super::settings::counter(ui, note_bytes, MAX_CONTACT_NOTE_BYTES, dark_mode);
    let invalid = to_save
        .as_deref()
        .and_then(|note| validate_contact_note(note).err());
    if let Some(error) = invalid {
        ui.label(RichText::new(error.to_string()).color(DashColors::ERROR));
    }

    ui.add_space(8.0);
    let fee = app_context.fee_estimator().estimate_contact_info_write();
    ui.label(
        RichText::new(note_fee_text(&format_credits_as_dash(fee)))
            .small()
            .color(DashColors::text_secondary(dark_mode)),
    );
    ui.add_space(6.0);

    let dirty = detail.is_dirty(&contact_id);
    let can_save = dirty && invalid.is_none() && !saving;
    let label = if saving {
        SAVING_NOTE_LABEL
    } else {
        SAVE_NOTE_LABEL
    };
    let save = ComponentStyles::add_primary_button_enabled(ui, can_save, label);
    let save = if can_save {
        save.clickable_tooltip(NOTE_SAVE_TOOLTIP)
    } else if saving {
        save.disabled_tooltip(NOTE_SAVING_TOOLTIP)
    } else if let Some(error) = invalid {
        save.disabled_tooltip(error.to_string())
    } else {
        save.disabled_tooltip(NOTE_UNCHANGED_TOOLTIP)
    };
    save.clicked() && can_save
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
            let handle = entry.counterpart_id().to_string(Encoding::Base58);
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

/// Active contacts — searchable list of established contacts. Clicking a contact
/// shows it in the detail pane; each row also offers, when enabled, the
/// send-payment screen. Pay is an experimental DashPay feature, classified
/// identically at every entry point into [`ScreenType::DashPaySendPayment`].
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
    let selected = state.detail().selected();
    let mut clicked_contact = None;

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
                let label = contact_label(contact);
                // The actions are laid out first so a long name or handle is
                // shortened to the room they leave instead of running under
                // them; hovering shows the full text.
                eframe::egui::Sides::new().shrink_left().truncate().show(
                    ui,
                    |ui| {
                        ui.vertical(|ui| {
                            let row = ui
                                .add(eframe::egui::Button::selectable(
                                    selected == Some(contact.identity_id),
                                    RichText::new(&label)
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                ))
                                .clickable_tooltip(SELECT_CONTACT_TOOLTIP);
                            if row.clicked() {
                                clicked_contact = Some(contact.identity_id);
                            }
                            if let Some(handle) = contact_handle(contact, &label) {
                                // Line the handle up with the name, which the
                                // row button insets by its padding.
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    ui.add_space(ui.spacing().button_padding.x);
                                    ui.label(
                                        RichText::new(handle)
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                });
                            }
                        });
                    },
                    |ui| {
                        if pay_available {
                            let pay = ui
                                .add(ComponentStyles::secondary_button(PAY_LABEL, dark_mode))
                                .clickable_tooltip("Send Dash to this contact.");
                            if pay.clicked() {
                                action = pay_action(app_context, identity, contact.identity_id);
                            }
                        }
                    },
                );
                ui.add_space(4.0);
            }
        }

        // The header already offers Add by username; the section repeats it
        // only as the empty state's call to action (design-spec §B.4).
        if !has_contacts {
            ui.add_space(8.0);
            let add = ui
                .add(ComponentStyles::primary_button(ADD_BY_USERNAME_LABEL))
                .clickable_tooltip(
                    "Find someone by their Dash username or identity ID and add them as a contact.",
                );
            if add.clicked() {
                action = resolve_contacts_button(ContactsButton::ActiveAddByUsername, app_context);
            }
        }

        action |= hidden_section(ui, identity, state, dark_mode);
    });

    if let Some(contact_id) = clicked_contact {
        state.select_contact(&contact_id);
    }
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
    let owner = identity.identity.id();
    for contact in state.hidden_contacts() {
        // A write to this contact is still running; the unhide waits for it.
        let busy = state.detail().is_writing(&owner, &contact.identity_id);
        // Unhide is laid out first so a long name is shortened to the room it
        // leaves instead of pushing the button past the content column.
        eframe::egui::Sides::new().shrink_left().truncate().show(
            ui,
            |ui| {
                ui.label(
                    RichText::new(contact_label(contact))
                        .color(DashColors::text_secondary(dark_mode)),
                );
            },
            |ui| {
                let unhide = ui
                    .add_enabled(
                        !busy,
                        ComponentStyles::secondary_button(UNHIDE_LABEL, dark_mode),
                    )
                    .clickable_tooltip(UNHIDE_TOOLTIP);
                if unhide.clicked() {
                    clicked = Some(contact.clone());
                }
            },
        );
        ui.add_space(4.0);
    }

    if let Some(contact) = clicked {
        action = dispatch_unhide(state, identity, &contact);
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
            let handle = entry.counterpart_id().to_string(Encoding::Base58);
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
        // Every section spans the content width, so the cards line up.
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(heading)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(8.0);
        body(ui);
    });
}

/// Detect whether the identity has a DashPay social profile. A profile not
/// synced yet, or none published, yields `false`, so the gate is shown until
/// a profile is confirmed to exist.
fn has_social_profile(profiles: &HubProfiles, identity: &QualifiedIdentity) -> bool {
    matches!(profiles.get(identity), Some(Some(_)))
}

/// A contact's `@username`, shown under its label unless the label already
/// is that username.
fn contact_handle(contact: &ContactData, label: &str) -> Option<String> {
    let username = contact.username.as_deref()?.trim();
    if username.is_empty() {
        return None;
    }
    let handle = format!("@{}", username.trim_start_matches('@'));
    (handle != label && username != label).then_some(handle)
}

/// Compact Home preview; request actions share the Contacts tab's paid-action guards.
pub(super) fn render_preview(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut ContactsState,
    outcome: &mut super::home::HomeOutcome,
) -> AppAction {
    state.reconcile_owner(identity);
    state.apply_snapshot(app_context.dashpay_snapshot(&identity.identity.id()));
    let mut action = AppAction::None;
    super::home::card(ui, |ui| {
        ui.strong("Contacts");
        let mut clicked = None;
        for entry in state.incoming().iter().take(3) {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} wants to connect.",
                    shorten_id(&entry.counterpart_id().to_string(Encoding::Base58))
                ));
                let busy = state.is_in_flight(&entry.request)
                    || app_context.contact_request_action_is_in_flight(&entry.request);
                if ui
                    .add_enabled(
                        !busy,
                        ComponentStyles::secondary_button("Accept", ui.visuals().dark_mode),
                    )
                    .clicked()
                {
                    clicked = Some((entry.request, RequestAction::Accepted));
                }
            });
        }
        for contact in state.matching_contacts().take(6) {
            if ui.button(contact_label(contact)).clicked() {
                *outcome = super::home::HomeOutcome::GoToContact(contact.identity_id);
            }
        }
        if !state.contacts_known() {
            ui.label("Loading contacts…");
        } else if state.contacts_len() == 0 && state.incoming().is_empty() {
            ui.label("No contacts yet. Add someone by their username.");
        }
        if ui.button("See all contacts").clicked() {
            *outcome = super::home::HomeOutcome::GoToContacts;
        }
        if ComponentStyles::add_secondary_button(ui, "Add contact", ui.visuals().dark_mode)
            .clicked()
        {
            action = resolve_contacts_button(ContactsButton::HeaderAddByUsername, app_context);
        }
        action |= dispatch_request_shared(app_context, state, identity, clicked);
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dashpay::ContactInfoField;
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
        state.record_contacts_for_test(vec![crate::backend_task::dashpay::ContactData {
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

    fn contact_named(nickname: &str) -> ContactData {
        ContactData {
            identity_id: id(5),
            nickname: Some(nickname.into()),
            note: None,
            is_hidden: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }
    }

    /// A contact row adds the `@username` under its label, unless the label
    /// already is the username or there is none.
    #[test]
    fn contact_rows_show_the_username_once() {
        let mut contact = contact_named("Bao");
        assert_eq!(contact_handle(&contact, "Bao"), None);

        contact.username = Some("bao".into());
        assert_eq!(contact_handle(&contact, "Bao"), Some("@bao".into()));

        contact.nickname = None;
        let label = contact_label(&contact);
        assert_eq!(contact_handle(&contact, &label), None, "label {label}");

        contact.username = Some("  ".into());
        assert_eq!(contact_handle(&contact, "Bao"), None);
    }

    /// The request between the owner `id(1)` and `id(counterparty)`.
    fn req(counterparty: u8) -> ContactRequestKey {
        ContactRequestKey {
            owner: id(1),
            counterparty: id(counterparty),
        }
    }

    /// The selection can change without the hub resetting this state. The lists
    /// must then follow the identity on screen, while an action already paid
    /// for and still running for the previous identity keeps its guard.
    #[test]
    fn lists_follow_the_identity_and_running_request_guards_survive_the_change() {
        let first = qualified_identity(id(1));
        let second = qualified_identity(id(2));
        let request = req(3);
        let mut state = ContactsState::default();

        state.reconcile_owner(&first);
        state.apply_snapshot(Some(Arc::new(crate::model::dashpay::DashPaySnapshot {
            contacts: vec![contact_named("Bao")],
            incoming: vec![crate::model::dashpay::PendingContactRequest {
                counterparty_id: id(3),
                created_at_ms: 0,
                account_reference: 0,
                has_auto_accept_proof: false,
                username: None,
                display_name: None,
                avatar_url: None,
            }],
            ..Default::default()
        })));
        assert!(state.begin_request(request));

        state.reconcile_owner(&first);
        assert_eq!(state.contacts_len(), 1, "the same identity keeps its lists");

        state.reconcile_owner(&second);
        assert_eq!(state.contacts_len(), 0);
        assert!(state.incoming().is_empty());
        assert!(
            state.is_in_flight(&request),
            "the action still running for the previous identity keeps its guard"
        );
        assert!(
            !state.begin_request(request),
            "the same request must not be acted on a second time"
        );
    }

    // ---------------------------------------------------------------
    // Private note — a paid, encrypted contactInfo write.
    // ---------------------------------------------------------------

    fn contact_with_note(note: Option<&str>) -> ContactData {
        ContactData {
            note: note.map(str::to_string),
            ..contact_named("Bao")
        }
    }

    /// A state bound to `identity` with Bao listed and selected.
    fn state_with_selected_contact(
        identity: &QualifiedIdentity,
        note: Option<&str>,
    ) -> ContactsState {
        let mut state = ContactsState::default();
        state.reconcile_owner(identity);
        state.record_contacts_for_test(vec![contact_with_note(note)]);
        state.select_contact(&id(5));
        state
    }

    fn saved_note(action: AppAction) -> (Identifier, Identifier, ContactInfoUpdate) {
        let AppAction::BackendTask(BackendTask::DashPayTask(task)) = action else {
            panic!("expected a contactInfo write, got {action:?}");
        };
        match *task {
            DashPayTask::UpdateContactInfo {
                identity,
                contact_id,
                update,
            } => (identity.identity.id(), contact_id, update),
            other => panic!("expected UpdateContactInfo, got {other:?}"),
        }
    }

    #[test]
    fn saving_a_note_replaces_only_the_note_of_that_contact() {
        let mut contact = contact_with_note(None);
        contact.is_hidden = true;
        let task = save_note_task(qualified_identity(id(1)), &contact, Some("Bao".into()));
        let (owner, contact_id, update) = saved_note(AppAction::BackendTask(
            BackendTask::DashPayTask(Box::new(task)),
        ));
        assert_eq!((owner, contact_id), (id(1), id(5)));
        assert_eq!(update, ContactInfoUpdate::note(Some("Bao".into())));
    }

    #[test]
    fn save_note_sends_the_edited_note_once() {
        let identity = qualified_identity(id(1));
        let mut state = state_with_selected_contact(&identity, Some("Old"));
        *state.detail_mut().note_mut(&id(5)).expect("draft") = "  Met at the meetup\n".into();

        let (owner, contact_id, update) = saved_note(dispatch_note_save(&mut state, &identity));
        assert_eq!((owner, contact_id), (id(1), id(5)));
        assert_eq!(
            update.note,
            crate::model::dashpay::ContactInfoField::Replace(Some("Met at the meetup".into()))
        );
        assert!(state.detail().is_writing(&id(1), &id(5)));
        assert_eq!(
            dispatch_note_save(&mut state, &identity),
            AppAction::None,
            "a second click must not pay for a second write"
        );
    }

    #[test]
    fn clearing_a_note_saves_no_note() {
        let identity = qualified_identity(id(1));
        let mut state = state_with_selected_contact(&identity, Some("Old"));
        state.detail_mut().note_mut(&id(5)).expect("draft").clear();

        let (_, _, update) = saved_note(dispatch_note_save(&mut state, &identity));
        assert_eq!(
            update.note,
            crate::model::dashpay::ContactInfoField::Replace(None)
        );
    }

    #[test]
    fn an_unchanged_or_too_long_note_is_not_saved() {
        let identity = qualified_identity(id(1));
        let mut state = state_with_selected_contact(&identity, Some("Old"));
        assert_eq!(dispatch_note_save(&mut state, &identity), AppAction::None);

        *state.detail_mut().note_mut(&id(5)).expect("draft") = "a".repeat(256);
        assert_eq!(dispatch_note_save(&mut state, &identity), AppAction::None);
        assert!(!state.detail().is_writing(&id(1), &id(5)));

        let mut unselected = ContactsState::default();
        unselected.reconcile_owner(&identity);
        assert_eq!(
            dispatch_note_save(&mut unselected, &identity),
            AppAction::None
        );
    }

    fn hidden_bao() -> ContactData {
        ContactData {
            is_hidden: true,
            ..contact_with_note(None)
        }
    }

    /// An unhide and a note save both rewrite the same contactInfo document,
    /// so they run one at a time: the second would otherwise race the first
    /// and the hub could not tell which write a result settles.
    #[test]
    fn a_note_save_waits_for_a_running_unhide_of_that_contact() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();
        state.reconcile_owner(&identity);
        state.record_contacts_for_test(vec![hidden_bao()]);

        let (_, contact_id, update) =
            saved_note(dispatch_unhide(&mut state, &identity, &hidden_bao()));
        assert_eq!(contact_id, id(5));
        assert_eq!(update.display_hidden, ContactInfoField::Replace(false));
        assert_eq!(
            dispatch_unhide(&mut state, &identity, &hidden_bao()),
            AppAction::None,
            "a second unhide must not pay for a second write"
        );

        state.select_contact(&id(5));
        *state.detail_mut().note_mut(&id(5)).expect("draft") = "Bao".into();
        assert_eq!(
            dispatch_note_save(&mut state, &identity),
            AppAction::None,
            "the note save must wait for the unhide"
        );

        assert!(state.finish_contact_info_write(Network::Testnet, &id(1), &id(5)));
        assert_eq!(
            state.selected_contact().and_then(|c| c.note.as_deref()),
            None,
            "an unhide stores no note"
        );
        assert_ne!(dispatch_note_save(&mut state, &identity), AppAction::None);
    }

    #[test]
    fn an_unhide_waits_for_a_running_note_save_of_that_contact() {
        let identity = qualified_identity(id(1));
        let mut state = state_with_selected_contact(&identity, None);
        *state.detail_mut().note_mut(&id(5)).expect("draft") = "Bao".into();
        assert_ne!(dispatch_note_save(&mut state, &identity), AppAction::None);

        assert_eq!(
            dispatch_unhide(&mut state, &identity, &hidden_bao()),
            AppAction::None
        );
        state.release_contact_info_write(Network::Testnet, &id(1), &id(5));
        assert_ne!(
            dispatch_unhide(&mut state, &identity, &hidden_bao()),
            AppAction::None
        );
    }

    #[test]
    fn the_fee_line_is_one_sentence_with_the_amount() {
        assert_eq!(
            note_fee_text("0.0001 DASH"),
            "Estimated fee for saving this note: up to 0.0001 DASH."
        );
    }

    #[test]
    fn copy_constants_are_complete_sentences() {
        for line in [
            NO_SELECTION,
            NOTE_HINT,
            NOTE_UNCHANGED_TOOLTIP,
            NOTE_SAVE_TOOLTIP,
            NOTE_SAVED,
            SELECT_CONTACT_TOOLTIP,
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
                counterparty_id,
            } => {
                assert_eq!(identity.identity.id(), id(1));
                assert_eq!(
                    counterparty_id,
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
            matches!(task, DashPayTask::RejectContactRequest { counterparty_id, .. } if counterparty_id == id(2)),
            "Decline must dispatch RejectContactRequest for the clicked request"
        );
    }

    #[test]
    fn cancel_maps_to_the_cancel_task() {
        let task = request_task(RequestAction::Cancelled, qualified_identity(id(1)), id(2));
        assert!(
            matches!(task, DashPayTask::CancelContactRequest { counterparty_id, .. } if counterparty_id == id(2)),
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
                    update.display_hidden == ContactInfoField::Replace(false),
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

    // ---------------------------------------------------------------
    // In-flight guard — Accept / Decline / Cancel each sign and pay for a
    // state transition, so a second click while the first is running must
    // not buy a second one.
    // ---------------------------------------------------------------

    #[test]
    fn a_second_click_while_the_request_is_in_flight_dispatches_nothing() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();
        let clicked = Some((req(2), RequestAction::Accepted));

        assert_ne!(
            dispatch_request(&mut state, &identity, clicked),
            AppAction::None,
            "the first click must dispatch"
        );
        assert!(
            state.is_in_flight(&req(2)),
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
            Some((req(2), RequestAction::Accepted)),
        );

        assert_ne!(
            dispatch_request(
                &mut state,
                &identity,
                Some((req(3), RequestAction::Declined))
            ),
            AppAction::None,
            "the guard is per request — an unrelated row must still act"
        );
    }

    #[test]
    fn a_failed_request_becomes_clickable_again() {
        let identity = qualified_identity(id(1));
        let mut state = ContactsState::default();
        let clicked = Some((req(2), RequestAction::Cancelled));

        dispatch_request(&mut state, &identity, clicked);
        state.release_request(&req(2));

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
            Some((req(2), RequestAction::Accepted)),
        );
        state.remove_request(&req(2));

        assert!(
            !state.is_in_flight(&req(2)),
            "a resolved request must not hold its guard"
        );
    }

    #[test]
    fn a_row_of_another_identity_dispatches_nothing() {
        let mut state = ContactsState::default();
        let clicked = Some((req(2), RequestAction::Accepted));

        assert_eq!(
            dispatch_request(&mut state, &qualified_identity(id(5)), clicked),
            AppAction::None,
            "a row left from the previous identity must not act for the new one"
        );
        assert!(!state.is_in_flight(&req(2)));
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
