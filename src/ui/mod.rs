use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen;
use crate::ui::contracts_documents::create_document_screen::CreateDocumentScreen;
use crate::ui::dpns::dpns_contested_names_screen::DPNSScreen;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identities::keys::keys_screen::KeysScreen;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use crate::ui::identities::transfer_screen::TransferScreen;
use crate::ui::identities::withdraw_screen::WithdrawalScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::tokens::add_token_by_id_screen::AddTokenByIdScreen;
use crate::ui::tokens::tokens_screen::IdentityTokenInfo;
use crate::ui::tokens::transfer_tokens_screen::TransferTokensScreen;
use crate::ui::tokens::view_token_claims_screen::ViewTokenClaimsScreen;
use crate::ui::tools::document_visualizer_screen::DocumentVisualizerScreen;
use crate::ui::tools::proof_log_screen::ProofLogScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::wallets::import_wallet_screen::ImportWalletScreen;
use crate::ui::wallets::wallets_screen::WalletsBalancesScreen;
use contracts_documents::add_contracts_screen::AddContractsScreen;
use contracts_documents::group_actions_screen::GroupActionsScreen;
use contracts_documents::register_contract_screen::RegisterDataContractScreen;
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
use tokens::burn_tokens_screen::BurnTokensScreen;
use tokens::claim_tokens_screen::ClaimTokensScreen;
use tokens::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use tokens::freeze_tokens_screen::FreezeTokensScreen;
use tokens::mint_tokens_screen::MintTokensScreen;
use tokens::pause_tokens_screen::PauseTokensScreen;
use tokens::resume_tokens_screen::ResumeTokensScreen;
use tokens::tokens_screen::{IdentityTokenBalance, TokensScreen, TokensSubscreen};
use tokens::unfreeze_tokens_screen::UnfreezeTokensScreen;
use tokens::update_token_config::UpdateTokenConfigScreen;
use tools::transition_visualizer_screen::TransitionVisualizerScreen;
use wallets::add_new_wallet_screen::AddNewWalletScreen;

pub mod components;
pub mod contracts_documents;
pub mod dpns;
pub mod helpers;
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
    RootScreenToolsDocumentVisualizerScreen,
    RootScreenNetworkChooser,
    RootScreenToolsProofVisualizerScreen,
    RootScreenMyTokenBalances,
    RootScreenTokenSearch,
    RootScreenTokenCreator,
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
            RootScreenType::RootScreenTokenCreator => 14,
            RootScreenType::RootScreenToolsDocumentVisualizerScreen => 15,
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
            14 => Some(RootScreenType::RootScreenTokenCreator),
            15 => Some(RootScreenType::RootScreenToolsDocumentVisualizerScreen),
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
            RootScreenType::RootScreenDocumentQuery => ScreenType::DocumentQuery,
            RootScreenType::RootScreenNetworkChooser => ScreenType::NetworkChooser,
            RootScreenType::RootScreenWalletsBalances => ScreenType::WalletsBalances,
            RootScreenType::RootScreenToolsProofLogScreen => ScreenType::ProofLog,
            RootScreenType::RootScreenDPNSScheduledVotes => ScreenType::ScheduledVotes,
            RootScreenType::RootScreenToolsProofVisualizerScreen => ScreenType::ProofVisualizer,
            RootScreenType::RootScreenMyTokenBalances => ScreenType::TokenBalances,
            RootScreenType::RootScreenTokenSearch => ScreenType::TokenSearch,
            RootScreenType::RootScreenTokenCreator => ScreenType::TokenCreator,
            RootScreenType::RootScreenToolsDocumentVisualizerScreen => {
                ScreenType::DocumentsVisualizer
            }
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
    DocumentQuery,
    NetworkChooser,
    RegisterDpnsName,
    RegisterContract,
    ProofLog,
    TopUpIdentity(QualifiedIdentity),
    ScheduledVotes,
    AddContracts,
    ProofVisualizer,
    DocumentsVisualizer,
    CreateDocument,
    GroupActions,

    // Token Screens
    TokenBalances,
    TokenSearch,
    TokenCreator,
    AddTokenById,
    TransferTokensScreen(IdentityTokenBalance),
    MintTokensScreen(IdentityTokenInfo),
    BurnTokensScreen(IdentityTokenInfo),
    DestroyFrozenFundsScreen(IdentityTokenInfo),
    FreezeTokensScreen(IdentityTokenInfo),
    UnfreezeTokensScreen(IdentityTokenInfo),
    PauseTokensScreen(IdentityTokenInfo),
    ResumeTokensScreen(IdentityTokenInfo),
    ClaimTokensScreen(IdentityTokenBalance),
    ViewTokenClaimsScreen(IdentityTokenBalance),
    UpdateTokenConfigScreen(IdentityTokenInfo),
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
            ScreenType::RegisterContract => {
                Screen::RegisterDataContractScreen(RegisterDataContractScreen::new(app_context))
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
            ScreenType::DocumentQuery => {
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
            ScreenType::DocumentsVisualizer => {
                Screen::DocumentVisualizerScreen(DocumentVisualizerScreen::new(app_context))
            }
            ScreenType::CreateDocument => {
                Screen::CreateDocumentScreen(CreateDocumentScreen::new(app_context))
            }
            ScreenType::GroupActions => {
                Screen::GroupActionsScreen(GroupActionsScreen::new(app_context))
            }

            // Token Screens
            ScreenType::TokenBalances => {
                Screen::TokensScreen(TokensScreen::new(app_context, TokensSubscreen::MyTokens))
            }
            ScreenType::TokenSearch => Screen::TokensScreen(TokensScreen::new(
                app_context,
                TokensSubscreen::SearchTokens,
            )),
            ScreenType::TokenCreator => Screen::TokensScreen(TokensScreen::new(
                app_context,
                TokensSubscreen::TokenCreator,
            )),
            ScreenType::TransferTokensScreen(identity_token_balance) => {
                Screen::TransferTokensScreen(TransferTokensScreen::new(
                    identity_token_balance.clone(),
                    app_context,
                ))
            }
            ScreenType::MintTokensScreen(identity_token_info) => Screen::MintTokensScreen(
                MintTokensScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::BurnTokensScreen(identity_token_info) => Screen::BurnTokensScreen(
                BurnTokensScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::DestroyFrozenFundsScreen(identity_token_info) => {
                Screen::DestroyFrozenFundsScreen(DestroyFrozenFundsScreen::new(
                    identity_token_info.clone(),
                    app_context,
                ))
            }
            ScreenType::FreezeTokensScreen(identity_token_info) => Screen::FreezeTokensScreen(
                FreezeTokensScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::UnfreezeTokensScreen(identity_token_info) => Screen::UnfreezeTokensScreen(
                UnfreezeTokensScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::PauseTokensScreen(identity_token_info) => Screen::PauseTokensScreen(
                PauseTokensScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::ResumeTokensScreen(identity_token_info) => Screen::ResumeTokensScreen(
                ResumeTokensScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::ClaimTokensScreen(_) => {
                unreachable!();
            }
            ScreenType::ViewTokenClaimsScreen(identity_token_balance) => {
                Screen::ViewTokenClaimsScreen(ViewTokenClaimsScreen::new(
                    identity_token_balance.clone(),
                    app_context,
                ))
            }
            ScreenType::UpdateTokenConfigScreen(identity_token_info) => {
                Screen::UpdateTokenConfigScreen(UpdateTokenConfigScreen::new(
                    identity_token_info.clone(),
                    app_context,
                ))
            }
            ScreenType::AddTokenById => Screen::AddTokenById(AddTokenByIdScreen::new(app_context)),
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
    RegisterDataContractScreen(RegisterDataContractScreen),
    CreateDocumentScreen(CreateDocumentScreen),
    GroupActionsScreen(GroupActionsScreen),
    WithdrawalScreen(WithdrawalScreen),
    TopUpIdentityScreen(TopUpIdentityScreen),
    TransferScreen(TransferScreen),
    AddKeyScreen(AddKeyScreen),
    ProofLogScreen(ProofLogScreen),
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    DocumentVisualizerScreen(DocumentVisualizerScreen),
    NetworkChooserScreen(NetworkChooserScreen),
    WalletsBalancesScreen(WalletsBalancesScreen),
    AddContractsScreen(AddContractsScreen),
    ProofVisualizerScreen(ProofVisualizerScreen),

    // Token Screens
    TokensScreen(TokensScreen),
    TransferTokensScreen(TransferTokensScreen),
    MintTokensScreen(MintTokensScreen),
    BurnTokensScreen(BurnTokensScreen),
    DestroyFrozenFundsScreen(DestroyFrozenFundsScreen),
    FreezeTokensScreen(FreezeTokensScreen),
    UnfreezeTokensScreen(UnfreezeTokensScreen),
    PauseTokensScreen(PauseTokensScreen),
    ResumeTokensScreen(ResumeTokensScreen),
    ClaimTokensScreen(ClaimTokensScreen),
    ViewTokenClaimsScreen(ViewTokenClaimsScreen),
    UpdateTokenConfigScreen(UpdateTokenConfigScreen),
    AddTokenById(AddTokenByIdScreen),
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
            Screen::RegisterDataContractScreen(screen) => screen.app_context = app_context,
            Screen::CreateDocumentScreen(screen) => screen.app_context = app_context,
            Screen::GroupActionsScreen(screen) => screen.app_context = app_context,
            Screen::AddNewWalletScreen(screen) => screen.app_context = app_context,
            Screen::TransferScreen(screen) => screen.app_context = app_context,
            Screen::TopUpIdentityScreen(screen) => screen.app_context = app_context,
            Screen::WalletsBalancesScreen(screen) => screen.app_context = app_context,
            Screen::ImportWalletScreen(screen) => screen.app_context = app_context,
            Screen::ProofLogScreen(screen) => screen.app_context = app_context,
            Screen::AddContractsScreen(screen) => screen.app_context = app_context,
            Screen::ProofVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::DocumentVisualizerScreen(screen) => screen.app_context = app_context,

            // Token Screens
            Screen::TokensScreen(screen) => screen.app_context = app_context,
            Screen::TransferTokensScreen(screen) => screen.app_context = app_context,
            Screen::MintTokensScreen(screen) => screen.app_context = app_context,
            Screen::BurnTokensScreen(screen) => screen.app_context = app_context,
            Screen::DestroyFrozenFundsScreen(screen) => screen.app_context = app_context,
            Screen::FreezeTokensScreen(screen) => screen.app_context = app_context,
            Screen::UnfreezeTokensScreen(screen) => screen.app_context = app_context,
            Screen::PauseTokensScreen(screen) => screen.app_context = app_context,
            Screen::ResumeTokensScreen(screen) => screen.app_context = app_context,
            Screen::ClaimTokensScreen(screen) => screen.app_context = app_context,
            Screen::ViewTokenClaimsScreen(screen) => screen.app_context = app_context,
            Screen::UpdateTokenConfigScreen(screen) => screen.app_context = app_context,
            Screen::AddTokenById(screen) => screen.app_context = app_context,
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
            Screen::DocumentQueryScreen(_) => ScreenType::DocumentQuery,
            Screen::AddNewIdentityScreen(_) => ScreenType::AddExistingIdentity,
            Screen::TopUpIdentityScreen(screen) => {
                ScreenType::TopUpIdentity(screen.identity.clone())
            }
            Screen::RegisterDpnsNameScreen(_) => ScreenType::RegisterDpnsName,
            Screen::RegisterDataContractScreen(_) => ScreenType::RegisterContract,
            Screen::CreateDocumentScreen(_) => ScreenType::CreateDocument,
            Screen::GroupActionsScreen(_) => ScreenType::GroupActions,
            Screen::AddNewWalletScreen(_) => ScreenType::AddNewWallet,
            Screen::WalletsBalancesScreen(_) => ScreenType::WalletsBalances,
            Screen::ImportWalletScreen(_) => ScreenType::ImportWallet,
            Screen::ProofLogScreen(_) => ScreenType::ProofLog,
            Screen::AddContractsScreen(_) => ScreenType::AddContracts,
            Screen::ProofVisualizerScreen(_) => ScreenType::ProofVisualizer,
            Screen::DocumentVisualizerScreen(_) => ScreenType::DocumentsVisualizer,

            // Token Screens
            Screen::TokensScreen(TokensScreen {
                tokens_subscreen: TokensSubscreen::MyTokens,
                ..
            }) => ScreenType::TokenBalances,
            Screen::TokensScreen(TokensScreen {
                tokens_subscreen: TokensSubscreen::SearchTokens,
                ..
            }) => ScreenType::TokenSearch,
            Screen::TokensScreen(TokensScreen {
                tokens_subscreen: TokensSubscreen::TokenCreator,
                ..
            }) => ScreenType::TokenCreator,
            Screen::TransferScreen(screen) => ScreenType::TransferScreen(screen.identity.clone()),
            Screen::TransferTokensScreen(screen) => {
                ScreenType::TransferTokensScreen(screen.identity_token_balance.clone())
            }
            Screen::MintTokensScreen(screen) => {
                ScreenType::MintTokensScreen(screen.identity_token_info.clone())
            }
            Screen::BurnTokensScreen(screen) => {
                ScreenType::BurnTokensScreen(screen.identity_token_info.clone())
            }
            Screen::DestroyFrozenFundsScreen(screen) => {
                ScreenType::DestroyFrozenFundsScreen(screen.identity_token_info.clone())
            }
            Screen::FreezeTokensScreen(screen) => {
                ScreenType::FreezeTokensScreen(screen.identity_token_info.clone())
            }
            Screen::UnfreezeTokensScreen(screen) => {
                ScreenType::UnfreezeTokensScreen(screen.identity_token_info.clone())
            }
            Screen::PauseTokensScreen(screen) => {
                ScreenType::PauseTokensScreen(screen.identity_token_info.clone())
            }
            Screen::ResumeTokensScreen(screen) => {
                ScreenType::ResumeTokensScreen(screen.identity_token_info.clone())
            }
            Screen::ClaimTokensScreen(screen) => {
                ScreenType::ClaimTokensScreen(screen.identity_token_balance.clone())
            }
            Screen::ViewTokenClaimsScreen(screen) => {
                ScreenType::ViewTokenClaimsScreen(screen.identity_token_balance.clone())
            }
            Screen::UpdateTokenConfigScreen(screen) => {
                ScreenType::UpdateTokenConfigScreen(screen.identity_token_info.clone())
            }
            Screen::AddTokenById(_) => ScreenType::AddTokenById,
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
            Screen::RegisterDataContractScreen(screen) => screen.refresh(),
            Screen::CreateDocumentScreen(screen) => screen.refresh(),
            Screen::GroupActionsScreen(screen) => screen.refresh(),
            Screen::WithdrawalScreen(screen) => screen.refresh(),
            Screen::TransferScreen(screen) => screen.refresh(),
            Screen::AddKeyScreen(screen) => screen.refresh(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh(),
            Screen::NetworkChooserScreen(screen) => screen.refresh(),
            Screen::WalletsBalancesScreen(screen) => screen.refresh(),
            Screen::ProofLogScreen(screen) => screen.refresh(),
            Screen::AddContractsScreen(screen) => screen.refresh(),
            Screen::ProofVisualizerScreen(screen) => screen.refresh(),
            Screen::DocumentVisualizerScreen(screen) => screen.refresh(),

            // Token Screens
            Screen::TokensScreen(screen) => screen.refresh(),
            Screen::TransferTokensScreen(screen) => screen.refresh(),
            Screen::MintTokensScreen(screen) => screen.refresh(),
            Screen::BurnTokensScreen(screen) => screen.refresh(),
            Screen::DestroyFrozenFundsScreen(screen) => screen.refresh(),
            Screen::FreezeTokensScreen(screen) => screen.refresh(),
            Screen::UnfreezeTokensScreen(screen) => screen.refresh(),
            Screen::PauseTokensScreen(screen) => screen.refresh(),
            Screen::ResumeTokensScreen(screen) => screen.refresh(),
            Screen::ClaimTokensScreen(screen) => screen.refresh(),
            Screen::ViewTokenClaimsScreen(screen) => screen.refresh(),
            Screen::UpdateTokenConfigScreen(screen) => screen.refresh(),
            Screen::AddTokenById(screen) => screen.refresh(),
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
            Screen::RegisterDataContractScreen(screen) => screen.refresh_on_arrival(),
            Screen::CreateDocumentScreen(screen) => screen.refresh_on_arrival(),
            Screen::GroupActionsScreen(screen) => screen.refresh_on_arrival(),
            Screen::WithdrawalScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransferScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddKeyScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::NetworkChooserScreen(screen) => screen.refresh_on_arrival(),
            Screen::WalletsBalancesScreen(screen) => screen.refresh_on_arrival(),
            Screen::ProofLogScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddContractsScreen(screen) => screen.refresh_on_arrival(),
            Screen::ProofVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::DocumentVisualizerScreen(screen) => screen.refresh_on_arrival(),

            // Token Screens
            Screen::TokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransferTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::MintTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::BurnTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::DestroyFrozenFundsScreen(screen) => screen.refresh_on_arrival(),
            Screen::FreezeTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::UnfreezeTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::PauseTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::ResumeTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::ClaimTokensScreen(screen) => screen.refresh_on_arrival(),
            Screen::ViewTokenClaimsScreen(screen) => screen.refresh_on_arrival(),
            Screen::UpdateTokenConfigScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddTokenById(screen) => screen.refresh_on_arrival(),
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
            Screen::RegisterDataContractScreen(screen) => screen.ui(ctx),
            Screen::CreateDocumentScreen(screen) => screen.ui(ctx),
            Screen::GroupActionsScreen(screen) => screen.ui(ctx),
            Screen::WithdrawalScreen(screen) => screen.ui(ctx),
            Screen::TransferScreen(screen) => screen.ui(ctx),
            Screen::AddKeyScreen(screen) => screen.ui(ctx),
            Screen::TransitionVisualizerScreen(screen) => screen.ui(ctx),
            Screen::NetworkChooserScreen(screen) => screen.ui(ctx),
            Screen::WalletsBalancesScreen(screen) => screen.ui(ctx),
            Screen::ProofLogScreen(screen) => screen.ui(ctx),
            Screen::AddContractsScreen(screen) => screen.ui(ctx),
            Screen::ProofVisualizerScreen(screen) => screen.ui(ctx),
            Screen::DocumentVisualizerScreen(screen) => screen.ui(ctx),

            // Token Screens
            Screen::TokensScreen(screen) => screen.ui(ctx),
            Screen::TransferTokensScreen(screen) => screen.ui(ctx),
            Screen::MintTokensScreen(screen) => screen.ui(ctx),
            Screen::BurnTokensScreen(screen) => screen.ui(ctx),
            Screen::DestroyFrozenFundsScreen(screen) => screen.ui(ctx),
            Screen::FreezeTokensScreen(screen) => screen.ui(ctx),
            Screen::UnfreezeTokensScreen(screen) => screen.ui(ctx),
            Screen::PauseTokensScreen(screen) => screen.ui(ctx),
            Screen::ResumeTokensScreen(screen) => screen.ui(ctx),
            Screen::ClaimTokensScreen(screen) => screen.ui(ctx),
            Screen::ViewTokenClaimsScreen(screen) => screen.ui(ctx),
            Screen::UpdateTokenConfigScreen(screen) => screen.ui(ctx),
            Screen::AddTokenById(screen) => screen.ui(ctx),
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
            Screen::RegisterDataContractScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::CreateDocumentScreen(screen) => screen.display_message(message, message_type),
            Screen::GroupActionsScreen(screen) => screen.display_message(message, message_type),
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
            Screen::DocumentVisualizerScreen(screen) => {
                screen.display_message(message, message_type)
            }

            // Token Screens
            Screen::TokensScreen(screen) => screen.display_message(message, message_type),
            Screen::TransferTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::MintTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::BurnTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::DestroyFrozenFundsScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::FreezeTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::UnfreezeTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::PauseTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::ResumeTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::ClaimTokensScreen(screen) => screen.display_message(message, message_type),
            Screen::ViewTokenClaimsScreen(screen) => screen.display_message(message, message_type),
            Screen::UpdateTokenConfigScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::AddTokenById(screen) => screen.display_message(message, message_type),
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match self {
            Screen::IdentitiesScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DPNSScreen(screen) => screen.display_task_result(backend_task_success_result),
            Screen::DocumentQueryScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddNewWalletScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ImportWalletScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddNewIdentityScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::TopUpIdentityScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddExistingIdentityScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::KeyInfoScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::KeysScreen(screen) => screen.display_task_result(backend_task_success_result),
            Screen::RegisterDpnsNameScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::RegisterDataContractScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::CreateDocumentScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::GroupActionsScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::WithdrawalScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::TransferScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddKeyScreen(screen) => screen.display_task_result(backend_task_success_result),
            Screen::TransitionVisualizerScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DocumentVisualizerScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::NetworkChooserScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
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

            // Token Screens
            Screen::TokensScreen(screen) => screen.display_task_result(backend_task_success_result),
            Screen::TransferTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::MintTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::BurnTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DestroyFrozenFundsScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::FreezeTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::UnfreezeTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::PauseTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ResumeTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ClaimTokensScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ViewTokenClaimsScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::UpdateTokenConfigScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddTokenById(screen) => screen.display_task_result(backend_task_success_result),
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
            Screen::RegisterDataContractScreen(screen) => screen.pop_on_success(),
            Screen::CreateDocumentScreen(screen) => screen.pop_on_success(),
            Screen::GroupActionsScreen(screen) => screen.pop_on_success(),
            Screen::WithdrawalScreen(screen) => screen.pop_on_success(),
            Screen::TransferScreen(screen) => screen.pop_on_success(),
            Screen::AddKeyScreen(screen) => screen.pop_on_success(),
            Screen::TransitionVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::NetworkChooserScreen(screen) => screen.pop_on_success(),
            Screen::WalletsBalancesScreen(screen) => screen.pop_on_success(),
            Screen::ProofLogScreen(screen) => screen.pop_on_success(),
            Screen::AddContractsScreen(screen) => screen.pop_on_success(),
            Screen::ProofVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::DocumentVisualizerScreen(screen) => screen.pop_on_success(),

            // Token Screens
            Screen::TokensScreen(screen) => screen.pop_on_success(),
            Screen::TransferTokensScreen(screen) => screen.pop_on_success(),
            Screen::MintTokensScreen(screen) => screen.pop_on_success(),
            Screen::BurnTokensScreen(screen) => screen.pop_on_success(),
            Screen::DestroyFrozenFundsScreen(screen) => screen.pop_on_success(),
            Screen::FreezeTokensScreen(screen) => screen.pop_on_success(),
            Screen::UnfreezeTokensScreen(screen) => screen.pop_on_success(),
            Screen::PauseTokensScreen(screen) => screen.pop_on_success(),
            Screen::ResumeTokensScreen(screen) => screen.pop_on_success(),
            Screen::ClaimTokensScreen(screen) => screen.pop_on_success(),
            Screen::ViewTokenClaimsScreen(screen) => screen.pop_on_success(),
            Screen::UpdateTokenConfigScreen(screen) => screen.pop_on_success(),
            Screen::AddTokenById(screen) => screen.pop_on_success(),
        }
    }
}
