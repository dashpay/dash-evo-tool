mod cache_helpers;
mod core_items_tab;
mod qr_info_tab;
mod quorum_viewer_tab;

use cache_helpers::CacheState;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::mnlist::MnListTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::components::core_p2p_handler::CoreP2PHandler;
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::json::QuorumType;
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::network::constants::NetworkExt;
use dash_sdk::dpp::dashcore::network::message_qrinfo::{QRInfo, QuorumSnapshot};
use dash_sdk::dpp::dashcore::network::message_sml::MnListDiff;
use dash_sdk::dpp::dashcore::sml::llmq_type::LLMQType;
use dash_sdk::dpp::dashcore::sml::masternode_list::MasternodeList;
use dash_sdk::dpp::dashcore::sml::masternode_list_engine::{
    MasternodeListEngine, MasternodeListEngineBlockContainer,
};
use dash_sdk::dpp::dashcore::sml::masternode_list_entry::EntryMasternodeType;
use dash_sdk::dpp::dashcore::sml::masternode_list_entry::qualified_masternode_list_entry::QualifiedMasternodeListEntry;
use dash_sdk::dpp::dashcore::sml::quorum_entry::qualified_quorum_entry::QualifiedQuorumEntry;
use dash_sdk::dpp::dashcore::sml::quorum_validation_error::ClientDataRetrievalError;
use dash_sdk::dpp::dashcore::{
    Block, BlockHash as BlockHash2, ChainLock, InstantLock, Transaction,
};
use dash_sdk::dpp::dashcore::{BlockHash, Network, ProTxHash, QuorumHash};
use dash_sdk::dpp::prelude::CoreBlockHeight;
use eframe::egui::{self, Context, ScrollArea, Ui};
use egui::{Align, Color32, Frame, Layout, Margin, RichText, Stroke, TextEdit, Vec2};
use itertools::Itertools;
use rfd::FileDialog;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

type HeightHash = (u32, BlockHash);

pub(super) enum SelectedQRItem {
    SelectedSnapshot(QuorumSnapshot),
    MNListDiff(Box<MnListDiff>),
    QuorumEntry(Box<QualifiedQuorumEntry>),
}

/// Screen for viewing MNList diffs (diffs in the masternode list and quorums)
pub struct MasternodeListDiffScreen {
    pub app_context: Arc<AppContext>,

    /// Are we syncing?
    syncing: bool,

    /// The chain locked blocks received through zmq that we can attempt to verify
    chain_locked_blocks: BTreeMap<CoreBlockHeight, (Block, ChainLock, bool)>,

    /// Instant send locked transactions received through zmq that we can attempt to verify
    instant_send_transactions: Vec<(Transaction, InstantLock, bool)>,

    /// The user‐entered base block height (as text)
    base_block_height: String,
    /// The user‐entered end block height (as text)
    end_block_height: String,

    show_popup_for_render_masternode_list_engine: bool,

    /// Selected tab (0 = Diffs, 1 = Masternode Lists)
    selected_tab: usize,

    /// The engine to compute masternode lists
    masternode_list_engine: MasternodeListEngine,

    /// Masternode_list_heights with all quorum heights known
    masternode_lists_with_all_quorum_heights_known: BTreeSet<CoreBlockHeight>,

    /// The list of MNList diff items (one per block height)
    mnlist_diffs: BTreeMap<(CoreBlockHeight, CoreBlockHeight), MnListDiff>,

    /// The list of qr infos
    qr_infos: BTreeMap<BlockHash, QRInfo>,

    /// Selected MNList diff
    selected_dml_diff_key: Option<(CoreBlockHeight, CoreBlockHeight)>,

    /// This is to know which ones we have already checked for quorum heights
    dml_diffs_with_cached_quorum_heights: HashSet<(CoreBlockHeight, CoreBlockHeight)>,

    /// Selected MNList
    selected_dml_height_key: Option<CoreBlockHeight>,

    /// Selected display option
    selected_option_index: Option<usize>,
    /// Selected quorum within the MNList diff
    selected_quorum_in_diff_index: Option<usize>,

    /// Selected masternode within the MNList diff
    selected_masternode_in_diff_index: Option<usize>,

    /// Selected quorum within the MNList diff
    selected_quorum_hash_in_mnlist_diff: Option<(LLMQType, QuorumHash)>,

    /// Selected quorum within the quorum_viewer
    selected_quorum_type_in_quorum_viewer: Option<LLMQType>,

    /// Selected quorum within the quorum_viewer
    selected_quorum_hash_in_quorum_viewer: Option<QuorumHash>,

    /// Selected masternode within the MNList diff
    selected_masternode_pro_tx_hash: Option<ProTxHash>,

    /// Search term
    search_term: Option<String>,

    /// Block height, hash, and chain lock signature caches
    cache: CacheState,

    error: Option<String>,
    selected_qr_field: Option<String>,
    selected_qr_list_index: Option<String>,
    selected_core_item: Option<(CoreItem, bool)>,
    selected_qr_item: Option<SelectedQRItem>,
    pending: Option<PendingTask>,
    queued_task: Option<BackendTask>,
    message: Option<(String, MessageType)>,
}

impl MasternodeListDiffScreen {
    /// Create a new MNListDiffScreen
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let mut mnlist_diffs = BTreeMap::new();
        let engine = match app_context.network {
            Network::Dash => {
                use std::env;
                println!(
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
                            eprintln!("Failed to read MNListDiff file: {}", e);
                            MasternodeListEngine::default_for_network(Network::Dash)
                        }
                    }
                } else {
                    eprintln!("MNListDiff file not found: {}", file_path);
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
                            eprintln!("Failed to read MNListDiff file: {}", e);
                            MasternodeListEngine::default_for_network(Network::Testnet)
                        }
                    }
                } else {
                    eprintln!("MNListDiff file not found: {}", file_path);
                    MasternodeListEngine::default_for_network(Network::Dash)
                }
            }
            _ => MasternodeListEngine::default_for_network(app_context.network),
        };

        Self {
            app_context: app_context.clone(),
            syncing: false,
            chain_locked_blocks: Default::default(),
            instant_send_transactions: vec![],
            base_block_height: "".to_string(),
            end_block_height: "".to_string(),
            show_popup_for_render_masternode_list_engine: false,
            selected_tab: 0,
            masternode_list_engine: engine,
            search_term: None,
            mnlist_diffs,
            qr_infos: Default::default(),
            selected_dml_diff_key: None,
            dml_diffs_with_cached_quorum_heights: Default::default(),
            selected_dml_height_key: None,
            selected_option_index: None,
            selected_quorum_in_diff_index: None,
            selected_masternode_in_diff_index: None,
            selected_quorum_hash_in_mnlist_diff: None,
            selected_quorum_type_in_quorum_viewer: None,
            selected_quorum_hash_in_quorum_viewer: None,
            selected_masternode_pro_tx_hash: None,
            error: None,
            selected_qr_field: None,
            selected_qr_list_index: None,
            cache: CacheState::default(),
            selected_qr_item: None,
            selected_core_item: None,
            masternode_lists_with_all_quorum_heights_known: Default::default(),
            pending: None,
            queued_task: None,
            message: None,
        }
    }

    /// Build a backend task that fetches the extra diffs needed to validate non-rotating quorums.
    /// Returns None if requirements cannot be computed.
    fn build_validation_diffs_task(&mut self) -> Option<BackendTask> {
        // Determine hashes we need to validate
        let hashes = self
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
            if let Ok(h) = self.cache.get_height_and_cache(
                quorum_hash,
                &mut self.masternode_list_engine,
                &self.app_context,
            ) && h >= 8
            {
                heights.insert(h - 8);
            }
        }
        if heights.is_empty() {
            return None;
        }

        let client = self.app_context.core_client.read_or_recover();
        let mut chain: Vec<(u32, BlockHash, u32, BlockHash)> = Vec::new();

        // Determine base starting point similar to previous logic
        let (first_engine_height, first_engine_hash_opt) = self
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

    fn parse_heights(&mut self) -> Result<(HeightHash, HeightHash), String> {
        let base = if self.base_block_height.is_empty() {
            self.base_block_height = "0".to_string();
            match self
                .app_context
                .core_client
                .read_or_recover()
                .get_block_hash(0)
            {
                Ok(block_hash) => (0, BlockHash::from_byte_array(block_hash.to_byte_array())),
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        } else {
            match self.base_block_height.trim().parse() {
                Ok(start) => match self
                    .app_context
                    .core_client
                    .read_or_recover()
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
        let end = if self.end_block_height.is_empty() {
            match self
                .app_context
                .core_client
                .read_or_recover()
                .get_best_block_hash()
            {
                Ok(block_hash) => {
                    match self
                        .app_context
                        .core_client
                        .read_or_recover()
                        .get_block_header_info(&block_hash)
                    {
                        Ok(header) => {
                            self.end_block_height = format!("{}", header.height);
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
            match self.end_block_height.trim().parse() {
                Ok(end) => match self
                    .app_context
                    .core_client
                    .read_or_recover()
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
        match bincode::encode_to_vec(&self.masternode_list_engine, bincode::config::standard()) {
            Ok(encoded_bytes) => Ok(hex::encode(encoded_bytes)), // Convert to hex string
            Err(e) => Err(format!("Serialization failed: {}", e)),
        }
    }

    fn insert_mn_list_diff(&mut self, mn_list_diff: &MnListDiff) {
        let base_block_hash = mn_list_diff.base_block_hash;
        let base_height = match self.cache.get_height_and_cache(
            &base_block_hash,
            &mut self.masternode_list_engine,
            &self.app_context,
        ) {
            Ok(height) => height,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let block_hash = mn_list_diff.block_hash;
        let height = match self.cache.get_height_and_cache(
            &block_hash,
            &mut self.masternode_list_engine,
            &self.app_context,
        ) {
            Ok(height) => height,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        self.mnlist_diffs
            .insert((base_height, height), mn_list_diff.clone());
    }

    fn fetch_rotated_quorum_info(
        &mut self,
        p2p_handler: &mut CoreP2PHandler,
        base_block_hash: BlockHash,
        block_hash: BlockHash,
    ) -> Option<QRInfo> {
        let mut known_block_hashes: Vec<_> = self
            .mnlist_diffs
            .values()
            .map(|mn_list_diff| mn_list_diff.block_hash)
            .collect();
        known_block_hashes.push(base_block_hash);
        println!(
            "requesting with known_block_hashes {}",
            known_block_hashes
                .iter()
                .map(|bh| bh.to_string())
                .join(", ")
        );
        let qr_info = match p2p_handler.get_qr_info(known_block_hashes, block_hash) {
            Ok(list_diff) => list_diff,
            Err(e) => {
                self.error = Some(e);
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
        self.qr_infos.insert(block_hash, qr_info.clone());
        Some(qr_info)
    }

    fn fetch_diffs_with_hashes(
        &mut self,
        p2p_handler: &mut CoreP2PHandler,
        hashes: BTreeSet<QuorumHash>,
    ) {
        let mut hashes_needed_to_validate = BTreeMap::new();
        for quorum_hash in hashes {
            let height = match self.cache.get_height_and_cache(
                &quorum_hash,
                &mut self.masternode_list_engine,
                &self.app_context,
            ) {
                Ok(height) => height,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return;
                }
            };
            let validation_hash = match self
                .app_context
                .core_client
                .read_or_recover()
                .get_block_hash(height - 8)
            {
                Ok(block_hash) => block_hash,
                Err(e) => {
                    self.error = Some(e.to_string());
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
                    .masternode_list_engine
                    .network
                    .known_genesis_block_hash()
                {
                    None => match self
                        .app_context
                        .core_client
                        .read_or_recover()
                        .get_block_hash(0)
                    {
                        Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
                        Err(e) => {
                            self.error = Some(e.to_string());
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
                self.error = Some(e);
                return;
            }
        };

        if base_block_height == 0 && self.masternode_list_engine.masternode_lists.is_empty() {
            self.masternode_list_engine = match MasternodeListEngine::initialize_with_diff_to_height(
                list_diff.clone(),
                block_height,
                self.app_context.network,
            ) {
                Ok(masternode_list_engine) => masternode_list_engine,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return;
                }
            }
        } else if let Err(e) = self.masternode_list_engine.apply_diff(
            list_diff.clone(),
            Some(block_height),
            false,
            None,
        ) {
            self.error = Some(e.to_string());
            return;
        }

        if validate_quorums && !self.masternode_list_engine.masternode_lists.is_empty() {
            let hashes = self
                .masternode_list_engine
                .latest_masternode_list_non_rotating_quorum_hashes(
                    &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                    true,
                );
            self.fetch_diffs_with_hashes(p2p_handler, hashes);
            let hashes = self
                .masternode_list_engine
                .latest_masternode_list_rotating_quorum_hashes(&[]);
            for hash in &hashes {
                let height = match self.cache.get_height_and_cache(
                    hash,
                    &mut self.masternode_list_engine,
                    &self.app_context,
                ) {
                    Ok(height) => height,
                    Err(e) => {
                        self.error = Some(e.to_string());
                        return;
                    }
                };
                self.cache.block_height_cache.insert(*hash, height);
            }

            if let Err(e) = self
                .masternode_list_engine
                .verify_non_rotating_masternode_list_quorums(
                    block_height,
                    &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                )
            {
                self.error = Some(e.to_string());
            }
        }

        self.mnlist_diffs
            .insert((base_block_height, block_height), list_diff);
    }

    /// Clear all data and reset to initial state
    pub(crate) fn clear(&mut self) {
        self.masternode_list_engine =
            MasternodeListEngine::default_for_network(self.app_context.network);

        // Clear cached data structures
        self.mnlist_diffs.clear();
        self.qr_infos.clear();
        self.chain_locked_blocks.clear();
        self.instant_send_transactions.clear();
        self.cache.clear();
        self.masternode_lists_with_all_quorum_heights_known.clear();
        self.dml_diffs_with_cached_quorum_heights.clear();

        // Reset selections and UI state
        self.selected_dml_diff_key = None;
        self.selected_dml_height_key = None;
        self.selected_option_index = None;
        self.selected_quorum_in_diff_index = None;
        self.selected_masternode_in_diff_index = None;
        self.selected_quorum_hash_in_mnlist_diff = None;
        self.selected_masternode_pro_tx_hash = None;
        self.selected_qr_item = None;
        self.selected_core_item = None;
        self.pending = None;
        self.queued_task = None;
        self.search_term = None;
        self.error = None;
        self.message = None;
    }

    /// Clear all data except the oldest MNList diff starting from height 0
    fn clear_keep_base(&mut self) {
        let (engine, start_end_diff) =
            if let Some(((start, end), oldest_diff)) = self.mnlist_diffs.first_key_value() {
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

        self.masternode_list_engine = engine;
        self.mnlist_diffs = Default::default();
        if let Some((key, oldest_diff)) = start_end_diff {
            self.mnlist_diffs.insert(key, oldest_diff);
        }
        self.selected_dml_diff_key = None;
        self.selected_dml_height_key = None;
        self.selected_option_index = None;
        self.selected_quorum_in_diff_index = None;
        self.selected_masternode_in_diff_index = None;
        self.selected_quorum_hash_in_mnlist_diff = None;
        self.selected_masternode_pro_tx_hash = None;
        self.qr_infos = Default::default();
        self.message = None;
        // Clear chain lock signature caches as these are independent of the retained base diff
        self.cache.clear_chain_lock_caches();
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
                    self.error = Some(e);
                    return;
                }
            };

        let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
            Ok(p2p_handler) => p2p_handler,
            Err(e) => {
                self.error = Some(e);
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
        self.selected_dml_diff_key = None;
        self.selected_quorum_in_diff_index = None;
    }

    #[allow(dead_code)]
    fn fetch_end_qr_info(&mut self) {
        let ((_, base_block_hash), (_, block_hash)) = match self.parse_heights() {
            Ok(a) => a,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
            Ok(p2p_handler) => p2p_handler,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        self.fetch_rotated_quorum_info(&mut p2p_handler, base_block_hash, block_hash);

        // Reset selections when new data is loaded
        self.selected_dml_diff_key = None;
        self.selected_quorum_in_diff_index = None;
    }

    #[allow(dead_code)]
    fn fetch_chain_locks(&mut self) {
        let ((base_block_height, _base_block_hash), (block_height, _block_hash)) =
            match self.parse_heights() {
                Ok(a) => a,
                Err(e) => {
                    self.error = Some(e);
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
            if let Ok(block_hash) = self.cache.get_block_hash_and_cache(
                i,
                &self.masternode_list_engine,
                &self.app_context,
            ) {
                self.cache
                    .get_chain_lock_sig_and_cache(
                        &block_hash,
                        &mut self.masternode_list_engine,
                        &self.app_context,
                    )
                    .ok();
            }
        }
    }

    #[allow(dead_code)]
    fn sync(&mut self) {
        if !self.syncing {
            self.syncing = true;
            self.fetch_end_qr_info_with_dmls();
        }
    }

    #[allow(dead_code)]
    fn fetch_end_qr_info_with_dmls(&mut self) {
        let ((_, base_block_hash), (_, block_hash)) = match self.parse_heights() {
            Ok(a) => a,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
            Ok(p2p_handler) => p2p_handler,
            Err(e) => {
                self.error = Some(e);
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
                    self.error = Some(e);
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
                    .read_or_recover()
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
            self.masternode_list_engine
                .feed_qr_info(qr_info, false, true, Some(get_height_fn))
        {
            self.error = Some(e.to_string());
            return;
        }

        let hashes = self
            .masternode_list_engine
            .latest_masternode_list_non_rotating_quorum_hashes(
                &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                true,
            );
        self.fetch_diffs_with_hashes(&mut p2p_handler, hashes);
        let hashes = self
            .masternode_list_engine
            .latest_masternode_list_rotating_quorum_hashes(&[]);
        for hash in &hashes {
            let height = match self.cache.get_height_and_cache(
                hash,
                &mut self.masternode_list_engine,
                &self.app_context,
            ) {
                Ok(height) => height,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return;
                }
            };
            self.cache.block_height_cache.insert(*hash, height);
        }

        if let Some(latest_masternode_list) = self.masternode_list_engine.latest_masternode_list()
            && let Err(e) = self
                .masternode_list_engine
                .verify_non_rotating_masternode_list_quorums(
                    latest_masternode_list.known_height,
                    &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                )
        {
            self.error = Some(e.to_string());
        }

        // Reset selections when new data is loaded
        self.selected_dml_diff_key = None;
        self.selected_quorum_in_diff_index = None;
    }

    /// Render the input area at the top (base and end block height fields plus Get DMLs button)
    fn render_input_area(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ScrollArea::horizontal()
            .id_salt("dml_input_row_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Base Block Height:");
                    ui.add(TextEdit::singleline(&mut self.base_block_height).desired_width(80.0));
                    ui.label("End Block Height:");
                    ui.add(TextEdit::singleline(&mut self.end_block_height).desired_width(80.0));
                    if ui.button("Get single end DML diff").clicked()
                        && let Ok(((base_h, base_hash), (h, hash))) = self.parse_heights()
                    {
                        self.pending = Some(PendingTask::DmlDiffSingle);
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
                        self.pending = Some(PendingTask::QrInfo);
                        // Build known_block_hashes from current diffs + base hash (old UI behavior)
                        let mut known_block_hashes: Vec<_> = self
                            .mnlist_diffs
                            .values()
                            .map(|mn_list_diff| mn_list_diff.block_hash)
                            .collect();
                        known_block_hashes.push(base_hash);
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
                        self.pending = Some(PendingTask::DmlDiffNoRotation);
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
                        self.pending = Some(PendingTask::QrInfoWithDmls);
                        // Build known_block_hashes from current diffs + base hash (old UI behavior)
                        let mut known_block_hashes: Vec<_> = self
                            .mnlist_diffs
                            .values()
                            .map(|mn_list_diff| mn_list_diff.block_hash)
                            .collect();
                        known_block_hashes.push(base_hash);
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
                        self.pending = Some(PendingTask::QrInfoWithDmls);
                        // Build known_block_hashes from current diffs + base hash (old UI behavior)
                        let mut known_block_hashes: Vec<_> = self
                            .mnlist_diffs
                            .values()
                            .map(|mn_list_diff| mn_list_diff.block_hash)
                            .collect();
                        known_block_hashes.push(base_hash);
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
                        self.pending = Some(PendingTask::ChainLocks);
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
                            self.masternode_list_engine = engine;
                        }
                        Err(e) => {
                            eprintln!("Failed to decode QRInfo: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to read file: {:?}", e);
                }
            }
        }
    }

    fn save_masternode_list_engine(&mut self) {
        // Serialize the masternode list engine
        let serialized = match self.serialize_masternode_list_engine() {
            Ok(serialized) => serialized,
            Err(e) => {
                self.error = Some(format!("Serialization failed: {}", e));
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
                    println!("Masternode list engine saved to {:?}", path);
                }
                Err(e) => {
                    self.error = Some(format!("Failed to save file: {}", e));
                }
            }
        }
    }

    fn render_masternode_lists(&mut self, ui: &mut Ui) {
        ui.heading("Masternode lists");
        ScrollArea::vertical()
            .id_salt("dml_list_scroll_area")
            .show(ui, |ui| {
                for height in self.masternode_list_engine.masternode_lists.keys() {
                    let height_label = format!("{}", height);

                    if ui
                        .selectable_label(
                            self.selected_dml_height_key == Some(*height),
                            height_label,
                        )
                        .clicked()
                    {
                        self.selected_dml_diff_key = None;
                        self.selected_dml_height_key = Some(*height);
                        self.selected_quorum_in_diff_index = None;
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
                for (key, _dml) in self.mnlist_diffs.iter() {
                    let block_label = format!("Base: {} -> Block: {}", key.0, key.1);

                    if ui
                        .selectable_label(self.selected_dml_diff_key == Some(*key), block_label)
                        .clicked()
                    {
                        self.selected_dml_diff_key = Some(*key);
                        self.selected_dml_height_key = None;
                        self.selected_quorum_in_diff_index = None;
                    }
                }
            });
    }

    /// Render the list of quorums for the selected DML
    fn render_new_quorums(&mut self, ui: &mut Ui) {
        ui.heading("New Quorums");

        let should_get_heights = if let Some(selected_key) = self.selected_dml_diff_key {
            if self.mnlist_diffs.contains_key(&selected_key) {
                !self
                    .dml_diffs_with_cached_quorum_heights
                    .contains(&selected_key)
            } else {
                false
            }
        } else {
            false
        };

        let heights = if should_get_heights {
            if let Some(selected_key) = self.selected_dml_diff_key {
                if let Some(quorums) = self
                    .mnlist_diffs
                    .get(&selected_key)
                    .map(|dml| dml.new_quorums.clone())
                {
                    let mut map = HashMap::new();
                    for quorum in quorums {
                        let height = self
                            .cache
                            .get_height_and_cache(
                                &quorum.quorum_hash,
                                &mut self.masternode_list_engine,
                                &self.app_context,
                            )
                            .ok()
                            .unwrap_or_default();
                        map.insert(quorum.quorum_hash, height);
                    }
                    map
                } else {
                    HashMap::new()
                }
            } else {
                HashMap::new()
            }
        } else if let Some(selected_key) = self.selected_dml_diff_key {
            if let Some(quorums) = self
                .mnlist_diffs
                .get(&selected_key)
                .map(|dml| dml.new_quorums.clone())
            {
                let mut map = HashMap::new();
                for quorum in quorums {
                    let height = self
                        .cache
                        .get_height(
                            &quorum.quorum_hash,
                            &self.masternode_list_engine,
                            &self.app_context,
                        )
                        .ok()
                        .unwrap_or_default();
                    map.insert(quorum.quorum_hash, height);
                }
                map
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let new_quorums = self
            .selected_dml_diff_key
            .and_then(|selected_key| self.mnlist_diffs.get(&selected_key))
            .map(|diff| &diff.new_quorums);

        if let Some(new_quorums) = new_quorums {
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
                                self.selected_quorum_in_diff_index == Some(q_index),
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
                            self.selected_quorum_in_diff_index = Some(q_index);
                            self.selected_masternode_in_diff_index = None;
                        }
                    }
                });
        } else {
            ui.label("Select a block height to show quorums.");
        }
    }

    fn render_selected_masternode_list_items(&mut self, ui: &mut Ui) {
        ui.heading("Masternode List Explorer");

        // Define available options for selection
        let options = ["Quorums", "Masternodes"];
        let selected_index = self.selected_option_index.unwrap_or(0);

        // Render the selection buttons
        ui.horizontal(|ui| {
            for (index, option) in options.iter().enumerate() {
                if ui
                    .selectable_label(selected_index == index, *option)
                    .clicked()
                {
                    self.selected_option_index = Some(index);
                }
            }
        });

        ui.separator();

        // Borrow mn_list separately to avoid multiple borrows of `self`
        if self.selected_dml_height_key.is_some() {
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

    /// Filter masternodes based on the search term
    fn filter_masternodes(
        &self,
        mn_list: &MasternodeList,
    ) -> BTreeMap<ProTxHash, QualifiedMasternodeListEntry> {
        // If no search term, return all masternodes
        if let Some(search_term) = &self.search_term {
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
            let mut search_term = self.search_term.clone().unwrap_or_default();
            let response = ui.add(TextEdit::singleline(&mut search_term).desired_width(200.0));

            if response.changed() {
                self.search_term = if search_term.trim().is_empty() {
                    None
                } else {
                    Some(search_term)
                };
            }
        });
    }

    fn render_masternodes_in_masternode_list(&mut self, ui: &mut Ui) {
        if let Some(selected_height) = self.selected_dml_height_key
            && self
                .masternode_list_engine
                .masternode_lists
                .contains_key(&selected_height)
        {
            ui.heading("Masternodes in List");
            self.render_search_bar(ui);
        }
        if let Some(selected_height) = self.selected_dml_height_key
            && let Some(mn_list) = self
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
        {
            let filtered_masternodes = self.filter_masternodes(mn_list);
            ScrollArea::vertical()
                .id_salt("masternode_list_scroll_area")
                .show(ui, |ui| {
                    for (pro_tx_hash, masternode) in filtered_masternodes.iter() {
                        if ui
                            .selectable_label(
                                self.selected_masternode_pro_tx_hash == Some(*pro_tx_hash),
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
                            self.selected_quorum_hash_in_mnlist_diff = None;
                            self.selected_masternode_pro_tx_hash = Some(*pro_tx_hash);
                        }
                    }
                });
        }
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
                        if self.selected_quorum_hash_in_mnlist_diff.is_some() {
                            self.render_quorum_details(ui);
                        } else if self.selected_masternode_pro_tx_hash.is_some() {
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

        if self.syncing {
            tabs.push("Stop Syncing");
        }

        // Render the selection buttons (scrollable horizontally) styled as buttons
        ScrollArea::horizontal()
            .id_salt("dml_tabs_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, tab) in tabs.iter().enumerate() {
                        let is_selected = self.selected_tab == index;
                        if is_selected {
                            // Match the selected look used under "Masternode List Explorer"
                            let _ = ui.selectable_label(true, *tab);
                        } else if ui.button(*tab).clicked() {
                            match index {
                                7 => {
                                    // Show the popup when "Masternode List Engine" is selected
                                    self.show_popup_for_render_masternode_list_engine = true;
                                }
                                8 => {
                                    self.load_masternode_list_engine();
                                }
                                9 => {
                                    self.syncing = false;
                                }
                                index => self.selected_tab = index,
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
        if self.selected_tab == 0 {
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
                .show(ui, |ui| match self.selected_tab {
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
        if self.show_popup_for_render_masternode_list_engine {
            egui::Window::new("Confirmation")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label("This operation will take about 10 seconds. Are you sure you wish to continue?");

                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            self.save_masternode_list_engine();
                            self.show_popup_for_render_masternode_list_engine = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_popup_for_render_masternode_list_engine = false;
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
                        &self.masternode_list_engine.block_container,
                        bincode::config::standard(),
                    )
                    .expect("serialize container");
                    if let Err(e) = std::fs::write(&path, serialized_data) {
                        eprintln!("Failed to write file: {}", e);
                    }
                }
            }
        });

        ScrollArea::vertical()
            .id_salt("known_blocks_scroll")
            .show(ui, |ui| {
                ui.label(format!(
                    "Total Known Blocks: {}",
                    self.masternode_list_engine
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
                            &self.masternode_list_engine.block_container;

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
                    let serialized_data =
                        bincode::encode_to_vec(&self.mnlist_diffs, bincode::config::standard())
                            .expect("serialize container");
                    if let Err(e) = std::fs::write(&path, serialized_data) {
                        eprintln!("Failed to write file: {}", e);
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
                    if self.selected_quorum_in_diff_index.is_some() {
                        self.render_quorum_details(ui);
                    } else if self.selected_masternode_in_diff_index.is_some() {
                        self.render_mn_details(ui);
                    }
                },
            );
        });
    }

    fn render_masternode_changes(&mut self, ui: &mut Ui) {
        ui.heading("Masternode changes");
        if let Some(selected_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&selected_key) {
                ScrollArea::vertical()
                    .id_salt("quorum_list_scroll_area")
                    .show(ui, |ui| {
                        for (m_index, masternode) in dml.new_masternodes.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    self.selected_masternode_in_diff_index == Some(m_index),
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
                                self.selected_quorum_in_diff_index = None;
                                self.selected_masternode_in_diff_index = Some(m_index);
                            }
                        }
                    });
            }
        } else {
            ui.label("Select a block height to show quorums.");
        }
    }

    fn render_mn_diff_chain_locks(&mut self, ui: &mut Ui) {
        ui.heading("MN list diff chain locks");
        if let Some(selected_key) = self.selected_dml_diff_key
            && let Some(dml) = self.mnlist_diffs.get(&selected_key)
        {
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
    }

    fn save_mn_list_diff(&mut self) {
        let Some(selected_key) = self.selected_dml_diff_key else {
            self.error = Some("No MNListDiff selected.".to_string());
            return;
        };

        let Some(mn_list_diff) = self.mnlist_diffs.get(&selected_key) else {
            self.error = Some("Failed to retrieve selected MNListDiff.".to_string());
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
                    println!("MNListDiff saved to {:?}", path);
                }
                Err(e) => {
                    self.error = Some(format!("Failed to save file: {}", e));
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
        let selected_index = self.selected_option_index.unwrap_or(0);

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
                        self.selected_option_index = Some(index);
                    }
                }
            }
        });

        ui.separator();

        // Determine the selected category and display corresponding information
        if let Some(selected_key) = self.selected_dml_diff_key {
            if self.mnlist_diffs.contains_key(&selected_key) {
                ScrollArea::vertical()
                    .id_salt("dml_items_scroll_area")
                    .show(ui, |ui| match selected_index {
                        0 => self.render_new_quorums(ui),
                        1 => self.render_masternode_changes(ui),
                        2 => self.render_mn_diff_chain_locks(ui),
                        _ => (),
                    });
            }
        } else {
            ui.label("Select a block height to show details.");
        }
    }

    /// Render the details for the selected Masternode
    fn render_mn_details(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let border = DashColors::border(dark_mode);
        ui.heading("Masternode Details");

        if let Some(dml_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&dml_key) {
                if let Some(mn_index) = self.selected_masternode_in_diff_index {
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
        } else if let Some(selected_height) = self.selected_dml_height_key {
            if let Some(mn_list) = self
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
                && let Some(selected_pro_tx_hash) = self.selected_masternode_pro_tx_hash
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
}

impl ScreenLike for MasternodeListDiffScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Error => {
                self.pending = None;
                self.error = Some(message.to_string());
            }
            MessageType::Success => {
                self.message = Some((message.to_string(), message_type));
            }
            MessageType::Info => {
                // Do not show transient info messages to avoid noisy black text banners.
            }
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::CoreItem(core_item) = backend_task_success_result {
            // println!("received core item {:?}", core_item);
            match core_item {
                CoreItem::InstantLockedTransaction(transaction, _, instant_lock) => {
                    let valid = self.attempt_verify_transaction_lock(&instant_lock);
                    self.instant_send_transactions
                        .push((transaction, instant_lock, valid));
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
                if base_height == 0 && self.masternode_list_engine.masternode_lists.is_empty() {
                    match MasternodeListEngine::initialize_with_diff_to_height(
                        diff.clone(),
                        height,
                        self.app_context.network,
                    ) {
                        Ok(engine) => self.masternode_list_engine = engine,
                        Err(e) => self.error = Some(e.to_string()),
                    }
                } else if let Err(e) =
                    self.masternode_list_engine
                        .apply_diff(diff.clone(), Some(height), false, None)
                {
                    self.error = Some(e.to_string());
                }
                self.mnlist_diffs.insert((base_height, height), diff);
                // If this was the no-rotation path, queue the extra diffs needed for verification (restored behavior)
                if matches!(self.pending, Some(PendingTask::DmlDiffNoRotation)) {
                    if let Some(task) = self.build_validation_diffs_task() {
                        self.queued_task = Some(task);
                        self.display_message(
                            "Fetched DMLs (no rotation); fetching validation diffs…",
                            MessageType::Info,
                        );
                    } else if !self.masternode_list_engine.masternode_lists.is_empty() {
                        // Fallback: attempt verification directly
                        if let Err(e) = self
                            .masternode_list_engine
                            .verify_non_rotating_masternode_list_quorums(
                                height,
                                &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                            )
                        {
                            self.error = Some(e.to_string());
                        }
                        self.pending = None;
                        self.display_message("Fetched DMLs (no rotation)", MessageType::Success);
                    } else {
                        self.pending = None;
                        self.display_message("Fetched DMLs (no rotation)", MessageType::Success);
                    }
                } else {
                    self.pending = None;
                    self.display_message("Fetched DML diff", MessageType::Success);
                }
                self.selected_dml_diff_key = None;
                self.selected_quorum_in_diff_index = None;
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
                        .read_or_recover()
                        .get_block_header_info(
                            &(BlockHash2::from_byte_array(block_hash.to_byte_array())),
                        ) {
                        Ok(block_info) => Ok(block_info.height as CoreBlockHeight),
                        Err(_) => Err(ClientDataRetrievalError::RequiredBlockNotPresent(
                            *block_hash,
                        )),
                    }
                };
                if let Err(e) = self.masternode_list_engine.feed_qr_info(
                    qr_info.clone(),
                    false,
                    true,
                    Some(get_height_fn),
                ) {
                    self.error = Some(e.to_string());
                }
                // Store full qr_info for the QR tab
                let key = qr_info.mn_list_diff_tip.block_hash;
                self.qr_infos.insert(key, qr_info);
                self.selected_dml_diff_key = None;
                self.selected_quorum_in_diff_index = None;
                // Queue extra diffs required for verification (previous behavior)
                if let Some(task) = self.build_validation_diffs_task() {
                    self.queued_task = Some(task);
                    self.display_message(
                        "Fetched QR info + DMLs; fetching validation diffs…",
                        MessageType::Info,
                    );
                } else {
                    self.pending = None;
                    self.display_message("Fetched QR info + DMLs", MessageType::Success);
                }
            }
            BackendTaskSuccessResult::MnListFetchedDiffs { items } => {
                // Apply returned diffs sequentially
                for ((base_h, h), diff) in items {
                    if base_h == 0 && self.masternode_list_engine.masternode_lists.is_empty() {
                        if let Ok(engine) = MasternodeListEngine::initialize_with_diff_to_height(
                            diff.clone(),
                            h,
                            self.app_context.network,
                        ) {
                            self.masternode_list_engine = engine;
                        }
                    } else {
                        let _ = self.masternode_list_engine.apply_diff(
                            diff.clone(),
                            Some(h),
                            false,
                            None,
                        );
                    }
                    self.mnlist_diffs.insert((base_h, h), diff);
                }
                // Update rotating quorum heights cache (previous behavior)
                let hashes = self
                    .masternode_list_engine
                    .latest_masternode_list_rotating_quorum_hashes(&[]);
                for hash in &hashes {
                    if let Ok(height) = self.cache.get_height_and_cache(
                        hash,
                        &mut self.masternode_list_engine,
                        &self.app_context,
                    ) {
                        self.cache.block_height_cache.insert(*hash, height);
                    }
                }
                // Verify non-rotating quorums as before
                if let Some(latest_masternode_list) =
                    self.masternode_list_engine.latest_masternode_list()
                    && let Err(e) = self
                        .masternode_list_engine
                        .verify_non_rotating_masternode_list_quorums(
                            latest_masternode_list.known_height,
                            &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85],
                        )
                {
                    self.error = Some(e.to_string());
                }
                self.pending = None;
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
                self.pending = None;
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
            if let Some(task) = self.queued_task.take() {
                inner |= AppAction::BackendTask(task);
            }

            if let Some((msg, msg_type)) = self.message.clone() {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let message_color = match msg_type {
                    MessageType::Error => Color32::from_rgb(255, 100, 100),
                    MessageType::Info => crate::ui::theme::DashColors::text_primary(dark_mode),
                    // Dark green for success text
                    MessageType::Success => Color32::DARK_GREEN,
                };
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(message_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, message_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(msg).color(message_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.message = None;
                                }
                            });
                        });
                });
                ui.add_space(10.0);
            }

            if let Some(error_msg) = self.error.clone() {
                let message_color = Color32::from_rgb(255, 100, 100);
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
                                    self.error = None;
                                }
                            });
                        });
                });
                ui.add_space(10.0);
            }

            // Pending spinner (Dash Blue spinner, black text)
            if let Some(p) = self.pending {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.scope(|ui| {
                        let style = ui.style_mut();
                        // Force spinner (fg stroke) to Dash Blue
                        style.visuals.widgets.inactive.fg_stroke.color =
                            crate::ui::theme::DashColors::DASH_BLUE;
                        style.visuals.widgets.active.fg_stroke.color =
                            crate::ui::theme::DashColors::DASH_BLUE;
                        style.visuals.widgets.hovered.fg_stroke.color =
                            crate::ui::theme::DashColors::DASH_BLUE;
                        ui.add(egui::Spinner::new());
                    });
                    let label = match p {
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
