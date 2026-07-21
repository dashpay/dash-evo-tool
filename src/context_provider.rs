use crate::context::AppContext;
use arc_swap::ArcSwapOption;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::{ContextProvider, DataContract, Identifier};
use rusqlite::Result;
use std::sync::Arc;

/// Number of system contracts cached on [`AppContext`].
/// Update this when adding a new system contract field.
///
/// The typed array size in [`resolve_data_contract`] must match — the compiler
/// will reject a mismatch, catching forgotten additions at build time.
pub(crate) const SYSTEM_CONTRACT_COUNT: usize = 5;

/// Diagnostic carried by the SDK while quorum keys are unavailable at startup.
pub(crate) const MASTERNODE_LIST_NOT_READY_DETAIL: &str =
    "masternode list not yet synced (quorums unavailable)";

/// Resolve a data contract by ID: check cached system contracts first, then DB.
///
/// All system contracts are listed in `cached` — adding a new one is a single
/// array edit. The array size is tied to [`SYSTEM_CONTRACT_COUNT`] so the
/// compiler enforces completeness.
fn resolve_data_contract(
    app_ctx: &AppContext,
    data_contract_id: &Identifier,
) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
    let cached: [&Arc<DataContract>; SYSTEM_CONTRACT_COUNT] = [
        &app_ctx.dpns_contract,
        &app_ctx.dashpay_contract,
        &app_ctx.token_history_contract,
        &app_ctx.withdraws_contract,
        &app_ctx.keyword_search_contract,
    ];

    for contract in &cached {
        if data_contract_id == &contract.id() {
            return Ok(Some(Arc::clone(contract)));
        }
    }

    // K/V fallback for user-added / non-system contracts
    let dc = app_ctx
        .get_contract_by_id(data_contract_id)
        .map_err(|e| ContextProviderError::Generic(e.to_string()))?;

    Ok(dc.map(|qc| Arc::new(qc.contract)))
}

/// Resolve a token configuration from the per-network k/v store.
fn resolve_token_configuration(
    app_ctx: &AppContext,
    token_id: &Identifier,
) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError> {
    app_ctx
        .get_token_config_for_id(token_id)
        .map_err(|e| ContextProviderError::Generic(e.to_string()))
}

/// SPV-based ContextProvider for the Dash SDK.
///
/// - DataContract and TokenConfiguration are served from the local DB.
/// - Quorum public keys are resolved by the upstream `platform-wallet`
///   chain sync via [`WalletBackend`](crate::wallet_backend::WalletBackend).
#[derive(Debug, Clone)]
pub(crate) struct SpvProvider {
    // `Arc` so a cloned provider (the one handed to the SDK) shares the same
    // slot as the original held on `AppContext`, letting a later rebind be seen
    // everywhere. `ArcSwapOption` keeps binding lock-free.
    app_context: Arc<ArcSwapOption<AppContext>>,
    network: Network,
}

impl SpvProvider {
    pub fn new(network: Network) -> Self {
        Self {
            app_context: Arc::new(ArcSwapOption::empty()),
            network,
        }
    }

    /// Attach the `AppContext` and register this provider with the SDK.
    ///
    /// After this call the SDK uses this provider for proof verification and
    /// quorum key resolution.
    ///
    /// # Thread safety
    /// Called during init and mode-switch only — not on hot paths.
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) {
        self.app_context.store(Some(app_context.clone()));
        app_context.sdk.load().set_context_provider(self.clone());
    }

    /// Load the bound `AppContext`, or a `Config` "not ready" error if this
    /// provider has not been bound yet (startup window before binding).
    fn app_context(&self) -> Result<Arc<AppContext>, ContextProviderError> {
        self.app_context
            .load_full()
            .ok_or_else(|| ContextProviderError::Config("no app context".to_string()))
    }
}

impl ContextProvider for SpvProvider {
    fn get_data_contract(
        &self,
        data_contract_id: &Identifier,
        _platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        let app_ctx = self.app_context()?;
        resolve_data_contract(&app_ctx, data_contract_id)
    }

    fn get_token_configuration(
        &self,
        token_id: &Identifier,
    ) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError>
    {
        let app_ctx = self.app_context()?;
        resolve_token_configuration(&app_ctx, token_id)
    }

    fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        // Quorum keys come from upstream `platform-wallet`'s `SpvRuntime`,
        // wrapped by `WalletBackend`. The trait method is sync but the
        // upstream lookup is async; bridge with `block_in_place` (this runs
        // inside SDK proof verification on a tokio worker, never the UI
        // thread).
        let app_ctx = self.app_context()?;

        // Before the SPV masternode list has synced, quorum keys are not yet
        // resolvable. A proof failure here would get the queried DAPI node
        // banned by the SDK (`ban_failed_address: true`), so short-circuit to a
        // `Config` "not ready" diagnostic — the same non-ban-inducing class the
        // pre-unlock guard below uses. Any stray early proof call thus degrades
        // gracefully instead of triggering the self-ban storm.
        if !app_ctx.connection_status().masternodes_ready() {
            // Consumed by `sdk_error_is_masternode_list_not_ready` in `backend_task::error`.
            return Err(ContextProviderError::Config(
                MASTERNODE_LIST_NOT_READY_DETAIL.to_string(),
            ));
        }

        // The wallet-backend gate ("not yet wired") is a startup-window
        // configuration state — `Config`, not `Generic`. Do NOT broadcast
        // the typed error's user-facing Display ("temporarily unavailable")
        // into the SDK retry classifier; emit a non-user-facing diagnostic.
        let backend = app_ctx.wallet_backend().map_err(|_| {
            ContextProviderError::Config("chain backend not initialized (pre-unlock)".to_string())
        })?;
        // `try_current` instead of `current`: the trait method is sync and may
        // be invoked outside a tokio runtime (e.g. a non-async test harness).
        // Return a typed Config error rather than panicking.
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ContextProviderError::Config("no async runtime available for quorum lookup".to_string())
        })?;
        tokio::task::block_in_place(|| {
            handle.block_on(backend.get_quorum_public_key(
                quorum_type,
                quorum_hash,
                core_chain_locked_height,
            ))
        })
        .map_err(|e| ContextProviderError::Generic(e.to_string()))
    }

    fn get_platform_activation_height(
        &self,
    ) -> Result<dash_sdk::dpp::prelude::CoreBlockHeight, ContextProviderError> {
        // Core block height at which Platform activated (the `mn_rr` L1
        // locked height) per network. Mirrors the SDK's own trusted
        // context provider; these are fixed once activation has happened.
        Ok(match self.network {
            Network::Mainnet => 2_132_092,
            Network::Testnet => 1_090_319,
            Network::Devnet | Network::Regtest => 1,
        })
    }
}
