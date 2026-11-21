//! Token actions on [`WalletBackend`], sent through the upstream
//! `IdentityWallet` of the wallet that holds the acting identity, or through
//! the SDK for an identity no wallet in this app holds.
//!
//! DET keeps choosing the signing key and the state-transition options, so
//! every action uses upstream's `token_*_with_signer` entry points with the
//! acting [`QualifiedIdentity`] as signer. Failures are classified by
//! [`TaskError::from_token_operation`], which names the operation.

use std::sync::Arc;

use dash_sdk::Sdk;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::tokens::builders::{
    burn::TokenBurnTransitionBuilder, claim::TokenClaimTransitionBuilder,
    config_update::TokenConfigUpdateTransitionBuilder,
    destroy::TokenDestroyFrozenFundsTransitionBuilder,
    emergency_action::TokenEmergencyActionTransitionBuilder, freeze::TokenFreezeTransitionBuilder,
    mint::TokenMintTransitionBuilder, purchase::TokenDirectPurchaseTransitionBuilder,
    set_price::TokenChangeDirectPurchasePriceTransitionBuilder,
    transfer::TokenTransferTransitionBuilder, unfreeze::TokenUnfreezeTransitionBuilder,
};
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
use crate::model::token::MintRecipient;
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

/// Where a token action is sent.
enum TokenRoute {
    /// The upstream wallet that holds the identity.
    Wallet(Arc<PlatformWallet>),
    /// The SDK, for an identity no wallet in this app holds (masternode and
    /// key-only identities). Upstream's `token_*_with_signer` builds the same
    /// transitions and sends them through the same SDK calls.
    Sdk(Arc<Sdk>),
}

/// An identity acting on tokens.
pub(crate) struct TokenActor<'a> {
    route: TokenRoute,
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
            route: TokenRoute::Wallet(self.resolve_wallet(seed_hash).await?),
            identity,
        })
    }
}

/// Applies the optional public note, group info and creation options to a
/// token transition builder.
macro_rules! with_common {
    ($builder:expr $(, note: $note:expr)? $(, group: $group:expr)?, options: $options:expr) => {{
        let mut builder = $builder;
        $(if let Some(note) = $note {
            builder = builder.with_public_note(note);
        })?
        $(if let Some(group) = $group {
            builder = builder.with_using_group_info(group);
        })?
        if let Some(options) = $options {
            builder = builder.with_state_transition_creation_options(options);
        }
        builder
    }};
}

impl<'a> TokenActor<'a> {
    /// The token actor for an identity no wallet in this app holds; it signs
    /// with the identity's own keys through `sdk`.
    pub(crate) fn without_wallet(sdk: Arc<Sdk>, identity: &'a QualifiedIdentity) -> Self {
        TokenActor {
            route: TokenRoute::Sdk(sdk),
            identity,
        }
    }

    /// Whether the action goes through the upstream wallet holding the identity.
    #[cfg(test)]
    pub(crate) fn is_wallet_held(&self) -> bool {
        matches!(self.route, TokenRoute::Wallet(_))
    }

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
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => 'sdk: {
                // Platform refuses a named recipient when the token forbids
                // choosing one; upstream drops it the same way, and fails the
                // same way when the contract has no token at that position.
                let config = match common
                    .data_contract
                    .expected_token_configuration(common.token_position)
                {
                    Ok(config) => config,
                    Err(e) => break 'sdk Err(dash_sdk::Error::Protocol(e)),
                };
                let recipient = MintRecipient::resolve(config.distribution_rules(), recipient)
                    .transition_recipient();
                let builder = TokenMintTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    amount,
                );
                let builder = match recipient {
                    Some(recipient) => builder.issued_to_identity_id(recipient),
                    None => builder,
                };
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_mint(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Mint, e))
    }

    pub(crate) async fn transfer(
        &self,
        common: TokenActionCommon<'_>,
        recipient: Identifier,
        amount: TokenAmount,
        public_note: Option<String>,
    ) -> Result<TransferResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenTransferTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    recipient,
                    amount,
                );
                let builder = with_common!(builder, note: public_note, options: common.options);
                sdk.token_transfer(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Transfer, e))
    }

    pub(crate) async fn burn(
        &self,
        common: TokenActionCommon<'_>,
        amount: TokenAmount,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<BurnResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenBurnTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    amount,
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_burn(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Burn, e))
    }

    pub(crate) async fn freeze(
        &self,
        common: TokenActionCommon<'_>,
        target: Identifier,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<FreezeResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenFreezeTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    target,
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_freeze(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Freeze, e))
    }

    pub(crate) async fn unfreeze(
        &self,
        common: TokenActionCommon<'_>,
        target: Identifier,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<UnfreezeResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenUnfreezeTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    target,
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_unfreeze_identity(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Unfreeze, e))
    }

    pub(crate) async fn destroy_frozen_funds(
        &self,
        common: TokenActionCommon<'_>,
        frozen_identity: Identifier,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<DestroyFrozenFundsResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenDestroyFrozenFundsTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    frozen_identity,
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_destroy_frozen_funds(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result
            .map_err(|e| TaskError::from_token_operation(TokenOperationKind::DestroyFrozenFunds, e))
    }

    pub(crate) async fn pause(
        &self,
        common: TokenActionCommon<'_>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<EmergencyActionResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenEmergencyActionTransitionBuilder::pause(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_emergency_action(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Pause, e))
    }

    pub(crate) async fn resume(
        &self,
        common: TokenActionCommon<'_>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<EmergencyActionResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenEmergencyActionTransitionBuilder::resume(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_emergency_action(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Resume, e))
    }

    pub(crate) async fn claim(
        &self,
        common: TokenActionCommon<'_>,
        distribution_type: TokenDistributionType,
        public_note: Option<String>,
    ) -> Result<ClaimResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenClaimTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    distribution_type,
                );
                let builder = with_common!(builder, note: public_note, options: common.options);
                sdk.token_claim(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Claim, e))
    }

    pub(crate) async fn set_price(
        &self,
        common: TokenActionCommon<'_>,
        pricing_schedule: Option<TokenPricingSchedule>,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<SetPriceResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenChangeDirectPurchasePriceTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                );
                let builder = match pricing_schedule {
                    Some(schedule) => builder.with_token_pricing_schedule(schedule),
                    None => builder,
                };
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_set_price_for_direct_purchase(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::SetPrice, e))
    }

    pub(crate) async fn purchase(
        &self,
        common: TokenActionCommon<'_>,
        amount: TokenAmount,
        total_agreed_price: Credits,
    ) -> Result<DirectPurchaseResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenDirectPurchaseTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    amount,
                    total_agreed_price,
                );
                let builder = with_common!(builder, options: common.options);
                sdk.token_purchase(builder, common.signing_key, self.identity)
                    .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::Purchase, e))
    }

    pub(crate) async fn update_config(
        &self,
        common: TokenActionCommon<'_>,
        change_item: TokenConfigurationChangeItem,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Result<ConfigUpdateResult, TaskError> {
        let result = match &self.route {
            TokenRoute::Wallet(wallet) => {
                wallet
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
            }
            TokenRoute::Sdk(sdk) => {
                let builder = TokenConfigUpdateTransitionBuilder::new(
                    common.data_contract,
                    common.token_position,
                    self.id(),
                    change_item,
                );
                let builder = with_common!(builder, note: public_note, group: group_info, options: common.options);
                sdk.token_update_contract_token_configuration(
                    builder,
                    common.signing_key,
                    self.identity,
                )
                .await
            }
        };
        result.map_err(|e| TaskError::from_token_operation(TokenOperationKind::ConfigUpdate, e))
    }
}
