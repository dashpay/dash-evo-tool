use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::contracts_documents::document_query_screen::DocumentQueryScreen;
use crate::ui::dpns::dpns_contested_names_screen::DPNSScreen;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identities::keys::keys_screen::KeysScreen;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use crate::ui::identities::transfer_screen::TransferScreen;
use crate::ui::identities::withdraw_screen::WithdrawalScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::tools::proof_log_screen::ProofLogScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::wallets::import_wallet_screen::ImportWalletScreen;
use crate::ui::wallets::wallets_screen::WalletsBalancesScreen;
use contracts_documents::add_contracts_screen::AddContractsScreen;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use dpns::dpns_contested_names_screen::DPNSSubscreen;
use egui::Context;
use identities::add_existing_identity_screen::AddExistingIdentityScreen;
use identities::add_new_identity_screen::AddNewIdentityScreen;
use identities::identities_screen::IdentitiesScreen;
use identities::register_dpns_name_screen::RegisterDpnsNameScreen;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use tokens::tokens_screen::{TokensScreen, TokensSubscreen};
use tools::transition_visualizer_screen::TransitionVisualizerScreen;
use wallets::add_new_wallet_screen::AddNewWalletScreen;

pub mod components;
pub mod contracts_documents;
pub mod dpns;
pub(crate) mod identities;
pub mod network_chooser_screen;
pub mod tokens;
pub mod tools;
pub(crate) mod wallets;

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum RootScreenType {
    RootScreenIdentities,
    RootScreenDPNSActiveContests,
    RootScreenDPNSPastContests,
    RootScreenDPNSOwnedNames,
    RootScreenDPNSScheduledVotes,
    RootScreenDocumentQuery,
    RootScreenWalletsBalances,
    RootScreenToolsProofLogScreen,
    RootScreenToolsTransitionVisualizerScreen,
    RootScreenNetworkChooser,
    RootScreenToolsProofVisualizerScreen,
    RootScreenMyTokenBalances,
    RootScreenTokenSearch,
}

impl RootScreenType {
    /// Convert `RootScreenType` to an integer
    pub fn to_int(self) -> u32 {
        match self {
            RootScreenType::RootScreenIdentities => 0,
            RootScreenType::RootScreenDPNSActiveContests => 1,
            RootScreenType::RootScreenDPNSPastContests => 2,
            RootScreenType::RootScreenDPNSOwnedNames => 3,
            RootScreenType::RootScreenDocumentQuery => 4,
            RootScreenType::RootScreenWalletsBalances => 5,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen => 6,
            RootScreenType::RootScreenNetworkChooser => 7,
            // 8 used to be the Withdrawals Statuses screen
            RootScreenType::RootScreenToolsProofLogScreen => 9,
            RootScreenType::RootScreenDPNSScheduledVotes => 10,
            RootScreenType::RootScreenToolsProofVisualizerScreen => 11,
            RootScreenType::RootScreenMyTokenBalances => 12,
            RootScreenType::RootScreenTokenSearch => 13,
        }
    }

    /// Convert an integer to a `RootScreenType`
    pub fn from_int(value: u32) -> Option<Self> {
        match value {
            0 => Some(RootScreenType::RootScreenIdentities),
            1 => Some(RootScreenType::RootScreenDPNSActiveContests),
            2 => Some(RootScreenType::RootScreenDPNSPastContests),
            3 => Some(RootScreenType::RootScreenDPNSOwnedNames),
            4 => Some(RootScreenType::RootScreenDocumentQuery),
            5 => Some(RootScreenType::RootScreenWalletsBalances),
            6 => Some(RootScreenType::RootScreenToolsTransitionVisualizerScreen),
            7 => Some(RootScreenType::RootScreenNetworkChooser),
            // 8 used to be the Withdrawals Statuses screen
            9 => Some(RootScreenType::RootScreenToolsProofLogScreen),
            10 => Some(RootScreenType::RootScreenDPNSScheduledVotes),
            11 => Some(RootScreenType::RootScreenToolsProofVisualizerScreen),
            12 => Some(RootScreenType::RootScreenMyTokenBalances),
            13 => Some(RootScreenType::RootScreenTokenSearch),
            _ => None,
        }
    }
}

impl From<RootScreenType> for ScreenType {
    fn from(value: RootScreenType) -> Self {
        match value {
            RootScreenType::RootScreenIdentities => ScreenType::Identities,
            RootScreenType::RootScreenDPNSActiveContests => ScreenType::DPNSActiveContests,
            RootScreenType::RootScreenDPNSPastContests => ScreenType::DPNSPastContests,
            RootScreenType::RootScreenDPNSOwnedNames => ScreenType::DPNSMyUsernames,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen => {
                ScreenType::TransitionVisualizer
            }
            RootScreenType::RootScreenDocumentQuery => ScreenType::DocumentQueryScreen,
            RootScreenType::RootScreenNetworkChooser => ScreenType::NetworkChooser,
            RootScreenType::RootScreenWalletsBalances => ScreenType::WalletsBalances,
            RootScreenType::RootScreenToolsProofLogScreen => ScreenType::ProofLog,
            RootScreenType::RootScreenDPNSScheduledVotes => ScreenType::ScheduledVotes,
            RootScreenType::RootScreenToolsProofVisualizerScreen => ScreenType::ProofVisualizer,
            RootScreenType::RootScreenMyTokenBalances => ScreenType::TokenBalances,
            RootScreenType::RootScreenTokenSearch => ScreenType::TokenSearch,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub enum ScreenType {
    #[default]
    Identities,
    DPNSActiveContests,
    DPNSPastContests,
    DPNSMyUsernames,
    AddNewIdentity,
    WalletsBalances,
    ImportWallet,
    AddNewWallet,
    AddExistingIdentity,
    TransitionVisualizer,
    WithdrawalScreen(QualifiedIdentity),
    TransferScreen(QualifiedIdentity),
    AddKeyScreen(QualifiedIdentity),
    KeyInfo(
        QualifiedIdentity,
        IdentityPublicKey,
        Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
    ),
    Keys(Identity),
    DocumentQueryScreen,
    NetworkChooser,
    RegisterDpnsName,
    ProofLog,
    TopUpIdentity(QualifiedIdentity),
    ScheduledVotes,
    AddContracts,
    ProofVisualizer,
    TokenBalances,
    TokenSearch,
}

impl ScreenType {
    pub fn create_screen(&self, app_context: &Arc<AppContext>) -> Screen {
        match self {
            ScreenType::Identities => Screen::IdentitiesScreen(IdentitiesScreen::new(app_context)),
            ScreenType::DPNSActiveContests => {
                Screen::DPNSScreen(DPNSScreen::new(app_context, DPNSSubscreen::Active))
            }
            ScreenType::DPNSPastContests => {
                Screen::DPNSScreen(DPNSScreen::new(app_context, DPNSSubscreen::Past))
            }
            ScreenType::DPNSMyUsernames => {
                Screen::DPNSScreen(DPNSScreen::new(app_context, DPNSSubscreen::Owned))
            }
            ScreenType::AddNewIdentity => {
                Screen::AddNewIdentityScreen(AddNewIdentityScreen::new(app_context))
            }
            ScreenType::TopUpIdentity(identity) => {
                Screen::TopUpIdentityScreen(TopUpIdentityScreen::new(identity.clone(), app_context))
            }
            ScreenType::AddExistingIdentity => {
                Screen::AddExistingIdentityScreen(AddExistingIdentityScreen::new(app_context))
            }
            ScreenType::Keys(identity) => {
                Screen::KeysScreen(KeysScreen::new(identity.clone(), app_context))
            }
            ScreenType::KeyInfo(identity, key, private_key) => {
                Screen::KeyInfoScreen(KeyInfoScreen::new(
                    identity.clone(),
                    key.clone(),
                    private_key.clone(),
                    app_context,
                ))
            }
            ScreenType::RegisterDpnsName => {
                Screen::RegisterDpnsNameScreen(RegisterDpnsNameScreen::new(app_context))
            }
            ScreenType::TransitionVisualizer => {
                Screen::TransitionVisualizerScreen(TransitionVisualizerScreen::new(app_context))
            }
            ScreenType::WithdrawalScreen(identity) => {
                Screen::WithdrawalScreen(WithdrawalScreen::new(identity.clone(), app_context))
            }
            ScreenType::TransferScreen(identity) => {
                Screen::TransferScreen(TransferScreen::new(identity.clone(), app_context))
            }
            ScreenType::NetworkChooser => {
                unreachable!()
            }
            ScreenType::AddKeyScreen(identity) => {
                Screen::AddKeyScreen(AddKeyScreen::new(identity.clone(), app_context))
            }
            ScreenType::DocumentQueryScreen => {
                Screen::DocumentQueryScreen(DocumentQueryScreen::new(app_context))
            }
            ScreenType::AddNewWallet => {
                Screen::AddNewWalletScreen(AddNewWalletScreen::new(app_context))
            }
            ScreenType::WalletsBalances => {
                Screen::WalletsBalancesScreen(WalletsBalancesScreen::new(app_context))
            }
            ScreenType::ImportWallet => {
                Screen::ImportWalletScreen(ImportWalletScreen::new(app_context))
            }
            ScreenType::ProofLog => Screen::ProofLogScreen(ProofLogScreen::new(app_context)),
            ScreenType::ScheduledVotes => {
                Screen::DPNSScreen(DPNSScreen::new(app_context, DPNSSubscreen::ScheduledVotes))
            }
            ScreenType::AddContracts => {
                Screen::AddContractsScreen(AddContractsScreen::new(app_context))
            }
            ScreenType::ProofVisualizer => {
                Screen::ProofVisualizerScreen(ProofVisualizerScreen::new(app_context))
            }
            ScreenType::TokenBalances => {
                Screen::TokensScreen(TokensScreen::new(app_context, TokensSubscreen::MyTokens))
            }
            ScreenType::TokenSearch => Screen::TokensScreen(TokensScreen::new(
                app_context,
                TokensSubscreen::SearchTokens,
            )),
        }
    }
}

pub enum Screen {
    IdentitiesScreen(IdentitiesScreen),
    DPNSScreen(DPNSScreen),
    DocumentQueryScreen(DocumentQueryScreen),
    AddNewWalletScreen(AddNewWalletScreen),
    ImportWalletScreen(ImportWalletScreen),
    AddNewIdentityScreen(AddNewIdentityScreen),
    AddExistingIdentityScreen(AddExistingIdentityScreen),
    KeyInfoScreen(KeyInfoScreen),
    KeysScreen(KeysScreen),
    RegisterDpnsNameScreen(RegisterDpnsNameScreen),
    WithdrawalScreen(WithdrawalScreen),
    TopUpIdentityScreen(TopUpIdentityScreen),
    TransferScreen(TransferScreen),
    AddKeyScreen(AddKeyScreen),
    ProofLogScreen(ProofLogScreen),
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    NetworkChooserScreen(NetworkChooserScreen),
    WalletsBalancesScreen(WalletsBalancesScreen),
    AddContractsScreen(AddContractsScreen),
    ProofVisualizerScreen(ProofVisualizerScreen),
    TokensScreen(TokensScreen),
}

impl Screen {
    pub fn change_context(&mut self, app_context: Arc<AppContext>) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.app_context = app_context,
            Screen::DPNSScreen(screen) => screen.app_context = app_context,
            Screen::AddExistingIdentityScreen(screen) => screen.app_context = app_context,
            Screen::KeyInfoScreen(screen) => screen.app_context = app_context,
            Screen::KeysScreen(screen) => screen.app_context = app_context,
            Screen::WithdrawalScreen(screen) => screen.app_context = app_context,
            Screen::TransitionVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::NetworkChooserScreen(screen) => screen.current_network = app_context.network,
            Screen::AddKeyScreen(screen) => screen.app_context = app_context,
            Screen::DocumentQueryScreen(screen) => screen.app_context = app_context,
            Screen::AddNewIdentityScreen(screen) => screen.app_context = app_context,
            Screen::RegisterDpnsNameScreen(screen) => screen.app_context = app_context,
            Screen::AddNewWalletScreen(screen) => screen.app_context = app_context,
            Screen::TransferScreen(screen) => screen.app_context = app_context,
            Screen::TopUpIdentityScreen(screen) => screen.app_context = app_context,
            Screen::WalletsBalancesScreen(screen) => screen.app_context = app_context,
            Screen::ImportWalletScreen(screen) => screen.app_context = app_context,
            Screen::ProofLogScreen(screen) => screen.app_context = app_context,
            Screen::AddContractsScreen(screen) => screen.app_context = app_context,
            Screen::ProofVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::TokensScreen(screen) => screen.app_context = app_context,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MessageType {
    Success,
    Info,
    Error,
}

pub trait ScreenLike {
    fn refresh(&mut self) {}
    fn refresh_on_arrival(&mut self) {
        self.refresh()
    }
    fn ui(&mut self, ctx: &Context) -> AppAction;
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}
    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {
        self.display_message("Success", MessageType::Success)
    }

    fn pop_on_success(&mut self) {}
}

// Implement Debug for Screen using the ScreenType
impl fmt::Debug for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.screen_type())
    }
}

// Implement PartialEq for Screen by comparing the ScreenType
impl PartialEq for Screen {
    fn eq(&self, other: &Self) -> bool {
        self.screen_type() == other.screen_type()
    }
}

impl Screen {
    pub fn screen_type(&self) -> ScreenType {
        match self {
            Screen::AddExistingIdentityScreen(_) => ScreenType::AddExistingIdentity,
            Screen::KeysScreen(screen) => ScreenType::Keys(screen.identity.clone()),
            Screen::KeyInfoScreen(screen) => ScreenType::KeyInfo(
                screen.identity.clone(),
                screen.key.clone(),
                screen.private_key_data.clone(),
            ),
            Screen::IdentitiesScreen(_) => ScreenType::Identities,
            Screen::DPNSScreen(DPNSScreen {
                dpns_subscreen: DPNSSubscreen::Active,
                ..
            }) => ScreenType::DPNSActiveContests,
            Screen::DPNSScreen(DPNSScreen {
                dpns_subscreen: DPNSSubscreen::Past,
                ..
            }) => ScreenType::DPNSPastContests,
            Screen::DPNSScreen(DPNSScreen {
                dpns_subscreen: DPNSSubscreen::Owned,
                ..
            }) => ScreenType::DPNSMyUsernames,
            Screen::DPNSScreen(DPNSScreen {
                dpns_subscreen: DPNSSubscreen::ScheduledVotes,
                ..
            }) => ScreenType::ScheduledVotes,
            Screen::TransitionVisualizerScreen(_) => ScreenType::TransitionVisualizer,
            Screen::WithdrawalScreen(screen) => {
                ScreenType::WithdrawalScreen(screen.identity.clone())
            }
            Screen::NetworkChooserScreen(_) => ScreenType::NetworkChooser,
            Screen::AddKeyScreen(screen) => ScreenType::AddKeyScreen(screen.identity.clone()),
            Screen::DocumentQueryScreen(_) => ScreenType::DocumentQueryScreen,
            Screen::AddNewIdentityScreen(_) => ScreenType::AddExistingIdentity,
            Screen::TopUpIdentityScreen(screen) => {
                ScreenType::TopUpIdentity(screen.identity.clone())
            }
            Screen::RegisterDpnsNameScreen(_) => ScreenType::RegisterDpnsName,
            Screen::AddNewWalletScreen(_) => ScreenType::AddNewWallet,
            Screen::TransferScreen(screen) => ScreenType::TransferScreen(screen.identity.clone()),
            Screen::WalletsBalancesScreen(_) => ScreenType::WalletsBalances,
            Screen::ImportWalletScreen(_) => ScreenType::ImportWallet,
            Screen::ProofLogScreen(_) => ScreenType::ProofLog,
            Screen::AddContractsScreen(_) => ScreenType::AddContracts,
            Screen::ProofVisualizerScreen(_) => ScreenType::ProofVisualizer,
            Screen::TokensScreen(TokensScreen {
                tokens_subscreen: TokensSubscreen::MyTokens,
                ..
            }) => ScreenType::TokenBalances,
            Screen::TokensScreen(TokensScreen {
                tokens_subscreen: TokensSubscreen::SearchTokens,
                ..
            }) => ScreenType::TokenSearch,
        }
    }
}

impl ScreenLike for Screen {
    fn refresh(&mut self) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.refresh(),
            Screen::DPNSScreen(screen) => screen.refresh(),
            Screen::DocumentQueryScreen(screen) => screen.refresh(),
            Screen::AddNewWalletScreen(screen) => screen.refresh(),
            Screen::ImportWalletScreen(screen) => screen.refresh(),
            Screen::AddNewIdentityScreen(screen) => screen.refresh(),
            Screen::TopUpIdentityScreen(screen) => screen.refresh(),
            Screen::AddExistingIdentityScreen(screen) => screen.refresh(),
            Screen::KeyInfoScreen(screen) => screen.refresh(),
            Screen::KeysScreen(screen) => screen.refresh(),
            Screen::RegisterDpnsNameScreen(screen) => screen.refresh(),
            Screen::WithdrawalScreen(screen) => screen.refresh(),
            Screen::TransferScreen(screen) => screen.refresh(),
            Screen::AddKeyScreen(screen) => screen.refresh(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh(),
            Screen::NetworkChooserScreen(screen) => screen.refresh(),
            Screen::WalletsBalancesScreen(screen) => screen.refresh(),
            Screen::ProofLogScreen(screen) => screen.refresh(),
            Screen::AddContractsScreen(screen) => screen.refresh(),
            Screen::ProofVisualizerScreen(screen) => screen.refresh(),
            Screen::TokensScreen(screen) => screen.refresh(),
        }
    }

    fn refresh_on_arrival(&mut self) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.refresh_on_arrival(),
            Screen::DPNSScreen(screen) => screen.refresh_on_arrival(),
            Screen::DocumentQueryScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddNewWalletScreen(screen) => screen.refresh_on_arrival(),
            Screen::ImportWalletScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddNewIdentityScreen(screen) => screen.refresh_on_arrival(),
            Screen::TopUpIdentityScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddExistingIdentityScreen(screen) => screen.refresh_on_arrival(),
            Screen::KeyInfoScreen(screen) => screen.refresh_on_arrival(),
            Screen::KeysScreen(screen) => screen.refresh_on_arrival(),
            Screen::RegisterDpnsNameScreen(screen) => screen.refresh_on_arrival(),
            Screen::WithdrawalScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransferScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddKeyScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::NetworkChooserScreen(screen) => screen.refresh_on_arrival(),
            Screen::WalletsBalancesScreen(screen) => screen.refresh_on_arrival(),
            Screen::ProofLogScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddContractsScreen(screen) => screen.refresh_on_arrival(),
            Screen::ProofVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::TokensScreen(screen) => screen.refresh_on_arrival(),
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        match self {
            Screen::IdentitiesScreen(screen) => screen.ui(ctx),
            Screen::DPNSScreen(screen) => screen.ui(ctx),
            Screen::DocumentQueryScreen(screen) => screen.ui(ctx),
            Screen::AddNewWalletScreen(screen) => screen.ui(ctx),
            Screen::ImportWalletScreen(screen) => screen.ui(ctx),
            Screen::AddNewIdentityScreen(screen) => screen.ui(ctx),
            Screen::TopUpIdentityScreen(screen) => screen.ui(ctx),
            Screen::AddExistingIdentityScreen(screen) => screen.ui(ctx),
            Screen::KeyInfoScreen(screen) => screen.ui(ctx),
            Screen::KeysScreen(screen) => screen.ui(ctx),
            Screen::RegisterDpnsNameScreen(screen) => screen.ui(ctx),
            Screen::WithdrawalScreen(screen) => screen.ui(ctx),
            Screen::TransferScreen(screen) => screen.ui(ctx),
            Screen::AddKeyScreen(screen) => screen.ui(ctx),
            Screen::TransitionVisualizerScreen(screen) => screen.ui(ctx),
            Screen::NetworkChooserScreen(screen) => screen.ui(ctx),
            Screen::WalletsBalancesScreen(screen) => screen.ui(ctx),
            Screen::ProofLogScreen(screen) => screen.ui(ctx),
            Screen::AddContractsScreen(screen) => screen.ui(ctx),
            Screen::ProofVisualizerScreen(screen) => screen.ui(ctx),
            Screen::TokensScreen(screen) => screen.ui(ctx),
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.display_message(message, message_type),
            Screen::DPNSScreen(screen) => screen.display_message(message, message_type),
            Screen::DocumentQueryScreen(screen) => screen.display_message(message, message_type),
            Screen::AddNewWalletScreen(screen) => screen.display_message(message, message_type),
            Screen::ImportWalletScreen(screen) => screen.display_message(message, message_type),
            Screen::AddNewIdentityScreen(screen) => screen.display_message(message, message_type),
            Screen::TopUpIdentityScreen(screen) => screen.display_message(message, message_type),
            Screen::AddExistingIdentityScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::KeyInfoScreen(screen) => screen.display_message(message, message_type),
            Screen::KeysScreen(screen) => screen.display_message(message, message_type),
            Screen::RegisterDpnsNameScreen(screen) => screen.display_message(message, message_type),
            Screen::WithdrawalScreen(screen) => screen.display_message(message, message_type),
            Screen::TransferScreen(screen) => screen.display_message(message, message_type),
            Screen::AddKeyScreen(screen) => screen.display_message(message, message_type),
            Screen::TransitionVisualizerScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::NetworkChooserScreen(screen) => screen.display_message(message, message_type),
            Screen::WalletsBalancesScreen(screen) => screen.display_message(message, message_type),
            Screen::ProofLogScreen(screen) => screen.display_message(message, message_type),
            Screen::AddContractsScreen(screen) => screen.display_message(message, message_type),
            Screen::ProofVisualizerScreen(screen) => screen.display_message(message, message_type),
            Screen::TokensScreen(screen) => screen.display_message(message, message_type),
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match self {
            Screen::IdentitiesScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::DPNSScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::DocumentQueryScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::AddNewWalletScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::ImportWalletScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::AddNewIdentityScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::TopUpIdentityScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::AddExistingIdentityScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::KeyInfoScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::KeysScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::RegisterDpnsNameScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::WithdrawalScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::TransferScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::AddKeyScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::TransitionVisualizerScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::NetworkChooserScreen(screen) => {
                screen.display_task_result(backend_task_success_result.clone())
            }
            Screen::WalletsBalancesScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ProofLogScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddContractsScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ProofVisualizerScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::TokensScreen(screen) => screen.display_task_result(backend_task_success_result),
        }
    }

    fn pop_on_success(&mut self) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.pop_on_success(),
            Screen::DPNSScreen(screen) => screen.pop_on_success(),
            Screen::DocumentQueryScreen(screen) => screen.pop_on_success(),
            Screen::AddNewWalletScreen(screen) => screen.pop_on_success(),
            Screen::ImportWalletScreen(screen) => screen.pop_on_success(),
            Screen::AddNewIdentityScreen(screen) => screen.pop_on_success(),
            Screen::TopUpIdentityScreen(screen) => screen.pop_on_success(),
            Screen::AddExistingIdentityScreen(screen) => screen.pop_on_success(),
            Screen::KeyInfoScreen(screen) => screen.pop_on_success(),
            Screen::KeysScreen(screen) => screen.pop_on_success(),
            Screen::RegisterDpnsNameScreen(screen) => screen.pop_on_success(),
            Screen::WithdrawalScreen(screen) => screen.pop_on_success(),
            Screen::TransferScreen(screen) => screen.pop_on_success(),
            Screen::AddKeyScreen(screen) => screen.pop_on_success(),
            Screen::TransitionVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::NetworkChooserScreen(screen) => screen.pop_on_success(),
            Screen::WalletsBalancesScreen(screen) => screen.pop_on_success(),
            Screen::ProofLogScreen(screen) => screen.pop_on_success(),
            Screen::AddContractsScreen(screen) => screen.pop_on_success(),
            Screen::ProofVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::TokensScreen(screen) => screen.pop_on_success(),
        }
    }
}
