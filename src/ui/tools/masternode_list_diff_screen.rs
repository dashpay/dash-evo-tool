use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::mnlist::MnListTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::components::core_p2p_handler::CoreP2PHandler;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::json::QuorumType;
use dash_sdk::dpp::dashcore::bls_sig_utils::BLSSignature;
use dash_sdk::dpp::dashcore::consensus::serialize as serialize2;
use dash_sdk::dpp::dashcore::consensus::{Decodable, deserialize, serialize};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::network::constants::NetworkExt;
use dash_sdk::dpp::dashcore::network::message_qrinfo::{QRInfo, QuorumSnapshot};
use dash_sdk::dpp::dashcore::network::message_sml::MnListDiff;
use dash_sdk::dpp::dashcore::sml::llmq_entry_verification::LLMQEntryVerificationStatus;
use dash_sdk::dpp::dashcore::sml::llmq_type::LLMQType;
use dash_sdk::dpp::dashcore::sml::masternode_list::MasternodeList;
use dash_sdk::dpp::dashcore::sml::masternode_list_engine::{
    MasternodeListEngine, MasternodeListEngineBlockContainer,
};
use dash_sdk::dpp::dashcore::sml::masternode_list_entry::EntryMasternodeType;
use dash_sdk::dpp::dashcore::sml::masternode_list_entry::qualified_masternode_list_entry::QualifiedMasternodeListEntry;
use dash_sdk::dpp::dashcore::sml::quorum_entry::qualified_quorum_entry::{
    QualifiedQuorumEntry, VerifyingChainLockSignaturesType,
};
use dash_sdk::dpp::dashcore::sml::quorum_validation_error::ClientDataRetrievalError;
use dash_sdk::dpp::dashcore::transaction::special_transaction::quorum_commitment::QuorumEntry;
use dash_sdk::dpp::dashcore::{
    Block, BlockHash as BlockHash2, ChainLock, InstantLock, Transaction,
};
use dash_sdk::dpp::dashcore::{
    BlockHash, ChainLock as ChainLock2, InstantLock as InstantLock2, Network, ProTxHash, QuorumHash,
};
use dash_sdk::dpp::prelude::CoreBlockHeight;
use eframe::egui::{self, Context, ScrollArea, Ui};
use egui::{Align, Frame, Layout, Margin, RichText, Stroke, TextEdit, Vec2};
use itertools::Itertools;
use rfd::FileDialog;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

type HeightHash = (u32, BlockHash);

enum SelectedQRItem {
    SelectedSnapshot(QuorumSnapshot),
    MNListDiff(Box<MnListDiff>),
    QuorumEntry(Box<QualifiedQuorumEntry>),
}

/// User-entered inputs and transient filters.
#[derive(Default)]
struct InputState {
    base_block_height: String,
    end_block_height: String,
    search_term: Option<String>,
}

/// UI presentation state (tabs, banners, dialogs).
#[derive(Default)]
struct UiState {
    selected_tab: usize,
    show_popup_for_render_masternode_list_engine: bool,
    error: Option<String>,
}

/// Backend task state and sync toggles.
#[derive(Default)]
struct TaskState {
    syncing: bool,
    pending: Option<PendingTask>,
    queued_task: Option<BackendTask>,
}

/// Domain data for the masternode list diff tool.
struct MnListData {
    masternode_list_engine: MasternodeListEngine,
    mnlist_diffs: BTreeMap<(CoreBlockHeight, CoreBlockHeight), MnListDiff>,
    qr_infos: BTreeMap<BlockHash, QRInfo>,
}

impl MnListData {
    fn new(app_context: &Arc<AppContext>) -> Self {
        let mut mnlist_diffs = BTreeMap::new();
        let masternode_list_engine = match app_context.network {
            Network::Dash => {
                use std::env;
                tracing::debug!(
                    "Current working directory: {:?}",
                    env::current_dir().unwrap()
                );
                let file_path = "artifacts/mn_list_diff_0_2227096.bin";
                // Attempt to load and parse the MNListDiff file
                if Path::new(file_path).exists() {
                    match fs::read(file_path) {
                        Ok(bytes) => {
                            let diff: MnListDiff =
                                deserialize(bytes.as_slice()).expect("expected to deserialize");
                            mnlist_diffs.insert((0, 2227096), diff.clone());
                            MasternodeListEngine::initialize_with_diff_to_height(
                                diff,
                                2227096,
                                Network::Dash,
                            )
                            .expect("expected to start engine")
                        }
                        Err(e) => {
                            tracing::error!("Failed to read MNListDiff file: {}", e);
                            MasternodeListEngine::default_for_network(Network::Dash)
                        }
                    }
                } else {
                    tracing::warn!("MNListDiff file not found: {}", file_path);
                    MasternodeListEngine::default_for_network(Network::Dash)
                }
            }
            Network::Testnet => {
                let file_path = "artifacts/mn_list_diff_testnet_0_1296600.bin";
                // Attempt to load and parse the MNListDiff file
                if Path::new(file_path).exists() {
                    match fs::read(file_path) {
                        Ok(bytes) => {
                            let diff: MnListDiff =
                                deserialize(bytes.as_slice()).expect("expected to deserialize");
                            mnlist_diffs.insert((0, 1296600), diff.clone());
                            MasternodeListEngine::initialize_with_diff_to_height(
                                diff,
                                1296600,
                                Network::Testnet,
                            )
                            .expect("expected to start engine")
                        }
                        Err(e) => {
                            tracing::error!("Failed to read MNListDiff file: {}", e);
                            MasternodeListEngine::default_for_network(Network::Testnet)
                        }
                    }
                } else {
                    tracing::warn!("MNListDiff file not found: {}", file_path);
                    MasternodeListEngine::default_for_network(Network::Dash)
                }
            }
            _ => MasternodeListEngine::default_for_network(app_context.network),
        };

        Self {
            masternode_list_engine,
            mnlist_diffs,
            qr_infos: Default::default(),
        }
    }
}

/// Derived caches to avoid repeated lookups or recomputation.
#[derive(Default)]
struct CacheState {
    masternode_lists_with_all_quorum_heights_known: BTreeSet<CoreBlockHeight>,
    dml_diffs_with_cached_quorum_heights: HashSet<(CoreBlockHeight, CoreBlockHeight)>,
    block_height_cache: BTreeMap<BlockHash, CoreBlockHeight>,
    block_hash_cache: BTreeMap<CoreBlockHeight, BlockHash>,
    masternode_list_quorum_hash_cache:
        BTreeMap<BlockHash, BTreeMap<LLMQType, Vec<(CoreBlockHeight, QualifiedQuorumEntry)>>>,
    chain_lock_sig_cache: BTreeMap<(CoreBlockHeight, BlockHash), Option<BLSSignature>>,
    chain_lock_reversed_sig_cache: BTreeMap<BLSSignature, BTreeSet<(CoreBlockHeight, BlockHash)>>,
}

/// User selection state for lists and detail panes.
#[derive(Default)]
struct SelectionState {
    selected_dml_diff_key: Option<(CoreBlockHeight, CoreBlockHeight)>,
    selected_dml_height_key: Option<CoreBlockHeight>,
    selected_option_index: Option<usize>,
    selected_quorum_in_diff_index: Option<usize>,
    selected_masternode_in_diff_index: Option<usize>,
    selected_quorum_hash_in_mnlist_diff: Option<(LLMQType, QuorumHash)>,
    selected_quorum_type_in_quorum_viewer: Option<LLMQType>,
    selected_quorum_hash_in_quorum_viewer: Option<QuorumHash>,
    selected_masternode_pro_tx_hash: Option<ProTxHash>,
    selected_qr_field: Option<String>,
    selected_qr_list_index: Option<String>,
    selected_core_item: Option<(CoreItem, bool)>,
    selected_qr_item: Option<SelectedQRItem>,
}

/// Incoming core items received via ZMQ or backend tasks.
#[derive(Default)]
struct IncomingState {
    chain_locked_blocks: BTreeMap<CoreBlockHeight, (Block, ChainLock, bool)>,
    instant_send_transactions: Vec<(Transaction, InstantLock, bool)>,
}

/// Screen for viewing MNList diffs (diffs in the masternode list and quorums)
pub struct MasternodeListDiffScreen {
    pub app_context: Arc<AppContext>,
    input: InputState,
    ui_state: UiState,
    task: TaskState,
    data: MnListData,
    cache: CacheState,
    selection: SelectionState,
    incoming: IncomingState,
}

impl MasternodeListDiffScreen {
    /// Create a new MNListDiffScreen
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let data = MnListData::new(app_context);
        Self {
            app_context: app_context.clone(),
            input: InputState::default(),
            ui_state: UiState::default(),
            task: TaskState::default(),
            data,
            cache: CacheState::default(),
            selection: SelectionState::default(),
            incoming: IncomingState::default(),
        }
    }

    fn selected_dml(&self) -> Option<&MnListDiff> {
        self.selection
            .selected_dml_diff_key
            .and_then(|key| self.data.mnlist_diffs.get(&key))
    }

    fn selected_mn_list(&self) -> Option<&MasternodeList> {
        self.selection.selected_dml_height_key.and_then(|height| {
            self.data
                .masternode_list_engine
                .masternode_lists
                .get(&height)
        })
    }

    fn known_block_hashes_with_base(&self, base_hash: BlockHash) -> Vec<BlockHash> {
        let mut known_block_hashes: Vec<_> = self
            .data
            .mnlist_diffs
            .values()
            .map(|mn_list_diff| mn_list_diff.block_hash)
            .collect();
        known_block_hashes.push(base_hash);
        known_block_hashes
    }

    fn get_height_or_error_as_string(&self, block_hash: &BlockHash) -> String {
        match self.get_height(block_hash) {
            Ok(height) => height.to_string(),
            Err(e) => format!("Failed to get height for {}: {}", block_hash, e),
        }
    }

    /// Build a backend task that fetches the extra diffs needed to validate non-rotating quorums.
    /// Returns None if requirements cannot be computed.
    fn build_validation_diffs_task(&mut self) -> Option<BackendTask> {
        // Determine hashes we need to validate
        let hashes = self
            .data
            .masternode_list_engine
            .latest_masternode_list_non_rotating_quorum_hashes(
                &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                true,
            );
        if hashes.is_empty() {
            return None;
        }

        // Compute target validation heights (h-8)
        let mut heights: BTreeSet<u32> = BTreeSet::new();
        for quorum_hash in &hashes {
            if let Ok(h) = self.get_height_and_cache(quorum_hash)
                && h >= 8
            {
                heights.insert(h - 8);
            }
        }
        if heights.is_empty() {
            return None;
        }

        let client = self.app_context.core_client.read().unwrap();
        let mut chain: Vec<(u32, BlockHash, u32, BlockHash)> = Vec::new();

        // Determine base starting point similar to previous logic
        let (first_engine_height, first_engine_hash_opt) = self
            .data
            .masternode_list_engine
            .masternode_lists
            .first_key_value()
            .map(|(h, l)| (*h, Some(l.block_hash)))
            .unwrap_or((0, None));

        let oldest_needed = *heights.first().unwrap();
        let mut base_height: u32;
        let mut base_hash: BlockHash;
        if first_engine_height != 0 && first_engine_height < oldest_needed {
            base_height = first_engine_height;
            base_hash = first_engine_hash_opt.unwrap();
        } else {
            // Use genesis as base
            base_height = 0;
            let Ok(genesis) = client.get_block_hash(0) else {
                return None;
            };
            base_hash = BlockHash::from_byte_array(genesis.to_byte_array());
        }

        for h in heights {
            let Ok(bh) = client.get_block_hash(h) else {
                continue;
            };
            let bh = BlockHash::from_byte_array(bh.to_byte_array());
            chain.push((base_height, base_hash, h, bh));
            base_height = h;
            base_hash = bh;
        }

        if chain.is_empty() {
            return None;
        }
        Some(BackendTask::MnListTask(MnListTask::FetchDiffsChain {
            chain,
        }))
    }

    fn get_height(&self, block_hash: &BlockHash) -> Result<CoreBlockHeight, String> {
        let Some(height) = self
            .data
            .masternode_list_engine
            .block_container
            .get_height(block_hash)
        else {
            let Some(height) = self.cache.block_height_cache.get(block_hash) else {
                tracing::debug!(
                    "Asking core for height no cache {} ({})",
                    block_hash,
                    block_hash.reverse()
                );
                return match self
                    .app_context
                    .core_client
                    .read()
                    .unwrap()
                    .get_block_header_info(
                        &(BlockHash2::from_byte_array(block_hash.to_byte_array())),
                    ) {
                    Ok(block_hash) => Ok(block_hash.height as CoreBlockHeight),
                    Err(e) => Err(e.to_string()),
                };
            };
            return Ok(*height);
        };
        Ok(height)
    }

    #[allow(dead_code)]
    fn get_height_and_cache_or_error_as_string(&mut self, block_hash: &BlockHash) -> String {
        match self.get_height_and_cache(block_hash) {
            Ok(height) => height.to_string(),
            Err(e) => format!("Failed to get height for {}: {}", block_hash, e),
        }
    }

    fn get_height_and_cache(&mut self, block_hash: &BlockHash) -> Result<CoreBlockHeight, String> {
        let Some(height) = self
            .data
            .masternode_list_engine
            .block_container
            .get_height(block_hash)
        else {
            let Some(height) = self.cache.block_height_cache.get(block_hash) else {
                tracing::debug!(
                    "Asking core for height {} ({})",
                    block_hash,
                    block_hash.reverse()
                );
                return match self
                    .app_context
                    .core_client
                    .read()
                    .unwrap()
                    .get_block_header_info(
                        &(BlockHash2::from_byte_array(block_hash.to_byte_array())),
                    ) {
                    Ok(result) => {
                        self.cache
                            .block_height_cache
                            .insert(*block_hash, result.height as CoreBlockHeight);
                        self.data
                            .masternode_list_engine
                            .feed_block_height(result.height as CoreBlockHeight, *block_hash);
                        Ok(result.height as CoreBlockHeight)
                    }
                    Err(e) => Err(e.to_string()),
                };
            };
            return Ok(*height);
        };
        Ok(height)
    }

    #[allow(dead_code)]
    fn get_chain_lock_sig_and_cache(
        &mut self,
        block_hash: &BlockHash,
    ) -> Result<Option<BLSSignature>, String> {
        let height = self.get_height_and_cache(block_hash)?;
        if !self
            .cache
            .chain_lock_sig_cache
            .contains_key(&(height, *block_hash))
        {
            let block = self
                .app_context
                .core_client
                .read()
                .unwrap()
                .get_block(&(BlockHash2::from_byte_array(block_hash.to_byte_array())))
                .map_err(|e| e.to_string())?;
            let Some(coinbase) = block
                .coinbase()
                .and_then(|coinbase| coinbase.special_transaction_payload.as_ref())
                .and_then(|payload| payload.clone().to_coinbase_payload().ok())
            else {
                return Err(format!("coinbase not found on block hash {}", block_hash));
            };
            //todo clean up
            self.cache.chain_lock_sig_cache.insert(
                (height, *block_hash),
                coinbase.best_cl_signature.map(|sig| sig.to_bytes().into()),
            );
            if let Some(sig) = coinbase.best_cl_signature.map(|sig| sig.to_bytes().into()) {
                self.cache
                    .chain_lock_reversed_sig_cache
                    .entry(sig)
                    .or_default()
                    .insert((height, *block_hash));
            }
        }

        Ok(*self
            .cache
            .chain_lock_sig_cache
            .get(&(height, *block_hash))
            .unwrap())
    }

    fn get_chain_lock_sig(&self, block_hash: &BlockHash) -> Result<Option<BLSSignature>, String> {
        let height = self.get_height(block_hash)?;
        if !self
            .cache
            .chain_lock_sig_cache
            .contains_key(&(height, *block_hash))
        {
            let block = self
                .app_context
                .core_client
                .read()
                .unwrap()
                .get_block(&(BlockHash2::from_byte_array(block_hash.to_byte_array())))
                .map_err(|e| e.to_string())?;
            let Some(coinbase) = block
                .coinbase()
                .and_then(|coinbase| coinbase.special_transaction_payload.as_ref())
                .and_then(|payload| payload.clone().to_coinbase_payload().ok())
            else {
                return Err(format!("coinbase not found on block hash {}", block_hash));
            };
            Ok(coinbase.best_cl_signature.map(|sig| sig.to_bytes().into()))
        } else {
            Ok(*self
                .cache
                .chain_lock_sig_cache
                .get(&(height, *block_hash))
                .unwrap())
        }
    }

    fn get_block_hash(&self, height: CoreBlockHeight) -> Result<BlockHash, String> {
        let Some(block_hash) = self
            .data
            .masternode_list_engine
            .block_container
            .get_hash(&height)
        else {
            let Some(block_hash) = self.cache.block_hash_cache.get(&height) else {
                // println!("Asking core for hash of {}", height);
                return match self
                    .app_context
                    .core_client
                    .read()
                    .unwrap()
                    .get_block_hash(height)
                {
                    Ok(block_hash) => Ok(BlockHash::from_byte_array(block_hash.to_byte_array())),
                    Err(e) => Err(e.to_string()),
                };
            };
            return Ok(*block_hash);
        };
        Ok(*block_hash)
    }

    #[allow(dead_code)]
    fn get_block_hash_and_cache(&mut self, height: CoreBlockHeight) -> Result<BlockHash, String> {
        // First, try to get the hash from masternode_list_engine's block_container.
        if let Some(block_hash) = self
            .data
            .masternode_list_engine
            .block_container
            .get_hash(&height)
        {
            return Ok(*block_hash);
        }

        // Then, check the cache.
        if let Some(cached_hash) = self.cache.block_hash_cache.get(&height) {
            return Ok(*cached_hash);
        }

        // If not cached, retrieve from core client and insert into cache.
        // println!("Asking core for hash of {} and caching it", height);
        match self
            .app_context
            .core_client
            .read()
            .unwrap()
            .get_block_hash(height)
        {
            Ok(core_block_hash) => {
                let block_hash = BlockHash::from_byte_array(core_block_hash.to_byte_array());
                self.cache.block_hash_cache.insert(height, block_hash);
                Ok(block_hash)
            }
            Err(e) => Err(e.to_string()),
        }
    }
    //
    // fn feed_qr_info_cl_sigs(&mut self, qr_info: &QRInfo) {
    //     let heights = match self.data.masternode_list_engine.required_cl_sig_heights(qr_info) {
    //         Ok(heights) => heights,
    //         Err(e) => {
    //             self.ui_state.error = Some(e.to_string());
    //             return;
    //         }
    //     };
    //     for height in heights {
    //         let block_hash = match self.get_block_hash(height) {
    //             Ok(block_hash) => block_hash,
    //             Err(e) => {
    //                 self.ui_state.error = Some(e.to_string());
    //                 return;
    //             }
    //         };
    //         let maybe_chain_lock_sig = match self
    //             .app_context
    //             .core_client
    //             .get_block(&(BlockHash2::from_byte_array(block_hash.to_byte_array())))
    //         {
    //             Ok(block) => {
    //                 let Some(coinbase) = block
    //                     .coinbase()
    //                     .and_then(|coinbase| coinbase.special_transaction_payload.as_ref())
    //                     .and_then(|payload| payload.clone().to_coinbase_payload().ok())
    //                 else {
    //                     self.ui_state.error =
    //                         Some(format!("coinbase not found on block hash {}", block_hash));
    //                     return;
    //                 };
    //                 coinbase.best_cl_signature
    //             }
    //             Err(e) => {
    //                 self.ui_state.error = Some(e.to_string());
    //                 return;
    //             }
    //         };
    //         if let Some(maybe_chain_lock_sig) = maybe_chain_lock_sig {
    //             self.data.masternode_list_engine.feed_chain_lock_sig(
    //                 block_hash,
    //                 BLSSignature::from(maybe_chain_lock_sig.to_bytes()),
    //             );
    //         }
    //     }
    // }

    #[allow(dead_code)]
    fn feed_qr_info_block_heights(&mut self, qr_info: &QRInfo) {
        let mn_list_diffs = [
            &qr_info.mn_list_diff_tip,
            &qr_info.mn_list_diff_h,
            &qr_info.mn_list_diff_at_h_minus_c,
            &qr_info.mn_list_diff_at_h_minus_2c,
            &qr_info.mn_list_diff_at_h_minus_3c,
        ];

        // If h-4c exists, add it to the list
        if let Some((_, mn_list_diff_h_minus_4c)) =
            &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c
        {
            mn_list_diffs.iter().for_each(|&mn_list_diff| {
                self.feed_mn_list_diff_heights(mn_list_diff);
            });

            // Feed h-4c separately
            self.feed_mn_list_diff_heights(mn_list_diff_h_minus_4c);
        } else {
            mn_list_diffs.iter().for_each(|&mn_list_diff| {
                self.feed_mn_list_diff_heights(mn_list_diff);
            });
        }

        // Process `last_commitment_per_index` quorum hashes
        qr_info
            .last_commitment_per_index
            .iter()
            .for_each(|quorum_entry| {
                self.feed_quorum_entry_height(quorum_entry);
            });

        // Process `mn_list_diff_list` (extra diffs)
        qr_info.mn_list_diff_list.iter().for_each(|mn_list_diff| {
            self.feed_mn_list_diff_heights(mn_list_diff);
        });
    }

    /// **Helper function:** Feeds the base and block hash heights of an `MnListDiff`
    fn feed_mn_list_diff_heights(&mut self, mn_list_diff: &MnListDiff) {
        // Feed base block hash height
        if let Ok(base_height) = self.get_height(&mn_list_diff.base_block_hash) {
            tracing::debug!("feeding {} {}", base_height, mn_list_diff.base_block_hash);
            self.data
                .masternode_list_engine
                .feed_block_height(base_height, mn_list_diff.base_block_hash);
        } else {
            self.ui_state.error = Some(format!(
                "Failed to get height for base block hash: {}",
                mn_list_diff.base_block_hash
            ));
        }

        // Feed block hash height
        if let Ok(block_height) = self.get_height(&mn_list_diff.block_hash) {
            tracing::debug!("feeding {} {}", block_height, mn_list_diff.block_hash);
            self.data
                .masternode_list_engine
                .feed_block_height(block_height, mn_list_diff.block_hash);
        } else {
            self.ui_state.error = Some(format!(
                "Failed to get height for block hash: {}",
                mn_list_diff.block_hash
            ));
        }
    }

    /// **Helper function:** Feeds the quorum hash height of a `QuorumEntry`
    fn feed_quorum_entry_height(&mut self, quorum_entry: &QuorumEntry) {
        if let Ok(height) = self.get_height(&quorum_entry.quorum_hash) {
            self.data
                .masternode_list_engine
                .feed_block_height(height, quorum_entry.quorum_hash);
        } else {
            self.ui_state.error = Some(format!(
                "Failed to get height for quorum hash: {}",
                quorum_entry.quorum_hash
            ));
        }
    }

    fn parse_heights(&mut self) -> Result<(HeightHash, HeightHash), String> {
        let base = if self.input.base_block_height.is_empty() {
            self.input.base_block_height = "0".to_string();
            match self
                .app_context
                .core_client
                .read()
                .unwrap()
                .get_block_hash(0)
            {
                Ok(block_hash) => (0, BlockHash::from_byte_array(block_hash.to_byte_array())),
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        } else {
            match self.input.base_block_height.trim().parse() {
                Ok(start) => match self
                    .app_context
                    .core_client
                    .read()
                    .unwrap()
                    .get_block_hash(start)
                {
                    Ok(block_hash) => (
                        start,
                        BlockHash::from_byte_array(block_hash.to_byte_array()),
                    ),
                    Err(e) => {
                        return Err(e.to_string());
                    }
                },
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        };
        let end = if self.input.end_block_height.is_empty() {
            match self
                .app_context
                .core_client
                .read()
                .unwrap()
                .get_best_block_hash()
            {
                Ok(block_hash) => {
                    match self
                        .app_context
                        .core_client
                        .read()
                        .unwrap()
                        .get_block_header_info(&block_hash)
                    {
                        Ok(header) => {
                            self.input.end_block_height = format!("{}", header.height);
                            (
                                header.height as u32,
                                BlockHash::from_byte_array(block_hash.to_byte_array()),
                            )
                        }
                        Err(e) => {
                            return Err(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        } else {
            match self.input.end_block_height.trim().parse() {
                Ok(end) => match self
                    .app_context
                    .core_client
                    .read()
                    .unwrap()
                    .get_block_hash(end)
                {
                    Ok(block_hash) => (end, BlockHash::from_byte_array(block_hash.to_byte_array())),
                    Err(e) => {
                        return Err(e.to_string());
                    }
                },
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        };
        Ok((base, end))
    }

    fn serialize_masternode_list_engine(&self) -> Result<String, String> {
        match bincode::encode_to_vec(
            &self.data.masternode_list_engine,
            bincode::config::standard(),
        ) {
            Ok(encoded_bytes) => Ok(hex::encode(encoded_bytes)), // Convert to hex string
            Err(e) => Err(format!("Serialization failed: {}", e)),
        }
    }

    fn insert_mn_list_diff(&mut self, mn_list_diff: &MnListDiff) {
        let base_block_hash = mn_list_diff.base_block_hash;
        let base_height = match self.get_height_and_cache(&base_block_hash) {
            Ok(height) => height,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };
        let block_hash = mn_list_diff.block_hash;
        let height = match self.get_height_and_cache(&block_hash) {
            Ok(height) => height,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        self.data
            .mnlist_diffs
            .insert((base_height, height), mn_list_diff.clone());
    }

    fn fetch_rotated_quorum_info(
        &mut self,
        p2p_handler: &mut CoreP2PHandler,
        base_block_hash: BlockHash,
        block_hash: BlockHash,
    ) -> Option<QRInfo> {
        let known_block_hashes = self.known_block_hashes_with_base(base_block_hash);
        tracing::debug!(
            "requesting with known_block_hashes {}",
            known_block_hashes
                .iter()
                .map(|bh| bh.to_string())
                .join(", ")
        );
        let qr_info = match p2p_handler.get_qr_info(known_block_hashes, block_hash) {
            Ok(list_diff) => list_diff,
            Err(e) => {
                self.ui_state.error = Some(e);
                return None;
            }
        };
        self.insert_mn_list_diff(&qr_info.mn_list_diff_tip);
        self.insert_mn_list_diff(&qr_info.mn_list_diff_h);
        self.insert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_c);
        self.insert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_2c);
        self.insert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_3c);
        if let Some((_, mn_list_diff_at_h_minus_4c)) =
            &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c
        {
            self.insert_mn_list_diff(mn_list_diff_at_h_minus_4c);
        }
        for diff in &qr_info.mn_list_diff_list {
            self.insert_mn_list_diff(diff)
        }
        self.data.qr_infos.insert(block_hash, qr_info.clone());
        Some(qr_info)
    }

    fn fetch_diffs_with_hashes(
        &mut self,
        p2p_handler: &mut CoreP2PHandler,
        hashes: BTreeSet<QuorumHash>,
    ) {
        let mut hashes_needed_to_validate = BTreeMap::new();
        for quorum_hash in hashes {
            let height = match self.get_height_and_cache(&quorum_hash) {
                Ok(height) => height,
                Err(e) => {
                    self.ui_state.error = Some(e.to_string());
                    return;
                }
            };
            let validation_hash = match self
                .app_context
                .core_client
                .read()
                .unwrap()
                .get_block_hash(height - 8)
            {
                Ok(block_hash) => block_hash,
                Err(e) => {
                    self.ui_state.error = Some(e.to_string());
                    return;
                }
            };
            hashes_needed_to_validate.insert(
                height - 8,
                BlockHash::from_byte_array(validation_hash.to_byte_array()),
            );
        }

        if let Some((oldest_needed_height, _)) = hashes_needed_to_validate.first_key_value() {
            let (first_engine_height, first_masternode_list) = self
                .data
                .masternode_list_engine
                .masternode_lists
                .first_key_value()
                .unwrap();
            let (mut base_block_height, mut base_block_hash) = if *first_engine_height
                < *oldest_needed_height
            {
                (*first_engine_height, first_masternode_list.block_hash)
            } else {
                let known_genesis_block_hash = match self
                    .data
                    .masternode_list_engine
                    .network
                    .known_genesis_block_hash()
                {
                    None => match self
                        .app_context
                        .core_client
                        .read()
                        .unwrap()
                        .get_block_hash(0)
                    {
                        Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
                        Err(e) => {
                            self.ui_state.error = Some(e.to_string());
                            return;
                        }
                    },
                    Some(known_genesis_block_hash) => known_genesis_block_hash,
                };
                (0, known_genesis_block_hash)
            };

            for (core_block_height, block_hash) in hashes_needed_to_validate {
                self.fetch_single_dml(
                    p2p_handler,
                    base_block_hash,
                    base_block_height,
                    block_hash,
                    core_block_height,
                    false,
                );
                base_block_hash = block_hash;
                base_block_height = core_block_height;
            }
        }
    }

    fn fetch_single_dml(
        &mut self,
        p2p_handler: &mut CoreP2PHandler,
        base_block_hash: BlockHash,
        base_block_height: u32,
        block_hash: BlockHash,
        block_height: u32,
        validate_quorums: bool,
    ) {
        let list_diff = match p2p_handler.get_dml_diff(base_block_hash, block_hash) {
            Ok(list_diff) => list_diff,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        if base_block_height == 0 && self.data.masternode_list_engine.masternode_lists.is_empty() {
            self.data.masternode_list_engine =
                match MasternodeListEngine::initialize_with_diff_to_height(
                    list_diff.clone(),
                    block_height,
                    self.app_context.network,
                ) {
                    Ok(masternode_list_engine) => masternode_list_engine,
                    Err(e) => {
                        self.ui_state.error = Some(e.to_string());
                        return;
                    }
                }
        } else if let Err(e) = self.data.masternode_list_engine.apply_diff(
            list_diff.clone(),
            Some(block_height),
            false,
            None,
        ) {
            self.ui_state.error = Some(e.to_string());
            return;
        }

        if validate_quorums && !self.data.masternode_list_engine.masternode_lists.is_empty() {
            let hashes = self
                .data
                .masternode_list_engine
                .latest_masternode_list_non_rotating_quorum_hashes(
                    &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                    true,
                );
            self.fetch_diffs_with_hashes(p2p_handler, hashes);
            let hashes = self
                .data
                .masternode_list_engine
                .latest_masternode_list_rotating_quorum_hashes(&[]);
            for hash in &hashes {
                let height = match self.get_height_and_cache(hash) {
                    Ok(height) => height,
                    Err(e) => {
                        self.ui_state.error = Some(e.to_string());
                        return;
                    }
                };
                self.cache.block_height_cache.insert(*hash, height);
            }

            if let Err(e) = self
                .data
                .masternode_list_engine
                .verify_non_rotating_masternode_list_quorums(
                    block_height,
                    &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                )
            {
                self.ui_state.error = Some(e.to_string());
            }
        }

        self.data
            .mnlist_diffs
            .insert((base_block_height, block_height), list_diff);
    }

    // fn fetch_range_dml(&mut self, step: u32, include_at_minus_8: bool, count: u32) {
    //     let ((base_block_height, base_block_hash), (block_height, block_hash)) =
    //         match self.parse_heights() {
    //             Ok(a) => a,
    //             Err(e) => {
    //                 self.ui_state.error = Some(e);
    //                 return;
    //             }
    //         };
    //
    //     let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
    //         Ok(p2p_handler) => p2p_handler,
    //         Err(e) => {
    //             self.ui_state.error = Some(e);
    //             return;
    //         }
    //     };
    //
    //     let rem = block_height % 24;
    //
    //     let intermediate_block_height = (block_height - rem).saturating_sub(count * step);
    //
    //     let intermediate_block_hash = match self
    //         .app_context
    //         .core_client
    //         .get_block_hash(intermediate_block_height)
    //     {
    //         Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //         Err(e) => {
    //             self.ui_state.error = Some(e.to_string());
    //             return;
    //         }
    //     };
    //
    //     self.fetch_single_dml(
    //         &mut p2p_handler,
    //         base_block_hash,
    //         base_block_height,
    //         intermediate_block_hash,
    //         intermediate_block_height,
    //         false,
    //     );
    //
    //     let mut last_height = intermediate_block_height;
    //     let mut last_block_hash = intermediate_block_hash;
    //
    //     for _i in 0..count {
    //         if include_at_minus_8 {
    //             let end_height = last_height + step - 8;
    //             let end_block_hash = match self.app_context.core_client.read().unwrap().get_block_hash(end_height) {
    //                 Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //                 Err(e) => {
    //                     self.ui_state.error = Some(e.to_string());
    //                     return;
    //                 }
    //             };
    //             self.fetch_single_dml(
    //                 &mut p2p_handler,
    //                 last_block_hash,
    //                 last_height,
    //                 end_block_hash,
    //                 end_height,
    //             );
    //             last_height = end_height;
    //             last_block_hash = end_block_hash;
    //
    //             let end_height = last_height + 8;
    //             let end_block_hash = match self.app_context.core_client.read().unwrap().get_block_hash(end_height) {
    //                 Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //                 Err(e) => {
    //                     self.ui_state.error = Some(e.to_string());
    //                     return;
    //                 }
    //             };
    //             self.fetch_single_dml(
    //                 &mut p2p_handler,
    //                 last_block_hash,
    //                 last_height,
    //                 end_block_hash,
    //                 end_height,
    //             );
    //             last_height = end_height;
    //             last_block_hash = end_block_hash;
    //         } else {
    //             let end_height = last_height + step;
    //             let end_block_hash = match self.app_context.core_client.read().unwrap().get_block_hash(end_height) {
    //                 Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //                 Err(e) => {
    //                     self.ui_state.error = Some(e.to_string());
    //                     return;
    //                 }
    //             };
    //             self.fetch_single_dml(
    //                 &mut p2p_handler,
    //                 last_block_hash,
    //                 last_height,
    //                 end_block_hash,
    //                 end_height,
    //             );
    //             last_height = end_height;
    //             last_block_hash = end_block_hash;
    //         }
    //     }
    //
    //     if rem != 0 {
    //         let end_height = last_height + rem;
    //         let end_block_hash = match self.app_context.core_client.read().unwrap().get_block_hash(end_height) {
    //             Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //             Err(e) => {
    //                 self.ui_state.error = Some(e.to_string());
    //                 return;
    //             }
    //         };
    //         self.fetch_single_dml(
    //             &mut p2p_handler,
    //             last_block_hash,
    //             last_height,
    //             end_block_hash,
    //             end_height,
    //         );
    //     }
    //
    //     // Reset selections when new data is loaded
    //     self.selection.selected_dml_diff_key = None;
    //     self.selection.selected_quorum_in_diff_index = None;
    // }

    /// Clear all data and reset to initial state
    pub(crate) fn clear(&mut self) {
        self.data.masternode_list_engine =
            MasternodeListEngine::default_for_network(self.app_context.network);

        // Clear cached data structures
        self.data.mnlist_diffs.clear();
        self.data.qr_infos.clear();
        self.incoming.chain_locked_blocks.clear();
        self.incoming.instant_send_transactions.clear();
        self.cache.block_height_cache.clear();
        self.cache.block_hash_cache.clear();
        self.cache.masternode_list_quorum_hash_cache.clear();
        self.cache
            .masternode_lists_with_all_quorum_heights_known
            .clear();
        self.cache.dml_diffs_with_cached_quorum_heights.clear();
        self.cache.chain_lock_sig_cache.clear();
        self.cache.chain_lock_reversed_sig_cache.clear();

        // Reset selections and UI state
        self.selection.selected_dml_diff_key = None;
        self.selection.selected_dml_height_key = None;
        self.selection.selected_option_index = None;
        self.selection.selected_quorum_in_diff_index = None;
        self.selection.selected_masternode_in_diff_index = None;
        self.selection.selected_quorum_hash_in_mnlist_diff = None;
        self.selection.selected_masternode_pro_tx_hash = None;
        self.selection.selected_qr_item = None;
        self.selection.selected_core_item = None;
        self.task.pending = None;
        self.task.queued_task = None;
        self.input.search_term = None;
        self.ui_state.error = None;
    }

    /// Clear all data except the oldest MNList diff starting from height 0
    fn clear_keep_base(&mut self) {
        let (engine, start_end_diff) =
            if let Some(((start, end), oldest_diff)) = self.data.mnlist_diffs.first_key_value() {
                if start == &0 {
                    MasternodeListEngine::initialize_with_diff_to_height(
                        oldest_diff.clone(),
                        *end,
                        self.app_context.network,
                    )
                    .map(|engine| (engine, Some(((*start, *end), oldest_diff.clone()))))
                    .unwrap_or((
                        MasternodeListEngine::default_for_network(self.app_context.network),
                        None,
                    ))
                } else {
                    (
                        MasternodeListEngine::default_for_network(self.app_context.network),
                        None,
                    )
                }
            } else {
                (
                    MasternodeListEngine::default_for_network(self.app_context.network),
                    None,
                )
            };

        self.data.masternode_list_engine = engine;
        self.data.mnlist_diffs = Default::default();
        if let Some((key, oldest_diff)) = start_end_diff {
            self.data.mnlist_diffs.insert(key, oldest_diff);
        }
        self.selection.selected_dml_diff_key = None;
        self.selection.selected_dml_height_key = None;
        self.selection.selected_option_index = None;
        self.selection.selected_quorum_in_diff_index = None;
        self.selection.selected_masternode_in_diff_index = None;
        self.selection.selected_quorum_hash_in_mnlist_diff = None;
        self.selection.selected_masternode_pro_tx_hash = None;
        self.data.qr_infos = Default::default();
        // Clear chain lock signatures caches as these are independent of the retained base diff
        self.cache.chain_lock_sig_cache.clear();
        self.cache.chain_lock_reversed_sig_cache.clear();
    }

    /// Fetch the MNList diffs between the given base and end block heights.
    /// In a real implementation, you would replace the dummy function below with a call to
    /// dash_core’s DB (or other data source) to retrieve the MNList diffs.
    #[allow(dead_code)]
    fn fetch_end_dml_diff(&mut self, validate_quorums: bool) {
        let ((base_block_height, base_block_hash), (block_height, block_hash)) =
            match self.parse_heights() {
                Ok(a) => a,
                Err(e) => {
                    self.ui_state.error = Some(e);
                    return;
                }
            };

        let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
            Ok(p2p_handler) => p2p_handler,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        self.fetch_single_dml(
            &mut p2p_handler,
            base_block_hash,
            base_block_height,
            block_hash,
            block_height,
            validate_quorums,
        );

        // Reset selections when new data is loaded
        self.selection.selected_dml_diff_key = None;
        self.selection.selected_quorum_in_diff_index = None;
    }

    #[allow(dead_code)]
    fn fetch_end_qr_info(&mut self) {
        let ((_, base_block_hash), (_, block_hash)) = match self.parse_heights() {
            Ok(a) => a,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
            Ok(p2p_handler) => p2p_handler,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        self.fetch_rotated_quorum_info(&mut p2p_handler, base_block_hash, block_hash);

        // Reset selections when new data is loaded
        self.selection.selected_dml_diff_key = None;
        self.selection.selected_quorum_in_diff_index = None;
    }

    #[allow(dead_code)]
    fn fetch_chain_locks(&mut self) {
        let ((base_block_height, _base_block_hash), (block_height, _block_hash)) =
            match self.parse_heights() {
                Ok(a) => a,
                Err(e) => {
                    self.ui_state.error = Some(e);
                    return;
                }
            };

        let max_blocks = 2000;

        let loaded_list_height = match self.app_context.network {
            Network::Dash => 2227096,
            Network::Testnet => 1296600,
            _ => 0,
        };

        let start_height = if base_block_height < loaded_list_height {
            block_height - max_blocks
        } else {
            base_block_height
        };

        let end_height = std::cmp::min(start_height + max_blocks, block_height);

        for i in start_height..end_height {
            if let Ok(block_hash) = self.get_block_hash_and_cache(i) {
                self.get_chain_lock_sig_and_cache(&block_hash).ok();
            }
        }
    }

    #[allow(dead_code)]
    fn sync(&mut self) {
        if !self.task.syncing {
            self.task.syncing = true;
            self.fetch_end_qr_info_with_dmls();
        }
    }

    #[allow(dead_code)]
    fn fetch_end_qr_info_with_dmls(&mut self) {
        let ((_, base_block_hash), (_, block_hash)) = match self.parse_heights() {
            Ok(a) => a,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
            Ok(p2p_handler) => p2p_handler,
            Err(e) => {
                self.ui_state.error = Some(e);
                return;
            }
        };

        let Some(qr_info) =
            self.fetch_rotated_quorum_info(&mut p2p_handler, base_block_hash, block_hash)
        else {
            return;
        };

        self.feed_qr_info_and_get_dmls(qr_info, Some(p2p_handler))
    }

    fn feed_qr_info_and_get_dmls(
        &mut self,
        qr_info: QRInfo,
        core_p2phandler: Option<CoreP2PHandler>,
    ) {
        let mut p2p_handler = match core_p2phandler {
            None => match CoreP2PHandler::new(self.app_context.network, None) {
                Ok(p2p_handler) => p2p_handler,
                Err(e) => {
                    self.ui_state.error = Some(e);
                    return;
                }
            },
            Some(core_p2phandler) => core_p2phandler,
        };

        // Extracting immutable references before calling `feed_qr_info`
        let get_height_fn = {
            let block_height_cache = &self.cache.block_height_cache;
            let app_context = &self.app_context;

            move |block_hash: &BlockHash| {
                if block_hash.as_byte_array() == &[0; 32] {
                    return Ok(0);
                }
                if let Some(height) = block_height_cache.get(block_hash) {
                    return Ok(*height);
                }
                match app_context
                    .core_client
                    .read()
                    .unwrap()
                    .get_block_header_info(
                        &(BlockHash2::from_byte_array(block_hash.to_byte_array())),
                    ) {
                    Ok(block_info) => Ok(block_info.height as CoreBlockHeight),
                    Err(_) => Err(ClientDataRetrievalError::RequiredBlockNotPresent(
                        *block_hash,
                    )),
                }
            }
        };

        if let Err(e) =
            self.data
                .masternode_list_engine
                .feed_qr_info(qr_info, false, true, Some(get_height_fn))
        {
            self.ui_state.error = Some(e.to_string());
            return;
        }

        let hashes = self
            .data
            .masternode_list_engine
            .latest_masternode_list_non_rotating_quorum_hashes(
                &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                true,
            );
        self.fetch_diffs_with_hashes(&mut p2p_handler, hashes);
        let hashes = self
            .data
            .masternode_list_engine
            .latest_masternode_list_rotating_quorum_hashes(&[]);
        for hash in &hashes {
            let height = match self.get_height_and_cache(hash) {
                Ok(height) => height,
                Err(e) => {
                    self.ui_state.error = Some(e.to_string());
                    return;
                }
            };
            self.cache.block_height_cache.insert(*hash, height);
        }

        if let Some(latest_masternode_list) =
            self.data.masternode_list_engine.latest_masternode_list()
            && let Err(e) = self
                .data
                .masternode_list_engine
                .verify_non_rotating_masternode_list_quorums(
                    latest_masternode_list.known_height,
                    &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                )
        {
            self.ui_state.error = Some(e.to_string());
        }

        // Reset selections when new data is loaded
        self.selection.selected_dml_diff_key = None;
        self.selection.selected_quorum_in_diff_index = None;
    }

    /// Render the input area at the top (base and end block height fields plus Get DMLs button)
    fn render_input_area(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ScrollArea::horizontal()
            .id_salt("dml_input_row_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Base Block Height:");
                    ui.add(
                        TextEdit::singleline(&mut self.input.base_block_height).desired_width(80.0),
                    );
                    ui.label("End Block Height:");
                    ui.add(
                        TextEdit::singleline(&mut self.input.end_block_height).desired_width(80.0),
                    );
                    if ui.button("Get single end DML diff").clicked()
                        && let Ok(((base_h, base_hash), (h, hash))) = self.parse_heights()
                    {
                        self.task.pending = Some(PendingTask::DmlDiffSingle);
                        action = AppAction::BackendTask(BackendTask::MnListTask(
                            MnListTask::FetchEndDmlDiff {
                                base_block_height: base_h,
                                base_block_hash: base_hash,
                                block_height: h,
                                block_hash: hash,
                                validate_quorums: false,
                            },
                        ));
                    }
                    if ui.button("Get single end QR info").clicked()
                        && let Ok(((_, base_hash), (_, hash))) = self.parse_heights()
                    {
                        self.task.pending = Some(PendingTask::QrInfo);
                        // Build known_block_hashes from current diffs + base hash (old UI behavior)
                        let known_block_hashes = self.known_block_hashes_with_base(base_hash);
                        action = AppAction::BackendTask(BackendTask::MnListTask(
                            MnListTask::FetchEndQrInfo {
                                known_block_hashes,
                                block_hash: hash,
                            },
                        ));
                    }
                    if ui.button("Get DMLs w/o rotation").clicked()
                        && let Ok(((base_h, base_hash), (h, hash))) = self.parse_heights()
                    {
                        self.task.pending = Some(PendingTask::DmlDiffNoRotation);
                        action = AppAction::BackendTask(BackendTask::MnListTask(
                            MnListTask::FetchEndDmlDiff {
                                base_block_height: base_h,
                                base_block_hash: base_hash,
                                block_height: h,
                                block_hash: hash,
                                validate_quorums: true,
                            },
                        ));
                    }
                    if ui.button("Get DMLs w/ rotation").clicked()
                        && let Ok(((_, base_hash), (_, hash))) = self.parse_heights()
                    {
                        self.task.pending = Some(PendingTask::QrInfoWithDmls);
                        // Build known_block_hashes from current diffs + base hash (old UI behavior)
                        let known_block_hashes = self.known_block_hashes_with_base(base_hash);
                        action = AppAction::BackendTask(BackendTask::MnListTask(
                            MnListTask::FetchEndQrInfoWithDmls {
                                known_block_hashes,
                                block_hash: hash,
                            },
                        ));
                    }
                    if ui.button("Sync").clicked()
                        && let Ok(((_, base_hash), (_, hash))) = self.parse_heights()
                    {
                        self.task.pending = Some(PendingTask::QrInfoWithDmls);
                        // Build known_block_hashes from current diffs + base hash (old UI behavior)
                        let known_block_hashes = self.known_block_hashes_with_base(base_hash);
                        action = AppAction::BackendTask(BackendTask::MnListTask(
                            MnListTask::FetchEndQrInfoWithDmls {
                                known_block_hashes,
                                block_hash: hash,
                            },
                        ));
                    }
                    if ui.button("Get chain locks").clicked()
                        && let Ok(((base_h, _), (h, _))) = self.parse_heights()
                    {
                        self.task.pending = Some(PendingTask::ChainLocks);
                        action = AppAction::BackendTask(BackendTask::MnListTask(
                            MnListTask::FetchChainLocks {
                                base_block_height: base_h,
                                block_height: h,
                            },
                        ));
                    }
                    if ui
                        .button("Clear")
                        .on_hover_text("Clear all data and reset to initial state.")
                        .clicked()
                    {
                        self.clear();
                        self.display_message("Cleared all data", MessageType::Success);
                    }
                    if ui
                        .button("Clear keep base")
                        .on_hover_text(
                            "Clear all data except the oldest MNList diff starting from height 0.",
                        )
                        .clicked()
                    {
                        self.clear_keep_base();
                        self.display_message(
                            "Cleared data and kept base diff",
                            MessageType::Success,
                        );
                    }
                });
                // Add bottom padding so the horizontal scrollbar doesn't overlap buttons
                ui.add_space(12.0);
            });
        action
    }

    fn render_error_banner(&mut self, ui: &mut Ui) {
        let Some(error_msg) = self.ui_state.error.clone() else {
            return;
        };

        let message_color = DashColors::ERROR;
        ui.horizontal(|ui| {
            Frame::new()
                .fill(message_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, message_color))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(error_msg).color(message_color));
                        ui.add_space(10.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.ui_state.error = None;
                        }
                    });
                });
        });
        ui.add_space(10.0);
    }

    fn render_pending_status(&self, ui: &mut Ui) {
        let Some(pending) = self.task.pending else {
            return;
        };

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                let style = ui.style_mut();
                // Force spinner (fg stroke) to Dash Blue
                style.visuals.widgets.inactive.fg_stroke.color = DashColors::DASH_BLUE;
                style.visuals.widgets.active.fg_stroke.color = DashColors::DASH_BLUE;
                style.visuals.widgets.hovered.fg_stroke.color = DashColors::DASH_BLUE;
                ui.add(egui::Spinner::new());
            });
            let label = match pending {
                PendingTask::DmlDiffSingle => "Fetching DML diff…",
                PendingTask::DmlDiffNoRotation => "Fetching DMLs (no rotation)…",
                PendingTask::QrInfo => "Fetching QR info…",
                PendingTask::QrInfoWithDmls => "Fetching QR info + DMLs…",
                PendingTask::ChainLocks => "Fetching chain locks…",
            };
            let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
            ui.colored_label(text_primary, label);
        });
        ui.add_space(6.0);
    }

    fn load_masternode_list_engine(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Binary", &["dat"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    match bincode::decode_from_slice::<MasternodeListEngine, _>(
                        &bytes,
                        bincode::config::standard(),
                    ) {
                        Ok((engine, _)) => {
                            self.data.masternode_list_engine = engine;
                        }
                        Err(e) => {
                            tracing::error!("Failed to decode QRInfo: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to read file: {:?}", e);
                }
            }
        }
    }

    fn save_masternode_list_engine(&mut self) {
        // Serialize the masternode list engine
        let serialized = match self.serialize_masternode_list_engine() {
            Ok(serialized) => serialized,
            Err(e) => {
                self.ui_state.error = Some(format!("Serialization failed: {}", e));
                return;
            }
        };

        // Open a file save dialog
        if let Some(path) = FileDialog::new()
            .set_title("Save Masternode List Engine")
            .add_filter("JSON", &["hex"])
            .add_filter("Binary", &["bin"])
            .set_file_name("masternode_list_engine.hex")
            .save_file()
        {
            // Attempt to write the serialized data to the selected file
            match fs::write(&path, serialized) {
                Ok(_) => {
                    tracing::info!("Masternode list engine saved to {:?}", path);
                }
                Err(e) => {
                    self.ui_state.error = Some(format!("Failed to save file: {}", e));
                }
            }
        }
    }

    fn render_masternode_lists(&mut self, ui: &mut Ui) {
        ui.heading("Masternode lists");
        ScrollArea::vertical()
            .id_salt("dml_list_scroll_area")
            .show(ui, |ui| {
                for height in self.data.masternode_list_engine.masternode_lists.keys() {
                    let height_label = format!("{}", height);

                    if ui
                        .selectable_label(
                            self.selection.selected_dml_height_key == Some(*height),
                            height_label,
                        )
                        .clicked()
                    {
                        self.selection.selected_dml_diff_key = None;
                        self.selection.selected_dml_height_key = Some(*height);
                        self.selection.selected_quorum_in_diff_index = None;
                    }
                }
            });
    }

    /// Render MNList diffs list (block heights)
    fn render_diff_list(&mut self, ui: &mut Ui) {
        ui.heading("MNList Diffs");
        ScrollArea::vertical()
            .id_salt("dml_list_scroll_area")
            .show(ui, |ui| {
                for (key, _dml) in self.data.mnlist_diffs.iter() {
                    let block_label = format!("Base: {} -> Block: {}", key.0, key.1);

                    if ui
                        .selectable_label(
                            self.selection.selected_dml_diff_key == Some(*key),
                            block_label,
                        )
                        .clicked()
                    {
                        self.selection.selected_dml_diff_key = Some(*key);
                        self.selection.selected_dml_height_key = None;
                        self.selection.selected_quorum_in_diff_index = None;
                    }
                }
            });
    }

    /// Render the list of quorums for the selected DML
    fn render_new_quorums(&mut self, ui: &mut Ui) {
        ui.heading("New Quorums");

        let Some(selected_key) = self.selection.selected_dml_diff_key else {
            ui.label("Select a block height to show quorums.");
            return;
        };

        let Some(dml) = self.data.mnlist_diffs.get(&selected_key) else {
            ui.label("Select a block height to show quorums.");
            return;
        };

        let should_get_heights = !self
            .cache
            .dml_diffs_with_cached_quorum_heights
            .contains(&selected_key);
        let new_quorums = dml.new_quorums.clone();
        let mut heights: HashMap<QuorumHash, CoreBlockHeight> = HashMap::new();
        for quorum in &new_quorums {
            let height = if should_get_heights {
                self.get_height_and_cache(&quorum.quorum_hash)
            } else {
                self.get_height(&quorum.quorum_hash)
            }
            .ok()
            .unwrap_or_default();
            heights.insert(quorum.quorum_hash, height);
        }

        ScrollArea::vertical()
            .id_salt("quorum_list_scroll_area")
            .show(ui, |ui| {
                for (q_index, quorum) in new_quorums.iter().enumerate() {
                    let quorum_height = heights
                        .get(&quorum.quorum_hash)
                        .copied()
                        .unwrap_or_default();
                    if ui
                        .selectable_label(
                            self.selection.selected_quorum_in_diff_index == Some(q_index),
                            format!(
                                "Quorum height {} [..]{}{} Type: {}",
                                quorum_height,
                                quorum.quorum_hash.to_string().as_str().split_at(58).1,
                                quorum
                                    .quorum_index
                                    .map(|i| format!(" (index {})", i))
                                    .unwrap_or_default(),
                                QuorumType::from(quorum.llmq_type as u32)
                            ),
                        )
                        .clicked()
                    {
                        self.selection.selected_quorum_in_diff_index = Some(q_index);
                        self.selection.selected_masternode_in_diff_index = None;
                    }
                }
            });
    }

    fn render_selected_masternode_list_items(&mut self, ui: &mut Ui) {
        ui.heading("Masternode List Explorer");

        // Define available options for selection
        let options = ["Quorums", "Masternodes"];
        let selected_index = self.selection.selected_option_index.unwrap_or(0);

        // Render the selection buttons
        ui.horizontal(|ui| {
            for (index, option) in options.iter().enumerate() {
                if ui
                    .selectable_label(selected_index == index, *option)
                    .clicked()
                {
                    self.selection.selected_option_index = Some(index);
                }
            }
        });

        ui.separator();

        // Borrow mn_list separately to avoid multiple borrows of `self`
        if self.selection.selected_dml_height_key.is_some() {
            ScrollArea::vertical()
                .id_salt("mnlist_items_scroll_area")
                .show(ui, |ui| match selected_index {
                    0 => self.render_quorums_in_masternode_list(ui),
                    1 => self.render_masternodes_in_masternode_list(ui),
                    _ => (),
                });
        } else {
            ui.label("Select a block height to show details.");
        }
    }

    fn render_quorums_in_masternode_list(&mut self, ui: &mut Ui) {
        let mut heights: BTreeMap<QuorumHash, CoreBlockHeight> = BTreeMap::new();
        let mut masternode_block_hash = None;
        if let Some(selected_height) = self.selection.selected_dml_height_key {
            if !self
                .cache
                .masternode_lists_with_all_quorum_heights_known
                .contains(&selected_height)
            {
                if let Some(quorum_hashes) = self
                    .data
                    .masternode_list_engine
                    .masternode_lists
                    .get(&selected_height)
                    .map(|list| {
                        list.quorums
                            .values()
                            .flat_map(|quorums| quorums.keys())
                            .copied()
                            .collect::<BTreeSet<_>>()
                    })
                {
                    for quorum_hash in quorum_hashes.iter() {
                        if let Ok(height) = self.get_height_and_cache(quorum_hash) {
                            heights.insert(*quorum_hash, height);
                        }
                    }
                }
                self.cache
                    .masternode_lists_with_all_quorum_heights_known
                    .insert(selected_height);
            }
            if let Some(mn_list) = self
                .data
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
            {
                masternode_block_hash = Some(mn_list.block_hash);
                for (llmq_type, quorum_map) in &mn_list.quorums {
                    if llmq_type == &LLMQType::Llmqtype50_60
                        || llmq_type == &LLMQType::Llmqtype400_85
                    {
                        continue;
                    }
                    for quorum_hash in quorum_map.keys() {
                        if let Ok(height) = self.get_height(quorum_hash) {
                            heights.insert(*quorum_hash, height);
                        }
                    }
                }
                self.cache
                    .masternode_list_quorum_hash_cache
                    .entry(mn_list.block_hash)
                    .or_insert_with(|| {
                        let mut btree_map = BTreeMap::new();
                        for (llmq_type, quorum_map) in &mn_list.quorums {
                            let quorums_by_height = quorum_map
                                .iter()
                                .map(|(quorum_hash, quorum_entry)| {
                                    (
                                        heights.get(quorum_hash).copied().unwrap_or_default(),
                                        quorum_entry.clone(),
                                    )
                                })
                                .collect();
                            btree_map.insert(*llmq_type, quorums_by_height);
                        }
                        btree_map
                    });
            }
        }
        if let Some(quorums) = masternode_block_hash.and_then(|block_hash| {
            self.cache
                .masternode_list_quorum_hash_cache
                .get(&block_hash)
        }) {
            ui.heading("Quorums in Masternode List");
            ui.label("(excluding 50_60 and 400_85)");
            ScrollArea::vertical()
                .id_salt("quorum_list_scroll_area")
                .show(ui, |ui| {
                    for (llmq_type, quorum_map) in quorums {
                        if llmq_type == &LLMQType::Llmqtype50_60
                            || llmq_type == &LLMQType::Llmqtype400_85
                        {
                            continue;
                        }
                        for (quorum_height, quorum_entry) in quorum_map.iter() {
                            if ui
                                .selectable_label(
                                    self.selection.selected_quorum_hash_in_mnlist_diff
                                        == Some((
                                            *llmq_type,
                                            quorum_entry.quorum_entry.quorum_hash,
                                        )),
                                    format!(
                                        "Quorum {} Type: {} Valid {}",
                                        quorum_height,
                                        QuorumType::from(*llmq_type as u32),
                                        quorum_entry.verified
                                            == LLMQEntryVerificationStatus::Verified
                                    ),
                                )
                                .clicked()
                            {
                                self.selection.selected_quorum_hash_in_mnlist_diff =
                                    Some((*llmq_type, quorum_entry.quorum_entry.quorum_hash));
                                self.selection.selected_masternode_pro_tx_hash = None;
                                self.selection.selected_dml_diff_key = None;
                            }
                        }
                    }
                });
        }
    }

    /// Filter masternodes based on the search term
    fn filter_masternodes(
        &self,
        mn_list: &MasternodeList,
    ) -> BTreeMap<ProTxHash, QualifiedMasternodeListEntry> {
        // If no search term, return all masternodes
        if let Some(search_term) = &self.input.search_term {
            let search_term = search_term.to_lowercase();

            if search_term.len() < 3 {
                return mn_list.masternodes.clone(); // Require at least 3 characters to filter
            }

            mn_list
                .masternodes
                .iter()
                .filter(|(pro_tx_hash, mn_entry)| {
                    let masternode = &mn_entry.masternode_list_entry;

                    // Convert fields to lowercase for case-insensitive search
                    let pro_tx_hash_str = pro_tx_hash.to_string().to_lowercase();
                    let confirmed_hash_str = masternode
                        .confirmed_hash
                        .map(|h| h.to_string().to_lowercase())
                        .unwrap_or_default();
                    let service_ip = masternode.service_address.ip().to_string().to_lowercase();
                    let operator_public_key =
                        masternode.operator_public_key.to_string().to_lowercase();
                    let voting_key_id = masternode.key_id_voting.to_string().to_lowercase();

                    // Check reversed versions
                    let pro_tx_hash_reversed = pro_tx_hash.reverse().to_string().to_lowercase();
                    let confirmed_hash_reversed = masternode
                        .confirmed_hash
                        .map(|h| h.reverse().to_string().to_lowercase())
                        .unwrap_or_default();

                    // Match against search term
                    pro_tx_hash_str.contains(&search_term)
                        || confirmed_hash_str.contains(&search_term)
                        || service_ip.contains(&search_term)
                        || operator_public_key.contains(&search_term)
                        || voting_key_id.contains(&search_term)
                        || pro_tx_hash_reversed.contains(&search_term)
                        || confirmed_hash_reversed.contains(&search_term)
                })
                .map(|(pro_tx_hash, entry)| (*pro_tx_hash, entry.clone()))
                .collect()
        } else {
            mn_list.masternodes.clone()
        }
    }

    /// Render search bar
    fn render_search_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            let mut search_term = self.input.search_term.clone().unwrap_or_default();
            let response = ui.add(TextEdit::singleline(&mut search_term).desired_width(200.0));

            if response.changed() {
                self.input.search_term = if search_term.trim().is_empty() {
                    None
                } else {
                    Some(search_term)
                };
            }
        });
    }

    fn render_masternodes_in_masternode_list(&mut self, ui: &mut Ui) {
        if self.selected_mn_list().is_some() {
            ui.heading("Masternodes in List");
            self.render_search_bar(ui);
        }
        let Some(mn_list) = self.selected_mn_list() else {
            return;
        };

        let filtered_masternodes = self.filter_masternodes(mn_list);
        ScrollArea::vertical()
            .id_salt("masternode_list_scroll_area")
            .show(ui, |ui| {
                for (pro_tx_hash, masternode) in filtered_masternodes.iter() {
                    if ui
                        .selectable_label(
                            self.selection.selected_masternode_pro_tx_hash == Some(*pro_tx_hash),
                            format!(
                                "{} {} {}",
                                if masternode.masternode_list_entry.mn_type
                                    == EntryMasternodeType::Regular
                                {
                                    "MN"
                                } else {
                                    "EN"
                                },
                                masternode.masternode_list_entry.service_address.ip(),
                                pro_tx_hash.to_string().as_str().split_at(5).0
                            ),
                        )
                        .clicked()
                    {
                        self.selection.selected_quorum_hash_in_mnlist_diff = None;
                        self.selection.selected_masternode_pro_tx_hash = Some(*pro_tx_hash);
                    }
                }
            });
    }

    fn render_masternode_list_page(&mut self, ui: &mut Ui) {
        // Use a left-to-right layout that fills the available height so columns can expand fully
        let full_w = ui.available_width();
        let full_h = ui.available_height();
        ui.allocate_ui_with_layout(
            egui::Vec2::new(full_w, full_h),
            Layout::left_to_right(Align::Min),
            |ui| {
                // Left column (Fixed width: 120px)
                ui.allocate_ui_with_layout(
                    egui::Vec2::new(120.0, ui.available_height()),
                    Layout::top_down(Align::Min),
                    |ui| {
                        self.render_masternode_lists(ui);
                    },
                );

                ui.separator();

                // Middle column (40% of the remaining space)
                let mid_w = ui.available_width() * 0.4;
                ui.allocate_ui_with_layout(
                    egui::Vec2::new(mid_w, ui.available_height()),
                    Layout::top_down(Align::Min),
                    |ui| {
                        self.render_selected_masternode_list_items(ui);
                    },
                );

                // Right column (Remaining space)
                ui.allocate_ui_with_layout(
                    egui::Vec2::new(ui.available_width(), ui.available_height()),
                    Layout::top_down(Align::Min),
                    |ui| {
                        if self.selection.selected_quorum_hash_in_mnlist_diff.is_some() {
                            self.render_quorum_details(ui);
                        } else if self.selection.selected_masternode_pro_tx_hash.is_some() {
                            self.render_mn_details(ui);
                        }
                    },
                );
            },
        );
    }

    fn render_selected_tab(&mut self, ui: &mut Ui) {
        // Define available tabs
        let mut tabs = vec![
            "Masternode Lists",
            "Quorums",
            "Diffs",
            "QRInfo",
            "Known Blocks",
            "Known Chain Lock Sigs",
            "Core Items",
            "Save Masternode List Engine",
            "Load Masternode List Engine",
        ];

        if self.task.syncing {
            tabs.push("Stop Syncing");
        }

        // Render the selection buttons (scrollable horizontally) styled as buttons
        ScrollArea::horizontal()
            .id_salt("dml_tabs_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, tab) in tabs.iter().enumerate() {
                        let is_selected = self.ui_state.selected_tab == index;
                        if is_selected {
                            // Match the selected look used under "Masternode List Explorer"
                            let _ = ui.selectable_label(true, *tab);
                        } else if ui.button(*tab).clicked() {
                            match index {
                                7 => {
                                    // Show the popup when "Masternode List Engine" is selected
                                    self.ui_state.show_popup_for_render_masternode_list_engine =
                                        true;
                                }
                                8 => {
                                    self.load_masternode_list_engine();
                                }
                                9 => {
                                    self.task.syncing = false;
                                }
                                index => self.ui_state.selected_tab = index,
                            }
                        }
                    }
                });
                // Add bottom padding so the horizontal scrollbar doesn't overlap tabs
                ui.add_space(12.0);
            });

        ui.separator();

        // Scroll only the content below the tab row; for the Masternode Lists page,
        // let its own columns manage scrolling independently.
        if self.ui_state.selected_tab == 0 {
            // Make the Masternode Lists section occupy remaining height
            let full_w = ui.available_width();
            let full_h = ui.available_height();
            ui.allocate_ui_with_layout(
                egui::Vec2::new(full_w, full_h),
                Layout::top_down(Align::Min),
                |ui| {
                    self.render_masternode_list_page(ui);
                },
            );
        } else {
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .id_salt("dml_tab_content_scroll")
                .show(ui, |ui| match self.ui_state.selected_tab {
                    1 => self.render_quorums(ui),
                    2 => self.render_diffs(ui),
                    3 => self.render_qr_info(ui),
                    4 => self.render_engine_known_blocks(ui),
                    5 => self.render_known_chain_lock_sigs(ui),
                    6 => self.render_core_items(ui),
                    _ => {}
                });
        }

        // Render the confirmation popup if needed
        if self.ui_state.show_popup_for_render_masternode_list_engine {
            egui::Window::new("Confirmation")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label("This operation will take about 10 seconds. Are you sure you wish to continue?");

                    ui.horizontal(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let yes_btn = egui::Button::new(
                            egui::RichText::new("Yes")
                                .strong()
                                .color(ComponentStyles::primary_button_text()),
                        )
                        .fill(ComponentStyles::primary_button_fill())
                        .stroke(ComponentStyles::primary_button_stroke())
                        .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                        .min_size(ComponentStyles::DIALOG_BUTTON_MIN_SIZE);
                        if ui
                            .add(yes_btn)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.save_masternode_list_engine();
                            self.ui_state.show_popup_for_render_masternode_list_engine = false;
                        }
                        let cancel_btn = egui::Button::new(
                            egui::RichText::new("Cancel")
                                .strong()
                                .color(ComponentStyles::secondary_button_text(dark_mode)),
                        )
                        .fill(ComponentStyles::secondary_button_fill(dark_mode))
                        .stroke(ComponentStyles::secondary_button_stroke(dark_mode))
                        .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                        .min_size(ComponentStyles::DIALOG_BUTTON_MIN_SIZE);
                        if ui
                            .add(cancel_btn)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.ui_state.show_popup_for_render_masternode_list_engine = false;
                        }
                    });
                });
        }
    }

    fn render_known_chain_lock_sigs(&mut self, ui: &mut Ui) {
        ui.heading("Known Chain Lock Sigs");

        ScrollArea::vertical()
            .id_salt("known_chain_lock_sigs_scroll")
            .show(ui, |ui| {
                egui::Grid::new("known_chain_lock_sigs_grid")
                    .num_columns(3) // Two columns: Block Height | Block Hash | Sig
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Block Height");
                        ui.label("Block Hash");
                        ui.label("Chain Lock Sig");
                        ui.end_row();

                        for ((height, block_hash), sig) in &self.cache.chain_lock_sig_cache {
                            ui.label(format!("{}", height));
                            ui.label(format!("{}", block_hash));
                            if let Some(sig) = sig {
                                ui.label(format!("{}", sig));
                            } else {
                                ui.label("None");
                            }

                            ui.end_row();
                        }
                    });
            });
    }

    fn render_engine_known_blocks(&mut self, ui: &mut Ui) {
        ui.heading("Known Blocks in Masternode List Engine");

        // Add Save/Load functionality
        ui.horizontal(|ui| {
            if ui.button("Save Block Container").clicked() {
                // Open native save dialog
                if let Some(path) = FileDialog::new()
                    .set_file_name("block_container.dat")
                    .add_filter("Data Files", &["dat"])
                    .save_file()
                {
                    // Serialize and save the block container
                    let serialized_data = bincode::encode_to_vec(
                        &self.data.masternode_list_engine.block_container,
                        bincode::config::standard(),
                    )
                    .expect("serialize container");
                    if let Err(e) = std::fs::write(&path, serialized_data) {
                        tracing::error!("Failed to write file: {}", e);
                    }
                }
            }
        });

        ScrollArea::vertical()
            .id_salt("known_blocks_scroll")
            .show(ui, |ui| {
                ui.label(format!(
                    "Total Known Blocks: {}",
                    self.data
                        .masternode_list_engine
                        .block_container
                        .known_block_count()
                ));

                egui::Grid::new("known_blocks_grid")
                    .num_columns(2) // Two columns: Block Height | Block Hash
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Block Height");
                        ui.label("Block Hash");
                        ui.end_row();

                        let MasternodeListEngineBlockContainer::BTreeMapContainer(map) =
                            &self.data.masternode_list_engine.block_container;

                        // Sort block heights for ordered display
                        let mut known_blocks: Vec<_> = map.block_heights.iter().collect();
                        known_blocks.sort_by_key(|(_, height)| *height);

                        for (block_hash, height) in known_blocks {
                            ui.label(format!("{}", height));
                            let hash_str = format!("{}", block_hash);

                            if ui.selectable_label(false, hash_str.clone()).clicked() {
                                ui.ctx().copy_text(hash_str.clone());
                            }

                            ui.end_row();
                        }
                    });
            });
    }

    fn render_diffs(&mut self, ui: &mut Ui) {
        // Add Save/Load functionality
        ui.horizontal(|ui| {
            if ui.button("Save MN List Diffs").clicked() {
                // Open native save dialog
                if let Some(path) = FileDialog::new()
                    .set_file_name("mnlistdiffs.dat")
                    .add_filter("Data Files", &["dat"])
                    .save_file()
                {
                    // Serialize and save the block container
                    let serialized_data = bincode::encode_to_vec(
                        &self.data.mnlist_diffs,
                        bincode::config::standard(),
                    )
                    .expect("serialize container");
                    if let Err(e) = std::fs::write(&path, serialized_data) {
                        tracing::error!("Failed to write file: {}", e);
                    }
                }
            }
        });
        // Create a three-column layout:
        // - Left column: list of MNList Diffs (by block height)
        // - Middle column: list of quorums for the selected DML
        // - Right column: quorum details
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::Vec2::new(150.0, 800.0), // Set fixed width for left column
                Layout::top_down(Align::Min),
                |ui| {
                    self.render_diff_list(ui);
                },
            );

            ui.separator(); // Optional: Adds a visual separator

            ui.allocate_ui_with_layout(
                egui::Vec2::new(ui.available_width() * 0.4, 800.0), // Middle column
                Layout::top_down(Align::Min),
                |ui| {
                    self.render_selected_dml_items(ui);
                },
            );

            ui.allocate_ui_with_layout(
                egui::Vec2::new(ui.available_width(), ui.available_height()), // Right column takes remaining space
                Layout::top_down(Align::Min),
                |ui| {
                    if self.selection.selected_quorum_in_diff_index.is_some() {
                        self.render_quorum_details(ui);
                    } else if self.selection.selected_masternode_in_diff_index.is_some() {
                        self.render_mn_details(ui);
                    }
                },
            );
        });
    }

    fn render_masternode_changes(&mut self, ui: &mut Ui) {
        ui.heading("Masternode changes");
        let Some(dml) = self.selected_dml() else {
            ui.label("Select a block height to show quorums.");
            return;
        };
        let new_masternodes = dml.new_masternodes.clone();

        ScrollArea::vertical()
            .id_salt("quorum_list_scroll_area")
            .show(ui, |ui| {
                for (m_index, masternode) in new_masternodes.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.selection.selected_masternode_in_diff_index == Some(m_index),
                            format!(
                                "{} {} {}",
                                if masternode.mn_type == EntryMasternodeType::Regular {
                                    "MN"
                                } else {
                                    "EN"
                                },
                                masternode.service_address.ip(),
                                masternode
                                    .pro_reg_tx_hash
                                    .to_string()
                                    .as_str()
                                    .split_at(5)
                                    .0
                            ),
                        )
                        .clicked()
                    {
                        self.selection.selected_quorum_in_diff_index = None;
                        self.selection.selected_masternode_in_diff_index = Some(m_index);
                    }
                }
            });
    }

    fn render_mn_diff_chain_locks(&mut self, ui: &mut Ui) {
        ui.heading("MN list diff chain locks");
        let Some(dml) = self.selected_dml() else {
            return;
        };

        ScrollArea::vertical()
            .id_salt("quorum_list_chain_locks_scroll_area")
            .show(ui, |ui| {
                for (index, sig) in dml.quorums_chainlock_signatures.iter().enumerate() {
                    ui.group(|ui| {
                        ui.label(format!("Signature #{}", index));
                        ui.monospace(format!(
                            "Signature: {}",
                            hex::encode(sig.signature.as_bytes())
                        ));
                        ui.label(format!("Index Set: {:?}", sig.index_set));
                    });
                }
            });
    }

    fn save_mn_list_diff(&mut self) {
        let Some(selected_key) = self.selection.selected_dml_diff_key else {
            self.ui_state.error = Some("No MNListDiff selected.".to_string());
            return;
        };

        let Some(mn_list_diff) = self.data.mnlist_diffs.get(&selected_key) else {
            self.ui_state.error = Some("Failed to retrieve selected MNListDiff.".to_string());
            return;
        };

        // Extract block heights from the selected key
        let (base_block_height, block_height) = selected_key;

        // Serialize the MNListDiff
        let serialized = serialize(mn_list_diff);

        // Generate the dynamic filename
        let file_name = format!("mn_list_diff_{}_{}.bin", base_block_height, block_height);

        // Open a file save dialog with the generated file name
        if let Some(path) = FileDialog::new()
            .set_title("Save MNListDiff")
            .add_filter("Binary", &["bin"])
            .set_file_name(&file_name) // Set the dynamic filename
            .save_file()
        {
            // Attempt to write the serialized data to the selected file
            match fs::write(&path, serialized) {
                Ok(_) => {
                    tracing::info!("MNListDiff saved to {:?}", path);
                }
                Err(e) => {
                    self.ui_state.error = Some(format!("Failed to save file: {}", e));
                }
            }
        }
    }

    /// Render the list of items for the selected DML, with a selector at the top
    fn render_selected_dml_items(&mut self, ui: &mut Ui) {
        ui.heading("Masternode List Diff Explorer");

        // Define available options for selection
        let options = [
            "New Quorums",
            "Masternode Changes",
            "Chain Locks",
            "Save Diff",
        ];
        let selected_index = self.selection.selected_option_index.unwrap_or(0);

        // Render the selection buttons
        ui.horizontal(|ui| {
            for (index, option) in options.iter().enumerate() {
                if ui
                    .selectable_label(selected_index == index, *option)
                    .clicked()
                {
                    // If the user selects "Save MNListDiff", trigger save function
                    if index == 3 {
                        self.save_mn_list_diff();
                    } else {
                        self.selection.selected_option_index = Some(index);
                    }
                }
            }
        });

        ui.separator();

        // Determine the selected category and display corresponding information
        if self.selected_dml().is_some() {
            ScrollArea::vertical()
                .id_salt("dml_items_scroll_area")
                .show(ui, |ui| match selected_index {
                    0 => self.render_new_quorums(ui),
                    1 => self.render_masternode_changes(ui),
                    2 => self.render_mn_diff_chain_locks(ui),
                    _ => (),
                });
        } else {
            ui.label("Select a block height to show details.");
        }
    }

    pub fn required_cl_sig_heights(&self, quorum: &QuorumEntry) -> BTreeSet<u32> {
        let mut required_heights = BTreeSet::new();
        let Ok(quorum_block_height) = self.get_height(&quorum.quorum_hash) else {
            return BTreeSet::new();
        };
        let llmq_params = quorum.llmq_type.params();
        let quorum_index = quorum_block_height % llmq_params.dkg_params.interval;
        let cycle_base_height = quorum_block_height - quorum_index;
        let cycle_length = llmq_params.dkg_params.interval;
        for i in 0..=3 {
            required_heights.insert(cycle_base_height - i * cycle_length - 8);
        }
        required_heights
    }

    /// Render the details for the selected quorum
    fn render_quorum_details(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let border = DashColors::border(dark_mode);
        ui.heading("Quorum Details");
        if let Some(dml_key) = self.selection.selected_dml_diff_key {
            let Some(dml) = self.data.mnlist_diffs.get(&dml_key) else {
                return;
            };
            let Some(q_index) = self.selection.selected_quorum_in_diff_index else {
                ui.label("Select a quorum to view details.");
                return;
            };
            let Some(quorum) = dml.new_quorums.get(q_index) else {
                return;
            };

            Frame::NONE
                .stroke(Stroke::new(1.0, border))
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                    let height = self.get_height(&quorum.quorum_hash).ok();

                    // Build a vector of optional signatures with slots matching new_quorums length
                    let mut quorum_sig_lookup: Vec<Option<&BLSSignature>> = vec![None; dml.new_quorums.len()];

                    // Fill each slot with the corresponding signature
                    for quorum_sig_obj in &dml.quorums_chainlock_signatures {
                        for &index in &quorum_sig_obj.index_set {
                            if let Some(slot) = quorum_sig_lookup.get_mut(index as usize) {
                                *slot = Some(&quorum_sig_obj.signature);
                            } else {
                                return;
                            }
                        }
                    }

                    // Verify all slots have been filled
                    if quorum_sig_lookup.iter().any(Option::is_none) {
                        return;
                    }

                    let chain_lock_msg = if let Some(a) = quorum_sig_lookup.get(q_index) {
                        if let Some(b) = a {
                            hex::encode(b)
                        } else {
                            "Error a".to_string()
                        }
                    } else {
                        "Error b".to_string()
                    };

                    let expected_chain_lock_sig = if let Some(height) = height {
                        if let Ok(hash) = self.get_block_hash(height - 8) {
                            if let Ok(Some(sig)) = self.get_chain_lock_sig(&hash) {
                                hex::encode(sig)
                            } else {
                                "Error (Did not find chain lock sig for hash)".to_string()
                            }
                        } else {
                            "Error (Did not find block hash of 8 blocks ago)".to_string()
                        }
                    } else {
                        "Error (Did not find quorum hash height)".to_string()
                    };
                    if quorum.llmq_type.is_rotating_quorum_type() {
                        ScrollArea::vertical().id_salt("render_quorum_details").show(ui, |ui| {
                            ui.label(format!(
                                "Version: {}\nQuorum Hash Height: {}\nQuorum Hash: {}\nCycle Hash Height: {}\nQuorum Index: {}\nSigners: {} members\nValid Members: {} members\nQuorum Public Key: {}\nAssociated Chain Lock Sig: {}\nExpected Chain Lock Sig: {}",
                                quorum.version,
                                self.get_height(&quorum.quorum_hash).ok().map(|height| format!("{}", height)).unwrap_or("Unknown".to_string()),
                                quorum.quorum_hash,
                                self.get_height(&quorum.quorum_hash).ok().and_then(|height| quorum.quorum_index.map(|index| format!("{}", height - index as CoreBlockHeight))).unwrap_or("Unknown".to_string()),
                                quorum.quorum_index.map(|quorum_index| quorum_index.to_string()).unwrap_or("Unknown".to_string()),
                                quorum.signers.iter().filter(|&&b| b).count(),
                                quorum.valid_members.iter().filter(|&&b| b).count(),
                                quorum.quorum_public_key,
                                chain_lock_msg,
                                expected_chain_lock_sig,
                            ));
                        });
                    } else {
                        ScrollArea::vertical().id_salt("render_quorum_details").show(ui, |ui| {
                            ui.label(format!(
                                "Version: {}\nQuorum Hash Height: {}\nQuorum Hash: {}\nSigners: {} members\nValid Members: {} members\nQuorum Public Key: {}\nAssociated Chain Lock Sig: {}\nExpected Chain Lock Sig: {}",
                                quorum.version,
                                self.get_height(&quorum.quorum_hash).ok().map(|height| format!("{}", height)).unwrap_or("Unknown".to_string()),
                                quorum.quorum_hash,
                                quorum.signers.iter().filter(|&&b| b).count(),
                                quorum.valid_members.iter().filter(|&&b| b).count(),
                                quorum.quorum_public_key,
                                chain_lock_msg,
                                expected_chain_lock_sig,
                            ));
                        });
                    }
                });
            return;
        }

        if let Some(selected_height) = self.selection.selected_dml_height_key {
            if let Some(mn_list) = self
                .data
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
            {
                if let Some((llmq_type, quorum_hash)) =
                    self.selection.selected_quorum_hash_in_mnlist_diff
                {
                    if let Some(quorum) = mn_list
                        .quorums
                        .get(&llmq_type)
                        .and_then(|quorums_by_type| quorums_by_type.get(&quorum_hash))
                    {
                        let height = self.get_height(&quorum.quorum_entry.quorum_hash).ok();
                        let chain_lock_sig =
                            if quorum.quorum_entry.llmq_type.is_rotating_quorum_type() {
                                let heights = self.required_cl_sig_heights(&quorum.quorum_entry);
                                format!(
                                    "heights [{}]",
                                    heights.iter().map(|h| h.to_string()).join(" | ")
                                )
                            } else if let Some(height) = height {
                                if let Ok(hash) = self.get_block_hash(height - 8) {
                                    if let Ok(Some(sig)) = self.get_chain_lock_sig(&hash) {
                                        hex::encode(sig)
                                    } else {
                                        "Error (Did not find chain lock sig for hash)".to_string()
                                    }
                                } else {
                                    "Error (Did not find block hash of 8 blocks ago)".to_string()
                                }
                            } else {
                                "Error (Did not find quorum hash height)".to_string()
                            };

                        let get_used_heights = |bls_signature: BLSSignature| {
                            let Some(used) =
                                self.cache.chain_lock_reversed_sig_cache.get(&bls_signature)
                            else {
                                return String::default();
                            };
                            if used.is_empty() {
                                String::default()
                            } else if used.len() == 1 {
                                format!(" [height: {}]", used.iter().next().unwrap().0)
                            } else {
                                format!(
                                    " [height: {} to {}]",
                                    used.iter().next().unwrap().0,
                                    used.last().unwrap().0
                                )
                            }
                        };

                        let associated_chain_lock_sig = match quorum.verifying_chain_lock_signature
                        {
                            Some(VerifyingChainLockSignaturesType::NonRotating(
                                associated_chain_lock_sig,
                            )) => hex::encode(associated_chain_lock_sig),
                            Some(VerifyingChainLockSignaturesType::Rotating(
                                associated_chain_lock_sigs,
                            )) => {
                                format!(
                                    "[\n-3: {}{}\n-2: {}{}\n-1: {}{}\n0: {}{}\n]",
                                    hex::encode(associated_chain_lock_sigs[0]),
                                    get_used_heights(associated_chain_lock_sigs[0]),
                                    hex::encode(associated_chain_lock_sigs[1]),
                                    get_used_heights(associated_chain_lock_sigs[1]),
                                    hex::encode(associated_chain_lock_sigs[2]),
                                    get_used_heights(associated_chain_lock_sigs[2]),
                                    hex::encode(associated_chain_lock_sigs[3]),
                                    get_used_heights(associated_chain_lock_sigs[3])
                                )
                            }
                            None => "None set".to_string(),
                        };

                        Frame::NONE
                            .stroke(Stroke::new(1.0, border))
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                                ScrollArea::vertical().id_salt("render_quorum_details_2").show(ui, |ui| {
                                    ui.label(format!(
                                        "Quorum Type: {}\nQuorum Height: {}\nQuorum Hash: {}\nCommitment Hash: {}\nCommitment Data: {}\nEntry Hash: {}\nSigners: {} members\nValid Members: {} members\nQuorum Public Key: {}\nValidation Status: {}\nAssociated Chain Lock Sig: {}\nExpected Chain Lock Sig: {}",
                                        QuorumType::from(quorum.quorum_entry.llmq_type as u32),
                                        self.get_height(&quorum.quorum_entry.quorum_hash).ok().map(|height| format!("{}", height)).unwrap_or("Unknown".to_string()),
                                        quorum.quorum_entry.quorum_hash,
                                        quorum.commitment_hash,
                                        hex::encode(quorum.quorum_entry.commitment_data()),
                                        quorum.entry_hash,
                                        quorum.quorum_entry.signers.iter().filter(|&&b| b).count(),
                                        quorum.quorum_entry.valid_members.iter().filter(|&&b| b).count(),
                                        quorum.quorum_entry.quorum_public_key,
                                        quorum.verified,
                                        associated_chain_lock_sig,
                                        chain_lock_sig,
                                    ));
                                });
                            });
                    }
                } else {
                    ui.label("Select a quorum to view details.");
                }
            }
        } else {
            ui.label("Select a block height and quorum.");
        }
    }

    /// Render the details for the selected Masternode
    fn render_mn_details(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let border = DashColors::border(dark_mode);
        ui.heading("Masternode Details");

        if let Some(dml_key) = self.selection.selected_dml_diff_key {
            if let Some(dml) = self.data.mnlist_diffs.get(&dml_key) {
                if let Some(mn_index) = self.selection.selected_masternode_in_diff_index {
                    if let Some(masternode) = dml.new_masternodes.get(mn_index) {
                        Frame::NONE.stroke(Stroke::new(1.0, border)).show(ui, |ui| {
                            ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                            ScrollArea::vertical()
                                .id_salt("render_mn_details")
                                .show(ui, |ui| {
                                    ui.label(format!(
                                        "Version: {}\n\
                                     ProRegTxHash: {}\n\
                                     Confirmed Hash: {}\n\
                                     Service Address: {}:{}\n\
                                     Operator Public Key: {}\n\
                                     Voting Key ID: {}\n\
                                     Is Valid: {}\n\
                                     Masternode Type: {}",
                                        masternode.version,
                                        masternode.pro_reg_tx_hash.reverse(),
                                        match masternode.confirmed_hash {
                                            None => "No confirmed hash".to_string(),
                                            Some(confirmed_hash) =>
                                                confirmed_hash.reverse().to_string(),
                                        },
                                        masternode.service_address.ip(),
                                        masternode.service_address.port(),
                                        masternode.operator_public_key,
                                        masternode.key_id_voting,
                                        masternode.is_valid,
                                        match masternode.mn_type {
                                            EntryMasternodeType::Regular => "Regular".to_string(),
                                            EntryMasternodeType::HighPerformance {
                                                platform_http_port,
                                                platform_node_id,
                                            } => {
                                                format!(
                                                    "High Performance (Port: {}, Node ID: {})",
                                                    platform_http_port, platform_node_id
                                                )
                                            }
                                        }
                                    ));
                                });
                        });
                    }
                } else {
                    ui.label("Select a Masternode to view details.");
                }
            }
        } else if let Some(selected_height) = self.selection.selected_dml_height_key {
            if let Some(mn_list) = self
                .data
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
                && let Some(selected_pro_tx_hash) = self.selection.selected_masternode_pro_tx_hash
                && let Some(qualified_masternode) = mn_list.masternodes.get(&selected_pro_tx_hash)
            {
                let masternode = &qualified_masternode.masternode_list_entry;
                Frame::NONE.stroke(Stroke::new(1.0, border)).show(ui, |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                    ScrollArea::vertical()
                        .id_salt("render_mn_details_2")
                        .show(ui, |ui| {
                            ui.label(format!(
                                "Version: {}\n\
                                     ProRegTxHash: {}\n\
                                     Confirmed Hash: {}\n\
                                     Service Address: {}:{}\n\
                                     Operator Public Key: {}\n\
                                     Voting Key ID: {}\n\
                                     Is Valid: {}\n\
                                     Masternode Type: {}\n\
                                     Entry Hash: {}\n\
                                     Confirmed Hash hashed with ProRegTx: {}\n",
                                masternode.version,
                                masternode.pro_reg_tx_hash.reverse(),
                                match masternode.confirmed_hash {
                                    None => "No confirmed hash".to_string(),
                                    Some(confirmed_hash) => confirmed_hash.reverse().to_string(),
                                },
                                masternode.service_address.ip(),
                                masternode.service_address.port(),
                                masternode.operator_public_key,
                                masternode.key_id_voting,
                                masternode.is_valid,
                                match masternode.mn_type {
                                    EntryMasternodeType::Regular => "Regular".to_string(),
                                    EntryMasternodeType::HighPerformance {
                                        platform_http_port,
                                        platform_node_id,
                                    } => {
                                        format!(
                                            "High Performance (Port: {}, Node ID: {})",
                                            platform_http_port, platform_node_id
                                        )
                                    }
                                },
                                hex::encode(qualified_masternode.entry_hash),
                                if let Some(hash) =
                                    qualified_masternode.confirmed_hash_hashed_with_pro_reg_tx
                                {
                                    hash.reverse().to_string()
                                } else {
                                    "None".to_string()
                                },
                            ));
                        });
                });
            }
        } else {
            ui.label("Select a block height and Masternode.");
        }
    }

    fn render_selected_shapshot_details(ui: &mut Ui, snapshot: &QuorumSnapshot) {
        ui.heading("Quorum Snapshot Details");

        // Display Skip List Mode
        ui.label(format!("Skip List Mode: {}", snapshot.skip_list_mode));

        // Display Active Quorum Members (Bitset)
        ui.label(format!(
            "Active Quorum Members: {} members",
            snapshot.active_quorum_members.len()
        ));

        // Show active members in a scrollable area
        ScrollArea::vertical()
            .id_salt("render_snapshot_details")
            .show(ui, |ui| {
                ui.label("Active Quorum Members:");
                for (i, active) in snapshot.active_quorum_members.iter().enumerate() {
                    ui.label(format!(
                        "Member {}: {}",
                        i,
                        if *active { "Active" } else { "Inactive" }
                    ));
                }
            });

        ui.separator();

        // Display Skip List
        ui.label(format!("Skip List: {} entries", snapshot.skip_list.len()));

        // Show skip list entries
        ScrollArea::vertical()
            .id_salt("render_snapshot_details_2")
            .show(ui, |ui| {
                ui.label("Skip List Entries:");
                for (i, skip_entry) in snapshot.skip_list.iter().enumerate() {
                    ui.label(format!("Entry {}: {}", i, skip_entry));
                }
            });
    }

    fn render_qr_info(&mut self, ui: &mut Ui) {
        ui.heading("QRInfo Viewer");

        // Select the first available QRInfo if none is selected
        let selected_qr_info = {
            let Some((_, selected_qr_info)) = self.data.qr_infos.first_key_value() else {
                ui.label("No QRInfo available.");
                if ui.button("Load QR Info").clicked()
                    && let Some(path) = FileDialog::new()
                        .add_filter("Data Files", &["dat"])
                        .pick_file()
                {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            // Let's first try consensus decode
                            match QRInfo::consensus_decode(&mut std::io::Cursor::new(&bytes)) {
                                Ok(qr_info) => {
                                    let key = qr_info.mn_list_diff_tip.block_hash;
                                    self.data.qr_infos.insert(key, qr_info.clone());
                                    self.feed_qr_info_and_get_dmls(qr_info, None);
                                }
                                Err(_) => {
                                    match bincode::decode_from_slice::<QRInfo, _>(
                                        &bytes,
                                        bincode::config::standard(),
                                    ) {
                                        Ok((qr_info, _)) => {
                                            let key = qr_info.mn_list_diff_tip.block_hash;
                                            self.data.qr_infos.insert(key, qr_info);
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to decode QRInfo: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to read file: {}", e);
                        }
                    }
                }
                return;
            };
            selected_qr_info.clone()
        };

        if let Ok(height) = self.get_height(&selected_qr_info.mn_list_diff_tip.block_hash) {
            // Add Save/Load functionality
            ui.horizontal(|ui| {
                if ui.button("Save QR Info").clicked() {
                    // Open native save dialog
                    if let Some(path) = FileDialog::new()
                        .set_file_name(format!("qrinfo_{}.dat", height))
                        .add_filter("Data Files", &["dat"])
                        .save_file()
                    {
                        // Serialize and save the block container
                        let serialized_data =
                            bincode::encode_to_vec(&selected_qr_info, bincode::config::standard())
                                .expect("serialize container");
                        if let Err(e) = std::fs::write(&path, serialized_data) {
                            tracing::error!("Failed to write file: {}", e);
                        }
                    }
                }
            });
        }

        // Track user selections
        if self.selection.selected_qr_field.is_none() {
            self.selection.selected_qr_field = Some("Quorum Snapshots".to_string());
        }

        ui.horizontal(|ui| {
            // Left Panel: Fields of QRInfo
            ui.allocate_ui_with_layout(
                egui::Vec2::new(180.0, ui.available_height()),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.label("QRInfo Fields:");
                    let fields = [
                        "Rotated Quorums At Index",
                        "Masternode List Diffs",
                        "Quorum Snapshots",
                        "Quorum Snapshot List",
                        "MN List Diff List",
                    ];

                    for field in &fields {
                        if ui
                            .selectable_label(
                                self.selection.selected_qr_field.as_deref() == Some(*field),
                                *field,
                            )
                            .clicked()
                        {
                            self.selection.selected_qr_field = Some(field.to_string());
                            self.selection.selected_qr_list_index = None;
                            self.selection.selected_qr_item = None;
                        }
                    }
                },
            );

            ui.separator();

            // Center Panel: Items in the selected field
            ui.allocate_ui_with_layout(
                egui::Vec2::new(ui.available_width() * 0.5, ui.available_height()),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.heading("Selected Field Items");

                    match self.selection.selected_qr_field.as_deref() {
                        Some("Quorum Snapshots") => {
                            self.render_quorum_snapshots(ui, &selected_qr_info)
                        }
                        Some("Masternode List Diffs") => {
                            self.render_mn_list_diffs(ui, &selected_qr_info)
                        }
                        Some("Rotated Quorums At Index") => self.render_last_commitments(
                            ui,
                            selected_qr_info
                                .last_commitment_per_index
                                .first()
                                .map(|entry| entry.quorum_hash),
                        ),
                        Some("Quorum Snapshot List") => {
                            self.render_quorum_snapshot_list(ui, &selected_qr_info)
                        }
                        Some("MN List Diff List") => {
                            self.render_mn_list_diff_list(ui, &selected_qr_info)
                        }
                        _ => {
                            ui.label("Select a field to display.");
                        }
                    }
                },
            );

            ui.separator();

            // Right Panel: Detailed View of Selected Item
            ui.allocate_ui_with_layout(
                egui::Vec2::new(ui.available_width(), ui.available_height()),
                Layout::top_down(Align::Min),
                |ui| {
                    if let Some(selected_item) = &self.selection.selected_qr_item {
                        match selected_item {
                            SelectedQRItem::SelectedSnapshot(snapshot) => {
                                Self::render_selected_shapshot_details(ui, snapshot);
                            }
                            SelectedQRItem::MNListDiff(mn_list_diff) => {
                                self.render_selected_mn_list_diff(ui, mn_list_diff);
                            }
                            SelectedQRItem::QuorumEntry(quorum_entry) => {
                                Self::render_selected_quorum_entry(ui, quorum_entry);
                            }
                        }
                    } else {
                        ui.label("Select an item to view details.");
                    }
                },
            );
        });
    }
    fn render_selected_mn_list_diff(&self, ui: &mut Ui, mn_list_diff: &MnListDiff) {
        ui.heading("MNListDiff Details");

        // General MNListDiff Info
        ui.label(format!(
            "Version: {}\nBase Block Hash: {} ({})\nBlock Hash: {} ({})",
            mn_list_diff.version,
            mn_list_diff.base_block_hash,
            self.get_height_or_error_as_string(&mn_list_diff.base_block_hash),
            mn_list_diff.block_hash,
            self.get_height_or_error_as_string(&mn_list_diff.block_hash)
        ));

        ui.label(format!(
            "Total Transactions: {}",
            mn_list_diff.total_transactions
        ));

        ui.separator();

        // Merkle Tree Data
        ui.heading("Merkle Tree");
        ui.label(format!(
            "Merkle Hashes: {} entries",
            mn_list_diff.merkle_hashes.len()
        ));
        ScrollArea::vertical()
            .id_salt("render_selected_mn_list_diff")
            .show(ui, |ui| {
                for (i, merkle_hash) in mn_list_diff.merkle_hashes.iter().enumerate() {
                    ui.label(format!("{}: {}", i, merkle_hash));
                }
            });

        ui.separator();
        ui.label(format!(
            "Merkle Flags ({} bytes)",
            mn_list_diff.merkle_flags.len()
        ));

        // Coinbase Transaction
        ui.heading("Coinbase Transaction");
        ScrollArea::vertical()
            .id_salt("render_selected_mn_list_diff_2")
            .show(ui, |ui| {
                ui.label(format!(
                    "Coinbase TXID: {}\nSize: {} bytes",
                    mn_list_diff.coinbase_tx.txid(),
                    mn_list_diff.coinbase_tx.size()
                ));
            });

        ui.separator();

        // Masternode Changes
        ui.heading("Masternode Changes");
        ui.label(format!(
            "New Masternodes: {}\nDeleted Masternodes: {}",
            mn_list_diff.new_masternodes.len(),
            mn_list_diff.deleted_masternodes.len(),
        ));

        ScrollArea::vertical()
            .id_salt("render_selected_mn_list_diff_3")
            .show(ui, |ui| {
                ui.heading("New Masternodes");
                for masternode in &mn_list_diff.new_masternodes {
                    ui.label(format!(
                        "{} {}:{}",
                        masternode.pro_reg_tx_hash,
                        masternode.service_address.ip(),
                        masternode.service_address.port(),
                    ));
                }

                ui.separator();
                ui.heading("Removed Masternodes");
                for removed_pro_tx in &mn_list_diff.deleted_masternodes {
                    ui.label(removed_pro_tx.to_string());
                }
            });

        ui.separator();

        // Quorum Changes
        ui.heading("Quorum Changes");
        ui.label(format!(
            "New Quorums: {}\nDeleted Quorums: {}",
            mn_list_diff.new_quorums.len(),
            mn_list_diff.deleted_quorums.len()
        ));

        ScrollArea::vertical()
            .id_salt("render_selected_mn_list_diff_4")
            .show(ui, |ui| {
                ui.heading("New Quorums");
                for quorum in &mn_list_diff.new_quorums {
                    ui.label(format!(
                        "Quorum {} Type: {}",
                        quorum.quorum_hash,
                        QuorumType::from(quorum.llmq_type as u32)
                    ));
                }

                ui.separator();
                ui.heading("Removed Quorums");
                for deleted_quorum in &mn_list_diff.deleted_quorums {
                    ui.label(format!(
                        "Quorum {} Type: {}",
                        deleted_quorum.quorum_hash,
                        QuorumType::from(deleted_quorum.llmq_type as u32)
                    ));
                }
            });

        ui.separator();

        // Quorums ChainLock Signatures
        ui.heading("Quorums ChainLock Signatures");
        ui.label(format!(
            "Total ChainLock Signatures: {}",
            mn_list_diff.quorums_chainlock_signatures.len()
        ));

        ScrollArea::vertical()
            .id_salt("render_selected_mn_list_diff_5")
            .show(ui, |ui| {
                for (i, cl_sig) in mn_list_diff.quorums_chainlock_signatures.iter().enumerate() {
                    ui.label(format!(
                        "Signature {}: {} for indexes [{}]",
                        i,
                        hex::encode(cl_sig.signature),
                        cl_sig
                            .index_set
                            .iter()
                            .map(|index| index.to_string())
                            .collect::<Vec<_>>()
                            .join("-")
                    ));
                }
            });
    }

    fn render_quorum_snapshots(&mut self, ui: &mut Ui, qr_info: &QRInfo) {
        let snapshots = [
            ("Quorum Snapshot h-c", &qr_info.quorum_snapshot_at_h_minus_c),
            (
                "Quorum Snapshot h-2c",
                &qr_info.quorum_snapshot_at_h_minus_2c,
            ),
            (
                "Quorum Snapshot h-3c",
                &qr_info.quorum_snapshot_at_h_minus_3c,
            ),
        ];

        if let Some((qs4c, _)) = &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c {
            snapshots.iter().for_each(|(name, snapshot)| {
                if ui
                    .selectable_label(
                        self.selection.selected_qr_list_index == Some(name.to_string()),
                        *name,
                    )
                    .clicked()
                {
                    self.selection.selected_qr_list_index = Some(name.to_string());
                    self.selection.selected_qr_item =
                        Some(SelectedQRItem::SelectedSnapshot((*snapshot).clone()));
                }
            });

            if ui
                .selectable_label(
                    self.selection.selected_qr_list_index
                        == Some("Quorum Snapshot h-4c".to_string()),
                    "Quorum Snapshot h-4c",
                )
                .clicked()
            {
                self.selection.selected_qr_list_index = Some("Quorum Snapshot h-4c".to_string());
                self.selection.selected_qr_item =
                    Some(SelectedQRItem::SelectedSnapshot((*qs4c).clone()));
            }
        }
    }

    fn render_selected_quorum_entry(ui: &mut Ui, qualified_quorum_entry: &QualifiedQuorumEntry) {
        ui.heading("Quorum Entry Details");

        // General Quorum Info
        ui.label(format!(
            "Version: {}\nQuorum Type: {}\nQuorum Hash: {}",
            qualified_quorum_entry.quorum_entry.version,
            QuorumType::from(qualified_quorum_entry.quorum_entry.llmq_type as u32),
            qualified_quorum_entry.quorum_entry.quorum_hash
        ));

        ui.label(format!(
            "Quorum Index: {}",
            qualified_quorum_entry
                .quorum_entry
                .quorum_index
                .map_or("None".to_string(), |idx| idx.to_string())
        ));

        ui.separator();

        // **Additional Qualified Quorum Entry Information**
        ui.heading("Quorum Verification Details");
        let verification_symbol = match &qualified_quorum_entry.verified {
            LLMQEntryVerificationStatus::Verified => "✔ Verified".to_string(),
            LLMQEntryVerificationStatus::Invalid(reason) => format!("❌ Invalid ({})", reason),
            LLMQEntryVerificationStatus::Unknown => "⬜ Unknown".to_string(),
            LLMQEntryVerificationStatus::Skipped(reason) => format!("⬜ Skipped ({})", reason),
        };
        ui.label(format!("Verification Status: {}", verification_symbol));

        ui.separator();

        ui.heading("Commitment & Entry Hashes");
        ScrollArea::vertical()
            .id_salt("commitment_entry_hash")
            .show(ui, |ui| {
                ui.label(format!(
                    "Commitment Hash: {}",
                    qualified_quorum_entry.commitment_hash
                ));
                ui.label(format!("Entry Hash: {}", qualified_quorum_entry.entry_hash));
            });

        ui.separator();

        // Signers & Valid Members
        ui.heading("Quorum Members");
        ui.label(format!(
            "Total Signers: {}\nValid Members: {}",
            qualified_quorum_entry
                .quorum_entry
                .signers
                .iter()
                .filter(|&&b| b)
                .count(),
            qualified_quorum_entry
                .quorum_entry
                .valid_members
                .iter()
                .filter(|&&b| b)
                .count()
        ));

        ScrollArea::vertical()
            .id_salt("quorum_members_grid")
            .show(ui, |ui| {
                ui.label(format!(
                    "Total Signers: {}\nValid Members: {}",
                    qualified_quorum_entry
                        .quorum_entry
                        .signers
                        .iter()
                        .filter(|&&b| b)
                        .count(),
                    qualified_quorum_entry
                        .quorum_entry
                        .valid_members
                        .iter()
                        .filter(|&&b| b)
                        .count()
                ));

                ui.separator();

                ui.heading("Signers & Valid Members Grid");

                egui::Grid::new("quorum_members_grid")
                    .num_columns(8) // Adjust based on UI width
                    .striped(true)
                    .show(ui, |ui| {
                        for (i, (is_signer, is_valid)) in qualified_quorum_entry
                            .quorum_entry
                            .signers
                            .iter()
                            .zip(qualified_quorum_entry.quorum_entry.valid_members.iter())
                            .enumerate()
                        {
                            let text = match (*is_signer, *is_valid) {
                                (true, true) => "✔✔",
                                (true, false) => "✔❌",
                                (false, true) => "❌✔",
                                (false, false) => "❌❌",
                            };

                            let response = ui.label(text);

                            // Tooltip on hover to show member index
                            if response.hovered() {
                                ui.ctx().debug_painter().text(
                                    response.rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    format!("Member {}", i),
                                    egui::FontId::proportional(14.0),
                                    egui::Color32::BLUE,
                                );
                            }

                            // Create a new row every 8 members
                            if (i + 1) % 8 == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });

        ui.separator();

        // Quorum Public Key
        ui.heading("Quorum Public Key");
        ScrollArea::vertical()
            .id_salt("render_selected_quorum_entry_2")
            .show(ui, |ui| {
                ui.label(format!(
                    "Public Key: {}",
                    qualified_quorum_entry.quorum_entry.quorum_public_key
                ));
            });

        ui.separator();

        // Quorum Verification Vector Hash
        ui.heading("Verification Vector Hash");
        ui.label(format!(
            "Quorum VVec Hash: {}",
            qualified_quorum_entry.quorum_entry.quorum_vvec_hash
        ));

        ui.separator();

        // Threshold Signature
        ui.heading("Threshold Signature");
        ScrollArea::vertical()
            .id_salt("render_selected_quorum_entry_3")
            .show(ui, |ui| {
                ui.label(format!(
                    "Signature: {}",
                    hex::encode(qualified_quorum_entry.quorum_entry.threshold_sig.to_bytes())
                ));
            });

        ui.separator();

        // Aggregated Signature
        ui.heading("All Commitment Aggregated Signature");
        ScrollArea::vertical()
            .id_salt("render_selected_quorum_entry_4")
            .show(ui, |ui| {
                ui.label(format!(
                    "Signature: {}",
                    hex::encode(
                        qualified_quorum_entry
                            .quorum_entry
                            .all_commitment_aggregated_signature
                            .to_bytes()
                    )
                ));
            });
    }

    fn show_mn_list_diff_heights_as_string(
        &mut self,
        mn_list_diff: &MnListDiff,
        last_diff: Option<&MnListDiff>,
    ) -> String {
        let base_height_as_string = match self.get_height_and_cache(&mn_list_diff.base_block_hash) {
            Ok(height) => height.to_string(),
            Err(_) => "?".to_string(),
        };

        let height = self.get_height_and_cache(&mn_list_diff.block_hash).ok();

        let height_as_string = match height {
            Some(height) => height.to_string(),
            None => "?".to_string(),
        };

        let extra_block_diff_info = height
            .and_then(|height| {
                last_diff.and_then(|diff| {
                    self.get_height(&diff.block_hash)
                        .ok()
                        .and_then(|start_height| {
                            height
                                .checked_sub(start_height)
                                .map(|diff| format!(" (+ {})", diff))
                        })
                })
            })
            .unwrap_or_default();

        format!(
            "{} -> {}{}",
            base_height_as_string, height_as_string, extra_block_diff_info
        )
    }

    fn render_mn_list_diffs(&mut self, ui: &mut Ui, qr_info: &QRInfo) {
        let mn_diffs = [
            (
                format!(
                    "MNListDiff h-3c {}",
                    self.show_mn_list_diff_heights_as_string(
                        &qr_info.mn_list_diff_at_h_minus_3c,
                        qr_info
                            .quorum_snapshot_and_mn_list_diff_at_h_minus_4c
                            .as_ref()
                            .map(|(_, diff)| diff)
                    )
                ),
                &qr_info.mn_list_diff_at_h_minus_3c,
            ),
            (
                format!(
                    "MNListDiff h-2c {}",
                    self.show_mn_list_diff_heights_as_string(
                        &qr_info.mn_list_diff_at_h_minus_2c,
                        Some(&qr_info.mn_list_diff_at_h_minus_3c)
                    )
                ),
                &qr_info.mn_list_diff_at_h_minus_2c,
            ),
            (
                format!(
                    "MNListDiff h-c {}",
                    self.show_mn_list_diff_heights_as_string(
                        &qr_info.mn_list_diff_at_h_minus_c,
                        Some(&qr_info.mn_list_diff_at_h_minus_2c)
                    )
                ),
                &qr_info.mn_list_diff_at_h_minus_c,
            ),
            (
                format!(
                    "MNListDiff h {}",
                    self.show_mn_list_diff_heights_as_string(
                        &qr_info.mn_list_diff_h,
                        Some(&qr_info.mn_list_diff_at_h_minus_c)
                    )
                ),
                &qr_info.mn_list_diff_h,
            ),
            (
                format!(
                    "MNListDiff Tip {}",
                    self.show_mn_list_diff_heights_as_string(
                        &qr_info.mn_list_diff_tip,
                        Some(&qr_info.mn_list_diff_h)
                    )
                ),
                &qr_info.mn_list_diff_tip,
            ),
        ];
        if let Some((_, mn_diff4c)) = &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c {
            let string = format!(
                "MNListDiff h-4c {}",
                self.show_mn_list_diff_heights_as_string(mn_diff4c, None)
            );

            if ui
                .selectable_label(
                    self.selection.selected_qr_list_index == Some(string.clone()),
                    string.as_str(),
                )
                .clicked()
            {
                self.selection.selected_qr_list_index = Some(string);
                self.selection.selected_qr_item =
                    Some(SelectedQRItem::MNListDiff(Box::new((*mn_diff4c).clone())));
            }
        }

        mn_diffs.iter().for_each(|(name, diff)| {
            if ui
                .selectable_label(
                    self.selection.selected_qr_list_index == Some(name.to_string()),
                    name,
                )
                .clicked()
            {
                self.selection.selected_qr_list_index = Some(name.to_string());
                self.selection.selected_qr_item =
                    Some(SelectedQRItem::MNListDiff(Box::new((*diff).clone())));
            }
        });
    }

    fn render_last_commitments(&mut self, ui: &mut Ui, cycle_hash: Option<BlockHash>) {
        let Some(cycle_hash) = cycle_hash else {
            ui.label("QR Info had no rotated quorums. This should not happen.");
            return;
        };
        let Some(cycle_quorums) = self
            .data
            .masternode_list_engine
            .rotated_quorums_per_cycle
            .get(&cycle_hash)
        else {
            ui.label(format!(
                "Engine does not know of cycle {} at height {}, we know of cycles [{}]",
                cycle_hash,
                self.get_height_or_error_as_string(&cycle_hash),
                self.data
                    .masternode_list_engine
                    .rotated_quorums_per_cycle
                    .keys()
                    .map(|key| format!("{}, {}", self.get_height_or_error_as_string(key), key))
                    .join(", ")
            ));
            return;
        };
        if cycle_quorums.is_empty() {
            ui.label(format!(
                "Engine does not contain any rotated quorums for cycle {}",
                cycle_hash
            ));
        }
        for (index, commitment) in cycle_quorums.iter().enumerate() {
            // Determine the appropriate symbol based on verification status
            let verification_symbol = match commitment.verified {
                LLMQEntryVerificationStatus::Verified => "✔", // Checkmark
                LLMQEntryVerificationStatus::Invalid(_) => "❌", // Cross
                LLMQEntryVerificationStatus::Unknown | LLMQEntryVerificationStatus::Skipped(_) => {
                    "⬜"
                } // Box
            };

            let label_text = format!("{} Quorum at Index {}", verification_symbol, index);

            if ui
                .selectable_label(
                    self.selection.selected_qr_list_index == Some(index.to_string()),
                    label_text,
                )
                .clicked()
            {
                self.selection.selected_qr_list_index = Some(index.to_string());
                self.selection.selected_qr_item =
                    Some(SelectedQRItem::QuorumEntry(Box::new(commitment.clone())));
            }
        }
    }

    fn render_quorum_snapshot_list(&mut self, ui: &mut Ui, qr_info: &QRInfo) {
        for (index, snapshot) in qr_info.quorum_snapshot_list.iter().enumerate() {
            if ui
                .selectable_label(
                    self.selection.selected_qr_list_index == Some(index.to_string()),
                    format!("Snapshot {}", index),
                )
                .clicked()
            {
                self.selection.selected_qr_list_index = Some(index.to_string());
                self.selection.selected_qr_item =
                    Some(SelectedQRItem::SelectedSnapshot(snapshot.clone()));
            }
        }
    }

    fn render_mn_list_diff_list(&mut self, ui: &mut Ui, qr_info: &QRInfo) {
        for (index, diff) in qr_info.mn_list_diff_list.iter().enumerate() {
            if ui
                .selectable_label(
                    self.selection.selected_qr_list_index == Some(index.to_string()),
                    format!("MNListDiff {}", index),
                )
                .clicked()
            {
                self.selection.selected_qr_list_index = Some(index.to_string());
                self.selection.selected_qr_item =
                    Some(SelectedQRItem::MNListDiff(Box::new(diff.clone())));
            }
        }
    }

    fn render_quorums(&mut self, ui: &mut Ui) {
        ui.heading("Quorum Viewer");

        // Get all available quorum types
        let quorum_types: Vec<LLMQType> = self
            .data
            .masternode_list_engine
            .quorum_statuses
            .keys()
            .cloned()
            .collect();

        // Ensure a quorum type is selected
        if self
            .selection
            .selected_quorum_type_in_quorum_viewer
            .is_none()
        {
            self.selection.selected_quorum_type_in_quorum_viewer = quorum_types.first().copied();
        }

        // Render quorum type selection bar
        ui.horizontal(|ui| {
            for quorum_type in &quorum_types {
                if ui
                    .selectable_label(
                        self.selection.selected_quorum_type_in_quorum_viewer == Some(*quorum_type),
                        quorum_type.to_string(),
                    )
                    .clicked()
                {
                    self.selection.selected_quorum_type_in_quorum_viewer = Some(*quorum_type);
                    self.selection.selected_quorum_hash_in_quorum_viewer = None; // Reset selected quorum when switching types
                }
            }
        });

        ui.separator();

        let Some(selected_quorum_type) = self.selection.selected_quorum_type_in_quorum_viewer
        else {
            ui.label("No quorum types available.");
            return;
        };

        let Some(quorum_map) = self
            .data
            .masternode_list_engine
            .quorum_statuses
            .get(&selected_quorum_type)
        else {
            ui.label("No quorums found for this type.");
            return;
        };

        // Create a horizontal layout to align quorum hashes on the left and heights on the right
        ui.horizontal(|ui| {
            // Left Column: Quorum Hashes
            ui.allocate_ui_with_layout(
                egui::Vec2::new(500.0, 800.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.heading(format!("Quorums of Type: {}", selected_quorum_type));

                    ScrollArea::vertical()
                        .id_salt("quorum_hashes_scroll")
                        .show(ui, |ui| {
                            egui::Grid::new("quorum_hashes_grid")
                                .num_columns(2) // Two columns: Quorum Hash | Status
                                .striped(true)
                                .show(ui, |ui| {
                                    ui.label("Quorum Hash");
                                    ui.label("Status");
                                    ui.end_row();

                                    for (quorum_hash, (_, _, status)) in quorum_map {
                                        let hash_label = format!("{}", quorum_hash);

                                        // Display quorum hash as selectable
                                        let hash_response = ui.selectable_label(
                                            self.selection.selected_quorum_hash_in_quorum_viewer
                                                == Some(*quorum_hash),
                                            hash_label,
                                        );

                                        if hash_response.clicked() {
                                            self.selection.selected_quorum_hash_in_quorum_viewer =
                                                Some(*quorum_hash);
                                        }

                                        // Determine status symbol
                                        let (status_symbol, tooltip_text) = match status {
                                            LLMQEntryVerificationStatus::Verified => ("✔", None),
                                            LLMQEntryVerificationStatus::Invalid(reason) => {
                                                ("❌", Some(reason.to_string()))
                                            }
                                            LLMQEntryVerificationStatus::Unknown => ("⬜", None),
                                            LLMQEntryVerificationStatus::Skipped(reason) => {
                                                ("⚠", Some(reason.to_string()))
                                            }
                                        };

                                        // Display small status icon
                                        let status_response = ui.label(status_symbol);

                                        // Show tooltip on hover if there's an error message
                                        if let Some(tooltip) = tooltip_text
                                            && status_response.hovered()
                                        {
                                            ui.ctx().debug_painter().text(
                                                status_response.rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                tooltip,
                                                egui::FontId::proportional(14.0),
                                                egui::Color32::RED,
                                            );
                                        }

                                        ui.end_row();
                                    }
                                });
                        });
                },
            );

            ui.separator();

            // Right Column: Heights where selected quorum exists
            ui.allocate_ui_with_layout(
                Vec2::new(500.0, 800.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.heading("Quorum Heights");

                    if let Some(selected_quorum_hash) =
                        self.selection.selected_quorum_hash_in_quorum_viewer
                    {
                        if let Some((heights, key, status)) = quorum_map.get(&selected_quorum_hash)
                        {
                            ui.label(format!("Public Key: {}", key));
                            ui.label(format!("Verification Status: {}", status));
                            ScrollArea::vertical()
                                .id_salt("quorum_heights_scroll")
                                .show(ui, |ui| {
                                    for height in heights {
                                        ui.label(format!("Height: {}", height));
                                    }
                                });
                        } else {
                            ui.label("Selected quorum not found.");
                        }
                    } else {
                        ui.label("Select a quorum to see its heights.");
                    }
                },
            );
        });
    }

    #[allow(dead_code)]
    fn render_selected_item_details(&mut self, ui: &mut Ui, selected_item: String) {
        ui.heading("Details");

        ScrollArea::vertical().show(ui, |ui| {
            ui.monospace(selected_item);
        });
    }

    /// Render core items, including chain-locked blocks and instant send transactions.
    fn render_core_items(&mut self, ui: &mut Ui) {
        ui.heading("Core Items Viewer");

        // Layout: Left (ChainLocked Blocks), Middle (InstantSend Transactions), Right (Details)
        ui.horizontal(|ui| {
            // Left Column: Chain Locked Blocks
            ui.allocate_ui_with_layout(
                Vec2::new(200.0, 1000.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.heading("ChainLocked Blocks");

                    ScrollArea::vertical().id_salt("chain_locked_blocks_scroll").show(ui, |ui| {
                        for (block_height, (block, chain_lock, is_valid)) in
                            self.incoming.chain_locked_blocks.iter()
                        {
                            let label_text = format!(
                                "{} {} {}",
                                if *is_valid { "✔" } else { "❌" },
                                block_height,
                                block.header.block_hash()
                            );

                            if ui
                                .selectable_label(
                                    matches!(self.selection.selected_core_item, Some((CoreItem::ChainLockedBlock(_, ref l), _)) if l.block_height == *block_height),
                                    label_text,
                                )
                                .clicked()
                            {
                                self.selection.selected_core_item = Some((CoreItem::ChainLockedBlock(block.clone(), chain_lock.clone()), *is_valid));
                            }
                        }
                    });
                },
            );

            ui.separator();

            // Middle Column: Instant Send Transactions
            ui.allocate_ui_with_layout(
                egui::Vec2::new(300.0, 1000.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.heading("Instant Send Transactions");

                    ScrollArea::vertical().id_salt("instant_send_scroll").show(ui, |ui| {
                        for (transaction, instant_lock, is_valid) in
                            self.incoming.instant_send_transactions.iter()
                        {
                            let label_text = format!(
                                "{} TxID: {}",
                                if *is_valid { "✔" } else { "❌" },
                                transaction.txid()
                            );

                            if ui
                                .selectable_label(
                                    matches!(self.selection.selected_core_item, Some((CoreItem::InstantLockedTransaction(ref t, _, _), _)) if t == transaction),
                                    label_text,
                                )
                                .clicked()
                            {
                                self.selection.selected_core_item = Some((CoreItem::InstantLockedTransaction(transaction.clone(), vec![], instant_lock.clone()), *is_valid));
                            }
                        }
                    });
                },
            );

            ui.separator();

            // Right Column: Details of the Selected Item
            ui.allocate_ui_with_layout(
                egui::Vec2::new(ui.available_width(), ui.available_height()),
                Layout::top_down(Align::Min),
                |ui| {
                    if let Some((selected_core_item, _)) = &self.selection.selected_core_item {
                        match selected_core_item {
                            CoreItem::ChainLockedBlock(..) => self.render_chain_lock_details(ui),
                            CoreItem::InstantLockedTransaction(..) => self.render_instant_send_details(ui),
                            _ => {
                                ui.label("Select an item to view details.");
                            },
                        }
                    } else {
                        ui.label("Select an item to view details.");
                    }
                },
            );
        });
    }

    /// Render details of a selected ChainLock
    fn render_chain_lock_details(&mut self, ui: &mut Ui) {
        ui.heading("ChainLock Details");

        if let Some((CoreItem::ChainLockedBlock(block, chain_lock), is_valid)) =
            &self.selection.selected_core_item
        {
            ui.label(format!(
                "Block Height: {}\nBlock Hash: {}\nValid: {}",
                chain_lock.block_height,
                chain_lock.block_hash,
                if *is_valid { "✔ Yes" } else { "❌ No" },
            ));

            ui.separator();

            ui.heading("Block Transactions");
            ScrollArea::vertical()
                .id_salt("block_tx_scroll")
                .show(ui, |ui| {
                    if block.txdata.is_empty() {
                        ui.label("No transactions in this block.");
                    } else {
                        for transaction in &block.txdata {
                            ui.label(format!("TxID: {}", transaction.txid()));
                        }
                    }
                });

            ui.separator();
            ui.heading("Quorum Signature");
            ui.label(format!(
                "Signature: {}",
                hex::encode(chain_lock.signature.to_bytes())
            ));

            //todo clean this
            let b = serialize2(chain_lock);
            let chain_lock_2: ChainLock2 = deserialize(b.as_slice()).expect("todo");
            match self
                .data
                .masternode_list_engine
                .chain_lock_potential_quorum_under(&chain_lock_2)
            {
                Ok(Some(quorum)) => {
                    ui.label(format!("Quorum Hash: {}", quorum.quorum_entry.quorum_hash,));
                    ui.label(format!(
                        "Request Id: {}",
                        chain_lock.request_id().expect("expected request id")
                    ));
                    let sign_id = chain_lock_2
                        .sign_id(
                            quorum.quorum_entry.llmq_type,
                            quorum.quorum_entry.quorum_hash,
                            None,
                        )
                        .expect("expected sign id");
                    ui.label(format!("Sign Hash (Sign ID): {}", sign_id));
                    if let Err(e) = quorum
                        .verify_message_digest(sign_id.to_byte_array(), chain_lock_2.signature)
                    {
                        ui.label(format!("Signature Verification Error: {}", e));
                    }
                }
                Ok(None) => {
                    ui.label("No quorum".to_string());
                }
                Err(err) => {
                    ui.label(format!("Error finding quorum: {}", err));
                }
            };

            ui.separator();

            ui.heading("Data");

            ui.label(format!("Block Data {}", hex::encode(serialize2(block)),));

            ui.label(format!("Lock Data {}", hex::encode(serialize2(chain_lock)),));

            ui.separator();
        } else {
            ui.label("No ChainLock selected.");
        }
    }

    /// Render details of a selected Instant Send transaction
    fn render_instant_send_details(&mut self, ui: &mut Ui) {
        ui.heading("Instant Send Details");

        if let Some((CoreItem::InstantLockedTransaction(transaction, _, instant_lock), is_valid)) =
            &self.selection.selected_core_item
        {
            ui.label(format!(
                "TxID: {}\nValid: {}\nCycle Hash:{}",
                transaction.txid(),
                if *is_valid { "✔ Yes" } else { "❌ No" },
                instant_lock.cyclehash,
            ));

            ui.separator();

            ui.heading("Transaction Inputs");
            ScrollArea::vertical()
                .id_salt("tx_inputs_scroll")
                .show(ui, |ui| {
                    if transaction.input.is_empty() {
                        ui.label("No inputs.");
                    } else {
                        for txin in &transaction.input {
                            ui.label(format!(
                                "Input: {}:{}",
                                txin.previous_output.txid, txin.previous_output.vout
                            ));
                        }
                    }
                });

            ui.separator();
            ui.heading("Transaction Outputs");
            ScrollArea::vertical()
                .id_salt("tx_outputs_scroll")
                .show(ui, |ui| {
                    if transaction.output.is_empty() {
                        ui.label("No outputs.");
                    } else {
                        for txout in &transaction.output {
                            ui.label(format!(
                                "Output: {} sat -> {}",
                                txout.value, txout.script_pubkey
                            ));
                        }
                    }
                });

            ui.separator();
            ui.heading("Signing Info");

            //todo clean this
            let b = serialize2(instant_lock);
            let instant_lock_2: InstantLock2 = deserialize(b.as_slice()).expect("todo");
            match self
                .data
                .masternode_list_engine
                .is_lock_quorum(&instant_lock_2)
            {
                Ok((quorum, request_sign_id, index)) => {
                    ui.label(format!(
                        "Quorum Hash: {} at index {}",
                        quorum.quorum_entry.quorum_hash, index,
                    ));
                    ui.label(format!("Request Id: {}", request_sign_id));
                    let sign_id = instant_lock_2
                        .sign_id(
                            quorum.quorum_entry.llmq_type,
                            quorum.quorum_entry.quorum_hash,
                            Some(request_sign_id),
                        )
                        .expect("expected sign id");
                    ui.label(format!("Sign Hash (Sign ID): {}", sign_id));
                    if let Err(e) = quorum
                        .verify_message_digest(sign_id.to_byte_array(), instant_lock_2.signature)
                    {
                        ui.label(format!("Signature Verification Error: {}", e));
                    }
                }
                Err(err) => {
                    ui.label(format!("Error finding quorum: {}", err));
                }
            };

            ui.separator();
            ui.heading("Quorum Signature");
            ui.label(format!(
                "Signature: {}",
                hex::encode(instant_lock.signature.to_bytes())
            ));

            ui.separator();

            ui.heading("Data");

            ui.label(format!(
                "Transaction Data {}",
                hex::encode(serialize2(transaction)),
            ));

            ui.label(format!(
                "Lock Data {}",
                hex::encode(serialize2(instant_lock)),
            ));
        } else {
            ui.label("No Instant Send transaction selected.");
        }
    }

    fn attempt_verify_chain_lock(&self, chain_lock: &ChainLock) -> bool {
        let b = serialize2(chain_lock);
        let chain_lock_2: ChainLock2 = deserialize(b.as_slice()).expect("todo");
        self.data
            .masternode_list_engine
            .verify_chain_lock(&chain_lock_2)
            .is_ok()
    }

    fn attempt_verify_transaction_lock(&self, instant_lock: &InstantLock) -> bool {
        let b = serialize2(instant_lock);
        let instant_lock_2: InstantLock2 = deserialize(b.as_slice()).expect("todo");
        self.data
            .masternode_list_engine
            .verify_is_lock(&instant_lock_2)
            .is_ok()
    }

    fn received_new_block(&mut self, block: Block, chain_lock: ChainLock) {
        let valid = self.attempt_verify_chain_lock(&chain_lock);
        self.input.end_block_height = chain_lock.block_height.to_string();
        if self.task.syncing
            && let Some((base_block_height, masternode_list)) = self
                .data
                .masternode_list_engine
                .masternode_lists
                .last_key_value()
            && *base_block_height < chain_lock.block_height
        {
            let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
                Ok(p2p_handler) => p2p_handler,
                Err(e) => {
                    self.ui_state.error = Some(e);
                    return;
                }
            };

            let Some(qr_info) = self.fetch_rotated_quorum_info(
                &mut p2p_handler,
                masternode_list.block_hash,
                chain_lock.block_hash.to_byte_array().into(),
            ) else {
                return;
            };

            self.feed_qr_info_and_get_dmls(qr_info, Some(p2p_handler));

            // self.fetch_single_dml(
            //     &mut p2p_handler,
            //     masternode_list.block_hash,
            //     *base_block_height,
            //     BlockHash::from_byte_array(chain_lock.block_hash.to_byte_array()),
            //     chain_lock.block_height,
            //     true,
            // );

            // Reset selections when new data is loaded
            self.selection.selected_dml_diff_key = None;
            self.selection.selected_quorum_in_diff_index = None;
        }
        self.incoming
            .chain_locked_blocks
            .insert(chain_lock.block_height, (block, chain_lock, valid));
    }
}

impl ScreenLike for MasternodeListDiffScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.task.pending = None;
            self.ui_state.error = Some(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::CoreItem(core_item) = backend_task_success_result {
            // println!("received core item {:?}", core_item);
            match core_item {
                CoreItem::InstantLockedTransaction(transaction, _, instant_lock) => {
                    let valid = self.attempt_verify_transaction_lock(&instant_lock);
                    self.incoming.instant_send_transactions.push((
                        transaction,
                        instant_lock,
                        valid,
                    ));
                }
                CoreItem::ChainLockedBlock(block, chain_lock) => {
                    self.received_new_block(block, chain_lock);
                }
                _ => {}
            }
            return;
        }
        match backend_task_success_result {
            BackendTaskSuccessResult::MnListFetchedDiff {
                base_height,
                height,
                diff,
            } => {
                // Apply to engine similarly to original UI method
                if base_height == 0 && self.data.masternode_list_engine.masternode_lists.is_empty()
                {
                    match MasternodeListEngine::initialize_with_diff_to_height(
                        diff.clone(),
                        height,
                        self.app_context.network,
                    ) {
                        Ok(engine) => self.data.masternode_list_engine = engine,
                        Err(e) => self.ui_state.error = Some(e.to_string()),
                    }
                } else if let Err(e) = self.data.masternode_list_engine.apply_diff(
                    diff.clone(),
                    Some(height),
                    false,
                    None,
                ) {
                    self.ui_state.error = Some(e.to_string());
                }
                self.data.mnlist_diffs.insert((base_height, height), diff);
                // If this was the no-rotation path, queue the extra diffs needed for verification (restored behavior)
                if matches!(self.task.pending, Some(PendingTask::DmlDiffNoRotation)) {
                    if let Some(task) = self.build_validation_diffs_task() {
                        self.task.queued_task = Some(task);
                        self.display_message(
                            "Fetched DMLs (no rotation); fetching validation diffs…",
                            MessageType::Info,
                        );
                    } else if !self.data.masternode_list_engine.masternode_lists.is_empty() {
                        // Fallback: attempt verification directly
                        if let Err(e) = self
                            .data
                            .masternode_list_engine
                            .verify_non_rotating_masternode_list_quorums(
                                height,
                                &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                            )
                        {
                            self.ui_state.error = Some(e.to_string());
                        }
                        self.task.pending = None;
                        self.display_message("Fetched DMLs (no rotation)", MessageType::Success);
                    } else {
                        self.task.pending = None;
                        self.display_message("Fetched DMLs (no rotation)", MessageType::Success);
                    }
                } else {
                    self.task.pending = None;
                    self.display_message("Fetched DML diff", MessageType::Success);
                }
                self.selection.selected_dml_diff_key = None;
                self.selection.selected_quorum_in_diff_index = None;
            }
            BackendTaskSuccessResult::MnListFetchedQrInfo { qr_info } => {
                // Warm heights and cache diffs before feed_qr_info (replicates old flow)
                self.insert_mn_list_diff(&qr_info.mn_list_diff_tip);
                self.insert_mn_list_diff(&qr_info.mn_list_diff_h);
                self.insert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_c);
                self.insert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_2c);
                self.insert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_3c);
                if let Some((_, d)) = &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c {
                    self.insert_mn_list_diff(d);
                }
                for d in &qr_info.mn_list_diff_list {
                    self.insert_mn_list_diff(d);
                }

                // Apply to engine using the same closure as before to resolve heights
                let block_height_cache = self.cache.block_height_cache.clone();
                let app_context = self.app_context.clone();
                let get_height_fn = move |block_hash: &BlockHash| {
                    if block_hash.as_byte_array() == &[0; 32] {
                        return Ok(0);
                    }
                    if let Some(height) = block_height_cache.get(block_hash) {
                        return Ok(*height);
                    }
                    match app_context
                        .core_client
                        .read()
                        .unwrap()
                        .get_block_header_info(
                            &(BlockHash2::from_byte_array(block_hash.to_byte_array())),
                        ) {
                        Ok(block_info) => Ok(block_info.height as CoreBlockHeight),
                        Err(_) => Err(ClientDataRetrievalError::RequiredBlockNotPresent(
                            *block_hash,
                        )),
                    }
                };
                if let Err(e) = self.data.masternode_list_engine.feed_qr_info(
                    qr_info.clone(),
                    false,
                    true,
                    Some(get_height_fn),
                ) {
                    self.ui_state.error = Some(e.to_string());
                }
                // Store full qr_info for the QR tab
                let key = qr_info.mn_list_diff_tip.block_hash;
                self.data.qr_infos.insert(key, qr_info);
                self.selection.selected_dml_diff_key = None;
                self.selection.selected_quorum_in_diff_index = None;
                // Queue extra diffs required for verification (previous behavior)
                if let Some(task) = self.build_validation_diffs_task() {
                    self.task.queued_task = Some(task);
                    self.display_message(
                        "Fetched QR info + DMLs; fetching validation diffs…",
                        MessageType::Info,
                    );
                } else {
                    self.task.pending = None;
                    self.display_message("Fetched QR info + DMLs", MessageType::Success);
                }
            }
            BackendTaskSuccessResult::MnListFetchedDiffs { items } => {
                // Apply returned diffs sequentially
                for ((base_h, h), diff) in items {
                    if base_h == 0 && self.data.masternode_list_engine.masternode_lists.is_empty() {
                        if let Ok(engine) = MasternodeListEngine::initialize_with_diff_to_height(
                            diff.clone(),
                            h,
                            self.app_context.network,
                        ) {
                            self.data.masternode_list_engine = engine;
                        }
                    } else {
                        let _ = self.data.masternode_list_engine.apply_diff(
                            diff.clone(),
                            Some(h),
                            false,
                            None,
                        );
                    }
                    self.data.mnlist_diffs.insert((base_h, h), diff);
                }
                // Update rotating quorum heights cache (previous behavior)
                let hashes = self
                    .data
                    .masternode_list_engine
                    .latest_masternode_list_rotating_quorum_hashes(&[]);
                for hash in &hashes {
                    if let Ok(height) = self.get_height_and_cache(hash) {
                        self.cache.block_height_cache.insert(*hash, height);
                    }
                }
                // Verify non-rotating quorums as before
                if let Some(latest_masternode_list) =
                    self.data.masternode_list_engine.latest_masternode_list()
                    && let Err(e) = self
                        .data
                        .masternode_list_engine
                        .verify_non_rotating_masternode_list_quorums(
                            latest_masternode_list.known_height,
                            &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                        )
                {
                    self.ui_state.error = Some(e.to_string());
                }
                self.task.pending = None;
                self.display_message(
                    "Fetched validation diffs and verified non-rotating quorums",
                    MessageType::Success,
                );
            }
            BackendTaskSuccessResult::MnListChainLockSigs { entries } => {
                for ((h, bh), sig) in entries {
                    self.cache.chain_lock_sig_cache.insert((h, bh), sig);
                    if let Some(sig) = sig {
                        self.cache
                            .chain_lock_reversed_sig_cache
                            .entry(sig)
                            .or_default()
                            .insert((h, bh));
                    }
                }
                self.task.pending = None;
                self.display_message("Fetched chain lock signatures", MessageType::Success);
            }
            _ => {}
        }
    }

    fn refresh_on_arrival(&mut self) {
        // Optionally refresh data when this screen is shown
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Tools", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenToolsMasternodeListDiffScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        // Styled central panel consistent with other tool screens; scroll only below tab row
        action |= island_central_panel(ctx, |ui| {
            // Top: input area (base/end block height + Get DMLs button)
            let mut inner = AppAction::None;
            inner |= self.render_input_area(ui);
            // If we queued a backend task from a prior result processing, send it now
            if let Some(task) = self.task.queued_task.take() {
                inner |= AppAction::BackendTask(task);
            }

            self.render_error_banner(ui);
            self.render_pending_status(ui);

            ui.separator();

            self.render_selected_tab(ui);
            inner
        });
        action
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingTask {
    DmlDiffSingle,
    DmlDiffNoRotation,
    QrInfo,
    QrInfoWithDmls,
    ChainLocks,
}
