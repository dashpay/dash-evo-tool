use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::dashpay::{
    ContactDetails, ContactInfoField, ContactInfoUpdate, DashPaySnapshot, PaymentDirection,
};
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::MessageBanner;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::dashpay::DashPaySubscreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{Color32, RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

const PRIVATE_CONTACT_INFO_TEXT: &str = "About Private Contact Information:\n\n\
    This information is encrypted and stored on Platform.\n\n\
    It is never shared with the contact - only you can decrypt it.\n\n\
    Only you can see these nicknames and notes.\n\n\
    Use this to organize and remember your contacts.";

#[derive(Debug, Clone)]
pub struct Payment {
    pub tx_id: String,
    /// Payment amount in **duffs** (1 DASH = 100,000,000 duffs). Despite the `Credits` alias this is a duff value,
    /// so render it with `format_duffs_as_dash`.
    pub amount: Credits,
    pub timestamp: u64,
    pub is_incoming: bool,
    pub memo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContactInfo {
    pub identity_id: Identifier,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub is_hidden: bool,
    pub account_reference: u32,
}

pub struct ContactDetailsScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    pub contact_id: Identifier,
    contact_info: Option<ContactInfo>,
    payment_history: Vec<Payment>,
    editing_info: bool,
    edit_nickname: String,
    edit_note: String,
    edit_hidden: bool,
    /// The details the editor opened with, so a save replaces only the
    /// fields the user changed.
    edit_opened: ContactDetails,
    /// A save is running.
    loading: bool,
    /// The sync reading the published details is running; editing waits for
    /// it so the editor opens from the published details rather than this
    /// device's copy.
    details_loading: bool,
    /// `contact_info`'s nickname, note and hidden flag came from the DashPay
    /// snapshot, so a reread of this device's copy keeps them.
    details_read: bool,
    /// The snapshot `contact_info` was last read from.
    applied: Option<Arc<DashPaySnapshot>>,
    show_info_popup: bool,
    needs_backend_fetch: bool,
    pending_update: Option<ContactInfoUpdate>,
}

impl ContactDetailsScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        identity: QualifiedIdentity,
        contact_id: Identifier,
    ) -> Self {
        let mut screen = Self {
            app_context,
            identity,
            contact_id,
            contact_info: None,
            payment_history: Vec::new(),
            editing_info: false,
            edit_nickname: String::new(),
            edit_note: String::new(),
            edit_hidden: false,
            edit_opened: ContactDetails::default(),
            loading: false,
            details_loading: false,
            details_read: false,
            applied: None,
            show_info_popup: false,
            needs_backend_fetch: true,
            pending_update: None,
        };
        screen.load_from_database();
        screen
    }

    /// Initialise `contact_info` from this device's copy of the nickname, note
    /// and hidden flag, until the DashPay snapshot holds the contact. Public
    /// profile fields (username, display_name, avatar, bio) come from the
    /// snapshot too, or from `FetchContactProfile` for someone who is not an
    /// established contact — see `display_task_result`.
    fn load_from_database(&mut self) {
        let identity_id = self.identity.identity.id();

        // Load private contact info (nickname, notes, hidden) — DET-local memo,
        // backed by the WalletBackend k/v sidecar post-D4c.
        let (nickname, note, is_hidden) =
            match self.app_context.wallet_backend().and_then(|backend| {
                backend.dashpay_get_private_info(&identity_id, &self.contact_id)
            }) {
                Ok(Some(info)) => (info.nickname, info.notes, info.is_hidden),
                Ok(None) => (String::new(), String::new(), false),
                Err(e) => {
                    tracing::warn!(
                        "DashPay private-info sidecar read failed; defaulting to empty: {e:?}"
                    );
                    (String::new(), String::new(), false)
                }
            };

        let nickname = if nickname.is_empty() {
            None
        } else {
            Some(nickname)
        };
        let note = if note.is_empty() { None } else { Some(note) };
        // Details read from the backend outrank this device's copy.
        let (nickname, note, is_hidden) = match self.contact_info.as_ref() {
            Some(existing) if self.details_read => (
                existing.nickname.clone(),
                existing.note.clone(),
                existing.is_hidden,
            ),
            _ => (nickname, note, is_hidden),
        };

        // Preserve any public profile fields already in `contact_info`; otherwise
        // start empty and let the async fetch fill them in.
        let (username, display_name, avatar_url, bio) =
            if let Some(existing) = self.contact_info.as_ref() {
                (
                    existing.username.clone(),
                    existing.display_name.clone(),
                    existing.avatar_url.clone(),
                    existing.bio.clone(),
                )
            } else {
                (None, None, None, None)
            };

        self.contact_info = Some(ContactInfo {
            identity_id: self.contact_id,
            username,
            display_name,
            bio,
            avatar_url,
            nickname,
            note,
            is_hidden,
            account_reference: 0,
        });
    }

    /// Show the contact as the owner's DashPay snapshot holds it, when the
    /// snapshot is not the one it was last read from.
    fn apply_snapshot(&mut self) {
        let Some(snapshot) = self
            .app_context
            .dashpay_snapshot(&self.identity.identity.id())
        else {
            return;
        };
        if self
            .applied
            .as_ref()
            .is_some_and(|applied| Arc::ptr_eq(applied, &snapshot))
        {
            return;
        }
        self.show_snapshot(&snapshot);
        self.applied = Some(snapshot);
    }

    /// Show the contact and the payments exchanged with them as `snapshot`
    /// holds them. Leaves `contact_info` alone when the snapshot does not hold
    /// the contact.
    fn show_snapshot(&mut self, snapshot: &DashPaySnapshot) {
        let contact_bytes = self.contact_id.to_vec();
        self.payment_history = snapshot
            .payments
            .iter()
            .filter(|payment| {
                payment.from_identity_id == contact_bytes || payment.to_identity_id == contact_bytes
            })
            .map(|payment| Payment {
                tx_id: payment.tx_id.clone(),
                amount: Credits::try_from(payment.amount).unwrap_or_default(),
                timestamp: u64::try_from(payment.created_at).unwrap_or_default(),
                is_incoming: payment.payment_type == PaymentDirection::Received,
                memo: payment.memo.clone().filter(|memo| !memo.is_empty()),
            })
            .collect();
        let Some(contact) = snapshot
            .contacts
            .iter()
            .find(|contact| contact.identity_id == self.contact_id)
        else {
            return;
        };
        self.contact_info = Some(ContactInfo {
            identity_id: self.contact_id,
            username: contact.username.clone(),
            display_name: contact.display_name.clone(),
            bio: contact.bio.clone(),
            avatar_url: contact.avatar_url.clone(),
            nickname: contact.nickname.clone(),
            note: contact.note.clone(),
            is_hidden: contact.is_hidden,
            account_reference: contact.account_reference,
        });
        self.details_read = true;
    }

    /// Sync the owner's DashPay state, which reads the published details, and
    /// read the public profile of someone the snapshot does not hold as a
    /// contact.
    fn trigger_backend_fetch(&mut self) -> AppAction {
        self.details_loading = true;
        let task = |task| BackendTask::DashPayTask(Box::new(task));
        let mut tasks = vec![task(DashPayTask::SyncNow {
            identity: Some(self.identity.clone()),
        })];
        if !self.details_read {
            tasks.push(task(DashPayTask::FetchContactProfile {
                identity: self.identity.clone(),
                contact_id: self.contact_id,
            }));
        }
        AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Concurrent)
    }

    fn start_editing(&mut self) {
        if self.details_loading {
            return;
        }
        if let Some(info) = &self.contact_info {
            self.edit_nickname = info.nickname.clone().unwrap_or_default();
            self.edit_note = info.note.clone().unwrap_or_default();
            self.edit_hidden = info.is_hidden;
            self.edit_opened = ContactDetails {
                nickname: info.nickname.clone(),
                note: info.note.clone(),
                hidden: info.is_hidden,
            };
            self.editing_info = true;
        }
    }

    fn save_contact_info(&mut self) -> AppAction {
        let non_empty = |text: &str| (!text.is_empty()).then(|| text.to_string());
        // The editor opens from this device's copy, which can be older than
        // the published details; untouched fields keep the published ones.
        let update = ContactInfoUpdate::edits(
            &self.edit_opened,
            ContactDetails {
                nickname: non_empty(&self.edit_nickname),
                note: non_empty(&self.edit_note),
                hidden: self.edit_hidden,
            },
        );
        self.pending_update = Some(update.clone());

        self.loading = true;

        // Dispatch backend task to persist to Platform (encrypted)
        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::UpdateContactInfo {
                identity: self.identity.clone(),
                contact_id: self.contact_id,
                update,
            },
        )))
    }

    fn commit_pending_update(&mut self) {
        let Some(update) = self.pending_update.take() else {
            return;
        };
        let current_nickname = self
            .contact_info
            .as_ref()
            .and_then(|info| info.nickname.clone());
        let current_note = self
            .contact_info
            .as_ref()
            .and_then(|info| info.note.clone());
        let nickname = match update.nickname {
            ContactInfoField::Preserve => current_nickname,
            ContactInfoField::Replace(value) => value,
        };
        let note = match update.note {
            ContactInfoField::Preserve => current_note,
            ContactInfoField::Replace(value) => value,
        };

        let hidden = match update.display_hidden {
            ContactInfoField::Preserve => self
                .contact_info
                .as_ref()
                .is_some_and(|info| info.is_hidden),
            ContactInfoField::Replace(hidden) => hidden,
        };
        if let Some(info) = &mut self.contact_info {
            info.nickname = nickname.clone();
            info.note = note.clone();
            info.is_hidden = hidden;
        }

        let identity_id = self.identity.identity.id();
        if let Err(e) = crate::ui::dashpay::persist_contact_private_info(
            &self.app_context,
            &identity_id,
            &self.contact_id,
            nickname.unwrap_or_default(),
            note.unwrap_or_default(),
            hidden,
        ) {
            tracing::warn!("DashPay private-info sidecar write failed after Platform save: {e:?}");
        }
        self.editing_info = false;
    }

    fn cancel_editing(&mut self) {
        self.editing_info = false;
        self.edit_nickname.clear();
        self.edit_note.clear();
        self.edit_hidden = false;
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        self.apply_snapshot();

        // Dispatch deferred backend fetch if flagged
        if self.needs_backend_fetch {
            self.needs_backend_fetch = false;
            action = self.trigger_backend_fetch();
        }

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Contact Details");
        });

        ui.separator();

        // Loading indicator
        if self.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading contact details...");
            });
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {
            if let Some(info) = self.contact_info.clone() {
                // Contact profile section
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        // Avatar placeholder
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("👤").size(60.0).color(DashColors::DEEP_BLUE));
                            ui.small("Contact");
                        });

                        ui.vertical(|ui| {
                            // Display nickname if set, otherwise display name
                            let name = info
                                .nickname
                                .as_ref()
                                .or(info.display_name.as_ref())
                                .or(info.username.as_ref())
                                .cloned()
                                .unwrap_or_else(|| "Unknown".to_string());
                            ui.label(RichText::new(name).heading());

                            // Username
                            if let Some(username) = &info.username {
                                ui.label(RichText::new(format!("@{}", username)).strong());
                            }

                            // Bio
                            if let Some(bio) = &info.bio {
                                ui.label(RichText::new(bio).weak());
                            }

                            // Identity ID
                            ui.label(
                                RichText::new(format!("ID: {}", info.identity_id))
                                    .small()
                                    .weak(),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            // Send Payment is an experimental DashPay feature.
                            if FeatureGate::DashPayOperations.is_available(&self.app_context)
                                && ui.button("Send Payment").clicked()
                            {
                                action = AppAction::AddScreen(
                                    ScreenType::DashPaySendPayment(
                                        self.identity.clone(),
                                        self.contact_id,
                                    )
                                    .create_screen(&self.app_context),
                                );
                            }
                        });
                    });
                });

                ui.add_space(10.0);

                // Contact info section
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Private Contact Information").strong());
                        ui.add_space(5.0);
                        if crate::ui::helpers::info_icon_button(ui, PRIVATE_CONTACT_INFO_TEXT)
                            .clicked()
                        {
                            self.show_info_popup = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.editing_info {
                                if ui.button("Cancel").clicked() {
                                    self.cancel_editing();
                                }
                                if ui.button("Save").clicked() {
                                    action = self.save_contact_info();
                                }
                            } else if ui
                                .add_enabled(!self.details_loading, egui::Button::new("Edit"))
                                .clicked()
                            {
                                self.start_editing();
                            }
                        });
                    });

                    ui.separator();

                    if self.editing_info {
                        // Edit mode
                        ui.horizontal(|ui| {
                            ui.label("Nickname:");
                            ui.add(
                                TextEdit::singleline(&mut self.edit_nickname)
                                    .hint_text("Optional nickname for this contact"),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label("Note:");
                            ui.add(
                                TextEdit::multiline(&mut self.edit_note)
                                    .hint_text("Private notes about this contact")
                                    .desired_rows(3),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.edit_hidden, "Hide this contact");
                            if self.edit_hidden {
                                ui.label(
                                    RichText::new("(Contact will not appear in lists)")
                                        .small()
                                        .weak(),
                                );
                            }
                        });
                    } else {
                        // View mode
                        if let Some(nickname) = &info.nickname {
                            ui.horizontal(|ui| {
                                ui.label("Nickname:");
                                ui.label(nickname);
                            });
                        }

                        if let Some(note) = &info.note {
                            ui.horizontal(|ui| {
                                ui.label("Note:");
                                ui.label(note);
                            });
                        }

                        if info.is_hidden {
                            ui.label(
                                RichText::new("⚠️ This contact is hidden")
                                    .color(Color32::from_rgb(200, 150, 50)),
                            );
                        }

                        if info.nickname.is_none() && info.note.is_none() && !info.is_hidden {
                            ui.label(
                                RichText::new(
                                    "No private info set. Click Edit to add a nickname or note.",
                                )
                                .weak(),
                            );
                        }
                    }
                });

                ui.add_space(10.0);

                // Payment history section
                ui.group(|ui| {
                    ui.label(RichText::new("Payment History").strong());
                    ui.separator();

                    if self.payment_history.is_empty() {
                        ui.label("No payment history with this contact");
                    } else {
                        for payment in &self.payment_history {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.horizontal(|ui| {
                                // Direction indicator
                                if payment.is_incoming {
                                    ui.label(RichText::new("⬇").color(DashColors::SUCCESS));
                                } else {
                                    ui.label(RichText::new("⬆").color(DashColors::ERROR));
                                }

                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        // Amount (payment.amount is in duffs)
                                        let amount_str = format_duffs_as_dash(payment.amount);
                                        if payment.is_incoming {
                                            ui.label(
                                                RichText::new(format!("+{}", amount_str))
                                                    .color(DashColors::SUCCESS),
                                            );
                                        } else {
                                            ui.label(
                                                RichText::new(format!("-{}", amount_str))
                                                    .color(DashColors::ERROR),
                                            );
                                        }

                                        // Memo
                                        if let Some(memo) = &payment.memo {
                                            ui.label(
                                                RichText::new(format!("\"{}\"", memo))
                                                    .italics()
                                                    .color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }
                                    });

                                    ui.horizontal(|ui| {
                                        // Transaction ID
                                        ui.label(
                                            RichText::new(&payment.tx_id)
                                                .small()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
                                    });
                                });
                            });
                            ui.separator();
                        }
                    }
                });

                ui.add_space(10.0);

                // Actions section
                ui.group(|ui| {
                    ui.label(RichText::new("Actions").strong());
                    ui.separator();

                    ui.label(
                        RichText::new(
                            "Contact removal and blocking are not yet available. \
                             Contact requests cannot be revoked once sent on Platform.",
                        )
                        .weak(),
                    );
                });
            } else {
                // No contact info loaded
                ui.group(|ui| {
                    ui.label("No contact information available");
                    ui.separator();
                    ui.label(format!("Contact ID: {}", self.contact_id));
                    ui.add_space(10.0);
                    if ui.button("Refresh from Platform").clicked() {
                        action = self.trigger_backend_fetch();
                    }
                });

                ui.add_space(10.0);
            }
        });

        action
    }
}

impl ScreenLike for ContactDetailsScreen {
    fn refresh(&mut self) {
        self.load_from_database();
    }

    fn refresh_on_arrival(&mut self) {
        self.load_from_database();
        self.applied = None;
        // Flag that we need a backend fetch; it will be dispatched from render()
        self.needs_backend_fetch = true;
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel with contact name if available
        let contact_name = self
            .contact_info
            .as_ref()
            .and_then(|info| {
                info.nickname
                    .as_ref()
                    .or(info.display_name.as_ref().or(info.username.as_ref()))
            })
            .map(|name| format!("Contact: {}", name))
            .unwrap_or_else(|| "Contact Details".to_string());

        action |= add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                (&contact_name, AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ui, &self.app_context, DashPaySubscreen::Contacts);

        action |= island_central_panel(ui, |ui| self.render(ui));

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("dashpay_contact_details_private_info_popup"),
                        "Private Contact Information",
                        PRIVATE_CONTACT_INFO_TEXT,
                    );
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        // A failed read leaves this device's copy in place; a save from it
        // still replaces only the fields the user edits.
        self.loading = false;
        self.details_loading = false;
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::DashPaySynced => {
                self.details_loading = false;
            }
            BackendTaskSuccessResult::DashPayContactProfile(Some(doc)) => {
                // Update contact info with fresh profile data from Platform
                use dash_sdk::dpp::document::DocumentV0Getters;
                let properties = doc.properties();

                let display_name = properties
                    .get("displayName")
                    .and_then(|v| v.as_text())
                    .map(|s| s.to_string());

                let bio = properties
                    .get("bio")
                    .and_then(|v| v.as_text())
                    .map(|s| s.to_string());

                let avatar_url = properties
                    .get("avatarUrl")
                    .and_then(|v| v.as_text())
                    .map(|s| s.to_string());

                // `FetchContactProfile` re-queries Platform on each open, and
                // the WalletBackend mirror covers identities we manage.
                // Out-of-wallet contact profiles are not cached through the
                // upstream seam.

                // Update the in-memory contact info
                if let Some(info) = &mut self.contact_info {
                    if display_name.is_some() {
                        info.display_name = display_name;
                    }
                    if bio.is_some() {
                        info.bio = bio;
                    }
                    if avatar_url.is_some() {
                        info.avatar_url = avatar_url;
                    }
                } else {
                    // No existing info — create one with the profile data
                    self.contact_info = Some(ContactInfo {
                        identity_id: self.contact_id,
                        username: None,
                        display_name,
                        bio,
                        avatar_url,
                        nickname: None,
                        note: None,
                        is_hidden: false,
                        account_reference: 0,
                    });
                }
            }
            BackendTaskSuccessResult::DashPayContactInfoUpdated {
                identity,
                contact_id,
                published,
            } => {
                if identity == self.identity.identity.id() && contact_id == self.contact_id {
                    self.loading = false;
                    self.commit_pending_update();
                    let (message, kind) = if published {
                        ("Contact information saved.", MessageType::Success)
                    } else {
                        (
                            crate::ui::identity::contacts::DETAILS_SAVED_ON_THIS_DEVICE,
                            MessageType::Info,
                        )
                    };
                    MessageBanner::set_global(self.app_context.egui_ctx(), message, kind);
                }
            }
            _ => {}
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        self.loading = false;
        if matches!(
            error,
            TaskError::DashPayContactInfoActionFailed {
                identity_id,
                contact_id,
                ..
            } if *identity_id == self.identity.identity.id() && *contact_id == self.contact_id
        ) {
            self.pending_update = None;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{bare_user_identity, test_app_context};
    use dash_sdk::dpp::dashcore::Network;

    fn dispatched_update(action: AppAction) -> ContactInfoUpdate {
        match action {
            AppAction::BackendTask(BackendTask::DashPayTask(task)) => match *task {
                DashPayTask::UpdateContactInfo { update, .. } => update,
                other => panic!("expected UpdateContactInfo, got {other:?}"),
            },
            _ => panic!("expected a DashPay backend task"),
        }
    }

    /// This screen opens from this device's copy of the details, which can be
    /// older than what another device or the Identity Hub published. Saving
    /// an edited note must keep the published nickname and visibility rather
    /// than republish the stale local ones.
    #[test]
    fn saving_one_edited_field_keeps_the_published_others() {
        let dir = tempfile::tempdir().expect("temp dir");
        let context = test_app_context(dir.path());
        let identity = bare_user_identity(Identifier::from([1; 32]), Network::Testnet);
        let mut screen = ContactDetailsScreen::new(context, identity, Identifier::from([2; 32]));

        screen.start_editing();
        screen.edit_note = "Met at the meetup".to_string();
        let update = dispatched_update(screen.save_contact_info());

        assert_eq!(
            update,
            ContactInfoUpdate {
                nickname: ContactInfoField::Preserve,
                note: ContactInfoField::Replace(Some("Met at the meetup".into())),
                display_hidden: ContactInfoField::Preserve,
            }
        );
    }

    /// The editor opens from the details shown for the contact everywhere
    /// else (published first), held in the DashPay snapshot, not from this
    /// device's older copy. While the sync reading them runs, it stays closed.
    #[test]
    fn the_editor_opens_from_the_published_details() {
        use crate::model::dashpay::ContactData;
        let dir = tempfile::tempdir().expect("temp dir");
        let context = test_app_context(dir.path());
        let owner = Identifier::from([1; 32]);
        let contact = Identifier::from([2; 32]);
        let identity = bare_user_identity(owner, Network::Testnet);
        let mut screen = ContactDetailsScreen::new(context, identity, contact);

        let AppAction::BackendTasks(tasks, _) = screen.trigger_backend_fetch() else {
            panic!("expected the sync and the profile read");
        };
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::DashPayTask(task)
                if matches!(task.as_ref(), DashPayTask::SyncNow { identity: Some(_) })
        )));
        screen.start_editing();
        assert!(!screen.editing_info, "the editor waits for the sync");

        screen.show_snapshot(&DashPaySnapshot {
            contacts: vec![ContactData {
                identity_id: contact,
                nickname: Some("Bao".into()),
                note: Some("published".into()),
                is_hidden: true,
                account_reference: 0,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            }],
            ..Default::default()
        });
        screen.display_task_result(BackendTaskSuccessResult::DashPaySynced);
        // A refresh rereads this device's copy, which must not replace them.
        screen.refresh();
        screen.start_editing();

        assert!(screen.editing_info);
        assert_eq!(screen.edit_nickname, "Bao");
        assert_eq!(screen.edit_note, "published");
        assert!(screen.edit_hidden);
    }

    /// Clearing a field is an edit too: it is sent as a replacement with no
    /// value, not dropped.
    #[test]
    fn clearing_and_hiding_are_sent_as_edits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let context = test_app_context(dir.path());
        let identity = bare_user_identity(Identifier::from([1; 32]), Network::Testnet);
        let mut screen = ContactDetailsScreen::new(context, identity, Identifier::from([2; 32]));
        if let Some(info) = &mut screen.contact_info {
            info.nickname = Some("Bao".into());
        }

        screen.start_editing();
        screen.edit_nickname.clear();
        screen.edit_hidden = true;
        let update = dispatched_update(screen.save_contact_info());

        assert_eq!(
            update,
            ContactInfoUpdate {
                nickname: ContactInfoField::Replace(None),
                note: ContactInfoField::Preserve,
                display_hidden: ContactInfoField::Replace(true),
            }
        );
    }
}
