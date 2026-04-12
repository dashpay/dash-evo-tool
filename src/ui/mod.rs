use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::wallet::Wallet;
use crate::model::wallet::WalletId;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen;
use crate::ui::contracts_documents::document_action_screen::{
    DocumentActionScreen, DocumentActionType,
};
use crate::ui::dashpay::add_contact_screen::AddContactScreen;
use crate::ui::dashpay::contact_details::ContactDetailsScreen;
use crate::ui::dashpay::contact_info_editor::ContactInfoEditorScreen;
use crate::ui::dashpay::contact_profile_viewer::ContactProfileViewerScreen;
use crate::ui::dashpay::profile_search::ProfileSearchScreen;
use crate::ui::dashpay::qr_code_generator::QRCodeGeneratorScreen;
use crate::ui::dashpay::send_payment::SendPaymentScreen;
use crate::ui::dashpay::{DashPayScreen, DashPaySubscreen};
use crate::ui::dpns::dpns_contested_names_screen::DPNSScreen;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::identities::keys::keys_screen::KeysScreen;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use crate::ui::identities::transfer_screen::TransferScreen;
use crate::ui::identities::withdraw_screen::WithdrawalScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::tokens::add_token_by_id_screen::AddTokenByIdScreen;
use crate::ui::tokens::tokens_screen::{IdentityTokenBasicInfo, IdentityTokenInfo};
use crate::ui::tokens::transfer_tokens_screen::TransferTokensScreen;
use crate::ui::tokens::view_token_claims_screen::ViewTokenClaimsScreen;
use crate::ui::tools::address_balance_screen::AddressBalanceScreen;
use crate::ui::tools::contract_visualizer_screen::ContractVisualizerScreen;
use crate::ui::tools::document_visualizer_screen::DocumentVisualizerScreen;
use crate::ui::tools::grovestark_screen::GroveSTARKScreen;
use crate::ui::tools::masternode_list_diff_screen::MasternodeListDiffScreen;
use crate::ui::tools::platform_info_screen::PlatformInfoScreen;
use crate::ui::tools::proof_log_screen::ProofLogScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::wallets::asset_lock_detail_screen::AssetLockDetailScreen;
use crate::ui::wallets::create_asset_lock_screen::CreateAssetLockScreen;
use crate::ui::wallets::import_mnemonic_screen::ImportMnemonicScreen;
use crate::ui::wallets::send_screen::WalletSendScreen;
use crate::ui::wallets::single_key_send_screen::SingleKeyWalletSendScreen;
use crate::ui::wallets::wallets_screen::WalletsBalancesScreen;
use contracts_documents::add_contracts_screen::AddContractsScreen;
use contracts_documents::group_actions_screen::GroupActionsScreen;
use contracts_documents::register_contract_screen::RegisterDataContractScreen;
use contracts_documents::update_contract_screen::UpdateDataContractScreen;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use dash_sdk::platform::Identifier;
use dpns::dpns_contested_names_screen::DPNSSubscreen;
use egui::Context;
use identities::add_existing_identity_screen::AddExistingIdentityScreen;
use identities::add_new_identity_screen::AddNewIdentityScreen;
use identities::identities_screen::IdentitiesScreen;
use identities::register_dpns_name_screen::{RegisterDpnsNameScreen, RegisterDpnsNameSource};
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::RwLock;
use tokens::burn_tokens_screen::BurnTokensScreen;
use tokens::claim_tokens_screen::ClaimTokensScreen;
use tokens::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use tokens::direct_token_purchase_screen::PurchaseTokenScreen;
use tokens::freeze_tokens_screen::FreezeTokensScreen;
use tokens::mint_tokens_screen::MintTokensScreen;
use tokens::pause_tokens_screen::PauseTokensScreen;
use tokens::resume_tokens_screen::ResumeTokensScreen;
use tokens::set_token_price_screen::SetTokenPriceScreen;
use tokens::tokens_screen::{IdentityTokenBalance, TokensScreen, TokensSubscreen};
use tokens::unfreeze_tokens_screen::UnfreezeTokensScreen;
use tokens::update_token_config::UpdateTokenConfigScreen;
use tools::transition_visualizer_screen::TransitionVisualizerScreen;
use wallets::add_new_wallet_screen::AddNewWalletScreen;
use wallets::shield_screen::ShieldScreen;
use wallets::shielded_send_screen::ShieldedSendScreen;
use wallets::unshield_credits_screen::UnshieldCreditsScreen;

pub mod components;
pub mod contracts_documents;
pub mod dashpay;
pub mod dpns;
pub mod helpers;
pub(crate) mod identities;
pub mod network_chooser_screen;
pub mod theme;
pub mod tokens;
pub mod tools;
pub(crate) mod wallets;
pub mod welcome_screen;

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
#[allow(clippy::enum_variant_names)]
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
    RootScreenToolsMasternodeListDiffScreen,
    RootScreenToolsContractVisualizerScreen,
    RootScreenToolsPlatformInfoScreen,
    RootScreenDashPayContacts,
    RootScreenDashPayProfile,
    RootScreenDashPayPayments,
    RootScreenDashPayProfileSearch,
    RootScreenToolsGroveSTARKScreen,
    RootScreenToolsAddressBalanceScreen,
    RootScreenDashpay,
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
            RootScreenType::RootScreenToolsContractVisualizerScreen => 16,
            RootScreenType::RootScreenToolsPlatformInfoScreen => 17,
            RootScreenType::RootScreenDashPayContacts => 18,
            // 19 used to be RootScreenDashPayRequests (now consolidated into Contacts)
            RootScreenType::RootScreenDashPayProfile => 20,
            RootScreenType::RootScreenDashPayPayments => 21,
            RootScreenType::RootScreenDashPayProfileSearch => 22,
            RootScreenType::RootScreenToolsMasternodeListDiffScreen => 23,
            RootScreenType::RootScreenDashpay => 24,
            RootScreenType::RootScreenToolsGroveSTARKScreen => 25,
            RootScreenType::RootScreenToolsAddressBalanceScreen => 26,
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
            16 => Some(RootScreenType::RootScreenToolsContractVisualizerScreen),
            17 => Some(RootScreenType::RootScreenToolsPlatformInfoScreen),
            18 => Some(RootScreenType::RootScreenDashPayContacts),
            // 19 used to be RootScreenDashPayRequests (now consolidated into Contacts)
            20 => Some(RootScreenType::RootScreenDashPayProfile),
            21 => Some(RootScreenType::RootScreenDashPayPayments),
            22 => Some(RootScreenType::RootScreenDashPayProfileSearch),
            23 => Some(RootScreenType::RootScreenToolsMasternodeListDiffScreen),
            24 => Some(RootScreenType::RootScreenDashpay),
            25 => Some(RootScreenType::RootScreenToolsGroveSTARKScreen),
            26 => Some(RootScreenType::RootScreenToolsAddressBalanceScreen),
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
            RootScreenType::RootScreenToolsMasternodeListDiffScreen => {
                ScreenType::MasternodeListDiff
            }
            RootScreenType::RootScreenToolsDocumentVisualizerScreen => {
                ScreenType::DocumentsVisualizer
            }
            RootScreenType::RootScreenToolsContractVisualizerScreen => {
                ScreenType::ContractsVisualizer
            }
            RootScreenType::RootScreenToolsPlatformInfoScreen => ScreenType::PlatformInfo,
            RootScreenType::RootScreenDashPayContacts => ScreenType::DashPayContacts,
            RootScreenType::RootScreenDashPayProfile => ScreenType::DashPayProfile,
            RootScreenType::RootScreenDashPayPayments => ScreenType::DashPayPayments,
            RootScreenType::RootScreenDashPayProfileSearch => ScreenType::DashPayProfileSearch,
            RootScreenType::RootScreenToolsGroveSTARKScreen => ScreenType::GroveSTARK,
            RootScreenType::RootScreenToolsAddressBalanceScreen => ScreenType::AddressBalance,
            RootScreenType::RootScreenDashpay => ScreenType::Dashpay,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum ScreenType {
    #[default]
    Identities,
    DPNSActiveContests,
    DPNSPastContests,
    DPNSMyUsernames,
    AddNewIdentity,
    WalletsBalances,
    ImportMnemonic,
    AddNewWallet,
    WalletSendScreen(Arc<RwLock<Wallet>>),
    SingleKeyWalletSendScreen(Arc<RwLock<SingleKeyWallet>>),
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
    RegisterDpnsName(RegisterDpnsNameSource),
    RegisterContract,
    UpdateContract,
    ProofLog,
    MasternodeListDiff,
    TopUpIdentity(QualifiedIdentity),
    ScheduledVotes,
    AddContracts,
    ProofVisualizer,
    DocumentsVisualizer,
    ContractsVisualizer,
    PlatformInfo,
    GroveSTARK,
    AddressBalance,
    Dashpay,
    CreateDocument,
    DeleteDocument,
    ReplaceDocument,
    TransferDocument,
    PurchaseDocument,
    SetDocumentPrice,
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
    ClaimTokensScreen(IdentityTokenBasicInfo),
    ViewTokenClaimsScreen(IdentityTokenBasicInfo),
    UpdateTokenConfigScreen(IdentityTokenInfo),
    PurchaseTokenScreen(IdentityTokenInfo),
    SetTokenPriceScreen(IdentityTokenInfo),

    // Wallet screens
    AssetLockDetail([u8; 32], [u8; 32]),
    CreateAssetLock(Arc<RwLock<Wallet>>),

    // Shielded screens
    ShieldScreen(WalletId),
    ShieldedSendScreen(WalletId),
    UnshieldCreditsScreen(WalletId),

    // DashPay Screens
    DashPayContacts,
    DashPayProfile,
    DashPayPayments,
    DashPayAddContact,
    DashPayAddContactWithId(String), // Pre-populated identity ID
    DashPayContactDetails(QualifiedIdentity, Identifier),
    DashPayContactProfileViewer(QualifiedIdentity, Identifier),
    DashPaySendPayment(QualifiedIdentity, Identifier),
    DashPayContactInfoEditor(QualifiedIdentity, Identifier),
    DashPayQRGenerator,
    DashPayProfileSearch,
}

impl PartialEq for ScreenType {
    fn eq(&self, other: &Self) -> bool {
        // Compare variants, ignoring Arc<RwLock<Wallet>> contents for WalletSendScreen
        match (self, other) {
            (ScreenType::WalletSendScreen(_), ScreenType::WalletSendScreen(_)) => true,
            (
                ScreenType::SingleKeyWalletSendScreen(_),
                ScreenType::SingleKeyWalletSendScreen(_),
            ) => true,
            (ScreenType::CreateAssetLock(_), ScreenType::CreateAssetLock(_)) => true,
            (ScreenType::AssetLockDetail(a1, a2), ScreenType::AssetLockDetail(b1, b2)) => {
                a1 == b1 && a2 == b2
            }
            (ScreenType::Identities, ScreenType::Identities) => true,
            (ScreenType::DPNSActiveContests, ScreenType::DPNSActiveContests) => true,
            (ScreenType::DPNSPastContests, ScreenType::DPNSPastContests) => true,
            (ScreenType::DPNSMyUsernames, ScreenType::DPNSMyUsernames) => true,
            (ScreenType::AddNewIdentity, ScreenType::AddNewIdentity) => true,
            (ScreenType::WalletsBalances, ScreenType::WalletsBalances) => true,
            (ScreenType::ImportMnemonic, ScreenType::ImportMnemonic) => true,
            (ScreenType::AddNewWallet, ScreenType::AddNewWallet) => true,
            (ScreenType::AddExistingIdentity, ScreenType::AddExistingIdentity) => true,
            (ScreenType::TransitionVisualizer, ScreenType::TransitionVisualizer) => true,
            (ScreenType::WithdrawalScreen(a), ScreenType::WithdrawalScreen(b)) => a == b,
            (ScreenType::TransferScreen(a), ScreenType::TransferScreen(b)) => a == b,
            (ScreenType::AddKeyScreen(a), ScreenType::AddKeyScreen(b)) => a == b,
            (ScreenType::KeyInfo(a1, a2, a3), ScreenType::KeyInfo(b1, b2, b3)) => {
                a1 == b1 && a2 == b2 && a3 == b3
            }
            (ScreenType::Keys(a), ScreenType::Keys(b)) => a == b,
            (ScreenType::DocumentQuery, ScreenType::DocumentQuery) => true,
            (ScreenType::NetworkChooser, ScreenType::NetworkChooser) => true,
            (ScreenType::RegisterDpnsName(a), ScreenType::RegisterDpnsName(b)) => a == b,
            (ScreenType::RegisterContract, ScreenType::RegisterContract) => true,
            (ScreenType::UpdateContract, ScreenType::UpdateContract) => true,
            (ScreenType::ProofLog, ScreenType::ProofLog) => true,
            (ScreenType::MasternodeListDiff, ScreenType::MasternodeListDiff) => true,
            (ScreenType::TopUpIdentity(a), ScreenType::TopUpIdentity(b)) => a == b,
            (ScreenType::ScheduledVotes, ScreenType::ScheduledVotes) => true,
            (ScreenType::AddContracts, ScreenType::AddContracts) => true,
            (ScreenType::ProofVisualizer, ScreenType::ProofVisualizer) => true,
            (ScreenType::DocumentsVisualizer, ScreenType::DocumentsVisualizer) => true,
            (ScreenType::ContractsVisualizer, ScreenType::ContractsVisualizer) => true,
            (ScreenType::PlatformInfo, ScreenType::PlatformInfo) => true,
            (ScreenType::GroveSTARK, ScreenType::GroveSTARK) => true,
            (ScreenType::AddressBalance, ScreenType::AddressBalance) => true,
            (ScreenType::Dashpay, ScreenType::Dashpay) => true,
            (ScreenType::CreateDocument, ScreenType::CreateDocument) => true,
            (ScreenType::DeleteDocument, ScreenType::DeleteDocument) => true,
            (ScreenType::ReplaceDocument, ScreenType::ReplaceDocument) => true,
            (ScreenType::TransferDocument, ScreenType::TransferDocument) => true,
            (ScreenType::PurchaseDocument, ScreenType::PurchaseDocument) => true,
            (ScreenType::SetDocumentPrice, ScreenType::SetDocumentPrice) => true,
            (ScreenType::GroupActions, ScreenType::GroupActions) => true,
            // Token Screens
            (ScreenType::TokenBalances, ScreenType::TokenBalances) => true,
            (ScreenType::TokenSearch, ScreenType::TokenSearch) => true,
            (ScreenType::TokenCreator, ScreenType::TokenCreator) => true,
            (ScreenType::AddTokenById, ScreenType::AddTokenById) => true,
            (ScreenType::TransferTokensScreen(a), ScreenType::TransferTokensScreen(b)) => a == b,
            (ScreenType::MintTokensScreen(a), ScreenType::MintTokensScreen(b)) => a == b,
            (ScreenType::BurnTokensScreen(a), ScreenType::BurnTokensScreen(b)) => a == b,
            (ScreenType::DestroyFrozenFundsScreen(a), ScreenType::DestroyFrozenFundsScreen(b)) => {
                a == b
            }
            (ScreenType::FreezeTokensScreen(a), ScreenType::FreezeTokensScreen(b)) => a == b,
            (ScreenType::UnfreezeTokensScreen(a), ScreenType::UnfreezeTokensScreen(b)) => a == b,
            (ScreenType::PauseTokensScreen(a), ScreenType::PauseTokensScreen(b)) => a == b,
            (ScreenType::ResumeTokensScreen(a), ScreenType::ResumeTokensScreen(b)) => a == b,
            (ScreenType::ClaimTokensScreen(a), ScreenType::ClaimTokensScreen(b)) => a == b,
            (ScreenType::ViewTokenClaimsScreen(a), ScreenType::ViewTokenClaimsScreen(b)) => a == b,
            (ScreenType::UpdateTokenConfigScreen(a), ScreenType::UpdateTokenConfigScreen(b)) => {
                a == b
            }
            (ScreenType::PurchaseTokenScreen(a), ScreenType::PurchaseTokenScreen(b)) => a == b,
            (ScreenType::SetTokenPriceScreen(a), ScreenType::SetTokenPriceScreen(b)) => a == b,
            // DashPay Screens
            (ScreenType::DashPayContacts, ScreenType::DashPayContacts) => true,
            (ScreenType::DashPayProfile, ScreenType::DashPayProfile) => true,
            (ScreenType::DashPayPayments, ScreenType::DashPayPayments) => true,
            (ScreenType::DashPayAddContact, ScreenType::DashPayAddContact) => true,
            (ScreenType::DashPayAddContactWithId(a), ScreenType::DashPayAddContactWithId(b)) => {
                a == b
            }
            (
                ScreenType::DashPayContactDetails(a1, a2),
                ScreenType::DashPayContactDetails(b1, b2),
            ) => a1 == b1 && a2 == b2,
            (
                ScreenType::DashPayContactProfileViewer(a1, a2),
                ScreenType::DashPayContactProfileViewer(b1, b2),
            ) => a1 == b1 && a2 == b2,
            (ScreenType::DashPaySendPayment(a1, a2), ScreenType::DashPaySendPayment(b1, b2)) => {
                a1 == b1 && a2 == b2
            }
            (
                ScreenType::DashPayContactInfoEditor(a1, a2),
                ScreenType::DashPayContactInfoEditor(b1, b2),
            ) => a1 == b1 && a2 == b2,
            (ScreenType::DashPayQRGenerator, ScreenType::DashPayQRGenerator) => true,
            (ScreenType::DashPayProfileSearch, ScreenType::DashPayProfileSearch) => true,
            // Shielded screens
            (ScreenType::ShieldScreen(a), ScreenType::ShieldScreen(b)) => a == b,
            (ScreenType::ShieldedSendScreen(a), ScreenType::ShieldedSendScreen(b)) => a == b,
            (ScreenType::UnshieldCreditsScreen(a), ScreenType::UnshieldCreditsScreen(b)) => a == b,
            _ => false,
        }
    }
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
            ScreenType::RegisterDpnsName(source) => {
                Screen::RegisterDpnsNameScreen(RegisterDpnsNameScreen::new(app_context, *source))
            }
            ScreenType::RegisterContract => {
                Screen::RegisterDataContractScreen(RegisterDataContractScreen::new(app_context))
            }
            ScreenType::UpdateContract => {
                Screen::UpdateDataContractScreen(UpdateDataContractScreen::new(app_context))
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
            ScreenType::ImportMnemonic => {
                Screen::ImportMnemonicScreen(ImportMnemonicScreen::new(app_context))
            }
            ScreenType::WalletSendScreen(wallet) => {
                Screen::WalletSendScreen(WalletSendScreen::new(app_context, wallet.clone()))
            }
            ScreenType::SingleKeyWalletSendScreen(wallet) => Screen::SingleKeyWalletSendScreen(
                SingleKeyWalletSendScreen::new(app_context, wallet.clone()),
            ),
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
            ScreenType::ContractsVisualizer => {
                Screen::ContractVisualizerScreen(ContractVisualizerScreen::new(app_context))
            }
            ScreenType::PlatformInfo => {
                Screen::PlatformInfoScreen(PlatformInfoScreen::new(app_context))
            }
            ScreenType::GroveSTARK => Screen::GroveSTARKScreen(GroveSTARKScreen::new(app_context)),
            ScreenType::AddressBalance => {
                Screen::AddressBalanceScreen(AddressBalanceScreen::new(app_context))
            }
            ScreenType::Dashpay => {
                Screen::DashPayScreen(DashPayScreen::new(app_context, DashPaySubscreen::Profile))
            }
            ScreenType::CreateDocument => Screen::DocumentActionScreen(DocumentActionScreen::new(
                app_context.clone(),
                None,
                DocumentActionType::Create,
            )),
            ScreenType::DeleteDocument => Screen::DocumentActionScreen(DocumentActionScreen::new(
                app_context.clone(),
                None,
                DocumentActionType::Delete,
            )),
            ScreenType::ReplaceDocument => Screen::DocumentActionScreen(DocumentActionScreen::new(
                app_context.clone(),
                None,
                DocumentActionType::Replace,
            )),
            ScreenType::TransferDocument => Screen::DocumentActionScreen(
                DocumentActionScreen::new(app_context.clone(), None, DocumentActionType::Transfer),
            ),
            ScreenType::PurchaseDocument => Screen::DocumentActionScreen(
                DocumentActionScreen::new(app_context.clone(), None, DocumentActionType::Purchase),
            ),
            ScreenType::SetDocumentPrice => Screen::DocumentActionScreen(
                DocumentActionScreen::new(app_context.clone(), None, DocumentActionType::SetPrice),
            ),
            ScreenType::GroupActions => {
                Screen::GroupActionsScreen(GroupActionsScreen::new(app_context))
            }
            // Token Screens
            ScreenType::TokenBalances => Screen::TokensScreen(Box::new(TokensScreen::new(
                app_context,
                TokensSubscreen::MyTokens,
            ))),
            ScreenType::TokenSearch => Screen::TokensScreen(Box::new(TokensScreen::new(
                app_context,
                TokensSubscreen::SearchTokens,
            ))),
            ScreenType::TokenCreator => Screen::TokensScreen(Box::new(TokensScreen::new(
                app_context,
                TokensSubscreen::TokenCreator,
            ))),
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
                unreachable!()
            }
            ScreenType::ViewTokenClaimsScreen(_) => {
                unreachable!()
            }
            ScreenType::UpdateTokenConfigScreen(identity_token_info) => {
                Screen::UpdateTokenConfigScreen(Box::new(UpdateTokenConfigScreen::new(
                    identity_token_info.clone(),
                    app_context,
                )))
            }
            ScreenType::MasternodeListDiff => {
                Screen::MasternodeListDiffScreen(MasternodeListDiffScreen::new(app_context))
            }
            ScreenType::AddTokenById => Screen::AddTokenById(AddTokenByIdScreen::new(app_context)),
            ScreenType::PurchaseTokenScreen(identity_token_info) => Screen::PurchaseTokenScreen(
                PurchaseTokenScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::SetTokenPriceScreen(identity_token_info) => Screen::SetTokenPriceScreen(
                SetTokenPriceScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::AssetLockDetail(wallet_seed_hash, txid) => Screen::AssetLockDetailScreen(
                AssetLockDetailScreen::new(*wallet_seed_hash, *txid, app_context),
            ),
            ScreenType::CreateAssetLock(wallet) => Screen::CreateAssetLockScreen(
                CreateAssetLockScreen::new(wallet.clone(), app_context),
            ),

            // DashPay Screens
            ScreenType::DashPayContacts => {
                Screen::DashPayScreen(DashPayScreen::new(app_context, DashPaySubscreen::Contacts))
            }
            ScreenType::DashPayProfile => {
                Screen::DashPayScreen(DashPayScreen::new(app_context, DashPaySubscreen::Profile))
            }
            ScreenType::DashPayPayments => {
                Screen::DashPayScreen(DashPayScreen::new(app_context, DashPaySubscreen::Payments))
            }
            ScreenType::DashPayAddContact => {
                Screen::DashPayAddContactScreen(AddContactScreen::new(app_context.clone()))
            }
            ScreenType::DashPayAddContactWithId(identity_id) => Screen::DashPayAddContactScreen(
                AddContactScreen::new_with_identity_id(app_context.clone(), identity_id.clone()),
            ),
            ScreenType::DashPayContactDetails(identity, contact_id) => {
                Screen::DashPayContactDetailsScreen(ContactDetailsScreen::new(
                    app_context.clone(),
                    identity.clone(),
                    *contact_id,
                ))
            }
            ScreenType::DashPayContactProfileViewer(identity, contact_id) => {
                Screen::DashPayContactProfileViewerScreen(ContactProfileViewerScreen::new(
                    app_context.clone(),
                    identity.clone(),
                    *contact_id,
                ))
            }
            ScreenType::DashPaySendPayment(identity, contact_id) => {
                Screen::DashPaySendPaymentScreen(SendPaymentScreen::new(
                    app_context.clone(),
                    identity.clone(),
                    *contact_id,
                ))
            }
            ScreenType::DashPayContactInfoEditor(identity, contact_id) => {
                Screen::DashPayContactInfoEditorScreen(ContactInfoEditorScreen::new(
                    app_context.clone(),
                    identity.clone(),
                    *contact_id,
                ))
            }
            ScreenType::DashPayQRGenerator => {
                Screen::DashPayQRGeneratorScreen(QRCodeGeneratorScreen::new(app_context.clone()))
            }
            ScreenType::DashPayProfileSearch => {
                Screen::DashPayProfileSearchScreen(ProfileSearchScreen::new(app_context.clone()))
            }
            // Shielded screens
            ScreenType::ShieldScreen(seed_hash) => {
                Screen::ShieldScreen(ShieldScreen::new(*seed_hash, app_context))
            }
            ScreenType::ShieldedSendScreen(seed_hash) => {
                Screen::ShieldedSendScreen(ShieldedSendScreen::new(*seed_hash, app_context))
            }
            ScreenType::UnshieldCreditsScreen(seed_hash) => {
                Screen::UnshieldCreditsScreen(UnshieldCreditsScreen::new(*seed_hash, app_context))
            }
        }
    }
}

#[allow(clippy::enum_variant_names, clippy::large_enum_variant)]
pub enum Screen {
    IdentitiesScreen(IdentitiesScreen),
    DPNSScreen(DPNSScreen),
    DocumentQueryScreen(DocumentQueryScreen),
    AddNewWalletScreen(AddNewWalletScreen),
    ImportMnemonicScreen(ImportMnemonicScreen),
    AddNewIdentityScreen(AddNewIdentityScreen),
    AddExistingIdentityScreen(AddExistingIdentityScreen),
    KeyInfoScreen(KeyInfoScreen),
    KeysScreen(KeysScreen),
    RegisterDpnsNameScreen(RegisterDpnsNameScreen),
    RegisterDataContractScreen(RegisterDataContractScreen),
    UpdateDataContractScreen(UpdateDataContractScreen),
    DocumentActionScreen(DocumentActionScreen),
    GroupActionsScreen(GroupActionsScreen),
    WithdrawalScreen(WithdrawalScreen),
    TopUpIdentityScreen(TopUpIdentityScreen),
    TransferScreen(TransferScreen),
    AddKeyScreen(AddKeyScreen),
    ProofLogScreen(ProofLogScreen),
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    DocumentVisualizerScreen(DocumentVisualizerScreen),
    ContractVisualizerScreen(ContractVisualizerScreen),
    NetworkChooserScreen(NetworkChooserScreen),
    WalletsBalancesScreen(WalletsBalancesScreen),
    WalletSendScreen(WalletSendScreen),
    SingleKeyWalletSendScreen(SingleKeyWalletSendScreen),
    AddContractsScreen(AddContractsScreen),
    ProofVisualizerScreen(ProofVisualizerScreen),
    MasternodeListDiffScreen(MasternodeListDiffScreen),
    PlatformInfoScreen(PlatformInfoScreen),
    GroveSTARKScreen(GroveSTARKScreen),
    AddressBalanceScreen(AddressBalanceScreen),

    // Token Screens
    TokensScreen(Box<TokensScreen>),
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
    UpdateTokenConfigScreen(Box<UpdateTokenConfigScreen>),
    AddTokenById(AddTokenByIdScreen),
    PurchaseTokenScreen(PurchaseTokenScreen),
    SetTokenPriceScreen(SetTokenPriceScreen),
    AssetLockDetailScreen(AssetLockDetailScreen),
    CreateAssetLockScreen(CreateAssetLockScreen),

    // Shielded Screens
    ShieldScreen(ShieldScreen),
    ShieldedSendScreen(ShieldedSendScreen),
    UnshieldCreditsScreen(UnshieldCreditsScreen),

    // DashPay Screens
    DashPayScreen(DashPayScreen),
    DashPayAddContactScreen(AddContactScreen),
    DashPayContactDetailsScreen(ContactDetailsScreen),
    DashPayContactProfileViewerScreen(ContactProfileViewerScreen),
    DashPaySendPaymentScreen(SendPaymentScreen),
    DashPayContactInfoEditorScreen(ContactInfoEditorScreen),
    DashPayQRGeneratorScreen(QRCodeGeneratorScreen),
    DashPayProfileSearchScreen(ProfileSearchScreen),
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
            Screen::ContractVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::NetworkChooserScreen(screen) => screen.current_network = app_context.network,
            Screen::AddKeyScreen(screen) => screen.app_context = app_context,
            Screen::DocumentQueryScreen(screen) => screen.app_context = app_context,
            Screen::AddNewIdentityScreen(screen) => screen.app_context = app_context,
            Screen::RegisterDpnsNameScreen(screen) => screen.app_context = app_context,
            Screen::RegisterDataContractScreen(screen) => screen.app_context = app_context,
            Screen::UpdateDataContractScreen(screen) => screen.app_context = app_context,
            Screen::DocumentActionScreen(screen) => screen.app_context = app_context,
            Screen::GroupActionsScreen(screen) => screen.app_context = app_context,
            Screen::AddNewWalletScreen(screen) => {
                screen.app_context = app_context;
                screen.reset_core_wallets_cache();
            }
            Screen::TransferScreen(screen) => {
                screen.app_context = app_context;
                screen.invalidate_address_input();
            }
            Screen::TopUpIdentityScreen(screen) => screen.app_context = app_context,
            Screen::WalletsBalancesScreen(screen) => {
                screen.app_context = app_context;
                screen.reset_pending_list_state();
                screen.update_selected_wallet_for_network();
                screen.invalidate_address_inputs();
            }
            Screen::ImportMnemonicScreen(screen) => {
                screen.app_context = app_context;
                screen.reset_core_wallets_cache();
            }
            Screen::WalletSendScreen(screen) => {
                screen.app_context = app_context;
                screen.invalidate_address_input();
            }
            Screen::SingleKeyWalletSendScreen(screen) => screen.app_context = app_context,
            Screen::ProofLogScreen(screen) => screen.app_context = app_context,
            Screen::AddContractsScreen(screen) => screen.app_context = app_context,
            Screen::ProofVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::MasternodeListDiffScreen(screen) => {
                let old_net = screen.app_context.network;
                if old_net != app_context.network {
                    // Switch context and clear state to avoid cross-network bleed
                    screen.app_context = app_context.clone();
                    screen.clear();
                } else {
                    screen.app_context = app_context;
                }
            }
            Screen::DocumentVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::PlatformInfoScreen(screen) => screen.app_context = app_context,
            Screen::GroveSTARKScreen(screen) => screen.app_context = app_context,
            Screen::AddressBalanceScreen(screen) => {
                screen.app_context = app_context;
                screen.invalidate_address_input();
            }

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
            Screen::PurchaseTokenScreen(screen) => screen.app_context = app_context,
            Screen::SetTokenPriceScreen(screen) => screen.app_context = app_context,
            Screen::AssetLockDetailScreen(screen) => screen.app_context = app_context,
            Screen::CreateAssetLockScreen(screen) => screen.app_context = app_context,

            // DashPay Screens
            Screen::DashPayScreen(screen) => {
                screen.app_context = app_context.clone();
                screen.contacts_list.app_context = app_context.clone();
                screen.contacts_list.contact_requests.app_context = app_context.clone();
                screen.profile_screen.app_context = app_context.clone();
                screen.payment_history.app_context = app_context;
            }
            Screen::DashPayAddContactScreen(screen) => screen.app_context = app_context,
            Screen::DashPayContactDetailsScreen(screen) => screen.app_context = app_context,
            Screen::DashPayContactProfileViewerScreen(screen) => screen.app_context = app_context,
            Screen::DashPaySendPaymentScreen(screen) => screen.app_context = app_context,
            Screen::DashPayContactInfoEditorScreen(screen) => screen.app_context = app_context,
            Screen::DashPayQRGeneratorScreen(screen) => screen.app_context = app_context,
            Screen::DashPayProfileSearchScreen(screen) => screen.app_context = app_context,
            // Shielded screens
            Screen::ShieldScreen(screen) => {
                screen.app_context = app_context.clone();
                screen.invalidate_address_input();
            }
            Screen::ShieldedSendScreen(screen) => {
                screen.app_context = app_context.clone();
                screen.invalidate_address_input();
            }
            Screen::UnshieldCreditsScreen(screen) => {
                screen.app_context = app_context.clone();
                screen.invalidate_address_input();
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MessageType {
    Success,
    Info,
    Warning,
    Error,
}

pub trait ScreenLike {
    fn refresh(&mut self) {}
    fn refresh_on_arrival(&mut self) {
        self.refresh()
    }
    fn ui(&mut self, ctx: &Context) -> AppAction;
    /// Called by `AppState` **after** the global banner has already been set.
    ///
    /// Override **only for side-effects** such as clearing a progress banner
    /// (`self.refresh_banner.take_and_clear()`) or updating an internal status enum.
    /// Do **not** set your own banner here — `AppState` already did that.
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}

    /// Called by `AppState` when a backend task completes successfully.
    ///
    /// Global success/error banners are handled centrally by `AppState::update()`.
    /// Override this to perform screen-specific side-effects (e.g., storing a
    /// result, transitioning status, clearing a progress banner).
    /// The default is a **no-op** — screens that dispatch backend tasks should
    /// override this for their expected result variants.
    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {}

    /// Called by `AppState` when a backend task fails with a typed error.
    ///
    /// Override to handle specific error variants (e.g., `CoreWalletNotConfigured`).
    /// Return `true` to suppress the default error banner in `AppState`.
    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        false
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
            Screen::ContractVisualizerScreen(_) => ScreenType::ContractsVisualizer,
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
            Screen::RegisterDpnsNameScreen(screen) => ScreenType::RegisterDpnsName(screen.source),
            Screen::RegisterDataContractScreen(_) => ScreenType::RegisterContract,
            Screen::UpdateDataContractScreen(_) => ScreenType::UpdateContract,
            Screen::DocumentActionScreen(screen) => match screen.action_type {
                DocumentActionType::Create => ScreenType::CreateDocument,
                DocumentActionType::Delete => ScreenType::DeleteDocument,
                DocumentActionType::Replace => ScreenType::ReplaceDocument,
                DocumentActionType::Transfer => ScreenType::TransferDocument,
                DocumentActionType::Purchase => ScreenType::PurchaseDocument,
                DocumentActionType::SetPrice => ScreenType::SetDocumentPrice,
            },
            Screen::GroupActionsScreen(_) => ScreenType::GroupActions,
            Screen::AddNewWalletScreen(_) => ScreenType::AddNewWallet,
            Screen::WalletsBalancesScreen(_) => ScreenType::WalletsBalances,
            Screen::ImportMnemonicScreen(_) => ScreenType::ImportMnemonic,
            Screen::WalletSendScreen(screen) => {
                ScreenType::WalletSendScreen(screen.selected_wallet.clone().unwrap())
            }
            Screen::SingleKeyWalletSendScreen(screen) => {
                ScreenType::SingleKeyWalletSendScreen(screen.selected_wallet.clone().unwrap())
            }
            Screen::ProofLogScreen(_) => ScreenType::ProofLog,
            Screen::AddContractsScreen(_) => ScreenType::AddContracts,
            Screen::ProofVisualizerScreen(_) => ScreenType::ProofVisualizer,
            Screen::MasternodeListDiffScreen(_) => ScreenType::MasternodeListDiff,
            Screen::DocumentVisualizerScreen(_) => ScreenType::DocumentsVisualizer,
            Screen::PlatformInfoScreen(_) => ScreenType::PlatformInfo,
            Screen::GroveSTARKScreen(_) => ScreenType::GroveSTARK,
            Screen::AddressBalanceScreen(_) => ScreenType::AddressBalance,

            // Token Screens
            Screen::TokensScreen(screen)
                if screen.tokens_subscreen == TokensSubscreen::MyTokens =>
            {
                ScreenType::TokenBalances
            }
            Screen::TokensScreen(screen)
                if screen.tokens_subscreen == TokensSubscreen::SearchTokens =>
            {
                ScreenType::TokenSearch
            }
            Screen::TokensScreen(screen)
                if screen.tokens_subscreen == TokensSubscreen::TokenCreator =>
            {
                ScreenType::TokenCreator
            }
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
                ScreenType::ClaimTokensScreen(screen.identity_token_basic_info.clone())
            }
            Screen::ViewTokenClaimsScreen(screen) => {
                ScreenType::ViewTokenClaimsScreen(screen.identity_token_basic_info.clone())
            }
            Screen::UpdateTokenConfigScreen(screen) => {
                ScreenType::UpdateTokenConfigScreen(screen.identity_token_info.clone())
            }
            Screen::AddTokenById(_) => ScreenType::AddTokenById,
            Screen::PurchaseTokenScreen(screen) => {
                ScreenType::PurchaseTokenScreen(screen.identity_token_info.clone())
            }
            Screen::SetTokenPriceScreen(screen) => {
                ScreenType::SetTokenPriceScreen(screen.identity_token_info.clone())
            }
            Screen::AssetLockDetailScreen(screen) => {
                ScreenType::AssetLockDetail(screen.wallet_seed_hash, screen.asset_lock_txid)
            }
            Screen::CreateAssetLockScreen(screen) => {
                ScreenType::CreateAssetLock(screen.wallet.clone())
            }
            Screen::TokensScreen(_) => {
                // Default fallback for any unmatched TokensScreen variants
                ScreenType::TokenBalances
            }

            // DashPay Screens
            Screen::DashPayScreen(screen) => match screen.dashpay_subscreen {
                DashPaySubscreen::Contacts => ScreenType::DashPayContacts,
                DashPaySubscreen::Profile => ScreenType::DashPayProfile,
                DashPaySubscreen::Payments => ScreenType::DashPayPayments,
                DashPaySubscreen::ProfileSearch => ScreenType::DashPayProfileSearch,
            },
            Screen::DashPayAddContactScreen(_) => ScreenType::DashPayAddContact,
            Screen::DashPayContactDetailsScreen(screen) => {
                ScreenType::DashPayContactDetails(screen.identity.clone(), screen.contact_id)
            }
            Screen::DashPayContactProfileViewerScreen(screen) => {
                ScreenType::DashPayContactProfileViewer(screen.identity.clone(), screen.contact_id)
            }
            Screen::DashPaySendPaymentScreen(screen) => {
                ScreenType::DashPaySendPayment(screen.from_identity.clone(), screen.to_contact_id)
            }
            Screen::DashPayContactInfoEditorScreen(screen) => {
                ScreenType::DashPayContactInfoEditor(screen.identity.clone(), screen.contact_id)
            }
            Screen::DashPayQRGeneratorScreen(_) => ScreenType::DashPayQRGenerator,
            Screen::DashPayProfileSearchScreen(_) => ScreenType::DashPayProfileSearch,
            // Shielded screens
            Screen::ShieldScreen(s) => ScreenType::ShieldScreen(s.seed_hash),
            Screen::ShieldedSendScreen(s) => ScreenType::ShieldedSendScreen(s.seed_hash),
            Screen::UnshieldCreditsScreen(s) => ScreenType::UnshieldCreditsScreen(s.seed_hash),
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
            Screen::ImportMnemonicScreen(screen) => screen.refresh(),
            Screen::AddNewIdentityScreen(screen) => screen.refresh(),
            Screen::TopUpIdentityScreen(screen) => screen.refresh(),
            Screen::AddExistingIdentityScreen(screen) => screen.refresh(),
            Screen::KeyInfoScreen(screen) => screen.refresh(),
            Screen::KeysScreen(screen) => screen.refresh(),
            Screen::RegisterDpnsNameScreen(screen) => screen.refresh(),
            Screen::RegisterDataContractScreen(screen) => screen.refresh(),
            Screen::UpdateDataContractScreen(screen) => screen.refresh(),
            Screen::DocumentActionScreen(screen) => screen.refresh(),
            Screen::GroupActionsScreen(screen) => screen.refresh(),
            Screen::WithdrawalScreen(screen) => screen.refresh(),
            Screen::TransferScreen(screen) => screen.refresh(),
            Screen::AddKeyScreen(screen) => screen.refresh(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh(),
            Screen::NetworkChooserScreen(screen) => screen.refresh(),
            Screen::WalletsBalancesScreen(screen) => screen.refresh(),
            Screen::WalletSendScreen(screen) => screen.refresh(),
            Screen::SingleKeyWalletSendScreen(screen) => screen.refresh(),
            Screen::ProofLogScreen(screen) => screen.refresh(),
            Screen::AddContractsScreen(screen) => screen.refresh(),
            Screen::ProofVisualizerScreen(screen) => screen.refresh(),
            Screen::MasternodeListDiffScreen(screen) => screen.refresh(),
            Screen::DocumentVisualizerScreen(screen) => screen.refresh(),
            Screen::ContractVisualizerScreen(screen) => screen.refresh(),
            Screen::PlatformInfoScreen(screen) => screen.refresh(),
            Screen::GroveSTARKScreen(screen) => screen.refresh(),
            Screen::AddressBalanceScreen(screen) => screen.refresh(),

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
            Screen::PurchaseTokenScreen(screen) => screen.refresh(),
            Screen::SetTokenPriceScreen(screen) => screen.refresh(),
            Screen::AssetLockDetailScreen(screen) => screen.refresh(),
            Screen::CreateAssetLockScreen(screen) => screen.refresh(),

            // DashPay Screens
            Screen::DashPayScreen(screen) => screen.refresh(),
            Screen::DashPayAddContactScreen(screen) => screen.refresh(),
            Screen::DashPayContactDetailsScreen(screen) => screen.refresh(),
            Screen::DashPayContactProfileViewerScreen(screen) => screen.refresh(),
            Screen::DashPaySendPaymentScreen(screen) => screen.refresh(),
            Screen::DashPayContactInfoEditorScreen(screen) => screen.refresh(),
            Screen::DashPayQRGeneratorScreen(_) => {}
            Screen::DashPayProfileSearchScreen(screen) => screen.refresh(),
            // Shielded screens
            Screen::ShieldScreen(screen) => screen.refresh(),
            Screen::ShieldedSendScreen(screen) => screen.refresh(),
            Screen::UnshieldCreditsScreen(screen) => screen.refresh(),
        }
    }

    fn refresh_on_arrival(&mut self) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.refresh_on_arrival(),
            Screen::DPNSScreen(screen) => screen.refresh_on_arrival(),
            Screen::DocumentQueryScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddNewWalletScreen(screen) => screen.refresh_on_arrival(),
            Screen::ImportMnemonicScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddNewIdentityScreen(screen) => screen.refresh_on_arrival(),
            Screen::TopUpIdentityScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddExistingIdentityScreen(screen) => screen.refresh_on_arrival(),
            Screen::KeyInfoScreen(screen) => screen.refresh_on_arrival(),
            Screen::KeysScreen(screen) => screen.refresh_on_arrival(),
            Screen::RegisterDpnsNameScreen(screen) => screen.refresh_on_arrival(),
            Screen::RegisterDataContractScreen(screen) => screen.refresh_on_arrival(),
            Screen::UpdateDataContractScreen(screen) => screen.refresh_on_arrival(),
            Screen::DocumentActionScreen(screen) => screen.refresh_on_arrival(),
            Screen::GroupActionsScreen(screen) => screen.refresh_on_arrival(),
            Screen::WithdrawalScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransferScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddKeyScreen(screen) => screen.refresh_on_arrival(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::NetworkChooserScreen(screen) => screen.refresh_on_arrival(),
            Screen::WalletsBalancesScreen(screen) => screen.refresh_on_arrival(),
            Screen::WalletSendScreen(screen) => screen.refresh_on_arrival(),
            Screen::SingleKeyWalletSendScreen(screen) => screen.refresh_on_arrival(),
            Screen::ProofLogScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddContractsScreen(screen) => screen.refresh_on_arrival(),
            Screen::ProofVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::MasternodeListDiffScreen(screen) => screen.refresh_on_arrival(),
            Screen::DocumentVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::ContractVisualizerScreen(screen) => screen.refresh_on_arrival(),
            Screen::PlatformInfoScreen(screen) => screen.refresh_on_arrival(),
            Screen::GroveSTARKScreen(screen) => screen.refresh_on_arrival(),
            Screen::AddressBalanceScreen(screen) => screen.refresh_on_arrival(),

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
            Screen::PurchaseTokenScreen(screen) => screen.refresh_on_arrival(),
            Screen::SetTokenPriceScreen(screen) => screen.refresh_on_arrival(),
            Screen::AssetLockDetailScreen(screen) => screen.refresh_on_arrival(),
            Screen::CreateAssetLockScreen(screen) => screen.refresh_on_arrival(),

            // DashPay Screens
            Screen::DashPayScreen(screen) => screen.refresh_on_arrival(),
            Screen::DashPayAddContactScreen(screen) => screen.refresh_on_arrival(),
            Screen::DashPayContactDetailsScreen(screen) => screen.refresh_on_arrival(),
            Screen::DashPayContactProfileViewerScreen(screen) => screen.refresh_on_arrival(),
            Screen::DashPaySendPaymentScreen(screen) => screen.refresh_on_arrival(),
            Screen::DashPayContactInfoEditorScreen(screen) => screen.refresh_on_arrival(),
            Screen::DashPayQRGeneratorScreen(_) => {}
            Screen::DashPayProfileSearchScreen(screen) => screen.refresh_on_arrival(),
            // Shielded screens
            Screen::ShieldScreen(screen) => screen.refresh_on_arrival(),
            Screen::ShieldedSendScreen(screen) => screen.refresh_on_arrival(),
            Screen::UnshieldCreditsScreen(screen) => screen.refresh_on_arrival(),
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        match self {
            Screen::IdentitiesScreen(screen) => screen.ui(ctx),
            Screen::DPNSScreen(screen) => screen.ui(ctx),
            Screen::DocumentQueryScreen(screen) => screen.ui(ctx),
            Screen::AddNewWalletScreen(screen) => screen.ui(ctx),
            Screen::ImportMnemonicScreen(screen) => screen.ui(ctx),
            Screen::AddNewIdentityScreen(screen) => screen.ui(ctx),
            Screen::TopUpIdentityScreen(screen) => screen.ui(ctx),
            Screen::AddExistingIdentityScreen(screen) => screen.ui(ctx),
            Screen::KeyInfoScreen(screen) => screen.ui(ctx),
            Screen::KeysScreen(screen) => screen.ui(ctx),
            Screen::RegisterDpnsNameScreen(screen) => screen.ui(ctx),
            Screen::RegisterDataContractScreen(screen) => screen.ui(ctx),
            Screen::UpdateDataContractScreen(screen) => screen.ui(ctx),
            Screen::DocumentActionScreen(screen) => screen.ui(ctx),
            Screen::GroupActionsScreen(screen) => screen.ui(ctx),
            Screen::WithdrawalScreen(screen) => screen.ui(ctx),
            Screen::TransferScreen(screen) => screen.ui(ctx),
            Screen::AddKeyScreen(screen) => screen.ui(ctx),
            Screen::TransitionVisualizerScreen(screen) => screen.ui(ctx),
            Screen::NetworkChooserScreen(screen) => screen.ui(ctx),
            Screen::WalletsBalancesScreen(screen) => screen.ui(ctx),
            Screen::WalletSendScreen(screen) => screen.ui(ctx),
            Screen::SingleKeyWalletSendScreen(screen) => screen.ui(ctx),
            Screen::ProofLogScreen(screen) => screen.ui(ctx),
            Screen::AddContractsScreen(screen) => screen.ui(ctx),
            Screen::ProofVisualizerScreen(screen) => screen.ui(ctx),
            Screen::MasternodeListDiffScreen(screen) => screen.ui(ctx),
            Screen::DocumentVisualizerScreen(screen) => screen.ui(ctx),
            Screen::ContractVisualizerScreen(screen) => screen.ui(ctx),
            Screen::PlatformInfoScreen(screen) => screen.ui(ctx),
            Screen::GroveSTARKScreen(screen) => screen.ui(ctx),
            Screen::AddressBalanceScreen(screen) => screen.ui(ctx),

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
            Screen::PurchaseTokenScreen(screen) => screen.ui(ctx),
            Screen::SetTokenPriceScreen(screen) => screen.ui(ctx),
            Screen::AssetLockDetailScreen(screen) => screen.ui(ctx),
            Screen::CreateAssetLockScreen(screen) => screen.ui(ctx),

            // DashPay Screens
            Screen::DashPayScreen(screen) => screen.ui(ctx),
            Screen::DashPayAddContactScreen(screen) => screen.ui(ctx),
            Screen::DashPayContactDetailsScreen(screen) => screen.ui(ctx),
            Screen::DashPayContactProfileViewerScreen(screen) => screen.ui(ctx),
            Screen::DashPaySendPaymentScreen(screen) => screen.ui(ctx),
            Screen::DashPayContactInfoEditorScreen(screen) => screen.ui(ctx),
            Screen::DashPayQRGeneratorScreen(screen) => screen.ui(ctx),
            Screen::DashPayProfileSearchScreen(screen) => screen.ui(ctx),
            // Shielded screens
            Screen::ShieldScreen(screen) => screen.ui(ctx),
            Screen::ShieldedSendScreen(screen) => screen.ui(ctx),
            Screen::UnshieldCreditsScreen(screen) => screen.ui(ctx),
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.display_message(message, message_type),
            Screen::DPNSScreen(screen) => screen.display_message(message, message_type),
            Screen::DocumentQueryScreen(screen) => screen.display_message(message, message_type),
            Screen::AddNewWalletScreen(screen) => screen.display_message(message, message_type),
            Screen::ImportMnemonicScreen(screen) => screen.display_message(message, message_type),
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
            Screen::UpdateDataContractScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DocumentActionScreen(screen) => screen.display_message(message, message_type),
            Screen::GroupActionsScreen(screen) => screen.display_message(message, message_type),
            Screen::WithdrawalScreen(screen) => screen.display_message(message, message_type),
            Screen::TransferScreen(screen) => screen.display_message(message, message_type),
            Screen::AddKeyScreen(screen) => screen.display_message(message, message_type),
            Screen::TransitionVisualizerScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::NetworkChooserScreen(screen) => screen.display_message(message, message_type),
            Screen::WalletsBalancesScreen(screen) => screen.display_message(message, message_type),
            Screen::WalletSendScreen(screen) => screen.display_message(message, message_type),
            Screen::SingleKeyWalletSendScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::ProofLogScreen(screen) => screen.display_message(message, message_type),
            Screen::AddContractsScreen(screen) => screen.display_message(message, message_type),
            Screen::ProofVisualizerScreen(screen) => screen.display_message(message, message_type),
            Screen::MasternodeListDiffScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DocumentVisualizerScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::ContractVisualizerScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::PlatformInfoScreen(screen) => screen.display_message(message, message_type),
            Screen::GroveSTARKScreen(screen) => screen.display_message(message, message_type),
            Screen::AddressBalanceScreen(screen) => screen.display_message(message, message_type),

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
            Screen::PurchaseTokenScreen(screen) => screen.display_message(message, message_type),
            Screen::SetTokenPriceScreen(screen) => screen.display_message(message, message_type),
            Screen::AssetLockDetailScreen(screen) => screen.display_message(message, message_type),
            Screen::CreateAssetLockScreen(screen) => screen.display_message(message, message_type),

            // DashPay Screens
            Screen::DashPayScreen(screen) => screen.display_message(message, message_type),
            Screen::DashPayAddContactScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DashPayContactDetailsScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DashPayContactProfileViewerScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DashPaySendPaymentScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DashPayContactInfoEditorScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DashPayQRGeneratorScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::DashPayProfileSearchScreen(screen) => {
                screen.display_message(message, message_type)
            }
            // Shielded screens
            Screen::ShieldScreen(screen) => screen.display_message(message, message_type),
            Screen::ShieldedSendScreen(screen) => screen.display_message(message, message_type),
            Screen::UnshieldCreditsScreen(screen) => screen.display_message(message, message_type),
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
            Screen::ImportMnemonicScreen(screen) => {
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
            Screen::UpdateDataContractScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DocumentActionScreen(screen) => {
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
            Screen::WalletSendScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::SingleKeyWalletSendScreen(screen) => {
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
            Screen::MasternodeListDiffScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::ContractVisualizerScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::PlatformInfoScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::GroveSTARKScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AddressBalanceScreen(screen) => {
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
            Screen::PurchaseTokenScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::SetTokenPriceScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::AssetLockDetailScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::CreateAssetLockScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }

            // DashPay Screens
            Screen::DashPayScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPayAddContactScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPayContactDetailsScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPayContactProfileViewerScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPaySendPaymentScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPayContactInfoEditorScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPayQRGeneratorScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::DashPayProfileSearchScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            // Shielded screens
            Screen::ShieldScreen(screen) => screen.display_task_result(backend_task_success_result),
            Screen::ShieldedSendScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
            Screen::UnshieldCreditsScreen(screen) => {
                screen.display_task_result(backend_task_success_result)
            }
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        match self {
            Screen::IdentitiesScreen(screen) => screen.display_task_error(error),
            Screen::DPNSScreen(screen) => screen.display_task_error(error),
            Screen::DocumentQueryScreen(screen) => screen.display_task_error(error),
            Screen::AddNewWalletScreen(screen) => screen.display_task_error(error),
            Screen::ImportMnemonicScreen(screen) => screen.display_task_error(error),
            Screen::AddNewIdentityScreen(screen) => screen.display_task_error(error),
            Screen::TopUpIdentityScreen(screen) => screen.display_task_error(error),
            Screen::AddExistingIdentityScreen(screen) => screen.display_task_error(error),
            Screen::KeyInfoScreen(screen) => screen.display_task_error(error),
            Screen::KeysScreen(screen) => screen.display_task_error(error),
            Screen::RegisterDpnsNameScreen(screen) => screen.display_task_error(error),
            Screen::RegisterDataContractScreen(screen) => screen.display_task_error(error),
            Screen::UpdateDataContractScreen(screen) => screen.display_task_error(error),
            Screen::DocumentActionScreen(screen) => screen.display_task_error(error),
            Screen::GroupActionsScreen(screen) => screen.display_task_error(error),
            Screen::WithdrawalScreen(screen) => screen.display_task_error(error),
            Screen::TransferScreen(screen) => screen.display_task_error(error),
            Screen::AddKeyScreen(screen) => screen.display_task_error(error),
            Screen::TransitionVisualizerScreen(screen) => screen.display_task_error(error),
            Screen::NetworkChooserScreen(screen) => screen.display_task_error(error),
            Screen::WalletsBalancesScreen(screen) => screen.display_task_error(error),
            Screen::WalletSendScreen(screen) => screen.display_task_error(error),
            Screen::SingleKeyWalletSendScreen(screen) => screen.display_task_error(error),
            Screen::ProofLogScreen(screen) => screen.display_task_error(error),
            Screen::AddContractsScreen(screen) => screen.display_task_error(error),
            Screen::ProofVisualizerScreen(screen) => screen.display_task_error(error),
            Screen::MasternodeListDiffScreen(screen) => screen.display_task_error(error),
            Screen::DocumentVisualizerScreen(screen) => screen.display_task_error(error),
            Screen::ContractVisualizerScreen(screen) => screen.display_task_error(error),
            Screen::PlatformInfoScreen(screen) => screen.display_task_error(error),
            Screen::GroveSTARKScreen(screen) => screen.display_task_error(error),
            Screen::AddressBalanceScreen(screen) => screen.display_task_error(error),

            // Token Screens
            Screen::TokensScreen(screen) => screen.display_task_error(error),
            Screen::TransferTokensScreen(screen) => screen.display_task_error(error),
            Screen::MintTokensScreen(screen) => screen.display_task_error(error),
            Screen::BurnTokensScreen(screen) => screen.display_task_error(error),
            Screen::DestroyFrozenFundsScreen(screen) => screen.display_task_error(error),
            Screen::FreezeTokensScreen(screen) => screen.display_task_error(error),
            Screen::UnfreezeTokensScreen(screen) => screen.display_task_error(error),
            Screen::PauseTokensScreen(screen) => screen.display_task_error(error),
            Screen::ResumeTokensScreen(screen) => screen.display_task_error(error),
            Screen::ClaimTokensScreen(screen) => screen.display_task_error(error),
            Screen::ViewTokenClaimsScreen(screen) => screen.display_task_error(error),
            Screen::UpdateTokenConfigScreen(screen) => screen.display_task_error(error),
            Screen::AddTokenById(screen) => screen.display_task_error(error),
            Screen::PurchaseTokenScreen(screen) => screen.display_task_error(error),
            Screen::SetTokenPriceScreen(screen) => screen.display_task_error(error),
            Screen::AssetLockDetailScreen(screen) => screen.display_task_error(error),
            Screen::CreateAssetLockScreen(screen) => screen.display_task_error(error),

            // DashPay Screens
            Screen::DashPayScreen(screen) => screen.display_task_error(error),
            Screen::DashPayAddContactScreen(screen) => screen.display_task_error(error),
            Screen::DashPayContactDetailsScreen(screen) => screen.display_task_error(error),
            Screen::DashPayContactProfileViewerScreen(screen) => screen.display_task_error(error),
            Screen::DashPaySendPaymentScreen(screen) => screen.display_task_error(error),
            Screen::DashPayContactInfoEditorScreen(screen) => screen.display_task_error(error),
            Screen::DashPayQRGeneratorScreen(screen) => screen.display_task_error(error),
            Screen::DashPayProfileSearchScreen(screen) => screen.display_task_error(error),

            // Shielded Screens
            Screen::ShieldScreen(screen) => screen.display_task_error(error),
            Screen::ShieldedSendScreen(screen) => screen.display_task_error(error),
            Screen::UnshieldCreditsScreen(screen) => screen.display_task_error(error),
        }
    }

    fn pop_on_success(&mut self) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.pop_on_success(),
            Screen::DPNSScreen(screen) => screen.pop_on_success(),
            Screen::DocumentQueryScreen(screen) => screen.pop_on_success(),
            Screen::AddNewWalletScreen(screen) => screen.pop_on_success(),
            Screen::ImportMnemonicScreen(screen) => screen.pop_on_success(),
            Screen::AddNewIdentityScreen(screen) => screen.pop_on_success(),
            Screen::TopUpIdentityScreen(screen) => screen.pop_on_success(),
            Screen::AddExistingIdentityScreen(screen) => screen.pop_on_success(),
            Screen::KeyInfoScreen(screen) => screen.pop_on_success(),
            Screen::KeysScreen(screen) => screen.pop_on_success(),
            Screen::RegisterDpnsNameScreen(screen) => screen.pop_on_success(),
            Screen::RegisterDataContractScreen(screen) => screen.pop_on_success(),
            Screen::UpdateDataContractScreen(screen) => screen.pop_on_success(),
            Screen::DocumentActionScreen(screen) => screen.pop_on_success(),
            Screen::GroupActionsScreen(screen) => screen.pop_on_success(),
            Screen::WithdrawalScreen(screen) => screen.pop_on_success(),
            Screen::TransferScreen(screen) => screen.pop_on_success(),
            Screen::AddKeyScreen(screen) => screen.pop_on_success(),
            Screen::TransitionVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::NetworkChooserScreen(screen) => screen.pop_on_success(),
            Screen::WalletsBalancesScreen(screen) => screen.pop_on_success(),
            Screen::WalletSendScreen(screen) => screen.pop_on_success(),
            Screen::SingleKeyWalletSendScreen(screen) => screen.pop_on_success(),
            Screen::ProofLogScreen(screen) => screen.pop_on_success(),
            Screen::AddContractsScreen(screen) => screen.pop_on_success(),
            Screen::ProofVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::MasternodeListDiffScreen(screen) => screen.pop_on_success(),
            Screen::DocumentVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::ContractVisualizerScreen(screen) => screen.pop_on_success(),
            Screen::PlatformInfoScreen(screen) => screen.pop_on_success(),
            Screen::GroveSTARKScreen(screen) => screen.pop_on_success(),
            Screen::AddressBalanceScreen(screen) => screen.pop_on_success(),

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
            Screen::PurchaseTokenScreen(screen) => screen.pop_on_success(),
            Screen::SetTokenPriceScreen(screen) => screen.pop_on_success(),
            Screen::AssetLockDetailScreen(screen) => screen.pop_on_success(),
            Screen::CreateAssetLockScreen(screen) => screen.pop_on_success(),

            // DashPay Screens
            Screen::DashPayScreen(screen) => screen.pop_on_success(),
            Screen::DashPayAddContactScreen(_) => {}
            Screen::DashPayContactDetailsScreen(_) => {}
            Screen::DashPayContactProfileViewerScreen(_) => {}
            Screen::DashPaySendPaymentScreen(_) => {}
            Screen::DashPayContactInfoEditorScreen(_) => {}
            Screen::DashPayQRGeneratorScreen(_) => {}
            Screen::DashPayProfileSearchScreen(_) => {}
            // Shielded screens
            Screen::ShieldScreen(_) => {}
            Screen::ShieldedSendScreen(_) => {}
            Screen::UnshieldCreditsScreen(_) => {}
        }
    }
}
