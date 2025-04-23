//! Execute token query by keyword on Platform

use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use crate::{backend_task::BackendTaskSuccessResult, context::AppContext};
use dash_sdk::dpp::block::extended_epoch_info::v0::ExtendedEpochInfoV0Getters;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Accessors;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_moment::RewardDistributionMoment;
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dash_sdk::platform::query::TokenLastClaimQuery;
use dash_sdk::platform::{Fetch, Identifier};
use dash_sdk::Sdk;

impl AppContext {
    pub async fn query_token_non_claimed_perpetual_distribution_rewards(
        &self,
        identity_id: Identifier,
        token_id: Identifier,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        let query = TokenLastClaimQuery {
            identity_id,
            token_id,
        };

        // Fetch the last claim moment for the user
        let last_claim = RewardDistributionMoment::fetch(sdk, query)
            .await
            .map_err(|e| {
                format!("Failed to fetch token non claimed perpetual distribution rewards: {e}")
            })?;

        // Calculate how much the user has to claim based on the last time they claimed and the distribution function
        let token_config = self
            .db
            .get_token_config_for_id(&token_id, self)
            .map_err(|e| format!("Failed to get token config: {e}"))?
            .ok_or("Token config not found")?;
        let function = token_config
            .distribution_rules()
            .perpetual_distribution()
            .ok_or("Perpetual distribution function not found")?
            .distribution_type();

        // Get the current moment (block, time, or epoch)
        let current_epoch_with_metadata = ExtendedEpochInfo::fetch_current_with_metadata(sdk)
            .await
            .map_err(|e| format!("Failed to fetch current epoch: {e}"))?;
        let current_moment = match function.interval() {
            RewardDistributionMoment::BlockBasedMoment(_) => current_epoch_with_metadata.1.height,
            RewardDistributionMoment::TimeBasedMoment(_) => current_epoch_with_metadata.1.time_ms,
            RewardDistributionMoment::EpochBasedMoment(_) => {
                current_epoch_with_metadata.0.index().into()
            }
        };

        // Calculate how much the user has to claim based on the last time they claimed, the current time, and the distribution function
        match last_claim {
            Some(last_claim) => {
                let amount_to_claim = function
                    .function()
                    .evaluate(last_claim.into(), current_moment)
                    .map_err(|e| format!("Failed to evaluate distribution function: {e}"))?;

                Ok(
                    BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmount(
                        IdentityTokenIdentifier {
                            identity_id,
                            token_id,
                        },
                        amount_to_claim,
                    ),
                )
            }

            None => {
                let data_contract = self
                    .get_contract_by_token_id(&token_id)
                    .map_err(|e| format!("Failed to get data contract: {e}"))?
                    .ok_or("Data contract not found")?;
                let contract_creation_moment = function
                    .contract_creation_moment(&data_contract.contract)
                    .ok_or("Contract creation moment not found")?;
                let amount_to_claim = function
                    .function()
                    .evaluate(contract_creation_moment.into(), current_moment)
                    .map_err(|e| format!("Failed to evaluate distribution function: {e}"))?;

                Ok(
                    BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmount(
                        IdentityTokenIdentifier {
                            identity_id,
                            token_id,
                        },
                        amount_to_claim,
                    ),
                )
            }
        }
    }
}
