use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::dashpay::{AcceptedAccounts, ContactInfoField, ContactInfoUpdate};
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
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
    /// Payment amount in **duffs** (1 DASH = 100,000,000 duffs), as provided by
    /// `DashPayPaymentHistory`. Despite the `Credits` alias this is a duff value,
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
    loading: bool,
    show_info_popup: bool,
    needs_backend_fetch: bool,
    overwrite_dialog: Option<ConfirmationDialog>,
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
            loading: false,
            show_info_popup: false,
            needs_backend_fetch: true,
            overwrite_dialog: None,
            pending_update: None,
        };
        screen.load_from_database();
        screen
    }

    /// Initialise `contact_info` from local-only data (private notes / hidden flag).
    /// Public profile fields (username, display_name, avatar, bio) are populated
    /// asynchronously by `FetchContactProfile` — see `display_task_result`.
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

    /// Trigger a backend fetch to refresh contact profile data from Platform.
    fn trigger_backend_fetch(&mut self) -> AppAction {
        self.loading = true;

        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::FetchContactProfile {
                identity: self.identity.clone(),
                contact_id: self.contact_id,
            },
        )))
    }

    fn start_editing(&mut self) {
        if let Some(info) = &self.contact_info {
            self.edit_nickname = info.nickname.clone().unwrap_or_default();
            self.edit_note = info.note.clone().unwrap_or_default();
            self.edit_hidden = info.is_hidden;
            self.editing_info = true;
        }
    }

    fn save_contact_info(&mut self) -> AppAction {
        let update = ContactInfoUpdate {
            nickname: ContactInfoField::Replace(if self.edit_nickname.is_empty() {
                None
            } else {
                Some(self.edit_nickname.clone())
            }),
            note: ContactInfoField::Replace(if self.edit_note.is_empty() {
                None
            } else {
                Some(self.edit_note.clone())
            }),
            display_hidden: self.edit_hidden,
            accepted_accounts: AcceptedAccounts::Preserve,
            unreadable: Default::default(),
        };
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

        if let Some(info) = &mut self.contact_info {
            info.nickname = nickname.clone();
            info.note = note.clone();
            info.is_hidden = update.display_hidden;
        }

        let identity_id = self.identity.identity.id();
        if let Err(e) = crate::ui::dashpay::persist_contact_private_info(
            &self.app_context,
            &identity_id,
            &self.contact_id,
            nickname.unwrap_or_default(),
            note.unwrap_or_default(),
            update.display_hidden,
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
                            } else if ui.button("Edit").clicked() {
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

        if let Some(dialog) = &mut self.overwrite_dialog {
            match dialog.show(ui).inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) => {
                    if let Some(update) = self.pending_update.clone() {
                        self.loading = true;
                        action |= AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                            DashPayTask::UpdateContactInfo {
                                identity: self.identity.clone(),
                                contact_id: self.contact_id,
                                update: update.overwrite_unreadable(),
                            },
                        )));
                    }
                    self.overwrite_dialog = None;
                }
                Some(ConfirmationStatus::Canceled) => {
                    self.pending_update = None;
                    self.overwrite_dialog = None;
                }
                None => {}
            }
        }

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup =
                        InfoPopup::new("Private Contact Information", PRIVATE_CONTACT_INFO_TEXT);
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        self.loading = false;
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
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
            } => {
                if identity == self.identity.identity.id() && contact_id == self.contact_id {
                    self.commit_pending_update();
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Contact information saved.",
                        MessageType::Success,
                    );
                }
            }
            BackendTaskSuccessResult::DashPayContactsWithInfo {
                identity,
                contacts: contacts_data,
            } => {
                // Contacts loaded for another identity say nothing about this
                // screen's contact — a reload that outlived an identity switch
                // must not overwrite what is on screen.
                if identity != self.identity.identity.id() {
                    return;
                }

                // If a full contacts reload happened, update our contact if present
                for contact_data in contacts_data {
                    if contact_data.identity_id == self.contact_id {
                        if let Some(info) = &mut self.contact_info {
                            if contact_data.username.is_some() {
                                info.username = contact_data.username;
                            }
                            if contact_data.display_name.is_some() {
                                info.display_name = contact_data.display_name;
                            }
                            if contact_data.avatar_url.is_some() {
                                info.avatar_url = contact_data.avatar_url;
                            }
                            if contact_data.bio.is_some() {
                                info.bio = contact_data.bio;
                            }
                            info.nickname = contact_data.nickname;
                            info.note = contact_data.note;
                            info.is_hidden = contact_data.is_hidden;
                            info.account_reference = contact_data.account_reference;
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        self.loading = false;
        let is_matching_read_error = matches!(
            error,
            TaskError::DashPayContactInfoActionFailed {
                identity_id,
                contact_id,
                source,
            } if *identity_id == self.identity.identity.id()
                && *contact_id == self.contact_id
                && matches!(source.as_ref(), TaskError::DashPayContactInfoRead { .. })
        );
        if is_matching_read_error && self.pending_update.is_some() {
            self.overwrite_dialog = Some(
                ConfirmationDialog::new(
                    "Replace saved contact details?",
                    "This contact's saved details cannot be read. Continuing will save the nickname and note shown here and clear the accepted-account settings. You cannot undo this change.",
                )
                .confirm_text(Some("Replace saved details"))
                .danger_mode(true),
            );
            return true;
        }
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
