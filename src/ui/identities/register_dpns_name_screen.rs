use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::platform::identity::{IdentityTask, RegisterDpnsNameInput};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::Context;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub enum RegisterDpnsNameStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct RegisterDpnsNameScreen {
    qualified_identities: Vec<QualifiedIdentity>,
    selected_qualified_identity: Option<QualifiedIdentity>,
    name_input: String,
    register_dpns_name_status: RegisterDpnsNameStatus,
    pub app_context: Arc<AppContext>,
}

impl RegisterDpnsNameScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let qualified_identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_default();
        let selected_qualified_identity = qualified_identities.first().cloned();
        Self {
            qualified_identities,
            selected_qualified_identity,
            name_input: String::new(),
            register_dpns_name_status: RegisterDpnsNameStatus::NotStarted,
            app_context: app_context.clone(),
        }
    }

    fn render_identity_id_selection(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Identity ID:");

            // Create a ComboBox for selecting a Qualified Identity
            egui::ComboBox::from_label("")
                .selected_text(
                    self.selected_qualified_identity
                        .as_ref()
                        .map(|qi| {
                            qi.alias
                                .as_ref()
                                .unwrap_or(&qi.identity.id().to_string(Encoding::Base58))
                                .clone()
                        })
                        .unwrap_or_else(|| "Select an identity".to_string()),
                )
                .show_ui(ui, |ui| {
                    // Loop through the qualified identities and display each as selectable
                    for qualified_identity in &self.qualified_identities {
                        // Display each QualifiedIdentity as a selectable item
                        if ui
                            .selectable_value(
                                &mut self.selected_qualified_identity,
                                Some(qualified_identity.clone()),
                                qualified_identity.alias.as_ref().unwrap_or(
                                    &qualified_identity.identity.id().to_string(Encoding::Base58),
                                ),
                            )
                            .clicked()
                        {
                            self.selected_qualified_identity = Some(qualified_identity.clone());
                        }
                    }
                });
        });
    }

    fn register_dpns_name_clicked(&mut self) -> AppAction {
        let Some(qualified_identity) = self.selected_qualified_identity.as_ref() else {
            return AppAction::None;
        };
        let dpns_name_input = RegisterDpnsNameInput {
            qualified_identity: qualified_identity.clone(),
            name_input: self.name_input.trim().to_string(),
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(
            dpns_name_input,
        )))
    }
}

impl ScreenLike for RegisterDpnsNameScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Info && message == "Success" {
            self.register_dpns_name_status = RegisterDpnsNameStatus::Complete;
        } else {
            self.register_dpns_name_status =
                RegisterDpnsNameStatus::ErrorMessage(message.to_string());
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contested Names", AppAction::GoToMainScreen),
                ("Register DPNS Name", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Register DPNS Name");

            if self.qualified_identities.is_empty() {
                ui.label("No qualified identities available to register a DPNS name.");
                return;
            }

            // Select the identity to register the name for
            self.render_identity_id_selection(ui);

            // Input for the name
            ui.horizontal(|ui| {
                ui.label("Name (without \".dash\"):");
                ui.text_edit_singleline(&mut self.name_input);
            });

            if ui.button("Register Name").clicked() {
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.register_dpns_name_status = RegisterDpnsNameStatus::WaitingForResult(now);
                action = self.register_dpns_name_clicked();
            }

            match &self.register_dpns_name_status {
                RegisterDpnsNameStatus::NotStarted => {
                    // Do nothing
                }
                RegisterDpnsNameStatus::WaitingForResult(start_time) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    let elapsed_seconds = now - start_time;

                    let display_time = if elapsed_seconds < 60 {
                        format!(
                            "{} second{}",
                            elapsed_seconds,
                            if elapsed_seconds == 1 { "" } else { "s" }
                        )
                    } else {
                        let minutes = elapsed_seconds / 60;
                        let seconds = elapsed_seconds % 60;
                        format!(
                            "{} minute{} and {} second{}",
                            minutes,
                            if minutes == 1 { "" } else { "s" },
                            seconds,
                            if seconds == 1 { "" } else { "s" }
                        )
                    };

                    ui.label(format!(
                        "Registering... Time taken so far: {}",
                        display_time
                    ));
                }
                RegisterDpnsNameStatus::ErrorMessage(msg) => {
                    ui.label(format!("Error: {}", msg));
                }
                RegisterDpnsNameStatus::Complete => {
                    action = AppAction::PopScreenAndRefresh;
                }
            }
        });

        action
    }
}
