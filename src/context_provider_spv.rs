use crate::context::AppContext;
use crate::context_provider::{resolve_data_contract, resolve_token_configuration};
use crate::database::Database;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{ContextProvider, DataContract, Identifier};
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

    /// Attach the `AppContext` and register this provider with the SDK.
    ///
    /// Mirrors `RpcProvider::bind_app_context` — after this call, the SDK
    /// uses this provider for proof verification and quorum key resolution.
    ///
    /// Returns an error if the lock is poisoned (indicates a prior panic).
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) -> Result<(), String> {
        let cloned = app_context.clone();
        let mut ac = self
            .app_context
            .lock()
            .map_err(|_| "SpvProvider app_context lock poisoned".to_string())?;
        ac.replace(cloned);
        drop(ac);

        app_context.sdk.load().set_context_provider(self.clone());
        Ok(())
    }
}

impl ContextProvider for SpvProvider {
    fn get_data_contract(
        &self,
        data_contract_id: &Identifier,
        _platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        let guard = self
            .app_context
            .lock()
            .map_err(|_| ContextProviderError::Config("SpvProvider lock poisoned".to_string()))?;
        let app_ctx = guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;
        resolve_data_contract(app_ctx, &self.db, data_contract_id)
    }

    fn get_token_configuration(
        &self,
        token_id: &Identifier,
    ) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError>
    {
        let guard = self
            .app_context
            .lock()
            .map_err(|_| ContextProviderError::Config("SpvProvider lock poisoned".to_string()))?;
        let app_ctx = guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;
        resolve_token_configuration(app_ctx, &self.db, token_id)
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        let app_ctx_guard = self
            .app_context
            .lock()
            .map_err(|_| ContextProviderError::Config("SpvProvider lock poisoned".to_string()))?;
        let app_ctx = app_ctx_guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;

        let spv_manager = app_ctx.spv_manager();

        spv_manager
            .get_quorum_public_key(quorum_type, quorum_hash, core_chain_locked_height)
            .map_err(ContextProviderError::Generic)
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
        // Clone trait doesn't allow returning Result, so we use a fallback
        // If the lock is poisoned, clone with None app_context (will require rebinding)
        let app_context_clone = self
            .app_context
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| {
                tracing::warn!("SpvProvider lock poisoned during clone, using fallback");
                poisoned.into_inner().clone()
            });
        Self {
            db: self.db.clone(),
            app_context: Mutex::new(app_context_clone),
            _network: self._network,
        }
    }
}
