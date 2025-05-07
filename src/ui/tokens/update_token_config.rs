use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::v0::TokenPerpetualDistributionV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::TokenPerpetualDistribution;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui::{Color32, Ui};
use egui::{Context, RichText};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use super::tokens_screen::IdentityTokenBalance;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateTokenConfigStatus {
    NotUpdating,
    Updating(DateTime<Utc>),
}

pub struct UpdateTokenConfigScreen {
    pub identity_token_balance: IdentityTokenBalance,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
    update_status: UpdateTokenConfigStatus,
    pub app_context: Arc<AppContext>,
    change_item: TokenConfigurationChangeItem,
    signing_key: Option<IdentityPublicKey>,
    identity: QualifiedIdentity,
    public_note: Option<String>,
}

impl UpdateTokenConfigScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        // Find the local qualified identity that corresponds to `identity_token_balance.identity_id`
        let identity = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|id| id.identity.id() == identity_token_balance.identity_id)
            .expect("No local qualified identity found matching the token's identity");

        // Grab a default key if possible
        let identity_clone = identity.identity.clone();
        let possible_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
                SecurityLevel::CRITICAL,
            ]),
            KeyType::all_key_types().into(),
            false,
        );

        Self {
            identity_token_balance: identity_token_balance.clone(),
            backend_message: None,
            update_status: UpdateTokenConfigStatus::NotUpdating,
            app_context: app_context.clone(),
            change_item: TokenConfigurationChangeItem::TokenConfigurationNoChange,
            signing_key: possible_key.cloned(),
            identity,
            public_note: None,
        }
    }

    fn render_token_config_updater(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("2. Select the item to update");
        ui.add_space(10.0);

        let item = &mut self.change_item;

        ui.horizontal(|ui| {
            let label = match item {
                TokenConfigurationChangeItem::TokenConfigurationNoChange => "No Change",
                TokenConfigurationChangeItem::Conventions(_) => "Conventions",
                TokenConfigurationChangeItem::ConventionsControlGroup(_) => {
                    "Conventions Control Group"
                }
                TokenConfigurationChangeItem::ConventionsAdminGroup(_) => "Conventions Admin Group",
                TokenConfigurationChangeItem::MaxSupply(_) => "Max Supply",
                TokenConfigurationChangeItem::MaxSupplyControlGroup(_) => {
                    "Max Supply Control Group"
                }
                TokenConfigurationChangeItem::MaxSupplyAdminGroup(_) => "Max Supply Admin Group",
                TokenConfigurationChangeItem::PerpetualDistribution(_) => "Perpetual Distribution",
                TokenConfigurationChangeItem::PerpetualDistributionControlGroup(_) => {
                    "Perpetual Distribution Control Group"
                }
                TokenConfigurationChangeItem::PerpetualDistributionAdminGroup(_) => {
                    "Perpetual Distribution Admin Group"
                }
                TokenConfigurationChangeItem::NewTokensDestinationIdentity(_) => {
                    "New‑Tokens Destination"
                }
                TokenConfigurationChangeItem::NewTokensDestinationIdentityControlGroup(_) => {
                    "New‑Tokens Destination Control Group"
                }
                TokenConfigurationChangeItem::NewTokensDestinationIdentityAdminGroup(_) => {
                    "New‑Tokens Destination Admin Group"
                }
                TokenConfigurationChangeItem::MintingAllowChoosingDestination(_) => {
                    "Minting Allow Choosing Destination"
                }
                TokenConfigurationChangeItem::MintingAllowChoosingDestinationControlGroup(_) => {
                    "Minting Allow Choosing Destination Control Group"
                }
                TokenConfigurationChangeItem::MintingAllowChoosingDestinationAdminGroup(_) => {
                    "Minting Allow Choosing Destination Admin Group"
                }
                TokenConfigurationChangeItem::ManualMinting(_) => "Manual Minting",
                TokenConfigurationChangeItem::ManualMintingAdminGroup(_) => {
                    "Manual Minting Admin Group"
                }
                TokenConfigurationChangeItem::ManualBurning(_) => "Manual Burning",
                TokenConfigurationChangeItem::ManualBurningAdminGroup(_) => {
                    "Manual Burning Admin Group"
                }
                TokenConfigurationChangeItem::Freeze(_) => "Freeze",
                TokenConfigurationChangeItem::FreezeAdminGroup(_) => "Freeze Admin Group",
                TokenConfigurationChangeItem::Unfreeze(_) => "Unfreeze",
                TokenConfigurationChangeItem::UnfreezeAdminGroup(_) => "Unfreeze Admin Group",
                TokenConfigurationChangeItem::DestroyFrozenFunds(_) => "Destroy Frozen Funds",
                TokenConfigurationChangeItem::DestroyFrozenFundsAdminGroup(_) => {
                    "Destroy Frozen Funds Admin Group"
                }
                TokenConfigurationChangeItem::EmergencyAction(_) => "Emergency Action",
                TokenConfigurationChangeItem::EmergencyActionAdminGroup(_) => {
                    "Emergency Action Admin Group"
                }
                TokenConfigurationChangeItem::MainControlGroup(_) => "Main Control Group",
            };

            egui::ComboBox::from_id_salt(format!("cfg_item_type"))
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
                        TokenConfigurationChangeItem::NewTokensDestinationIdentity(Some(
                            Identifier::default(),
                        )),
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
                                TokenConfigurationChangeItem::$variant(
                                    AuthorizedActionTakers::ContractOwner,
                                ),
                                $label,
                            );
                        };
                    }

                    aat_item!(ManualMinting, "Manual Minting");
                    aat_item!(ManualMintingAdminGroup, "Manual Minting Admin Group");
                    ui.separator();
                    aat_item!(ManualBurning, "Manual Burning");
                    aat_item!(ManualBurningAdminGroup, "Manual Burning Admin Group");
                    ui.separator();
                    aat_item!(Freeze, "Freeze");
                    aat_item!(FreezeAdminGroup, "Freeze Admin Group");
                    ui.separator();
                    aat_item!(Unfreeze, "Unfreeze");
                    aat_item!(UnfreezeAdminGroup, "Unfreeze Admin Group");
                    ui.separator();
                    aat_item!(DestroyFrozenFunds, "Destroy Frozen Funds");
                    aat_item!(
                        DestroyFrozenFundsAdminGroup,
                        "Destroy Frozen Funds Admin Group"
                    );
                    ui.separator();
                    aat_item!(EmergencyAction, "Emergency Action");
                    aat_item!(EmergencyActionAdminGroup, "Emergency Action Admin Group");

                    ui.separator();

                    ui.selectable_value(
                        item,
                        TokenConfigurationChangeItem::MainControlGroup(Some(0)),
                        "Main Control Group",
                    );
                });
        });

        ui.add_space(10.0);

        /* ========== PER‑VARIANT EDITING ========== */
        match item {
            /* -------- simple value items -------- */
            TokenConfigurationChangeItem::Conventions(conv) => {
                ui.label(
                    "Paste a replacement JSON if you can't manually make the changes you want.",
                );
                ui.add_space(5.0);
                let mut txt = serde_json::to_string_pretty(conv).unwrap_or_default();
                if ui.text_edit_multiline(&mut txt).changed() {
                    *conv = serde_json::from_str(&txt).unwrap_or(TokenConfigurationConvention::V0(
                        TokenConfigurationConventionV0 {
                            localizations: BTreeMap::new(),
                            decimals: 8,
                        },
                    ));
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
                let mut txt = opt_id
                    .map(|id| id.to_string(Encoding::Base58))
                    .unwrap_or_default();
                if ui.text_edit_singleline(&mut txt).changed() {
                    *opt_id = Identifier::from_string(&txt, Encoding::Base58).ok();
                }
            }

            TokenConfigurationChangeItem::PerpetualDistribution(opt_json) => {
                ui.horizontal(|ui| {
                    if ui.button("Set to None").clicked() {
                        *opt_json = None;
                    }

                    if opt_json.is_none() {
                        if ui.button("Open Editor").clicked() {
                            *opt_json = Some(TokenPerpetualDistribution::V0(
                                TokenPerpetualDistributionV0 {
                                    distribution_type:
                                        RewardDistributionType::BlockBasedDistribution {
                                            interval: 100,
                                            function: DistributionFunction::FixedAmount {
                                                amount: 10,
                                            },
                                        },
                                    distribution_recipient:
                                        TokenDistributionRecipient::ContractOwner,
                                },
                            ));
                        }
                    }
                });

                if let Some(json) = opt_json {
                    ui.add_space(5.0);
                    ui.label(
                        "Paste a replacement JSON if you can't manually make the changes you want.",
                    );
                    ui.add_space(5.0);

                    let mut raw = serde_json::to_string_pretty(json).unwrap_or_default();
                    if ui.text_edit_multiline(&mut raw).changed() {
                        *opt_json = serde_json::from_str(&raw).unwrap_or(Some(
                            TokenPerpetualDistribution::V0(TokenPerpetualDistributionV0 {
                                distribution_type: RewardDistributionType::BlockBasedDistribution {
                                    interval: 100,
                                    function: DistributionFunction::FixedAmount { amount: 10 },
                                },
                                distribution_recipient: TokenDistributionRecipient::ContractOwner,
                            }),
                        ));
                    }
                }
            }

            TokenConfigurationChangeItem::MainControlGroup(opt_grp) => {
                let mut grp_txt = opt_grp.map(|g| g).unwrap_or_default();
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
            | TokenConfigurationChangeItem::MintingAllowChoosingDestinationAdminGroup(t) => {
                Self::render_authorized_action_takers_editor(ui, t);
            }

            TokenConfigurationChangeItem::TokenConfigurationNoChange => {
                ui.label("No parameters to edit for this entry.");
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.heading("3. Public note (optional)");
        ui.add_space(10.0);

        // Render text input for the public note
        ui.horizontal(|ui| {
            ui.label("Public note (optional):");
            ui.add_space(10.0);
            let mut txt = self.public_note.clone().unwrap_or_default();
            if ui
                .text_edit_singleline(&mut txt)
                .on_hover_text("A note to go with the transaction that can be seen by the public")
                .changed()
            {
                self.public_note = Some(txt);
            }
        });

        ui.add_space(20.0);

        let button = egui::Button::new(RichText::new("Broadcast Update").color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            self.update_status = UpdateTokenConfigStatus::Updating(Utc::now());
            action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::UpdateTokenConfig {
                identity_token_balance: self.identity_token_balance.clone(),
                change_item: self.change_item.clone(),
                signing_key: self.signing_key.clone().expect("Signing key must be set"),
                public_note: self.public_note.clone(),
            }));
        }

        action
    }

    /* ===================================================================== */
    /* Helper: render AuthorizedActionTakers editor                          */
    /* ===================================================================== */
    fn render_authorized_action_takers_editor(
        ui: &mut egui::Ui,
        takers: &mut AuthorizedActionTakers,
    ) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(format!("aat_combo"))
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

    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading(format!("{}", self.backend_message.as_ref().unwrap().0));

            ui.add_space(20.0);

            if ui.button("Back to Tokens").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }

    /// Renders a ComboBox or similar for selecting an authentication key
    fn render_key_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Select Key:");
            egui::ComboBox::from_id_salt("token_update_key_selector")
                .selected_text(match &self.signing_key {
                    Some(key) => format!("Key ID: {}", key.id()),
                    None => "Select a key".to_string(),
                })
                .show_ui(ui, |ui| {
                    if self.app_context.developer_mode {
                        // Show all loaded public keys
                        for key in self.identity.identity.public_keys().values() {
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.signing_key, Some(key.clone()), label);
                        }
                    } else {
                        // Show only "available" auth keys
                        for key_wrapper in self.identity.available_authentication_keys() {
                            let key = &key_wrapper.identity_public_key;
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.signing_key, Some(key.clone()), label);
                        }
                    }
                });
        });
    }
}

impl ScreenLike for UpdateTokenConfigScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.backend_message =
                    Some((message.to_string(), MessageType::Success, Utc::now()));
                if message.contains("Successfully updated token config item") {
                    self.update_status = UpdateTokenConfigStatus::NotUpdating;
                }
            }
            MessageType::Error => {
                self.backend_message = Some((message.to_string(), MessageType::Error, Utc::now()));
                if message.contains("Failed to update token config") {
                    self.update_status = UpdateTokenConfigStatus::NotUpdating;
                }
            }
            MessageType::Info => {
                self.backend_message = Some((message.to_string(), MessageType::Info, Utc::now()));
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
            if let Some(msg) = &self.backend_message {
                if msg.1 == MessageType::Success {
                    action |= self.show_success_screen(ui);
                    return;
                }
            }

            ui.heading("Update Token Configuration");
            ui.add_space(10.0);

            // 1) Key selection
            ui.heading("1. Select the key to sign the transaction");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                self.render_key_selection(ui);
                ui.add_space(5.0);
                let identity_id_string = self.identity.identity.id().to_string(Encoding::Base58);
                let identity_display = self
                    .identity
                    .alias
                    .as_deref()
                    .unwrap_or_else(|| &identity_id_string);
                ui.label(format!("Identity: {}", identity_display));
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            action |= self.render_token_config_updater(ui);

            if let Some((msg, msg_type, _)) = &self.backend_message {
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
