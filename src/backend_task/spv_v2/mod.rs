//! SPV V2 module using the new sync engine architecture
//!
//! This module provides a clean separation between sync operations
//! and status queries to avoid concurrency issues.

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
    /// SPV client handle (if running)
    client: Option<DashSpvClient>,

    /// Current sync status
    pub is_syncing: bool,
    pub is_monitoring: bool,
    pub current_height: u32,
    pub target_height: u32,
}

impl std::fmt::Debug for SpvManagerV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvManagerV2")
            .field("has_client", &self.client.is_some())
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

        // Set checkpoint height if provided
        // 0 means start from genesis
        if checkpoint_height > 0 {
            config.start_from_height = Some(checkpoint_height);
            tracing::info!(
                "Setting config.start_from_height = Some({})",
                checkpoint_height
            );
            tracing::info!(
                "Configured to start sync from checkpoint height: {}",
                checkpoint_height
            );
        } else {
            config.start_from_height = Some(0);
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

        // Configure with storage and logging, and set start height
        let config = if checkpoint_height > 0 {
            config
                .with_storage_path(storage_path)
                .with_log_level("warn")
                .with_start_height(checkpoint_height)
        } else {
            config
                .with_storage_path(storage_path)
                .with_log_level("warn")
                .with_start_height(0)
        };

        // Log the final config state
        tracing::info!(
            "Final SPV config: start_from_height = {:?}",
            config.start_from_height
        );

        // Create SPV client
        let mut client = DashSpvClient::new(config)
            .await
            .map_err(|e| format!("Failed to create SPV client: {:?}", e))?;

        tracing::info!("✅ Created SPV client");

        // Start the client
        client
            .start()
            .await
            .map_err(|e| format!("Failed to start SPV client: {:?}", e))?;

        self.client = Some(client);
        self.is_syncing = true;

        tracing::info!("SPV client V2 initialized and started successfully");
        Ok(())
    }

    pub async fn start_sync(&mut self) -> Result<(), String> {
        if self.client.is_some() {
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
        if let Some(client) = &mut self.client {
            // Get sync progress from the client
            let progress = client.sync_progress()
                .await
                .map_err(|e| format!("Failed to get sync progress: {:?}", e))?;

            self.current_height = progress.header_height;
            // For target height, we need to get it from the network (peers)
            // For now, use a reasonable estimate
            self.target_height = if progress.header_height > 0 {
                progress.header_height + 1000  // Estimate
            } else {
                1000  // Initial estimate
            };

            let progress_percent = if self.target_height > 0 {
                (self.current_height as f32 / self.target_height as f32) * 100.0
            } else {
                0.0
            };

            // Update our internal state based on sync status
            self.is_syncing = !progress.headers_synced;

            Ok((
                self.current_height,
                self.target_height,
                progress_percent,
            ))
        } else {
            Err("SPV client not initialized".to_string())
        }
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        if let Some(mut client) = self.client.take() {
            client
                .stop()
                .await
                .map_err(|e| format!("Failed to stop SPV client: {:?}", e))?;
        }

        self.is_syncing = false;
        self.is_monitoring = false;
        self.current_height = 0;
        self.target_height = 0;

        Ok(())
    }

    /// Get a quorum public key from the SPV client
    pub async fn get_quorum_public_key(
        &self,
        quorum_type: u8,
        quorum_hash: &[u8; 32],
    ) -> Option<[u8; 48]> {
        // This would need to be implemented in DashSpvClient
        // For now, return None as the functionality may not be directly available
        tracing::warn!("get_quorum_public_key not yet implemented for direct client access");
        None
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

                // Set initial progress to 0 until we get real data
                let (current, target) = (0, 0);

                Ok(BackendTaskSuccessResult::SpvResultV2(
                    SpvTaskResultV2::SyncProgress {
                        current_height: checkpoint_height,
                        target_height: checkpoint_height + 1000,
                        progress_percent: 0.0,
                    },
                ))
            }
            SpvTaskV2::GetSyncProgress => {
                tracing::debug!("Getting SPV sync progress");
                let (current, target, percent) =
                    spv_manager.get_sync_progress_with_phase().await?;
                
                tracing::info!(
                    "SPV sync progress - current: {}, target: {}, percent: {:.1}%",
                    current,
                    target,
                    percent
                );

                Ok(BackendTaskSuccessResult::SpvResultV2(
                    SpvTaskResultV2::SyncProgress {
                        current_height: current,
                        target_height: target,
                        progress_percent: percent,
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
