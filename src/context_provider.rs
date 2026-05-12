use crate::app_dir::core_cookie_path;
use crate::config::NetworkConfig;
use crate::context::AppContext;
use crate::database::Database;
use dash_sdk::core::LowLevelDashCoreClient as CoreClient;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{ContextProvider, DataContract, Identifier};
use rusqlite::Result;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Shared contract/token resolution used by both RPC and SPV providers.
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
    db: &Database,
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

    // DB fallback for user-added / non-system contracts
    let dc = db
        .get_contract_by_id(*data_contract_id, app_ctx)
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

pub(crate) struct Provider {
    db: Arc<Database>,
    app_context: Mutex<Option<Arc<AppContext>>>,
    pub core: CoreClient,
}

impl Provider {
    /// Create new ContextProvider.
    ///
    /// Note that you have to bind it to app context using [`Provider::bind_app_context`].
    pub fn new(
        db: Arc<Database>,
        network: Network,
        config: &NetworkConfig,
    ) -> Result<Self, String> {
        let cookie_path = core_cookie_path(network, &config.devnet_name)
            .map_err(|e| format!("Failed to get core cookie path: {}", e))?;

        // Read the cookie from disk
        let cookie = std::fs::read_to_string(cookie_path);
        let (user, pass) = if let Ok(cookie) = cookie {
            let cookie = cookie.trim();
            // split the cookie at ":", first part is user (__cookie__), second part is password
            if let Some((user, password)) = cookie.split_once(':') {
                (user.to_string(), password.to_string())
            } else {
                return Err("Malformed cookie file: expected 'user:password' format".to_string());
            }
        } else {
            // Fall back to the pre-set user / pass if needed
            (
                config.core_rpc_user.clone().unwrap_or_default(),
                config.core_rpc_password.clone().unwrap_or_default(),
            )
        };

        let host = config.rpc_host();
        let port = config.rpc_port(network);
        let core_client = CoreClient::new(host, port, &user, &pass).map_err(|e| e.to_string())?;

        Ok(Self {
            db,
            core: core_client,
            app_context: Default::default(),
        })
    }
    /// Set app context to the provider.
    ///
    /// Returns an error if any lock is poisoned (indicates a prior panic).
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) -> Result<(), String> {
        // order matters - can cause deadlock
        let cloned = app_context.clone();
        let mut ac = self
            .app_context
            .lock()
            .map_err(|_| "Provider app_context lock poisoned".to_string())?;
        ac.replace(cloned);
        drop(ac);

        app_context.sdk.load().set_context_provider(self.clone());
        Ok(())
    }
}

impl ContextProvider for Provider {
    fn get_data_contract(
        &self,
        data_contract_id: &Identifier,
        _platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        let guard = self
            .app_context
            .lock()
            .map_err(|_| ContextProviderError::Config("RpcProvider lock poisoned".to_string()))?;
        let app_ctx = guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?
            .clone();
        drop(guard);
        resolve_data_contract(&app_ctx, &self.db, data_contract_id)
    }

    fn get_token_configuration(
        &self,
        token_id: &Identifier,
    ) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError>
    {
        let guard = self
            .app_context
            .lock()
            .map_err(|_| ContextProviderError::Config("RpcProvider lock poisoned".to_string()))?;
        let app_ctx = guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?
            .clone();
        drop(guard);
        resolve_token_configuration(&app_ctx, &self.db, token_id)
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        _core_chain_locked_height: u32,
    ) -> std::result::Result<[u8; 48], ContextProviderError> {
        self.core
            .get_quorum_public_key(quorum_type, quorum_hash)
            .map_err(|e| ContextProviderError::Generic(e.to_string()))
    }

    fn get_platform_activation_height(
        &self,
    ) -> std::result::Result<
        dash_sdk::dpp::prelude::CoreBlockHeight,
        dash_sdk::error::ContextProviderError,
    > {
        Ok(1)
    }
}

impl Clone for Provider {
    fn clone(&self) -> Self {
        // Clone trait doesn't allow returning Result, so we use a fallback
        // If the lock is poisoned, clone with None app_context (will require rebinding)
        let app_context_clone = self
            .app_context
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| {
                tracing::warn!("Provider lock poisoned during clone, using fallback");
                poisoned.into_inner().clone()
            });
        Self {
            core: self.core.clone(),
            db: self.db.clone(),
            app_context: Mutex::new(app_context_clone),
        }
    }
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider").finish()
    }
}
