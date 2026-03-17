use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::Sdk;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::{FetchMany, Identifier};

impl AppContext {
    pub async fn query_token_pricing(
        &self,
        token_id: Identifier,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match TokenPricingSchedule::fetch_many(sdk, &[token_id][..]).await {
            Ok(pricing_data) => {
                if let Some((_, pricing_option)) = pricing_data.into_iter().next() {
                    Ok(BackendTaskSuccessResult::TokenPricing {
                        token_id,
                        prices: pricing_option,
                    })
                } else {
                    Ok(BackendTaskSuccessResult::TokenPricing {
                        token_id,
                        prices: None,
                    })
                }
            }
            Err(e) => Err(TaskError::TokenQueryError {
                detail: format!("Failed to fetch token pricing: {}", e),
            }),
        }
    }
}
