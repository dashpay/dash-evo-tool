use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::tokens::transitions::SetPriceResult;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn set_direct_purchase_price(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        token_pricing_schedule: Option<TokenPricingSchedule>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        _sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let platform_wallet = self.platform_wallet_for_identity(sending_identity)?;
        let token_wallet = platform_wallet.tokens();

        let result = token_wallet
            .set_price_with_signer(
                data_contract.clone(),
                token_position,
                sending_identity.identity.id(),
                token_pricing_schedule,
                &signing_key,
                sending_identity,
                public_note,
                group_info,
                self.state_transition_options(),
            )
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

        // Log the proof-verified set price result
        match result {
            SetPriceResult::PricingSchedule(owner_id, schedule) => {
                tracing::info!(
                    "SetDirectPurchasePrice: owner {} has_schedule={}",
                    owner_id,
                    schedule.is_some()
                );
            }
            SetPriceResult::HistoricalDocument(document) => {
                tracing::info!(
                    "SetDirectPurchasePrice: historical document id={}",
                    document.id()
                );
            }
            SetPriceResult::GroupActionWithDocument(power, doc) => {
                tracing::info!(
                    "SetDirectPurchasePrice: group action power={}, has_doc={}",
                    power,
                    doc.is_some()
                );
            }
            SetPriceResult::GroupActionWithPricingSchedule(power, status, schedule) => {
                tracing::info!(
                    "SetDirectPurchasePrice: group action power={}, status={:?}, has_schedule={}",
                    power,
                    status,
                    schedule.is_some()
                );
            }
        }

        // Return success with fee result
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::SetTokenPrice(fee_result))
    }
}
