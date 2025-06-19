use super::withdraw_screen::WithdrawalScreen;
use crate::app::{AppAction, BackendTasksExecutionMode, DesiredAppAction};
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use crate::model::wallet::WalletSeedHash;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use crate::ui::identities::transfer_screen::TransferScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use dash_sdk::platform::Identifier;
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Context};
use eframe::emath::Align;
use egui::{Color32, Frame, Margin, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use std::collections::{HashMap, HashSet};
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

    /// Reorders `self.identities` to match the order of the provided list of IDs.
    /// Any IDs not present in the provided list are left in their current position.
    fn reorder_map_to(&self, new_order: Vec<Identifier>) {
        let mut lock = self.identities.lock().unwrap();
        if lock.is_empty() || new_order.is_empty() {
            return;
        }

        // 1) Collect the set of IDs currently in `self.identities`
        let existing_ids: HashSet<Identifier> = lock.keys().cloned().collect();

        // 2) Build a filtered list that only includes IDs which exist in the map
        //    (and also limit the length so we don’t swap out of range)
        let valid_ids: Vec<Identifier> = new_order
            .into_iter()
            .filter(|id| existing_ids.contains(id))
            .take(lock.len()) // never try to reorder more items than we actually have
            .collect();

        // 3) Do the swaps only for items still present in our map,
        //    skipping any that no longer exist or where desired_idx is out of range
        for (desired_idx, id) in valid_ids.into_iter().enumerate() {
            // Double‐check the desired index is in range
            if desired_idx >= lock.len() {
                break;
            }
            if let Some(current_idx) = lock.get_index_of(&id) {
                if current_idx != desired_idx {
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
        let mut lock = self.identities.lock().unwrap();
        *lock = list
            .iter()
            .map(|qi| (qi.identity.id(), qi.clone()))
            .collect();
        drop(lock);
        self.save_current_order();
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

        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let text_edit = egui::TextEdit::singleline(&mut alias)
            .hint_text(placeholder_text)
            .desired_width(100.0)
            .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
            .background_color(crate::ui::theme::DashColors::input_background(dark_mode));

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
            match self.app_context.set_identity_alias(
                &identity_to_update.identity.id(),
                identity_to_update.alias.as_deref(),
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

    fn format_key_name(&self, key: &IdentityPublicKey) -> String {
        let purpose_letter = match key.purpose() {
            Purpose::AUTHENTICATION => "A",
            Purpose::ENCRYPTION => "E",
            Purpose::DECRYPTION => "D",
            Purpose::TRANSFER => "T",
            Purpose::SYSTEM => "S",
            Purpose::VOTING => "V",
            Purpose::OWNER => "O",
        };
        let security_level = match key.security_level() {
            SecurityLevel::MASTER => "Master",
            SecurityLevel::CRITICAL => "Critical",
            SecurityLevel::HIGH => "High",
            SecurityLevel::MEDIUM => "Medium",
        };
        format!("{} - {} - {}", key.id(), purpose_letter, security_level)
    }

    fn render_no_identities_view(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Optionally put everything in a framed "card"-like container
        Frame::group(ui.style())
            .fill(ui.visuals().extreme_bg_color) // background color
            .corner_radius(5.0) // rounded corners
            .outer_margin(Margin::same(20)) // space around the frame
            .shadow(ui.visuals().window_shadow) // drop shadow
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    // Heading
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("No Identities Loaded")
                            .strong()
                            .size(25.0)
                            .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                    );

                    // A separator line for visual clarity
                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Description
                    ui.label(
                        "It looks like you are not tracking any Identities, \
                         Evonodes, or Masternodes yet.",
                    );

                    ui.add_space(10.0);

                    // Subheading or emphasis
                    ui.heading(
                        RichText::new("Here’s what you can do:")
                            .strong()
                            .size(18.0)
                            .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(5.0);

                    // Bullet points
                    ui.label(
                        "• LOAD an Evonode/Masternode/Identity by clicking \
                         on \"Load Identity\" at the top right, or",
                    );
                    ui.add_space(1.0);
                    ui.label("• REGISTER an Identity after creating or importing a wallet.");

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Footnote or extra info
                    ui.label(
                        "(Make sure Dash Core is running. You can check in the \
                         network tab on the left.)",
                    );

                    ui.add_space(5.0);
                });
            });
    }

    /// Ensures that all identities have their status known (not `Unknown`).
    ///
    /// Returns Some `AppAction` if any identity needs to be refreshed,
    /// otherwise returns None.
    pub fn ensure_identities_status(&self, identities: &[QualifiedIdentity]) -> Option<AppAction> {
        if self.refreshing_status != IdentitiesRefreshingStatus::NotRefreshing {
            // avoid refresh loop
            return None;
        }

        let backend_tasks: Vec<BackendTask> = identities
            .iter()
            .filter_map(|qi| {
                if qi.status == IdentityStatus::Unknown {
                    Some(BackendTask::IdentityTask(IdentityTask::RefreshIdentity(
                        qi.clone(),
                    )))
                } else {
                    None
                }
            })
            .collect();

        if !backend_tasks.is_empty() {
            Some(AppAction::BackendTasks(
                backend_tasks,
                BackendTasksExecutionMode::Concurrent,
            ))
        } else {
            None
        }
    }

    fn render_identities_view(
        &mut self,
        ui: &mut Ui,
        identities: &[QualifiedIdentity],
    ) -> AppAction {
        let mut action = AppAction::None;

        // Refresh identities if needed
        if let Some(action) = self.ensure_identities_status(identities) {
            return action;
        }

        let mut local_identities = identities.to_vec();
        if !self.use_custom_order {
            self.sort_vec(&mut local_identities);
        }

        // Space allocation for UI elements is handled by the layout system

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                        .striped(false)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(Align::Center))
                        .column(Column::initial(80.0).resizable(true))   // Name
                        .column(Column::initial(330.0).resizable(true))  // Identity ID
                        .column(Column::initial(60.0).resizable(true))   // In Wallet
                        .column(Column::initial(80.0).resizable(true))   // Type
                        .column(Column::initial(140.0).resizable(true))  // Balance
                        .column(Column::initial(160.0).resizable(true))  // Actions (wider for more buttons)
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
                                if ui.button("Balance").clicked() {
                                    self.toggle_sort(IdentitiesSortColumn::Balance);
                                }
                            });
                            header.col(|ui| {
                                ui.heading("");
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

                                // Check if identity is active
                                let is_active = qualified_identity.status == IdentityStatus::Active;

                                body.row(30.0, |mut row| {
                                    row.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal_centered(|ui| {
                                                // Disable UI elements if identity is not active
                                                ui.add_enabled_ui(is_active, |ui| {
                                                    self.show_alias(ui, qualified_identity);
                                                });
                                            });
                                        });
                                    });
                                    row.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal_centered(|ui| {
                                                ui.add_enabled_ui(is_active, |ui| {
                                                    Self::show_identity_id(ui, qualified_identity);
                                                });
                                            });
                                        });
                                    });
                                    row.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal_centered(|ui| {
                                                ui.add_enabled_ui(is_active, |ui| {
                                                    self.show_in_wallet(ui, qualified_identity);
                                                });
                                            });
                                        });
                                    });
                                    row.col(|ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.horizontal_centered(|ui| {
                                                // Show identity type and status
                                                let type_text = format!("{}", qualified_identity.identity_type);
                                                let status = qualified_identity.status;
                                                // Always show status information (don't disable this column)
                                                ui.add_enabled_ui(true, |ui|{
                                                    if is_active {
                                                        ui.label(type_text);
                                                    } else{
                                                        ui.label(RichText::new(status.to_string()).color(status));
                                                    };
                                                });
                                            });
                                        });
                                    });

                                    row.col(|ui| {
                                        ui.add_enabled_ui(is_active, |ui| {
                                            Self::show_balance(ui, qualified_identity);

                                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                                ui.add_space(-1.0);
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 3.0;

                                                    // Actions dropdown button
                                                    let actions_button = egui::Button::new("Actions")
                                                        .fill(ui.visuals().widgets.inactive.bg_fill)
                                                        .frame(true)
                                                        .corner_radius(3.0)
                                                        .min_size(egui::vec2(60.0, 20.0));

                                                    let actions_response = ui.add(actions_button).on_hover_text("Manage identity credits");

                                                    let actions_popup_id = ui.make_persistent_id(format!("actions_popup_{}", qualified_identity.identity.id().to_string(Encoding::Base58)));

                                                    if actions_response.clicked() {
                                                        ui.memory_mut(|mem| mem.toggle_popup(actions_popup_id));
                                                    }

                                                    egui::popup::popup_below_widget(
                                                        ui,
                                                        actions_popup_id,
                                                        &actions_response,
                                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                                        |ui| {
                                                            ui.set_min_width(150.0);

                                                            if ui.button("💸 Withdraw").on_hover_text("Withdraw credits from this identity to a Dash Core address").clicked() {
                                                                action = AppAction::AddScreen(
                                                                    Screen::WithdrawalScreen(WithdrawalScreen::new(
                                                                        qualified_identity.clone(),
                                                                        &self.app_context,
                                                                    )),
                                                                );
                                                                ui.close_menu();
                                                            }

                                                            if ui.button("💰 Top up").on_hover_text("Increase this identity's balance by sending it Dash from the Core chain").clicked() {
                                                                action = AppAction::AddScreen(
                                                                    Screen::TopUpIdentityScreen(TopUpIdentityScreen::new(
                                                                        qualified_identity.clone(),
                                                                        &self.app_context,
                                                                    )),
                                                                );
                                                                ui.close_menu();
                                                            }

                                                            if ui.button("📤 Transfer").on_hover_text("Transfer credits from this identity to another identity").clicked() {
                                                                action = AppAction::AddScreen(
                                                                    Screen::TransferScreen(TransferScreen::new(
                                                                        qualified_identity.clone(),
                                                                        &self.app_context,
                                                                    )),
                                                                );
                                                                ui.close_menu();
                                                            }
                                                        },
                                                    );
                                            });
                                            });
                                        });
                                    });
                                    row.col(|ui| {
                                        // we always enable the actions column to be able to delete/reorder invalid identities
                                        ui.add_enabled_ui(true, |ui| {
                                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                            ui.add_space(-1.0);

                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 3.0;

                                                // Keys dropdown button
                                                let has_keys = !public_keys.is_empty() || voter_identity_public_keys.is_some();
                                                if has_keys {
                                                    let button = egui::Button::new("Keys")
                                                        .fill(ui.visuals().widgets.inactive.bg_fill)
                                                        .frame(true)
                                                        .corner_radius(3.0)
                                                        .min_size(egui::vec2(50.0, 20.0));

                                                    let response = ui.add(button).on_hover_text("View and manage keys for this identity");

                                                    let popup_id = ui.make_persistent_id(format!("keys_popup_{}", qualified_identity.identity.id().to_string(Encoding::Base58)));

                                                    if response.clicked() {
                                                        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                                                    }

                                                    egui::popup::popup_below_widget(
                                                        ui,
                                                        popup_id,
                                                        &response,
                                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                                        |ui| {
                                                            ui.set_min_width(200.0);

                                                            // Main Identity Keys
                                                            if !public_keys.is_empty() {
                                                                let dark_mode = ui.ctx().style().visuals.dark_mode;
                                                                ui.label(RichText::new("Main Identity Keys:").strong().color(crate::ui::theme::DashColors::text_primary(dark_mode)));
                                                                ui.separator();

                                                                for (key_id, key) in public_keys.iter() {
                                                                    let holding_private_key = qualified_identity.private_keys
                                                                        .get_cloned_private_key_data_and_wallet_info(&(PrivateKeyOnMainIdentity, *key_id));

                                                                    let button_color = if holding_private_key.is_some() {
                                                                        if dark_mode {
                                                                            Color32::from_rgb(100, 180, 180) // Darker blue for dark mode
                                                                        } else {
                                                                            Color32::from_rgb(167, 232, 232) // Light blue for light mode
                                                                        }
                                                                    } else {
                                                                        crate::ui::theme::DashColors::glass_white(dark_mode) // Theme-aware for unloaded keys
                                                                    };

                                                                    let text_color = if holding_private_key.is_some() {
                                                                        Color32::BLACK // Black text on light blue background
                                                                    } else {
                                                                        crate::ui::theme::DashColors::text_primary(dark_mode) // Theme-aware text
                                                                    };

                                                                    let button = egui::Button::new(RichText::new(self.format_key_name(key)).color(text_color))
                                                                        .fill(button_color)
                                                                        .frame(true);

                                                                    if ui.add(button).clicked() {
                                                                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                                                                            qualified_identity.clone(),
                                                                            key.clone(),
                                                                            holding_private_key,
                                                                            &self.app_context,
                                                                        )));
                                                                        ui.close_menu();
                                                                    }
                                                                }
                                                            }

                                                            // Voter Identity Keys
                                                            if let Some((voter_identity, _)) = qualified_identity.associated_voter_identity.as_ref() {
                                                                let voter_public_keys = voter_identity.public_keys();
                                                                if !voter_public_keys.is_empty() {
                                                                    if !public_keys.is_empty() {
                                                                        ui.add_space(5.0);
                                                                    }
                                                                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                                                                    ui.label(RichText::new("Voter Identity Keys:").strong().color(crate::ui::theme::DashColors::text_primary(dark_mode)));
                                                                    ui.separator();

                                                                    for (key_id, key) in voter_public_keys.iter() {
                                                                        let holding_private_key = qualified_identity.private_keys
                                                                            .get_cloned_private_key_data_and_wallet_info(&(PrivateKeyOnVoterIdentity, *key_id));

                                                                        let button_color = if holding_private_key.is_some() {
                                                                            if dark_mode {
                                                                                Color32::from_rgb(100, 180, 180) // Darker blue for dark mode
                                                                            } else {
                                                                                Color32::from_rgb(167, 232, 232) // Light blue for light mode
                                                                            }
                                                                        } else {
                                                                            crate::ui::theme::DashColors::glass_white(dark_mode) // Theme-aware for unloaded keys
                                                                        };

                                                                        let text_color = if holding_private_key.is_some() {
                                                                            Color32::BLACK // Black text on light blue background
                                                                        } else {
                                                                            crate::ui::theme::DashColors::text_primary(dark_mode) // Theme-aware text
                                                                        };

                                                                        let button = egui::Button::new(RichText::new(self.format_key_name(key)).color(text_color))
                                                                            .fill(button_color)
                                                                            .frame(true);

                                                                        if ui.add(button).clicked() {
                                                                            action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                                                                                qualified_identity.clone(),
                                                                                key.clone(),
                                                                                holding_private_key,
                                                                                &self.app_context,
                                                                            )));
                                                                            ui.close_menu();
                                                                        }
                                                                    }
                                                                }
                                                            }

                                                            // Add Key button
                                                            if qualified_identity.can_sign_with_master_key().is_some() {
                                                                ui.separator();
                                                                let dark_mode = ui.ctx().style().visuals.dark_mode;
                                                                let add_button = egui::Button::new("➕ Add Key")
                                                                    .fill(crate::ui::theme::DashColors::glass_white(dark_mode))
                                                                    .frame(true);

                                                                    if ui.add(add_button).on_hover_text("Add a new key to this identity").clicked() {
                                                                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                                                                        qualified_identity.clone(),
                                                                        &self.app_context,
                                                                    )));
                                                                    ui.close_menu();
                                                                }
                                                            }
                                                        },
                                                    );
                                                }

                                                // Remove
                                                if ui.button("Remove").on_hover_text("Remove this identity from Dash Evo Tool (it'll still exist on Dash Platform)").clicked() {
                                                    self.identity_to_remove =
                                                        Some(qualified_identity.clone());
                                                }

                                                // Up arrow
                                                let up_btn = ui.button("⬆").on_hover_text("Move this identity up in the list");
                                                // Down arrow
                                                let down_btn = ui.button("⬇").on_hover_text("Move this identity down in the list");

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
                                    });
                                });
                            }
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
        drop(identities);

        // Keep order after refreshing
        if let Ok(saved_ids) = self.app_context.db.load_identity_order() {
            self.reorder_map_to(saved_ids);
            self.use_custom_order = true;
        }
    }

    fn display_message(&mut self, message: &str, message_type: crate::ui::MessageType) {
        if message.contains("Error refreshing identity")
            || message.contains("Successfully refreshed identity")
        {
            self.refreshing_status = IdentitiesRefreshingStatus::NotRefreshing;
        }
        self.backend_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn display_task_result(
        &mut self,
        _backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        // Nothing
        // If we don't include this, success messages from ZMQ listener will keep popping up
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();

        let mut right_buttons = if !self.app_context.has_wallet.load(Ordering::Relaxed) {
            vec![
                (
                    "Import Wallet",
                    DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportWallet)),
                ),
                (
                    "Create Wallet",
                    DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
                ),
            ]
        } else {
            vec![(
                "Create Identity",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewIdentity)),
            )]
        };
        right_buttons.push((
            "Load Identity",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::AddExistingIdentity)),
        ));
        if !self.identities.lock().unwrap().is_empty() {
            // Create a vec of RefreshIdentity(identity) DesiredAppAction for each identity
            let backend_tasks: Vec<BackendTask> = self
                .identities
                .lock()
                .unwrap()
                .values()
                .map(|qi| BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi.clone())))
                .collect();
            right_buttons.push((
                "Refresh",
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

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;
            if identities_vec.is_empty() {
                self.render_no_identities_view(ui);
            } else {
                inner_action |= self.render_identities_view(ui, &identities_vec);
            }

            // Show either refreshing indicator or message, but not both
            if let IdentitiesRefreshingStatus::Refreshing(start_time) = self.refreshing_status {
                ui.add_space(25.0); // Space above
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(format!("Refreshing... Time taken so far: {}", elapsed));
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
                ui.add_space(2.0); // Space below
            } else if let Some((message, message_type, timestamp)) = self.backend_message.clone() {
                let message_color = match message_type {
                    MessageType::Error => egui::Color32::DARK_RED,
                    MessageType::Info => egui::Color32::BLACK,
                    MessageType::Success => egui::Color32::DARK_GREEN,
                };

                ui.add_space(25.0); // Same space as refreshing indicator
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    // Calculate remaining seconds
                    let now = Utc::now();
                    let elapsed = now.signed_duration_since(timestamp);
                    let remaining = (10 - elapsed.num_seconds()).max(0);

                    // Add the message with auto-dismiss countdown
                    let full_msg = format!("{} ({}s)", message, remaining);
                    ui.label(egui::RichText::new(full_msg).color(message_color));
                });
                ui.add_space(2.0); // Same space below as refreshing indicator
            }
            inner_action
        });

        if self.identity_to_remove.is_some() {
            self.show_identity_to_remove(ctx);
        }

        match action {
            AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RefreshIdentity(_))) => {
                self.refreshing_status =
                    IdentitiesRefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                self.backend_message = None // Clear any existing message
            }
            AppAction::BackendTasks(_, _) => {
                // Going to assume this is only going to be Refresh All
                self.refreshing_status =
                    IdentitiesRefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                self.backend_message = None // Clear any existing message
            }
            _ => {}
        }

        action
    }
}
