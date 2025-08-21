use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::dashpay::{ContactData, DashPayTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

pub struct ContactInfoEditorScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    pub contact_id: Identifier,
    contact_username: Option<String>,
    nickname: String,
    note: String,
    is_hidden: bool,
    accepted_accounts: Vec<u32>,
    account_input: String,
    message: Option<(String, MessageType)>,
    saving: bool,
}

impl ContactInfoEditorScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        identity: QualifiedIdentity,
        contact_id: Identifier,
    ) -> Self {
        let screen = Self {
            app_context,
            identity,
            contact_id,
            contact_username: None,
            nickname: String::new(),
            note: String::new(),
            is_hidden: false,
            accepted_accounts: Vec::new(),
            account_input: String::new(),
            message: None,
            saving: false,
        };
        screen
    }

    fn load_contact_info(&mut self) -> AppAction {
        // Trigger fetch from platform to get existing contact info
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
            identity: self.identity.clone(),
        }));
        AppAction::BackendTask(task)
    }

    fn handle_contacts_result(&mut self, contacts_data: Vec<ContactData>) {
        // Find the contact info for our specific contact
        for contact_data in contacts_data {
            if contact_data.identity_id == self.contact_id {
                self.nickname = contact_data.nickname.unwrap_or_default();
                self.note = contact_data.note.unwrap_or_default();
                self.is_hidden = contact_data.is_hidden;
                // Note: accepted_accounts would come from the ContactData but we're not fully implementing it yet
                break;
            }
        }
    }

    fn save_contact_info(&mut self) -> AppAction {
        self.saving = true;

        let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo {
            identity: self.identity.clone(),
            contact_id: self.contact_id,
            nickname: if self.nickname.is_empty() {
                None
            } else {
                Some(self.nickname.clone())
            },
            note: if self.note.is_empty() {
                None
            } else {
                Some(self.note.clone())
            },
            is_hidden: self.is_hidden,
            accepted_accounts: self.accepted_accounts.clone(),
        }));

        AppAction::BackendTask(task)
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Header with Back button and title
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Edit Private Contact Details");
            ui.add_space(10.0);
            crate::ui::helpers::info_icon_button(
                ui,
                "About Private Contact Information:\n\n\
                • This information is stored locally on your device\n\
                • It is NEVER shared with the contact or published\n\
                • Only you can see these nicknames and notes\n\
                • Hidden contacts can still send you payments\n\
                • Use this to organize and remember your contacts",
            );
        });

        ui.separator();

        // Show message if any
        if let Some((message, message_type)) = &self.message {
            let color = match message_type {
                MessageType::Success => DashColors::SUCCESS,
                MessageType::Error => DashColors::ERROR,
                MessageType::Info => DashColors::INFO,
            };
            ui.colored_label(color, message);
            ui.separator();
        }

        ScrollArea::vertical().show(ui, |ui| {
            ui.group(|ui| {
                // Contact identity
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Contact:").strong().color(if dark_mode { DashColors::DARK_TEXT_PRIMARY } else { DashColors::TEXT_PRIMARY }));
                    if let Some(username) = &self.contact_username {
                        ui.label(RichText::new(username).color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                    } else {
                        ui.label(RichText::new(format!("{}", self.contact_id))
                            .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                    }
                });

                ui.separator();

                // Nickname field
                ui.label(RichText::new("Private Nickname:").strong().color(if dark_mode { DashColors::DARK_TEXT_PRIMARY } else { DashColors::TEXT_PRIMARY }));
                ui.label(RichText::new("Give this contact a custom name that ONLY YOU will see").small()
                    .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                ui.add(
                    TextEdit::singleline(&mut self.nickname)
                        .hint_text("e.g., 'Mom', 'Boss', 'Alice from work'")
                        .desired_width(300.0)
                );

                ui.add_space(10.0);

                // Note field
                ui.label(RichText::new("Private Note:").strong().color(if dark_mode { DashColors::DARK_TEXT_PRIMARY } else { DashColors::TEXT_PRIMARY }));
                ui.label(RichText::new("Add notes about this contact (only visible to you)").small()
                    .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                ui.add(
                    TextEdit::multiline(&mut self.note)
                        .hint_text("e.g., 'Met at Dash conference 2024', 'Owes me for lunch'")
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                );

                ui.add_space(10.0);

                // Hidden checkbox
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.is_hidden, "Hide this contact from my list");
                });
                if self.is_hidden {
                    ui.label(RichText::new("⚠️ Hidden contacts won't appear in your contact list but can still send you payments")
                        .small().color(DashColors::WARNING));
                } else {
                    ui.label(RichText::new("Contact will appear in your contact list").small()
                        .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                }

                ui.add_space(10.0);

                // Account references section
                ui.label(RichText::new("Accepted Account Indices:").strong().color(if dark_mode { DashColors::DARK_TEXT_PRIMARY } else { DashColors::TEXT_PRIMARY }));
                ui.label(RichText::new("Specify which account indices this contact can pay to (comma-separated)").small()
                    .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));

                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.account_input)
                            .hint_text("e.g., 0, 1, 2")
                            .desired_width(200.0)
                    );

                    if ui.button("Parse").clicked() {
                        // Parse the account indices
                        self.accepted_accounts.clear();
                        for part in self.account_input.split(',') {
                            if let Ok(index) = part.trim().parse::<u32>() {
                                if !self.accepted_accounts.contains(&index) {
                                    self.accepted_accounts.push(index);
                                }
                            }
                        }
                        self.accepted_accounts.sort();

                        // Update the input field to show the parsed values
                        self.account_input = self.accepted_accounts
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                    }
                });

                if !self.accepted_accounts.is_empty() {
                    ui.label(RichText::new(format!("Accepted accounts: {:?}", self.accepted_accounts)).small()
                        .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                } else {
                    ui.label(RichText::new("All accounts accepted (default)").small()
                        .color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                }

                ui.add_space(20.0);

                // Action buttons
                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;

                    if self.saving {
                        ui.spinner();
                        ui.label(RichText::new("Saving...").color(if dark_mode { DashColors::DARK_TEXT_SECONDARY } else { DashColors::TEXT_SECONDARY }));
                    } else {
                        if ui.button(RichText::new("💾 Save Changes").size(16.0)).clicked() {
                            action = self.save_contact_info();
                        }

                        ui.add_space(10.0);

                        if ui.button(RichText::new("❌ Cancel").size(16.0)).clicked() {
                            action = AppAction::PopScreen;
                        }
                    }
                });
            });

        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.saving = false;
        match result {
            BackendTaskSuccessResult::Message(msg) => {
                self.display_message(&msg, MessageType::Success);
            }
            BackendTaskSuccessResult::DashPayContactsWithInfo(contacts_data) => {
                self.handle_contacts_result(contacts_data);
            }
            _ => {
                self.display_message("Contact information updated", MessageType::Success);
            }
        }
    }
}

impl ScreenLike for ContactInfoEditorScreen {
    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel with back button
        let right_buttons = vec![(
            "Refresh",
            DesiredAppAction::Custom("refresh_contact_info".to_string()),
        )];

        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Contact Details", AppAction::PopScreen),
                ("Edit", AppAction::None),
            ],
            right_buttons,
        );

        // Add left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDashPayContacts,
        );

        // Main content area with island styling
        action |= island_central_panel(ctx, |ui| self.render(ui));

        // Handle custom actions from top panel
        if let AppAction::Custom(command) = &action {
            match command.as_str() {
                "refresh_contact_info" => {
                    action = self.load_contact_info();
                }
                _ => {}
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.display_task_result(result);
    }
}
