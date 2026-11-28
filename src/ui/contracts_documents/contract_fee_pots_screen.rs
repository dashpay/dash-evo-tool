//! The fee pots of a contract (protocol version 14): what its document action
//! fees have collected for the contract owner and for its moderators, and
//! claiming a pot with an identity that receives it.

use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::contract_fee_pots::{SeatedModerators, minimum_claim_credits, pot_claimants};
use crate::model::datetime::local_date_time_from_unix_millis;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::add_contract_chooser_pre_filtered;
use crate::ui::state::PendingRequest;
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
        contract: Arc<dash_sdk::platform::DataContract>,
        pots: ContractFeePots,
        seated_moderators: SeatedModerators,
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

/// What decides whether a pot's Claim button is enabled.
#[derive(Debug, Clone, Copy)]
struct ClaimGate {
    has_recipients: bool,
    identity_selected: bool,
    identity_receives: bool,
    credits: u64,
    minimum_credits: u64,
    claim_in_flight: bool,
}

/// Why a pot cannot be claimed right now, `None` when it can. Each reason is
/// a claim Platform would refuse (and still charge for) or one that cannot
/// be sent yet.
///
/// A claim already made in the current epoch is not blocked here: the only
/// current epoch at hand is the response metadata's, which the proof does not
/// sign. Platform refuses such a claim, and the error says when to retry.
fn claim_blocked_reason(gate: ClaimGate) -> Option<&'static str> {
    if !gate.has_recipients {
        Some("This contract has no moderators, so nobody can claim these fees.")
    } else if !gate.identity_selected {
        Some("Select an identity above to claim these fees.")
    } else if !gate.identity_receives {
        Some("The selected identity does not receive these fees.")
    } else if gate.credits == 0 {
        Some("There are no fees to claim yet.")
    } else if gate.credits < gate.minimum_credits {
        Some(
            "There are not enough fees to pay every moderator yet. Refresh after more fees have collected.",
        )
    } else if gate.claim_in_flight {
        Some("A claim is in progress.")
    } else {
        None
    }
}

const CONTRACTS_LOAD_FAILED: &str = "Your saved contracts could not be loaded from this device. Select Retry Loading on this screen to try again.";
const IDENTITIES_LOAD_FAILED: &str = "Your identities could not be loaded from this device. Select Retry Loading on this screen to try again.";
const LOCAL_DATA_INCOMPLETE: &str =
    "Some saved data could not be loaded, so the lists below may be incomplete.";

pub struct ContractFeePotsScreen {
    pub app_context: Arc<AppContext>,
    contracts: Vec<QualifiedContract>,
    contract_search: String,
    selected_contract: Option<QualifiedContract>,
    identities: Vec<QualifiedIdentity>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_str: String,
    pots: FeePotsState,
    pending_read: PendingRequest<Identifier>,
    pending_claim: PendingRequest,
    /// The pot a claim is in flight for; claiming is disabled meanwhile.
    claim_in_flight: Option<ContractFeePot>,
    confirmation: Option<(
        ConfirmationDialog,
        Identifier,
        ContractFeePot,
        QualifiedIdentity,
    )>,
    progress_banner: Option<BannerHandle>,
    /// Set while the last contract read failed; the banner says why.
    contracts_load_banner: Option<BannerHandle>,
    /// Set while the last identity read failed; the banner says why.
    identities_load_banner: Option<BannerHandle>,
}

impl ContractFeePotsScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let mut screen = Self {
            app_context: app_context.clone(),
            contracts: vec![],
            contract_search: String::new(),
            selected_contract: None,
            identities: vec![],
            selected_identity: None,
            selected_identity_str: String::new(),
            pots: FeePotsState::NotLoaded,
            pending_read: PendingRequest::default(),
            pending_claim: PendingRequest::default(),
            claim_in_flight: None,
            confirmation: None,
            progress_banner: None,
            contracts_load_banner: None,
            identities_load_banner: None,
        };
        screen.reload_local_data();
        screen
    }

    /// Reads the contracts and identities saved on this device.
    fn reload_local_data(&mut self) {
        let contracts = self.app_context.get_contracts();
        let identities = self.app_context.load_local_qualified_identities();
        self.apply_local_data(contracts, identities);
    }

    /// Takes each list that loaded; a failed read keeps the previous list and
    /// raises a banner that stays until a retry succeeds.
    fn apply_local_data(
        &mut self,
        contracts: Result<Vec<QualifiedContract>, TaskError>,
        identities: Result<Vec<QualifiedIdentity>, TaskError>,
    ) {
        let ctx = self.app_context.egui_ctx().clone();
        self.contracts_load_banner.take_and_clear();
        match contracts {
            Ok(contracts) => self.contracts = contracts,
            Err(error) => {
                self.contracts_load_banner =
                    Some(load_failure_banner(&ctx, CONTRACTS_LOAD_FAILED, &error));
            }
        }
        self.identities_load_banner.take_and_clear();
        match identities {
            Ok(identities) => self.apply_identities(identities),
            Err(error) => {
                self.identities_load_banner =
                    Some(load_failure_banner(&ctx, IDENTITIES_LOAD_FAILED, &error));
            }
        }
    }

    fn apply_identities(&mut self, identities: Vec<QualifiedIdentity>) {
        self.selected_identity = self.selected_identity.as_ref().and_then(|selected| {
            identities
                .iter()
                .find(|identity| identity.identity.id() == selected.identity.id())
                .cloned()
        });
        if self.selected_identity.is_none() {
            self.selected_identity_str.clear();
        }
        self.identities = identities;
    }

    fn local_data_failed(&self) -> bool {
        self.contracts_load_banner.is_some() || self.identities_load_banner.is_some()
    }

    fn fetch_pots(&mut self, contract_id: Identifier) -> AppAction {
        self.pots = FeePotsState::Loading(contract_id);
        self.pending_read.dispatch_on_with(
            BackendTask::ContractTask(Box::new(ContractTask::FetchContractFeePots(contract_id))),
            self.app_context.network(),
            contract_id,
        )
    }

    /// The request that pays out `pot` of `contract_id` to `identity`.
    fn claim_request(
        &mut self,
        contract_id: Identifier,
        pot: ContractFeePot,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        self.pending_claim.dispatch_on(
            BackendTask::ContractTask(Box::new(ContractTask::ClaimContractFees {
                contract_id,
                pot,
                identity: Box::new(identity.clone()),
            })),
            self.app_context.network(),
        )
    }

    fn start_claim(
        &mut self,
        _ctx: &egui::Context,
        contract_id: Identifier,
        pot: ContractFeePot,
    ) -> AppAction {
        let Some(identity) = self.selected_identity.clone() else {
            return AppAction::None;
        };
        if self.claim_in_flight.is_some() || self.confirmation.is_some() {
            return AppAction::None;
        }
        let FeePotsState::Loaded {
            contract,
            pots,
            seated_moderators,
            ..
        } = &self.pots
        else {
            return AppAction::None;
        };
        let claimants = pot_claimants(pot, contract, seated_moderators);
        let crate::model::contract_fee_pots::PotClaimants::Known(ids) = claimants else {
            return AppAction::None;
        };
        if !ids.contains(&identity.identity.id()) || contract.id() != contract_id {
            return AppAction::None;
        }
        let credits = match pot {
            ContractFeePot::Owner => pots.owner.credits,
            ContractFeePot::Moderators => pots.moderators.credits,
        };
        let fee = self
            .app_context
            .fee_estimator()
            .estimate_contract_fee_claim(ids.len());
        // TODO: Show the exact seated-team payout when its reward split is available.
        let payout = if pot == ContractFeePot::Moderators
            && matches!(seated_moderators, SeatedModerators::Seated(_))
        {
            None
        } else {
            Some(credits / ids.len() as u64)
        };
        let net = match payout {
            Some(payout) if payout >= fee => format!(
                "Expected net change for your identity: +{amount}.",
                amount = format_credits_as_dash(payout - fee)
            ),
            Some(payout) => format!(
                "Expected net change for your identity: -{amount}. The estimated fee exceeds your payout.",
                amount = format_credits_as_dash(fee - payout)
            ),
            None => format!(
                "Your payout depends on the elected team's reward split. The estimated cost before any payout is {fee}. Your balance may decrease. The exact net change is unavailable.",
                fee = format_credits_as_dash(fee)
            ),
        };
        let message = format!(
            "Pot amount: {amount}.\nEstimated fee paid by your identity: {fee}.\n{net}\nActual fees and payouts may change before the claim completes.",
            amount = format_credits_as_dash(credits),
            fee = format_credits_as_dash(fee)
        );
        self.confirmation = Some((
            ConfirmationDialog::new("Claim contract fees?", message).blocks_input(true),
            contract_id,
            pot,
            identity,
        ));
        AppAction::None
    }

    fn finish_confirmation(
        &mut self,
        ctx: &egui::Context,
        status: ConfirmationStatus,
    ) -> AppAction {
        let Some((_, contract_id, pot, identity)) = self.confirmation.take() else {
            return AppAction::None;
        };
        if status == ConfirmationStatus::Canceled || self.claim_in_flight.is_some() {
            return AppAction::None;
        }
        self.claim_in_flight = Some(pot);
        self.progress_banner.take_and_clear();
        let handle = MessageBanner::set_global(ctx, "Claiming the fees...", MessageType::Info);
        handle.with_elapsed();
        self.progress_banner = Some(handle);
        self.claim_request(contract_id, pot, &identity)
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
        &self,
        ui: &mut egui::Ui,
        contract: &dash_sdk::platform::DataContract,
        pot: ContractFeePot,
        state: &ContractFeePotState,
        seated_moderators: &SeatedModerators,
    ) -> Option<ContractFeePot> {
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
                date = local_date_time_from_unix_millis(last_claim.time_ms),
                claimant = last_claim.claimant_id.to_string(Encoding::Base58)
            ),
            None => "Never claimed".to_string(),
        });

        let claimants = pot_claimants(pot, contract, seated_moderators);
        let claimant = self
            .selected_identity
            .as_ref()
            .filter(|identity| claimants.includes(&identity.identity.id()));
        let disabled_reason = if matches!(
            claimants,
            crate::model::contract_fee_pots::PotClaimants::Unknown
        ) {
            Some("The moderation team could not be checked. Select Refresh to try again.")
        } else {
            claim_blocked_reason(ClaimGate {
                has_recipients: claimants.any(),
                identity_selected: self.selected_identity.is_some(),
                identity_receives: claimant.is_some(),
                credits: state.credits,
                minimum_credits: minimum_claim_credits(&claimants, seated_moderators),
                claim_in_flight: self.claim_in_flight.is_some(),
            })
        };

        let mut selected_pot = None;
        let button = ui.add_enabled(disabled_reason.is_none(), egui::Button::new("Claim"));
        if let Some(reason) = disabled_reason {
            button.on_disabled_hover_text(
                RichText::new(reason).color(DashColors::text_secondary(dark_mode)),
            );
        } else if button.clicked() {
            selected_pot = Some(pot);
        }
        ui.end_row();
        selected_pot
    }
}

/// An error banner for a local read, kept until the read is retried.
fn load_failure_banner(ctx: &egui::Context, message: &str, error: &TaskError) -> BannerHandle {
    let handle = MessageBanner::set_global(ctx, message, MessageType::Error);
    handle.with_details(error);
    handle.disable_auto_dismiss();
    handle
}

impl Drop for ContractFeePotsScreen {
    fn drop(&mut self) {
        self.progress_banner.take_and_clear();
        self.contracts_load_banner.take_and_clear();
        self.identities_load_banner.take_and_clear();
    }
}

impl ScreenLike for ContractFeePotsScreen {
    fn refresh(&mut self) {
        self.confirmation = None;
        self.reload_local_data();
        self.pending_read.clear();
        self.pots = FeePotsState::NotLoaded;
    }

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.dispatch_id().is_some()
            && !self.pending_read.owns(context)
            && !self.pending_claim.owns(context)
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        if let Some(contract_id) = self.pending_read.settle(context) {
            if self.pots == FeePotsState::Loading(contract_id) {
                self.pots = FeePotsState::Failed(contract_id);
            }
        } else if self.pending_claim.settle(context).is_some() {
            self.progress_banner.take_and_clear();
            self.claim_in_flight = None;
            if matches!(
                error,
                TaskError::ContractFeesClaimedButBalanceNotSaved { .. }
            ) {
                self.pots = FeePotsState::NotLoaded;
            }
        }
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        let own = match &result {
            BackendTaskSuccessResult::ContractFeePots { .. } => {
                self.pending_read.settle(context).is_some()
            }
            BackendTaskSuccessResult::ContractFeesClaimed { .. } => {
                self.pending_claim.settle(context).is_some()
            }
            _ => false,
        };
        if own {
            self.display_task_result(result);
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::ContractFeePots {
                contract_id,
                contract,
                pots,
                seated_moderators,
            } if self.pots == FeePotsState::Loading(contract_id) => {
                self.pots = FeePotsState::Loaded {
                    contract_id,
                    contract,
                    pots,
                    seated_moderators,
                };
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
                    self.apply_identities(identities);
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
            let mut clicked_pot = None;
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

            if self.local_data_failed() {
                ui.colored_label(DashColors::warning_color(dark_mode), LOCAL_DATA_INCOMPLETE);
                if ui.button("Retry Loading").clicked() {
                    self.reload_local_data();
                }
                ui.add_space(10.0);
            }

            ui.heading("1. Select a contract:");
            ui.add_space(6.0);
            let visible: Vec<&QualifiedContract> = self
                .contracts
                .iter()
                .filter(|contract| self.contract_matches_search(contract))
                .collect();
            add_contract_chooser_pre_filtered(
                ui,
                &mut self.contract_search,
                visible.into_iter(),
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

            let Some(contract_id) = self
                .selected_contract
                .as_ref()
                .map(|contract| contract.contract.id())
            else {
                return inner_action;
            };
            if self.pots.contract_id() != Some(contract_id) {
                inner_action |= self.fetch_pots(contract_id);
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            match &self.pots {
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
                FeePotsState::Loaded {
                    contract,
                    pots,
                    seated_moderators,
                    ..
                } => {
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
                            clicked_pot = self
                                .render_pot_row(
                                    ui,
                                    contract,
                                    ContractFeePot::Owner,
                                    &pots.owner,
                                    seated_moderators,
                                )
                                .or(clicked_pot);
                            clicked_pot = self
                                .render_pot_row(
                                    ui,
                                    contract,
                                    ContractFeePot::Moderators,
                                    &pots.moderators,
                                    seated_moderators,
                                )
                                .or(clicked_pot);
                        });
                    ui.add_space(8.0);
                    if ui.button("Refresh").clicked() {
                        inner_action |= self.fetch_pots(contract_id);
                    }
                }
            }
            if let Some(pot) = clicked_pot {
                inner_action |= self.start_claim(ui.ctx(), contract_id, pot);
            }
            inner_action
        });
        let status = self
            .confirmation
            .as_mut()
            .and_then(|(dialog, ..)| dialog.show(ui).inner.dialog_response);
        if let Some(status) = status {
            action |= self.finish_confirmation(ui.ctx(), status);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fee_claim_requires_confirmation_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&ctx);
        screen.selected_identity = Some(crate::context::test_support::bare_user_identity(
            Identifier::random(),
            ctx.network,
        ));
        let action =
            screen.start_claim(ctx.egui_ctx(), Identifier::random(), ContractFeePot::Owner);
        assert!(
            matches!(action, AppAction::None),
            "Claim must ask before dispatch: {action:?}"
        );
        assert!(screen.claim_in_flight.is_none());
    }

    #[test]
    fn fee_claim_tiny_pot_warns_and_confirmation_dispatches_once() {
        for confirmed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = crate::context::test_support::test_app_context(dir.path());
            let mut screen = ContractFeePotsScreen::new(&ctx);
            let contract = test_contract();
            let id = contract.id();
            screen.selected_identity = Some(crate::context::test_support::bare_user_identity(
                contract.owner_id(),
                ctx.network,
            ));
            let mut pots = ContractFeePots::default();
            pots.owner.credits = 1;
            screen.pots = FeePotsState::Loaded {
                contract_id: id,
                contract,
                pots,
                seated_moderators: SeatedModerators::NotSeated,
            };
            assert!(matches!(
                screen.start_claim(ctx.egui_ctx(), id, ContractFeePot::Owner),
                AppAction::None
            ));
            let dialog = &screen.confirmation.as_ref().unwrap().0;
            assert!(dialog.is_input_blocking());
            assert!(
                dialog
                    .message_text()
                    .contains("Estimated fee paid by your identity:")
            );
            assert!(
                dialog
                    .message_text()
                    .contains("Expected net change for your identity: -")
            );
            let status = if confirmed {
                ConfirmationStatus::Confirmed
            } else {
                ConfirmationStatus::Canceled
            };
            let action = screen.finish_confirmation(ctx.egui_ctx(), status);
            assert_eq!(
                matches!(action, AppAction::BackendTaskWithContext { .. }),
                confirmed
            );
            assert!(matches!(
                screen.finish_confirmation(ctx.egui_ctx(), ConfirmationStatus::Confirmed),
                AppAction::None
            ));
        }
    }

    mod allocations {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;
        thread_local! {
            static LARGE: Cell<Option<usize>> = const { Cell::new(None) };
        }
        pub const PAYLOAD_BYTES: usize = 1_000_003;
        struct CountingAllocator;
        #[global_allocator]
        static ALLOCATOR: CountingAllocator = CountingAllocator;
        fn record(size: usize) {
            if size >= PAYLOAD_BYTES {
                let _ = LARGE.try_with(|count| {
                    if let Some(value) = count.get() {
                        count.set(Some(value.saturating_add(1)));
                    }
                });
            }
        }
        // Delegates every allocation to System unchanged; the thread-local counter never allocates.
        unsafe impl GlobalAlloc for CountingAllocator {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                record(layout.size());
                unsafe { System.alloc(layout) }
            }
            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                record(layout.size());
                unsafe { System.alloc_zeroed(layout) }
            }
            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
                record(new_size);
                unsafe { System.realloc(ptr, layout, new_size) }
            }
            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                unsafe { System.dealloc(ptr, layout) }
            }
        }
        pub fn measure(f: impl FnOnce()) -> usize {
            LARGE.with(|count| count.set(Some(0)));
            f();
            LARGE.with(|count| count.replace(None).unwrap())
        }
    }

    #[test]
    fn repainting_fee_pots_does_not_copy_contract_payload() {
        for loaded in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = crate::context::test_support::test_app_context(dir.path());
            let mut screen = ContractFeePotsScreen::new(&ctx);
            screen.apply_local_data(Ok(vec![]), Ok(vec![]));
            let mut contract = (*test_contract()).clone();
            let dash_sdk::platform::DataContract::V1(data) = &mut contract else {
                panic!("v1 contract")
            };
            data.description = Some("x".repeat(allocations::PAYLOAD_BYTES));
            let id = contract.id();
            screen.selected_contract = Some(QualifiedContract {
                contract: contract.clone(),
                alias: Some("Test contract".into()),
            });
            screen.pots = if loaded {
                FeePotsState::Loaded {
                    contract_id: id,
                    contract: Arc::new(contract),
                    pots: ContractFeePots::default(),
                    seated_moderators: SeatedModerators::NotSeated,
                }
            } else {
                FeePotsState::Loading(id)
            };
            let mut harness = egui_kittest::Harness::builder()
                .with_context_setup(|ctx| ctx.set_fonts(crate::bundled::fonts().unwrap()))
                .with_size(egui::vec2(1280.0, 900.0))
                .build_ui_state(
                    |ui, screen: &mut ContractFeePotsScreen| {
                        screen.ui(ui);
                    },
                    screen,
                );
            harness.run_steps(3);
            let copies = allocations::measure(|| harness.step());
            assert_eq!(
                copies, 0,
                "a warmed repaint must not allocate a contract-sized payload; loaded={loaded}"
            );
        }
    }

    #[test]
    fn unrelated_errors_remain_visible_on_fee_pots() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let screen = ContractFeePotsScreen::new(&ctx);
        assert!(!screen.should_suppress_backend_task_error(
            &BackendTaskContext::Other,
            &TaskError::DataContractNotFound
        ));
    }

    #[test]
    fn fee_claim_selection_follows_reloaded_identity_and_removal() {
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&ctx);
        let mut identity =
            crate::context::test_support::bare_user_identity(Identifier::random(), ctx.network);
        identity.alias = Some("Old signer".to_string());
        screen.selected_identity_str = identity.identity.id().to_string(Encoding::Base58);
        screen.selected_identity = Some(identity.clone());
        identity.alias = Some("Reloaded signer".to_string());
        identity.identity.set_revision(2);
        screen.apply_local_data(Ok(vec![]), Ok(vec![identity]));
        let selected = screen.selected_identity.as_ref().unwrap();
        assert_eq!(selected.identity.revision(), 2);
        assert_eq!(selected.alias.as_deref(), Some("Reloaded signer"));
        screen.apply_local_data(Ok(vec![]), Ok(vec![]));
        assert!(screen.selected_identity.is_none());
        assert!(screen.selected_identity_str.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fee_claim_selection_follows_post_claim_reload() {
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let mut screen = ContractFeePotsScreen::new(&ctx);
        let mut identity =
            crate::context::test_support::bare_user_identity(Identifier::random(), ctx.network);
        screen.selected_identity_str = identity.identity.id().to_string(Encoding::Base58);
        screen.selected_identity = Some(identity.clone());
        identity.identity.set_revision(2);
        ctx.insert_local_qualified_identity_observed(
            &mut identity,
            &None,
            crate::model::identity_balance::BalanceProvenance::ObservedAt(1),
        )
        .unwrap();
        screen.display_task_result(BackendTaskSuccessResult::ContractFeesClaimed {
            contract_id: Identifier::random(),
            pot: ContractFeePot::Owner,
            claimant_balance: Some(0),
            remaining_credits: 0,
        });
        assert_eq!(
            screen
                .selected_identity
                .as_ref()
                .unwrap()
                .identity
                .revision(),
            2
        );
    }

    #[test]
    fn closing_fee_pots_clears_owned_banners_and_reports_paid_claim() {
        use crate::ui::components::message_banner::global_banner_texts;
        use crate::ui::task_feedback::{Feedback, ResultDelivery};
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&ctx);
        assert!(!global_banner_texts(ctx.egui_ctx()).is_empty());
        let handle =
            MessageBanner::set_global(ctx.egui_ctx(), "Claiming the fees...", MessageType::Info);
        handle.with_elapsed();
        screen.progress_banner = Some(handle);
        drop(screen);
        assert!(
            global_banner_texts(ctx.egui_ctx()).is_empty(),
            "closed screen must clear its retry and progress banners"
        );
        let contexts = std::collections::BTreeMap::new();
        crate::ui::task_feedback::on_success(
            &Feedback {
                ctx: ctx.egui_ctx(),
                contexts: &contexts,
                delivery: ResultDelivery {
                    sender_in_view: false,
                    sender_gone: true,
                },
            },
            &BackendTaskContext::Other,
            &BackendTaskSuccessResult::ContractFeesClaimed {
                contract_id: Identifier::random(),
                pot: ContractFeePot::Owner,
                claimant_balance: Some(1000),
                remaining_credits: 0,
            },
        );
        let messages = global_banner_texts(ctx.egui_ctx());
        assert_eq!(messages.len(), 1);
        assert!(messages[0].starts_with("The fees were paid out."));
    }

    fn test_contract() -> Arc<dash_sdk::platform::DataContract> {
        Arc::new(
            dash_sdk::dpp::system_data_contracts::load_system_data_contract(
                dash_sdk::dpp::system_data_contracts::SystemDataContract::DPNS,
                dash_sdk::dpp::version::PlatformVersion::latest(),
            )
            .unwrap(),
        )
    }

    fn dispatched_context(action: AppAction) -> BackendTaskContext {
        let AppAction::BackendTaskWithContext { context, .. } = action else {
            panic!("expected a correlated request");
        };
        context
    }

    fn deliver_error(screen: &mut ContractFeePotsScreen, context: &BackendTaskContext) {
        let error = TaskError::DataContractNotFound;
        let suppress = screen.should_suppress_backend_task_error(context, &error);
        screen.display_backend_task_error(context, &error);
        if !suppress {
            screen.display_message(&error.to_string(), MessageType::Error);
        }
    }

    #[test]
    fn late_fee_read_error_preserves_newer_read() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&ctx);
        let old = dispatched_context(screen.fetch_pots(Identifier::random()));
        let current_id = Identifier::random();
        let current = dispatched_context(screen.fetch_pots(current_id));
        deliver_error(&mut screen, &old);
        assert_eq!(screen.pots, FeePotsState::Loading(current_id));
        screen.display_backend_task_result(
            &current,
            BackendTaskSuccessResult::ContractFeePots {
                contract_id: current_id,
                pots: ContractFeePots::default(),
                contract: test_contract(),
                seated_moderators: SeatedModerators::NotSeated,
            },
        );
        assert!(
            matches!(screen.pots, FeePotsState::Loaded { contract_id, .. } if contract_id == current_id)
        );
    }

    #[test]
    fn late_fee_read_error_does_not_unlock_claim() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&ctx);
        screen.apply_local_data(Ok(vec![]), Ok(vec![]));
        let old = dispatched_context(screen.fetch_pots(Identifier::random()));
        let current_id = Identifier::random();
        let current = dispatched_context(screen.fetch_pots(current_id));
        screen.display_backend_task_result(
            &current,
            BackendTaskSuccessResult::ContractFeePots {
                contract_id: current_id,
                pots: ContractFeePots::default(),
                contract: test_contract(),
                seated_moderators: SeatedModerators::NotSeated,
            },
        );
        let identity =
            crate::context::test_support::bare_user_identity(Identifier::random(), ctx.network);
        let claim =
            dispatched_context(screen.claim_request(current_id, ContractFeePot::Owner, &identity));
        screen.claim_in_flight = Some(ContractFeePot::Owner);
        let handle =
            MessageBanner::set_global(ctx.egui_ctx(), "Claiming the fees...", MessageType::Info);
        handle.with_elapsed();
        screen.progress_banner = Some(handle);
        deliver_error(&mut screen, &old);
        assert_eq!(screen.claim_in_flight, Some(ContractFeePot::Owner));
        assert!(screen.progress_banner.is_some());
        deliver_error(&mut screen, &claim);
        assert_eq!(screen.claim_in_flight, None);
        assert!(screen.progress_banner.is_none());
    }

    #[test]
    fn fee_pots_remain_available_without_a_known_protocol_version() {
        use egui_kittest::kittest::Queryable;

        for version in [None, Some(dash_sdk::dpp::version::LATEST_VERSION + 1)] {
            let dir = tempfile::tempdir().expect("tempdir");
            let ctx = crate::context::test_support::test_app_context(dir.path());
            if let Some(version) = version {
                crate::context::test_support::prove_protocol_version(&ctx, version);
                assert!(ctx.protocol_version_info().newer_protocol_available);
            } else {
                assert_eq!(ctx.protocol_version_info().verified_version, None);
            }
            let mut screen = ContractFeePotsScreen::new(&ctx);
            screen.apply_local_data(Ok(vec![]), Ok(vec![]));
            let mut harness = egui_kittest::Harness::builder()
                .with_context_setup(|ctx| {
                    ctx.set_fonts(crate::bundled::fonts().expect("bundled fonts load"));
                })
                .with_size(egui::vec2(1280.0, 900.0))
                .build_ui_state(
                    |ui, screen: &mut ContractFeePotsScreen| {
                        screen.ui(ui);
                    },
                    screen,
                );
            harness.run();
            assert!(
                harness.query_by_label("This network does not support contract fees yet. Connect to a network that supports them to see fee pots.").is_none(),
                "protocol {version:?}: the network must not be reported as lacking fee pots"
            );
            assert!(harness.query_by_label("1. Select a contract:").is_some());
            assert!(
                harness
                    .query_by_label("2. Select the identity that claims:")
                    .is_some()
            );
        }
    }

    /// A storage failure must not pass for an empty device: the screen says
    /// what failed, keeps a retry on screen, and a successful retry settles it.
    #[test]
    fn a_storage_failure_is_reported_and_can_be_retried() {
        use crate::ui::components::message_banner::global_banner_texts;

        let dir = tempfile::tempdir().expect("tempdir");
        // The test context has no wallet backend, so the contract read fails.
        let app_context = crate::context::test_support::test_app_context(dir.path());
        let ctx = app_context.egui_ctx().clone();

        let mut screen = ContractFeePotsScreen::new(&app_context);
        assert!(
            global_banner_texts(&ctx)
                .iter()
                .any(|text| text.starts_with("Your saved contracts could not be loaded")),
            "got: {:?}",
            global_banner_texts(&ctx)
        );
        assert!(screen.local_data_failed());

        screen.apply_local_data(Ok(vec![]), Ok(vec![]));
        assert!(!screen.local_data_failed());
        assert!(
            global_banner_texts(&ctx).is_empty(),
            "got: {:?}",
            global_banner_texts(&ctx)
        );
    }

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
                contract: test_contract(),
                seated_moderators: SeatedModerators::NotSeated,
            }
            .contract_id(),
            Some(id)
        );
    }

    /// The pots read and a claim carry a dispatch context, so their results
    /// and errors reach this screen even if another screen covers it.
    #[test]
    fn fee_pot_requests_come_back_to_this_screen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&app_context);
        let contract_id = Identifier::random();

        let read = screen.fetch_pots(contract_id);
        assert!(
            matches!(
                &read,
                AppAction::BackendTaskWithContext {
                    task: BackendTask::ContractTask(task),
                    ..
                } if matches!(**task, ContractTask::FetchContractFeePots(id) if id == contract_id)
            ),
            "got {read:?}"
        );

        let identity = crate::context::test_support::bare_user_identity(
            Identifier::random(),
            dash_sdk::dpp::dashcore::Network::Testnet,
        );
        let claim = screen.claim_request(contract_id, ContractFeePot::Owner, &identity);
        assert!(
            matches!(
                &claim,
                AppAction::BackendTaskWithContext {
                    task: BackendTask::ContractTask(task),
                    ..
                } if matches!(**task, ContractTask::ClaimContractFees { .. })
            ),
            "got {claim:?}"
        );
    }

    /// A claim paid out whose balance could not be saved re-reads the pots,
    /// so the emptied pot shows and no retry is invited.
    #[test]
    fn a_paid_out_claim_that_was_not_saved_rereads_the_pots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = ContractFeePotsScreen::new(&app_context);
        let contract_id = Identifier::random();
        screen.pots = FeePotsState::Loaded {
            contract_id,
            pots: ContractFeePots::default(),
            contract: test_contract(),
            seated_moderators: SeatedModerators::NotSeated,
        };
        screen.claim_in_flight = Some(ContractFeePot::Owner);
        let identity = crate::context::test_support::bare_user_identity(
            Identifier::random(),
            dash_sdk::dpp::dashcore::Network::Testnet,
        );
        let AppAction::BackendTaskWithContext { context, .. } =
            screen.claim_request(contract_id, ContractFeePot::Owner, &identity)
        else {
            panic!("the claim goes out with a context");
        };

        screen.display_backend_task_error(
            &context,
            &TaskError::ContractFeesClaimedButBalanceNotSaved {
                source: Box::new(TaskError::IdentityNotFoundLocally),
            },
        );

        assert_eq!(screen.pots, FeePotsState::NotLoaded);
    }

    #[test]
    fn equal_split_remainder_cannot_enable_claim() {
        assert!(
            claim_blocked_reason(ClaimGate {
                credits: 2,
                minimum_credits: 3,
                ..claimable()
            })
            .is_some()
        );
        assert_eq!(
            claim_blocked_reason(ClaimGate {
                credits: 3,
                minimum_credits: 3,
                ..claimable()
            }),
            None
        );
        assert_eq!(
            claim_blocked_reason(ClaimGate {
                credits: 1,
                minimum_credits: 1,
                ..claimable()
            }),
            None
        );
    }

    fn claimable() -> ClaimGate {
        ClaimGate {
            has_recipients: true,
            identity_selected: true,
            identity_receives: true,
            credits: 10,
            minimum_credits: 1,
            claim_in_flight: false,
        }
    }

    #[test]
    fn a_recipient_can_claim_a_filled_pot() {
        assert_eq!(claim_blocked_reason(claimable()), None);
    }

    /// Each refusal Platform would charge for is blocked up front.
    #[test]
    fn claims_platform_would_refuse_are_blocked() {
        for gate in [
            ClaimGate {
                has_recipients: false,
                ..claimable()
            },
            ClaimGate {
                identity_selected: false,
                identity_receives: false,
                ..claimable()
            },
            ClaimGate {
                identity_receives: false,
                ..claimable()
            },
            ClaimGate {
                credits: 0,
                ..claimable()
            },
            ClaimGate {
                claim_in_flight: true,
                ..claimable()
            },
        ] {
            assert!(claim_blocked_reason(gate).is_some(), "{gate:?}");
        }
    }
}
