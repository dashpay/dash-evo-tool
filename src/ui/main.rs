use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::qualified_identity::EncryptedPrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{ScreenLike, ScreenType};
use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::{KeyID, Purpose};
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::IdentityPublicKey;
use eframe::egui::{self, Context};
use eframe::emath::Align;
use egui::{Color32, Frame, Margin, Ui};
use egui_extras::{Column, TableBuilder};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub struct MainScreen {
    identities: Arc<Mutex<Vec<QualifiedIdentity>>>,
    app_context: Arc<AppContext>,
}

impl ScreenLike for MainScreen {
    fn refresh(&mut self) {
        let mut identities = self.identities.lock().unwrap();
        *identities = self.app_context.load_identities().unwrap_or_default();
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Home", AppAction::None)],
            Some((
                "Add Identity",
                DesiredAppAction::AddScreenType(ScreenType::AddIdentity),
            )),
        );

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            let identities = self.identities.lock().unwrap();

            egui::ScrollArea::vertical().show(ui, |ui| {
                // Define a frame with custom background color and border
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill) // Use panel fill color
                    .stroke(egui::Stroke::new(
                        1.0,
                        ui.visuals().widgets.inactive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        // Build the table
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            // Define columns with resizing and alignment
                            .column(Column::initial(200.0).resizable(true)) // Identity ID
                            .column(Column::initial(100.0).resizable(true)) // Balance
                            .column(Column::initial(100.0).resizable(true)) // Type
                            .column(Column::initial(80.0).resizable(true)) // Keys
                            .column(Column::initial(80.0).resizable(true)) // Withdraw
                            .column(Column::initial(80.0).resizable(true)) // Transfer
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    ui.heading("Identity ID");
                                });
                                header.col(|ui| {
                                    ui.heading("Balance");
                                });
                                header.col(|ui| {
                                    ui.heading("Type");
                                });
                                header.col(|ui| {
                                    ui.heading("Keys");
                                });
                                header.col(|ui| {
                                    ui.heading("Withdraw");
                                });
                                header.col(|ui| {
                                    ui.heading("Transfer");
                                });
                            })
                            .body(|mut body| {
                                for qualified_identity in identities.iter() {
                                    let identity = &qualified_identity.identity;
                                    let public_keys = identity.public_keys();
                                    let voter_identity_public_keys = qualified_identity
                                        .associated_voter_identity
                                        .as_ref()
                                        .map(|(identity, _)| identity.public_keys());
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            let encoding = match qualified_identity.identity_type {
                                                IdentityType::User => Encoding::Base58,
                                                IdentityType::Masternode
                                                | IdentityType::Evonode => Encoding::Hex,
                                            };
                                            ui.label(format!(
                                                "{}",
                                                identity.id().to_string(encoding)
                                            ));
                                        });
                                        row.col(|ui| {
                                            ui.label(format!("{}", identity.balance()));
                                        });
                                        row.col(|ui| {
                                            ui.label(format!(
                                                "{}",
                                                qualified_identity.identity_type
                                            ));
                                        });
                                        row.col(|ui| {
                                            fn show_public_key(
                                                ui: &mut Ui,
                                                key: &IdentityPublicKey,
                                                holding_private_key: bool,
                                            ) {
                                                let button_color = if holding_private_key {
                                                    Color32::from_rgb(167, 232, 232)
                                                // Light green-blue color if private key exists
                                                } else {
                                                    Color32::from_rgb(169, 169, 169)
                                                    // Gray if no private key
                                                };

                                                let name = match key.purpose() {
                                                    Purpose::AUTHENTICATION => {
                                                        format!("A{}", key.id())
                                                    }
                                                    Purpose::ENCRYPTION => {
                                                        format!("En{}", key.id())
                                                    }
                                                    Purpose::DECRYPTION => {
                                                        format!("De{}", key.id())
                                                    }
                                                    Purpose::TRANSFER => format!("W{}", key.id()),
                                                    Purpose::SYSTEM => format!("S{}", key.id()),
                                                    Purpose::VOTING => format!("V{}", key.id()),
                                                };

                                                let button = egui::Button::new(name)
                                                    .fill(button_color)
                                                    .frame(true)
                                                    .rounding(3.0)
                                                    .min_size(egui::vec2(80.0, 30.0));

                                                if ui.add(button).clicked() {
                                                    // Implement action when the key button is clicked
                                                    // For example, navigate to a detailed key view or perform another action
                                                }
                                            }
                                            for (key_id, key) in public_keys.iter() {
                                                let holding_private_key = qualified_identity
                                                    .encrypted_private_keys
                                                    .contains_key(&(
                                                        PrivateKeyOnMainIdentity,
                                                        *key_id,
                                                    ));
                                                show_public_key(ui, key, holding_private_key);
                                            }
                                            if let Some(voting_identity_public_keys) =
                                                voter_identity_public_keys
                                            {
                                                for (key_id, key) in
                                                    voting_identity_public_keys.iter()
                                                {
                                                    let holding_private_key = qualified_identity
                                                        .encrypted_private_keys
                                                        .contains_key(&(
                                                            PrivateKeyOnVoterIdentity,
                                                            *key_id,
                                                        ));
                                                    show_public_key(ui, key, holding_private_key);
                                                }
                                            }
                                        });
                                        row.col(|ui| {
                                            if ui.button("Withdraw").clicked() {
                                                // Implement Withdraw functionality
                                            }
                                        });
                                        row.col(|ui| {
                                            if ui.button("Transfer").clicked() {
                                                // Implement Transfer functionality
                                            }
                                        });
                                    });
                                }
                            });
                    });
            });
        });

        action
    }
}

impl MainScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let identities = Arc::new(Mutex::new(
            app_context.load_identities().unwrap_or_default(),
        ));
        Self {
            identities,
            app_context: app_context.clone(),
        }
    }
}
