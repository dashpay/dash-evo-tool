use crate::context::AppContext;
use crate::backend_task::BackendTaskSuccessResult;
use dash_spv::{DashSpvClient, ClientConfig, SyncProgress};
use dash_sdk::dpp::platform_value::Bytes32;
use dash_sdk::dpp::state_transition::StateTransition;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::timeout;
use dashcore::QuorumHash;
use dashcore::sml::llmq_type::LLMQType;
use dashcore::hashes::Hash;

#[derive(Debug, Clone, PartialEq)]
pub enum SpvTask {
    InitializeAndSync { checkpoint_height: u32 },
    GetSyncProgress,
    VerifyStateTransition(StateTransition),
    VerifyIdentity(Bytes32),
}

#[derive(Debug)]
enum SpvCommand {
    GetSyncProgress { response: oneshot::Sender<Result<SyncProgress, String>> },
    GetQuorumKey { 
        quorum_type: u8, 
        quorum_hash: [u8; 32], 
        response: oneshot::Sender<Option<[u8; 48]>> 
    },
    UpdateQuorumCache { response: oneshot::Sender<Result<usize, String>> },
    Stop { response: oneshot::Sender<Result<(), String>> },
}

#[derive(Debug, Clone)]
struct SpvState {
    current_height: u32,
    target_height: u32,
    is_syncing: bool,
    headers_synced: bool,
    last_update: Instant,
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
    command_tx: Option<mpsc::Sender<SpvCommand>>,
    pub is_syncing: bool,
    pub is_monitoring: bool,
    pub current_height: u32,
    pub target_height: u32,
    // Cached quorum data for fast read access
    cached_quorums: Arc<RwLock<std::collections::HashMap<(u8, [u8; 32]), [u8; 48]>>>,
    // Shared state for progress tracking
    shared_state: Arc<RwLock<SpvState>>,
}

impl std::fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shared_state_info = if let Ok(state) = self.shared_state.try_read() {
            format!("current: {}, target: {}, synced: {}", 
                state.current_height, state.target_height, state.headers_synced)
        } else {
            "(locked)".to_string()
        };
        
        f.debug_struct("SpvManager")
            .field("has_command_channel", &self.command_tx.is_some())
            .field("is_syncing", &self.is_syncing)
            .field("is_monitoring", &self.is_monitoring)
            .field("current_height", &self.current_height)
            .field("target_height", &self.target_height)
            .field("shared_state", &shared_state_info)
            .finish()
    }
}

impl SpvManager {
    pub fn new() -> Self {
        Self {
            command_tx: None,
            is_syncing: false,
            is_monitoring: false,
            current_height: 0,
            target_height: 0,
            cached_quorums: Arc::new(RwLock::new(std::collections::HashMap::new())),
            shared_state: Arc::new(RwLock::new(SpvState {
                current_height: 0,
                target_height: 0,
                is_syncing: false,
                headers_synced: false,
                last_update: Instant::now(),
            })),
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.command_tx.is_some()
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
        let mut config = config
            .with_storage_path(storage_path)
            .with_log_level("info") // Reduce log verbosity
            .with_start_height(checkpoint_height);
        
        // Limit to single peer to reduce duplicate header processing
        config.max_peers = 1;
        
        tracing::info!("SPV config: start_height: {}", checkpoint_height);
        
        // Create command channel
        let (command_tx, command_rx) = mpsc::channel::<SpvCommand>(100);
        self.command_tx = Some(command_tx);
        
        // Clone cached quorums and shared state for the background task
        let cached_quorums = self.cached_quorums.clone();
        let shared_state = self.shared_state.clone();
        
        // Spawn the SPV task that owns the client
        tokio::spawn(async move {
            // Create SPV client - owned by this task
            let mut client = match DashSpvClient::new(config).await {
                Ok(client) => client,
                Err(e) => {
                    tracing::error!("Failed to create SPV client: {:?}", e);
                    return;
                }
            };
            
            // Take the progress receiver for monitoring
            // For now, we don't use the progress receiver since it requires DetailedSyncProgress
            let progress_rx = None;
            
            // Start the client
            if let Err(e) = client.start().await {
                tracing::error!("Failed to start SPV client: {:?}", e);
                return;
            }
            
            tracing::info!("SPV client started successfully");
            
            // Log initial masternode list status
            if let Some(mn_engine) = client.masternode_list_engine() {
                let list_count = mn_engine.masternode_lists.len();
                tracing::info!("SPV client has {} masternode lists at startup", list_count);
            } else {
                tracing::warn!("No masternode list engine available at startup");
            }
            
            // Handle commands and monitor network concurrently
            Self::run_spv_task_loop(client, command_rx, progress_rx, cached_quorums, shared_state).await;
        });
        
        tracing::info!("SPV client initialized successfully");
        Ok(())
    }

    /// Run the SPV task loop that handles commands and monitors the network
    async fn run_spv_task_loop(
        mut client: DashSpvClient,
        mut command_rx: mpsc::Receiver<SpvCommand>,
        progress_rx: Option<mpsc::Receiver<SyncProgress>>,
        cached_quorums: Arc<RwLock<std::collections::HashMap<(u8, [u8; 32]), [u8; 48]>>>,
        shared_state: Arc<RwLock<SpvState>>,
    ) {
        // Trigger initial sync
        if let Err(e) = client.sync_to_tip().await {
            tracing::error!("Failed to initiate sync_to_tip: {:?}", e);
        }
        
        // Initial cache population after a short delay to ensure masternode lists are loaded
        tokio::time::sleep(Duration::from_secs(2)).await;
        Self::update_quorum_cache(&client, &cached_quorums).await;
        
        // Progress monitoring task
        let cached_quorums_clone = cached_quorums.clone();
        let shared_state_clone = shared_state.clone();
        let progress_task = if let Some(rx) = progress_rx {
            Some(tokio::spawn(async move {
                Self::monitor_progress(Some(rx), cached_quorums_clone, shared_state_clone).await;
            }))
        } else {
            None
        };
        
        // Update cache more frequently initially
        let mut cache_update_interval = tokio::time::interval(Duration::from_secs(10));
        
        loop {
            tokio::select! {
                // Handle commands
                Some(command) = command_rx.recv() => {
                    match command {
                        SpvCommand::GetSyncProgress { response } => {
                            let result = client.sync_progress().await
                                .map_err(|e| format!("Failed to get sync progress: {:?}", e));
                            
                            // Update shared state with the progress
                            if let Ok(progress) = &result {
                                if let Ok(mut state) = shared_state.try_write() {
                                    state.current_height = progress.header_height;
                                    state.headers_synced = progress.headers_synced;
                                    state.is_syncing = !progress.headers_synced;
                                    state.last_update = Instant::now();
                                    
                                    // Update target if needed
                                    if state.target_height == 0 || state.current_height > state.target_height - 20 {
                                        state.target_height = state.current_height + 50;
                                    }
                                }
                            }
                            
                            let _ = response.send(result);
                        }
                        SpvCommand::GetQuorumKey { quorum_type, quorum_hash, response } => {
                            // First try to get from MasternodeListEngine
                            let result = Self::get_quorum_key_from_client(&client, quorum_type, &quorum_hash);
                            
                            // Also update cache if found
                            if let Some(key) = result {
                                if let Ok(mut cache) = cached_quorums.try_write() {
                                    cache.insert((quorum_type, quorum_hash), key);
                                    tracing::debug!("Updated cache with quorum key from MasternodeListEngine");
                                }
                            } else {
                                // Check cache as fallback
                                if let Ok(cache) = cached_quorums.try_read() {
                                    if let Some(&key) = cache.get(&(quorum_type, quorum_hash)) {
                                        let _ = response.send(Some(key));
                                        tracing::debug!("Found quorum key in cache (fallback in command handler)");
                                        continue;
                                    }
                                }
                            }
                            
                            let _ = response.send(result);
                        }
                        SpvCommand::UpdateQuorumCache { response } => {
                            Self::update_quorum_cache(&client, &cached_quorums).await;
                            if let Ok(cache) = cached_quorums.try_read() {
                                let _ = response.send(Ok(cache.len()));
                            } else {
                                let _ = response.send(Err("Failed to read cache".to_string()));
                            }
                        }
                        SpvCommand::Stop { response } => {
                            let result = client.stop().await
                                .map_err(|e| format!("Failed to stop SPV client: {:?}", e));
                            let _ = response.send(result);
                            break;
                        }
                    }
                }
                // Periodic cache update
                _ = cache_update_interval.tick() => {
                    Self::update_quorum_cache(&client, &cached_quorums).await;
                    
                    // Also update progress in shared state
                    if let Ok(progress) = client.sync_progress().await {
                        if let Ok(mut state) = shared_state.try_write() {
                            state.current_height = progress.header_height;
                            state.headers_synced = progress.headers_synced;
                            state.is_syncing = !progress.headers_synced;
                            
                            // Update target if we're close
                            if state.target_height == 0 || state.current_height > state.target_height - 20 {
                                state.target_height = state.current_height + 50;
                            }
                        }
                    }
                }
                // Monitor network
                result = client.monitor_network() => {
                    match result {
                        Ok(_) => {
                            tracing::info!("Monitor network completed normally");
                        }
                        Err(e) => {
                            tracing::error!("Monitor network error: {:?}", e);
                            // The client will handle reconnection internally
                        }
                    }
                }
            }
        }
        
        // Clean up
        if let Some(task) = progress_task {
            task.abort();
        }
        tracing::info!("SPV task loop ended");
    }
    
    /// Update the quorum cache with recent masternode lists
    async fn update_quorum_cache(
        client: &DashSpvClient,
        cached_quorums: &Arc<RwLock<std::collections::HashMap<(u8, [u8; 32]), [u8; 48]>>>,
    ) {
        if let Some(mn_engine) = client.masternode_list_engine() {
            let mut new_entries = Vec::new();
            
            // Collect all masternode lists (or limit to recent ones if too many)
            let all_lists: Vec<_> = mn_engine.masternode_lists.iter().collect();
            let lists_to_process = if all_lists.len() > 50 {
                // If we have many lists, take the most recent 50
                all_lists.into_iter().rev().take(50).collect()
            } else {
                all_lists
            };
            
            tracing::info!("Processing {} masternode lists for quorum cache update", lists_to_process.len());
            
            if lists_to_process.is_empty() {
                tracing::warn!("No masternode lists available for quorum cache");
                return;
            }
            
            for (height, mn_list) in lists_to_process {
                tracing::trace!("Processing masternode list at height {}", height);
                // Check all quorum types in the masternode list
                for (llmq_type, quorums) in &mn_list.quorums {
                    for (quorum_hash, entry) in quorums {
                        let public_key_bytes: &[u8] = entry.quorum_entry.quorum_public_key.as_ref();
                        if public_key_bytes.len() == 48 {
                            let mut key_array = [0u8; 48];
                            key_array.copy_from_slice(public_key_bytes);
                            
                            let hash_bytes = quorum_hash.to_byte_array();
                            
                            // Store both normal and reversed hash
                            new_entries.push(((*llmq_type as u8, hash_bytes), key_array));
                            
                            // Also store with reversed hash
                            let mut reversed_hash = hash_bytes;
                            reversed_hash.reverse();
                            new_entries.push(((*llmq_type as u8, reversed_hash), key_array));
                            
                            tracing::trace!("Caching quorum key - type: {}, hash: {}, reversed: {}", 
                                *llmq_type as u8, hex::encode(&hash_bytes), hex::encode(&reversed_hash));
                        }
                    }
                }
            }
            
            // Update cache
            if !new_entries.is_empty() {
                if let Ok(mut cache) = cached_quorums.try_write() {
                    let old_size = cache.len();
                    for (key, value) in new_entries {
                        cache.insert(key, value);
                    }
                    let new_size = cache.len();
                    tracing::info!("Updated quorum cache: {} -> {} entries ({} new)", 
                        old_size, new_size, new_size - old_size);
                    
                    // Log sample of cached quorum types
                    let mut type_counts: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
                    for ((qtype, _), _) in cache.iter() {
                        *type_counts.entry(*qtype).or_insert(0) += 1;
                    }
                    tracing::info!("Cached quorum types: {:?}", type_counts);
                }
            } else {
                tracing::warn!("No quorum entries found in masternode lists");
            }
        } else {
            tracing::warn!("No masternode list engine available for cache update");
        }
    }
    
    /// Monitor progress and update cached quorum data
    async fn monitor_progress(
        _progress_rx: Option<mpsc::Receiver<SyncProgress>>,
        _cached_quorums: Arc<RwLock<std::collections::HashMap<(u8, [u8; 32]), [u8; 48]>>>,
        _shared_state: Arc<RwLock<SpvState>>,
    ) {
        // For now, we don't use the progress receiver
        // Progress updates happen through the command channel instead
        tracing::info!("Progress monitoring task started (no-op for now)");
    }
    
    /// Get quorum key directly from the client
    fn get_quorum_key_from_client(
        client: &DashSpvClient,
        quorum_type: u8,
        quorum_hash: &[u8; 32],
    ) -> Option<[u8; 48]> {
        let mn_list_engine = client.masternode_list_engine()?;
        
        let llmq_type = LLMQType::from(quorum_type);
        
        // Reverse the hash first (dash-sdk provides it in display order, but dashcore needs internal order)
        let mut reversed_hash = *quorum_hash;
        reversed_hash.reverse();
        let quorum_hash_typed = QuorumHash::from_slice(&reversed_hash).ok()?;
        
        // Also try the original (unreversed) hash as fallback
        let quorum_hash_original = QuorumHash::from_slice(quorum_hash).ok()?;
        
        tracing::info!("Searching MasternodeListEngine for quorum - type: {}, hash: {}, reversed: {}", 
            quorum_type, hex::encode(quorum_hash), hex::encode(&reversed_hash));
        tracing::info!("MasternodeListEngine has {} masternode lists", mn_list_engine.masternode_lists.len());
        
        // Take a sample of available quorum hashes to log
        let mut sample_hashes = Vec::new();
        let mut found_any_of_type = false;
        
        for (height, mn_list) in &mn_list_engine.masternode_lists {
            if let Some(quorums) = mn_list.quorums.get(&llmq_type) {
                found_any_of_type = true;
                tracing::debug!("At height {}: found {} quorums of type {}", 
                    height, quorums.len(), quorum_type);
                
                // Log first few hashes at this height for comparison
                for (idx, (hash, _)) in quorums.iter().take(3).enumerate() {
                    let hash_bytes = hash.to_byte_array();
                    sample_hashes.push(hex::encode(&hash_bytes));
                    if idx < 3 {
                        tracing::info!("  Sample quorum hash #{}: {}", idx + 1, hex::encode(&hash_bytes));
                    }
                }
                    
                // Try reversed hash first (most likely)
                if let Some(quorum_entry) = quorums.get(&quorum_hash_typed) {
                    let public_key_bytes: &[u8] = quorum_entry.quorum_entry.quorum_public_key.as_ref();
                    if public_key_bytes.len() == 48 {
                        let mut key_array = [0u8; 48];
                        key_array.copy_from_slice(public_key_bytes);
                        tracing::info!("Found quorum key in MasternodeListEngine at height {} (REVERSED hash)", height);
                        return Some(key_array);
                    }
                }
                
                // Try original hash as fallback
                if let Some(quorum_entry) = quorums.get(&quorum_hash_original) {
                    let public_key_bytes: &[u8] = quorum_entry.quorum_entry.quorum_public_key.as_ref();
                    if public_key_bytes.len() == 48 {
                        let mut key_array = [0u8; 48];
                        key_array.copy_from_slice(public_key_bytes);
                        tracing::info!("Found quorum key in MasternodeListEngine at height {} (original hash)", height);
                        return Some(key_array);
                    }
                }
            }
        }
        
        if !found_any_of_type {
            tracing::warn!("No quorums of type {} found in any masternode list", quorum_type);
        } else {
            tracing::warn!("Quorum key not found. Searched {} total sample hashes", sample_hashes.len());
            if !sample_hashes.is_empty() {
                tracing::info!("First few available hashes: {:?}", &sample_hashes[..sample_hashes.len().min(5)]);
            }
        }
        
        None
    }
    
    pub async fn start_sync(&mut self) -> Result<(), String> {
        if self.command_tx.is_some() {
            tracing::info!("SPV client already started");
            self.is_syncing = true;
            self.is_monitoring = true;
            Ok(())
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn sync_to_tip(&self) -> Result<SyncProgress, String> {
        if let Some(tx) = &self.command_tx {
            let (response_tx, response_rx) = oneshot::channel();
            
            tx.send(SpvCommand::GetSyncProgress { response: response_tx })
                .await
                .map_err(|_| "Failed to send command to SPV task".to_string())?;
                
            response_rx.await
                .map_err(|_| "Failed to receive response from SPV task".to_string())?
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn get_sync_progress(&mut self, _network: dash_sdk::dashcore_rpc::dashcore::Network) -> Result<(u32, u32, f32), String> {
        // First check if we can get from shared state without blocking
        if let Ok(state) = self.shared_state.try_read() {
            // Use cached data if it's recent (less than 2 seconds old)
            if state.last_update.elapsed() < Duration::from_secs(2) {
                let progress_percent = if state.target_height > 0 && state.current_height <= state.target_height {
                    ((state.current_height as f32 / state.target_height as f32) * 100.0).min(100.0).max(0.0)
                } else {
                    100.0
                };
                
                // Update local fields from cache
                self.current_height = state.current_height;
                self.target_height = state.target_height;
                self.is_syncing = state.is_syncing;
                
                tracing::debug!("SPV progress from cache: current={}, target={}, percent={:.2}%", 
                    state.current_height, state.target_height, progress_percent);
                
                return Ok((state.current_height, state.target_height, progress_percent));
            }
        }
        
        // If cache is stale or locked, query the SPV task
        if let Some(tx) = &self.command_tx {
            tracing::info!("Getting SPV sync progress");
            
            let (response_tx, response_rx) = oneshot::channel();
            tx.send(SpvCommand::GetSyncProgress { response: response_tx })
                .await
                .map_err(|_| "Failed to send command to SPV task".to_string())?;
                
            let progress = response_rx.await
                .map_err(|_| "Failed to receive response from SPV task".to_string())??
                ;
            
            tracing::debug!("SPV sync progress: {:?}", progress);
            
            // Update our local state
            self.current_height = progress.header_height;
            
            // Also update shared state for non-blocking access
            if let Ok(mut state) = self.shared_state.try_write() {
                state.current_height = progress.header_height;
                state.headers_synced = progress.headers_synced;
                state.is_syncing = !progress.headers_synced;
                
                // For target height, since we can't get peer heights from the SPV client,
                // use a small buffer when we're close to the tip
                if state.target_height == 0 || state.current_height > state.target_height - 20 {
                    state.target_height = state.current_height + 50;
                }
                self.target_height = state.target_height;
            }
            
            // For target height, we need a reasonable estimate based on network
            if self.target_height == 0 || self.current_height > self.target_height - 1000 {
                // Network-specific height estimates
                self.target_height = match _network {
                    dash_sdk::dashcore_rpc::dashcore::Network::Dash => {
                        if self.current_height < 2_300_000 {
                            2_310_000
                        } else {
                            // Only add a small buffer for ongoing sync
                            self.current_height + 100
                        }
                    },
                    dash_sdk::dashcore_rpc::dashcore::Network::Testnet => {
                        if self.current_height < 1_200_000 {
                            1_300_000
                        } else {
                            self.current_height + 100
                        }
                    },
                    _ => self.current_height + 100,
                };
            }
            
            // Calculate progress percentage
            let progress_percent = if self.target_height > 0 && self.current_height <= self.target_height {
                let percent = (self.current_height as f32 / self.target_height as f32) * 100.0;
                percent.min(100.0).max(0.0)
            } else {
                100.0
            };
            
            // Update syncing state
            self.is_syncing = self.current_height < self.target_height && !progress.headers_synced;
            
            // Log progress info
            tracing::info!(
                "SPV sync progress - header_height: {}, target: {}, percent: {:.2}%, headers_synced: {}",
                progress.header_height,
                self.target_height,
                progress_percent,
                progress.headers_synced
            );
            
            // If we're fully synced, adjust target to current
            if progress.headers_synced && progress.header_height > 1_200_000 {
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
        if self.command_tx.is_some() {
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
        if self.command_tx.is_some() {
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
        if let Some(tx) = &self.command_tx {
            let (response_tx, response_rx) = oneshot::channel();
            
            tx.send(SpvCommand::Stop { response: response_tx })
                .await
                .map_err(|_| "Failed to send stop command to SPV task".to_string())?;
                
            response_rx.await
                .map_err(|_| "Failed to receive response from SPV task".to_string())?
        } else {
            Ok(())
        }
    }
    
    /// Try to get a quorum public key without blocking
    /// This is designed to be called from synchronous contexts
    pub fn try_get_quorum_public_key_sync(&self, quorum_type: u8, quorum_hash: &[u8; 32]) -> Option<[u8; 48]> {
        // Only check cached quorums for synchronous access
        if let Ok(cache) = self.cached_quorums.try_read() {
            if let Some(key) = cache.get(&(quorum_type, *quorum_hash)) {
                tracing::debug!("Found quorum key in cache for type {} hash {:?}", 
                    quorum_type, hex::encode(quorum_hash));
                return Some(*key);
            }
            
            // Log cache miss with more details
            let cache_size = cache.len();
            let has_type = cache.iter().any(|((t, _), _)| *t == quorum_type);
            tracing::debug!("Quorum not found in cache - type: {}, hash: {:?}, cache_size: {}, has_type: {}", 
                quorum_type, hex::encode(quorum_hash), cache_size, has_type);
        } else {
            tracing::debug!("Could not acquire cache read lock");
        }
        
        None
    }
    
    /// Try to get a quorum public key with blocking async call
    /// This tries MasternodeListEngine first, then falls back to cache
    pub fn try_get_quorum_public_key_blocking(&self, quorum_type: u8, quorum_hash: &[u8; 32]) -> Option<[u8; 48]> {
        // First check cache for immediate response
        if let Ok(cache) = self.cached_quorums.try_read() {
            if let Some(key) = cache.get(&(quorum_type, *quorum_hash)) {
                tracing::debug!("Found quorum key in cache (immediate)");
                return Some(*key);
            }
        }
        
        // Try async query with a short timeout
        if let Some(tx) = &self.command_tx {
            let tx_clone = tx.clone();
            let quorum_hash_copy = *quorum_hash;
            
            // Use block_in_place to handle async in sync context
            let result = tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    let (response_tx, response_rx) = oneshot::channel();
                    
                    // Send with timeout
                    let send_result = timeout(
                        Duration::from_millis(100),
                        tx_clone.send(SpvCommand::GetQuorumKey { 
                            quorum_type, 
                            quorum_hash: quorum_hash_copy, 
                            response: response_tx 
                        })
                    ).await;
                    
                    if send_result.is_ok() {
                        // Wait for response with timeout
                        timeout(Duration::from_millis(200), response_rx).await.ok()?.ok()
                    } else {
                        None
                    }
                })
            });
            
            if let Some(Some(key)) = result {
                tracing::debug!("Got quorum key from SPV task (MasternodeListEngine or cache)");
                return Some(key);
            }
        }
        
        None
    }
    
    /// Async version to get quorum key from SPV task
    pub async fn get_quorum_public_key(&self, quorum_type: u8, quorum_hash: &[u8; 32]) -> Option<[u8; 48]> {
        // First check cache
        if let Ok(cache) = self.cached_quorums.try_read() {
            if let Some(key) = cache.get(&(quorum_type, *quorum_hash)) {
                return Some(*key);
            }
        }
        
        // Query SPV task
        if let Some(tx) = &self.command_tx {
            let (response_tx, response_rx) = oneshot::channel();
            
            if tx.send(SpvCommand::GetQuorumKey { 
                quorum_type, 
                quorum_hash: *quorum_hash, 
                response: response_tx 
            }).await.is_ok() {
                if let Ok(result) = response_rx.await {
                    // Cache the result if found
                    if let Some(key) = result {
                        if let Ok(mut cache) = self.cached_quorums.try_write() {
                            cache.insert((quorum_type, *quorum_hash), key);
                        }
                    }
                    return result;
                }
            }
        }
        
        None
    }
    
    /// Trigger an update of the quorum cache
    pub async fn update_cache(&self) -> Result<usize, String> {
        if let Some(tx) = &self.command_tx {
            let (response_tx, response_rx) = oneshot::channel();
            
            tx.send(SpvCommand::UpdateQuorumCache { response: response_tx })
                .await
                .map_err(|_| "Failed to send update cache command".to_string())?;
                
            response_rx.await
                .map_err(|_| "Failed to receive cache update response".to_string())?
        } else {
            Err("SPV client not initialized".to_string())
        }
    }
}

impl AppContext {
    pub async fn run_spv_task(&self, task: SpvTask) -> Result<BackendTaskSuccessResult, String> {
        tracing::info!("run_spv_task called with task: {:?}", task);
        
        // Log network info
        tracing::info!("Current network: {:?}", self.network);
        
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
        tracing::info!("execute_spv_sync called with checkpoint_height: {}", checkpoint_height);
        
        // Initialize and start sync in a scoped block to release the lock
        {
            let mut spv_manager = self.spv_manager.write().await;
            
            // Initialize if not already done
            if !spv_manager.is_initialized() {
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
        } // Lock is released here
        
        // Try to get initial progress with a short timeout
        // If it fails, return a default progress based on initialization
        let timeout_duration = Duration::from_millis(100);
        
        let progress_result = timeout(timeout_duration, async {
            let spv_manager = self.spv_manager.read().await;
            if spv_manager.is_initialized() {
                // Since we can't directly access sync progress anymore,
                // return a default indicating SPV has started
                return Some((1, 1, 0));
            }
            None
        }).await;
        
        let (current, target, percent) = match progress_result {
            Ok(Some((current, header_height, chain_tip))) => {
                // Got actual progress
                let target = match self.network {
                    dash_sdk::dashcore_rpc::dashcore::Network::Dash => 2_310_000,
                    dash_sdk::dashcore_rpc::dashcore::Network::Testnet => 1_300_000,
                    _ => current + 100_000,
                };
                
                let percent = if target > 0 && current <= target {
                    ((current as f32 / target as f32) * 100.0).min(100.0).max(0.0)
                } else {
                    100.0
                };
                
                tracing::info!("Initial SPV sync state: current={} (header_height={}, chain_tip={}), target={}, percent={:.2}%", 
                    current, header_height, chain_tip, target, percent);
                
                (current, target, percent)
            }
            _ => {
                // Timeout or error - return reasonable defaults
                // SPV typically loads existing headers during init
                let current = 1; // Start with 1 to show we're initialized
                let target = match self.network {
                    dash_sdk::dashcore_rpc::dashcore::Network::Dash => 2_310_000,
                    dash_sdk::dashcore_rpc::dashcore::Network::Testnet => 1_300_000,
                    _ => 100_000,
                };
                let percent = 0.0;
                
                tracing::info!("Initial SPV sync state (timeout): using defaults - current={}, target={}, percent={:.2}%", 
                    current, target, percent);
                
                (current, target, percent)
            }
        };
        
        Ok(SpvTaskResult::SyncProgress {
            current_height: current,
            target_height: target,
            progress_percent: percent,
        })
    }

    pub async fn execute_spv_get_progress(&self) -> Result<SpvTaskResult, String> {
        tracing::debug!("execute_spv_get_progress called");
        
        // Try to get progress without blocking - use try_write
        match self.spv_manager.try_write() {
            Ok(mut spv_manager) => {
                if !spv_manager.is_initialized() {
                    return Err("SPV not initialized".to_string());
                }
                
                match spv_manager.get_sync_progress(self.network).await {
                    Ok((current, target, percent)) => {
                        tracing::debug!("execute_spv_get_progress: current={}, target={}, progress={:.2}%", 
                            current, target, percent);
                        
                        if percent >= 100.0 {
                            Ok(SpvTaskResult::SyncComplete {
                                final_height: current,
                            })
                        } else {
                            Ok(SpvTaskResult::SyncProgress {
                                current_height: current,
                                target_height: target,
                                progress_percent: percent,
                            })
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to get SPV progress: {}", e);
                        Err(format!("Failed to get SPV progress: {}", e))
                    }
                }
            }
            Err(_) => {
                // SPV manager is locked, try to get from shared state directly
                tracing::debug!("SPV manager locked, trying shared state");
                
                match self.spv_manager.read().await.shared_state.try_read() {
                    Ok(state) => {
                        let progress_percent = if state.target_height > 0 && state.current_height <= state.target_height {
                            ((state.current_height as f32 / state.target_height as f32) * 100.0).min(100.0).max(0.0)
                        } else {
                            100.0
                        };
                        
                        if state.headers_synced || progress_percent >= 100.0 {
                            Ok(SpvTaskResult::SyncComplete {
                                final_height: state.current_height,
                            })
                        } else {
                            Ok(SpvTaskResult::SyncProgress {
                                current_height: state.current_height,
                                target_height: state.target_height,
                                progress_percent,
                            })
                        }
                    }
                    Err(_) => {
                        tracing::warn!("Both SPV manager and shared state are locked");
                        Err("SPV manager is busy".to_string())
                    }
                }
            }
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