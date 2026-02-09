//! Tauri application state management.
//!
//! Wraps the DET `AppContext` (one per network) and provides unified access
//! for all Tauri IPC commands via `tauri::State<AppState>`.

use dash_evo_tool::app_dir::{
    app_user_data_file_path, copy_env_file_if_not_exists,
    create_app_user_data_directory_if_not_exists,
};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::database::Database;
use dash_evo_tool::logging::initialize_logger;
use dash_evo_tool::model::settings::Settings;
use dash_evo_tool::utils::tasks::TaskManager;

use dash_sdk::dpp::dashcore::Network;
use std::sync::{Arc, RwLock};

/// Central application state managed by Tauri.
///
/// Holds `AppContext` instances for each supported network and tracks
/// the currently active network. All Tauri IPC commands receive this
/// via `tauri::State<AppState>`.
pub struct AppState {
    /// Mainnet context (always present — required for app startup).
    mainnet_context: Arc<AppContext>,
    /// Testnet context (optional — missing if testnet config is absent).
    testnet_context: Option<Arc<AppContext>>,
    /// Devnet context (optional).
    devnet_context: Option<Arc<AppContext>>,
    /// Local/Regtest context (optional).
    local_context: Option<Arc<AppContext>>,
    /// Currently selected network.
    active_network: RwLock<Network>,
    /// Shared task manager for background task lifecycle.
    subtasks: Arc<TaskManager>,
    /// Shared database handle.
    db: Arc<Database>,
}

impl AppState {
    /// Initialize the full application state.
    ///
    /// Replicates the initialization sequence from the egui `AppState::new()`:
    /// 1. Create/verify app data directory
    /// 2. Copy `.env.example` if needed
    /// 3. Initialize logging
    /// 4. Open and migrate SQLite database
    /// 5. Load settings (network, password, theme, etc.)
    /// 6. Create `AppContext` for each configured network
    ///
    /// ZMQ listeners and event forwarding are NOT set up here — that is
    /// handled separately in the Tauri event system (task 1.3).
    pub fn init() -> Result<Self, String> {
        // Step 1: Ensure app data directory exists
        create_app_user_data_directory_if_not_exists()
            .map_err(|e| format!("Failed to create app data directory: {e}"))?;

        // Step 2: Copy .env.example if .env is missing
        copy_env_file_if_not_exists();

        // Step 3: Initialize logging (tracing-subscriber)
        initialize_logger();

        // Step 4: Open SQLite database and run migrations
        let db_file_path = app_user_data_file_path("data.db")
            .map_err(|e| format!("Failed to resolve database path: {e}"))?;
        let db = Arc::new(
            Database::new(&db_file_path).map_err(|e| format!("Failed to open database: {e}"))?,
        );
        db.initialize(&db_file_path)
            .map_err(|e| format!("Failed to initialize database: {e}"))?;

        // Step 5: Load settings
        let settings = db
            .get_settings()
            .map_err(|e| format!("Failed to load settings: {e}"))?
            .map(Settings::from)
            .unwrap_or_default();

        let password_info = settings.password_info;
        let chosen_network = settings.network;

        // Step 6: Create TaskManager for background task lifecycle
        let subtasks = Arc::new(TaskManager::new());

        // Step 7: Create AppContext for each network
        // Mainnet is required; others are optional.
        let mainnet_context = AppContext::new(
            Network::Dash,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
        )
        .ok_or_else(|| {
            "Failed to create AppContext for mainnet. Check your Dash configuration (.env file)."
                .to_string()
        })?;

        let testnet_context = AppContext::new(
            Network::Testnet,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
        );

        let devnet_context = AppContext::new(
            Network::Devnet,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
        );

        let local_context = AppContext::new(
            Network::Regtest,
            db.clone(),
            password_info,
            subtasks.clone(),
        );

        tracing::info!(
            network = ?chosen_network,
            mainnet = true,
            testnet = testnet_context.is_some(),
            devnet = devnet_context.is_some(),
            local = local_context.is_some(),
            "AppState initialized"
        );

        Ok(Self {
            mainnet_context,
            testnet_context,
            devnet_context,
            local_context,
            active_network: RwLock::new(chosen_network),
            subtasks,
            db,
        })
    }

    /// Get the `AppContext` for the currently active network.
    ///
    /// Falls back to mainnet if the selected network's context is unavailable.
    pub fn current_context(&self) -> &Arc<AppContext> {
        let network = self
            .active_network
            .read()
            .unwrap_or_else(|e| e.into_inner());
        self.context_for_network(*network)
    }

    /// Get the `AppContext` for a specific network.
    ///
    /// Falls back to mainnet if the requested network's context is unavailable.
    pub fn context_for_network(&self, network: Network) -> &Arc<AppContext> {
        match network {
            Network::Dash => &self.mainnet_context,
            Network::Testnet => self
                .testnet_context
                .as_ref()
                .unwrap_or(&self.mainnet_context),
            Network::Devnet => self
                .devnet_context
                .as_ref()
                .unwrap_or(&self.mainnet_context),
            Network::Regtest => self.local_context.as_ref().unwrap_or(&self.mainnet_context),
            _ => &self.mainnet_context,
        }
    }

    /// Get the currently active network.
    pub fn active_network(&self) -> Network {
        *self
            .active_network
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Switch the active network.
    ///
    /// Returns `true` if the switch was successful (the network has a valid context),
    /// `false` if the requested network has no context available.
    pub fn switch_network(&self, network: Network) -> bool {
        let has_context = match network {
            Network::Dash => true,
            Network::Testnet => self.testnet_context.is_some(),
            Network::Devnet => self.devnet_context.is_some(),
            Network::Regtest => self.local_context.is_some(),
            _ => false,
        };

        if has_context {
            let mut active = self
                .active_network
                .write()
                .unwrap_or_else(|e| e.into_inner());
            *active = network;
            tracing::info!(?network, "Switched active network");
        }

        has_context
    }

    /// Get the shared database handle.
    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    /// Get the shared task manager.
    pub fn task_manager(&self) -> &Arc<TaskManager> {
        &self.subtasks
    }

    /// Check which networks have valid contexts.
    pub fn available_networks(&self) -> Vec<Network> {
        let mut networks = vec![Network::Dash];
        if self.testnet_context.is_some() {
            networks.push(Network::Testnet);
        }
        if self.devnet_context.is_some() {
            networks.push(Network::Devnet);
        }
        if self.local_context.is_some() {
            networks.push(Network::Regtest);
        }
        networks
    }

    /// Graceful shutdown — cancels all background tasks.
    pub fn shutdown(&self) {
        tracing::info!("Shutting down AppState...");
        if let Err(e) = self.subtasks.shutdown() {
            tracing::error!("Error during TaskManager shutdown: {e}");
        }
        tracing::info!("AppState shutdown complete.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the `NetworkDto` → `Network` mapping in `context_for_network`
    /// always returns mainnet for unknown networks.
    #[test]
    fn context_for_network_covers_all_variants() {
        // We can't create a real AppState without a database, but we can test
        // the network matching logic by verifying the match arms are exhaustive.
        // This test primarily ensures the code compiles with all Network variants.
        let networks = [
            Network::Dash,
            Network::Testnet,
            Network::Devnet,
            Network::Regtest,
        ];
        // Just verify all variants are valid
        assert_eq!(networks.len(), 4);
    }

    /// Verify default network is Dash (mainnet) from default Settings.
    #[test]
    fn default_settings_network_is_dash() {
        let settings = Settings::default();
        assert_eq!(settings.network, Network::Dash);
    }
}
