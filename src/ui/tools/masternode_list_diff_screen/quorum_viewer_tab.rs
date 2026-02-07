use super::MasternodeListDiffScreen;
use dash_sdk::dashcore_rpc::json::QuorumType;
use dash_sdk::dpp::dashcore::QuorumHash;
use dash_sdk::dpp::dashcore::bls_sig_utils::BLSSignature;
use dash_sdk::dpp::dashcore::sml::llmq_entry_verification::LLMQEntryVerificationStatus;
use dash_sdk::dpp::dashcore::sml::llmq_type::LLMQType;
use dash_sdk::dpp::dashcore::sml::quorum_entry::qualified_quorum_entry::VerifyingChainLockSignaturesType;
use dash_sdk::dpp::dashcore::transaction::special_transaction::quorum_commitment::QuorumEntry;
use dash_sdk::dpp::prelude::CoreBlockHeight;
use eframe::egui::{self, ScrollArea, Ui};
use egui::{Align, Frame, Layout, Stroke, Vec2};
use itertools::Itertools;
use std::collections::{BTreeMap, BTreeSet};

use crate::ui::theme::DashColors;

impl MasternodeListDiffScreen {
    pub(super) fn render_quorums_in_masternode_list(&mut self, ui: &mut Ui) {
        let mut heights: BTreeMap<QuorumHash, CoreBlockHeight> = BTreeMap::new();
        let mut masternode_block_hash = None;
        if let Some(selected_height) = self.selected_dml_height_key {
            if !self
                .masternode_lists_with_all_quorum_heights_known
                .contains(&selected_height)
            {
                if let Some(quorum_hashes) = self
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
                        if let Ok(height) = self.cache.get_height_and_cache(
                            quorum_hash,
                            &mut self.masternode_list_engine,
                            &self.app_context,
                        ) {
                            heights.insert(*quorum_hash, height);
                        }
                    }
                }
                self.masternode_lists_with_all_quorum_heights_known
                    .insert(selected_height);
            }
            if let Some(mn_list) = self
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
                        if let Ok(height) = self.cache.get_height(
                            quorum_hash,
                            &self.masternode_list_engine,
                            &self.app_context,
                        ) {
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
                                    self.selected_quorum_hash_in_mnlist_diff
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
                                self.selected_quorum_hash_in_mnlist_diff =
                                    Some((*llmq_type, quorum_entry.quorum_entry.quorum_hash));
                                self.selected_masternode_pro_tx_hash = None;
                                self.selected_dml_diff_key = None;
                            }
                        }
                    }
                });
        }
    }

    pub(super) fn required_cl_sig_heights(&self, quorum: &QuorumEntry) -> BTreeSet<u32> {
        let mut required_heights = BTreeSet::new();
        let Ok(quorum_block_height) = self.cache.get_height(
            &quorum.quorum_hash,
            &self.masternode_list_engine,
            &self.app_context,
        ) else {
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
    pub(super) fn render_quorum_details(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let border = DashColors::border(dark_mode);
        ui.heading("Quorum Details");
        if let Some(dml_key) = self.selected_dml_diff_key {
            if let Some(dml) = self.mnlist_diffs.get(&dml_key) {
                if let Some(q_index) = self.selected_quorum_in_diff_index {
                    if let Some(quorum) = dml.new_quorums.get(q_index) {
                        Frame::NONE
                            .stroke(Stroke::new(1.0, border))
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::new(ui.available_width(), 300.0));
                                let height = self.cache.get_height(&quorum.quorum_hash, &self.masternode_list_engine, &self.app_context).ok();

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
                                    if let Ok(hash) = self.cache.get_block_hash(height - 8, &self.masternode_list_engine, &self.app_context) {
                                        if let Ok(Some(sig)) = self.cache.get_chain_lock_sig(&hash, &self.masternode_list_engine, &self.app_context) {
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
                                            self.cache.get_height(&quorum.quorum_hash, &self.masternode_list_engine, &self.app_context).ok().map(|height| format!("{}", height)).unwrap_or("Unknown".to_string()),
                                            quorum.quorum_hash,
                                            self.cache.get_height(&quorum.quorum_hash, &self.masternode_list_engine, &self.app_context).ok().and_then(|height| quorum.quorum_index.map(|index| format!("{}", height - index as CoreBlockHeight))).unwrap_or("Unknown".to_string()),
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
                                            self.cache.get_height(&quorum.quorum_hash, &self.masternode_list_engine, &self.app_context).ok().map(|height| format!("{}", height)).unwrap_or("Unknown".to_string()),
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
                if let Some((llmq_type, quorum_hash)) = self.selected_quorum_hash_in_mnlist_diff {
                    if let Some(quorum) = mn_list
                        .quorums
                        .get(&llmq_type)
                        .and_then(|quorums_by_type| quorums_by_type.get(&quorum_hash))
                    {
                        let height = self
                            .cache
                            .get_height(
                                &quorum.quorum_entry.quorum_hash,
                                &self.masternode_list_engine,
                                &self.app_context,
                            )
                            .ok();
                        let chain_lock_sig =
                            if quorum.quorum_entry.llmq_type.is_rotating_quorum_type() {
                                let heights = self.required_cl_sig_heights(&quorum.quorum_entry);
                                format!(
                                    "heights [{}]",
                                    heights.iter().map(|h| h.to_string()).join(" | ")
                                )
                            } else if let Some(height) = height {
                                if let Ok(hash) = self.cache.get_block_hash(
                                    height - 8,
                                    &self.masternode_list_engine,
                                    &self.app_context,
                                ) {
                                    if let Ok(Some(sig)) = self.cache.get_chain_lock_sig(
                                        &hash,
                                        &self.masternode_list_engine,
                                        &self.app_context,
                                    ) {
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
                                        self.cache.get_height(&quorum.quorum_entry.quorum_hash, &self.masternode_list_engine, &self.app_context).ok().map(|height| format!("{}", height)).unwrap_or("Unknown".to_string()),
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

    pub(super) fn render_quorums(&mut self, ui: &mut Ui) {
        ui.heading("Quorum Viewer");

        // Get all available quorum types
        let quorum_types: Vec<LLMQType> = self
            .masternode_list_engine
            .quorum_statuses
            .keys()
            .cloned()
            .collect();

        // Ensure a quorum type is selected
        if self.selected_quorum_type_in_quorum_viewer.is_none() {
            self.selected_quorum_type_in_quorum_viewer = quorum_types.first().copied();
        }

        // Render quorum type selection bar
        ui.horizontal(|ui| {
            for quorum_type in &quorum_types {
                if ui
                    .selectable_label(
                        self.selected_quorum_type_in_quorum_viewer == Some(*quorum_type),
                        quorum_type.to_string(),
                    )
                    .clicked()
                {
                    self.selected_quorum_type_in_quorum_viewer = Some(*quorum_type);
                    self.selected_quorum_hash_in_quorum_viewer = None; // Reset selected quorum when switching types
                }
            }
        });

        ui.separator();

        let Some(selected_quorum_type) = self.selected_quorum_type_in_quorum_viewer else {
            ui.label("No quorum types available.");
            return;
        };

        let Some(quorum_map) = self
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
                                            self.selected_quorum_hash_in_quorum_viewer
                                                == Some(*quorum_hash),
                                            hash_label,
                                        );

                                        if hash_response.clicked() {
                                            self.selected_quorum_hash_in_quorum_viewer =
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

                    if let Some(selected_quorum_hash) = self.selected_quorum_hash_in_quorum_viewer {
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
}
