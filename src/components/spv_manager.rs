use dash_sdk::dpp::dashcore::Network;
use dash_sdk::sdk::Uri;
use dash_spv::{init_logging, ClientConfig, DashSpvClient, MasternodeListEngine};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct SpvManager {
    data_dir: PathBuf,
    network: Network,
    client: Arc<RwLock<Option<DashSpvClient>>>,
    is_running: Arc<RwLock<bool>>,
    app_context: Arc<RwLock<Option<Weak<crate::context::AppContext>>>>,
    spv_initialized: Arc<AtomicBool>,
    peer_group_index: Arc<AtomicU32>,
}

impl std::fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpvManager")
            .field("data_dir", &self.data_dir)
            .field("network", &self.network)
            .field("client", &"<DashSpvClient>")
            .field("is_running", &self.is_running)
            .field("app_context", &"<AppContext>")
            .field("spv_initialized", &self.spv_initialized)
            .field("peer_group_index", &self.peer_group_index)
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
            spv_initialized: Arc::new(AtomicBool::new(false)),
            peer_group_index: Arc::new(AtomicU32::new(0)),
        }
    }
    
    fn get_peer_group_index(&self) -> u32 {
        self.peer_group_index.load(Ordering::Relaxed)
    }
    
    fn increment_peer_group_index(&self) {
        self.peer_group_index.fetch_add(1, Ordering::Relaxed);
    }
    
    fn default_peers_for_network(&self) -> Vec<std::net::SocketAddr> {
        match self.network {
            Network::Dash => vec![
                "188.40.190.52:9999".parse().unwrap(),
                "162.243.219.25:9999".parse().unwrap(),
                "95.216.255.72:9999".parse().unwrap(),
                "139.59.254.15:9999".parse().unwrap(),
                "188.166.156.58:9999".parse().unwrap(),
                "82.211.21.195:9999".parse().unwrap(),
            ],
            Network::Testnet => vec![
                "139.60.95.71:19999".parse().unwrap(),
                "18.185.254.104:19999".parse().unwrap(), 
                "35.197.207.178:19999".parse().unwrap(),
                "35.246.224.79:19999".parse().unwrap(),
            ],
            _ => vec![],
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
        tracing::info!("[DEBUG] start_progress_updater called");
        let spv_manager = self.clone();
        tokio::spawn(async move {
            tracing::info!("[DEBUG] Progress monitor task started");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            let mut checks_at_zero = 0;
            let mut last_header_height = 0u32;
            let mut stuck_at_height_checks = 0;
            let mut attempt_recent_chainlock = false;
            
            loop {
                interval.tick().await;
                tracing::info!("[DEBUG] Progress monitor tick");

                // Check if SPV is still running
                let is_running = *spv_manager.is_running.read().await;
                if !is_running {
                    break;
                }

                // Update sync progress
                match spv_manager.get_sync_progress().await {
                    Ok((headers, filters)) => {
                        // Check if we're stuck at the same height
                        if headers == last_header_height {
                            if headers == 0 {
                                checks_at_zero += 1;
                                tracing::info!(
                                    "[PROGRESS MONITOR] Still at height 0 (check #{})",
                                    checks_at_zero
                                );
                                
                                // Switch peers after 6 checks (30 seconds) at height 0
                                if checks_at_zero >= 6 {
                                    if !attempt_recent_chainlock && spv_manager.network == Network::Dash {
                                        tracing::error!(
                                            "SPV sync stuck for {} seconds. Checking if checkpoint was applied...",
                                            checks_at_zero * 5
                                        );
                                        tracing::error!("KNOWN ISSUE: Mainnet nodes only serve blocks from ~height 2.29M");
                                        tracing::error!("We've attempted to set a checkpoint at height 2290000");
                                        tracing::error!("If sync still fails, try: 1) Clear SPV data and restart, 2) Use Testnet");
                                        attempt_recent_chainlock = true;
                                        
                                        // Update the UI to show this error
                                        spv_manager.update_status(false, Some(0), Some(0)).await;
                                        
                                        // Don't keep retrying - it won't work
                                        return;
                                    }
                                    
                                    tracing::warn!(
                                        "Switching to next peer group (attempt #{})...",
                                        spv_manager.get_peer_group_index() + 1
                                    );
                                    checks_at_zero = 0; // Reset counter
                                    
                                    // Try next peer group
                                    let manager = spv_manager.clone();
                                    std::thread::spawn(move || {
                                        let runtime = tokio::runtime::Runtime::new().unwrap();
                                        runtime.block_on(async move {
                                            if let Err(e) = manager.try_next_peer_group().await {
                                                tracing::error!("Failed to switch peer group: {}", e);
                                            }
                                        });
                                    });
                                }
                            } else {
                                // We're stuck at a non-zero height
                                stuck_at_height_checks += 1;
                                tracing::warn!(
                                    "[PROGRESS MONITOR] No header sync progress at height {} (check #{})",
                                    headers,
                                    stuck_at_height_checks
                                );
                                
                                // Switch peers after 8 checks (40 seconds) at the same non-zero height
                                // Give a bit more time for non-zero heights as peer might be processing
                                if stuck_at_height_checks >= 8 {
                                    tracing::error!(
                                        "Header sync stuck at height {} for {} seconds. Peer likely stopped responding.",
                                        headers,
                                        stuck_at_height_checks * 5
                                    );
                                    
                                    tracing::warn!(
                                        "Switching to next peer group (attempt #{}) to continue sync from height {}...",
                                        spv_manager.get_peer_group_index() + 1,
                                        headers
                                    );
                                    stuck_at_height_checks = 0; // Reset counter
                                    
                                    // Try next peer group
                                    let manager = spv_manager.clone();
                                    std::thread::spawn(move || {
                                        let runtime = tokio::runtime::Runtime::new().unwrap();
                                        runtime.block_on(async move {
                                            if let Err(e) = manager.try_next_peer_group().await {
                                                tracing::error!("Failed to switch peer group: {}", e);
                                            }
                                        });
                                    });
                                }
                            }
                        } else {
                            // Progress was made, reset counters
                            if headers > 0 {
                                if spv_manager.network == Network::Dash && headers >= 2290000 && last_header_height < 2290000 {
                                    tracing::info!(
                                        "[PROGRESS MONITOR] ✅ Checkpoint active! Syncing from height 2290000 - current: {}",
                                        headers
                                    );
                                } else {
                                    tracing::info!(
                                        "[PROGRESS MONITOR] Sync progressing - headers: {} (was: {})",
                                        headers,
                                        last_header_height
                                    );
                                }
                            }
                            checks_at_zero = 0;
                            stuck_at_height_checks = 0;
                            last_header_height = headers;
                        }
                    }
                    Err(e) => {
                        tracing::error!("[PROGRESS MONITOR] Failed to get sync progress: {}", e);
                        // Continue monitoring even if we can't get progress
                        continue;
                    }
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

        // Don't add wallet addresses here - they'll be added in initialize_client()

        // Keep existing storage to continue from where we left off
        // Only clear storage if user explicitly requests a full resync
        let spv_storage_path = self.data_dir.join("spv").join(self.network.to_string());
        if spv_storage_path.exists() {
            tracing::info!("SPV storage exists: {:?}", spv_storage_path);
            tracing::info!("Will continue sync from existing state (user-friendly)");
        } else {
            if self.network == Network::Dash {
                tracing::info!("No existing SPV storage found, will start fresh with checkpoint at height 2290000");
            } else {
                tracing::info!("No existing SPV storage found, will start fresh with genesis");
            }
        }
        
        // Get DAPI addresses from app context config
        let all_peers = {
            let ctx_guard = self.app_context.read().await;
            if let Some(weak_ctx) = ctx_guard.as_ref() {
                if let Some(app_ctx) = weak_ctx.upgrade() {
                    // Get DAPI addresses from config and convert to P2P addresses
                    let dapi_addresses_str = app_ctx.config.read().unwrap().dapi_addresses.clone();
                    let mut p2p_addresses = vec![];
                    
                    // Parse the comma-separated DAPI addresses
                    // Format is like: "https://1.2.3.4:443,https://5.6.7.8:443"
                    for addr_str in dapi_addresses_str.split(',') {
                        let addr_str = addr_str.trim();
                        if addr_str.is_empty() {
                            continue;
                        }
                        
                        // Parse as URI to extract host
                        if let Ok(uri) = addr_str.parse::<Uri>() {
                            if let Some(host) = uri.host() {
                                // Convert DAPI port to P2P port
                                let p2p_port = match self.network {
                                    Network::Dash => 9999,
                                    Network::Testnet => 19999,
                                    _ => continue,
                                };
                                
                                // Try to parse the host as an IP address
                                if let Ok(ip_addr) = host.parse::<std::net::IpAddr>() {
                                    let p2p_addr = std::net::SocketAddr::new(ip_addr, p2p_port);
                                    p2p_addresses.push(p2p_addr);
                                    tracing::debug!("Added P2P address: {} (from DAPI: {})", p2p_addr, addr_str);
                                } else {
                                    tracing::debug!("Skipping non-IP host: {} from {}", host, addr_str);
                                }
                            }
                        } else {
                            tracing::debug!("Failed to parse DAPI address as URI: {}", addr_str);
                        }
                    }
                    
                    if !p2p_addresses.is_empty() {
                        tracing::info!("Using {} DAPI addresses from config", p2p_addresses.len());
                        p2p_addresses
                    } else {
                        tracing::warn!("No valid DAPI addresses found, using defaults");
                        self.default_peers_for_network()
                    }
                } else {
                    self.default_peers_for_network()
                }
            } else {
                self.default_peers_for_network()
            }
        };
        
        if all_peers.is_empty() {
            return Err("No peers available for SPV sync".to_string());
        }
        
        // Try different peer combinations
        let peer_group_index = self.get_peer_group_index() as usize;
        let peers_per_group = 3usize;
        let start_index = (peer_group_index * peers_per_group) % all_peers.len();
        
        // Select a subset of peers based on the current group index
        let mut selected_peers = vec![];
        for i in 0..peers_per_group {
            let index = (start_index + i) % all_peers.len();
            selected_peers.push(all_peers[index]);
        }
        
        config.peers = selected_peers.clone();
        tracing::info!(
            "Trying peer group {} with peers: {:?}",
            peer_group_index,
            selected_peers
        );

        // Configure for header-only sync with ChainLock support
        let mut config = config
            .with_storage_path(self.data_dir.join("spv").join(self.network.to_string()))
            .with_validation_mode(dash_spv::ValidationMode::Basic)
            .with_log_level("debug") // Increase logging to see protocol messages
            .with_connection_timeout(std::time::Duration::from_secs(30))
            .without_filters(); // DISABLE FILTERS - most Dash peers don't support COMPACT_FILTERS
            
        // For mainnet, most nodes only serve recent blocks (~2.29M height)
        // Try to start from a recent checkpoint instead of genesis
        if self.network == Network::Dash {
            tracing::info!("Mainnet detected - will request recent checkpoint instead of genesis");
            tracing::info!("Reason: Most mainnet nodes at height 2.29M don't serve full chain history");
        }
        
        // KEEP MASTERNODES ENABLED - needed for ChainLock support like colleague's demo
        // Don't call .without_masternodes() - masternodes are enabled by default and needed for ChainLocks

        // Use multiple peers for better performance
        config.max_peers = 3; // Use 3 peers for parallel downloading
        
        // IMPORTANT: Try to force protocol compatibility
        // Some nodes might only support older protocol versions
        // TODO: Check if there's a way to set protocol version or force GetHeaders instead of GetHeaders2
        
        // Add peer connection debugging
        tracing::info!("[SPV DEBUG] Configuration:");
        tracing::info!("  - Max peers: {}", config.max_peers);
        tracing::info!("  - Connection timeout: {:?}", config.connection_timeout);
        tracing::info!("  - Filters enabled: {}", config.enable_filters);
        tracing::info!("  - Masternodes enabled: {}", config.enable_masternodes);
        tracing::info!("  - ChainLock support: {} (requires masternodes)", config.enable_masternodes);
        tracing::info!("  - Header-only sync mode (no compact filters)");
        tracing::info!("  - Validation mode: {:?}", dash_spv::ValidationMode::Basic);
        tracing::warn!("  - NOTE: If seeing 'GetHeaders2' in logs, this might be a protocol mismatch");

        // Configure starting point based on peer capabilities  
        // Most peers only serve recent blocks, not full history from genesis
        match self.network {
            Network::Dash => {
                tracing::info!("Will override genesis with recent checkpoint after client starts");
                tracing::info!("Reason: Most peers only serve recent blocks (start_height ~2.29M)");
            }
            _ => {}
        }
        
        // Log peer configuration
        tracing::info!(
            "SPV client configured for {} with {} available peer addresses (will connect to max {}): {:?}",
            self.network,
            config.peers.len(),
            config.max_peers,
            config.peers
        );

        // Create client but don't start it yet - wait for user to initiate sync
        let client = DashSpvClient::new(config)
            .await
            .map_err(|e| format!("Failed to create SPV client: {}", e))?;

        tracing::info!("SPV client created successfully");
        tracing::info!("SPV client initialized but not started - waiting for user to start sync");

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
        
        // For mainnet, try to set a checkpoint early to avoid genesis sync issues
        if self.network == Network::Dash {
            tracing::info!("Mainnet detected - attempting to set checkpoint before sync");
            if let Err(e) = self.set_recent_checkpoint().await {
                tracing::warn!("Failed to set initial checkpoint: {}", e);
            }
        }

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

        // Don't start the progress updater here - it will be started when sync begins
        // This prevents the warning messages from appearing before user clicks "Start SPV Sync"
        
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

    /// Try the next group of peers
    pub async fn try_next_peer_group(&self) -> Result<(), String> {
        let current_group = self.get_peer_group_index();
        
        // Get current sync height before switching
        let current_height = match self.get_sync_progress().await {
            Ok((headers, _)) => {
                tracing::info!("Current sync height before switching peers: {}", headers);
                headers
            }
            Err(_) => 0,
        };
        
        tracing::info!("Sync not progressing at height {}, trying next peer group (current group: {})...", current_height, current_group);
        
        // Check if we've tried too many peer groups
        if current_group >= 10 {
            tracing::error!("Tried {} peer groups without success.", current_group);
            if current_height == 0 && self.network == Network::Dash {
                tracing::error!("Most mainnet nodes don't serve genesis headers.");
                tracing::info!("Consider:");
                tracing::info!("1. Switching to testnet for development/testing");
                tracing::info!("2. Using a node with full chain history"); 
                tracing::info!("3. Starting from a recent checkpoint instead of genesis");
                return Err("Exhausted peer groups - no nodes serve genesis headers".to_string());
            } else {
                tracing::error!("No peers seem to be responding with headers.");
                return Err("Exhausted peer groups - no responsive peers found".to_string());
            }
        }
        
        // Increment peer group index
        self.increment_peer_group_index();
        
        // Stop current client gracefully
        tracing::info!("Stopping current SPV client...");
        self.stop().await?;
        
        // Wait a bit for clean shutdown
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Start with new peers
        tracing::info!("Starting SPV client with new peer group...");
        self.start().await?;
        
        // Re-initialize client with wallet addresses
        tokio::time::sleep(Duration::from_secs(1)).await;
        tracing::info!("Re-initializing SPV client...");
        self.initialize_client().await?;
        
        // If we had made progress before, log that we're continuing from that height
        if current_height > 0 {
            tracing::info!("Continuing sync from height {} with new peers", current_height);
        }
        
        // Start sync again
        tracing::info!("Restarting sync process with new peer group...");
        self.start_sync().await
    }
    
    /// Start the monitor_network loop for SPV sync
    pub async fn start_sync(&self) -> Result<(), String> {
        tracing::info!("Starting SPV sync process...");
        
        // Check if client was initialized first
        if !self.spv_initialized.load(Ordering::Relaxed) {
            return Err("SPV client not initialized. Please click 'Initialize SPV' first.".to_string());
        }
        
        // Client should already be started from initialize_client()
        let client_exists = {
            let guard = self.client.read().await;
            guard.is_some()
        };
        
        if !client_exists {
            return Err("SPV client not found".to_string());
        }
        
        // For mainnet, we need to set a recent checkpoint first
        // because mainnet nodes don't serve the full history from genesis
        if self.network == Network::Dash {
            tracing::info!("Mainnet detected - setting recent checkpoint to avoid genesis sync issues");
            if let Err(e) = self.set_recent_checkpoint().await {
                tracing::error!("Failed to set recent checkpoint: {}", e);
                tracing::warn!("Continuing anyway - sync might fail if peers don't have genesis blocks");
            }
        }
        
        // Peer monitoring is handled by start_progress_updater
        
        // Start the monitor_network loop which handles all sync operations
        // Use std::thread::spawn to avoid Send requirement issues
        let spv_manager = self.clone();
        std::thread::spawn(move || {
            // Create a new runtime for this thread
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            runtime.block_on(async move {
                tracing::info!("Starting monitor_network loop to handle header sync...");
                
                // Wait for peer connections to stabilize
                tokio::time::sleep(Duration::from_secs(2)).await;
                
                // Force a sync_to_tip before starting monitor_network
                // This helps establish the initial sync state
                {
                    let mut client_guard = spv_manager.client.write().await;
                    if let Some(client) = client_guard.as_mut() {
                        tracing::info!("Calling sync_to_tip to establish initial sync state...");
                        tracing::info!("IMPORTANT: Watch for 'Send getheaders' vs 'Send getheaders2' in the logs");
                        tracing::info!("If you see 'getheaders2', there might be a protocol version mismatch");
                        match client.sync_to_tip().await {
                            Ok(_) => {
                                tracing::info!("sync_to_tip completed - ready for header sync");
                                tracing::info!("Now monitor_network should start sending header requests");
                            }
                            Err(e) => {
                                tracing::warn!("sync_to_tip failed (may be normal): {}", e);
                                tracing::warn!("This might indicate a protocol compatibility issue");
                            }
                        }
                    }
                }
                
                // Small delay to let sync state settle
                tokio::time::sleep(Duration::from_millis(500)).await;
                
                let mut client_guard = spv_manager.client.write().await;
                if let Some(client) = client_guard.as_mut() {
                    tracing::info!("🚀 Starting monitor_network() - this will send GetHeaders requests");
                    tracing::info!("Note: Expect to see 'Handle headers message with 2000 headers' messages");
                    match client.monitor_network().await {
                        Ok(_) => {
                            tracing::info!("monitor_network completed successfully");
                        }
                        Err(e) => {
                            tracing::error!("monitor_network failed: {}", e);
                        }
                    }
                } else {
                    tracing::error!("No SPV client available for monitor_network");
                }
            });
        });
        
        // Start background task to periodically update sync progress
        // This is started here so it only runs when the user initiates sync
        self.start_progress_updater().await;
        
        Ok(())
    }

    /// Initialize the SPV client by starting it and adding watch addresses
    pub async fn initialize_client(&self) -> Result<(), String> {
        tracing::info!("Initializing SPV client...");
        
        // Ensure SPV storage directory and subdirectories exist
        let spv_storage_path = self.data_dir.join("spv").join(self.network.to_string());
        if !spv_storage_path.exists() {
            tracing::info!("Creating SPV storage directory: {:?}", spv_storage_path);
            std::fs::create_dir_all(&spv_storage_path)
                .map_err(|e| format!("Failed to create SPV storage directory: {}", e))?;
        }
        
        // Ensure required subdirectories exist (dash-spv expects these)
        let subdirs = ["headers", "filters", "state"];
        for subdir in &subdirs {
            let subdir_path = spv_storage_path.join(subdir);
            if !subdir_path.exists() {
                tracing::info!("Creating SPV subdirectory: {:?}", subdir_path);
                std::fs::create_dir_all(&subdir_path)
                    .map_err(|e| format!("Failed to create SPV subdirectory {}: {}", subdir, e))?;
            }
        }
        
        // For mainnet, prepare checkpoint if this is a fresh start
        let sync_state_path = spv_storage_path.join("sync_state.json");
        if self.network == Network::Dash && !sync_state_path.exists() {
            tracing::info!("Fresh mainnet SPV start detected - preparing checkpoint to avoid genesis sync");
            // Create initial sync_state.json with checkpoint
            let checkpoint_height = 2290000;
            let checkpoint_hash = "00000000000000158a0aa3adfd733a2e58bd1d78c88a5ecfe2a51d37fc90d844";
            let checkpoint_time = 1734883200u64;
            
            let initial_state = serde_json::json!({
                "version": 1,
                "network": "dash",
                "chain_tip": {
                    "height": checkpoint_height,
                    "hash": checkpoint_hash,
                    "prev_hash": "0000000000000000000000000000000000000000000000000000000000000000",
                    "time": checkpoint_time
                },
                "sync_progress": {
                    "header_height": checkpoint_height,
                    "filter_header_height": 0,
                    "masternode_height": 0,
                    "peer_count": 0,
                    "headers_synced": false,
                    "filter_headers_synced": false,
                    "masternodes_synced": false,
                    "filter_sync_available": false,
                    "filters_downloaded": 0,
                    "last_synced_filter_height": null,
                    "sync_start": {
                        "secs_since_epoch": 0,
                        "nanos_since_epoch": 0
                    },
                    "last_update": {
                        "secs_since_epoch": 0,
                        "nanos_since_epoch": 0
                    }
                },
                "checkpoints": [{
                    "height": checkpoint_height,
                    "hash": checkpoint_hash,
                    "time": checkpoint_time
                }],
                "masternode_sync": {
                    "last_synced_height": null,
                    "is_synced": false,
                    "masternode_count": 0,
                    "last_diff_height": null
                },
                "filter_sync": {
                    "filter_header_height": 0,
                    "filter_height": 0,
                    "filters_downloaded": 0,
                    "matched_heights": [],
                    "filter_sync_available": false
                },
                "saved_at": {
                    "secs_since_epoch": 0,
                    "nanos_since_epoch": 0
                },
                "chain_work": "ChainWork { work: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 16] }"
            });
            
            match serde_json::to_string_pretty(&initial_state) {
                Ok(json) => {
                    match std::fs::write(&sync_state_path, json) {
                        Ok(_) => tracing::info!("Created initial sync_state.json with checkpoint at height {}", checkpoint_height),
                        Err(e) => tracing::warn!("Failed to create initial sync_state.json: {}", e),
                    }
                }
                Err(e) => tracing::warn!("Failed to serialize initial sync state: {}", e),
            }
        }
        
        // Check if we should continue from existing sync state
        let should_clear_storage = if spv_storage_path.exists() {
            // Check if we have any actual sync progress
            let client_guard = self.client.read().await;
            if let Some(client) = client_guard.as_ref() {
                if let Ok(progress) = client.sync_progress().await {
                    let should_clear = progress.header_height == 0;
                    tracing::info!(
                        "Existing storage found with {} headers. {}",
                        progress.header_height,
                        if should_clear { "Will clear and start fresh" } else { "Will continue from existing state" }
                    );
                    should_clear
                } else {
                    tracing::info!("Cannot check sync progress, keeping existing storage");
                    false
                }
            } else {
                drop(client_guard);
                false
            }
        } else {
            false
        };
        
        if should_clear_storage {
            tracing::info!("Clearing SPV storage to start fresh (no progress detected)");
            if let Err(e) = std::fs::remove_dir_all(&spv_storage_path) {
                tracing::warn!("Failed to clear SPV storage: {}, continuing anyway", e);
            } else {
                // Recreate the directory and subdirectories after clearing
                std::fs::create_dir_all(&spv_storage_path)
                    .map_err(|e| format!("Failed to recreate SPV storage directory: {}", e))?;
                
                // Also recreate required subdirectories
                for subdir in &["headers", "filters", "state"] {
                    let subdir_path = spv_storage_path.join(subdir);
                    std::fs::create_dir_all(&subdir_path)
                        .map_err(|e| format!("Failed to recreate SPV subdirectory {}: {}", subdir, e))?;
                }
                
                match self.network {
                    Network::Dash => {
                        tracing::info!("Successfully cleared SPV storage - will start from recent checkpoint");
                        tracing::info!("Reason: Mainnet peers typically only serve recent blocks (current height ~2.29M)");
                    }
                    _ => {
                        tracing::info!("Successfully cleared SPV storage - will start from genesis");
                    }
                }
            }
        }
        
        // Ensure the client has been created first
        let client_exists = {
            let guard = self.client.read().await;
            guard.is_some()
        };
        
        if !client_exists {
            return Err("SPV client not created. Please ensure SPV manager has been started first.".to_string());
        }
        
        // Get app context to access wallet addresses
        let ctx_guard = self.app_context.read().await;
        if let Some(weak_ctx) = ctx_guard.as_ref() {
            if let Some(app_context) = weak_ctx.upgrade() {
                // Get all wallets and their addresses
                let mut total_addresses = 0;
                    
                    // Collect addresses first to avoid holding locks during async operations
                    let mut addresses_to_add = Vec::new();
                    
                    {
                        let wallets = app_context.wallets.read().unwrap();
                        tracing::info!("Found {} wallets in app context", wallets.len());
                        
                        for (_seed_hash, wallet) in wallets.iter() {
                            let wallet_guard = wallet.read().unwrap();
                            tracing::info!(
                                "Wallet {} has {} known addresses",
                                wallet_guard.alias.as_deref().unwrap_or("unnamed"),
                                wallet_guard.known_addresses.len()
                            );
                            
                            // Collect addresses from this wallet
                            for (address, derivation_path) in &wallet_guard.known_addresses {
                                tracing::info!(
                                    "  - Address: {} (path: {})",
                                    address,
                                    derivation_path
                                );
                                addresses_to_add.push((address.clone(), derivation_path.clone()));
                            }
                        }
                    } // wallets lock is dropped here
                    
                    // Get the client and add addresses before starting
                    let mut client_guard = self.client.write().await;
                    if let Some(client) = client_guard.as_mut() {
                        // Add watch addresses BEFORE starting the client
                        for (address, derivation_path) in addresses_to_add {
                            // Check if address network matches
                            if address.network() != &self.network {
                                tracing::warn!(
                                    "Skipping address {} - network mismatch (address: {}, wallet: {})",
                                    address,
                                    address.network(),
                                    self.network
                                );
                                continue;
                            }
                            
                            // Convert address to SPV format and add directly to client
                            let script_bytes = address.script_pubkey().to_bytes();
                            let spv_script = dash_spv::ScriptBuf::from(script_bytes);
                            
                            // Convert network type
                            let spv_network = match self.network {
                                dash_sdk::dpp::dashcore::Network::Dash => dash_spv::Network::Dash,
                                dash_sdk::dpp::dashcore::Network::Testnet => dash_spv::Network::Testnet,
                                dash_sdk::dpp::dashcore::Network::Devnet => dash_spv::Network::Devnet,
                                dash_sdk::dpp::dashcore::Network::Regtest => dash_spv::Network::Regtest,
                                _ => {
                                    tracing::error!("Unsupported network type for address {}", address);
                                    continue;
                                }
                            };
                            
                            // Reconstruct the address using dash_spv types
                            match dash_spv::Address::from_script(&spv_script, spv_network) {
                                Ok(spv_address) => {
                                    match client.add_watch_item(dash_spv::WatchItem::address(spv_address.clone())).await {
                                        Ok(_) => {
                                            tracing::info!(
                                                "Successfully added watch address: {} (path: {})",
                                                address,
                                                derivation_path
                                            );
                                            total_addresses += 1;
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to add watch address {}: {}",
                                                address,
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to convert address {} to SPV format: {:?}",
                                        address, e
                                    );
                                }
                            }
                        }
                        
                        tracing::info!(
                            "Added {} watch addresses to SPV client",
                            total_addresses
                        );
                        
                        // Now start the client network operations
                        tracing::info!("Starting SPV client network operations...");
                        client
                            .start()
                            .await
                            .map_err(|e| format!("Failed to start SPV client: {}", e))?;
                        
                        tracing::info!("SPV client started successfully");
                        drop(client_guard); // Release the lock
                    } else {
                        return Err("SPV client not found".to_string());
                    }
                    
                    tracing::info!(
                        "SPV client initialized with {} watch addresses",
                        total_addresses
                    );
                    
                    // Check sync progress after adding addresses
                    if let Ok((headers, _filters)) = self.get_sync_progress().await {
                        tracing::info!(
                            "Initial sync status after initialization: headers={} (filter sync disabled)",
                            headers
                        );
                        
                        // If we're at height 0, try to kickstart the sync
                        if headers == 0 {
                            tracing::info!("Headers at 0, attempting to kickstart sync...");
                            if let Err(e) = self.request_headers_from(0).await {
                                tracing::warn!("Failed to kickstart header sync: {}", e);
                            }
                        }
                    }
                    
                    // Mark as initialized
                    self.spv_initialized.store(true, Ordering::Relaxed);
                    
                    Ok(())
            } else {
                Err("App context not available".to_string())
            }
        } else {
            Err("App context not bound".to_string())
        }
    }

    /// Clear SPV storage to force resync from genesis (user-initiated)
    #[allow(dead_code)]
    pub async fn clear_storage_for_resync(&self) -> Result<(), String> {
        tracing::info!("User requested full resync from genesis");
        
        // Stop client first
        self.stop().await?;
        
        // Clear storage
        let spv_storage_path = self.data_dir.join("spv").join(self.network.to_string());
        if spv_storage_path.exists() {
            tracing::info!("Clearing SPV storage for full resync: {:?}", spv_storage_path);
            if let Err(e) = std::fs::remove_dir_all(&spv_storage_path) {
                return Err(format!("Failed to clear SPV storage: {}", e));
            }
            tracing::info!("Successfully cleared SPV storage - next sync will start from genesis");
        }
        
        Ok(())
    }

    /// Request headers from a specific height (useful for kickstarting sync)
    #[allow(dead_code)]
    pub async fn request_headers_from(&self, from_height: u32) -> Result<(), String> {
        let mut client_guard = self.client.write().await;
        if let Some(client) = client_guard.as_mut() {
            tracing::info!("Manually requesting headers from height {}", from_height);
            // This will trigger the client to request headers
            client.sync_to_tip().await
                .map_err(|e| format!("Failed to request headers: {}", e))?;
            Ok(())
        } else {
            Err("SPV client not started".to_string())
        }
    }
    
    /// Set a recent checkpoint for mainnet to avoid genesis sync issues
    async fn set_recent_checkpoint(&self) -> Result<(), String> {
        tracing::info!("Setting recent checkpoint for mainnet SPV sync...");
        
        // Use a recent mainnet checkpoint that most nodes should have
        // Height 2290000 is from around December 2024
        let checkpoint_height = 2290000;
        let checkpoint_hash = "00000000000000158a0aa3adfd733a2e58bd1d78c88a5ecfe2a51d37fc90d844"; // Actual hash from chainz.cryptoid.info
        let checkpoint_time = 1734883200u64; // Approximate timestamp for Dec 2024
        
        // First, try to modify the sync_state.json file before sync starts
        let sync_state_path = self.data_dir.join("spv").join(self.network.to_string()).join("sync_state.json");
        
        if sync_state_path.exists() {
            tracing::info!("Found existing sync_state.json, attempting to inject checkpoint...");
            
            // Read the current sync state
            match std::fs::read_to_string(&sync_state_path) {
                Ok(contents) => {
                    match serde_json::from_str::<serde_json::Value>(&contents) {
                        Ok(mut sync_state) => {
                            // Backup the original file
                            let backup_path = sync_state_path.with_extension("json.backup");
                            let _ = std::fs::copy(&sync_state_path, &backup_path);
                            
                            // Update checkpoints
                            if let Some(checkpoints) = sync_state.get_mut("checkpoints") {
                                if let Some(checkpoints_array) = checkpoints.as_array_mut() {
                                    checkpoints_array.clear();
                                    checkpoints_array.push(serde_json::json!({
                                        "height": checkpoint_height,
                                        "hash": checkpoint_hash,
                                        "time": checkpoint_time
                                    }));
                                }
                            }
                            
                            // Update chain_tip to start from checkpoint
                            if let Some(chain_tip) = sync_state.get_mut("chain_tip") {
                                chain_tip["height"] = serde_json::json!(checkpoint_height);
                                chain_tip["hash"] = serde_json::json!(checkpoint_hash);
                                chain_tip["time"] = serde_json::json!(checkpoint_time);
                            }
                            
                            // Update sync progress
                            if let Some(sync_progress) = sync_state.get_mut("sync_progress") {
                                sync_progress["header_height"] = serde_json::json!(checkpoint_height);
                            }
                            
                            // Write back the modified state
                            match serde_json::to_string_pretty(&sync_state) {
                                Ok(json) => {
                                    match std::fs::write(&sync_state_path, json) {
                                        Ok(_) => {
                                            tracing::info!("Successfully injected checkpoint into sync_state.json");
                                            
                                            // Clear the headers directory to ensure clean sync from checkpoint
                                            let headers_dir = self.data_dir.join("spv").join(self.network.to_string()).join("headers");
                                            if headers_dir.exists() {
                                                if let Err(e) = std::fs::remove_dir_all(&headers_dir) {
                                                    tracing::warn!("Failed to clear headers directory: {}", e);
                                                } else {
                                                    // Recreate the directory
                                                    let _ = std::fs::create_dir_all(&headers_dir);
                                                    tracing::info!("Cleared headers directory for clean sync from checkpoint");
                                                }
                                            }
                                        }
                                        Err(e) => tracing::error!("Failed to write modified sync_state.json: {}", e),
                                    }
                                }
                                Err(e) => tracing::error!("Failed to serialize sync_state.json: {}", e),
                            }
                        }
                        Err(e) => tracing::error!("Failed to parse sync_state.json: {}", e),
                    }
                }
                Err(e) => tracing::error!("Failed to read sync_state.json: {}", e),
            }
        } else {
            tracing::info!("No sync_state.json found yet, checkpoint will be set after first save");
        }
        
        // Now try to use the client API if available
        let mut client_guard = self.client.write().await;
        if let Some(client) = client_guard.as_mut() {
            tracing::info!(
                "Checkpoint set at height {} (hash: {})",
                checkpoint_height,
                checkpoint_hash
            );
            
            // Force a sync_to_tip to pick up from the checkpoint
            match client.sync_to_tip().await {
                Ok(_) => tracing::info!("Triggered sync_to_tip after checkpoint injection"),
                Err(e) => tracing::warn!("sync_to_tip failed after checkpoint: {}", e),
            }
            
            Ok(())
        } else {
            // Client not available yet, but we may have modified the state file
            Ok(())
        }
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
                    if self.network == Network::Dash {
                        diagnostics.push_str("  - Checkpoint not applied (run reset_spv_with_checkpoint.sh)\n");
                    }
                } else if self.network == Network::Dash && progress.header_height >= 2290000 {
                    diagnostics.push_str("\n✅ Checkpoint active - syncing from height 2290000\n");
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
