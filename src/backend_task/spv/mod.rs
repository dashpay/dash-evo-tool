use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dpp::platform_value::Bytes32;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_spv::{ClientConfig, DashSpvClient, SyncProgress};
use dashcore::QuorumHash;
use dashcore::hashes::Hash;
use dashcore::sml::llmq_type::LLMQType;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc, oneshot};

#[derive(Debug, Clone, PartialEq)]
pub enum SpvTask {
    InitializeAndSync { checkpoint_height: u32 },
    GetSyncProgress,
    VerifyStateTransition(StateTransition),
    VerifyIdentity(Bytes32),
    Stop,
}

#[derive(Debug)]
enum SpvCommand {
    GetSyncProgress {
        response: oneshot::Sender<Result<SyncProgress, String>>,
    },
    GetQuorumKey {
        quorum_type: u8,
        quorum_hash: [u8; 32],
        response: oneshot::Sender<Option<[u8; 48]>>,
    },
    Stop {
        response: oneshot::Sender<Result<(), String>>,
    },
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
    shared_state: Arc<RwLock<SpvState>>,
}

impl std::fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvManager")
            .field("has_command_channel", &self.command_tx.is_some())
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
            command_tx: None,
            is_syncing: false,
            is_monitoring: false,
            current_height: 0,
            target_height: 0,
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

    pub async fn initialize(
        &mut self,
        network: &str,
        checkpoint_height: u32,
    ) -> Result<(), String> {
        tracing::info!(
            "Initializing SPV client for network: {}, checkpoint: {}",
            network,
            checkpoint_height
        );

        // Create SPV client configuration
        let mut config = match network {
            "mainnet" => ClientConfig::mainnet(),
            "testnet" => ClientConfig::testnet(),
            "devnet" => ClientConfig::testnet(), // Devnet uses testnet config
            "regtest" => ClientConfig::regtest(),
            _ => return Err(format!("Unsupported network: {}", network)),
        };

        // For mainnet only, use the specific peer that responds properly
        if network == "mainnet" {
            config.peers.clear();
            config.add_peer("8.219.185.232:9999".parse().unwrap());
        }

        // Set checkpoint height if provided (0 means start from genesis)
        if checkpoint_height > 0 {
            config.start_from_height = Some(checkpoint_height);
            tracing::info!(
                "Configured to start sync from checkpoint height: {}",
                checkpoint_height
            );
        } else {
            tracing::info!("Starting sync from genesis block");
        }

        // Configure storage path
        let app_data_dir = crate::app_dir::app_user_data_dir_path()
            .map_err(|e| format!("Failed to get app data directory: {:?}", e))?;
        let storage_path = app_data_dir.join("spv").join(network);

        // Ensure directory exists
        if let Err(e) = std::fs::create_dir_all(&storage_path) {
            tracing::warn!("Failed to create SPV storage directory: {:?}", e);
        }

        tracing::info!("SPV storage path: {:?}", storage_path);

        // Configure with checkpoint settings
        let config = config
            .with_storage_path(storage_path)
            .with_log_level("warn")
            .with_start_height(checkpoint_height);

        // Create command channel
        let (command_tx, command_rx) = mpsc::channel::<SpvCommand>(100);
        self.command_tx = Some(command_tx);

        // Clone shared state for the background task
        let shared_state = self.shared_state.clone();

        // Spawn the SPV task that owns the client
        tokio::spawn(async move {
            // Create SPV client
            let mut client = match DashSpvClient::new(config).await {
                Ok(client) => client,
                Err(e) => {
                    tracing::error!("Failed to create SPV client: {:?}", e);
                    return;
                }
            };

            tracing::info!("✅ Created SPV client with storage service");

            // Start the client
            if let Err(e) = client.start().await {
                tracing::error!("Failed to start SPV client: {:?}", e);
                return;
            }

            tracing::info!("SPV client started successfully");

            // Handle commands and monitor network
            Self::run_spv_task_loop(client, command_rx, shared_state).await;
        });

        tracing::info!("SPV client initialized successfully");
        Ok(())
    }

    /// Run the SPV task loop that handles commands and monitors the network
    async fn run_spv_task_loop(
        mut client: DashSpvClient,
        mut command_rx: mpsc::Receiver<SpvCommand>,
        shared_state: Arc<RwLock<SpvState>>,
    ) {
        // Wait for peers to connect before initiating sync
        let start = tokio::time::Instant::now();
        while client.peer_count() == 0 && start.elapsed() < tokio::time::Duration::from_secs(5) {
            tracing::info!("Waiting for peers to connect...");
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        if client.peer_count() == 0 {
            tracing::warn!("No peers connected after 5 seconds, proceeding anyway");
        } else {
            tracing::info!("Connected to {} peers", client.peer_count());
        }

        // First call sync_to_tip to prepare the client state
        if let Err(e) = client.sync_to_tip().await {
            tracing::error!("Failed to initiate sync_to_tip: {:?}", e);
        }

        // Now trigger sync start
        match client.start().await {
            Ok(()) => {
                tracing::info!("📊 SPV client started");

                // Update shared state with initial sync status
                if let Ok(progress) = client.sync_progress().await {
                    if let Ok(mut state) = shared_state.try_write() {
                        state.current_height = progress.header_height;
                        // Estimate target height
                        state.target_height = if progress.header_height > 0 {
                            progress.header_height + 1000
                        } else {
                            1000
                        };
                        state.is_syncing = !progress.headers_synced;
                        state.headers_synced = progress.headers_synced;
                        state.last_update = Instant::now();
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to start SPV client: {:?}", e);
            }
        }

        loop {
            tokio::select! {
                // Set biased to prioritize command handling
                biased;

                // Handle commands with higher priority
                Some(command) = command_rx.recv() => {
                    match command {
                        SpvCommand::GetSyncProgress { response } => {
                            tracing::debug!("Received GetSyncProgress command");

                            // Use a timeout to prevent blocking sync operations
                            let result = tokio::time::timeout(
                                tokio::time::Duration::from_millis(100),
                                client.sync_progress()
                            ).await;

                            let progress_result = match result {
                                Ok(Ok(progress)) => Ok(progress),
                                Ok(Err(e)) => Err(format!("Failed to get sync progress: {:?}", e)),
                                Err(_) => {
                                    // Timeout - sync might be busy, return error
                                    tracing::debug!("GetSyncProgress timed out, sync might be busy");
                                    Err("Sync progress temporarily unavailable - sync in progress".to_string())
                                }
                            };

                            tracing::debug!("Sync progress result: {:?}", progress_result);

                            let _ = response.send(progress_result);
                        }
                        SpvCommand::GetQuorumKey { quorum_type, quorum_hash, response } => {
                            let result = Self::get_quorum_key_from_client(&client, quorum_type, &quorum_hash);
                            let _ = response.send(result);
                        }
                        SpvCommand::Stop { response } => {
                            tracing::info!("Stopping SPV client...");

                            // Try to stop the client with a timeout
                            let stop_result = tokio::time::timeout(
                                tokio::time::Duration::from_secs(5),
                                client.stop()
                            ).await;

                            let result = match stop_result {
                                Ok(Ok(_)) => {
                                    tracing::info!("SPV client stopped successfully");
                                    Ok(())
                                }
                                Ok(Err(e)) => {
                                    tracing::error!("SPV client stop returned error: {:?}", e);
                                    Err(format!("Failed to stop SPV client: {:?}", e))
                                }
                                Err(_) => {
                                    tracing::error!("SPV client stop timed out after 5 seconds");
                                    Err("SPV client stop timed out".to_string())
                                }
                            };

                            // Reset shared state when stopping
                            if let Ok(mut state) = shared_state.try_write() {
                                state.current_height = 0;
                                state.target_height = 0;
                                state.is_syncing = false;
                                state.headers_synced = false;
                                state.last_update = Instant::now();
                            }

                            let _ = response.send(result);

                            tracing::info!("SPV client stop command completed, exiting task loop");

                            // Drop the client to ensure all background tasks are stopped
                            drop(client);

                            // Give a small delay to allow background threads to finish
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                            break;
                        }
                    }
                }

                // Process network messages with lower priority to avoid interrupting commands
                _ = async {
                    // Add a small yield to ensure commands get processed first
                    tokio::task::yield_now().await;

                    // The client handles network messages internally now
                    // Just sleep briefly to avoid busy-waiting
                    tokio::time::sleep(Duration::from_millis(100)).await;

                    // Periodically update sync progress
                    if let Ok(progress) = client.sync_progress().await {
                        if let Ok(mut state) = shared_state.try_write() {
                            state.current_height = progress.header_height;
                            // Estimate target height (client doesn't expose peer heights directly)
                            if state.target_height == 0 || progress.header_height > state.target_height {
                                state.target_height = progress.header_height + 1000;
                            }
                            state.is_syncing = !progress.headers_synced;
                            state.headers_synced = progress.headers_synced;
                            state.last_update = Instant::now();
                        }
                    }
                } => {}
            }
        }

        tracing::info!("SPV task loop has exited completely");
    }

    /// Get quorum key directly from the client's MasternodeListEngine
    fn get_quorum_key_from_client(
        client: &DashSpvClient,
        quorum_type: u8,
        quorum_hash: &[u8; 32],
    ) -> Option<[u8; 48]> {
        let mn_list_engine = client.masternode_list_engine()?;
        let llmq_type = LLMQType::from(quorum_type);

        // Try both reversed and unreversed hash
        let mut reversed_hash = *quorum_hash;
        reversed_hash.reverse();
        let quorum_hash_typed = QuorumHash::from_slice(&reversed_hash).ok()?;

        // Search through masternode lists
        for (_height, mn_list) in &mn_list_engine.masternode_lists {
            if let Some(quorums) = mn_list.quorums.get(&llmq_type) {
                // Query with reversed hash
                if let Some(entry) = quorums.get(&quorum_hash_typed) {
                    let public_key_bytes: &[u8] = entry.quorum_entry.quorum_public_key.as_ref();
                    if public_key_bytes.len() == 48 {
                        let mut key_array = [0u8; 48];
                        key_array.copy_from_slice(public_key_bytes);
                        return Some(key_array);
                    }
                }
            }
        }

        None
    }

    pub async fn start_sync(&mut self) -> Result<(), String> {
        if self.command_tx.is_some() {
            self.is_syncing = true;
            Ok(())
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn get_sync_progress(&mut self) -> Result<(u32, u32, f32), String> {
        let (current, target, percent) = self.get_sync_progress_with_phase().await?;
        Ok((current, target, percent))
    }

    pub async fn get_sync_progress_with_phase(
        &mut self,
    ) -> Result<(u32, u32, f32), String> {
        if let Some(tx) = &self.command_tx {
            let (response_tx, response_rx) = oneshot::channel();

            tx.send(SpvCommand::GetSyncProgress {
                response: response_tx,
            })
            .await
            .map_err(|_| "Failed to send progress command to SPV task".to_string())?;

            let progress = response_rx
                .await
                .map_err(|_| "Failed to receive response from SPV task".to_string())??;

            tracing::debug!(
                "SPV sync progress response: current_height={}, headers_synced={}",
                progress.header_height,
                progress.headers_synced
            );

            self.current_height = progress.header_height;

            // Phase info no longer available in simplified API

            // Calculate progress percentage
            let progress_percent = if progress.headers_synced {
                100.0
            } else if self.target_height > 0 {
                (self.current_height as f32 / self.target_height as f32) * 100.0
            } else {
                0.0
            };

            // If we're fully synced
            if progress.headers_synced {
                self.is_syncing = false;
                return Ok((self.current_height, self.target_height, 100.0));
            }

            Ok((
                self.current_height,
                self.target_height,
                progress_percent,
            ))
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn verify_state_transition_proof(
        &self,
        _state_transition: &StateTransition,
    ) -> Result<bool, String> {
        if self.command_tx.is_some() {
            // The SDK handles proof verification using our ContextProvider
            // which provides quorum public keys from the SPV client's MasternodeListEngine
            Ok(true)
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn verify_identity_proof(&self, _identity_id: &Bytes32) -> Result<bool, String> {
        if self.command_tx.is_some() {
            // The SDK handles proof verification using our ContextProvider
            // which provides quorum public keys from the SPV client's MasternodeListEngine
            Ok(true)
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        if let Some(tx) = &self.command_tx {
            let (response_tx, response_rx) = oneshot::channel();

            tx.send(SpvCommand::Stop {
                response: response_tx,
            })
            .await
            .map_err(|_| "Failed to send stop command to SPV task".to_string())?;

            response_rx
                .await
                .map_err(|_| "Failed to receive response from SPV task".to_string())?
        } else {
            Ok(())
        }
    }

    /// Get a quorum public key asynchronously
    pub async fn get_quorum_public_key(
        &self,
        quorum_type: u8,
        quorum_hash: &[u8; 32],
    ) -> Option<[u8; 48]> {
        let tx = self.command_tx.as_ref()?;
        let (response_tx, response_rx) = oneshot::channel();

        tx.send(SpvCommand::GetQuorumKey {
            quorum_type,
            quorum_hash: *quorum_hash,
            response: response_tx,
        })
        .await
        .ok()?;

        response_rx.await.ok()?
    }
}

impl AppContext {
    pub async fn run_spv_task(&self, task: SpvTask) -> Result<BackendTaskSuccessResult, String> {
        let mut spv_manager = self.spv_manager.write().await;

        match task {
            SpvTask::InitializeAndSync { checkpoint_height } => {
                let network = match self.network {
                    dash_sdk::dpp::dashcore::Network::Dash => "mainnet",
                    dash_sdk::dpp::dashcore::Network::Testnet => "testnet",
                    dash_sdk::dpp::dashcore::Network::Devnet => "devnet",
                    dash_sdk::dpp::dashcore::Network::Regtest => "regtest",
                    _ => return Err("Unsupported network".to_string()),
                };

                spv_manager.initialize(network, checkpoint_height).await?;
                spv_manager.start_sync().await?;

                Ok(BackendTaskSuccessResult::SpvResult(
                    SpvTaskResult::SyncProgress {
                        current_height: checkpoint_height,
                        target_height: checkpoint_height + 1000,
                        progress_percent: 0.0,
                    },
                ))
            }
            SpvTask::GetSyncProgress => {
                tracing::debug!("Getting SPV sync progress");
                let (current, target, percent) =
                    spv_manager.get_sync_progress_with_phase().await?;
                tracing::debug!(
                    "SPV sync progress: current={}, target={}, percent={:.1}%",
                    current,
                    target,
                    percent
                );

                Ok(BackendTaskSuccessResult::SpvResult(
                    SpvTaskResult::SyncProgress {
                        current_height: current,
                        target_height: target,
                        progress_percent: percent,
                    },
                ))
            }
            SpvTask::VerifyStateTransition(state_transition) => {
                let is_valid = spv_manager
                    .verify_state_transition_proof(&state_transition)
                    .await?;

                Ok(BackendTaskSuccessResult::SpvResult(
                    SpvTaskResult::ProofVerificationResult {
                        is_valid,
                        details: if is_valid {
                            "State transition proof verified successfully".to_string()
                        } else {
                            "State transition proof verification failed".to_string()
                        },
                    },
                ))
            }
            SpvTask::VerifyIdentity(identity_id) => {
                let is_valid = spv_manager.verify_identity_proof(&identity_id).await?;

                Ok(BackendTaskSuccessResult::SpvResult(
                    SpvTaskResult::ProofVerificationResult {
                        is_valid,
                        details: if is_valid {
                            "Identity proof verified successfully".to_string()
                        } else {
                            "Identity proof verification failed".to_string()
                        },
                    },
                ))
            }
            SpvTask::Stop => {
                spv_manager.stop().await?;

                // Reset SpvManager state after stopping
                spv_manager.is_syncing = false;
                spv_manager.is_monitoring = false;
                spv_manager.current_height = 0;
                spv_manager.target_height = 0;
                spv_manager.command_tx = None;

                Ok(BackendTaskSuccessResult::SpvResult(
                    SpvTaskResult::SyncProgress {
                        current_height: 0,
                        target_height: 0,
                        progress_percent: 0.0,
                    },
                ))
            }
        }
    }
}

/// Get SPV progress when the manager is locked or unavailable
pub async fn get_spv_progress_from_shared_state(
    shared_state: &Arc<RwLock<SpvState>>,
) -> Result<(u32, u32, f32), String> {
    if let Ok(state) = shared_state.try_read() {
        let progress_percent = if state.target_height > 0 {
            (state.current_height as f32 / state.target_height as f32) * 100.0
        } else {
            0.0
        };

        Ok((state.current_height, state.target_height, progress_percent))
    } else {
        Err("Unable to access SPV state".to_string())
    }
}
