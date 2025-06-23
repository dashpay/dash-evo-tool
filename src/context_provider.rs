use crate::app_dir::core_cookie_path;
use crate::config::NetworkConfig;
use crate::context::AppContext;
use crate::database::Database;
use crate::model::connection_type::ConnectionType;
use dash_sdk::core::LowLevelDashCoreClient as CoreClient;
use dash_sdk::dpp::core_types::validator_set::v0::ValidatorSetV0Getters;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::ContextProvider;
use dash_sdk::platform::DataContract;
use dash_sdk::platform::FetchUnproved;
use dash_sdk::query_types::{CurrentQuorumsInfo, NoParamQuery};
use rusqlite::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Type alias for quorum key cache: (quorum_type, quorum_hash) -> public_key
type QuorumKeyCache = Arc<RwLock<HashMap<(u32, [u8; 32]), [u8; 48]>>>;

#[derive(Debug)]
pub(crate) struct Provider {
    db: Arc<Database>,
    app_context: Mutex<Option<Arc<AppContext>>>,
    pub core: Option<CoreClient>,
    connection_type: ConnectionType,
    /// Cache for quorum public keys: (quorum_type, quorum_hash) -> public_key
    quorum_key_cache: QuorumKeyCache,
}

impl Provider {
    /// Create new ContextProvider.
    ///
    /// Note that you have to bind it to app context using [Provider::set_app_context()].
    pub fn new(
        db: Arc<Database>,
        network: Network,
        config: &NetworkConfig,
    ) -> Result<Self, String> {
        let connection_type = config.connection_type.clone();

        // Only create core client if using DashCore connection
        let core = if connection_type == ConnectionType::DashCore {
            let cookie_path = core_cookie_path(network, &config.devnet_name)
                .expect("Failed to get core cookie path");

            // Read the cookie from disk
            let cookie = std::fs::read_to_string(cookie_path);
            let (user, pass) = if let Ok(cookie) = cookie {
                // split the cookie at ":", first part is user (__cookie__), second part is password
                let cookie_parts: Vec<&str> = cookie.split(':').collect();
                let user = cookie_parts[0];
                let password = cookie_parts[1];
                (user.to_string(), password.to_string())
            } else {
                // Fall back to the pre-set user / pass if needed
                (
                    config.core_rpc_user.clone(),
                    config.core_rpc_password.clone(),
                )
            };

            Some(
                CoreClient::new(&config.core_host, config.core_rpc_port, &user, &pass)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };

        Ok(Self {
            db,
            core,
            app_context: Default::default(),
            connection_type,
            quorum_key_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    /// Set app context to the provider.
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) {
        // order matters - can cause deadlock
        let cloned = app_context.clone();
        let mut ac = self.app_context.lock().expect("lock poisoned");
        ac.replace(cloned);
        drop(ac);

        let sdk = app_context.sdk.write().expect("lock poisoned");
        sdk.set_context_provider(self.clone());
    }

    /// Pre-fetch and cache current quorum keys for SPV mode
    pub async fn prefetch_quorum_keys(
        &self,
        sdk: &dash_sdk::Sdk,
    ) -> std::result::Result<(), String> {
        tracing::info!("🚀 PRE-FETCHING QUORUM KEYS FOR SPV MODE...");

        match CurrentQuorumsInfo::fetch_unproved(sdk, NoParamQuery {}).await {
            Ok(Some(quorums_info)) => {
                tracing::info!("Successfully connected to DAPI and retrieved quorum information");
                tracing::info!(
                    "Last Platform Block Height: {}",
                    quorums_info.last_platform_block_height
                );
                tracing::info!(
                    "Last Core Block Height: {}",
                    quorums_info.last_core_block_height
                );
                tracing::info!(
                    "Number of validator sets: {}",
                    quorums_info.validator_sets.len()
                );

                let mut cache = self.quorum_key_cache.write().map_err(|e| e.to_string())?;
                let mut count = 0;

                // Extract actual BLS public keys from each validator set
                for (i, validator_set) in quorums_info.validator_sets.iter().enumerate() {
                    let quorum_hash = quorums_info.quorum_hashes[i];

                    // Get the threshold public key (BLS public key) from the validator set
                    let threshold_public_key = validator_set.threshold_public_key();

                    // Convert BLS public key to compressed 48-byte format
                    let public_key_bytes: [u8; 48] = threshold_public_key.0.to_compressed();

                    // Cache this key for ALL common quorum types since we can't reliably
                    // predict which type the SDK will request for any given validator set.
                    // This ensures the SDK can find the key regardless of which type it requests.
                    let quorum_types = vec![
                        1u32, 3u32, // LLMQ_400_60 types
                        4u32, 5u32, // LLMQ_100_67 types
                        6u32, 7u32, 8u32, // LLMQ_60_75 types
                        100u32, 101u32, 102u32, 103u32, 104u32, 105u32, 106u32, // DIP24 types
                    ];

                    tracing::debug!(
                        "Validator set {}: hash={}, caching for {} types",
                        i,
                        hex::encode(quorum_hash),
                        quorum_types.len()
                    );

                    // Cache the BLS public key for all relevant quorum types
                    for quorum_type in quorum_types {
                        cache.insert((quorum_type, quorum_hash), public_key_bytes);
                        count += 1;

                        tracing::trace!(
                            "Cached quorum key: type={}, hash={}",
                            quorum_type,
                            hex::encode(quorum_hash)
                        );
                    }
                }

                tracing::info!(
                    "Successfully fetched and cached {} actual BLS quorum keys from DAPI",
                    count
                );
                Ok(())
            }
            Ok(None) => {
                tracing::warn!("No quorum info available from DAPI");
                Err(
                    "SPV mode initialization failed: No quorum info available from DAPI"
                        .to_string(),
                )
            }
            Err(e) => {
                tracing::error!("Failed to fetch quorum info from DAPI: {}", e);
                Err(format!(
                    "SPV mode initialization failed: Cannot connect to DAPI: {}",
                    e
                ))
            }
        }
    }
}

impl ContextProvider for Provider {
    fn get_data_contract(
        &self,
        data_contract_id: &dash_sdk::platform::Identifier,
        _platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, dash_sdk::error::ContextProviderError> {
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
                .map_err(|e| dash_sdk::error::ContextProviderError::Generic(e.to_string()))?;

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
            .map_err(|e| dash_sdk::error::ContextProviderError::Generic(e.to_string()))
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32], // quorum hash is 32 bytes
        _core_chain_locked_height: u32,
    ) -> std::result::Result<[u8; 48], dash_sdk::error::ContextProviderError> {
        tracing::debug!(
            "Quorum key request: type={}, hash={}",
            quorum_type,
            hex::encode(quorum_hash)
        );

        // First check the cache
        let cache_key = (quorum_type, quorum_hash);
        if let Ok(cache) = self.quorum_key_cache.read() {
            if let Some(key) = cache.get(&cache_key) {
                tracing::debug!(
                    "Cache hit: Using cached quorum public key for type {} hash {}",
                    quorum_type,
                    hex::encode(quorum_hash)
                );
                return Ok(*key);
            } else {
                tracing::warn!(
                    "Cache miss: No cached key found for type {} hash {} (cache has {} keys)",
                    quorum_type,
                    hex::encode(quorum_hash),
                    cache.len()
                );
            }
        }

        match &self.core {
            Some(core_client) => {
                let key = core_client.get_quorum_public_key(quorum_type, quorum_hash)?;

                // Cache the key for future use (including SPV mode)
                if let Ok(mut cache) = self.quorum_key_cache.write() {
                    cache.insert(cache_key, key);
                }

                Ok(key)
            }
            None => {
                // In SPV mode, if we don't have the quorum key cached,
                // it means either:
                // 1. SPV mode wasn't properly initialized with prefetch_quorum_keys()
                // 2. The requested quorum type/hash combination wasn't available in DAPI response
                // 3. There's a mismatch between what Core would provide vs what DAPI provides

                tracing::error!(
                    "SPV mode: Quorum key not available for type {} hash {:?}. This indicates either SPV mode wasn't properly initialized or the quorum is not available from DAPI.",
                    quorum_type,
                    hex::encode(quorum_hash)
                );

                Err(dash_sdk::error::ContextProviderError::Config(
                    format!(
                        "SPV mode: Quorum key not available for type {} hash {}. Ensure SPV mode is properly initialized with prefetch_quorum_keys() and the quorum exists in the current validator sets.",
                        quorum_type,
                        hex::encode(quorum_hash)
                    )
                ))
            }
        }
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
            connection_type: self.connection_type.clone(),
            quorum_key_cache: self.quorum_key_cache.clone(),
        }
    }
}
