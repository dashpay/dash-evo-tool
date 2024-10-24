use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::qualified_identity::EncryptedPrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::platform::identity::IdentityTask;
use crate::platform::BackendTask;
use crate::ui::add_key_screen::AddKeyScreen;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::key_info_screen::KeyInfoScreen;
use crate::ui::withdrawals::WithdrawalScreen;
use crate::ui::{RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use eframe::egui::{self, Context};
use eframe::emath::Align;
use egui::{Color32, Frame, Margin, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, Mutex};

pub struct IdentitiesScreen {
    pub identities: Arc<Mutex<Vec<QualifiedIdentity>>>,
    pub app_context: Arc<AppContext>,
}

impl IdentitiesScreen {
    fn show_alias(ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        if let Some(alias) = qualified_identity.alias.as_ref() {
            ui.label(alias.clone());
        }
    }
    fn show_identity_id(ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        let (encoding, helper) = match qualified_identity.identity_type {
            IdentityType::User => (Encoding::Base58, "UserId".to_string()),
            IdentityType::Masternode | IdentityType::Evonode => {
                (Encoding::Hex, "ProTxHash".to_string())
            }
        };
        let identifier_as_string = qualified_identity.identity.id().to_string(encoding);
        ui.add(egui::Label::new(identifier_as_string).sense(egui::Sense::hover()))
            .on_hover_text(helper);
    }
    fn show_balance(ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        // Calculate the balance in DASH (10^-11 conversion)
        let balance_in_dash = qualified_identity.identity.balance() as f64 * 1e-11;

        // Format the balance with 4 decimal places
        let formatted_balance = format!("{:.4} DASH", balance_in_dash);

        // Add the label with hover text
        ui.add(egui::Label::new(formatted_balance).sense(egui::Sense::hover()))
            .on_hover_text(format!("{}", qualified_identity.identity.balance()));
    }

    fn show_public_key(
        &self,
        ui: &mut Ui,
        identity: &QualifiedIdentity,
        key: &IdentityPublicKey,
        encrypted_private_key: Option<&[u8; 32]>,
    ) -> AppAction {
        let button_color = if encrypted_private_key.is_some() {
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
            Purpose::TRANSFER => format!("T{}", key.id()),
            Purpose::SYSTEM => format!("S{}", key.id()),
            Purpose::VOTING => format!("V{}", key.id()),
            Purpose::OWNER => format!("O{}", key.id()),
        };

        let button = egui::Button::new(name)
            .fill(button_color)
            .frame(true)
            .rounding(3.0)
            .min_size(egui::vec2(80.0, 30.0));

        if ui.add(button).clicked() {
            AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                identity.clone(),
                key.clone(),
                encrypted_private_key.cloned(),
                &self.app_context,
            )))
        } else {
            AppAction::None
        }
    }

    fn render_no_identities_view(&self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
                ui.add_space(20.0); // Add some space from the top

                // Use a larger font for the heading
                ui.label(
                    RichText::new("Not Tracking Any Identities")
                        .heading()
                        .size(30.0), // Larger size for heading
                );

                ui.add_space(10.0);

                // Use larger text for the labels
                ui.label(
                    RichText::new(
                        "It looks like you are not tracking any Identities, Evonodes or Masternodes yet.",
                    )
                        .size(20.0),
                );

                ui.add_space(30.0);

                ui.label(
                    RichText::new(
                        "* You can load an Evonode/Masternode/Identity by clicking on \"Load Identity\" on the top right of the screen.",
                    )
                        .size(18.0),
                );

                ui.add_space(10.0);

                ui.label(RichText::new("Or").size(22.0).strong()); // Emphasized text

                ui.add_space(10.0);

                ui.label(
                    RichText::new(
                        "* You can create a wallet and then register an Identity on Dash Evo.",
                    )
                        .size(18.0),
                );

                ui.add_space(30.0);

                ui.label(
                    RichText::new(
                        "(Make sure Dash Core is running, you can check in the settings tab on the left)",
                    )
                        .size(18.0),
                );
            });
    }

    fn render_identities_view(&self, ui: &mut Ui, identities: &[QualifiedIdentity]) -> AppAction {
        let mut action = AppAction::None;

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
                        .column(Column::initial(40.0).resizable(true)) // Name
                        .column(Column::initial(200.0).resizable(true)) // Identity ID
                        .column(Column::initial(100.0).resizable(true)) // Balance
                        .column(Column::initial(100.0).resizable(true)) // Type
                        .column(Column::initial(80.0).resizable(true)) // Refresh
                        .column(Column::initial(80.0).resizable(true)) // Keys
                        .column(Column::initial(80.0).resizable(true)) // Withdraw
                        // .column(Column::initial(80.0).resizable(true)) // Transfer
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.heading("Name");
                            });
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
                                ui.heading("Refresh");
                            });
                            header.col(|ui| {
                                ui.heading("Keys");
                            });
                            header.col(|ui| {
                                ui.heading("Withdraw");
                            });
                            // header.col(|ui| {
                            //     ui.heading("Transfer");
                            // });
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
                                        Self::show_alias(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        Self::show_identity_id(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        Self::show_balance(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        ui.label(format!("{}", qualified_identity.identity_type));
                                    });
                                    row.col(|ui| {
                                        if ui.button("Refresh").clicked() {
                                            action =
                                                AppAction::BackendTask(BackendTask::IdentityTask(
                                                    IdentityTask::RefreshIdentity(
                                                        qualified_identity.clone(),
                                                    ),
                                                ));
                                        }
                                    });
                                    row.col(|ui| {
                                        for (key_id, key) in public_keys.iter() {
                                            let holding_private_key = qualified_identity
                                                .encrypted_private_keys
                                                .get(&(PrivateKeyOnMainIdentity, *key_id))
                                                .map(|(_, p)| p);
                                            action |= self.show_public_key(
                                                ui,
                                                qualified_identity,
                                                key,
                                                holding_private_key,
                                            );
                                        }
                                        if let Some(voting_identity_public_keys) =
                                            voter_identity_public_keys
                                        {
                                            for (key_id, key) in voting_identity_public_keys.iter()
                                            {
                                                let holding_private_key = qualified_identity
                                                    .encrypted_private_keys
                                                    .get(&(PrivateKeyOnVoterIdentity, *key_id))
                                                    .map(|(_, p)| p);
                                                action |= self.show_public_key(
                                                    ui,
                                                    qualified_identity,
                                                    key,
                                                    holding_private_key,
                                                );
                                            }
                                        }
                                        if qualified_identity.can_sign_with_master_key().is_some()
                                            && ui.button("Add Key").clicked()
                                        {
                                            action = AppAction::AddScreen(Screen::AddKeyScreen(
                                                AddKeyScreen::new(
                                                    qualified_identity.clone(),
                                                    &self.app_context,
                                                ),
                                            ));
                                        }
                                    });
                                    row.col(|ui| {
                                        if ui.button("Withdraw").clicked() {
                                            action = AppAction::AddScreen(
                                                Screen::WithdrawalScreen(WithdrawalScreen::new(
                                                    qualified_identity.clone(),
                                                    &self.app_context,
                                                )),
                                            );
                                        }
                                    });
                                    // row.col(|ui| {
                                    //     if ui.button("Transfer").clicked() {
                                    //         action = AppAction::AddScreen(Screen::TransferScreen(
                                    //             TransferScreen::new(
                                    //                 qualified_identity.clone(),
                                    //                 &self.app_context,
                                    //             ),
                                    //         ));
                                    //     }
                                    // });
                                });
                            }
                        });
                });
        });

        action
    }
}

impl ScreenLike for IdentitiesScreen {
    fn refresh(&mut self) {
        let mut identities = self.identities.lock().unwrap();
        *identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let right_buttons = {
            // Acquire a read lock on wallets
            // let create_wallet_or_identity = if !self.app_context.has_wallet.load(Ordering::Relaxed)
            // {
            //     (
            //         "Create Wallet",
            //         DesiredAppAction::AddScreenType(ScreenType::AddNewWallet),
            //     )
            // } else {
            //     (
            //         "Create Identity",
            //         DesiredAppAction::AddScreenType(ScreenType::AddNewIdentity),
            //     )
            // };
            vec![
                // create_wallet_or_identity,
                (
                    "Load Identity",
                    DesiredAppAction::AddScreenType(ScreenType::AddExistingIdentity),
                ),
            ]
        };
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            right_buttons,
        );

        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenIdentities);

        let identities = self.identities.lock().unwrap();

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            if identities.is_empty() {
                self.render_no_identities_view(ui);
            } else {
                action |= self.render_identities_view(ui, identities.as_slice());
            }
        });

        action
    }
}

impl IdentitiesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let identities = Arc::new(Mutex::new(
            app_context
                .load_local_qualified_identities()
                .unwrap_or_default(),
        ));
        Self {
            identities,
            app_context: app_context.clone(),
        }
    }
}
