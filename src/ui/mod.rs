use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::wallet::Wallet;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen;
use crate::ui::contracts_documents::document_action_screen::{
    DocumentActionScreen, DocumentActionType,
};
use crate::ui::dashpay::add_contact_screen::AddContactScreen;
use crate::ui::dashpay::contact_details::ContactDetailsScreen;
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
use crate::ui::tools::platform_info_screen::PlatformInfoScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::wallets::asset_lock_detail_screen::AssetLockDetailScreen;
use crate::ui::wallets::create_asset_lock_screen::CreateAssetLockScreen;
use crate::ui::wallets::import_mnemonic_screen::ImportMnemonicScreen;
use crate::ui::wallets::send_screen::{SendFlow, WalletSendScreen};
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
use identities::add_existing_identity_screen::AddExistingIdentityScreen;
use identities::add_new_identity_screen::AddNewIdentityScreen;
use identities::identities_screen::IdentitiesScreen;
use identities::register_dpns_name_screen::{RegisterDpnsNameScreen, RegisterDpnsNameSource};
use identity::IdentityHubScreen;
use std::fmt;
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

pub mod components;
pub mod contracts_documents;
pub mod dashpay;
pub mod dpns;
pub mod helpers;
pub mod identities;
pub mod identity;
pub mod network_chooser_screen;
pub mod state;
pub mod theme;
pub mod tokens;
pub mod tools;
pub mod wallets;
pub mod welcome_screen;

pub use crate::model::settings::RootScreenType;

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
            RootScreenType::RootScreenDPNSScheduledVotes => ScreenType::ScheduledVotes,
            RootScreenType::RootScreenToolsProofVisualizerScreen => ScreenType::ProofVisualizer,
            RootScreenType::RootScreenMyTokenBalances => ScreenType::TokenBalances,
            RootScreenType::RootScreenTokenSearch => ScreenType::TokenSearch,
            RootScreenType::RootScreenTokenCreator => ScreenType::TokenCreator,
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
            RootScreenType::RootScreenIdentityHub => ScreenType::IdentityHub,
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
    WalletSendScreen(Arc<RwLock<Wallet>>, SendFlow),
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
    /// Unified Identities hub (new four-tab section).
    IdentityHub,
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
    AssetLockDetail([u8; 32], dash_sdk::dpp::dashcore::OutPoint),
    CreateAssetLock(Arc<RwLock<Wallet>>),

    // DashPay Screens
    DashPayContacts,
    DashPayProfile,
    DashPayPayments,
    DashPayAddContact,
    DashPayAddContactWithId(String), // Pre-populated identity ID
    DashPayContactDetails(QualifiedIdentity, Identifier),
    DashPayContactProfileViewer(QualifiedIdentity, Identifier),
    DashPaySendPayment(QualifiedIdentity, Identifier),
    DashPayQRGenerator,
    DashPayProfileSearch,
}

impl PartialEq for ScreenType {
    fn eq(&self, other: &Self) -> bool {
        use ScreenType::*;
        match (self, other) {
            // Variants whose payload participates in equality.
            (AssetLockDetail(a1, a2), AssetLockDetail(b1, b2)) => a1 == b1 && a2 == b2,
            (WithdrawalScreen(a), WithdrawalScreen(b)) => a == b,
            (TransferScreen(a), TransferScreen(b)) => a == b,
            (AddKeyScreen(a), AddKeyScreen(b)) => a == b,
            (KeyInfo(a1, a2, a3), KeyInfo(b1, b2, b3)) => a1 == b1 && a2 == b2 && a3 == b3,
            (Keys(a), Keys(b)) => a == b,
            (RegisterDpnsName(a), RegisterDpnsName(b)) => a == b,
            (TopUpIdentity(a), TopUpIdentity(b)) => a == b,
            (TransferTokensScreen(a), TransferTokensScreen(b)) => a == b,
            (MintTokensScreen(a), MintTokensScreen(b)) => a == b,
            (BurnTokensScreen(a), BurnTokensScreen(b)) => a == b,
            (DestroyFrozenFundsScreen(a), DestroyFrozenFundsScreen(b)) => a == b,
            (FreezeTokensScreen(a), FreezeTokensScreen(b)) => a == b,
            (UnfreezeTokensScreen(a), UnfreezeTokensScreen(b)) => a == b,
            (PauseTokensScreen(a), PauseTokensScreen(b)) => a == b,
            (ResumeTokensScreen(a), ResumeTokensScreen(b)) => a == b,
            (ClaimTokensScreen(a), ClaimTokensScreen(b)) => a == b,
            (ViewTokenClaimsScreen(a), ViewTokenClaimsScreen(b)) => a == b,
            (UpdateTokenConfigScreen(a), UpdateTokenConfigScreen(b)) => a == b,
            (PurchaseTokenScreen(a), PurchaseTokenScreen(b)) => a == b,
            (SetTokenPriceScreen(a), SetTokenPriceScreen(b)) => a == b,
            (DashPayAddContactWithId(a), DashPayAddContactWithId(b)) => a == b,
            (DashPayContactDetails(a1, a2), DashPayContactDetails(b1, b2)) => a1 == b1 && a2 == b2,
            (DashPayContactProfileViewer(a1, a2), DashPayContactProfileViewer(b1, b2)) => {
                a1 == b1 && a2 == b2
            }
            (DashPaySendPayment(a1, a2), DashPaySendPayment(b1, b2)) => a1 == b1 && a2 == b2,
            // The send screen's wallet payload is intentionally ignored, but the
            // flow preset distinguishes Shield / Send-Private / Unshield routes
            // so pushing one does not dedup against another.
            (WalletSendScreen(_, fa), WalletSendScreen(_, fb)) => fa == fb,
            // All other variants are equal iff they share a discriminant. This covers the
            // fieldless variants and the wallet screens (SingleKeyWalletSendScreen /
            // CreateAssetLock), whose Arc<RwLock<…>> payload is intentionally ignored.
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
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
            ScreenType::WalletSendScreen(wallet, flow) => Screen::WalletSendScreen(
                WalletSendScreen::new(app_context, wallet.clone()).with_flow(*flow),
            ),
            ScreenType::SingleKeyWalletSendScreen(wallet) => Screen::SingleKeyWalletSendScreen(
                SingleKeyWalletSendScreen::new(app_context, wallet.clone()),
            ),
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
            ScreenType::IdentityHub => {
                Screen::IdentityHubScreen(IdentityHubScreen::new(app_context))
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
            ScreenType::AddTokenById => Screen::AddTokenById(AddTokenByIdScreen::new(app_context)),
            ScreenType::PurchaseTokenScreen(identity_token_info) => Screen::PurchaseTokenScreen(
                PurchaseTokenScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::SetTokenPriceScreen(identity_token_info) => Screen::SetTokenPriceScreen(
                SetTokenPriceScreen::new(identity_token_info.clone(), app_context),
            ),
            ScreenType::AssetLockDetail(wallet_seed_hash, out_point) => {
                Screen::AssetLockDetailScreen(AssetLockDetailScreen::new(
                    *wallet_seed_hash,
                    *out_point,
                    app_context,
                ))
            }
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
            ScreenType::DashPayQRGenerator => {
                Screen::DashPayQRGeneratorScreen(QRCodeGeneratorScreen::new(app_context.clone()))
            }
            ScreenType::DashPayProfileSearch => {
                Screen::DashPayProfileSearchScreen(ProfileSearchScreen::new(app_context.clone()))
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
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    DocumentVisualizerScreen(DocumentVisualizerScreen),
    ContractVisualizerScreen(ContractVisualizerScreen),
    NetworkChooserScreen(NetworkChooserScreen),
    WalletsBalancesScreen(WalletsBalancesScreen),
    WalletSendScreen(WalletSendScreen),
    SingleKeyWalletSendScreen(SingleKeyWalletSendScreen),
    AddContractsScreen(AddContractsScreen),
    ProofVisualizerScreen(ProofVisualizerScreen),
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

    // DashPay Screens
    DashPayScreen(DashPayScreen),
    DashPayAddContactScreen(AddContactScreen),
    DashPayContactDetailsScreen(ContactDetailsScreen),
    DashPayContactProfileViewerScreen(ContactProfileViewerScreen),
    DashPaySendPaymentScreen(SendPaymentScreen),
    DashPayQRGeneratorScreen(QRCodeGeneratorScreen),
    DashPayProfileSearchScreen(ProfileSearchScreen),

    // New unified Identities hub
    IdentityHubScreen(IdentityHubScreen),
}

impl Screen {
    pub fn change_context(&mut self, app_context: Arc<AppContext>) {
        /// Assigns `app_context` for the majority of screen variants that simply
        /// store it as a field.  Only screens with additional side-effects are
        /// handled in the explicit match arms below.
        ///
        /// Every `Screen` variant must appear in exactly one of the two lists
        /// (`set` or `skip`) so the compiler catches new additions.
        macro_rules! set_ctx {
            (set: $($variant:ident),+ $(,)?; common_set: $($cvariant:ident),* $(,)?; skip: $($skip:ident),* $(,)?) => {
                match self {
                    $(Screen::$variant(screen) => screen.app_context = app_context,)+
                    // Token action screens keep their context under `.common`.
                    $(Screen::$cvariant(screen) => screen.set_app_context(app_context),)*
                    // Handled by the explicit match above (side-effects + return).
                    $(Screen::$skip(_) => {},)*
                }
            }
        }

        // Screens with side-effects on context change are handled first.
        // Everything else falls through to the macro default assignment.
        match self {
            Screen::NetworkChooserScreen(screen) => {
                let network = app_context.network;
                screen.network_contexts.insert(network, app_context);
                screen.current_network = network;
                return;
            }
            Screen::AddNewWalletScreen(screen) => {
                screen.app_context = app_context;
                return;
            }
            Screen::TransferScreen(screen) => {
                screen.app_context = app_context;
                screen.invalidate_address_input();
                return;
            }
            Screen::WalletsBalancesScreen(screen) => {
                screen.app_context = app_context;
                screen.update_selected_wallet_for_network();
                screen.invalidate_address_inputs();
                screen.reset_transient_state();
                return;
            }
            Screen::ImportMnemonicScreen(screen) => {
                screen.app_context = app_context;
                return;
            }
            Screen::WalletSendScreen(screen) => {
                screen.app_context = app_context;
                // Drop all state bound to the old network's wallet (wallet, seed
                // hash, source/destination/amount) so a preset flow cannot show a
                // stale cross-network balance.
                screen.reset_for_network_switch();
                return;
            }
            Screen::SingleKeyWalletSendScreen(screen) => {
                screen.app_context = app_context;
                // Clear wallet reference — it belongs to the old network
                screen.selected_wallet = None;
                return;
            }
            Screen::CreateAssetLockScreen(screen) => {
                screen.app_context = app_context;
                // Clear wallet reference — it belongs to the old network
                screen.selected_wallet = None;
                return;
            }
            Screen::AddressBalanceScreen(screen) => {
                screen.app_context = app_context;
                screen.invalidate_address_input();
                return;
            }
            Screen::DashPayScreen(screen) => {
                screen.app_context = app_context.clone();
                screen.contacts_list.app_context = app_context.clone();
                screen.contacts_list.contact_requests.app_context = app_context.clone();
                screen.profile_screen.app_context = app_context.clone();
                screen.payment_history.app_context = app_context;
                return;
            }
            Screen::IdentityHubScreen(screen) => {
                screen.app_context = app_context;
                // A network switch invalidates all per-identity caches (contacts
                // load guard, profile cache, search state). Without this refresh
                // the Contacts tab would stay permanently "already loaded" after
                // switching networks (T28).
                screen.refresh();
                return;
            }
            _ => {}
        }

        // Simple context assignment for all remaining screens.
        // The `skip` list must exactly match the explicit match arms above.
        set_ctx!(
            set:
            IdentitiesScreen,
            DPNSScreen,
            AddExistingIdentityScreen,
            KeyInfoScreen,
            KeysScreen,
            WithdrawalScreen,
            TransitionVisualizerScreen,
            ContractVisualizerScreen,
            AddKeyScreen,
            DocumentQueryScreen,
            AddNewIdentityScreen,
            RegisterDpnsNameScreen,
            RegisterDataContractScreen,
            UpdateDataContractScreen,
            DocumentActionScreen,
            GroupActionsScreen,
            TopUpIdentityScreen,
            AddContractsScreen,
            ProofVisualizerScreen,
            DocumentVisualizerScreen,
            PlatformInfoScreen,
            GroveSTARKScreen,
            TokensScreen,
            TransferTokensScreen,
            ClaimTokensScreen,
            ViewTokenClaimsScreen,
            UpdateTokenConfigScreen,
            AddTokenById,
            PurchaseTokenScreen,
            SetTokenPriceScreen,
            AssetLockDetailScreen,
            DashPayAddContactScreen,
            DashPayContactDetailsScreen,
            DashPayContactProfileViewerScreen,
            DashPaySendPaymentScreen,
            DashPayQRGeneratorScreen,
            DashPayProfileSearchScreen;
            common_set:
            MintTokensScreen,
            BurnTokensScreen,
            DestroyFrozenFundsScreen,
            FreezeTokensScreen,
            UnfreezeTokensScreen,
            PauseTokensScreen,
            ResumeTokensScreen;
            skip:
            NetworkChooserScreen,
            AddNewWalletScreen,
            TransferScreen,
            WalletsBalancesScreen,
            ImportMnemonicScreen,
            WalletSendScreen,
            SingleKeyWalletSendScreen,
            CreateAssetLockScreen,
            AddressBalanceScreen,
            DashPayScreen,
            IdentityHubScreen,
        );
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
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction;
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
            Screen::WalletSendScreen(screen) => ScreenType::WalletSendScreen(
                screen
                    .selected_wallet
                    .clone()
                    .expect("invariant: a live WalletSendScreen always has a selected wallet"),
                screen.flow(),
            ),
            Screen::SingleKeyWalletSendScreen(screen) => {
                ScreenType::SingleKeyWalletSendScreen(screen.selected_wallet.clone().expect(
                    "invariant: a live SingleKeyWalletSendScreen always has a selected wallet",
                ))
            }
            Screen::AddContractsScreen(_) => ScreenType::AddContracts,
            Screen::ProofVisualizerScreen(_) => ScreenType::ProofVisualizer,
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
                ScreenType::MintTokensScreen(screen.common.identity_token_info.clone())
            }
            Screen::BurnTokensScreen(screen) => {
                ScreenType::BurnTokensScreen(screen.common.identity_token_info.clone())
            }
            Screen::DestroyFrozenFundsScreen(screen) => {
                ScreenType::DestroyFrozenFundsScreen(screen.common.identity_token_info.clone())
            }
            Screen::FreezeTokensScreen(screen) => {
                ScreenType::FreezeTokensScreen(screen.common.identity_token_info.clone())
            }
            Screen::UnfreezeTokensScreen(screen) => {
                ScreenType::UnfreezeTokensScreen(screen.common.identity_token_info.clone())
            }
            Screen::PauseTokensScreen(screen) => {
                ScreenType::PauseTokensScreen(screen.common.identity_token_info.clone())
            }
            Screen::ResumeTokensScreen(screen) => {
                ScreenType::ResumeTokensScreen(screen.common.identity_token_info.clone())
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
                ScreenType::AssetLockDetail(screen.wallet_seed_hash, screen.out_point)
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
            Screen::DashPayQRGeneratorScreen(_) => ScreenType::DashPayQRGenerator,
            Screen::DashPayProfileSearchScreen(_) => ScreenType::DashPayProfileSearch,
            Screen::IdentityHubScreen(_) => ScreenType::IdentityHub,
        }
    }
}

/// Delegates a [`ScreenLike`] call to the wrapped screen for every [`Screen`] variant.
///
/// The match is exhaustive, so adding a `Screen` variant without extending this list is a
/// compile error — every screen stays reachable through the trait.
macro_rules! delegate_to_screen {
    ($self:expr, $screen:ident => $call:expr) => {
        match $self {
            Screen::IdentitiesScreen($screen) => $call,
            Screen::DPNSScreen($screen) => $call,
            Screen::DocumentQueryScreen($screen) => $call,
            Screen::AddNewWalletScreen($screen) => $call,
            Screen::ImportMnemonicScreen($screen) => $call,
            Screen::AddNewIdentityScreen($screen) => $call,
            Screen::AddExistingIdentityScreen($screen) => $call,
            Screen::KeyInfoScreen($screen) => $call,
            Screen::KeysScreen($screen) => $call,
            Screen::RegisterDpnsNameScreen($screen) => $call,
            Screen::RegisterDataContractScreen($screen) => $call,
            Screen::UpdateDataContractScreen($screen) => $call,
            Screen::DocumentActionScreen($screen) => $call,
            Screen::GroupActionsScreen($screen) => $call,
            Screen::WithdrawalScreen($screen) => $call,
            Screen::TopUpIdentityScreen($screen) => $call,
            Screen::TransferScreen($screen) => $call,
            Screen::AddKeyScreen($screen) => $call,
            Screen::TransitionVisualizerScreen($screen) => $call,
            Screen::DocumentVisualizerScreen($screen) => $call,
            Screen::ContractVisualizerScreen($screen) => $call,
            Screen::NetworkChooserScreen($screen) => $call,
            Screen::WalletsBalancesScreen($screen) => $call,
            Screen::WalletSendScreen($screen) => $call,
            Screen::SingleKeyWalletSendScreen($screen) => $call,
            Screen::AddContractsScreen($screen) => $call,
            Screen::ProofVisualizerScreen($screen) => $call,
            Screen::PlatformInfoScreen($screen) => $call,
            Screen::GroveSTARKScreen($screen) => $call,
            Screen::AddressBalanceScreen($screen) => $call,
            Screen::TokensScreen($screen) => $call,
            Screen::TransferTokensScreen($screen) => $call,
            Screen::MintTokensScreen($screen) => $call,
            Screen::BurnTokensScreen($screen) => $call,
            Screen::DestroyFrozenFundsScreen($screen) => $call,
            Screen::FreezeTokensScreen($screen) => $call,
            Screen::UnfreezeTokensScreen($screen) => $call,
            Screen::PauseTokensScreen($screen) => $call,
            Screen::ResumeTokensScreen($screen) => $call,
            Screen::ClaimTokensScreen($screen) => $call,
            Screen::ViewTokenClaimsScreen($screen) => $call,
            Screen::UpdateTokenConfigScreen($screen) => $call,
            Screen::AddTokenById($screen) => $call,
            Screen::PurchaseTokenScreen($screen) => $call,
            Screen::SetTokenPriceScreen($screen) => $call,
            Screen::AssetLockDetailScreen($screen) => $call,
            Screen::CreateAssetLockScreen($screen) => $call,
            Screen::DashPayScreen($screen) => $call,
            Screen::DashPayAddContactScreen($screen) => $call,
            Screen::DashPayContactDetailsScreen($screen) => $call,
            Screen::DashPayContactProfileViewerScreen($screen) => $call,
            Screen::DashPaySendPaymentScreen($screen) => $call,
            Screen::DashPayQRGeneratorScreen($screen) => $call,
            Screen::DashPayProfileSearchScreen($screen) => $call,
            Screen::IdentityHubScreen($screen) => $call,
        }
    };
}

impl ScreenLike for Screen {
    fn refresh(&mut self) {
        delegate_to_screen!(self, screen => screen.refresh())
    }

    fn refresh_on_arrival(&mut self) {
        delegate_to_screen!(self, screen => screen.refresh_on_arrival())
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        delegate_to_screen!(self, screen => screen.ui(ui))
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        delegate_to_screen!(self, screen => screen.display_message(message, message_type))
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        delegate_to_screen!(self, screen => screen.display_task_result(backend_task_success_result))
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        delegate_to_screen!(self, screen => screen.display_task_error(error))
    }

    fn pop_on_success(&mut self) {
        delegate_to_screen!(self, screen => screen.pop_on_success())
    }
}
