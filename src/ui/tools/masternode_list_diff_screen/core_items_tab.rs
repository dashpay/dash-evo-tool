use super::MasternodeListDiffScreen;
use crate::backend_task::core::CoreItem;
use crate::components::core_p2p_handler::CoreP2PHandler;
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize as serialize2};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    Block, ChainLock, ChainLock as ChainLock2, InstantLock, InstantLock as InstantLock2,
};
use eframe::egui::{self, ScrollArea, Ui};
use egui::{Align, Layout, Vec2};

impl MasternodeListDiffScreen {
    #[allow(dead_code)]
    pub(super) fn render_selected_item_details(&mut self, ui: &mut Ui, selected_item: String) {
        ui.heading("Details");

        ScrollArea::vertical().show(ui, |ui| {
            ui.monospace(selected_item);
        });
    }

    /// Render core items, including chain-locked blocks and instant send transactions.
    pub(super) fn render_core_items(&mut self, ui: &mut Ui) {
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
                            self.chain_locked_blocks.iter()
                        {
                            let label_text = format!(
                                "{} {} {}",
                                if *is_valid { "✔" } else { "❌" },
                                block_height,
                                block.header.block_hash()
                            );

                            if ui
                                .selectable_label(
                                    matches!(self.selected_core_item, Some((CoreItem::ChainLockedBlock(_, ref l), _)) if l.block_height == *block_height),
                                    label_text,
                                )
                                .clicked()
                            {
                                self.selected_core_item = Some((CoreItem::ChainLockedBlock(block.clone(), chain_lock.clone()), *is_valid));
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
                            self.instant_send_transactions.iter()
                        {
                            let label_text = format!(
                                "{} TxID: {}",
                                if *is_valid { "✔" } else { "❌" },
                                transaction.txid()
                            );

                            if ui
                                .selectable_label(
                                    matches!(self.selected_core_item, Some((CoreItem::InstantLockedTransaction(ref t, _, _), _)) if t == transaction),
                                    label_text,
                                )
                                .clicked()
                            {
                                self.selected_core_item = Some((CoreItem::InstantLockedTransaction(transaction.clone(), vec![], instant_lock.clone()), *is_valid));
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
                    if let Some((selected_core_item, _)) = &self.selected_core_item {
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
            &self.selected_core_item
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
            &self.selected_core_item
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
            match self.masternode_list_engine.is_lock_quorum(&instant_lock_2) {
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

    pub(super) fn attempt_verify_chain_lock(&self, chain_lock: &ChainLock) -> bool {
        let b = serialize2(chain_lock);
        let chain_lock_2: ChainLock2 = deserialize(b.as_slice()).expect("todo");
        self.masternode_list_engine
            .verify_chain_lock(&chain_lock_2)
            .is_ok()
    }

    pub(super) fn attempt_verify_transaction_lock(&self, instant_lock: &InstantLock) -> bool {
        let b = serialize2(instant_lock);
        let instant_lock_2: InstantLock2 = deserialize(b.as_slice()).expect("todo");
        self.masternode_list_engine
            .verify_is_lock(&instant_lock_2)
            .is_ok()
    }

    pub(super) fn received_new_block(&mut self, block: Block, chain_lock: ChainLock) {
        let valid = self.attempt_verify_chain_lock(&chain_lock);
        self.end_block_height = chain_lock.block_height.to_string();
        if self.syncing
            && let Some((base_block_height, masternode_list)) = self
                .masternode_list_engine
                .masternode_lists
                .last_key_value()
            && *base_block_height < chain_lock.block_height
        {
            let mut p2p_handler = match CoreP2PHandler::new(self.app_context.network, None) {
                Ok(p2p_handler) => p2p_handler,
                Err(e) => {
                    self.error = Some(e);
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

            // Reset selections when new data is loaded
            self.selected_dml_diff_key = None;
            self.selected_quorum_in_diff_index = None;
        }
        self.chain_locked_blocks
            .insert(chain_lock.block_height, (block, chain_lock, valid));
    }
}
