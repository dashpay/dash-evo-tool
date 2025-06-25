use dash_sdk::dpp::dashcore::Network;
use dash_spv::{init_logging, ClientConfig, DashSpvClient, MasternodeListEngine};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct SpvManager {
    data_dir: PathBuf,
    network: Network,
    client: Arc<RwLock<Option<DashSpvClient>>>,
    is_running: Arc<RwLock<bool>>,
    app_context: Arc<RwLock<Option<Weak<crate::context::AppContext>>>>,
}

impl std::fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvManager")
            .field("data_dir", &self.data_dir)
            .field("network", &self.network)
            .field("client", &"<DashSpvClient>")
            .field("is_running", &self.is_running)
            .field("app_context", &"<AppContext>")
            .finish()
    }
}

impl SpvManager {
    pub fn new(data_dir: PathBuf, network: Network) -> Self {
        Self {
            data_dir,
            network,
            client: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            app_context: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn bind_app_context(&self, app_context: Arc<crate::context::AppContext>) {
        let mut ctx_guard = self.app_context.write().await;
        *ctx_guard = Some(Arc::downgrade(&app_context));
    }

    async fn update_status(
        &self,
        is_running: bool,
        header_height: Option<u32>,
        filter_height: Option<u32>,
    ) {
        let ctx_guard = self.app_context.read().await;
        if let Some(weak_ctx) = ctx_guard.as_ref() {
            if let Some(app_context) = weak_ctx.upgrade() {
                if let Ok(mut status) = app_context.spv_status.lock() {
                    status.is_running = is_running;
                    status.header_height = header_height;
                    status.filter_height = filter_height;
                    status.last_updated = std::time::Instant::now();
                }
            }
        }
    }

    async fn start_progress_updater(&self) {
        let spv_manager = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;

                // Check if SPV is still running
                let is_running = *spv_manager.is_running.read().await;
                if !is_running {
                    break;
                }

                // Update sync progress
                if let Ok((headers, filters)) = spv_manager.get_sync_progress().await {
                    // Progress is automatically updated in get_sync_progress

                    // Log sync progress every 30 seconds if not syncing
                    static mut LAST_LOG: Option<std::time::Instant> = None;
                    let should_log = unsafe {
                        if LAST_LOG.is_none()
                            || LAST_LOG.unwrap().elapsed() > std::time::Duration::from_secs(30)
                        {
                            LAST_LOG = Some(std::time::Instant::now());
                            true
                        } else {
                            false
                        }
                    };

                    if should_log && headers == 0 {
                        tracing::warn!(
                            "SPV client not syncing - headers: {}, filters: {}. Possible issues:",
                            headers,
                            filters
                        );
                        tracing::warn!("- All testnet peers are rejecting connections (closing immediately after version message)");
                        tracing::warn!("- This SPV implementation now advertises NETWORK_LIMITED service but peers still reject");
                        tracing::warn!("- Public testnet nodes may require additional protocol features not implemented in this SPV client");
                        tracing::warn!("- Consider using a local testnet node or Dash Core connection instead");
                        tracing::warn!(
                            "- If using a local node, ensure it's configured to accept connections on port {}",
                            if spv_manager.network == Network::Dash {
                                "9999"
                            } else {
                                "19999"
                            }
                        );
                    }

                    // Check masternode list engine status periodically
                    let guard = spv_manager.client.read().await;
                    if let Some(client) = guard.as_ref() {
                        if let Some(engine) = client.masternode_list_engine() {
                            let mn_count = engine.masternode_lists.len();
                            let quorum_types = engine.quorum_statuses.len();
                            if mn_count > 0 || quorum_types > 0 {
                                tracing::info!(
                                    "MasternodeListEngine status: {} masternode lists, {} quorum types",
                                    mn_count,
                                    quorum_types
                                );
                            }
                        }
                    }
                }
            }
        });
    }

    pub async fn start(&self) -> Result<(), String> {
        // Initialize logging for SPV with debug level for better diagnostics
        let _ = init_logging("debug");

        // Stop any existing client
        self.stop().await?;

        // Configure SPV client based on network
        let mut config = match self.network {
            Network::Dash => ClientConfig::mainnet(),
            Network::Testnet => ClientConfig::testnet(),
            Network::Devnet => return Err("SPV client does not support devnet".to_string()),
            Network::Regtest => return Err("SPV client does not support regtest".to_string()),
            _ => return Err("Unsupported network".to_string()),
        };

        // Add wallet addresses as watch items BEFORE starting
        let ctx_guard = self.app_context.read().await;
        if let Some(weak_ctx) = ctx_guard.as_ref() {
            if let Some(app_context) = weak_ctx.upgrade() {
                let wallets = app_context.wallets.read().unwrap();
                let mut total_addresses = 0;

                for (_seed_hash, wallet_arc) in wallets.iter() {
                    let wallet = wallet_arc.read().unwrap();

                    // Add all known addresses from the wallet
                    for (address, _derivation_path) in wallet.known_addresses.iter() {
                        // Convert from dash_sdk address to dash_spv address
                        let script_bytes = address.script_pubkey().to_bytes();
                        let spv_script = dash_spv::ScriptBuf::from(script_bytes);

                        // Convert network type
                        let spv_network = match self.network {
                            dash_sdk::dpp::dashcore::Network::Dash => dash_spv::Network::Dash,
                            dash_sdk::dpp::dashcore::Network::Testnet => dash_spv::Network::Testnet,
                            dash_sdk::dpp::dashcore::Network::Devnet => dash_spv::Network::Devnet,
                            dash_sdk::dpp::dashcore::Network::Regtest => dash_spv::Network::Regtest,
                            _ => {
                                tracing::warn!("Unsupported network type for address {}", address);
                                continue;
                            }
                        };

                        // Reconstruct the address using dash_spv types
                        if let Ok(spv_address) =
                            dash_spv::Address::from_script(&spv_script, spv_network)
                        {
                            config = config.watch_address(spv_address);
                            total_addresses += 1;
                            tracing::info!("Added watch item: Address {{ address: {} }}", address);
                        } else {
                            tracing::warn!("Failed to convert address {} to SPV format", address);
                        }
                    }
                }

                if total_addresses > 0 {
                    tracing::info!(
                        "Added {} wallet addresses as SPV watch items for optimized sync",
                        total_addresses
                    );
                } else {
                    tracing::warn!(
                        "No wallet addresses found to watch. SPV sync will be slow without watch addresses."
                    );
                    
                    // Add some common testnet addresses to watch for testing
                    if self.network == Network::Testnet {
                        tracing::info!("Adding default testnet watch addresses for testing...");
                        
                        // Add some well-known testnet addresses
                        let test_addresses = vec![
                            "yTsGq4wV8WF5GKLaYV2C43zrkr2sfTtysT", // Testnet faucet
                            "yNPbcFfabtNmmxKdGwhHomdYfVs6gikbPf", // Common testnet address
                        ];
                        
                        for addr_str in test_addresses {
                            // Parse directly as dash_spv address and assume network
                            if let Ok(spv_address) = dash_spv::Address::from_str(addr_str) {
                                // Assume the address network matches our testnet
                                let spv_address = spv_address.assume_checked();
                                config = config.watch_address(spv_address);
                                tracing::info!("Added default watch address: {}", addr_str);
                            }
                        }
                    }
                }
            }
        }

        // Override default peers with working ones
        match self.network {
            Network::Dash => {
                // Use IP addresses of known working Dash mainnet peers
                config.peers = vec![
                    "188.40.190.52:9999".parse().unwrap(),    // Reliable mainnet peer
                    "162.243.219.25:9999".parse().unwrap(),   // Reliable mainnet peer
                    "95.216.255.72:9999".parse().unwrap(),    // Reliable mainnet peer
                    "139.59.254.15:9999".parse().unwrap(),    // Reliable mainnet peer
                    "188.166.156.58:9999".parse().unwrap(),   // Reliable mainnet peer
                    "82.211.21.195:9999".parse().unwrap(),    // Reliable mainnet peer
                ];
            }
            Network::Testnet => {
                // Use the specific testnet peer provided
                config.peers = vec![
                    "139.60.95.71:19999".parse().unwrap(),    // Specific testnet peer to test
                ];
            }
            _ => {}
        }

        // Configure for exclusive peer mode like colleague's demo
        let mut config = config
            .with_storage_path(self.data_dir.join("spv").join(self.network.to_string()))
            .with_validation_mode(dash_spv::ValidationMode::Basic)
            .with_log_level("debug") // Increase logging to see what's happening
            .with_max_concurrent_filter_requests(1) // Very conservative - only 1 request at a time
            .with_filter_flow_control(true) // Enable flow control
            .with_filter_request_delay(500) // Longer delay - 500ms between requests
            .with_connection_timeout(std::time::Duration::from_secs(60)); // Longer connection timeout

        // Try with multiple peers first to find a working one
        config.max_peers = 3; // Allow multiple peers to increase chances of connection

        // Log peer configuration
        tracing::info!(
            "SPV client configured for {} with {} peers (max_peers={}): {:?}",
            self.network,
            config.peers.len(),
            config.max_peers,
            config.peers
        );

        // Create and start client
        let mut client = DashSpvClient::new(config)
            .await
            .map_err(|e| format!("Failed to create SPV client: {}", e))?;

        tracing::info!("SPV client created, starting...");

        client
            .start()
            .await
            .map_err(|e| format!("Failed to start SPV client: {}", e))?;

        // Log the client status
        tracing::info!("SPV client started successfully");

        // Don't block the connection switch on header sync
        // Just give it a moment to establish peer connections
        tracing::info!("Waiting briefly for peer connections...");
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Check initial sync status but don't wait for headers
        if let Ok(progress) = client.sync_progress().await {
            if progress.header_height > 0 {
                tracing::info!(
                    "Initial sync already started: headers={}, filter_headers={}",
                    progress.header_height,
                    progress.filter_header_height
                );
            } else {
                tracing::info!("SPV client connected but headers not synced yet. Sync will continue in background.");
                tracing::info!(
                    "Note: Header sync may take 10-30 seconds depending on network conditions."
                );
            }
        }

        // Check if masternode list engine is available
        if let Some(engine) = client.masternode_list_engine() {
            tracing::info!("MasternodeListEngine is available at startup");
            tracing::info!(
                "Initial masternode lists: {}",
                engine.masternode_lists.len()
            );
        } else {
            tracing::warn!("MasternodeListEngine is NOT available at startup");
        }

        // Store the client
        {
            let mut client_guard = self.client.write().await;
            *client_guard = Some(client);
        }
        *self.is_running.write().await = true;

        // Update status
        self.update_status(true, None, None).await;

        // Log that we're ready to sync (like colleague's demo)
        tracing::info!("Ready for sync!");
        
        // IMPORTANT: Don't start monitor_network immediately - this causes the connection to fail
        // In colleague's demo, sync is started with a separate button press
        // For now, let's comment this out to see if we can establish connection first
        /*
        // Start the monitor_network loop - THIS IS CRITICAL for header sync!
        tracing::info!("Starting monitor_network loop - this will send header requests");
        let spv_manager = self.clone();
        tokio::spawn(async move {
            // Get the client once and run monitor_network
            let mut client_guard = spv_manager.client.write().await;
            if let Some(client) = client_guard.as_mut() {
                tracing::info!("Running monitor_network - this handles all sync operations");
                if let Err(e) = client.monitor_network().await {
                    tracing::error!("monitor_network failed: {}", e);
                }
            } else {
                tracing::error!("No SPV client available for monitor_network");
            }
        });
        */

        // Start background task to periodically update sync progress
        self.start_progress_updater().await;

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), String> {
        *self.is_running.write().await = false;

        // Stop client if it exists
        if let Some(mut client) = self.client.write().await.take() {
            client
                .stop()
                .await
                .map_err(|e| format!("Failed to stop SPV client: {}", e))?;
        }

        // Update status
        self.update_status(false, None, None).await;

        Ok(())
    }

    // sync_to_tip is not needed anymore - monitor_network handles all syncing
    // The sync_to_tip method in dash-spv doesn't actually send sync requests,
    // it just prepares the state. All actual syncing happens in monitor_network.

    /// Wait for masternode lists to be available
    pub async fn wait_for_masternode_lists(&self, timeout_secs: u64) -> Result<(), String> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            if start.elapsed() > timeout {
                return Err("Timeout waiting for masternode lists".to_string());
            }

            let has_lists = {
                let client_guard = self.client.read().await;
                if let Some(client) = client_guard.as_ref() {
                    if let Some(engine) = client.masternode_list_engine() {
                        let count = engine.masternode_lists.len();
                        if count > 0 {
                            tracing::info!(
                                "MasternodeListEngine now has {} masternode lists",
                                count
                            );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            };

            if has_lists {
                return Ok(());
            }

            // Wait a bit before checking again
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    pub async fn get_sync_progress(&self) -> Result<(u32, u32), String> {
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref() {
            let progress = client
                .sync_progress()
                .await
                .map_err(|e| format!("Failed to get sync progress: {}", e))?;

            // Update cached status with latest progress
            self.update_status(
                true,
                Some(progress.header_height),
                Some(progress.filter_header_height),
            )
            .await;

            Ok((progress.header_height, progress.filter_header_height))
        } else {
            Err("SPV client not started".to_string())
        }
    }

    /// Add a wallet address to watch. This can be called after SPV is already running.
    pub async fn add_watch_address(
        &self,
        address: dash_sdk::dpp::dashcore::Address,
    ) -> Result<(), String> {
        let mut client_guard = self.client.write().await;
        if let Some(client) = client_guard.as_mut() {
            // Convert from dash_sdk address to dash_spv address
            // Since the types are from different crate versions, we need to convert via bytes
            let script_bytes = address.script_pubkey().to_bytes();
            let spv_script = dash_spv::ScriptBuf::from(script_bytes);

            // Convert network type
            let spv_network = match self.network {
                dash_sdk::dpp::dashcore::Network::Dash => dash_spv::Network::Dash,
                dash_sdk::dpp::dashcore::Network::Testnet => dash_spv::Network::Testnet,
                dash_sdk::dpp::dashcore::Network::Devnet => dash_spv::Network::Devnet,
                dash_sdk::dpp::dashcore::Network::Regtest => dash_spv::Network::Regtest,
                _ => {
                    return Err(format!("Unsupported network type for address {}", address));
                }
            };

            // Reconstruct the address using dash_spv types
            match dash_spv::Address::from_script(&spv_script, spv_network) {
                Ok(spv_address) => {
                    client
                        .add_watch_item(dash_spv::WatchItem::address(spv_address))
                        .await
                        .map_err(|e| format!("Failed to add watch address: {}", e))?;

                    tracing::info!("Added watch address to SPV: {}", address);
                    Ok(())
                }
                Err(e) => Err(format!(
                    "Failed to convert address {} to SPV format: {:?}",
                    address, e
                )),
            }
        } else {
            Err("SPV client not started".to_string())
        }
    }

    #[allow(dead_code)]
    pub async fn get_address_balance(&self, address: &dash_spv::Address) -> Result<u64, String> {
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref() {
            let balance = client
                .get_address_balance(address)
                .await
                .map_err(|e| format!("Failed to get address balance: {}", e))?;
            Ok((balance.confirmed + balance.unconfirmed).to_sat())
        } else {
            Err("SPV client not started".to_string())
        }
    }

    #[allow(dead_code)]
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    /// Get a reference to the masternode list engine if available.
    /// Returns None if SPV client is not started or masternode sync is not enabled.
    pub async fn masternode_list_engine(&self) -> Option<Arc<MasternodeListEngine>> {
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref() {
            // We need to clone the engine since we can't return a reference across await boundary
            client
                .masternode_list_engine()
                .map(|engine| Arc::new(engine.clone()))
        } else {
            None
        }
    }

    /// Start the monitor_network loop for SPV sync
    pub async fn start_sync(&self) -> Result<(), String> {
        tracing::info!("Starting SPV sync process...");
        
        // First, check if we have any peer connections
        let client_guard = self.client.read().await;
        if client_guard.is_none() {
            return Err("SPV client not initialized".to_string());
        }
        drop(client_guard);
        
        // Try to establish peer connections first
        tracing::info!("Attempting to establish peer connections...");
        
        // Give the network layer more time to establish connections
        tokio::time::sleep(Duration::from_secs(5)).await;
        
        let spv_manager = self.clone();
        tokio::spawn(async move {
            // Add retry logic
            let mut retry_count = 0;
            let max_retries = 3;
            
            while retry_count < max_retries {
                tracing::info!("Starting monitor_network attempt {} of {}", retry_count + 1, max_retries);
                
                // Get the client once and run monitor_network
                let mut client_guard = spv_manager.client.write().await;
                if let Some(client) = client_guard.as_mut() {
                    tracing::info!("Running monitor_network - this handles all sync operations");
                    match client.monitor_network().await {
                        Ok(_) => {
                            tracing::info!("monitor_network completed successfully");
                            break;
                        }
                        Err(e) => {
                            tracing::error!("monitor_network failed (attempt {}): {}", retry_count + 1, e);
                            retry_count += 1;
                            if retry_count < max_retries {
                                drop(client_guard); // Release the lock before sleeping
                                tracing::info!("Waiting 10 seconds before retry...");
                                tokio::time::sleep(Duration::from_secs(10)).await;
                            }
                        }
                    }
                } else {
                    tracing::error!("No SPV client available for monitor_network");
                    break;
                }
            }
            
            if retry_count >= max_retries {
                tracing::error!("Failed to start SPV sync after {} attempts", max_retries);
            }
        });
        
        Ok(())
    }

    /// Get detailed diagnostic information about the SPV client status
    pub async fn get_diagnostics(&self) -> Result<String, String> {
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref() {
            let mut diagnostics = String::new();

            // Get sync progress
            if let Ok(progress) = client.sync_progress().await {
                diagnostics.push_str(&format!(
                    "Sync Status:\n  Headers: {}\n  Filter Headers: {}\n",
                    progress.header_height, progress.filter_header_height
                ));

                if progress.header_height == 0 {
                    diagnostics.push_str("\nWARNING: No headers synced. Possible issues:\n");
                    diagnostics.push_str("  - No connection to peers\n");
                    diagnostics.push_str("  - Firewall blocking connections\n");
                    diagnostics.push_str("  - All peers offline\n");
                }
            } else {
                diagnostics.push_str("Failed to get sync progress\n");
            }

            // Check masternode list engine
            if let Some(engine) = client.masternode_list_engine() {
                diagnostics.push_str(&format!(
                    "\nMasternode Lists: {}\n",
                    engine.masternode_lists.len()
                ));
                diagnostics.push_str(&format!("Quorum Types: {}\n", engine.quorum_statuses.len()));
            } else {
                diagnostics.push_str("\nMasternode List Engine: Not available\n");
            }

            // Add network info
            diagnostics.push_str(&format!("\nNetwork: {}\n", self.network));
            diagnostics.push_str(&format!("Data Directory: {:?}\n", self.data_dir));

            Ok(diagnostics)
        } else {
            Err("SPV client not started".to_string())
        }
    }
}
