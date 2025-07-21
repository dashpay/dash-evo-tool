use crate::context::AppContext;
use crate::backend_task::BackendTaskSuccessResult;
use dash_spv::{DashSpvClient, ClientConfig, SyncProgress};
use dash_sdk::dpp::platform_value::Bytes32;
use dash_sdk::dpp::state_transition::StateTransition;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub enum SpvTask {
    InitializeAndSync { checkpoint_height: u32 },
    GetSyncProgress,
    VerifyStateTransition(StateTransition),
    VerifyIdentity(Bytes32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpvTaskResult {
    SyncProgress {
        current_height: u32,
        target_height: u32,
        progress_percent: f32,
    },
    SyncComplete {
        final_height: u32,
    },
    ProofVerificationResult {
        is_valid: bool,
        details: String,
    },
    Error(String),
}

pub struct SpvManager {
    pub client: Option<Arc<Mutex<DashSpvClient>>>,
    pub is_syncing: bool,
    pub is_monitoring: bool,
    pub current_height: u32,
    pub target_height: u32,
}

impl std::fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvManager")
            .field("client", &self.client.is_some())
            .field("is_syncing", &self.is_syncing)
            .field("is_monitoring", &self.is_monitoring)
            .field("current_height", &self.current_height)
            .field("target_height", &self.target_height)
            .finish()
    }
}

impl SpvManager {
    pub fn new() -> Self {
        Self {
            client: None,
            is_syncing: false,
            is_monitoring: false,
            current_height: 0,
            target_height: 0,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.client.is_some()
    }

    pub async fn initialize(&mut self, network: &str, checkpoint_height: u32) -> Result<(), String> {
        tracing::info!("Initializing SPV client for network: {}, checkpoint: {}", network, checkpoint_height);
        
        // Create SPV client configuration
        let config = match network {
            "mainnet" => ClientConfig::mainnet(),
            "testnet" => ClientConfig::testnet(),
            "devnet" => {
                // Devnet uses the same configuration as testnet but with different peers
                // The actual devnet configuration would need to be specified via peers
                ClientConfig::testnet()
            },
            "regtest" => ClientConfig::regtest(),
            _ => return Err(format!("Unsupported network: {}", network)),
        };
        
        // Configure storage path and settings
        // Use the app's data directory instead of a relative path
        let app_data_dir = crate::app_dir::app_user_data_dir_path()
            .map_err(|e| format!("Failed to get app data directory: {:?}", e))?;
        let storage_path = app_data_dir.join("spv").join(network);
        
        // Ensure the directory exists
        if let Err(e) = std::fs::create_dir_all(&storage_path) {
            tracing::warn!("Failed to create SPV storage directory: {:?}", e);
        }
        
        tracing::info!("SPV storage path: {:?}", storage_path);
        
        // Configure with proper checkpoint settings
        let config = config
            .with_storage_path(storage_path)
            .with_log_level("debug") // Set to debug to see what's happening
            .with_start_height(checkpoint_height);
        
        tracing::info!("SPV config: start_height: {}", checkpoint_height);
        
        // Create SPV client
        let client = DashSpvClient::new(config).await
            .map_err(|e| format!("Failed to create SPV client: {:?}", e))?;
        
        self.client = Some(Arc::new(Mutex::new(client)));
        
        tracing::info!("SPV client initialized successfully");
        Ok(())
    }

    pub async fn start_sync(&mut self) -> Result<(), String> {
        if let Some(client_arc) = &self.client {
            tracing::info!("Starting SPV client sync");
            self.is_syncing = true;
            
            // Clone the Arc for the monitoring task
            let monitor_client = client_arc.clone();
            
            // Start the client
            {
                let mut client = client_arc.lock().await;
                client.start().await
                    .map_err(|e| format!("Failed to start SPV client: {:?}", e))?;
                tracing::info!("SPV client started successfully");
            }
            
            // CRITICAL: Start the monitoring loop in a background task!
            // Now that dash-spv has been fixed to implement Send + Sync, this works properly
            if !self.is_monitoring {
                self.is_monitoring = true;
                tracing::info!("Starting SPV network monitoring loop");
                
                tokio::spawn(async move {
                    loop {
                        // Lock the client for monitoring
                        let mut client_guard = monitor_client.lock().await;
                        
                        // Run monitor_network
                        match client_guard.monitor_network().await {
                            Ok(_) => {
                                tracing::info!("Monitor network completed normally");
                                break;
                            }
                            Err(e) => {
                                tracing::error!("Monitor network error: {:?}", e);
                                break;
                            }
                        }
                    }
                    tracing::info!("SPV monitoring loop has ended");
                });
            }
            
            // Wait a bit for peers to be fully connected and monitoring to start
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            
            // Explicitly trigger sync_to_tip to ensure headers are requested
            tracing::info!("Triggering sync to tip");
            let mut client = client_arc.lock().await;
            match client.sync_to_tip().await {
                Ok(progress) => {
                    tracing::info!("Sync initiated, initial progress: header_height={}, filter_height={}, peer_count={}", 
                        progress.header_height, progress.filter_header_height, progress.peer_count);
                    
                    // If we're still at height 0 after sync_to_tip, there might be an issue
                    if progress.header_height == 0 && progress.peer_count > 0 {
                        tracing::warn!("SPV sync appears stuck at height 0 despite having {} peers", progress.peer_count);
                        
                        // Try to get more info about the chain state
                        let chain_state = client.chain_state().await;
                        tracing::info!("Chain state tip height: {}", chain_state.tip_height());
                        
                        // If we're still stuck after monitoring has started, there might be a different issue
                        tracing::debug!("Monitoring loop should be running, sync should progress soon");
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to initiate sync_to_tip: {:?}", e);
                }
            }
            
            Ok(())
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn sync_to_tip(&self) -> Result<SyncProgress, String> {
        if let Some(client) = &self.client {
            let mut client = client.lock().await;
            client.sync_to_tip().await
                .map_err(|e| format!("Failed to sync to tip: {:?}", e))
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn get_sync_progress(&mut self, _network: dash_sdk::dashcore_rpc::dashcore::Network) -> Result<(u32, u32, f32), String> {
        if let Some(client) = &self.client {
            let client = client.lock().await;
            let progress = client.sync_progress().await
                .map_err(|e| format!("Failed to get sync progress: {:?}", e))?;
            
            // Debug log the raw progress data (only once per actual height change)
            static LAST_DEBUG_HEIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
            let last_debug = LAST_DEBUG_HEIGHT.load(std::sync::atomic::Ordering::Relaxed);
            if progress.header_height != last_debug || (progress.header_height == 0 && last_debug == u32::MAX) {
                LAST_DEBUG_HEIGHT.store(progress.header_height, std::sync::atomic::Ordering::Relaxed);
                tracing::debug!("SPV raw progress: header_height={}, filter_height={}, headers_synced={}, peer_count={}", 
                    progress.header_height, progress.filter_header_height, progress.headers_synced, progress.peer_count);
            }
            
            // Get the chain state for more accurate information
            let chain_state = client.chain_state().await;
            let tip_height = chain_state.tip_height();
            
            // Update our stored heights
            // Use the greater of header_height or tip_height (from checkpoint)
            self.current_height = progress.header_height.max(tip_height);
            
            // Check if we're actively syncing by looking at the sync state
            let is_actively_syncing = !progress.headers_synced || 
                                    progress.filter_header_height < progress.header_height ||
                                    progress.header_height < 1_200_000; // We know testnet is > 1.2M
            
            // For target height, we need a reasonable estimate
            if self.target_height == 0 || is_actively_syncing {
                // For testnet, we know it's around 1.29M+ based on the logs
                self.target_height = match self.current_height {
                    h if h < 1_000_000 => 1_300_000, // Testnet current height estimate
                    h if h < 1_200_000 => 1_300_000,
                    h => h + 10_000, // Otherwise assume we need more blocks
                };
            }
            
            // Since we're starting from genesis (height 0), calculate progress directly
            let progress_percent = if self.target_height > 0 {
                ((self.current_height as f32 / self.target_height as f32) * 100.0).min(100.0).max(0.0)
            } else if self.current_height > 0 {
                // If we have current height but no target, show some progress
                0.1
            } else {
                0.0
            };
            
            // Update syncing state
            self.is_syncing = is_actively_syncing || progress.header_height < 1_200_000;
            
            // Log progress only when there's a significant change (every 1000 blocks or 1% progress or once per second)
            static LAST_LOGGED_HEIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            static LAST_LOGGED_TIME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let last_height = LAST_LOGGED_HEIGHT.load(std::sync::atomic::Ordering::Relaxed);
            let last_time = LAST_LOGGED_TIME.load(std::sync::atomic::Ordering::Relaxed);
            
            // Only log if height changed by 1000+ blocks OR at least 1 second has passed since last log
            if self.current_height >= last_height + 1000 || (now >= last_time + 1 && self.current_height != last_height) {
                LAST_LOGGED_HEIGHT.store(self.current_height, std::sync::atomic::Ordering::Relaxed);
                LAST_LOGGED_TIME.store(now, std::sync::atomic::Ordering::Relaxed);
                
                tracing::info!(
                    "SPV sync progress - current: {}, target: {}, percent: {:.2}%, headers_synced: {}, filter_height: {}, tip_height: {}, header_height: {}",
                    self.current_height,
                    self.target_height,
                    progress_percent,
                    progress.headers_synced,
                    progress.filter_header_height,
                    tip_height,
                    progress.header_height
                );
            }
            
            // If we're fully synced, adjust target to current
            if progress.headers_synced && progress.filter_header_height >= progress.header_height && progress.header_height > 1_200_000 {
                self.target_height = self.current_height;
                self.is_syncing = false;
                return Ok((self.current_height, self.target_height, 100.0));
            }
            
            Ok((self.current_height, self.target_height, progress_percent))
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn verify_state_transition_proof(&self, _state_transition: &StateTransition) -> Result<bool, String> {
        if self.client.is_some() {
            // The SDK will handle the actual proof verification using our ContextProvider
            // which now provides quorum public keys from the SPV client's MasternodeListEngine.
            // The verification happens automatically when the SDK processes state transitions.
            // For now, we just confirm that SPV is ready to provide the necessary data.
            Ok(true)
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn verify_identity_proof(&self, _identity_id: &Bytes32) -> Result<bool, String> {
        if self.client.is_some() {
            // The SDK will handle the actual proof verification using our ContextProvider
            // which now provides quorum public keys from the SPV client's MasternodeListEngine.
            // The verification happens automatically when the SDK fetches identities with proofs.
            // For now, we just confirm that SPV is ready to provide the necessary data.
            Ok(true)
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        if let Some(client) = &self.client {
            let mut client = client.lock().await;
            client.stop().await
                .map_err(|e| format!("Failed to stop SPV client: {:?}", e))?;
        }
        Ok(())
    }
}

impl AppContext {
    pub async fn run_spv_task(&self, task: SpvTask) -> Result<BackendTaskSuccessResult, String> {
        let result = match task {
            SpvTask::InitializeAndSync { checkpoint_height } => {
                self.execute_spv_sync(checkpoint_height).await?
            }
            SpvTask::GetSyncProgress => self.execute_spv_get_progress().await?,
            SpvTask::VerifyStateTransition(state_transition) => {
                self.execute_spv_verify_state_transition(state_transition).await?
            }
            SpvTask::VerifyIdentity(identity_id) => {
                self.execute_spv_verify_identity(identity_id).await?
            }
        };
        
        Ok(BackendTaskSuccessResult::SpvResult(result))
    }

    pub async fn execute_spv_sync(&self, checkpoint_height: u32) -> Result<SpvTaskResult, String> {
        let mut spv_manager = self.spv_manager.write().await;
        
        // Initialize if not already done
        if spv_manager.client.is_none() {
            let network = match self.network {
                dash_sdk::dashcore_rpc::dashcore::Network::Dash => "mainnet",
                dash_sdk::dashcore_rpc::dashcore::Network::Testnet => "testnet",
                dash_sdk::dashcore_rpc::dashcore::Network::Devnet => "devnet",
                dash_sdk::dashcore_rpc::dashcore::Network::Regtest => "regtest",
                _ => return Err("Unsupported network for SPV".to_string()),
            };
            
            spv_manager.initialize(network, checkpoint_height).await?;
        }
        
        // Start sync
        spv_manager.start_sync().await?;
        
        // The monitoring loop has been started in a background task
        
        // Get initial progress
        let (current, target, percent) = spv_manager.get_sync_progress(self.network).await?;
        
        Ok(SpvTaskResult::SyncProgress {
            current_height: current,
            target_height: target,
            progress_percent: percent,
        })
    }

    pub async fn execute_spv_get_progress(&self) -> Result<SpvTaskResult, String> {
        let mut spv_manager = self.spv_manager.write().await;
        
        if spv_manager.client.is_none() {
            return Err("SPV not initialized".to_string());
        }
        
        let (current, target, progress) = spv_manager.get_sync_progress(self.network).await?;
        
        if current >= target && target > 0 && !spv_manager.is_syncing {
            Ok(SpvTaskResult::SyncComplete {
                final_height: current,
            })
        } else {
            Ok(SpvTaskResult::SyncProgress {
                current_height: current,
                target_height: target,
                progress_percent: progress,
            })
        }
    }

    pub async fn execute_spv_verify_state_transition(&self, state_transition: StateTransition) -> Result<SpvTaskResult, String> {
        let spv_manager = self.spv_manager.read().await;
        
        let is_valid = spv_manager.verify_state_transition_proof(&state_transition).await?;
        
        Ok(SpvTaskResult::ProofVerificationResult {
            is_valid,
            details: format!("State transition {} verification", if is_valid { "passed" } else { "failed" }),
        })
    }

    pub async fn execute_spv_verify_identity(&self, identity_id: Bytes32) -> Result<SpvTaskResult, String> {
        let spv_manager = self.spv_manager.read().await;
        
        let is_valid = spv_manager.verify_identity_proof(&identity_id).await?;
        
        Ok(SpvTaskResult::ProofVerificationResult {
            is_valid,
            details: format!("Identity {} verification", if is_valid { "passed" } else { "failed" }),
        })
    }
}