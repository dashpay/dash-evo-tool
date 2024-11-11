use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::PrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::wallet::WalletSeedHash;
use crate::ui::add_key_screen::AddKeyScreen;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::key_info_screen::KeyInfoScreen;
use crate::ui::transfers::TransferScreen;
use crate::ui::withdrawals::WithdrawalScreen;
use crate::ui::{RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use dash_sdk::platform::Identifier;
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Context};
use eframe::emath::Align;
use egui::{Color32, Frame, Margin, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub struct IdentitiesScreen {
    pub identities: Arc<Mutex<IndexMap<Identifier, QualifiedIdentity>>>,
    pub app_context: Arc<AppContext>,
    pub show_more_keys_popup: Option<QualifiedIdentity>,
    pub identity_to_remove: Option<QualifiedIdentity>,
    pub wallet_seed_hash_cache: HashMap<WalletSeedHash, String>,
}

impl IdentitiesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let identities = Arc::new(Mutex::new(
            app_context
                .load_local_qualified_identities()
                .unwrap_or_default()
                .into_iter()
                .map(|qualified_identity| (qualified_identity.identity.id(), qualified_identity))
                .collect(),
        ));
        Self {
            identities,
            app_context: app_context.clone(),
            show_more_keys_popup: None,
            identity_to_remove: None,
            wallet_seed_hash_cache: Default::default(),
        }
    }

    fn show_alias(&self, ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        let placeholder_text = match qualified_identity.identity_type {
            IdentityType::Masternode => "A Masternode",
            IdentityType::Evonode => "An Evonode",
            IdentityType::User => "An Identity",
        };

        // Check if alias is set, otherwise use placeholder text
        let mut alias = qualified_identity.alias.clone().unwrap_or_default();

        // Render the text field with placeholder functionality
        let text_edit = egui::TextEdit::singleline(&mut alias)
            .hint_text(placeholder_text)
            .desired_width(100.0); // Adjust width as needed

        if ui.add(text_edit).changed() {
            let mut identities = self.identities.lock().unwrap();
            let identity_to_update = identities
                .get_mut(&qualified_identity.identity.id())
                .unwrap();

            // Update the alias if the user changed the text
            if alias == placeholder_text || alias == "" {
                identity_to_update.alias = None;
            } else {
                identity_to_update.alias = Some(alias);
            }
            self.app_context
                .db
                .set_alias(
                    &identity_to_update.identity.id(),
                    identity_to_update.alias.as_ref().map(|s| s.as_str()),
                )
                .ok();
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

    fn find_wallet(&mut self, wallet_seed_hash: &WalletSeedHash) -> Option<String> {
        if let Some(in_wallet_text) = self.wallet_seed_hash_cache.get(wallet_seed_hash) {
            return Some(in_wallet_text.clone());
        }
        let wallets = self.app_context.wallets.read().unwrap();
        for wallet in wallets.iter() {
            let wallet_guard = wallet.read().unwrap();
            if &wallet_guard.seed_hash() == wallet_seed_hash {
                let in_wallet_text = if let Some(alias) = wallet_guard.alias.as_ref() {
                    alias.clone()
                } else {
                    hex::encode(wallet_guard.seed_hash())
                        .split_at(5)
                        .0
                        .to_string()
                };
                self.wallet_seed_hash_cache
                    .insert(*wallet_seed_hash, in_wallet_text.clone());
                return Some(in_wallet_text);
            }
        }
        return None;
    }

    fn show_in_wallet(&mut self, ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        let master_identity_public_key = qualified_identity.private_keys.find_master_key();

        let message = match master_identity_public_key {
            None => "".to_string(),
            Some(qualified_identity_public_key) => {
                match qualified_identity_public_key
                    .in_wallet_at_derivation_path
                    .as_ref()
                {
                    None => "".to_string(),
                    Some(wallet_derivation_path) => self
                        .find_wallet(&wallet_derivation_path.wallet_seed_hash)
                        .unwrap_or_default(),
                }
            }
        };

        // Add the label with hover text
        ui.add(egui::Label::new(message).sense(egui::Sense::hover()))
            .on_hover_text(format!("{}", qualified_identity.identity.balance()));
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
        encrypted_private_key: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
    ) -> AppAction {
        let button_color = if encrypted_private_key.is_some() {
            Color32::from_rgb(167, 232, 232)
            // Light green-blue color if private key exists
        } else {
            Color32::from_rgb(169, 169, 169)
            // Gray if no private key
        };

        let name = match key.purpose() {
            Purpose::AUTHENTICATION => format!("A{}", key.id()),
            Purpose::ENCRYPTION => format!("En{}", key.id()),
            Purpose::DECRYPTION => format!("De{}", key.id()),
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
                encrypted_private_key,
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

    fn render_identities_view(
        &mut self,
        ui: &mut Ui,
        identities: &[QualifiedIdentity],
    ) -> AppAction {
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
                        .column(Column::initial(60.0).resizable(true)) // Name
                        .column(Column::initial(100.0).resizable(true)) // Identity ID
                        .column(Column::initial(60.0).resizable(true)) // In Wallet
                        .column(Column::initial(100.0).resizable(true)) // Balance
                        .column(Column::initial(80.0).resizable(true)) // Type
                        .column(Column::initial(80.0).resizable(true)) // Keys
                        .column(Column::initial(80.0).resizable(true)) // Withdraw
                        .column(Column::initial(80.0).resizable(true)) // Transfer
                        .column(Column::initial(80.0).resizable(true)) // Actions
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.heading("Name");
                            });
                            header.col(|ui| {
                                ui.heading("Identity ID");
                            });
                            header.col(|ui| {
                                ui.heading("In Wallet");
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
                            header.col(|ui| {
                                ui.heading("Actions");
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
                                        self.show_alias(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        Self::show_identity_id(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        self.show_in_wallet(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        Self::show_balance(ui, qualified_identity);
                                    });
                                    row.col(|ui| {
                                        ui.label(format!("{}", qualified_identity.identity_type));
                                    });
                                    row.col(|ui| {
                                        let mut total_keys_shown = 0;
                                        let max_keys_to_show = 3;
                                        let mut more_keys_available = false;

                                        // Show keys from main identity
                                        let public_keys_vec: Vec<_> = public_keys.iter().collect();
                                        for (key_id, key) in public_keys_vec.iter() {
                                            if total_keys_shown < max_keys_to_show {
                                                let holding_private_key = qualified_identity
                                                    .private_keys
                                                    .get_cloned_private_key_data_and_wallet_info(&(
                                                        PrivateKeyOnMainIdentity,
                                                        **key_id,
                                                    ));
                                                action |= self.show_public_key(
                                                    ui,
                                                    qualified_identity,
                                                    *key,
                                                    holding_private_key,
                                                );
                                                total_keys_shown += 1;
                                            } else {
                                                more_keys_available = true;
                                                break;
                                            }
                                        }

                                        // If we have not reached max keys, show keys from voter identity
                                        if total_keys_shown < max_keys_to_show {
                                            if let Some(voting_identity_public_keys) =
                                                voter_identity_public_keys
                                            {
                                                let voter_public_keys_vec: Vec<_> =
                                                    voting_identity_public_keys.iter().collect();
                                                for (key_id, key) in voter_public_keys_vec.iter() {
                                                    if total_keys_shown < max_keys_to_show {
                                                        let holding_private_key =
                                                            qualified_identity
                                                                .private_keys
                                                                .get_cloned_private_key_data_and_wallet_info(&(
                                                                    PrivateKeyOnVoterIdentity,
                                                                    **key_id,
                                                                ));
                                                        action |= self.show_public_key(
                                                            ui,
                                                            qualified_identity,
                                                            *key,
                                                            holding_private_key,
                                                        );
                                                        total_keys_shown += 1;
                                                    } else {
                                                        more_keys_available = true;
                                                        break;
                                                    }
                                                }
                                            }
                                        }

                                        // If there are more keys, show "View More" button
                                        if more_keys_available {
                                            if ui.button("View More").clicked() {
                                                self.show_more_keys_popup =
                                                    Some(qualified_identity.clone());
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
                                    row.col(|ui| {
                                        if ui.button("Transfer").clicked() {
                                            action = AppAction::AddScreen(Screen::TransferScreen(
                                                TransferScreen::new(
                                                    qualified_identity.clone(),
                                                    &self.app_context,
                                                ),
                                            ));
                                        }
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
                                        if ui.button("Remove").clicked() {
                                            self.identity_to_remove =
                                                Some(qualified_identity.clone());
                                        }
                                    });
                                });
                            }
                        });
                });
        });

        action
    }

    fn show_identity_to_remove(&mut self, ctx: &Context) {
        // Check if we need to show the confirmation dialog
        if let Some(identity_to_remove) = self.identity_to_remove.clone() {
            egui::Window::new("Confirm Removal")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("Are you sure you want to no longer track this {} identity? It will still be on the blockchain.", identity_to_remove.identity_type));
                    ui.label(format!(
                        "Identity ID: {}",
                        identity_to_remove
                            .identity
                            .id()
                            .to_string(identity_to_remove.identity_type.default_encoding())
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            // Remove the identity
                            let identity_id = identity_to_remove.identity.id();

                            // Remove from in-memory identities
                            {
                                let mut identities_lock = self.identities.lock().unwrap();
                                identities_lock.shift_remove(&identity_id);
                            }

                            // Remove from persistent storage
                            self.app_context
                                .db
                                .delete_local_qualified_identity(&identity_id, &self.app_context).ok();

                            // Also remove associated voter identity, if any
                            if let Some((voter_identity, _)) =
                                &identity_to_remove.associated_voter_identity
                            {
                                let voter_identity_id = voter_identity.id();

                                self.app_context
                                    .db
                                    .delete_local_qualified_identity(&voter_identity_id, &self.app_context)
                                    .ok();
                            }

                            // Clear the identity_to_remove
                            self.identity_to_remove = None;
                        }
                        if ui.button("No").clicked() {
                            // Cancel the removal
                            self.identity_to_remove = None;
                        }
                    });
                });
        }
    }

    fn show_more_keys(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let Some(qualified_identity) = self.show_more_keys_popup.as_ref() else {
            return action;
        };

        // Get keys from main identity
        let identity = &qualified_identity.identity;
        let public_keys = identity.public_keys();
        let public_keys_vec: Vec<_> = public_keys.iter().collect();

        // Skip the first three keys
        let main_identity_rest_keys = public_keys_vec.iter().skip(3);

        ui.label(format!(
            "{}...",
            identity
                .id()
                .to_string(Encoding::Base58)
                .chars()
                .take(8)
                .collect::<String>()
        ));
        for (key_id, key) in main_identity_rest_keys {
            let holding_private_key = qualified_identity
                .private_keys
                .get_cloned_private_key_data_and_wallet_info(&(PrivateKeyOnMainIdentity, **key_id));
            action |= self.show_public_key(ui, qualified_identity, *key, holding_private_key);
        }

        // Get keys from voter identity
        if let Some((voter_identity, _)) = qualified_identity.associated_voter_identity.as_ref() {
            let voter_public_keys = voter_identity.public_keys();
            let voter_public_keys_vec: Vec<_> = voter_public_keys.iter().collect();

            // Assuming we want to show all keys from voter identity
            ui.label("Voter Identity Keys:");
            for (key_id, key) in voter_public_keys_vec.iter() {
                let holding_private_key = qualified_identity
                    .private_keys
                    .get_cloned_private_key_data_and_wallet_info(&(
                        PrivateKeyOnVoterIdentity,
                        **key_id,
                    ));
                action |= self.show_public_key(ui, qualified_identity, *key, holding_private_key);
            }
        }

        // Close button
        if ui.button("Close").clicked() {
            self.show_more_keys_popup = None;
        }

        action
    }
}

impl ScreenLike for IdentitiesScreen {
    fn refresh(&mut self) {
        let mut identities = self.identities.lock().unwrap();
        *identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qualified_identity| (qualified_identity.identity.id(), qualified_identity))
            .collect();

        self.show_more_keys_popup = None;
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut right_buttons = if !self.app_context.has_wallet.load(Ordering::Relaxed) {
            [
                (
                    "Import Wallet",
                    DesiredAppAction::AddScreenType(ScreenType::ImportWallet),
                ),
                (
                    "Create Wallet",
                    DesiredAppAction::AddScreenType(ScreenType::AddNewWallet),
                ),
            ]
            .to_vec()
        } else {
            [(
                "Create Identity",
                DesiredAppAction::AddScreenType(ScreenType::AddNewIdentity),
            )]
            .to_vec()
        };
        right_buttons.push((
            "Load Identity",
            DesiredAppAction::AddScreenType(ScreenType::AddExistingIdentity),
        ));
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            right_buttons,
        );

        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenIdentities);

        // Limit the scope of the MutexGuard by enclosing in a block
        let identities = {
            let identities_guard = self.identities.lock().unwrap();
            identities_guard.values().cloned().collect::<Vec<_>>()
        };

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            if identities.is_empty() {
                self.render_no_identities_view(ui);
            } else {
                action |= self.render_identities_view(ui, identities.as_slice());
            }
        });

        // Check if we need to show the pop-up
        if self.show_more_keys_popup.is_some() {
            egui::Window::new("More Keys")
                .collapsible(false)
                .show(ctx, |ui| {
                    action |= self.show_more_keys(ui);
                });
        }

        // Check if we need to show the confirmation dialog
        if self.identity_to_remove.is_some() {
            self.show_identity_to_remove(ctx);
        }

        action
    }
}
