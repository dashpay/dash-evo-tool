//! Query Platform for which identities are frozen for a given token

use super::TokenResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::Sdk;
use dash_sdk::dpp::tokens::info::IdentityTokenInfo;
use dash_sdk::dpp::tokens::info::v0::IdentityTokenInfoV0Accessors;
use dash_sdk::platform::tokens::token_info::IdentitiesTokenInfosQuery;
use dash_sdk::platform::{FetchMany, Identifier};
use dash_sdk::query_types::token_info::IdentitiesTokenInfos;

impl AppContext {
    pub async fn query_frozen_identities(
        &self,
        sdk: &Sdk,
        token_id: Identifier,
        identity_ids: Vec<Identifier>,
    ) -> Result<BackendTaskSuccessResult, String> {
        if identity_ids.is_empty() {
            return Ok(BackendTaskSuccessResult::Token(
                TokenResult::FrozenIdentities(vec![]),
            ));
        }

        let query = IdentitiesTokenInfosQuery {
            identity_ids,
            token_id,
        };

        let token_infos: IdentitiesTokenInfos = IdentityTokenInfo::fetch_many(sdk, query)
            .await
            .map_err(|e| format!("Failed to query frozen identities: {e}"))?;

        let frozen_ids: Vec<Identifier> = token_infos
            .iter()
            .filter_map(|(id, info)| {
                if let Some(info) = info
                    && info.frozen()
                {
                    return Some(*id);
                }
                None
            })
            .collect();

        Ok(BackendTaskSuccessResult::Token(
            TokenResult::FrozenIdentities(frozen_ids),
        ))
    }
}
