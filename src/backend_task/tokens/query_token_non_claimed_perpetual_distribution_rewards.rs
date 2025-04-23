//! Execute token query by keyword on Platform

use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use crate::{backend_task::BackendTaskSuccessResult, context::AppContext};
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::{platform::Identifier, Sdk};

impl AppContext {
    pub async fn query_token_non_claimed_perpetual_distribution_rewards(
        &self,
        identity_id: Identifier,
        token_id: Identifier,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // ── 3. return result ────────────────────────────────────────────────
        Ok(
            BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmount(
                IdentityTokenIdentifier {
                    identity_id,
                    token_id,
                },
                0,
            ),
        )
    }
}
