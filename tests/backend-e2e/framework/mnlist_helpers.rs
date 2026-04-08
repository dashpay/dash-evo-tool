//! Helpers for retrieving block info for MnList tests.
//!
//! Uses DAPI (Platform gRPC, always available via testnet nodes) for chain tip
//! and the well-known genesis block hash from the `Network` type.
//! No Core RPC required.

// TODO(production-reuse): This helper parallels `src/backend_task/mnlist.rs` and
// `src/ui/tools/masternode_list_diff_screen.rs::get_block_hash`.
// Before extracting to production, diff against the original source — it may have
// changed since this helper was written (created 2026-04-08 based on commit 79a6907c).
// The production code undergoes heavy refactoring; inspect for divergence before reuse.

use dash_evo_tool::context::AppContext;
use dash_sdk::SdkBuilder;
use dash_sdk::dapi_client::{AddressList, DapiRequestExecutor, IntoInner, RequestSettings};
use dash_sdk::dapi_grpc::core::v0::GetBlockchainStatusRequest;
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::prelude::CoreBlockHeight;
use dash_sdk::error::ContextProviderError;
use dash_sdk::platform::ContextProvider;
use std::sync::Arc;

/// No-op ContextProvider for the lightweight DAPI-only SDK instance.
struct NoopContextProvider;

impl ContextProvider for NoopContextProvider {
    fn get_data_contract(
        &self,
        _id: &dash_sdk::platform::Identifier,
        _pv: &dash_sdk::dpp::version::PlatformVersion,
    ) -> Result<Option<Arc<dash_sdk::dpp::prelude::DataContract>>, ContextProviderError> {
        Ok(None)
    }

    fn get_token_configuration(
        &self,
        _token_id: &dash_sdk::platform::Identifier,
    ) -> Result<Option<dash_sdk::dpp::data_contract::TokenConfiguration>, ContextProviderError>
    {
        Ok(None)
    }

    fn get_quorum_public_key(
        &self,
        _quorum_type: u32,
        _quorum_hash: [u8; 32],
        _core_chain_locked_height: u32,
    ) -> Result<[u8; 48], ContextProviderError> {
        Err(ContextProviderError::Config(
            "NoopContextProvider: quorum keys not needed for DAPI status queries".into(),
        ))
    }

    fn get_platform_activation_height(&self) -> Result<CoreBlockHeight, ContextProviderError> {
        Err(ContextProviderError::Config(
            "NoopContextProvider: activation height not needed for DAPI status queries".into(),
        ))
    }
}

/// Build a lightweight SDK connected to testnet DAPI nodes.
///
/// Only used for `GetBlockchainStatus` calls — no proof verification needed.
fn build_dapi_sdk(app_context: &Arc<AppContext>) -> dash_sdk::Sdk {
    let raw = std::env::var("TESTNET_dapi_addresses")
        .expect("mnlist_helpers: TESTNET_dapi_addresses env var not set — ensure .env is loaded");

    let address_list: AddressList = raw
        .parse()
        .expect("mnlist_helpers: invalid TESTNET_dapi_addresses");

    SdkBuilder::new(address_list)
        .with_network(app_context.network())
        .with_version(app_context.platform_version())
        .with_context_provider(NoopContextProvider)
        .build()
        .expect("mnlist_helpers: failed to build DAPI SDK")
}

/// Get current block tip height and hash from DAPI (Platform gRPC).
///
/// Uses `GetBlockchainStatus` which is always available through DAPI nodes —
/// no Core RPC node required.
///
/// # Panics
///
/// Panics if the DAPI request fails or the response is missing chain info.
pub async fn get_current_block_info(app_context: &Arc<AppContext>) -> (u32, BlockHash) {
    let sdk = build_dapi_sdk(app_context);

    let response = sdk
        .execute(GetBlockchainStatusRequest {}, RequestSettings::default())
        .await
        .into_inner()
        .expect("mnlist_helpers: GetBlockchainStatus DAPI request failed");

    let chain = response
        .chain
        .expect("mnlist_helpers: GetBlockchainStatus response missing chain info");

    let height = chain.blocks_count;
    let hash_bytes: [u8; 32] = chain
        .best_block_hash
        .try_into()
        .expect("mnlist_helpers: best_block_hash is not 32 bytes");
    let block_hash = BlockHash::from_byte_array(hash_bytes);

    (height, block_hash)
}

/// Get the well-known genesis block hash for the current network.
///
/// Uses `Network::known_genesis_block_hash()` — a compile-time constant
/// for mainnet and testnet. No network call required.
///
/// # Panics
///
/// Panics on devnet/regtest networks where genesis hash is not known.
pub fn get_genesis_hash(app_context: &Arc<AppContext>) -> BlockHash {
    app_context
        .network()
        .known_genesis_block_hash()
        .expect("mnlist_helpers: genesis block hash not known for this network (devnet/regtest?)")
}
