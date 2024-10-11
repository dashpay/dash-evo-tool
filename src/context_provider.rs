use crate::config::NetworkConfig;
use crate::context::AppContext;
use crate::database::Database;
use dash_sdk::core::LowLevelDashCoreClient as CoreClient;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::ContextProvider;
use dash_sdk::platform::DataContract;
use rusqlite::Result;
use std::sync::{Arc, Mutex};

pub(crate) struct Provider {
    db: Arc<Database>,
    app_context: Mutex<Option<Arc<AppContext>>>,
    core: CoreClient,
}

impl Provider {
    /// Create new ContextProvider.
    ///
    /// Note that you have to bind it to app context using [Provider::set_app_context()].
    pub fn new(db: Arc<Database>, config: &NetworkConfig) -> Result<Self, String> {
        let core_client = CoreClient::new(
            &config.core_host,
            config.core_rpc_port,
            &config.core_rpc_user,
            &config.core_rpc_password,
        )
        .map_err(|e| e.to_string())?;

        Ok(Self {
            db,
            core: core_client,
            app_context: Default::default(),
        })
    }
    /// Set app context to the provider.
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) {
        // order matters - can cause deadlock
        let cloned = app_context.clone();
        let mut ac = self.app_context.lock().expect("lock poisoned");
        ac.replace(cloned);
        drop(ac);

        app_context.sdk.set_context_provider(self.clone());
    }
}

impl ContextProvider for Provider {
    fn get_data_contract(
        &self,
        data_contract_id: &dash_sdk::platform::Identifier,
    ) -> Result<Option<Arc<DataContract>>, dash_sdk::error::ContextProviderError> {
        let app_ctx_guard = self.app_context.lock().expect("lock poisoned");
        let app_ctx = app_ctx_guard
            .as_ref()
            .ok_or(ContextProviderError::Config("no app context".to_string()))?;

        if data_contract_id == &app_ctx.dpns_contract.id() {
            Ok(Some(app_ctx.dpns_contract.clone()))
        } else {
            let dc = self
                .db
                .get_contract_by_id(*data_contract_id, app_ctx.as_ref())
                .map_err(|e| dash_sdk::error::ContextProviderError::Generic(e.to_string()))?;

            drop(app_ctx_guard);

            Ok(dc.map(Arc::new))
        }
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32], // quorum hash is 32 bytes
        _core_chain_locked_height: u32,
    ) -> std::result::Result<[u8; 48], dash_sdk::error::ContextProviderError> {
        let key = self.core.get_quorum_public_key(quorum_type, quorum_hash)?;

        Ok(key)
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
        let app_guard = self.app_context.lock().expect("lock poisoned");
        Self {
            core: self.core.clone(),
            db: self.db.clone(),
            app_context: Mutex::new(app_guard.clone()),
        }
    }
}
