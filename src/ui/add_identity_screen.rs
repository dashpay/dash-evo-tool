use eframe::egui::{Context};
use std::sync::{Arc, Mutex};
use dash_sdk::platform::Fetch;
use dpp::identifier::Identifier;
use dpp::identity::Identity;
use dpp::platform_value::string_encoding::Encoding;
use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;

pub struct AddIdentityScreen {
    identity_id_input: String,
    error_message: Option<String>,
    identity_result: Option<Arc<Mutex<Option<Result<Identity, String>>>>>,
    app_context: Arc<AppContext>,
}

impl AddIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            identity_id_input: String::new(),
            error_message: None,
            identity_result: None,
            app_context: app_context.clone(),
        }
    }

    pub fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(ctx, &self.app_context, vec![("Identities", AppAction::GoToMainScreen), ("Add Identity", AppAction::None)], None);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Add Identity");

            ui.horizontal(|ui| {
                ui.label("Identity ID (Hex or Base58):");
                ui.text_edit_singleline(&mut self.identity_id_input);
            });

            if ui.button("Load Identity").clicked() {
                let identity_id_input = self.identity_id_input.trim().to_string();
                let sdk_clone = Arc::clone(&self.app_context.sdk);

                // Create the shared state
                let identity_result = Arc::new(Mutex::new(None));
                let identity_result_clone = Arc::clone(&identity_result);

                // Lock the mutex and clone the Sdk
                let sdk_instance = {
                    let sdk = sdk_clone.lock().unwrap();
                    sdk.clone() // Assuming Sdk implements Clone
                };
                // Now sdk_instance is owned and can be moved into the async block

                // Spawn the async task
                tokio::spawn(async move {
                    // Parse the identity ID
                    let identity_id = match Identifier::from_string(&identity_id_input, Encoding::Base58) {
                        Ok(id) => id,
                        Err(_) => match Identifier::from_string(&identity_id_input, Encoding::Hex) {
                            Ok(id) => id,
                            Err(e) => {
                                // Store the error and return early
                                let mut identity_result = identity_result_clone.lock().unwrap();
                                *identity_result = Some(Err(format!("Identifier error: {}", e)));
                                return;
                            }
                        },
                    };

                    // Fetch the identity using the cloned sdk_instance
                    let fetch_result = Identity::fetch_by_identifier(&sdk_instance, identity_id).await;

                    let result = match fetch_result {
                        Ok(Some(identity)) => Ok(identity),
                        Ok(None) => Err("Identity not found".to_string()),
                        Err(e) => Err(format!("Error fetching identity: {}", e)),
                    };

                    // Store the result in the shared state
                    let mut identity_result = identity_result_clone.lock().unwrap();
                    *identity_result = Some(result);
                });

                // Store the shared state in the struct
                self.identity_result = Some(identity_result);
            }

            // Handle the promise
            // Handle the result
            if let Some(identity_result) = &self.identity_result {
                let result = identity_result.lock().unwrap().clone();
                if let Some(result) = result {
                    match result {
                        Ok(identity) => {
                            // Insert identity into the database
                            if let Err(e) = self.app_context.insert_identity(&identity) {
                                self.error_message = Some(format!("Database error: {}", e));
                            } else {
                                action = AppAction::PopScreen;
                            }
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                        }
                    }
                    // Clear the shared state
                    self.identity_result = None;
                } else {
                    // The task is still running
                    ui.label("Loading...");
                    // Request a repaint to update the UI
                    ctx.request_repaint();
                }
            }

            if let Some(error) = &self.error_message {
                ui.label(format!("Error: {}", error));
            }
        });

        action
    }
}