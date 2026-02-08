use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::bls_sig_utils::BLSSignature;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo;
use dash_sdk::dpp::dashcore::network::message_sml::MnListDiff;
use dash_sdk::dpp::dashcore::sml::llmq_type::LLMQType;
use dash_sdk::dpp::dashcore::sml::masternode_list_engine::MasternodeListEngine;
use dash_sdk::dpp::dashcore::sml::quorum_entry::qualified_quorum_entry::QualifiedQuorumEntry;
use dash_sdk::dpp::dashcore::transaction::special_transaction::quorum_commitment::QuorumEntry;
use dash_sdk::dpp::dashcore::{BlockHash, BlockHash as BlockHash2};
use dash_sdk::dpp::prelude::CoreBlockHeight;
use std::collections::{BTreeMap, BTreeSet};

/// Holds block height, block hash, and chain lock signature caches.
///
/// These caches avoid repeated RPC calls to Dash Core for block header lookups.
/// Methods fall back through: engine block container -> local cache -> RPC call.
#[derive(Default)]
pub(super) struct CacheState {
    /// Maps block hash -> block height
    pub block_height_cache: BTreeMap<BlockHash, CoreBlockHeight>,
    /// Maps block height -> block hash
    pub block_hash_cache: BTreeMap<CoreBlockHeight, BlockHash>,
    /// Maps block hash -> (quorum type -> list of (height, qualified quorum entry))
    pub masternode_list_quorum_hash_cache:
        BTreeMap<BlockHash, BTreeMap<LLMQType, Vec<(CoreBlockHeight, QualifiedQuorumEntry)>>>,
    /// Maps (height, block_hash) -> optional BLS chain lock signature
    pub chain_lock_sig_cache: BTreeMap<(CoreBlockHeight, BlockHash), Option<BLSSignature>>,
    /// Reverse index: BLS signature -> set of (height, block_hash) pairs
    pub chain_lock_reversed_sig_cache:
        BTreeMap<BLSSignature, BTreeSet<(CoreBlockHeight, BlockHash)>>,
}

impl CacheState {
    /// Clear all caches.
    pub fn clear(&mut self) {
        self.block_height_cache.clear();
        self.block_hash_cache.clear();
        self.masternode_list_quorum_hash_cache.clear();
        self.chain_lock_sig_cache.clear();
        self.chain_lock_reversed_sig_cache.clear();
    }

    /// Clear only chain lock signature caches.
    pub fn clear_chain_lock_caches(&mut self) {
        self.chain_lock_sig_cache.clear();
        self.chain_lock_reversed_sig_cache.clear();
    }

    /// Look up the height for a block hash without caching.
    /// Falls back through: engine block container -> local cache -> RPC call.
    pub fn get_height(
        &self,
        block_hash: &BlockHash,
        engine: &MasternodeListEngine,
        app_context: &AppContext,
    ) -> Result<CoreBlockHeight, String> {
        if let Some(height) = engine.block_container.get_height(block_hash) {
            return Ok(height);
        }
        if let Some(height) = self.block_height_cache.get(block_hash) {
            return Ok(*height);
        }
        tracing::debug!(
            "Asking core for height no cache {} ({})",
            block_hash,
            block_hash.reverse()
        );
        app_context
            .core_client
            .read_or_recover()
            .get_block_header_info(&BlockHash2::from_byte_array(block_hash.to_byte_array()))
            .map(|info| info.height as CoreBlockHeight)
            .map_err(|e| e.to_string())
    }

    /// Look up the height for a block hash, converting errors to display strings.
    pub fn get_height_or_error_as_string(
        &self,
        block_hash: &BlockHash,
        engine: &MasternodeListEngine,
        app_context: &AppContext,
    ) -> String {
        match self.get_height(block_hash, engine, app_context) {
            Ok(height) => height.to_string(),
            Err(e) => format!("Failed to get height for {}: {}", block_hash, e),
        }
    }

    /// Look up the height for a block hash, caching the result and feeding it to the engine.
    pub fn get_height_and_cache(
        &mut self,
        block_hash: &BlockHash,
        engine: &mut MasternodeListEngine,
        app_context: &AppContext,
    ) -> Result<CoreBlockHeight, String> {
        if let Some(height) = engine.block_container.get_height(block_hash) {
            return Ok(height);
        }
        if let Some(height) = self.block_height_cache.get(block_hash) {
            return Ok(*height);
        }
        tracing::debug!(
            "Asking core for height {} ({})",
            block_hash,
            block_hash.reverse()
        );
        match app_context
            .core_client
            .read_or_recover()
            .get_block_header_info(&BlockHash2::from_byte_array(block_hash.to_byte_array()))
        {
            Ok(result) => {
                let height = result.height as CoreBlockHeight;
                self.block_height_cache.insert(*block_hash, height);
                engine.feed_block_height(height, *block_hash);
                Ok(height)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Look up height with caching, converting errors to display strings.
    #[allow(dead_code)]
    pub fn get_height_and_cache_or_error_as_string(
        &mut self,
        block_hash: &BlockHash,
        engine: &mut MasternodeListEngine,
        app_context: &AppContext,
    ) -> String {
        match self.get_height_and_cache(block_hash, engine, app_context) {
            Ok(height) => height.to_string(),
            Err(e) => format!("Failed to get height for {}: {}", block_hash, e),
        }
    }

    /// Look up the block hash for a height without caching.
    pub fn get_block_hash(
        &self,
        height: CoreBlockHeight,
        engine: &MasternodeListEngine,
        app_context: &AppContext,
    ) -> Result<BlockHash, String> {
        if let Some(block_hash) = engine.block_container.get_hash(&height) {
            return Ok(*block_hash);
        }
        if let Some(block_hash) = self.block_hash_cache.get(&height) {
            return Ok(*block_hash);
        }
        app_context
            .core_client
            .read_or_recover()
            .get_block_hash(height)
            .map(|h| BlockHash::from_byte_array(h.to_byte_array()))
            .map_err(|e| e.to_string())
    }

    /// Look up the block hash for a height, caching the result.
    #[allow(dead_code)]
    pub fn get_block_hash_and_cache(
        &mut self,
        height: CoreBlockHeight,
        engine: &MasternodeListEngine,
        app_context: &AppContext,
    ) -> Result<BlockHash, String> {
        if let Some(block_hash) = engine.block_container.get_hash(&height) {
            return Ok(*block_hash);
        }
        if let Some(cached_hash) = self.block_hash_cache.get(&height) {
            return Ok(*cached_hash);
        }
        match app_context
            .core_client
            .read_or_recover()
            .get_block_hash(height)
        {
            Ok(core_block_hash) => {
                let block_hash = BlockHash::from_byte_array(core_block_hash.to_byte_array());
                self.block_hash_cache.insert(height, block_hash);
                Ok(block_hash)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Look up the chain lock signature for a block hash without caching.
    pub fn get_chain_lock_sig(
        &self,
        block_hash: &BlockHash,
        engine: &MasternodeListEngine,
        app_context: &AppContext,
    ) -> Result<Option<BLSSignature>, String> {
        let height = self.get_height(block_hash, engine, app_context)?;
        if let Some(sig) = self.chain_lock_sig_cache.get(&(height, *block_hash)) {
            return Ok(*sig);
        }
        let block = app_context
            .core_client
            .read_or_recover()
            .get_block(&BlockHash2::from_byte_array(block_hash.to_byte_array()))
            .map_err(|e| e.to_string())?;
        let Some(coinbase) = block
            .coinbase()
            .and_then(|coinbase| coinbase.special_transaction_payload.as_ref())
            .and_then(|payload| payload.clone().to_coinbase_payload().ok())
        else {
            return Err(format!("coinbase not found on block hash {}", block_hash));
        };
        Ok(coinbase.best_cl_signature.map(|sig| sig.to_bytes().into()))
    }

    /// Look up the chain lock signature for a block hash, caching the result.
    #[allow(dead_code)]
    pub fn get_chain_lock_sig_and_cache(
        &mut self,
        block_hash: &BlockHash,
        engine: &mut MasternodeListEngine,
        app_context: &AppContext,
    ) -> Result<Option<BLSSignature>, String> {
        let height = self.get_height_and_cache(block_hash, engine, app_context)?;
        if self
            .chain_lock_sig_cache
            .contains_key(&(height, *block_hash))
        {
            return Ok(*self
                .chain_lock_sig_cache
                .get(&(height, *block_hash))
                .unwrap());
        }
        let block = app_context
            .core_client
            .read_or_recover()
            .get_block(&BlockHash2::from_byte_array(block_hash.to_byte_array()))
            .map_err(|e| e.to_string())?;
        let Some(coinbase) = block
            .coinbase()
            .and_then(|coinbase| coinbase.special_transaction_payload.as_ref())
            .and_then(|payload| payload.clone().to_coinbase_payload().ok())
        else {
            return Err(format!("coinbase not found on block hash {}", block_hash));
        };
        self.chain_lock_sig_cache.insert(
            (height, *block_hash),
            coinbase.best_cl_signature.map(|sig| sig.to_bytes().into()),
        );
        if let Some(sig) = coinbase.best_cl_signature.map(|sig| sig.to_bytes().into()) {
            self.chain_lock_reversed_sig_cache
                .entry(sig)
                .or_default()
                .insert((height, *block_hash));
        }
        Ok(*self
            .chain_lock_sig_cache
            .get(&(height, *block_hash))
            .unwrap())
    }

    /// Feed all block heights from a QRInfo into the engine.
    #[allow(dead_code)]
    pub fn feed_qr_info_block_heights(
        &self,
        qr_info: &QRInfo,
        engine: &mut MasternodeListEngine,
        app_context: &AppContext,
        error: &mut Option<String>,
    ) {
        let mn_list_diffs = [
            &qr_info.mn_list_diff_tip,
            &qr_info.mn_list_diff_h,
            &qr_info.mn_list_diff_at_h_minus_c,
            &qr_info.mn_list_diff_at_h_minus_2c,
            &qr_info.mn_list_diff_at_h_minus_3c,
        ];

        if let Some((_, mn_list_diff_h_minus_4c)) =
            &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c
        {
            for mn_list_diff in &mn_list_diffs {
                self.feed_mn_list_diff_heights(mn_list_diff, engine, app_context, error);
            }
            self.feed_mn_list_diff_heights(mn_list_diff_h_minus_4c, engine, app_context, error);
        } else {
            for mn_list_diff in &mn_list_diffs {
                self.feed_mn_list_diff_heights(mn_list_diff, engine, app_context, error);
            }
        }

        for quorum_entry in &qr_info.last_commitment_per_index {
            self.feed_quorum_entry_height(quorum_entry, engine, app_context, error);
        }

        for mn_list_diff in &qr_info.mn_list_diff_list {
            self.feed_mn_list_diff_heights(mn_list_diff, engine, app_context, error);
        }
    }

    /// Feed the base and block hash heights of an MnListDiff into the engine.
    pub fn feed_mn_list_diff_heights(
        &self,
        mn_list_diff: &MnListDiff,
        engine: &mut MasternodeListEngine,
        app_context: &AppContext,
        error: &mut Option<String>,
    ) {
        if let Ok(base_height) = self.get_height(&mn_list_diff.base_block_hash, engine, app_context)
        {
            tracing::debug!("feeding {} {}", base_height, mn_list_diff.base_block_hash);
            engine.feed_block_height(base_height, mn_list_diff.base_block_hash);
        } else {
            *error = Some(format!(
                "Failed to get height for base block hash: {}",
                mn_list_diff.base_block_hash
            ));
        }

        if let Ok(block_height) = self.get_height(&mn_list_diff.block_hash, engine, app_context) {
            tracing::debug!("feeding {} {}", block_height, mn_list_diff.block_hash);
            engine.feed_block_height(block_height, mn_list_diff.block_hash);
        } else {
            *error = Some(format!(
                "Failed to get height for block hash: {}",
                mn_list_diff.block_hash
            ));
        }
    }

    /// Feed the quorum hash height of a QuorumEntry into the engine.
    pub fn feed_quorum_entry_height(
        &self,
        quorum_entry: &QuorumEntry,
        engine: &mut MasternodeListEngine,
        app_context: &AppContext,
        error: &mut Option<String>,
    ) {
        if let Ok(height) = self.get_height(&quorum_entry.quorum_hash, engine, app_context) {
            engine.feed_block_height(height, quorum_entry.quorum_hash);
        } else {
            *error = Some(format!(
                "Failed to get height for quorum hash: {}",
                quorum_entry.quorum_hash
            ));
        }
    }
}
