use crate::app::AppAction;
use crate::context::AppContext;
use crate::platform::identity::{DpnsNameInputToRegister, IdentityTask};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::Context;
use itertools::Itertools;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub enum RegisterDpnsNameStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

// TO DO: If there are no qualified identities, just display "No qualified identities" with no input options
pub struct RegisterDpnsNameScreen {
    qualified_identity_ids: Vec<Identifier>,
    identity_id_input: Identifier,
    name_input: String,
    register_dpns_name_status: RegisterDpnsNameStatus,
    pub app_context: Arc<AppContext>,
}

impl RegisterDpnsNameScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let qualified_identity_ids = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .iter()
            .map(|identity| identity.identity.id())
            .collect_vec();
        Self {
            qualified_identity_ids,
            identity_id_input: Identifier::random(),
            name_input: String::new(),
            register_dpns_name_status: RegisterDpnsNameStatus::NotStarted,
            app_context: app_context.clone(),
        }
    }

    fn render_identity_id_selection(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Identity ID:");

            // Create a ComboBox for selecting Identity IDs
            egui::ComboBox::from_label("")
                .selected_text(format!(
                    "{:?}",
                    self.qualified_identity_ids
                        .first()
                        .expect("Expected to have at least one qualified identity")
                        .to_string(Encoding::Base58)
                ))
                .show_ui(ui, |ui| {
                    // Loop through the qualified_identity_ids and display each as selectable
                    for id in &self.qualified_identity_ids {
                        // Display each Identifier as a selectable item
                        ui.selectable_value(
                            &mut self.identity_id_input,
                            *id,
                            id.to_string(Encoding::Base58),
                        );
                    }
                });
        });
    }

    fn register_dpns_name_clicked(&mut self) -> AppAction {
        let dpns_name_input = DpnsNameInputToRegister {
            identity_id_input: self.identity_id_input,
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
