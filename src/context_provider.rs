use crate::context::AppContext;
use crate::database::Database;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{DataContract, Identifier};
use rusqlite::Result;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared contract/token resolution used by the SPV context provider.
// ---------------------------------------------------------------------------

/// Number of system contracts cached on [`AppContext`].
/// Update this when adding a new system contract field.
///
/// The typed array size in [`resolve_data_contract`] must match — the compiler
/// will reject a mismatch, catching forgotten additions at build time.
pub(crate) const SYSTEM_CONTRACT_COUNT: usize = 5;

/// Resolve a data contract by ID: check cached system contracts first, then DB.
///
/// All system contracts are listed in `cached` — adding a new one is a single
/// array edit, which prevents the two providers from drifting out of sync.
/// The array size is tied to [`SYSTEM_CONTRACT_COUNT`] so the compiler enforces
/// completeness.
pub(crate) fn resolve_data_contract(
    app_ctx: &AppContext,
    _db: &Database,
    data_contract_id: &Identifier,
) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
    let cached: [&Arc<DataContract>; SYSTEM_CONTRACT_COUNT] = [
        &app_ctx.dpns_contract,
        &app_ctx.dashpay_contract,
        &app_ctx.token_history_contract,
        &app_ctx.withdraws_contract,
        &app_ctx.keyword_search_contract,
    ];

    for contract in &cached {
        if data_contract_id == &contract.id() {
            return Ok(Some(Arc::clone(contract)));
        }
    }

    // K/V fallback for user-added / non-system contracts
    let dc = app_ctx
        .get_contract_by_id(data_contract_id)
        .map_err(|e| ContextProviderError::Generic(e.to_string()))?;

    Ok(dc.map(|qc| Arc::new(qc.contract)))
}

/// Resolve a token configuration from the database.
pub(crate) fn resolve_token_configuration(
    app_ctx: &AppContext,
    db: &Database,
    token_id: &Identifier,
) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError> {
    db.get_token_config_for_id(token_id, app_ctx)
        .map_err(|e| ContextProviderError::Generic(e.to_string()))
}
