use crate::app::AppAction;
use crate::components::core_p2p_handler::CoreP2PHandler;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::json::QuorumType;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::prelude::CoreBlockHeight;
use dashcoretemp::hashes::Hash as tempHash;
use dashcoretemp::network::message_sml::MnListDiff;
use dashcoretemp::sml::masternode_list_engine::MasternodeListEngine;
use dashcoretemp::sml::masternode_list_entry::MasternodeType;
use dashcoretemp::{BlockHash, Network};
use eframe::egui::{self, Context, ScrollArea, Ui};
use egui::{Align, Color32, Frame, Layout, Stroke, TextEdit, Vec2};
use std::collections::BTreeMap;
use std::sync::Arc;
use dash_sdk::dpp::dashcore::BlockHash as BlockHash2;
use dashcoretemp::sml::llmq_type::LLMQType;

/// Screen for viewing MNList diffs (diffs in the masternode list and quorums)
pub struct MasternodeListDiffScreen {
    pub app_context: Arc<AppContext>,

    /// The user‐entered base block height (as text)
    base_block_height: String,
    /// The user‐entered end block height (as text)
    end_block_height: String,

    /// Selected tab (0 = Diffs, 1 = Masternode Lists)
    selected_tab: usize,

    /// The engine to compute masternode lists
    masternode_list_engine: MasternodeListEngine,

    /// The list of MNList diff items (one per block height)
    mnlist_diffs: BTreeMap<(CoreBlockHeight, CoreBlockHeight), MnListDiff>,

    /// Selected MNList diff
    selected_dml_diff_key: Option<(CoreBlockHeight, CoreBlockHeight)>,

    /// Selected MNList
    selected_dml_height_key: Option<CoreBlockHeight>,

    /// Selected display option
    selected_option_index: Option<usize>,
    /// Selected quorum within the MNList diff
    selected_quorum_in_diff_index: Option<usize>,

    /// Selected masternode within the MNList diff
    selected_masternode_in_diff_index: Option<usize>,

    /// Selected quorum within the MNList diff
    selected_quorum_index: Option<usize>,

    /// Selected masternode within the MNList diff
    selected_masternode_index: Option<usize>,

    error: Option<String>,
}

impl MasternodeListDiffScreen {
    /// Create a new MNListDiffScreen
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            base_block_height: "".to_string(),
            end_block_height: "".to_string(),
            selected_tab: 0,
            masternode_list_engine: MasternodeListEngine {
                block_hashes: Default::default(),
                block_heights: Default::default(),
                masternode_lists: Default::default(),
                known_chain_locks: Default::default(),
                network: Network::Dash,
            },
            mnlist_diffs: Default::default(),
            selected_dml_diff_key: None,
            selected_dml_height_key: None,
            selected_option_index: None,
            selected_quorum_in_diff_index: None,
            selected_masternode_in_diff_index: None,
            selected_quorum_index: None,
            selected_masternode_index: None,
            error: None,
        }
    }

    fn parse_heights(&mut self) -> Result<((u32, BlockHash), (u32, BlockHash)), String> {
        let base = if self.base_block_height.is_empty() {
            self.base_block_height = "0".to_string();
            match self.app_context.core_client.get_block_hash(0) {
                Ok(block_hash) => (0, BlockHash::from_byte_array(block_hash.to_byte_array())),
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        } else {
            match self.base_block_height.trim().parse() {
                Ok(start) => match self.app_context.core_client.get_block_hash(start) {
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
            match self.app_context.core_client.get_best_block_hash() {
                Ok(block_hash) => {
                    match self
                        .app_context
                        .core_client
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
                Ok(end) => match self.app_context.core_client.get_block_hash(end) {
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
            //todo put correct network
            self.masternode_list_engine = match MasternodeListEngine::initialize_with_diff_to_height(
                list_diff.clone(),
                block_height,
                Network::Dash,
            ) {
                Ok(masternode_list_engine) => masternode_list_engine,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return;
                }
            }
        } else {
            if let Err(e) =
                self.masternode_list_engine
                    .apply_diff(list_diff.clone(), block_height, false)
            {
                self.error = Some(e.to_string());
                return;
            }
        }

        if validate_quorums && !self.masternode_list_engine.masternode_lists.is_empty() {
            let hashes = self.masternode_list_engine.latest_masternode_list_non_rotating_quorum_hashes(&[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85]);
            let mut hashes_needed_to_validate = BTreeMap::new();
            for quorum_hash in hashes {
                let height = match self.app_context.core_client.get_block_header_info(&(BlockHash2::from_byte_array(quorum_hash.to_byte_array()))) {
                    Ok(header) => {
                        header.height as CoreBlockHeight
                    },
                    Err(e) => {
                        self.error = Some(e.to_string());
                        return;
                    }
                };
                let validation_hash = match self.app_context.core_client.get_block_hash(height - 8) {
                    Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
                    Err(e) => {
                        self.error = Some(e.to_string());
                        return;
                    }
                };
                hashes_needed_to_validate.insert(height, validation_hash);
            };

            if let Some((oldest_needed_height, oldest_needed_hash)) = hashes_needed_to_validate.first_key_value() {
                let (first_engine_height, first_masternode_list) = self.masternode_list_engine.masternode_lists.first_key_value().unwrap();
                    let (mut base_block_height, mut base_block_hash) = if *first_engine_height < *oldest_needed_height {
                        (*first_engine_height, first_masternode_list.block_hash)
                    } else {
                        let known_genesis_block_hash = match self.masternode_list_engine.network.known_genesis_block_hash() {
                            None => match self.app_context.core_client.get_block_hash(0) {
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

            if let Err(e) = self.masternode_list_engine.verify_masternode_list_quorums(block_height, &[LLMQType::Llmqtype50_60, LLMQType::Llmqtype400_85]) {
                self.error = Some(e.to_string());
            }
        }

        self.mnlist_diffs
            .insert((base_block_height, block_height), list_diff);
    }
    // fn fetch_range_dml(&mut self, step: u32, include_at_minus_8: bool, count: u32) {
    //     let ((base_block_height, base_block_hash), (block_height, block_hash)) =
    //         match self.parse_heights() {
    //             Ok(a) => a,
    //             Err(e) => {
    //                 self.error = Some(e);
    //                 return;
    //             }
    //         };
    //
    //     let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
    //         Ok(p2p_handler) => p2p_handler,
    //         Err(e) => {
    //             self.error = Some(e);
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
    //             self.error = Some(e.to_string());
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
    //             let end_block_hash = match self.app_context.core_client.get_block_hash(end_height) {
    //                 Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //                 Err(e) => {
    //                     self.error = Some(e.to_string());
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
    //             let end_block_hash = match self.app_context.core_client.get_block_hash(end_height) {
    //                 Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //                 Err(e) => {
    //                     self.error = Some(e.to_string());
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
    //             let end_block_hash = match self.app_context.core_client.get_block_hash(end_height) {
    //                 Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //                 Err(e) => {
    //                     self.error = Some(e.to_string());
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
    //         let end_block_hash = match self.app_context.core_client.get_block_hash(end_height) {
    //             Ok(block_hash) => BlockHash::from_byte_array(block_hash.to_byte_array()),
    //             Err(e) => {
    //                 self.error = Some(e.to_string());
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
    //     self.selected_dml_diff_key = None;
    //     self.selected_quorum_in_diff_index = None;
    // }

    /// Fetch the MNList diffs between the given base and end block heights.
    /// In a real implementation, you would replace the dummy function below with a call to
    /// dash_core’s DB (or other data source) to retrieve the MNList diffs.
    fn fetch_end_dml_diff(&mut self, validate_quorums: bool,) {
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

    /// Render the input area at the top (base and end block height fields plus Get DMLs button)
    fn render_input_area(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Base Block Height:");
            ui.add(TextEdit::singleline(&mut self.base_block_height).desired_width(80.0));
            ui.label("End Block Height:");
            ui.add(TextEdit::singleline(&mut self.end_block_height).desired_width(80.0));
            if ui.button("Get single end DML diff").clicked() {
                self.fetch_end_dml_diff(false);
            }
            if ui.button("Get DMLs and validate").clicked() {
                self.fetch_end_dml_diff(true);
            }
        });
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
                for (key, dml) in self.mnlist_diffs.iter() {
                    let block_label = format!("Base: {} -> Block: {}", key.0, key.1);

                    if ui
                        .selectable_label(self.selected_dml_diff_key == Some(*key), block_label)
                        .clicked()
                    {
                        self.selected_dml_diff_key = Some(*key);
                        self.selected_quorum_in_diff_index = None;
                    }
                }
            });
    }

    /// Render the list of quorums for the selected DML
    fn render_new_quorums(&mut self, ui: &mut Ui) {
        ui.heading("New Quorums");
        if let Some(selected_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&selected_key) {
                ScrollArea::vertical()
                    .id_salt("quorum_list_scroll_area")
                    .show(ui, |ui| {
                        for (q_index, quorum) in dml.new_quorums.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    self.selected_quorum_in_diff_index == Some(q_index),
                                    format!(
                                        "Quorum {} Type: {}",
                                        quorum.quorum_hash.to_string().as_str().split_at(5).0,
                                        QuorumType::from(quorum.llmq_type as u32).to_string()
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_quorum_in_diff_index = Some(q_index);
                                self.selected_masternode_in_diff_index = None;
                            }
                        }
                    });
            }
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

    fn render_quorums_in_masternode_list(&mut self, ui: &mut Ui) {
        if let Some(selected_height) = self.selected_dml_height_key {
            if let Some(mn_list) = self
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
            {
                ui.heading("Quorums in Masternode List");
                ScrollArea::vertical()
                    .id_salt("quorum_list_scroll_area")
                    .show(ui, |ui| {
                        for (llmq_type, quorum_map) in &mn_list.quorums {
                            for (q_index, (quorum_hash, quorum_entry)) in
                                quorum_map.iter().enumerate()
                            {
                                if ui
                                    .selectable_label(
                                        self.selected_quorum_index == Some(q_index),
                                        format!(
                                            "Quorum {} Type: {}",
                                            quorum_hash.to_string().as_str().split_at(5).0,
                                            QuorumType::from(*llmq_type as u32).to_string()
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.selected_quorum_index = Some(q_index);
                                    self.selected_masternode_index = None;
                                }
                            }
                        }
                    });
            }
        }
    }

    fn render_masternodes_in_masternode_list(&mut self, ui: &mut Ui) {
        if let Some(selected_height) = self.selected_dml_height_key {
            if let Some(mn_list) = self
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
            {
                ui.heading("Masternodes in List");
                ScrollArea::vertical()
                    .id_salt("masternode_list_scroll_area")
                    .show(ui, |ui| {
                        for (m_index, (pro_tx_hash, masternode)) in
                            mn_list.masternodes.iter().enumerate()
                        {
                            if ui
                                .selectable_label(
                                    self.selected_masternode_index == Some(m_index),
                                    format!(
                                        "{} {} {}",
                                        if masternode.masternode_list_entry.mn_type
                                            == MasternodeType::Regular
                                        {
                                            "MN"
                                        } else {
                                            "EN"
                                        },
                                        masternode
                                            .masternode_list_entry
                                            .service_address
                                            .ip()
                                            .to_string(),
                                        pro_tx_hash.to_string().as_str().split_at(5).0
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_quorum_index = None;
                                self.selected_masternode_index = Some(m_index);
                            }
                        }
                    });
            }
        }
    }

    fn render_masternode_list_page(&mut self, ui: &mut Ui) {
        ui.columns(3, |cols| {
            cols[0].with_layout(Layout::top_down(Align::Min), |ui| {
                self.render_masternode_lists(ui);
            });
            cols[1].with_layout(Layout::top_down(Align::Min), |ui| {
                self.render_selected_masternode_list_items(ui);
            });
            cols[2].with_layout(Layout::top_down(Align::Min), |ui| {
                if self.selected_quorum_index.is_some() {
                    self.render_quorum_details(ui);
                } else if self.selected_masternode_index.is_some() {
                    self.render_mn_details(ui);
                }
            });
        });
    }

    fn render_selected_tab(&mut self, ui: &mut Ui) {
        // Define available tabs
        let tabs = ["Diffs", "Masternode Lists"];

        // Render the selection buttons
        ui.horizontal(|ui| {
            for (index, tab) in tabs.iter().enumerate() {
                if ui
                    .selectable_label(self.selected_tab == index, *tab)
                    .clicked()
                {
                    self.selected_tab = index;
                }
            }
        });

        ui.separator();

        match self.selected_tab {
            0 => self.render_diffs(ui), // Existing diffs rendering logic
            1 => self.render_masternode_list_page(ui), // Placeholder for masternode list display
            _ => {}
        }
    }

    fn render_diffs(&mut self, ui: &mut Ui) {
        // Create a three-column layout:
        // - Left column: list of MNList Diffs (by block height)
        // - Middle column: list of quorums for the selected DML
        // - Right column: quorum details
        ui.columns(3, |cols| {
            cols[0].with_layout(Layout::top_down(Align::Min), |ui| {
                self.render_diff_list(ui);
            });
            cols[1].with_layout(Layout::top_down(Align::Min), |ui| {
                self.render_selected_dml_items(ui);
            });
            cols[2].with_layout(Layout::top_down(Align::Min), |ui| {
                if self.selected_quorum_in_diff_index.is_some() {
                    self.render_quorum_details(ui);
                } else if self.selected_masternode_in_diff_index.is_some() {
                    self.render_mn_details(ui);
                }
            });
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
                                        if masternode.mn_type == MasternodeType::Regular {
                                            "MN"
                                        } else {
                                            "EN"
                                        },
                                        masternode.service_address.ip().to_string(),
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

    /// Render the list of items for the selected DML, with a selector at the top
    fn render_selected_dml_items(&mut self, ui: &mut Ui) {
        ui.heading("Masternode List Diff Explorer");

        // Define available options for selection
        let options = ["New Quorums", "Masternode Changes"];
        let mut selected_index = self.selected_option_index.unwrap_or(0);

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

        // Determine the selected category and display corresponding information
        if let Some(selected_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&selected_key) {
                ScrollArea::vertical()
                    .id_salt("dml_items_scroll_area")
                    .show(ui, |ui| match selected_index {
                        0 => self.render_new_quorums(ui),
                        1 => self.render_masternode_changes(ui),
                        _ => (),
                    });
            }
        } else {
            ui.label("Select a block height to show details.");
        }
    }

    /// Render the details for the selected quorum
    fn render_quorum_details(&mut self, ui: &mut Ui) {
        ui.heading("Quorum Details");
        if let Some(dml_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&dml_key) {
                if let Some(q_index) = self.selected_quorum_in_diff_index {
                    if let Some(quorum) = dml.new_quorums.get(q_index) {
                        Frame::none()
                            .stroke(Stroke::new(1.0, Color32::BLACK))
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                                ScrollArea::vertical().show(ui, |ui| {
                                    ui.label(format!(
                                        "Version: {}\nQuorum Hash: {}\nSigners: {} members\nValid Members: {} members\nQuorum Public Key: {}",
                                        quorum.version,
                                        quorum.quorum_hash,
                                        quorum.signers.len(),
                                        quorum.valid_members.len(),
                                        quorum.quorum_public_key
                                    ));
                                });
                            });
                    }
                } else {
                    ui.label("Select a quorum to view details.");
                }
            }
        } else if let Some(selected_height) = self.selected_dml_height_key {
            if let Some(mn_list) = self
                .masternode_list_engine
                .masternode_lists
                .get(&selected_height)
            {
                if let Some(q_index) = self.selected_quorum_index {
                    let mut quorums: Vec<_> = mn_list
                        .quorums
                        .iter()
                        .flat_map(|(_, quorum_map)| quorum_map.values())
                        .collect();

                    if let Some(quorum) = quorums.get(q_index) {
                        Frame::none()
                            .stroke(Stroke::new(1.0, Color32::BLACK))
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                                ScrollArea::vertical().show(ui, |ui| {
                                    ui.label(format!(
                                        "Quorum Type: {}\nQuorum Hash: {}\nSigners: {} members\nValid Members: {} members\nQuorum Public Key: {}\nValidation status: {}",
                                        QuorumType::from(quorum.quorum_entry.llmq_type as u32),
                                        quorum.quorum_entry.quorum_hash,
                                        quorum.quorum_entry.signers.len(),
                                        quorum.quorum_entry.valid_members.len(),
                                        quorum.quorum_entry.quorum_public_key,
                                        quorum.verified,
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
        ui.heading("Masternode Details");

        if let Some(dml_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&dml_key) {
                if let Some(mn_index) = self.selected_masternode_in_diff_index {
                    if let Some(masternode) = dml.new_masternodes.get(mn_index) {
                        Frame::none()
                            .stroke(Stroke::new(1.0, Color32::BLACK))
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                                ScrollArea::vertical().show(ui, |ui| {
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
                                        masternode.pro_reg_tx_hash,
                                        match masternode.confirmed_hash {
                                            None => "No confirmed hash".to_string(),
                                            Some(confirmed_hash) => confirmed_hash.to_string(),
                                        },
                                        masternode.service_address.ip(),
                                        masternode.service_address.port(),
                                        masternode.operator_public_key,
                                        masternode.key_id_voting,
                                        masternode.is_valid,
                                        match masternode.mn_type {
                                            MasternodeType::Regular => "Regular".to_string(),
                                            MasternodeType::HighPerformance {
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
        } else {
            ui.label("Select a block height and Masternode.");
        }
    }
}

impl ScreenLike for MasternodeListDiffScreen {
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Optionally implement message display here
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

        // In this example we simply use the CentralPanel; you can add top/left panels as in your other screens.
        egui::CentralPanel::default().show(ctx, |ui| {
            // Top: input area (base/end block height + Get DMLs button)
            self.render_input_area(ui);

            if let Some(error) = &self.error {
                ui.label(error);
            }

            ui.separator();

            self.render_selected_tab(ui);
        });
        action
    }
}
