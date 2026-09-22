//! The fee pots of a contract (protocol version 14): what its document action
//! fees have collected for the contract owner and for its moderators, and
//! claiming a pot with an identity that receives it.

use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::{add_contract_chooser_pre_filtered, format_timestamp_ms_local};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::action_fees::ContractFeePot;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use dash_sdk::platform::contract_fee_pots::{ContractFeePotState, ContractFeePots};
use eframe::egui::{self, RichText};
use std::sync::Arc;

/// What the screen knows about the selected contract's fee pots.
#[derive(Debug, Clone, PartialEq)]
enum FeePotsState {
    /// No contract selected, or its pots not asked for yet.
    NotLoaded,
    /// The read for this contract is in flight.
    Loading(Identifier),
    /// Platform's answer for this contract.
    Loaded {
        contract_id: Identifier,
        pots: ContractFeePots,
    },
    /// The read failed; the banner says why.
    Failed(Identifier),
}

impl FeePotsState {
    fn contract_id(&self) -> Option<Identifier> {
        match self {
            FeePotsState::NotLoaded => None,
            FeePotsState::Loading(id) | FeePotsState::Failed(id) => Some(*id),
            FeePotsState::Loaded { contract_id, .. } => Some(*contract_id),
        }
    }
}

pub struct ContractFeePotsScreen {
    pub app_context: Arc<AppContext>,
    contracts: Vec<QualifiedContract>,
    contract_search: String,
    selected_contract: Option<QualifiedContract>,
    identities: Vec<QualifiedIdentity>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_str: String,
    pots: FeePotsState,
    /// The pot a claim is in flight for; claiming is disabled meanwhile.
    claim_in_flight: Option<ContractFeePot>,
    progress_banner: Option<BannerHandle>,
}

impl ContractFeePotsScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let contracts = app_context.get_contracts().unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to load contracts for the fee pots screen");
            vec![]
        });
        let identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "Failed to load identities for the fee pots screen");
                vec![]
            });
        Self {
            app_context: app_context.clone(),
            contracts,
            contract_search: String::new(),
            selected_contract: None,
            identities,
            selected_identity: None,
            selected_identity_str: String::new(),
            pots: FeePotsState::NotLoaded,
            claim_in_flight: None,
            progress_banner: None,
        }
    }

    fn fetch_pots(&mut self, contract_id: Identifier) -> AppAction {
        self.pots = FeePotsState::Loading(contract_id);
        AppAction::BackendTask(BackendTask::ContractTask(Box::new(
            ContractTask::FetchContractFeePots(contract_id),
        )))
    }

    fn contract_matches_search(&self, contract: &QualifiedContract) -> bool {
        let term = self.contract_search.to_lowercase();
        term.is_empty()
            || contract
                .alias
                .as_ref()
                .is_some_and(|alias| alias.to_lowercase().contains(&term))
            || contract
                .contract
                .id()
                .to_string(Encoding::Base58)
                .to_lowercase()
                .contains(&term)
    }

    /// One row of the pots grid: the pot, its credits, its last payout and,
    /// for a selected identity that receives it, the Claim button.
    fn render_pot_row(
        &mut self,
        ui: &mut egui::Ui,
        contract: &QualifiedContract,
        pot: ContractFeePot,
        state: &ContractFeePotState,
    ) -> AppAction {
        let dark_mode = ui.style().visuals.dark_mode;
        ui.label(match pot {
            ContractFeePot::Owner => "Contract owner",
            ContractFeePot::Moderators => "Moderators",
        });
        ui.label(format_credits_as_dash(state.credits));
        ui.label(match state.last_claim {
            Some(last_claim) => format!(
                "Epoch {epoch}, {date}, by {claimant}",
                epoch = last_claim.epoch_index,
                date = format_timestamp_ms_local(last_claim.time_ms),
                claimant = last_claim.claimant_id.to_string(Encoding::Base58)
            ),
            None => "Never claimed".to_string(),
        });

        let recipients = pot.recipients(&contract.contract);
        let claimant = self
            .selected_identity
            .as_ref()
            .filter(|identity| recipients.contains(&identity.identity.id()));
        let disabled_reason = if recipients.is_empty() {
            Some("This contract has no moderators, so nobody can claim these fees.")
        } else if self.selected_identity.is_none() {
            Some("Select an identity above to claim these fees.")
        } else if claimant.is_none() {
            Some("The selected identity does not receive these fees.")
        } else if state.credits == 0 {
            Some("There are no fees to claim yet.")
        } else if self.claim_in_flight.is_some() {
            Some("A claim is in progress.")
        } else {
            None
        };

        let mut action = AppAction::None;
        let button = ui.add_enabled(disabled_reason.is_none(), egui::Button::new("Claim"));
        if let Some(reason) = disabled_reason {
            button.on_disabled_hover_text(
                RichText::new(reason).color(DashColors::text_secondary(dark_mode)),
            );
        } else if button.clicked()
            && let Some(identity) = claimant
        {
            self.claim_in_flight = Some(pot);
            self.progress_banner.take_and_clear();
            let handle =
                MessageBanner::set_global(ui.ctx(), "Claiming the fees...", MessageType::Info);
            handle.with_elapsed();
            self.progress_banner = Some(handle);
            action = AppAction::BackendTask(BackendTask::ContractTask(Box::new(
                ContractTask::ClaimContractFees {
                    contract_id: contract.contract.id(),
                    pot,
                    identity: Box::new(identity.clone()),
                },
            )));
        }
        ui.end_row();
        action
    }
}

impl ScreenLike for ContractFeePotsScreen {
    fn refresh(&mut self) {
        if let Ok(identities) = self.app_context.load_local_qualified_identities() {
            self.identities = identities;
        }
        self.pots = FeePotsState::NotLoaded;
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // The result banner is set by AppState; settle this screen's state.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.progress_banner.take_and_clear();
            self.claim_in_flight = None;
            if let FeePotsState::Loading(contract_id) = self.pots {
                self.pots = FeePotsState::Failed(contract_id);
            }
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::ContractFeePots { contract_id, pots }
                if self.pots == FeePotsState::Loading(contract_id) =>
            {
                self.pots = FeePotsState::Loaded { contract_id, pots };
            }
            BackendTaskSuccessResult::ContractFeesClaimed {
                contract_id,
                claimant_balance,
                ..
            } => {
                self.progress_banner.take_and_clear();
                self.claim_in_flight = None;
                let message = match claimant_balance {
                    Some(balance) => format!(
                        "The fees were paid out. The identity's balance is now {balance}.",
                        balance = format_credits_as_dash(balance)
                    ),
                    None => "The fees were paid out.".to_string(),
                };
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    message,
                    MessageType::Success,
                );
                // Show the emptied pot and the new balance.
                if let Ok(identities) = self.app_context.load_local_qualified_identities() {
                    self.identities = identities;
                }
                if self.pots.contract_id() == Some(contract_id) {
                    self.pots = FeePotsState::NotLoaded;
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Fee Pots", AppAction::None),
            ],
            vec![],
        );
        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.style().visuals.dark_mode;
            ui.heading("Contract Fee Pots");
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Fees that users pay for document actions collect in two pots: one for the contract owner and one shared by its moderators. A pot can be claimed once per epoch.",
                )
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(10.0);

            if !FeatureGate::ContractFeePots.is_available(&self.app_context) {
                ui.label(
                    "This network does not support contract fees yet. Connect to a network that supports them to see fee pots.",
                );
                return inner_action;
            }

            ui.heading("1. Select a contract:");
            ui.add_space(6.0);
            let visible: Vec<QualifiedContract> = self
                .contracts
                .iter()
                .filter(|contract| self.contract_matches_search(contract))
                .cloned()
                .collect();
            add_contract_chooser_pre_filtered(
                ui,
                &mut self.contract_search,
                visible.iter(),
                &mut self.selected_contract,
            );

            ui.add_space(10.0);
            ui.heading("2. Select the identity that claims:");
            ui.add_space(6.0);
            // Session-local selection: claiming for a contract must not
            // re-point the app-wide identity selection.
            ui.add(
                IdentitySelector::new(
                    "contract_fee_pots_identity_selector",
                    &mut self.selected_identity_str,
                    &self.identities,
                )
                .selected_identity(&mut self.selected_identity)
                .expect("identity selector over the loaded identities")
                .other_option(false)
                .width(250.0)
                .label("Identity:"),
            );

            let Some(contract) = self.selected_contract.clone() else {
                return inner_action;
            };
            let contract_id = contract.contract.id();
            if self.pots.contract_id() != Some(contract_id) {
                inner_action |= self.fetch_pots(contract_id);
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            match self.pots.clone() {
                FeePotsState::Loading(_) | FeePotsState::NotLoaded => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading the fee pots...");
                    });
                }
                FeePotsState::Failed(_) => {
                    if ui.button("Try Again").clicked() {
                        inner_action |= self.fetch_pots(contract_id);
                    }
                }
                FeePotsState::Loaded { pots, .. } => {
                    egui::Grid::new("contract_fee_pots_grid")
                        .num_columns(4)
                        .spacing([16.0, 8.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label(RichText::new("Pot").strong());
                            ui.label(RichText::new("Available").strong());
                            ui.label(RichText::new("Last claimed").strong());
                            ui.label("");
                            ui.end_row();
                            inner_action |= self.render_pot_row(
                                ui,
                                &contract,
                                ContractFeePot::Owner,
                                &pots.owner,
                            );
                            inner_action |= self.render_pot_row(
                                ui,
                                &contract,
                                ContractFeePot::Moderators,
                                &pots.moderators,
                            );
                        });
                    ui.add_space(8.0);
                    if ui.button("Refresh").clicked() {
                        inner_action |= self.fetch_pots(contract_id);
                    }
                }
            }
            inner_action
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pots_state_names_the_contract_it_is_about() {
        let id = Identifier::random();
        assert_eq!(FeePotsState::NotLoaded.contract_id(), None);
        assert_eq!(FeePotsState::Loading(id).contract_id(), Some(id));
        assert_eq!(FeePotsState::Failed(id).contract_id(), Some(id));
        assert_eq!(
            FeePotsState::Loaded {
                contract_id: id,
                pots: ContractFeePots::default(),
            }
            .contract_id(),
            Some(id)
        );
    }
}
