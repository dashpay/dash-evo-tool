use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use eframe::egui::Context;
use std::sync::Arc;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use egui::TextBuffer;

pub struct UpdateIdentityPayoutScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    payout_address_private_key_input: String,
    error_message: Option<String>,
}

impl UpdateIdentityPayoutScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            identity,
            payout_address_private_key_input: String::new(),
            error_message: None,
        }
    }

    fn verify_key_input(
        untrimmed_private_key: String,
        type_key: &str,
    ) -> Result<Option<[u8; 32]>, String> {
        let private_key = untrimmed_private_key.trim().to_string();
        match private_key.len() {
            64 => {
                // hex
                match hex::decode(private_key.as_str()) {
                    Ok(decoded) => Ok(Some(decoded.try_into().unwrap())),
                    Err(_) => Err(format!(
                        "{} key is the size of a hex key but isn't hex",
                        type_key
                    )),
                }
            }
            51 | 52 => {
                // wif
                match PrivateKey::from_wif(private_key.as_str()) {
                    Ok(key) => Ok(Some(key.inner.secret_bytes())),
                    Err(_) => Err(format!(
                        "{} key is the length of a WIF key but is invalid",
                        type_key
                    )),
                }
            }
            0 => Ok(None),
            _ => Err(format!("{} key is of incorrect size", type_key)),
        }
    }
}

impl ScreenLike for UpdateIdentityPayoutScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {

    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Update Payout Address", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            if (self.identity.identity_type == IdentityType::User) {
                ui.heading("Updating Payout Address for User identities is not allowed.".to_string());
            }
            else {
                ui.heading("Update Payout Address".to_string());
            }

            //let owner_key = self.identity.identity.get_first_public_key_matching(Purpose::OWNER, SecurityLevel::full_range().into(), KeyType::all_key_types().into(), false);
            let payout_address_private_key_input = &mut self.payout_address_private_key_input;
            ui.horizontal(|ui| {
                ui.label("Payout Address Private Key:");
                ui.text_edit_singleline(payout_address_private_key_input);
            });

            if ui.button("Update Payout Address").clicked() {
                match Self::verify_key_input(payout_address_private_key_input.clone(), "test".as_str()) {
                    Ok(value) => {
                        println!("Success");
                    }
                    Err(error) => {
                        eprintln!("Error: {}", error);
                    }
                }
            }
        });
        
        action
    }
}
