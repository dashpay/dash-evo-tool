use dash_sdk::dpp::dashcore::Network;
use dash_spv::{init_logging, Address, ClientConfig, DashSpvClient, WatchItem};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
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

    async fn update_status(&self, is_running: bool, header_height: Option<u32>, filter_height: Option<u32>) {
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
                if let Ok(_) = spv_manager.get_sync_progress().await {
                    // Progress is automatically updated in get_sync_progress
                }
            }
        });
    }

    pub async fn start(&self) -> Result<(), String> {
        // Initialize logging for SPV
        let _ = init_logging("info");

        // Stop any existing client
        self.stop().await?;

        // Configure SPV client based on network
        let config = match self.network {
            Network::Dash => ClientConfig::mainnet(),
            Network::Testnet => ClientConfig::testnet(),
            Network::Devnet => return Err("SPV client does not support devnet".to_string()),
            Network::Regtest => return Err("SPV client does not support regtest".to_string()),
            _ => return Err("Unsupported network".to_string()),
        }
        .with_storage_path(self.data_dir.join("spv").join(self.network.to_string()))
        .with_validation_mode(dash_spv::ValidationMode::Basic)
        .with_log_level("info");

        // Create and start client
        let mut client = DashSpvClient::new(config)
            .await
            .map_err(|e| format!("Failed to create SPV client: {}", e))?;

        client
            .start()
            .await
            .map_err(|e| format!("Failed to start SPV client: {}", e))?;

        *self.client.write().await = Some(client);
        *self.is_running.write().await = true;

        // Update status
        self.update_status(true, None, None).await;

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

    pub async fn sync_to_tip(&self) -> Result<(), String> {
        let mut client_guard = self.client.write().await;
        if let Some(client) = client_guard.as_mut() {
            client
                .sync_to_tip()
                .await
                .map_err(|e| format!("Failed to sync to tip: {}", e))?;
            Ok(())
        } else {
            Err("SPV client not started".to_string())
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
            self.update_status(true, Some(progress.header_height), Some(progress.filter_header_height)).await;
            
            Ok((progress.header_height, progress.filter_header_height))
        } else {
            Err("SPV client not started".to_string())
        }
    }

    pub async fn add_watch_address(&self, address: Address) -> Result<(), String> {
        let mut client_guard = self.client.write().await;
        if let Some(client) = client_guard.as_mut() {
            client
                .add_watch_item(WatchItem::Address {
                    address: address.clone(),
                    earliest_height: None,
                })
                .await
                .map_err(|e| format!("Failed to add watch address: {}", e))?;
            Ok(())
        } else {
            Err("SPV client not started".to_string())
        }
    }

    pub async fn get_address_balance(&self, address: &Address) -> Result<u64, String> {
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

    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
}
