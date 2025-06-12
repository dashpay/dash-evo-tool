use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::{FetchMany, Identifier};
use dash_sdk::Sdk;

impl AppContext {
    pub async fn query_token_pricing(
        &self,
        token_id: Identifier,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Query token pricing schedule using fetch_many
        match TokenPricingSchedule::fetch_many(sdk, &[token_id][..]).await {
            Ok(pricing_data) => {
                // Check if we got any pricing data for this token
                if let Some((_, pricing_option)) = pricing_data.into_iter().next() {
                    Ok(BackendTaskSuccessResult::TokenPricing {
                        token_id,
                        prices: pricing_option,
                    })
                } else {
                    // No pricing data found
                    Ok(BackendTaskSuccessResult::TokenPricing {
                        token_id,
                        prices: None,
                    })
                }
            }
            Err(e) => Err(format!("Failed to fetch token pricing: {}", e)),
        }
    }
}
