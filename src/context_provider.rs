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
/// System contracts are served as `platform_version` defines them: the proof
/// verifier passes the version it verifies with, which during an upgrade can
/// be newer than the SDK's stored version.
///
/// All system contracts are listed in `cached` — adding a new one is a single
/// array edit. The array size is tied to [`SYSTEM_CONTRACT_COUNT`] so the
/// compiler enforces completeness.
pub(crate) fn resolve_data_contract(
    app_ctx: &AppContext,
    data_contract_id: &Identifier,
    platform_version: &PlatformVersion,
) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
    // Never another version's contracts: the verifier would check the proof
    // against the wrong schema.
    let system_contracts = app_ctx
        .system_contracts_at(platform_version)
        .map_err(|error| {
            ContextProviderError::DataContractFailure(format!(
                "system contracts for protocol version {} could not be loaded: {error}",
                platform_version.protocol_version
            ))
        })?;
    let cached: [&Arc<DataContract>; SYSTEM_CONTRACT_COUNT] = [
        &system_contracts.dpns,
        &system_contracts.dashpay,
        &system_contracts.token_history,
        &system_contracts.withdrawals,
        &system_contracts.keyword_search,
    ];

    if let Some(contract) = cached
        .into_iter()
        .find(|contract| data_contract_id == &contract.id())
    {
        return Ok(Some(Arc::clone(contract)));
    }

    // K/V fallback for user-added / non-system contracts, decoded at the
    // verifier's version too.
    let dc = app_ctx
        .get_contract_by_id_at(data_contract_id, platform_version)
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
    fee_claim_contract: Option<Arc<FeeClaimContract>>,
}

#[derive(Debug)]
struct FeeClaimContract {
    id: Identifier,
    registered: ArcSwapOption<DataContract>,
}

impl SpvProvider {
    pub fn new(network: Network) -> Self {
        Self {
            app_context: Arc::new(ArcSwapOption::empty()),
            network,
            fee_claim_contract: None,
        }
    }

    /// Isolate the contract refreshed for one payout from all other SDK callers.
    pub(crate) fn for_fee_claim(&self, contract_id: Identifier) -> Self {
        Self {
            fee_claim_contract: Some(Arc::new(FeeClaimContract {
                id: contract_id,
                registered: ArcSwapOption::empty(),
            })),
            ..self.clone()
        }
    }

    /// Attach the `AppContext`.
    ///
    /// Every SDK the context runs is built with a clone of this provider
    /// ([`initialize_sdk`](crate::sdk_wrapper::initialize_sdk)), and clones
    /// share the attached context, so the SDK sees it without registering the
    /// provider again.
    ///
    /// # Thread safety
    /// Called during init and mode-switch only — not on hot paths.
    pub fn bind_app_context(&self, app_context: Arc<AppContext>) {
        self.app_context.store(Some(app_context));
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
        platform_version: &PlatformVersion,
    ) -> Result<Option<Arc<DataContract>>, ContextProviderError> {
        if let Some(scope) = &self.fee_claim_contract
            && scope.id == *data_contract_id
            && let Some(contract) = scope.registered.load_full()
        {
            return Ok(Some(contract));
        }
        let app_ctx = self.app_context()?;
        resolve_data_contract(&app_ctx, data_contract_id, platform_version)
    }

    fn register_data_contract(&self, contract: Arc<DataContract>) {
        if let Some(scope) = &self.fee_claim_contract
            && scope.id == contract.id()
        {
            scope.registered.rcu(|saved| match saved {
                Some(saved) if saved.version() > contract.version() => Some(saved.clone()),
                _ => Some(contract.clone()),
            });
        }
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

        // The wallet-backend gate ("not yet wired") is a startup-window
        // configuration state — `Config`, not `Generic`. Do NOT broadcast
        // the typed error's user-facing Display ("temporarily unavailable")
        // into the SDK retry classifier; emit a non-user-facing diagnostic.
        let backend = app_ctx.wallet_backend().map_err(|_| {
            ContextProviderError::Config("chain backend not initialized (pre-unlock)".to_string())
        })?;

        // Before the SPV masternode list has synced, quorum keys are not yet
        // resolvable, so answer with a typed "not ready" diagnostic that DET
        // maps to `TaskError::MasternodeListNotReady`. The SDK returns a
        // provider `Config` failure as a configuration error of its own: it
        // does not retry the request and bans no node for it (pinned by
        // `sdk_wrapper::tests::a_proof_read_before_masternodes_are_ready_is_a_readiness_wait_and_bans_no_node`).
        // The read still fails, so callers hold proof-verifying reads until
        // this backend's `quorum_keys_ready()`; this branch names the failure
        // for one that gets through.
        if !backend.quorum_keys_ready() {
            // Consumed by `sdk_error_is_masternode_list_not_ready` in `backend_task::error`.
            return Err(ContextProviderError::Config(
                MASTERNODE_LIST_NOT_READY_DETAIL.to_string(),
            ));
        }

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
        .map_err(quorum_lookup_error)
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

fn quorum_lookup_error(error: crate::backend_task::error::TaskError) -> ContextProviderError {
    match error {
        crate::backend_task::error::TaskError::WalletBackend { source } => {
            source.quorum_lookup_context_error()
        }
        other => ContextProviderError::Generic(other.to_string()),
    }
}

#[cfg(test)]
mod quorum_error_tests {
    use super::*;
    use crate::backend_task::error::TaskError;
    use dash_sdk::dpp::dashcore::sml::llmq_entry_verification::{
        LLMQEntryVerificationSkipStatus, LLMQEntryVerificationStatus,
    };

    #[test]
    fn unverified_local_quorum_does_not_blame_the_dapi_endpoint() {
        let error = TaskError::WalletBackend {
            source: Arc::new(
                platform_wallet::error::PlatformWalletError::QuorumNotVerified {
                    quorum_type: 6,
                    quorum_hash: [0xab; 32],
                    status: LLMQEntryVerificationStatus::Skipped(
                        LLMQEntryVerificationSkipStatus::NotMarkedForVerification,
                    ),
                },
            ),
        };
        assert!(matches!(
            quorum_lookup_error(error),
            ContextProviderError::QuorumSourceUnavailable(_)
        ));
    }

    #[test]
    fn unrelated_quorum_lookup_errors_are_not_reclassified_as_readiness() {
        assert!(matches!(
            quorum_lookup_error(TaskError::NoIdentitiesFound),
            ContextProviderError::Generic(_)
        ));
    }
}

#[cfg(test)]
mod data_contract_tests {
    use super::*;
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};

    /// The proof verifier asks for a contract at the version it verifies
    /// with, which during an upgrade can be newer than the SDK's stored
    /// version. A system contract is served as that version defines it.
    #[test]
    fn system_contracts_resolve_at_the_requested_platform_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let v14 = PlatformVersion::get(14).expect("PV14");
        let v15 = PlatformVersion::get(15).expect("PV15");
        assert_ne!(
            load_system_data_contract(SystemDataContract::DPNS, v14).unwrap(),
            load_system_data_contract(SystemDataContract::DPNS, v15).unwrap(),
            "the lookup fixtures must distinguish the proof versions",
        );
        for protocol_version in [14, 15] {
            let platform_version = PlatformVersion::get(protocol_version).expect("known version");
            let expected =
                load_system_data_contract(SystemDataContract::DPNS, platform_version).unwrap();
            let resolved = resolve_data_contract(&ctx, &expected.id(), platform_version)
                .expect("lookup")
                .expect("a cached system contract");
            assert_eq!(*resolved, expected, "protocol {protocol_version}");
        }
    }

    /// A version whose system contracts cannot be loaded is refused, not
    /// answered with the set loaded for another version.
    #[test]
    fn system_contracts_that_cannot_load_at_the_requested_version_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let dpns_id = SystemDataContract::DPNS.id();
        // Load a set first, so a fallback would have something to serve.
        let latest = PlatformVersion::latest();
        assert!(matches!(
            resolve_data_contract(&ctx, &dpns_id, latest),
            Ok(Some(_))
        ));

        let mut unloadable = latest.clone();
        unloadable.protocol_version = u32::MAX;
        let resolved = resolve_data_contract(&ctx, &dpns_id, &unloadable);

        assert!(
            matches!(resolved, Err(ContextProviderError::DataContractFailure(_))),
            "got {resolved:?}"
        );
    }
}

#[cfg(test)]
mod fee_claim_tests {
    use super::*;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Setters;
    use dash_sdk::dpp::data_contract::config::DataContractConfig;
    use dash_sdk::dpp::data_contract::config::moderation::ContractModerationConfig;
    use dash_sdk::dpp::data_contract::config::moderation::ContractModerators;
    use dash_sdk::dpp::data_contract::document_type::action_fees::ContractFeePot;
    use std::collections::BTreeSet;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fee_claim_refresh_resolves_new_team_without_changing_other_callers() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let version = PlatformVersion::latest();
        let mut saved = dash_sdk::dpp::system_data_contracts::load_system_data_contract(
            dash_sdk::dpp::system_data_contracts::SystemDataContract::DPNS,
            version,
        )
        .unwrap();
        saved.set_id(Identifier::random());
        let [a, b, c] = std::array::from_fn(|_| Identifier::random());
        let team = |ids| ContractModerationConfig {
            banlist: true,
            suspensions: false,
            warnings: false,
            moderators: ContractModerators::AppointedModerators(BTreeSet::from(ids)),
        };
        let config: DataContractConfig = saved.config().clone();
        saved.set_config(config.with_moderation(Some(team([a, b]))));
        ctx.replace_contract(saved.id(), &saved).unwrap();
        let original = SpvProvider::new(ctx.network);
        original.bind_app_context(ctx);
        let claim = original.for_fee_claim(saved.id());
        let verifier = claim.clone();
        let mut current = saved.clone();
        current.set_version(saved.version() + 1);
        let config: DataContractConfig = current.config().clone();
        current.set_config(config.with_moderation(Some(team([a, c]))));
        claim.register_data_contract(Arc::new(current));
        let resolved = verifier
            .get_data_contract(&saved.id(), version)
            .unwrap()
            .unwrap();
        assert_eq!(
            ContractFeePot::Moderators.claim_proof_identities(&resolved, a),
            BTreeSet::from([a, c])
        );
        claim.register_data_contract(Arc::new(saved.clone()));
        assert_eq!(
            verifier
                .get_data_contract(&saved.id(), version)
                .unwrap()
                .unwrap()
                .version(),
            saved.version() + 1
        );
        assert_eq!(
            *original
                .get_data_contract(&saved.id(), version)
                .unwrap()
                .unwrap(),
            saved
        );
        assert_eq!(
            *original
                .for_fee_claim(saved.id())
                .get_data_contract(&saved.id(), version)
                .unwrap()
                .unwrap(),
            saved
        );
    }
}
