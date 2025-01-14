use super::withdraw_from_identity_screen::WithdrawalScreen;
use crate::app::{AppAction, BackendTasksExecutionMode, DesiredAppAction};
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
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use crate::ui::identities::transfers::TransferScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentitiesSortColumn {
    Alias,
    IdentityID,
    InWallet,
    Type,
    Balance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentitiesSortOrder {
    Ascending,
    Descending,
}

#[derive(PartialEq)]
enum IdentitiesRefreshingStatus {
    Refreshing(u64),
    NotRefreshing,
}

pub struct IdentitiesScreen {
    pub identities: Arc<Mutex<IndexMap<Identifier, QualifiedIdentity>>>,
    pub app_context: Arc<AppContext>,
    pub show_more_keys_popup: Option<QualifiedIdentity>,
    pub identity_to_remove: Option<QualifiedIdentity>,
    pub wallet_seed_hash_cache: HashMap<WalletSeedHash, String>,
    sort_column: IdentitiesSortColumn,
    sort_order: IdentitiesSortOrder,
    use_custom_order: bool,
    refreshing_status: IdentitiesRefreshingStatus,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
}

impl IdentitiesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let initial_map: IndexMap<Identifier, QualifiedIdentity> = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();

        let identities = Arc::new(Mutex::new(initial_map));

        let mut screen = Self {
            identities,
            app_context: app_context.clone(),
            show_more_keys_popup: None,
            identity_to_remove: None,
            wallet_seed_hash_cache: Default::default(),
            sort_column: IdentitiesSortColumn::Alias,
            sort_order: IdentitiesSortOrder::Ascending,
            use_custom_order: true,
            refreshing_status: IdentitiesRefreshingStatus::NotRefreshing,
            backend_message: None,
        };

        if let Ok(saved_ids) = screen.app_context.db.load_identity_order() {
            // reorder the IndexMap
            screen.reorder_map_to(saved_ids);
            screen.use_custom_order = true;
        }

        screen
    }

    /// Reorder the underlying IndexMap to match a list of IDs
    fn reorder_map_to(&self, new_order: Vec<Identifier>) {
        let mut lock = self.identities.lock().unwrap();
        for (desired_idx, id) in new_order.iter().enumerate() {
            if let Some(current_idx) = lock.get_index_of(id) {
                if current_idx != desired_idx && current_idx < lock.len() {
                    lock.swap_indices(current_idx, desired_idx);
                }
            }
        }
    }

    fn toggle_sort(&mut self, column: IdentitiesSortColumn) {
        self.use_custom_order = false;
        if self.sort_column == column {
            self.sort_order = match self.sort_order {
                IdentitiesSortOrder::Ascending => IdentitiesSortOrder::Descending,
                IdentitiesSortOrder::Descending => IdentitiesSortOrder::Ascending,
            };
        } else {
            self.sort_column = column;
            self.sort_order = IdentitiesSortOrder::Ascending;
        }
    }

    /// Sorts a list of QIs
    fn sort_vec(&self, list: &mut [QualifiedIdentity]) {
        list.sort_by(|a, b| {
            let ordering = match self.sort_column {
                IdentitiesSortColumn::Alias => {
                    let alias_a = a.alias.as_deref().unwrap_or("");
                    let alias_b = b.alias.as_deref().unwrap_or("");
                    alias_a.cmp(alias_b)
                }
                IdentitiesSortColumn::IdentityID => {
                    let short_a: String = a
                        .identity
                        .id()
                        .to_string(a.identity_type.default_encoding())
                        .chars()
                        .take(6)
                        .collect();
                    let short_b: String = b
                        .identity
                        .id()
                        .to_string(b.identity_type.default_encoding())
                        .chars()
                        .take(6)
                        .collect();
                    short_a.cmp(&short_b)
                }
                IdentitiesSortColumn::InWallet => {
                    // We'll just do a dummy for now
                    let wallet_a = self.wallet_name_for(a);
                    let wallet_b = self.wallet_name_for(b);
                    wallet_a.cmp(&wallet_b)
                }
                IdentitiesSortColumn::Type => a
                    .identity_type
                    .to_string()
                    .cmp(&b.identity_type.to_string()),
                IdentitiesSortColumn::Balance => a.identity.balance().cmp(&b.identity.balance()),
            };
            match self.sort_order {
                IdentitiesSortOrder::Ascending => ordering,
                IdentitiesSortOrder::Descending => ordering.reverse(),
            }
        });
    }

    fn wallet_name_for(&self, qi: &QualifiedIdentity) -> String {
        if let Some(master_identity_public_key) = qi.private_keys.find_master_key() {
            if let Some(wallet_derivation_path) =
                &master_identity_public_key.in_wallet_at_derivation_path
            {
                if let Some(alias) = self
                    .wallet_seed_hash_cache
                    .get(&wallet_derivation_path.wallet_seed_hash)
                {
                    return alias.clone();
                }
            }
        }
        "".to_owned()
    }

    fn show_alias(&self, ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        let placeholder_text = match qualified_identity.identity_type {
            IdentityType::Masternode => "A Masternode",
            IdentityType::Evonode => "An Evonode",
            IdentityType::User => "An Identity",
        };

        let mut alias = qualified_identity.alias.clone().unwrap_or_default();

        let text_edit = egui::TextEdit::singleline(&mut alias)
            .hint_text(placeholder_text)
            .desired_width(100.0);

        if ui.add(text_edit).changed() {
            // If user edits alias, we do not necessarily turn on "custom order."
            // This is a separate property. But we do update the stored alias.
            let mut identities = self.identities.lock().unwrap();
            let identity_to_update = identities
                .get_mut(&qualified_identity.identity.id())
                .unwrap();

            if alias == placeholder_text || alias.is_empty() {
                identity_to_update.alias = None;
            } else {
                identity_to_update.alias = Some(alias);
            }
            match self.app_context.set_alias(
                &identity_to_update.identity.id(),
                identity_to_update.alias.as_ref().map(|s| s.as_str()),
            ) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{}", e);
                }
            }
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
        ui.add(
            egui::Label::new(identifier_as_string)
                .sense(egui::Sense::hover())
                .truncate(),
        )
        .on_hover_text(helper);
    }

    // Up/down reorder methods
    fn move_identity_up(&mut self, identity_id: &Identifier) {
        let mut lock = self.identities.lock().unwrap();
        if let Some(idx) = lock.get_index_of(identity_id) {
            if idx > 0 {
                lock.swap_indices(idx, idx - 1);
            }
        }
        drop(lock);
        self.save_current_order();
    }

    // arrow down
    fn move_identity_down(&mut self, identity_id: &Identifier) {
        let mut lock = self.identities.lock().unwrap();
        if let Some(idx) = lock.get_index_of(identity_id) {
            if idx + 1 < lock.len() {
                lock.swap_indices(idx, idx + 1);
            }
        }
        drop(lock);
        self.save_current_order();
    }

    // Save the current index order to DB
    fn save_current_order(&self) {
        let lock = self.identities.lock().unwrap();
        let all_ids = lock.keys().cloned().collect::<Vec<_>>();
        drop(lock);
        self.app_context.db.save_identity_order(all_ids).ok();
    }

    /// This method merges the ephemeral-sorted `Vec` back into the IndexMap
    /// so the IndexMap is updated to the user’s currently displayed order.
    fn update_index_map_to_current_ephemeral(&self, ephemeral_list: Vec<QualifiedIdentity>) {
        let mut lock = self.identities.lock().unwrap();
        // basically reorder the underlying IndexMap to match ephemeral_list
        for (desired_idx, qi) in ephemeral_list.into_iter().enumerate() {
            let id = qi.identity.id();
            if let Some(current_idx) = lock.get_index_of(&id) {
                if current_idx != desired_idx {
                    lock.swap_indices(current_idx, desired_idx);
                }
            }
        }
    }

    fn find_wallet(&mut self, wallet_seed_hash: &WalletSeedHash) -> Option<String> {
        if let Some(in_wallet_text) = self.wallet_seed_hash_cache.get(wallet_seed_hash) {
            return Some(in_wallet_text.clone());
        }
        let wallets = self.app_context.wallets.read().unwrap();
        for wallet in wallets.values() {
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
        None
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

        ui.add(egui::Label::new(message).sense(egui::Sense::hover()))
            .on_hover_text(format!("{}", qualified_identity.identity.balance()));
    }

    fn show_balance(ui: &mut Ui, qualified_identity: &QualifiedIdentity) {
        let balance_in_dash = qualified_identity.identity.balance() as f64 * 1e-11;
        let formatted_balance = format!("{:.4} DASH", balance_in_dash);
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
        } else {
            Color32::from_rgb(169, 169, 169)
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
            .min_size(egui::Vec2::new(30.0, 18.0));

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
            ui.add_space(20.0);
            ui.label(
                RichText::new("Not Tracking Any Identities")
                    .heading()
                    .size(30.0),
            );
            ui.add_space(10.0);
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
            ui.label(RichText::new("Or").size(22.0).strong());
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

        let mut local_identities = identities.to_vec();
        if !self.use_custom_order {
            self.sort_vec(&mut local_identities);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            Frame::group(ui.style())
                .fill(ui.visuals().panel_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(Align::Center))
                        .column(Column::initial(80.0).resizable(true))   // Name
                        .column(Column::initial(330.0).resizable(true))  // Identity ID
                        .column(Column::initial(60.0).resizable(true))   // In Wallet
                        .column(Column::initial(80.0).resizable(true))   // Type
                        .column(Column::initial(80.0).resizable(true))   // Keys
                        .column(Column::initial(140.0).resizable(true))  // Balance
                        .column(Column::initial(120.0).resizable(true))  // Actions (wider for up/down)
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Name").clicked() {
                                    self.toggle_sort(IdentitiesSortColumn::Alias);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Identity ID").clicked() {
                                    self.toggle_sort(IdentitiesSortColumn::IdentityID);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("In Wallet").clicked() {
                                    self.toggle_sort(IdentitiesSortColumn::InWallet);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Type").clicked() {
                                    self.toggle_sort(IdentitiesSortColumn::Type);
                                }
                            });
                            header.col(|ui| {
                                ui.heading("Keys");
                            });
                            header.col(|ui| {
                                if ui.button("Balance").clicked() {
                                    self.toggle_sort(IdentitiesSortColumn::Balance);
                                }
                            });
                            header.col(|ui| {
                                ui.heading("Actions");
                            });
                        })
                        .body(|mut body| {
                            for qualified_identity in &local_identities {
                                let identity = &qualified_identity.identity;
                                let public_keys = identity.public_keys();
                                let voter_identity_public_keys = qualified_identity
                                    .associated_voter_identity
                                    .as_ref()
                                    .map(|(id, _)| id.public_keys());

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
                                        ui.label(format!("{}", qualified_identity.identity_type));
                                    });
                                    row.col(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 3.0;

                                            let mut total_keys_shown = 0;
                                            let max_keys_to_show = 3;
                                            let mut more_keys_available = false;

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

                                            if let Some(voting_identity_public_keys) =
                                                voter_identity_public_keys
                                            {
                                                if total_keys_shown < max_keys_to_show {
                                                    let voter_vec: Vec<_> = voting_identity_public_keys.iter().collect();
                                                    for (key_id, key) in voter_vec.iter() {
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
                                                } else {
                                                    more_keys_available = true;
                                                }
                                            }

                                            if more_keys_available {
                                                if ui.button("...").clicked() {
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
                                    });
                                    row.col(|ui| {
                                        Self::show_balance(ui, qualified_identity);

                                        ui.spacing_mut().item_spacing.x = 3.0;

                                        if ui.button("Withdraw").clicked() {
                                            action = AppAction::AddScreen(
                                                Screen::WithdrawalScreen(WithdrawalScreen::new(
                                                    qualified_identity.clone(),
                                                    &self.app_context,
                                                )),
                                            );
                                        }
                                        if ui.button("Top up").clicked() {
                                            action = AppAction::AddScreen(
                                                Screen::TopUpIdentityScreen(TopUpIdentityScreen::new(
                                                    qualified_identity.clone(),
                                                    &self.app_context,
                                                )),
                                            );
                                        }
                                        if ui.button("Transfer").clicked() {
                                            action = AppAction::AddScreen(
                                                Screen::TransferScreen(TransferScreen::new(
                                                    qualified_identity.clone(),
                                                    &self.app_context,
                                                )),
                                            );
                                        }
                                    });
                                    row.col(|ui| {
                                        ui.spacing_mut().item_spacing.x = 3.0;

                                        ui.horizontal(|ui| {
                                            // Refresh
                                            if ui.button("Refresh").clicked() {
                                                action = AppAction::BackendTask(
                                                    BackendTask::IdentityTask(
                                                        IdentityTask::RefreshIdentity(
                                                            qualified_identity.clone(),
                                                        ),
                                                    ),
                                                );
                                            }

                                            // Remove
                                            if ui.button("Remove").clicked() {
                                                self.identity_to_remove =
                                                    Some(qualified_identity.clone());
                                            }
                                        });

                                        ui.horizontal(|ui| {
                                            // Up arrow
                                            let up_btn = ui.button("⬆");
                                            // Down arrow
                                            let down_btn = ui.button("⬇");

                                            if up_btn.clicked() {
                                                // If we are currently sorted (not custom),
                                                // unify the IndexMap to reflect that ephemeral sort
                                                if !self.use_custom_order {
                                                    self.update_index_map_to_current_ephemeral(local_identities.clone());
                                                }
                                                // Now do the swap
                                                self.use_custom_order = true;
                                                self.move_identity_up(&identity.id());
                                            }
                                            if down_btn.clicked() {
                                                if !self.use_custom_order {
                                                    self.update_index_map_to_current_ephemeral(local_identities.clone());
                                                }
                                                self.use_custom_order = true;
                                                self.move_identity_down(&identity.id());
                                            }
                                        });
                                    });
                                });
                            }
                        });
                });
        });

        action
    }

    fn show_identity_to_remove(&mut self, ctx: &Context) {
        if let Some(identity_to_remove) = self.identity_to_remove.clone() {
            egui::Window::new("Confirm Removal")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Are you sure you want to no longer track this {} identity?",
                        identity_to_remove.identity_type
                    ));
                    ui.label(format!(
                        "Identity ID: {}",
                        identity_to_remove
                            .identity
                            .id()
                            .to_string(identity_to_remove.identity_type.default_encoding())
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            let identity_id = identity_to_remove.identity.id();
                            let mut lock = self.identities.lock().unwrap();
                            lock.shift_remove(&identity_id);

                            self.app_context
                                .db
                                .delete_local_qualified_identity(&identity_id, &self.app_context)
                                .ok();

                            if let Some((voter_identity, _)) =
                                &identity_to_remove.associated_voter_identity
                            {
                                let voter_identity_id = voter_identity.id();
                                self.app_context
                                    .db
                                    .delete_local_qualified_identity(
                                        &voter_identity_id,
                                        &self.app_context,
                                    )
                                    .ok();
                            }

                            self.identity_to_remove = None;
                        }
                        if ui.button("No").clicked() {
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

        let identity = &qualified_identity.identity;
        let public_keys = identity.public_keys();
        let public_keys_vec: Vec<_> = public_keys.iter().collect();
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

        if let Some((voter_identity, _)) = qualified_identity.associated_voter_identity.as_ref() {
            let voter_public_keys = voter_identity.public_keys();
            let voter_public_keys_vec: Vec<_> = voter_public_keys.iter().collect();

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

        if ui.button("Close").clicked() {
            self.show_more_keys_popup = None;
        }

        action
    }

    fn dismiss_message(&mut self) {
        self.backend_message = None;
    }

    fn check_message_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.backend_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            // Automatically dismiss the message after 10 seconds
            if elapsed.num_seconds() >= 10 {
                self.dismiss_message();
            }
        }
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
            .map(|qi| (qi.identity.id(), qi))
            .collect();

        self.show_more_keys_popup = None;
    }

    fn display_message(&mut self, message: &str, message_type: crate::ui::MessageType) {
        if message.contains("Error refreshing identities")
            || message.contains("Successfully refreshed identity")
        {
            self.refreshing_status = IdentitiesRefreshingStatus::NotRefreshing;
        }
        self.backend_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut right_buttons = if !self.app_context.has_wallet.load(Ordering::Relaxed) {
            vec![
                (
                    "Import Wallet",
                    DesiredAppAction::AddScreenType(ScreenType::ImportWallet),
                ),
                (
                    "Create Wallet",
                    DesiredAppAction::AddScreenType(ScreenType::AddNewWallet),
                ),
            ]
        } else {
            vec![(
                "Create Identity",
                DesiredAppAction::AddScreenType(ScreenType::AddNewIdentity),
            )]
        };
        right_buttons.push((
            "Load Identity",
            DesiredAppAction::AddScreenType(ScreenType::AddExistingIdentity),
        ));
        if self.identities.lock().unwrap().len() > 0 {
            // Create a vec of RefreshIdentity(identity) DesiredAppAction for each identity
            let backend_tasks: Vec<BackendTask> = self
                .identities
                .lock()
                .unwrap()
                .values()
                .map(|qi| BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi.clone())))
                .collect();
            right_buttons.push((
                "Refresh All",
                DesiredAppAction::BackendTasks(
                    backend_tasks,
                    BackendTasksExecutionMode::Concurrent,
                ),
            ));
        }

        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Identities", AppAction::None)],
            right_buttons,
        );

        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenIdentities);

        let identities_vec = {
            let guard = self.identities.lock().unwrap();
            guard.values().cloned().collect::<Vec<_>>()
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            if identities_vec.is_empty() {
                self.render_no_identities_view(ui);
            } else {
                action |= self.render_identities_view(ui, &identities_vec);
            }

            // If we are refreshing, show a spinner at the bottom
            if let IdentitiesRefreshingStatus::Refreshing(start_time) = self.refreshing_status {
                ui.add_space(5.0);
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(format!("Refreshing... Time taken so far: {}", elapsed));
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
                ui.add_space(10.0);
            }

            let message = self.backend_message.clone();
            if let Some((message, message_type, timestamp)) = message {
                let message_color = match message_type {
                    MessageType::Error => egui::Color32::DARK_RED,
                    MessageType::Info => egui::Color32::BLACK,
                    MessageType::Success => egui::Color32::DARK_GREEN,
                };

                ui.add_space(10.0);
                ui.allocate_ui(egui::Vec2::new(ui.available_width(), 30.0), |ui| {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(message).color(message_color));
                            let now = Utc::now();
                            let elapsed = now.signed_duration_since(timestamp);
                            if ui
                                .button(format!("Dismiss ({})", 10 - elapsed.num_seconds()))
                                .clicked()
                            {
                                // Update the state outside the closure
                                self.dismiss_message();
                            }
                        });
                    });
                });
                ui.add_space(10.0);
            }
        });

        if self.show_more_keys_popup.is_some() {
            egui::Window::new("More Keys")
                .collapsible(false)
                .show(ctx, |ui| {
                    action |= self.show_more_keys(ui);
                });
        }

        if self.identity_to_remove.is_some() {
            self.show_identity_to_remove(ctx);
        }

        match action {
            AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RefreshIdentity(_))) => {
                self.refreshing_status =
                    IdentitiesRefreshingStatus::Refreshing(Utc::now().timestamp() as u64)
            }
            AppAction::BackendTasks(_, _) => {
                // Going to assume this is only going to be Refresh All
                self.refreshing_status =
                    IdentitiesRefreshingStatus::Refreshing(Utc::now().timestamp() as u64)
            }
            _ => {}
        }

        action
    }
}
