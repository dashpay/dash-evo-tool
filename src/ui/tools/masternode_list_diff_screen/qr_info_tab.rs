use super::{MasternodeListDiffScreen, SelectedQRItem};
use dash_sdk::dashcore_rpc::json::QuorumType;
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::consensus::Decodable;
use dash_sdk::dpp::dashcore::network::message_qrinfo::{QRInfo, QuorumSnapshot};
use dash_sdk::dpp::dashcore::network::message_sml::MnListDiff;
use dash_sdk::dpp::dashcore::sml::llmq_entry_verification::LLMQEntryVerificationStatus;
use dash_sdk::dpp::dashcore::sml::quorum_entry::qualified_quorum_entry::QualifiedQuorumEntry;
use eframe::egui::{self, ScrollArea, Ui};
use egui::{Align, Layout};
use itertools::Itertools;
use rfd::FileDialog;

impl MasternodeListDiffScreen {
    pub(super) fn render_qr_info(&mut self, ui: &mut Ui) {
        ui.heading("QRInfo Viewer");

        // Select the first available QRInfo if none is selected
        let selected_qr_info = {
            let Some((_, selected_qr_info)) = self.qr_infos.first_key_value() else {
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
                                    self.qr_infos.insert(key, qr_info.clone());
                                    self.feed_qr_info_and_get_dmls(qr_info, None);
                                }
                                Err(_) => {
                                    match bincode::decode_from_slice::<QRInfo, _>(
                                        &bytes,
                                        bincode::config::standard(),
                                    ) {
                                        Ok((qr_info, _)) => {
                                            let key = qr_info.mn_list_diff_tip.block_hash;
                                            self.qr_infos.insert(key, qr_info);
                                        }
                                        Err(e) => {
                                            eprintln!("Failed to decode QRInfo: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to read file: {}", e);
                        }
                    }
                }
                return;
            };
            selected_qr_info.clone()
        };

        if let Ok(height) = self.cache.get_height(
            &selected_qr_info.mn_list_diff_tip.block_hash,
            &self.masternode_list_engine,
            &self.app_context,
        ) {
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
                            eprintln!("Failed to write file: {}", e);
                        }
                    }
                }
            });
        }

        // Track user selections
        if self.selected_qr_field.is_none() {
            self.selected_qr_field = Some("Quorum Snapshots".to_string());
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
                                self.selected_qr_field.as_deref() == Some(*field),
                                *field,
                            )
                            .clicked()
                        {
                            self.selected_qr_field = Some(field.to_string());
                            self.selected_qr_list_index = None;
                            self.selected_qr_item = None;
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

                    match self.selected_qr_field.as_deref() {
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
                    if let Some(selected_item) = &self.selected_qr_item {
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
            self.cache.get_height_or_error_as_string(
                &mn_list_diff.base_block_hash,
                &self.masternode_list_engine,
                &self.app_context
            ),
            mn_list_diff.block_hash,
            self.cache.get_height_or_error_as_string(
                &mn_list_diff.block_hash,
                &self.masternode_list_engine,
                &self.app_context
            )
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
                    .selectable_label(self.selected_qr_list_index == Some(name.to_string()), *name)
                    .clicked()
                {
                    self.selected_qr_list_index = Some(name.to_string());
                    self.selected_qr_item =
                        Some(SelectedQRItem::SelectedSnapshot((*snapshot).clone()));
                }
            });

            if ui
                .selectable_label(
                    self.selected_qr_list_index == Some("Quorum Snapshot h-4c".to_string()),
                    "Quorum Snapshot h-4c",
                )
                .clicked()
            {
                self.selected_qr_list_index = Some("Quorum Snapshot h-4c".to_string());
                self.selected_qr_item = Some(SelectedQRItem::SelectedSnapshot((*qs4c).clone()));
            }
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
        let base_height_as_string = match self.cache.get_height_and_cache(
            &mn_list_diff.base_block_hash,
            &mut self.masternode_list_engine,
            &self.app_context,
        ) {
            Ok(height) => height.to_string(),
            Err(_) => "?".to_string(),
        };

        let height = self
            .cache
            .get_height_and_cache(
                &mn_list_diff.block_hash,
                &mut self.masternode_list_engine,
                &self.app_context,
            )
            .ok();

        let height_as_string = match height {
            Some(height) => height.to_string(),
            None => "?".to_string(),
        };

        let extra_block_diff_info = height
            .and_then(|height| {
                last_diff.and_then(|diff| {
                    self.cache
                        .get_height(
                            &diff.block_hash,
                            &self.masternode_list_engine,
                            &self.app_context,
                        )
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
                    self.selected_qr_list_index == Some(string.clone()),
                    string.as_str(),
                )
                .clicked()
            {
                self.selected_qr_list_index = Some(string);
                self.selected_qr_item =
                    Some(SelectedQRItem::MNListDiff(Box::new((*mn_diff4c).clone())));
            }
        }

        mn_diffs.iter().for_each(|(name, diff)| {
            if ui
                .selectable_label(self.selected_qr_list_index == Some(name.to_string()), name)
                .clicked()
            {
                self.selected_qr_list_index = Some(name.to_string());
                self.selected_qr_item = Some(SelectedQRItem::MNListDiff(Box::new((*diff).clone())));
            }
        });
    }

    fn render_last_commitments(&mut self, ui: &mut Ui, cycle_hash: Option<BlockHash>) {
        let Some(cycle_hash) = cycle_hash else {
            ui.label("QR Info had no rotated quorums. This should not happen.");
            return;
        };
        let Some(cycle_quorums) = self
            .masternode_list_engine
            .rotated_quorums_per_cycle
            .get(&cycle_hash)
        else {
            ui.label(format!(
                "Engine does not know of cycle {} at height {}, we know of cycles [{}]",
                cycle_hash,
                self.cache.get_height_or_error_as_string(
                    &cycle_hash,
                    &self.masternode_list_engine,
                    &self.app_context
                ),
                self.masternode_list_engine
                    .rotated_quorums_per_cycle
                    .keys()
                    .map(|key| format!(
                        "{}, {}",
                        self.cache.get_height_or_error_as_string(
                            key,
                            &self.masternode_list_engine,
                            &self.app_context
                        ),
                        key
                    ))
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
                    self.selected_qr_list_index == Some(index.to_string()),
                    label_text,
                )
                .clicked()
            {
                self.selected_qr_list_index = Some(index.to_string());
                self.selected_qr_item =
                    Some(SelectedQRItem::QuorumEntry(Box::new(commitment.clone())));
            }
        }
    }

    fn render_quorum_snapshot_list(&mut self, ui: &mut Ui, qr_info: &QRInfo) {
        for (index, snapshot) in qr_info.quorum_snapshot_list.iter().enumerate() {
            if ui
                .selectable_label(
                    self.selected_qr_list_index == Some(index.to_string()),
                    format!("Snapshot {}", index),
                )
                .clicked()
            {
                self.selected_qr_list_index = Some(index.to_string());
                self.selected_qr_item = Some(SelectedQRItem::SelectedSnapshot(snapshot.clone()));
            }
        }
    }

    fn render_mn_list_diff_list(&mut self, ui: &mut Ui, qr_info: &QRInfo) {
        for (index, diff) in qr_info.mn_list_diff_list.iter().enumerate() {
            if ui
                .selectable_label(
                    self.selected_qr_list_index == Some(index.to_string()),
                    format!("MNListDiff {}", index),
                )
                .clicked()
            {
                self.selected_qr_list_index = Some(index.to_string());
                self.selected_qr_item = Some(SelectedQRItem::MNListDiff(Box::new(diff.clone())));
            }
        }
    }
}
