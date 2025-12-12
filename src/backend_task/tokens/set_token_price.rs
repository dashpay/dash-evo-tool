use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::tokens::builders::set_price::TokenChangeDirectPurchasePriceTransitionBuilder;
use dash_sdk::platform::tokens::transitions::SetPriceResult;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
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
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut builder = TokenChangeDirectPurchasePriceTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            sending_identity.identity.id(),
        );

        if let Some(pricing_schedule) = token_pricing_schedule {
            builder = builder.with_token_pricing_schedule(pricing_schedule);
        }

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        if let Some(group_info) = group_info {
            builder = builder.with_using_group_info(group_info);
        }

        if let Some(options) = self.state_transition_options() {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_set_price_for_direct_purchase(builder, &signing_key, sending_identity)
            .await
            .map_err(|e| match e {
                Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                    self.db
                        .insert_proof_log_item(ProofLogItem {
                            request_type: RequestType::BroadcastStateTransition,
                            request_bytes: vec![],
                            verification_path_query_bytes: vec![],
                            height: block_info.height,
                            time_ms: block_info.time_ms,
                            proof_bytes,
                            error: Some(proof_error.to_string()),
                        })
                        .ok();
                    format!(
                        "Error broadcasting SetPrice Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting SetPrice Tokens transition: {}", e),
            })?;

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

        // Return success
        Ok(BackendTaskSuccessResult::SetTokenPrice)
    }
}
