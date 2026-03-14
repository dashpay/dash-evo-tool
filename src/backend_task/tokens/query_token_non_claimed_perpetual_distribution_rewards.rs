//! Execute token query by keyword on Platform

use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::IdentityType;
use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use crate::{backend_task::BackendTaskSuccessResult, context::AppContext};
use dash_sdk::dpp::block::epoch::EpochIndex;
use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
use dash_sdk::dpp::block::extended_epoch_info::v0::ExtendedEpochInfoV0Getters;
use dash_sdk::dpp::block::finalized_epoch_info::FinalizedEpochInfo;
use dash_sdk::dpp::block::finalized_epoch_info::v0::getters::FinalizedEpochInfoGettersV0;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::reward_ratio::RewardRatio;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::{
    TokenPerpetualDistributionV0Accessors, TokenPerpetualDistributionV0Methods,
};
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_moment::RewardDistributionMoment;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dash_sdk::platform::query::TokenLastClaimQuery;
use dash_sdk::platform::{Fetch, FetchMany, Identifier};
use dash_sdk::Sdk;
use std::collections::BTreeMap;

impl AppContext {
    fn validate_perpetual_distribution_recipient(
        &self,
        contract_owner_id: Identifier,
        recipient: TokenDistributionRecipient,
        identity_id: Identifier,
    ) -> Result<(), TaskError> {
        match recipient {
            TokenDistributionRecipient::ContractOwner => {
                if contract_owner_id != identity_id {
                    Err(TaskError::NotTokenDistributionRecipient {
                        reason: "This token's distribution recipient is the contract owner, and this identity is not the contract owner",
                    })
                } else {
                    Ok(())
                }
            }
            TokenDistributionRecipient::Identity(identifier) => {
                if identifier != identity_id {
                    Err(TaskError::NotTokenDistributionRecipient {
                        reason: "This identity is not a valid distribution recipient for this token",
                    })
                } else {
                    Ok(())
                }
            }
            TokenDistributionRecipient::EvonodesByParticipation => {
                let qualified_identities = self.load_local_qualified_identities()?;
                let qi = qualified_identities
                    .iter()
                    .find(|identity| identity.identity.id() == identity_id)
                    .ok_or(TaskError::IdentityNotFoundLocally)?;
                if qi.identity_type != IdentityType::Evonode {
                    Err(TaskError::NotTokenDistributionRecipient {
                        reason: "This token's distribution recipient is EvonodesByParticipation, and this identity is not an evonode",
                    })
                } else {
                    Ok(())
                }
            }
        }
    }

    pub async fn query_token_non_claimed_perpetual_distribution_rewards_with_explanation(
        &self,
        identity_id: Identifier,
        token_id: Identifier,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let token_config = self.db.get_token_config_for_id(&token_id, self)?.ok_or(
            TaskError::TokenQueryError {
                detail: "Token config not found in database".to_string(),
            },
        )?;
        let perpetual_distribution = token_config
            .distribution_rules()
            .perpetual_distribution()
            .ok_or(TaskError::TokenQueryError {
                detail: "Token does not have perpetual distribution".to_string(),
            })?;
        let data_contract = self
            .get_contract_by_token_id(&token_id)?
            .ok_or(TaskError::DataContractNotFound)?;

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

        let last_claim = RewardDistributionMoment::fetch(sdk, query).await?;

        let contract_creation_moment = perpetual_distribution
            .distribution_type()
            .contract_creation_moment(&data_contract.contract)
            .ok_or(TaskError::TokenQueryError {
                detail: "Contract does not have a start moment".to_string(),
            })?;

        let contract_creation_cycle_start = contract_creation_moment
            .cycle_start(perpetual_distribution.distribution_type().interval())
            .map_err(|e| TaskError::TokenQueryError {
                detail: format!("Failed to calculate estimated rewards: {}", e),
            })?;

        let start_from_moment_for_distribution =
            last_claim.unwrap_or(contract_creation_cycle_start);

        let current_epoch_with_metadata = ExtendedEpochInfo::fetch_current_with_metadata(sdk)
            .await
            .map_err(|e| TaskError::PlatformInfoFetchError {
                source: Box::new(e),
            })?;

        let block_info = dash_sdk::dpp::block::block_info::BlockInfo {
            time_ms: current_epoch_with_metadata.1.time_ms,
            height: current_epoch_with_metadata.1.height,
            core_height: current_epoch_with_metadata.1.core_chain_locked_height,
            epoch: current_epoch_with_metadata
                .0
                .index()
                .try_into()
                .expect("we should never get back an epoch so far in the future"),
        };

        let current_cycle_moment = perpetual_distribution.current_interval(&block_info);

        let max_cycles = self
            .platform_version()
            .system_limits
            .max_token_redemption_cycles;
        let max_cycle_moment = perpetual_distribution
            .distribution_type()
            .max_cycle_moment(
                start_from_moment_for_distribution,
                current_cycle_moment,
                max_cycles,
            )
            .map_err(|e| TaskError::TokenQueryError {
                detail: format!("Failed to calculate estimated rewards: {}", e),
            })?;

        let explanation = if perpetual_distribution.distribution_recipient()
            == TokenDistributionRecipient::EvonodesByParticipation
        {
            let start_epoch = match start_from_moment_for_distribution {
                RewardDistributionMoment::EpochBasedMoment(epoch) => epoch,
                _ => {
                    return Err(TaskError::TokenQueryError {
                        detail: "Expected epoch-based moment for evonode participation rewards"
                            .to_string(),
                    });
                }
            };

            let current_epoch = match max_cycle_moment {
                RewardDistributionMoment::EpochBasedMoment(epoch) => epoch,
                _ => {
                    return Err(TaskError::TokenQueryError {
                        detail: "Expected epoch-based moment for evonode participation rewards"
                            .to_string(),
                    });
                }
            };

            let finalized_epoch_infos =
                FinalizedEpochInfo::fetch_many(sdk, (start_epoch, current_epoch)).await?;

            let mut blocks_per_epoch: BTreeMap<EpochIndex, u64> = BTreeMap::new();
            let mut evonode_blocks_per_epoch: BTreeMap<EpochIndex, u64> = BTreeMap::new();

            for (epoch_index, epoch_info) in finalized_epoch_infos {
                if let Some(epoch_info) = epoch_info {
                    blocks_per_epoch.insert(epoch_index, epoch_info.total_blocks_in_epoch());

                    let evonode_block_count = epoch_info
                        .block_proposers()
                        .get(&identity_id)
                        .cloned()
                        .unwrap_or_default();

                    evonode_blocks_per_epoch.insert(epoch_index, evonode_block_count);
                }
            }

            let get_epoch_reward_ratio =
                |range: std::ops::RangeInclusive<EpochIndex>| -> Option<RewardRatio> {
                    if range.start() == range.end() {
                        let epoch = *range.start();
                        let evonode_blocks = evonode_blocks_per_epoch
                            .get(&epoch)
                            .copied()
                            .unwrap_or_default();
                        let total_blocks = blocks_per_epoch.get(&epoch).copied().unwrap_or(1);

                        if total_blocks > 0 {
                            Some(RewardRatio {
                                numerator: evonode_blocks,
                                denominator: total_blocks,
                            })
                        } else {
                            None
                        }
                    } else {
                        let mut total_blocks = 0;
                        let mut total_proposed_blocks = 0;

                        for epoch_index in range {
                            let evonode_blocks = evonode_blocks_per_epoch
                                .get(&epoch_index)
                                .copied()
                                .unwrap_or_default();
                            let epoch_total =
                                blocks_per_epoch.get(&epoch_index).copied().unwrap_or(1);

                            total_proposed_blocks += evonode_blocks;
                            total_blocks += epoch_total;
                        }

                        if total_blocks > 0 {
                            Some(RewardRatio {
                                numerator: total_proposed_blocks,
                                denominator: total_blocks,
                            })
                        } else {
                            None
                        }
                    }
                };
            reward_distribution_type
                .rewards_in_interval_with_explanation(
                    contract_creation_cycle_start,
                    start_from_moment_for_distribution,
                    max_cycle_moment,
                    Some(get_epoch_reward_ratio),
                    last_claim.is_none(),
                )
                .map_err(|e| TaskError::TokenQueryError {
                    detail: format!("Failed to calculate estimated rewards: {}", e),
                })?
        } else {
            reward_distribution_type
                .rewards_in_interval_with_explanation::<fn(std::ops::RangeInclusive<EpochIndex>) -> Option<RewardRatio>>(
                    contract_creation_cycle_start,
                    start_from_moment_for_distribution,
                    max_cycle_moment,
                    None,
                    last_claim.is_none(),
                )
                .map_err(|e| TaskError::TokenQueryError {
                    detail: format!("Failed to calculate estimated rewards: {}", e),
                })?
        };

        Ok(
            BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(
                IdentityTokenIdentifier {
                    identity_id,
                    token_id,
                },
                explanation.total_amount,
                explanation,
            ),
        )
    }
}
