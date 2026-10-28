use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
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
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let actor = self.token_actor(sending_identity).await?;
        let common = self.token_action_common(data_contract.clone(), token_position, &signing_key);

        self.execute_token_op(
            actor.set_price(common, token_pricing_schedule, public_note, group_info),
            |result| match result {
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
            },
            BackendTaskSuccessResult::SetTokenPrice,
        )
        .await
    }
}
