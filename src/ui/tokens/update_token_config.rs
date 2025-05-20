use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};
use crate::ui::contracts_documents::group_actions_screen::GroupActionsScreen;
use chrono::{DateTime, Utc};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::v0::TokenPerpetualDistributionV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::TokenPerpetualDistribution;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::render_group_action_text;
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateTokenConfigStatus {
    NotUpdating,
    Updating(DateTime<Utc>),
}

pub struct UpdateTokenConfigScreen {
    pub identity_token_info: IdentityTokenInfo,
    data_contract_option: Option<QualifiedContract>,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
    update_status: UpdateTokenConfigStatus,
    pub app_context: Arc<AppContext>,
    pub change_item: TokenConfigurationChangeItem,
    signing_key: Option<IdentityPublicKey>,
    identity: QualifiedIdentity,
    pub public_note: Option<String>,
    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,

    // Input state fields
    pub authorized_identity_input: Option<String>,
    pub authorized_group_input: Option<String>,

    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>, // unused
}

impl UpdateTokenConfigScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let possible_key = identity_token_info
            .identity
            .identity
            .get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            )
            .cloned();

        let mut error_message = None;

        let group = match identity_token_info
            .token_config
            .manual_minting_rules()
            .authorized_to_make_change_action_takers()
        {
            AuthorizedActionTakers::NoOne => {
                error_message = Some("Minting is not allowed on this token".to_string());
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != &identity_token_info.identity.identity.id()
                {
                    error_message = Some(
                        "You are not allowed to mint this token. Only the contract owner is."
                            .to_string(),
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    error_message = Some("You are not allowed to mint this token".to_string());
                }
                None
            }
            AuthorizedActionTakers::MainGroup => {
                match identity_token_info.token_config.main_control_group() {
                    None => {
                        error_message = Some(
                            "Invalid contract: No main control group, though one should exist"
                                .to_string(),
                        );
                        None
                    }
                    Some(group_pos) => {
                        match identity_token_info
                            .data_contract
                            .contract
                            .expected_group(group_pos)
                        {
                            Ok(group) => Some((group_pos, group.clone())),
                            Err(e) => {
                                error_message = Some(format!("Invalid contract: {}", e));
                                None
                            }
                        }
                    }
                }
            }
            AuthorizedActionTakers::Group(group_pos) => {
                match identity_token_info
                    .data_contract
                    .contract
                    .expected_group(*group_pos)
                {
                    Ok(group) => Some((*group_pos, group.clone())),
                    Err(e) => {
                        error_message = Some(format!("Invalid contract: {}", e));
                        None
                    }
                }
            }
        };

        let mut is_unilateral_group_member = false;
        if group.is_some() {
            if let Some((_, group)) = group.clone() {
                let your_power = group
                    .members()
                    .get(&identity_token_info.identity.identity.id());

                if let Some(your_power) = your_power {
                    if your_power >= &group.required_power() {
                        is_unilateral_group_member = true;
                    }
                }
            }
        };

        // Attempt to get an unlocked wallet reference
        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key.as_ref(),
            &mut error_message,
        );

        let data_contract_option = app_context
            .get_contract_by_id(&identity_token_info.data_contract.contract.id())
            .unwrap_or_default();

        Self {
            identity_token_info: identity_token_info.clone(),
            data_contract_option,
            backend_message: None,
            update_status: UpdateTokenConfigStatus::NotUpdating,
            app_context: app_context.clone(),
            change_item: TokenConfigurationChangeItem::TokenConfigurationNoChange,
            signing_key: possible_key,
            public_note: None,

            authorized_identity_input: None,
            authorized_group_input: None,

            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message,

            identity: identity_token_info.identity,
            group,
            is_unilateral_group_member,
            group_action_id: None,
        }
    }

    fn render_token_config_updater(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("2. Select the item to update");
        ui.add_space(10.0);
        if self.group_action_id.is_some() {
            ui.label("You are signing an existing group action. Make sure you construct the exact same item as the one in the group action, details of which can be found on the previous screen.");
        }

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
                Self::render_authorized_action_takers_editor(
                    ui,
                    t,
                    &mut self.authorized_identity_input,
                    &mut self.authorized_group_input,
                    &self.data_contract_option,
                );
            }

            TokenConfigurationChangeItem::TokenConfigurationNoChange => {
                ui.label("No parameters to edit for this entry.");
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.heading("3. Public note (optional)");
        ui.add_space(5.0);
        if self.group_action_id.is_some() {
            ui.label(
                "You are signing an existing group ConfigUpdate so you are not allowed to put a note.",
            );
            ui.add_space(5.0);
            ui.label(format!(
                "Note: {}",
                self.public_note.clone().unwrap_or("None".to_string())
            ));
        } else {
            ui.horizontal(|ui| {
                ui.label("Public note (optional):");
                ui.add_space(10.0);
                let mut txt = self.public_note.clone().unwrap_or_default();
                if ui
                    .text_edit_singleline(&mut txt)
                    .on_hover_text("A note about the transaction that can be seen by the public.")
                    .changed()
                {
                    self.public_note = if txt.len() > 0 { Some(txt) } else { None };
                }
            });
        }

        let button_text = render_group_action_text(
            ui,
            &self.group,
            &self.identity_token_info,
            "Update Config",
            &self.group_action_id,
        );

        let button = egui::Button::new(RichText::new(&button_text).color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .frame(true)
            .corner_radius(3.0);

        if self.app_context.developer_mode || !button_text.contains("Test") {
            ui.add_space(20.0);
            if ui.add(button).clicked() {
                let group_info;
                if self.group_action_id.is_some() {
                    group_info = self.group.as_ref().map(|(pos, _)| {
                        GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                            GroupStateTransitionInfo {
                                group_contract_position: *pos,
                                action_id: self.group_action_id.unwrap(),
                                action_is_proposer: false,
                            },
                        )
                    });
                } else {
                    group_info = self.group.as_ref().map(|(pos, _)| {
                        GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
                    });
                }

                self.update_status = UpdateTokenConfigStatus::Updating(Utc::now());
                action =
                    AppAction::BackendTask(BackendTask::TokenTask(TokenTask::UpdateTokenConfig {
                        identity_token_info: self.identity_token_info.clone(),
                        change_item: self.change_item.clone(),
                        signing_key: self.signing_key.clone().expect("Signing key must be set"),
                        public_note: if self.group_action_id.is_some() {
                            None
                        } else {
                            self.public_note.clone()
                        },
                        group_info,
                    }));
            }
        }

        action
    }

    /* ===================================================================== */
    /* Helper: render AuthorizedActionTakers editor                          */
    /* ===================================================================== */
    pub fn render_authorized_action_takers_editor(
        ui: &mut egui::Ui,
        takers: &mut AuthorizedActionTakers,
        authorized_identity_input: &mut Option<String>,
        authorized_group_input: &mut Option<String>,
        data_contract_option: &Option<QualifiedContract>,
    ) {
        ui.horizontal(|ui| {
            // Display label
            ui.label("Authorized:");

            // Combo box for selecting the type of authorized taker
            egui::ComboBox::from_id_salt("authorized_action_takers")
                .selected_text(match takers {
                    AuthorizedActionTakers::NoOne => "No One".to_string(),
                    AuthorizedActionTakers::ContractOwner => "Contract Owner".to_string(),
                    AuthorizedActionTakers::MainGroup => "Main Group".to_string(),
                    AuthorizedActionTakers::Identity(id) => {
                        if id == &Identifier::default() {
                            "Identity".to_string()
                        } else {
                            format!("Identity({})", id)
                        }
                    }
                    AuthorizedActionTakers::Group(_) => format!("Group"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(takers, AuthorizedActionTakers::NoOne, "No One");
                    ui.selectable_value(
                        takers,
                        AuthorizedActionTakers::ContractOwner,
                        "Contract Owner",
                    );
                    ui.selectable_value(takers, AuthorizedActionTakers::MainGroup, "Main Group");

                    // Set temporary input fields on select
                    if ui
                        .selectable_label(
                            matches!(takers, AuthorizedActionTakers::Identity(_)),
                            "Identity",
                        )
                        .clicked()
                    {
                        *takers = AuthorizedActionTakers::Identity(Identifier::default());
                        authorized_identity_input.get_or_insert_with(String::new);
                    }

                    if ui
                        .selectable_label(
                            matches!(takers, AuthorizedActionTakers::Group(_)),
                            "Group",
                        )
                        .clicked()
                    {
                        *takers = AuthorizedActionTakers::Group(0);
                        authorized_group_input.get_or_insert_with(|| "0".to_owned());
                    }
                });

            // Render input for Identity
            if let AuthorizedActionTakers::Identity(id) = takers {
                authorized_identity_input.get_or_insert_with(String::new);
                if let Some(ref mut id_str) = authorized_identity_input {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [300.0, 22.0],
                            egui::TextEdit::singleline(id_str).hint_text("Enter base58 identity"),
                        );

                        if !id_str.is_empty() {
                            let is_valid =
                                Identifier::from_string(id_str, Encoding::Base58).is_ok();
                            let (symbol, color) = if is_valid {
                                ("✔", Color32::DARK_GREEN)
                            } else {
                                ("×", Color32::RED)
                            };
                            ui.label(RichText::new(symbol).color(color).strong());

                            if is_valid {
                                *id = Identifier::from_string(id_str, Encoding::Base58).unwrap();
                            }
                        }
                    });
                }
            }

            // Render input for Group
            if let Some(data_contract) = data_contract_option {
                let contract_group_positions: Vec<u16> =
                    data_contract.contract.groups().keys().cloned().collect();
                if let AuthorizedActionTakers::Group(g) = takers {
                    authorized_group_input.get_or_insert_with(|| g.to_string());
                    egui::ComboBox::from_id_salt("group_position_selector")
                        .selected_text(format!(
                            "Group Position: {}",
                            authorized_group_input.as_deref().unwrap_or(&g.to_string())
                        ))
                        .show_ui(ui, |ui| {
                            for position in &contract_group_positions {
                                if ui
                                    .selectable_value(g, *position, format!("Group {}", position))
                                    .clicked()
                                {
                                    *authorized_group_input = Some(position.to_string());
                                }
                            }
                        });
                }
            } else {
                if let AuthorizedActionTakers::Group(g) = takers {
                    authorized_group_input.get_or_insert_with(|| g.to_string());
                    if let Some(ref mut group_str) = authorized_group_input {
                        ui.add(
                            egui::TextEdit::singleline(group_str).hint_text("Enter group position"),
                        );
                        if let Ok(parsed) = group_str.parse::<u16>() {
                            *g = parsed;
                        }
                    }
                }
            }
        });
    }

    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            if self.group_action_id.is_some() {
                // This ConfigUpdate is already initiated by the group, we are just signing it
                ui.heading("Group ConfigUpdate Signing Successful.");
            } else {
                if !self.is_unilateral_group_member {
                    ui.heading("Group ConfigUpdate Initiated.");
                } else {
                    ui.heading("ConfigUpdate Successful.");
                }
            }

            ui.add_space(20.0);

            if self.group_action_id.is_some() {
                if ui.button("Back to Group Actions").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenMyTokenBalances,
                    );
                }
            } else {
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }

                if !self.is_unilateral_group_member {
                    if ui.button("Go to Group Actions").clicked() {
                        action = AppAction::PopThenAddScreenToMainScreen(
                            RootScreenType::RootScreenDocumentQuery,
                            Screen::GroupActionsScreen(GroupActionsScreen::new(
                                &self.app_context.clone(),
                            )),
                        );
                    }
                }
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
                            let is_valid = key.purpose() == Purpose::AUTHENTICATION
                                && key.security_level() == SecurityLevel::CRITICAL;

                            let label = format!(
                                "Key ID: {} (Info: {}/{}/{})",
                                key.id(),
                                key.purpose(),
                                key.security_level(),
                                key.key_type()
                            );
                            let styled_label = if is_valid {
                                RichText::new(label.clone())
                            } else {
                                RichText::new(label.clone()).color(Color32::RED)
                            };

                            ui.selectable_value(
                                &mut self.signing_key,
                                Some(key.clone()),
                                styled_label,
                            );
                        }
                    } else {
                        // Show only "available" auth keys
                        for key_wrapper in self
                            .identity
                            .available_authentication_keys_with_critical_security_level()
                        {
                            let key = &key_wrapper.identity_public_key;
                            let label = format!(
                                "Key ID: {} (Info: {}/{}/{})",
                                key.id(),
                                key.purpose(),
                                key.security_level(),
                                key.key_type()
                            );
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
        let mut action;

        // Build a top panel
        if self.group_action_id.is_some() {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Contracts", AppAction::GoToMainScreen),
                    ("Group Actions", AppAction::PopScreen),
                    ("Update Token Config", AppAction::None),
                ],
                vec![],
            );
        } else {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Tokens", AppAction::GoToMainScreen),
                    (&self.identity_token_info.token_alias, AppAction::PopScreen),
                    ("Update Token Config", AppAction::None),
                ],
                vec![],
            );
        }

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

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self
                    .identity
                    .available_authentication_keys_with_critical_security_level()
                    .is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    Color32::DARK_RED,
                    format!(
                        "No authentication keys with CRITICAL security level found for this {} identity.",
                        self.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                // Show "Add key" or "Check keys" option
                let first_key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Possibly handle locked wallet scenario (similar to TransferTokens)
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        // Must unlock before we can proceed
                        return;
                    }
                }

                // 1) Key selection
                ui.heading("1. Select the key to sign the transaction with");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    self.render_key_selection(ui);
                    ui.add_space(5.0);
                    let identity_id_string =
                        self.identity.identity.id().to_string(Encoding::Base58);
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
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for UpdateTokenConfigScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}
