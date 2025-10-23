use crate::context::AppContext;
use crate::database::Database;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{ContextProvider, DataContract};
use std::sync::{Arc, Mutex};

/// SPV-based ContextProvider for the Dash SDK.
///
/// - DataContract and TokenConfiguration are served from the local DB (same as RPC provider)
/// - Quorum public keys are resolved via dash-spv (through SpvManager) when in SPV mode
#[derive(Debug)]
pub(crate) struct SpvProvider {
    db: Arc<Database>,
    app_context: Mutex<Option<Arc<AppContext>>>,
    _network: Network,
}

impl SpvProvider {
    pub fn new(db: Arc<Database>, network: Network) -> Result<Self, String> {
        Ok(Self {
            db,
            app_context: Default::default(),
            _network: network,
        })
    }

    /// Attach the `AppContext` so we can access SpvManager and settings.
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) {
        let mut ac = self.app_context.lock().expect("lock poisoned");
        ac.replace(app_context);
    }
}

impl ContextProvider for SpvProvider {
    fn get_data_contract(
        &self,
        data_contract_id: &dash_sdk::platform::Identifier,
        _platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        let app_ctx_guard = self.app_context.lock().expect("lock poisoned");
        let app_ctx = app_ctx_guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;

        if data_contract_id == &app_ctx.dpns_contract.id() {
            Ok(Some(app_ctx.dpns_contract.clone()))
        } else if data_contract_id == &app_ctx.token_history_contract.id() {
            Ok(Some(app_ctx.token_history_contract.clone()))
        } else if data_contract_id == &app_ctx.withdraws_contract.id() {
            Ok(Some(app_ctx.withdraws_contract.clone()))
        } else if data_contract_id == &app_ctx.keyword_search_contract.id() {
            Ok(Some(app_ctx.keyword_search_contract.clone()))
        } else {
            let dc = self
                .db
                .get_contract_by_id(*data_contract_id, app_ctx.as_ref())
                .map_err(|e| ContextProviderError::Generic(e.to_string()))?;

            drop(app_ctx_guard);

            Ok(dc.map(|qc| Arc::new(qc.contract)))
        }
    }

    fn get_token_configuration(
        &self,
        token_id: &dash_sdk::platform::Identifier,
    ) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError>
    {
        let app_ctx_guard = self.app_context.lock().expect("lock poisoned");
        let app_ctx = app_ctx_guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;

        self.db
            .get_token_config_for_id(token_id, app_ctx)
            .map_err(|e| ContextProviderError::Generic(e.to_string()))
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        let app_ctx_guard = self.app_context.lock().expect("lock poisoned");
        let app_ctx = app_ctx_guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;

        // Ask SPV manager for the public key corresponding to (type, hash)
        match app_ctx.spv_manager().get_quorum_public_key(
            quorum_type,
            quorum_hash,
            core_chain_locked_height,
        ) {
            Ok(key) => Ok(key),
            Err(e) => Err(ContextProviderError::Generic(format!(
                "SPV quorum key lookup failed: {}",
                e
            ))),
        }
    }

    fn get_platform_activation_height(
        &self,
    ) -> Result<dash_sdk::dpp::prelude::CoreBlockHeight, ContextProviderError> {
        // TODO: wire actual activation height if needed
        Ok(1)
    }
}

impl Clone for SpvProvider {
    fn clone(&self) -> Self {
        let app_guard = self.app_context.lock().expect("lock poisoned");
        Self {
            db: self.db.clone(),
            app_context: Mutex::new(app_guard.clone()),
            _network: self._network,
        }
    }
}
