//! Execute token query by keyword on Platform

use crate::model::qualified_identity::IdentityType;
use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use crate::{backend_task::BackendTaskSuccessResult, context::AppContext};
use dash_sdk::{platform::Identifier, Sdk};
use dash_sdk::dpp::block::epoch::EpochIndex;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::block::extended_epoch_info::v0::ExtendedEpochInfoV0Getters;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::reward_ratio::RewardRatio;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::{TokenPerpetualDistributionV0Accessors, TokenPerpetualDistributionV0Methods};
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_moment::RewardDistributionMoment;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Fetch;
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dash_sdk::platform::query::TokenLastClaimQuery;

impl AppContext {
    fn validate_perpetual_distribution_recipient(
        &self,
        contract_owner_id: Identifier,
        recipient: TokenDistributionRecipient,
        identity_id: Identifier,
    ) -> Result<(), String> {
        match recipient {
            TokenDistributionRecipient::ContractOwner => {
                if contract_owner_id != identity_id {
                    Err("This token's distribution recipient is the contract owner, and this identity is not the contract owner".to_string())
                } else {
                    Ok(())
                }
            }
            TokenDistributionRecipient::Identity(identifier) => {
                if identifier != identity_id {
                    Err(
                        "This identity is not a valid distribution recipient for this token"
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
            TokenDistributionRecipient::EvonodesByParticipation => {
                // This validation method is not perfect because you can say an identity is an evonode even if it's not when loading identities
                let qualified_identities = self
                    .load_local_qualified_identities()
                    .map_err(|e| format!("Failed to load local qualified identities: {e}"))?;
                let qi = qualified_identities
                    .iter()
                    .find(|identity| identity.identity.id() == identity_id)
                    .ok_or("Identity not found in local database")?;
                if qi.identity_type != IdentityType::Evonode {
                    Err("This token's distribution recipient is EvonodesByParticipation, and this identity is not an evonode".to_string())
                } else {
                    Ok(())
                }
            }
        }
    }
    pub async fn query_token_non_claimed_perpetual_distribution_rewards(
        &self,
        identity_id: Identifier,
        token_id: Identifier,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // it may not be as simple as calculating the amount distributed since last claim
        // what if the recipient has changed? for example, since this identity's last claim,
        // the recipient changed to a different identity for half the time, they claimed the rewards,
        // now we are calculating since last claim but the amount is actually only half of what we calculate.

        let token_config = self
            .db
            .get_token_config_for_id(&token_id, self)
            .map_err(|e| format!("Failed to get token config from local database: {e}"))?
            .ok_or("Corrupted DET state: Token config not found in database")?;
        let perpetual_distribution = token_config
            .distribution_rules()
            .perpetual_distribution()
            .ok_or("Token doesn't have perpetual distribution")?;
        let data_contract = self
            .get_contract_by_token_id(&token_id)
            .map_err(|e| format!("Failed to get data contract from local database: {e}"))?
            .ok_or("Corrupted DET state: Data contract not found in database")?;

        let recipient = perpetual_distribution.distribution_recipient();
        self.validate_perpetual_distribution_recipient(
            data_contract.contract.owner_id(),
            recipient,
            identity_id,
        )?;

        let reward_distribution_type = perpetual_distribution.distribution_type();

        let query = TokenLastClaimQuery {
            identity_id,
            token_id,
        };

        // Fetch the last claim moment for the user
        let last_claim = RewardDistributionMoment::fetch(sdk, query)
            .await
            .map_err(|e| {
                format!("Failed to fetch token non claimed perpetual distribution rewards from Platform: {e}")
            })?;

        let contract_creation_moment = perpetual_distribution
            .distribution_type()
            .contract_creation_moment(&data_contract.contract)
            .ok_or("Calculation error: Contract does not have a start moment".to_string())?;

        let contract_creation_cycle_start = contract_creation_moment
            .cycle_start(perpetual_distribution.distribution_type().interval())
            .map_err(|e| format!("Failed to calculate estimated rewards: {e}"))?;

        let start_from_moment_for_distribution =
            last_claim.unwrap_or(contract_creation_cycle_start);

        // Calculate how much the user has to claim based on the last time they claimed and the distribution function
        // Get the current moment (block, time, or epoch)
        let current_epoch_with_metadata = ExtendedEpochInfo::fetch_current_with_metadata(sdk)
            .await
            .map_err(|e| {
                format!(
                    "Failed to fetch current epoch from Platform, required for calculation: {e}"
                )
            })?;

        let block_info = dash_sdk::dpp::block::block_info::BlockInfo {
            time_ms: current_epoch_with_metadata.1.time_ms,
            height: current_epoch_with_metadata.1.height,
            core_height: current_epoch_with_metadata.1.core_chain_locked_height, // This will not matter
            epoch: current_epoch_with_metadata
                .0
                .index()
                .try_into()
                .expect("we should never get back an epoch so far in the future"),
        };

        let current_cycle_moment = perpetual_distribution.current_interval(&block_info);

        // We need to get the max cycles allowed
        let max_cycles = self
            .platform_version
            .system_limits
            .max_token_redemption_cycles;
        let max_cycle_moment = perpetual_distribution
            .distribution_type()
            .max_cycle_moment(
                start_from_moment_for_distribution,
                current_cycle_moment,
                max_cycles,
            )
            .map_err(|e| format!("Failed to calculate estimated rewards: {e}"))?;

        let amount_to_claim = reward_distribution_type
            .rewards_in_interval::<fn(std::ops::RangeInclusive<EpochIndex>) -> Option<RewardRatio>>(
                contract_creation_cycle_start,
                start_from_moment_for_distribution,
                max_cycle_moment,
                None,
            )
            .map_err(|e| format!("Failed to calculate estimated rewards: {e}"))?;

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
