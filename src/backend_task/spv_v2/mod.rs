//! SPV V2 module using the new sync engine architecture
//!
//! This module provides a clean separation between sync operations
//! and status queries to avoid concurrency issues.

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dpp::platform_value::Bytes32;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_spv::sync::sync_engine::SyncEngine;
use dash_spv::sync::sync_state::SyncStateReader;
use dash_spv::types::{NetworkEvent, SyncPhaseInfo};
use dash_spv::{ClientConfig, DashSpvClient, SyncProgress};
use dashcore::QuorumHash;
use dashcore::hashes::Hash;
use dashcore::sml::llmq_type::LLMQType;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc, oneshot};

#[derive(Debug, Clone, PartialEq)]
pub enum SpvTaskV2 {
    InitializeAndSync { checkpoint_height: u32 },
    GetSyncProgress,
    VerifyStateTransition(StateTransition),
    VerifyIdentity(Bytes32),
    Stop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpvTaskResultV2 {
    SyncProgress {
        current_height: u32,
        target_height: u32,
        progress_percent: f32,
        phase_info: Option<SyncPhaseInfo>,
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

/// SPV Manager V2 with improved architecture
pub struct SpvManagerV2 {
    /// Sync engine handle (if running)
    sync_engine: Option<SyncEngine>,

    /// Sync state reader for concurrent access
    sync_state_reader: Option<SyncStateReader>,

    /// Current sync status
    pub is_syncing: bool,
    pub is_monitoring: bool,
    pub current_height: u32,
    pub target_height: u32,
}

impl std::fmt::Debug for SpvManagerV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvManagerV2")
            .field("has_sync_engine", &self.sync_engine.is_some())
            .field("is_syncing", &self.is_syncing)
            .field("is_monitoring", &self.is_monitoring)
            .field("current_height", &self.current_height)
            .field("target_height", &self.target_height)
            .finish()
    }
}

impl SpvManagerV2 {
    pub fn new() -> Self {
        Self {
            sync_engine: None,
            sync_state_reader: None,
            is_syncing: false,
            is_monitoring: false,
            current_height: 0,
            target_height: 0,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.sync_engine.is_some()
    }

    pub async fn initialize(
        &mut self,
        network: &str,
        checkpoint_height: u32,
    ) -> Result<(), String> {
        tracing::info!(
            "Initializing SPV client V2 for network: {}, checkpoint: {}",
            network,
            checkpoint_height
        );

        // Create SPV client configuration
        let config = match network {
            "mainnet" => ClientConfig::mainnet(),
            "testnet" => ClientConfig::testnet(),
            "devnet" => ClientConfig::testnet(), // Devnet uses testnet config
            "regtest" => ClientConfig::regtest(),
            _ => return Err(format!("Unsupported network: {}", network)),
        };

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

        // Create SPV client
        let client = DashSpvClient::new_with_storage_service(config)
            .await
            .map_err(|e| format!("Failed to create SPV client: {:?}", e))?;

        tracing::info!("✅ Created SPV client with storage service");

        // Create sync engine
        let mut sync_engine = SyncEngine::new(client);

        // Get state reader before starting
        let sync_state_reader = sync_engine.state_reader();

        // Start the sync engine
        sync_engine
            .start()
            .await
            .map_err(|e| format!("Failed to start sync engine: {:?}", e))?;

        self.sync_engine = Some(sync_engine);
        self.sync_state_reader = Some(sync_state_reader);
        self.is_syncing = true;

        tracing::info!("SPV client V2 initialized successfully");
        Ok(())
    }

    pub async fn start_sync(&mut self) -> Result<(), String> {
        if self.sync_engine.is_some() {
            self.is_syncing = true;
            Ok(())
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn get_sync_progress(&mut self) -> Result<(u32, u32, f32), String> {
        let (current, target, percent, _) = self.get_sync_progress_with_phase().await?;
        Ok((current, target, percent))
    }

    pub async fn get_sync_progress_with_phase(
        &mut self,
    ) -> Result<(u32, u32, f32, Option<SyncPhaseInfo>), String> {
        if let Some(reader) = &self.sync_state_reader {
            // Get state from the reader (non-blocking)
            let state = reader.get_state().await;

            self.current_height = state.current_height;
            self.target_height = state.target_height;

            let progress_percent = if state.target_height > 0 {
                (state.current_height as f32 / state.target_height as f32) * 100.0
            } else {
                0.0
            };

            // Update our internal state
            self.is_syncing = !matches!(
                state.phase,
                dash_spv::sync::sync_state::SyncPhase::Idle
                    | dash_spv::sync::sync_state::SyncPhase::Synced
            );

            Ok((
                state.current_height,
                state.target_height,
                progress_percent,
                state.phase_info,
            ))
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        if let Some(mut engine) = self.sync_engine.take() {
            engine
                .stop()
                .await
                .map_err(|e| format!("Failed to stop sync engine: {:?}", e))?;
        }

        self.sync_state_reader = None;
        self.is_syncing = false;
        self.is_monitoring = false;
        self.current_height = 0;
        self.target_height = 0;

        Ok(())
    }

    /// Get a quorum public key from the sync engine's client
    pub async fn get_quorum_public_key(
        &self,
        quorum_type: u8,
        quorum_hash: &[u8; 32],
    ) -> Option<[u8; 48]> {
        let engine = self.sync_engine.as_ref()?;

        // Call the sync engine's get_quorum_public_key method
        match engine.get_quorum_public_key(quorum_type, quorum_hash).await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("Failed to get quorum public key: {:?}", e);
                None
            }
        }
    }
}

impl AppContext {
    pub async fn run_spv_v2_task(
        &self,
        task: SpvTaskV2,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut spv_manager = self.spv_manager_v2.write().await;

        match task {
            SpvTaskV2::InitializeAndSync { checkpoint_height } => {
                let network = match self.network {
                    dash_sdk::dpp::dashcore::Network::Dash => "mainnet",
                    dash_sdk::dpp::dashcore::Network::Testnet => "testnet",
                    dash_sdk::dpp::dashcore::Network::Devnet => "devnet",
                    dash_sdk::dpp::dashcore::Network::Regtest => "regtest",
                    _ => return Err("Unsupported network".to_string()),
                };

                spv_manager.initialize(network, checkpoint_height).await?;
                spv_manager.start_sync().await?;

                Ok(BackendTaskSuccessResult::SpvResultV2(
                    SpvTaskResultV2::SyncProgress {
                        current_height: checkpoint_height,
                        target_height: checkpoint_height + 1000,
                        progress_percent: 0.0,
                        phase_info: None,
                    },
                ))
            }
            SpvTaskV2::GetSyncProgress => {
                tracing::debug!("Getting SPV sync progress");
                let (current, target, percent, phase_info) =
                    spv_manager.get_sync_progress_with_phase().await?;
                tracing::debug!(
                    "SPV sync progress: current={}, target={}, percent={:.1}%",
                    current,
                    target,
                    percent
                );

                Ok(BackendTaskSuccessResult::SpvResultV2(
                    SpvTaskResultV2::SyncProgress {
                        current_height: current,
                        target_height: target,
                        progress_percent: percent,
                        phase_info,
                    },
                ))
            }
            SpvTaskV2::Stop => {
                spv_manager.stop().await?;

                Ok(BackendTaskSuccessResult::SpvResultV2(
                    SpvTaskResultV2::SyncProgress {
                        current_height: 0,
                        target_height: 0,
                        progress_percent: 0.0,
                        phase_info: None,
                    },
                ))
            }
            _ => {
                // Not implemented yet
                Err("Operation not implemented in V2".to_string())
            }
        }
    }
}
