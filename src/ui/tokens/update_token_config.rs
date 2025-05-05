use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui::Color32;
use egui::Context;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::tokens_screen::IdentityTokenBalance;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateTokenConfigStatus {
    NotUpdating,
    Updating(DateTime<Utc>),
}

pub struct UpdateTokenConfigScreen {
    pub identity_token_balance: IdentityTokenBalance,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    update_status: UpdateTokenConfigStatus,
    pub app_context: Arc<AppContext>,
    change_items: Vec<TokenConfigurationChangeItem>,
    signing_key: IdentityPublicKey,
    public_note: Option<String>,
}

impl UpdateTokenConfigScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let identity = app_context
            .get_identity_by_id(&identity_token_balance.identity_id)
            .expect("Error getting identity by ID")
            .expect("Identity not found");
        let possible_key = identity
            .available_authentication_keys_non_master()
            .first()
            .expect("No authentication keys found")
            .identity_public_key
            .clone();

        Self {
            identity_token_balance: identity_token_balance.clone(),
            message: None,
            update_status: UpdateTokenConfigStatus::NotUpdating,
            app_context: app_context.clone(),
            change_items: vec![],
            signing_key: possible_key,
            public_note: None,
        }
    }

    fn render_token_config_updater(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("Select the token configuration items to update");
        ui.add_space(8.0);

        /* ---------- per‑row UI ---------- */
        let mut to_remove: Vec<usize> = Vec::new();

        for (idx, item) in self.change_items.iter_mut().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                let label = match item {
                    TokenConfigurationChangeItem::TokenConfigurationNoChange => "No Change",
                    TokenConfigurationChangeItem::Conventions(_) => "Conventions",
                    TokenConfigurationChangeItem::ConventionsControlGroup(_) => "Conventions Control Group",
                    TokenConfigurationChangeItem::ConventionsAdminGroup(_) => "Conventions Admin Group",
                    TokenConfigurationChangeItem::MaxSupply(_) => "Max Supply",
                    TokenConfigurationChangeItem::MaxSupplyControlGroup(_) => "Max Supply Control Group",
                    TokenConfigurationChangeItem::MaxSupplyAdminGroup(_) => "Max Supply Admin Group",
                    TokenConfigurationChangeItem::PerpetualDistribution(_) => "Perpetual Distribution",
                    TokenConfigurationChangeItem::PerpetualDistributionControlGroup(_) => "Perpetual Distribution Control Group",
                    TokenConfigurationChangeItem::PerpetualDistributionAdminGroup(_) => "Perpetual Distribution Admin Group",
                    TokenConfigurationChangeItem::NewTokensDestinationIdentity(_) => "New‑Tokens Destination",
                    TokenConfigurationChangeItem::NewTokensDestinationIdentityControlGroup(_) => "New‑Tokens Destination Control Group",
                    TokenConfigurationChangeItem::NewTokensDestinationIdentityAdminGroup(_) => "New‑Tokens Destination Admin Group",
                    TokenConfigurationChangeItem::MintingAllowChoosingDestination(_) => "Minting Allow Choosing Destination",
                    TokenConfigurationChangeItem::MintingAllowChoosingDestinationControlGroup(_) => "Minting Allow Choosing Destination Control Group",
                    TokenConfigurationChangeItem::MintingAllowChoosingDestinationAdminGroup(_) => "Minting Allow Choosing Destination Admin Group",
                    TokenConfigurationChangeItem::ManualMinting(_) => "Manual Minting",
                    TokenConfigurationChangeItem::ManualMintingAdminGroup(_) => "Manual Minting Admin Group",
                    TokenConfigurationChangeItem::ManualBurning(_) => "Manual Burning",
                    TokenConfigurationChangeItem::ManualBurningAdminGroup(_) => "Manual Burning Admin Group",
                    TokenConfigurationChangeItem::Freeze(_) => "Freeze",
                    TokenConfigurationChangeItem::FreezeAdminGroup(_) => "Freeze Admin Group",
                    TokenConfigurationChangeItem::Unfreeze(_) => "Unfreeze",
                    TokenConfigurationChangeItem::UnfreezeAdminGroup(_) => "Unfreeze Admin Group",
                    TokenConfigurationChangeItem::DestroyFrozenFunds(_) => "Destroy Frozen Funds",
                    TokenConfigurationChangeItem::DestroyFrozenFundsAdminGroup(_) => "Destroy Frozen Funds Admin Group",
                    TokenConfigurationChangeItem::EmergencyAction(_) => "Emergency Action",
                    TokenConfigurationChangeItem::EmergencyActionAdminGroup(_) => "Emergency Action Admin Group",
                    TokenConfigurationChangeItem::MainControlGroup(_) => "Main Control Group",
                };

                egui::ComboBox::from_id_salt(format!("cfg_item_type_{idx}"))
                    .selected_text(label)
                    .width(270.0)
                    .show_ui(ui, |ui| {
                        /* ───────── “No change” ───────── */
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::TokenConfigurationNoChange,
                            "No Change",
                        );

                        ui.separator();

                        /* ───────── Conventions + groups ───────── */
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::Conventions(
                                TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
                                    localizations: BTreeMap::new(),
                                    decimals: 0,
                                }),
                            ),
                            "Conventions",
                        );
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::ConventionsControlGroup(
                                AuthorizedActionTakers::ContractOwner,
                            ),
                            "Conventions Control Group",
                        );
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::ConventionsAdminGroup(
                                AuthorizedActionTakers::ContractOwner,
                            ),
                            "Conventions Admin Group",
                        );

                        ui.separator();

                        /* ───────── Max‑supply + groups ───────── */
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::MaxSupply(Some(TokenAmount::from(0u64))),
                            "Max Supply",
                        );
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::MaxSupplyControlGroup(
                                AuthorizedActionTakers::ContractOwner,
                            ),
                            "Max Supply Control Group",
                        );
                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::MaxSupplyAdminGroup(
                                AuthorizedActionTakers::ContractOwner,
                            ),
                            "Max Supply Admin Group",
                        );

                        ui.separator();

                        /* ───────── Perpetual‑dist + groups ───────── */
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::PerpetualDistribution(None),
                        "Perpetual Distribution",
                        );
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::PerpetualDistributionControlGroup(
                        AuthorizedActionTakers::ContractOwner,
                        ),
                        "Perpetual Distribution Control Group",
                        );
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::PerpetualDistributionAdminGroup(
                        AuthorizedActionTakers::ContractOwner,
                        ),
                        "Perpetual Distribution Admin Group",
                        );

                        ui.separator();

                        /* ───────── New‑tokens destination + groups ───────── */
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::NewTokensDestinationIdentity(Some(Identifier::default())),
                        "New‑Tokens Destination",
                        );
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::NewTokensDestinationIdentityControlGroup(
                        AuthorizedActionTakers::ContractOwner,
                        ),
                        "New‑Tokens Destination Control Group",
                        );
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::NewTokensDestinationIdentityAdminGroup(
                        AuthorizedActionTakers::ContractOwner,
                        ),
                        "New‑Tokens Destination Admin Group",
                        );

                        ui.separator();

                        /* ───────── Mint‑dest‑choice + groups ───────── */
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::MintingAllowChoosingDestination(false),
                        "Minting Allow Choosing Destination",
                        );
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::MintingAllowChoosingDestinationControlGroup(
                        AuthorizedActionTakers::ContractOwner,
                        ),
                        "Minting Allow Choosing Destination Control Group",
                        );
                        ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::MintingAllowChoosingDestinationAdminGroup(
                        AuthorizedActionTakers::ContractOwner,
                        ),
                        "Minting Allow Choosing Destination Admin Group",
                        );

                        ui.separator();

                        /* ───────── Remaining AuthorizedActionTakers variants ───────── */
                        macro_rules! aat_item {
                            ($variant:ident, $label:expr) => {
                                ui.selectable_value(
                                    item,
                                    TokenConfigurationChangeItem::$variant(AuthorizedActionTakers::ContractOwner),
                                    $label,
                                );
                            };
                        }

                        aat_item!(ManualMinting,                "Manual Minting");
                        aat_item!(ManualMintingAdminGroup,      "Manual Minting Admin Group");
                        aat_item!(ManualBurning,                "Manual Burning");
                        aat_item!(ManualBurningAdminGroup,      "Manual Burning Admin Group");
                        aat_item!(Freeze,                       "Freeze");
                        aat_item!(FreezeAdminGroup,             "Freeze Admin Group");
                        aat_item!(Unfreeze,                     "Unfreeze");
                        aat_item!(UnfreezeAdminGroup,           "Unfreeze Admin Group");
                        aat_item!(DestroyFrozenFunds,           "Destroy Frozen Funds");
                        aat_item!(DestroyFrozenFundsAdminGroup, "Destroy Frozen Funds Admin Group");
                        aat_item!(EmergencyAction,              "Emergency Action");
                        aat_item!(EmergencyActionAdminGroup,    "Emergency Action Admin Group");

                        ui.separator();

                        ui.selectable_value(
                            item,
                            TokenConfigurationChangeItem::MainControlGroup(Some(0)),
                                "Main Control Group",
                            );
                        });

                        /* “Remove” button */
                        if ui.button("Remove").clicked() {
                            to_remove.push(idx);
                        }
                    });

                ui.add_space(4.0);

                /* ========== PER‑VARIANT EDITING ========== */
                match item {
                    /* -------- simple value items -------- */
                    TokenConfigurationChangeItem::Conventions(conv) => {
                        let mut txt = conv.to_string();
                        if ui.text_edit_singleline(&mut txt).changed() {
                            *conv = TokenConfigurationConvention::V0(
                                serde_json::from_str(&txt).unwrap_or_default(),
                            );
                        }
                    }

                    TokenConfigurationChangeItem::MaxSupply(opt_amt) => {
                        let mut txt = opt_amt.map(|a| a.to_string()).unwrap_or_default();
                        if ui.text_edit_singleline(&mut txt).changed() {
                            *opt_amt = txt.parse::<u64>().ok().map(TokenAmount::from);
                        }
                    }

                    TokenConfigurationChangeItem::MintingAllowChoosingDestination(b) => {
                        ui.checkbox(b, "Allow user to choose destination when minting");
                    }

                    TokenConfigurationChangeItem::NewTokensDestinationIdentity(opt_id) => {
                        let mut txt = opt_id.map(|id| id.to_string(Encoding::Base58)).unwrap_or_default();
                        if ui.text_edit_singleline(&mut txt).changed() {
                            *opt_id = Identifier::from_string(&txt, Encoding::Base58).ok();
                        }
                    }

                    TokenConfigurationChangeItem::PerpetualDistribution(opt_json) => {
                        let mut raw = opt_json
                            .as_ref()
                            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                            .unwrap_or_default();
                        if ui.text_edit_multiline(&mut raw).changed() {
                            *opt_json = serde_json::from_str(&raw).ok();
                        }
                    }

                    TokenConfigurationChangeItem::MainControlGroup(opt_grp) => {
                        let mut grp_txt = opt_grp
                            .map(|g| g)
                            .unwrap_or_default();
                        let mut grp_txt_str = grp_txt.to_string();
                        if ui.text_edit_singleline(&mut grp_txt_str).changed() {
                            grp_txt = grp_txt_str.parse::<u16>().unwrap_or_default();
                        }
                        *opt_grp = Some(grp_txt);
                    }

                    /* -------- all AuthorizedActionTakers variants -------- */
                    TokenConfigurationChangeItem::ManualMinting(t)
                    | TokenConfigurationChangeItem::ManualMintingAdminGroup(t)
                    | TokenConfigurationChangeItem::ManualBurning(t)
                    | TokenConfigurationChangeItem::ManualBurningAdminGroup(t)
                    | TokenConfigurationChangeItem::Freeze(t)
                    | TokenConfigurationChangeItem::FreezeAdminGroup(t)
                    | TokenConfigurationChangeItem::Unfreeze(t)
                    | TokenConfigurationChangeItem::UnfreezeAdminGroup(t)
                    | TokenConfigurationChangeItem::DestroyFrozenFunds(t)
                    | TokenConfigurationChangeItem::DestroyFrozenFundsAdminGroup(t)
                    | TokenConfigurationChangeItem::EmergencyAction(t)
                    | TokenConfigurationChangeItem::EmergencyActionAdminGroup(t)
                    | TokenConfigurationChangeItem::ConventionsControlGroup(t)
                    | TokenConfigurationChangeItem::ConventionsAdminGroup(t)
                    | TokenConfigurationChangeItem::MaxSupplyControlGroup(t)
                    | TokenConfigurationChangeItem::MaxSupplyAdminGroup(t)
                    | TokenConfigurationChangeItem::PerpetualDistributionControlGroup(t)
                    | TokenConfigurationChangeItem::PerpetualDistributionAdminGroup(t)
                    | TokenConfigurationChangeItem::NewTokensDestinationIdentityControlGroup(t)
                    | TokenConfigurationChangeItem::NewTokensDestinationIdentityAdminGroup(t)
                    | TokenConfigurationChangeItem::MintingAllowChoosingDestinationControlGroup(t)
                    | TokenConfigurationChangeItem::MintingAllowChoosingDestinationAdminGroup(t) =>
                    {
                        Self::render_authorized_action_takers_editor(ui, t, idx);
                    }

                    TokenConfigurationChangeItem::TokenConfigurationNoChange => {
                        ui.label("No parameters to edit for this entry.");
                    }
                }
            });

            ui.add_space(6.0);
        }

        /* ---------- removal pass ---------- */
        for i in to_remove.into_iter().rev() {
            self.change_items.remove(i);
        }

        /* ---------- add / submit buttons ---------- */
        if ui.button("+ Add another item").clicked() {
            self.change_items
                .push(TokenConfigurationChangeItem::TokenConfigurationNoChange);
        }

        ui.add_space(10.0);

        if !self.change_items.is_empty() {
            let updating = ui.button("Broadcast Update");
            if updating.clicked() {
                self.update_status = UpdateTokenConfigStatus::Updating(Utc::now());
                action =
                    AppAction::BackendTask(BackendTask::TokenTask(TokenTask::UpdateTokenConfig {
                        identity_token_balance: self.identity_token_balance.clone(),
                        change_items: self.change_items.clone(),
                        signing_key: self.signing_key.clone(),
                        public_note: self.public_note.clone(),
                    }));
            }
        }

        action
    }

    /* ===================================================================== */
    /* Helper: render AuthorizedActionTakers editor                          */
    /* ===================================================================== */
    fn render_authorized_action_takers_editor(
        ui: &mut egui::Ui,
        takers: &mut AuthorizedActionTakers,
        row_idx: usize,
    ) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(format!("aat_combo_{row_idx}"))
                .selected_text(format!("{takers:?}"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(takers, AuthorizedActionTakers::NoOne, "No One");
                    ui.selectable_value(
                        takers,
                        AuthorizedActionTakers::ContractOwner,
                        "Contract Owner",
                    );
                    ui.selectable_value(takers, AuthorizedActionTakers::MainGroup, "Main Group");
                    ui.selectable_value(
                        takers,
                        AuthorizedActionTakers::Identity(Default::default()),
                        "Specific Identity",
                    );
                    ui.selectable_value(takers, AuthorizedActionTakers::Group(0), "Specific Group");
                });

            match takers {
                AuthorizedActionTakers::Identity(id) => {
                    let mut txt = id.to_string();
                    if ui.text_edit_singleline(&mut txt).changed() {
                        *id = Identifier::from_string(&txt, Encoding::Base58).unwrap_or_default();
                    }
                }
                AuthorizedActionTakers::Group(g) => {
                    let mut txt = g.to_string();
                    if ui.text_edit_singleline(&mut txt).changed() {
                        *g = txt.parse::<u16>().unwrap_or(0);
                    }
                }
                _ => {}
            }
        });
    }
}

impl ScreenLike for UpdateTokenConfigScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.message = Some((message.to_string(), MessageType::Success, Utc::now()));
                if message.contains("Successfully updated all token config items") {
                    self.update_status = UpdateTokenConfigStatus::NotUpdating;
                }
            }
            MessageType::Error => {
                self.message = Some((message.to_string(), MessageType::Error, Utc::now()));
                if message.contains("Failed to update token config") {
                    self.update_status = UpdateTokenConfigStatus::NotUpdating;
                }
            }
            MessageType::Info => {
                self.message = Some((message.to_string(), MessageType::Info, Utc::now()));
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        // Top panel
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_balance.token_alias,
                    AppAction::PopScreen,
                ),
                ("Update Config", AppAction::None),
            ],
            vec![],
        );

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        // Central panel
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Update Token Configuration");
            ui.add_space(10.0);

            action |= self.render_token_config_updater(ui);
            ui.add_space(10.0);

            if let Some((msg, msg_type, _)) = &self.message {
                ui.add_space(10.0);
                match msg_type {
                    MessageType::Success => {
                        ui.colored_label(Color32::DARK_GREEN, msg);
                    }
                    MessageType::Error => {
                        ui.colored_label(Color32::DARK_RED, msg);
                    }
                    MessageType::Info => {
                        ui.label(msg);
                    }
                };
            }

            if self.update_status != UpdateTokenConfigStatus::NotUpdating {
                ui.add_space(10.0);
                match &self.update_status {
                    UpdateTokenConfigStatus::Updating(start_time) => {
                        let elapsed = Utc::now().signed_duration_since(*start_time);
                        ui.label(format!("Updating... ({} seconds)", elapsed.num_seconds()));
                    }
                    _ => {}
                }
            }
        });

        action
    }
}
