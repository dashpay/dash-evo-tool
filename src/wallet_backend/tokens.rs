//! Token actions on [`WalletBackend`], sent through the upstream
//! `IdentityWallet` of the wallet that holds the acting identity.
//!
//! DET keeps choosing the signing key and the state-transition options, so
//! every action uses upstream's `token_*_with_signer` entry points with the
//! acting [`QualifiedIdentity`] as signer. Failures are classified by
//! [`TaskError::from_token_operation`], which names the operation.

use std::sync::Arc;

use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::tokens::transitions::{
    BurnResult, ClaimResult, ConfigUpdateResult, DestroyFrozenFundsResult, DirectPurchaseResult,
    EmergencyActionResult, FreezeResult, MintResult, SetPriceResult, TransferResult,
    UnfreezeResult,
};
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use platform_wallet::PlatformWallet;

use super::WalletBackend;
use crate::backend_task::error::{TaskError, TokenOperationKind};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::WalletSeedHash;

/// What every token action carries besides its own arguments.
pub(crate) struct TokenActionCommon<'a> {
    pub data_contract: Arc<DataContract>,
    pub token_position: TokenContractPosition,
    /// The key the user chose; Platform accepts only AUTHENTICATION/CRITICAL.
    pub signing_key: &'a IdentityPublicKey,
    /// `AppContext::state_transition_options` at the time of the action.
    pub options: Option<StateTransitionCreationOptions>,
}

/// An identity acting on tokens through the upstream wallet that holds it.
pub(crate) struct TokenActor<'a> {
    wallet: Arc<PlatformWallet>,
    identity: &'a QualifiedIdentity,
}

impl WalletBackend {
    /// The token actor for `identity`, held by the wallet `seed_hash` (its
    /// DET wallet link).
    ///
    /// # Errors
    /// [`TaskError::WalletNotLoaded`] / [`TaskError::WalletLockedNotLoaded`]
    /// when that wallet is not registered with the upstream manager.
    pub(crate) async fn token_actor<'a>(
        &self,
        seed_hash: &WalletSeedHash,
        identity: &'a QualifiedIdentity,
    ) -> Result<TokenActor<'a>, TaskError> {
        Ok(TokenActor {
            wallet: self.resolve_wallet(seed_hash).await?,
            identity,
        })
    }
}

impl TokenActor<'_> {
    fn id(&self) -> Identifier {
        self.identity.identity.id()
    }

    pub(crate) async fn mint(
        &self,
        common: TokenActionCommon<'_>,
        amount: TokenAmount,
        recipient: Option<Identifier>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<MintResult, TaskError> {
        self.wallet
            .identity()
            .token_mint_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                amount,
                recipient,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Mint, e))
    }

    pub(crate) async fn transfer(
        &self,
        common: TokenActionCommon<'_>,
        recipient: Identifier,
        amount: TokenAmount,
        public_note: Option<String>,
    ) -> Result<TransferResult, TaskError> {
        self.wallet
            .identity()
            .token_transfer_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                recipient,
                amount,
                common.signing_key,
                self.identity,
                public_note,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Transfer, e))
    }

    pub(crate) async fn burn(
        &self,
        common: TokenActionCommon<'_>,
        amount: TokenAmount,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<BurnResult, TaskError> {
        self.wallet
            .identity()
            .token_burn_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                amount,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Burn, e))
    }

    pub(crate) async fn freeze(
        &self,
        common: TokenActionCommon<'_>,
        target: Identifier,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<FreezeResult, TaskError> {
        self.wallet
            .identity()
            .token_freeze_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                target,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Freeze, e))
    }

    pub(crate) async fn unfreeze(
        &self,
        common: TokenActionCommon<'_>,
        target: Identifier,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<UnfreezeResult, TaskError> {
        self.wallet
            .identity()
            .token_unfreeze_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                target,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Unfreeze, e))
    }

    pub(crate) async fn destroy_frozen_funds(
        &self,
        common: TokenActionCommon<'_>,
        frozen_identity: Identifier,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<DestroyFrozenFundsResult, TaskError> {
        self.wallet
            .identity()
            .token_destroy_frozen_funds_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                frozen_identity,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::DestroyFrozenFunds, e))
    }

    pub(crate) async fn pause(
        &self,
        common: TokenActionCommon<'_>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<EmergencyActionResult, TaskError> {
        self.wallet
            .identity()
            .token_pause_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Pause, e))
    }

    pub(crate) async fn resume(
        &self,
        common: TokenActionCommon<'_>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<EmergencyActionResult, TaskError> {
        self.wallet
            .identity()
            .token_resume_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Resume, e))
    }

    pub(crate) async fn claim(
        &self,
        common: TokenActionCommon<'_>,
        distribution_type: TokenDistributionType,
        public_note: Option<String>,
    ) -> Result<ClaimResult, TaskError> {
        self.wallet
            .identity()
            .token_claim_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                distribution_type,
                common.signing_key,
                self.identity,
                public_note,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Claim, e))
    }

    pub(crate) async fn set_price(
        &self,
        common: TokenActionCommon<'_>,
        pricing_schedule: Option<TokenPricingSchedule>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<SetPriceResult, TaskError> {
        self.wallet
            .identity()
            .token_set_price_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                pricing_schedule,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::SetPrice, e))
    }

    pub(crate) async fn purchase(
        &self,
        common: TokenActionCommon<'_>,
        amount: TokenAmount,
        total_agreed_price: Credits,
    ) -> Result<DirectPurchaseResult, TaskError> {
        self.wallet
            .identity()
            .token_purchase_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                amount,
                total_agreed_price,
                common.signing_key,
                self.identity,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::Purchase, e))
    }

    // TODO(upstream): the SDK's ConfigUpdateResult requires a document for a
    // group action, so a group proposal proved without one surfaces as a proof
    // error although Platform accepted it; needs a backend-e2e group run.
    pub(crate) async fn update_config(
        &self,
        common: TokenActionCommon<'_>,
        change_item: TokenConfigurationChangeItem,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<ConfigUpdateResult, TaskError> {
        self.wallet
            .identity()
            .token_update_config_with_signer(
                common.data_contract,
                common.token_position,
                self.id(),
                change_item,
                common.signing_key,
                self.identity,
                public_note,
                group_info,
                common.options,
            )
            .await
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::ConfigUpdate, e))
    }
}
