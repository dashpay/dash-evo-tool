use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::tokens::burn_tokens_screen::BurnTokensScreen;
use crate::ui::tokens::claim_tokens_screen::ClaimTokensScreen;
use crate::ui::tokens::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use crate::ui::tokens::direct_token_purchase_screen::PurchaseTokenScreen;
use crate::ui::tokens::freeze_tokens_screen::FreezeTokensScreen;
use crate::ui::tokens::mint_tokens_screen::MintTokensScreen;
use crate::ui::tokens::pause_tokens_screen::PauseTokensScreen;
use crate::ui::tokens::resume_tokens_screen::ResumeTokensScreen;
use crate::ui::tokens::set_token_price_screen::SetTokenPriceScreen;
use crate::ui::tokens::tokens_screen::{
    validate_perpetual_distribution_recipient, IdentityTokenIdentifier, IdentityTokenInfo,
    IdentityTokenMaybeBalance, RefreshingStatus, SortColumn, TokensScreen,
};
use crate::ui::tokens::transfer_tokens_screen::TransferTokensScreen;
use crate::ui::tokens::unfreeze_tokens_screen::UnfreezeTokensScreen;
use crate::ui::tokens::update_token_config::UpdateTokenConfigScreen;
use crate::ui::tokens::view_token_claims_screen::ViewTokenClaimsScreen;
use crate::ui::Screen;
use chrono::Utc;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Accessors;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::group::action_taker::{ActionGoal, ActionTaker};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::emath::Align;
use eframe::epaint::Margin;
use egui::{Frame, Ui};
use egui_extras::{Column, TableBuilder};
use std::sync::atomic::Ordering;

impl TokensScreen {
    /// Renders details for the selected token_id: a row per identity that holds that token.
    pub(super) fn render_token_details(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let Some((token_id, contract, token_configuration)) = self.selected_token.as_ref() else {
            return action;
        };

        //todo remove clones
        let contract = contract.clone();
        let token_configuration = token_configuration.clone();

        let Some(token_info) = self.all_known_tokens.get(token_id) else {
            return action;
        };

        let identities = &self.identities;

        let mut detail_list: Vec<(IdentityTokenMaybeBalance, QualifiedIdentity)> = vec![];

        for (identity_id, identity) in identities {
            let record = if let Some(known_token_balance) =
                self.my_tokens.get(&IdentityTokenIdentifier {
                    identity_id: *identity_id,
                    token_id: *token_id,
                }) {
                (
                    IdentityTokenMaybeBalance {
                        token_id: *token_id,
                        token_name: token_info.token_name.clone(),
                        identity_id: *identity_id,
                        identity_alias: identity.alias.clone(),
                        balance: Some(known_token_balance.clone()),
                    },
                    identity.clone(),
                )
            } else {
                (
                    IdentityTokenMaybeBalance {
                        token_id: *token_id,
                        token_name: token_info.token_name.clone(),
                        identity_id: *identity_id,
                        identity_alias: identity.alias.clone(),
                        balance: None,
                    },
                    identity.clone(),
                )
            };
            detail_list.push(record);
        }

        // if !self.use_custom_order {
        //     self.sort_vec(&mut detail_list);
        // }

        // Allocate space for refreshing indicator
        let refreshing_height = 33.0;
        let mut max_scroll_height = if let RefreshingStatus::Refreshing(_) = self.refreshing_status
        {
            ui.available_height() - refreshing_height
        } else {
            ui.available_height()
        };

        // Allocate space for backend message
        let backend_message_height = 40.0;
        if let Some((_, _, _)) = self.backend_message.clone() {
            max_scroll_height -= backend_message_height;
        }

        let in_dev_mode = self.app_context.developer_mode.load(Ordering::Relaxed);

        let main_group = token_configuration.main_control_group();
        let groups = contract.groups();
        let contract_owner_id = contract.owner_id();

        let shows_estimation_column = in_dev_mode
            || token_configuration
                .distribution_rules()
                .perpetual_distribution()
                .is_some();
        let can_claim = |identity: &QualifiedIdentity| {
            if let Some(distribution) = token_configuration
                .distribution_rules()
                .perpetual_distribution()
            {
                in_dev_mode
                    || validate_perpetual_distribution_recipient(
                        contract_owner_id,
                        distribution.distribution_recipient(),
                        identity,
                    )
                    .is_ok()
                    || token_configuration
                        .distribution_rules()
                        .pre_programmed_distribution()
                        .is_some()
            } else {
                in_dev_mode
                    || token_configuration
                        .distribution_rules()
                        .pre_programmed_distribution()
                        .is_some()
            }
        };
        let can_estimate = |identity: &QualifiedIdentity| {
            if let Some(distribution) = token_configuration
                .distribution_rules()
                .perpetual_distribution()
            {
                in_dev_mode
                    || validate_perpetual_distribution_recipient(
                        contract_owner_id,
                        distribution.distribution_recipient(),
                        identity,
                    )
                    .is_ok()
            } else {
                in_dev_mode
            }
        };
        let can_mint = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .manual_minting_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .manual_minting_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        let can_burn = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .manual_burning_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .manual_burning_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        let can_freeze = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .freeze_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .freeze_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        let can_unfreeze = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .unfreeze_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .unfreeze_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        let can_destroy = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .destroy_frozen_funds_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .destroy_frozen_funds_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        let can_do_emergency_action = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .emergency_action_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .emergency_action_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        let can_maybe_purchase = in_dev_mode
            || token_configuration
                .distribution_rules()
                .change_direct_purchase_pricing_rules()
                .authorized_to_make_change_action_takers()
                != &AuthorizedActionTakers::NoOne;

        let can_set_price = |identity_id: Identifier| {
            let solo_action_taker = ActionTaker::SingleIdentity(identity_id);
            let authorized = token_configuration
                .distribution_rules()
                .change_direct_purchase_pricing_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionCompletion,
                );
            let authorized_in_group = token_configuration
                .distribution_rules()
                .change_direct_purchase_pricing_rules()
                .authorized_to_make_change_action_takers()
                .allowed_for_action_taker(
                    &contract_owner_id,
                    main_group,
                    groups,
                    &solo_action_taker,
                    ActionGoal::ActionParticipation,
                );
            in_dev_mode || authorized || authorized_in_group
        };

        // A simple table with columns: [Token Name | Token ID | Total Balance]
        egui::ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ui.visuals().widgets.inactive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        let mut table = TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(60.0).resizable(true)) // Identity Alias
                            .column(Column::initial(200.0).resizable(true)) // Identity ID
                            .column(Column::initial(60.0).resizable(true)); // Balance


                        if shows_estimation_column {
                            table = table.column(Column::initial(60.0).resizable(true)); // Estimated Rewards
                        }

                        table = table.column(Column::initial(200.0).resizable(true));// Actions
                        table.header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Identity Alias").clicked() {
                                    self.toggle_sort(SortColumn::OwnerIdentityAlias);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Identity ID").clicked() {
                                    self.toggle_sort(SortColumn::OwnerIdentity);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Balance").clicked() {
                                    self.toggle_sort(SortColumn::Balance);
                                }
                            });
                            if shows_estimation_column {
                                header.col(|ui| {
                                    ui.label("Estimated Unclaimed Rewards");
                                });
                            }
                            header.col(|ui| {
                                ui.label("Actions");
                            });
                        })
                            .body(|mut body| {
                                for (itb, qi) in &detail_list {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            // Show identity alias or ID
                                            if let Some(alias) = self
                                                .app_context
                                                .get_identity_alias(&itb.identity_id)
                                                .expect("Expected to get alias")
                                            {
                                                ui.label(alias);
                                            } else {
                                                ui.label("-");
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.label(itb.identity_id.to_string(Encoding::Base58));
                                        });
                                        row.col(|ui| {
                                            if let Some(balance) = itb.balance.as_ref().map(|balance| balance.balance.to_string()) {
                                                ui.label(balance);
                                            } else {
                                                if ui.button("Check").clicked() {
                                                    action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryIdentityTokenBalance(itb.clone().into())));
                                                }
                                            }
                                        });
                                        if shows_estimation_column {
                                            row.col(|ui| {
                                                if can_estimate(qi) {
                                                    if let Some(known_rewards) = itb.balance.as_ref().map(|itb| itb.estimated_unclaimed_rewards) {
                                                        if let Some(known_rewards) = known_rewards {
                                                            ui.horizontal(|ui| {
                                                                ui.label(known_rewards.to_string());
                                                                if ui.button("Estimate").clicked() {
                                                                    action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::EstimatePerpetualTokenRewards {
                                                                        identity_id: itb.identity_id,
                                                                        token_id: itb.token_id,
                                                                    }));
                                                                    self.refreshing_status = RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                                                                }
                                                            });
                                                        } else {
                                                            if ui.button("Estimate").clicked() {
                                                                action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::EstimatePerpetualTokenRewards {
                                                                    identity_id: itb.identity_id,
                                                                    token_id: itb.token_id,
                                                                }));
                                                                self.refreshing_status = RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                        row.col(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 3.0;

                                                if let Some(itb) = itb.balance.as_ref() {
                                                    // Transfer
                                                    if ui.button("Transfer").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::TransferTokensScreen(
                                                                TransferTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                    }

                                                    // Claim
                                                    if can_claim(qi) {
                                                        if ui.button("Claim").clicked() {
                                                            let token_contract = match self.app_context.get_contract_by_token_id(&itb.token_id) {
                                                                Ok(Some(contract)) => contract,
                                                                Ok(None) => {
                                                                    self.set_error_message(Some("Token contract not found".to_string()));
                                                                    return;
                                                                }
                                                                Err(e) => {
                                                                    self.set_error_message(Some(format!("Error fetching token contract: {e}")));
                                                                    return;
                                                                }
                                                            };

                                                            action = AppAction::AddScreen(
                                                                Screen::ClaimTokensScreen(
                                                                    ClaimTokensScreen::new(
                                                                        itb.clone(),
                                                                        token_contract,
                                                                        token_configuration.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                    }

                                                    // Expandable advanced actions menu
                                                    ui.menu_button("...", |ui| {
                                                        if can_mint(itb.identity_id) {
                                                            if ui.button("Mint").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::MintTokensScreen(
                                                                                MintTokensScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };

                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_burn(itb.identity_id) {
                                                            if ui.button("Burn").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::BurnTokensScreen(
                                                                                BurnTokensScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };
                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_freeze(itb.identity_id) {
                                                            if ui.button("Freeze").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::FreezeTokensScreen(
                                                                                FreezeTokensScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };
                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_destroy(itb.identity_id) {
                                                            if ui.button("Destroy").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::DestroyFrozenFundsScreen(
                                                                                DestroyFrozenFundsScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };
                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_unfreeze(itb.identity_id) {
                                                            if ui.button("Unfreeze").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::UnfreezeTokensScreen(
                                                                                UnfreezeTokensScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };
                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_do_emergency_action(itb.identity_id) {
                                                            if ui.button("Pause").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::PauseTokensScreen(
                                                                                PauseTokensScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };
                                                                ui.close_menu();
                                                            }

                                                            if ui.button("Resume").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::ResumeTokensScreen(
                                                                                ResumeTokensScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };
                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_claim(qi) {
                                                            if ui.button("View Claims").clicked() {
                                                                action = AppAction::AddScreen(
                                                                    Screen::ViewTokenClaimsScreen(
                                                                        ViewTokenClaimsScreen::new(
                                                                            itb.clone(),
                                                                            &self.app_context,
                                                                        ),
                                                                    ),
                                                                );
                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if ui.button("Update Config").clicked() {
                                                            match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                Ok(info) => {
                                                                    action = AppAction::AddScreen(
                                                                        Screen::UpdateTokenConfigScreen(
                                                                            UpdateTokenConfigScreen::new(
                                                                                info,
                                                                                &self.app_context,
                                                                            ),
                                                                        ),
                                                                    );
                                                                }
                                                                Err(e) => {
                                                                    self.set_error_message(Some(e));
                                                                }
                                                            };
                                                            ui.close_menu();
                                                        }
                                                        if can_maybe_purchase {
                                                            // Purchase
                                                            if ui.button("Purchase").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::PurchaseTokenScreen(
                                                                                PurchaseTokenScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };

                                                                ui.close_menu();
                                                            }
                                                        }
                                                        if can_set_price(itb.identity_id) {
                                                            // Set Price
                                                            if ui.button("Set Price").clicked() {
                                                                match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(itb, &self.app_context) {
                                                                    Ok(info) => {
                                                                        action = AppAction::AddScreen(
                                                                            Screen::SetTokenPriceScreen(
                                                                                SetTokenPriceScreen::new(
                                                                                    info,
                                                                                    &self.app_context,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        self.set_error_message(Some(e));
                                                                    }
                                                                };

                                                                ui.close_menu();
                                                            }
                                                        }
                                                    });

                                                    // Remove
                                                    if ui
                                                        .button("X")
                                                        .on_hover_text(
                                                            "Remove identity token balance from DET",
                                                        )
                                                        .clicked()
                                                    {
                                                        self.confirm_remove_identity_token_balance_popup = true;
                                                        self.identity_token_balance_to_remove = Some(itb.clone());
                                                    }
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
}
