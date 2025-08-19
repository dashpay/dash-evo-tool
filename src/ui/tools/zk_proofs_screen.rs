use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::zk_proofs::ZKProofTask;
use crate::context::AppContext;
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::ui::RootScreenType;
use crate::ui::ScreenLike;
use crate::ui::components::confirmation_dialog::ConfirmationDialog;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{DashColors, Shape, Spacing, Typography};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{
    Identity, IdentityPublicKey, KeyType, Purpose, accessors::IdentityGettersV0,
};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{Button, ComboBox, Context, Frame, Grid, Margin, RichText, ScrollArea, TextEdit, Ui};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// Helper function to render a custom collapsing header with +/- button
fn render_collapsing_header(ui: &mut egui::Ui, text: impl Into<String>, is_expanded: bool) -> bool {
    let text = text.into();
    let dark_mode = ui.ctx().style().visuals.dark_mode;

    let mut clicked = false;

    ui.horizontal(|ui| {
        // +/- button
        let button_text = if is_expanded { "−" } else { "+" };
        let button_response = ui.add(
            egui::Button::new(
                RichText::new(button_text)
                    .size(20.0)
                    .color(DashColors::DASH_BLUE),
            )
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE),
        );

        if button_response.clicked() {
            clicked = true;
        }

        // Label text
        let label_text = RichText::new(text)
            .size(14.0)
            .heading()
            .color(DashColors::text_primary(dark_mode));

        let label_response = ui.add(egui::Label::new(label_text).sense(egui::Sense::click()));
        if label_response.clicked() {
            clicked = true;
        }
    });

    clicked
}

#[derive(Clone, PartialEq)]
pub enum ProofMode {
    Generate,
    Verify,
}

#[derive(Clone)]
pub enum InputMethod {
    Paste,
    File,
    Platform,
}

#[derive(Clone)]
pub struct VerificationResult {
    pub is_valid: bool,
    pub verified_at: u64,
    pub contract_id: String,
    pub security_level: u32,
    pub error_message: Option<String>,
    pub technical_details: String,
}

#[derive(Clone)]
pub struct ProofData {
    pub full_proof: crate::proofs::grovestark_integration::ProofDataOutput,
    pub hash: String,
    pub size: usize,
    pub generation_time: Duration,
}

pub struct ZKProofsScreen {
    pub(crate) app_context: Arc<AppContext>,
    mode: ProofMode,

    // Generation fields
    selected_identity: Option<String>,
    selected_key: Option<IdentityPublicKey>,
    selected_contract: Option<String>,
    selected_document_type: Option<String>,
    available_document_types: Vec<String>, // Document types for selected contract
    selected_document: Option<String>,
    available_identities: Vec<Identity>,
    qualified_identities: Vec<QualifiedIdentity>, // Store full qualified identities for key access
    available_contracts: Vec<(String, String)>,   // (id, name)
    // Documents will be entered directly via text input
    is_generating: bool,
    generated_proof: Option<ProofData>,
    proof_size: Option<String>,
    generation_time: Option<Duration>,
    security_level: u32,
    grinding_bits: u32,

    // Verification fields
    input_method: InputMethod,
    proof_text: String,
    proof_file_path: Option<PathBuf>,
    proof_id: String,
    is_verifying: bool,
    verification_result: Option<VerificationResult>,
    state_root: String,
    contract_id: String,
    check_timestamp: bool,
    expected_timestamp: u64,
    advanced_expanded: bool,

    // Error handling
    error_message: Option<String>,

    // Dialogs
    confirmation_dialog: Option<ConfirmationDialog>,

    // UI state
    public_inputs_expanded: bool,
}

impl ZKProofsScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        // Load initial qualified identities
        let qualified_identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        let available_identities = qualified_identities
            .iter()
            .map(|qualified_identity| qualified_identity.identity.clone())
            .collect();

        tracing::info!(
            "ZK Proofs screen loaded {} identities",
            qualified_identities.len()
        );

        // Load initial contracts (exclude system contracts)
        let excluded_aliases = ["dpns", "keyword_search", "token_history", "withdrawals"];
        let all_contracts = app_context.get_contracts(None, None).unwrap_or_default();

        tracing::info!(
            "ZK Proofs screen found {} total contracts",
            all_contracts.len()
        );

        let available_contracts: Vec<(String, String)> = all_contracts
            .into_iter()
            .filter(|c| match &c.alias {
                Some(alias) => {
                    let is_system = excluded_aliases.contains(&alias.as_str());
                    if is_system {
                        tracing::debug!("Excluding system contract: {}", alias);
                    }
                    !is_system
                }
                None => true,
            })
            .map(|qualified_contract| {
                let id = qualified_contract
                    .contract
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
                let name = qualified_contract
                    .alias
                    .unwrap_or_else(|| format!("Contract {}", &id[..8]));
                tracing::debug!("Including contract: {} ({})", name, id);
                (id, name)
            })
            .collect();

        tracing::info!(
            "ZK Proofs screen loaded {} user contracts after filtering",
            available_contracts.len()
        );

        Self {
            app_context: app_context.clone(),
            mode: ProofMode::Generate,
            selected_identity: None,
            selected_key: None,
            selected_contract: None,
            selected_document_type: None,
            available_document_types: Vec::new(),
            selected_document: None,
            available_identities,
            qualified_identities,
            available_contracts,
            is_generating: false,
            generated_proof: None,
            proof_size: None,
            generation_time: None,
            security_level: 128,
            grinding_bits: 16,
            input_method: InputMethod::Paste,
            proof_text: String::new(),
            proof_file_path: None,
            proof_id: String::new(),
            is_verifying: false,
            verification_result: None,
            state_root: String::new(),
            contract_id: String::new(),
            check_timestamp: false,
            expected_timestamp: 0,
            advanced_expanded: false,
            error_message: None,
            confirmation_dialog: None,
            public_inputs_expanded: false,
        }
    }

    fn refresh_identities(&mut self, app_context: &AppContext) {
        let all_qualified_identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        // Filter identities to only show those with EdDSA keys
        self.qualified_identities = all_qualified_identities
            .into_iter()
            .filter(|qi| self.has_eddsa_keys(&qi.identity))
            .collect();

        self.available_identities = self
            .qualified_identities
            .iter()
            .map(|qualified_identity| qualified_identity.identity.clone())
            .collect();

        tracing::info!(
            "Refreshed identities: found {} identities with EdDSA keys (filtered for ZK proof compatibility)",
            self.qualified_identities.len()
        );

        // Reset selected key if identity changed
        self.selected_key = None;
    }

    fn get_qualified_identity(&self, identity_id_str: &str) -> Option<&QualifiedIdentity> {
        self.qualified_identities
            .iter()
            .find(|qi| qi.identity.id().to_string(Encoding::Base58) == identity_id_str)
    }

    /// Check if an identity has any EdDSA keys suitable for ZK proofs
    fn has_eddsa_keys(&self, identity: &Identity) -> bool {
        identity.public_keys().iter().any(|(_, key)| {
            matches!(key.key_type(), KeyType::EDDSA_25519_HASH160)
                && (key.purpose() == Purpose::AUTHENTICATION || key.purpose() == Purpose::TRANSFER)
        })
    }

    fn get_available_keys(&self, identity_id_str: &str) -> Vec<&IdentityPublicKey> {
        if let Some(qualified_identity) = self.get_qualified_identity(identity_id_str) {
            qualified_identity
                .private_keys
                .identity_public_keys()
                .into_iter()
                .filter(|(target, _)| **target == PrivateKeyTarget::PrivateKeyOnMainIdentity)
                .map(|(_, key_ref)| &key_ref.identity_public_key)
                .filter(|key| {
                    // Only show EdDSA keys suitable for signing
                    matches!(key.key_type(), KeyType::EDDSA_25519_HASH160)
                        && (key.purpose() == Purpose::AUTHENTICATION
                            || key.purpose() == Purpose::TRANSFER)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn refresh_contracts(&mut self, app_context: &AppContext) {
        let excluded_aliases = ["dpns", "keyword_search", "token_history", "withdrawals"];
        let all_contracts = app_context.get_contracts(None, None).unwrap_or_default();

        self.available_contracts = all_contracts
            .into_iter()
            .filter(|c| match &c.alias {
                Some(alias) => !excluded_aliases.contains(&alias.as_str()),
                None => true,
            })
            .map(|qualified_contract| {
                let id = qualified_contract
                    .contract
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
                let name = qualified_contract
                    .alias
                    .unwrap_or_else(|| format!("Contract {}", &id[..8]));
                (id, name)
            })
            .collect();

        tracing::info!(
            "Refreshed contracts: found {} user contracts",
            self.available_contracts.len()
        );
    }

    fn refresh_document_types(&mut self, app_context: &AppContext, contract_id: &str) {
        self.available_document_types.clear();
        self.selected_document_type = None;

        if let Ok(contracts) = app_context.get_contracts(None, None) {
            for contract in contracts {
                let id = contract
                    .contract
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

                if id == contract_id {
                    self.available_document_types = contract
                        .contract
                        .document_types()
                        .keys()
                        .map(|s| s.to_string())
                        .collect();

                    tracing::info!(
                        "Found {} document types for contract {}: {:?}",
                        self.available_document_types.len(),
                        &contract_id[..8],
                        self.available_document_types
                    );

                    break;
                }
            }
        }
    }

    fn generate_proof(&mut self, app_context: &AppContext) -> AppAction {
        self.is_generating = true;
        self.error_message = None;

        // Get the required IDs
        let identity_id = match &self.selected_identity {
            Some(id) => {
                // Debug: Log the identity ID being used
                tracing::info!(
                    "ZK Proof generation: Using identity ID: '{}' (length: {})",
                    id,
                    id.len()
                );
                id.clone()
            }
            None => {
                self.error_message = Some("No identity selected".to_string());
                self.is_generating = false;
                return AppAction::None;
            }
        };

        let selected_key = match &self.selected_key {
            Some(key) => key,
            None => {
                self.error_message = Some("No key selected".to_string());
                self.is_generating = false;
                return AppAction::None;
            }
        };

        let contract_id = match &self.selected_contract {
            Some(id) => {
                tracing::info!(
                    "ZK Proof generation: Using contract ID: '{}' (length: {})",
                    id,
                    id.len()
                );
                id.clone()
            }
            None => {
                self.error_message = Some("No contract selected".to_string());
                self.is_generating = false;
                return AppAction::None;
            }
        };

        let document_type = match &self.selected_document_type {
            Some(doc_type) => {
                tracing::info!("ZK Proof generation: Using document type: '{}'", doc_type);
                doc_type.clone()
            }
            None => {
                self.error_message = Some("No document type selected".to_string());
                self.is_generating = false;
                return AppAction::None;
            }
        };

        let document_id = match &self.selected_document {
            Some(id) => {
                tracing::info!(
                    "ZK Proof generation: Using document ID: '{}' (length: {})",
                    id,
                    id.len()
                );
                id.clone()
            }
            None => {
                self.error_message = Some("No document selected".to_string());
                self.is_generating = false;
                return AppAction::None;
            }
        };

        // Get the private key from the qualified identity
        let private_key = match self.get_qualified_identity(&identity_id) {
            Some(qualified_identity) => {
                // Get the wallets for resolving encrypted keys
                let wallets = app_context.wallets.read().unwrap();
                let wallet_vec: Vec<_> = wallets.values().cloned().collect();

                // Try to get the private key
                match qualified_identity.private_keys.get_resolve(
                    &(
                        PrivateKeyTarget::PrivateKeyOnMainIdentity,
                        selected_key.id(),
                    ),
                    &wallet_vec,
                    app_context.network,
                ) {
                    Ok(Some((_, private_key_bytes))) => private_key_bytes,
                    Ok(None) => {
                        self.error_message = Some("Private key not found in storage".to_string());
                        self.is_generating = false;
                        return AppAction::None;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to get private key: {}", e));
                        self.is_generating = false;
                        return AppAction::None;
                    }
                }
            }
            None => {
                self.error_message = Some("Qualified identity not found".to_string());
                self.is_generating = false;
                return AppAction::None;
            }
        };

        let task = BackendTask::ZKProofTask(ZKProofTask::GenerateProof {
            identity_id,
            contract_id,
            document_type,
            document_id,
            private_key,
            security_level: self.security_level,
            grinding_bits: self.grinding_bits,
        });

        AppAction::BackendTask(task)
    }

    fn verify_proof(&mut self, _app_context: &AppContext) -> AppAction {
        self.is_verifying = true;
        self.error_message = None;
        self.verification_result = None; // Clear any previous results

        // Parse the proof based on input method
        let proof_result = match self.input_method {
            InputMethod::Paste => {
                // Try to parse from base64-encoded JSON first, then raw JSON
                crate::proofs::grovestark_integration::ProofDataOutput::from_base64(
                    &self.proof_text,
                )
                .or_else(|_| {
                    crate::proofs::grovestark_integration::ProofDataOutput::from_json_string(
                        &self.proof_text,
                    )
                })
            }
            InputMethod::File => {
                // TODO: Read from file
                Err(
                    crate::proofs::grovestark_integration::ProofError::DeserializationError(
                        "File input not yet implemented".to_string(),
                    ),
                )
            }
            InputMethod::Platform => {
                // TODO: Fetch from platform
                Err(
                    crate::proofs::grovestark_integration::ProofError::DeserializationError(
                        "Platform fetch not yet implemented".to_string(),
                    ),
                )
            }
        };

        match proof_result {
            Ok(proof_data) => {
                let task = BackendTask::ZKProofTask(ZKProofTask::VerifyProof {
                    proof_data,
                    security_level: self.security_level,
                });
                AppAction::BackendTask(task)
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to parse proof: {}", e));
                self.is_verifying = false;
                AppAction::None
            }
        }
    }

    fn copy_proof_to_clipboard(&self) {
        if let Some(proof) = &self.generated_proof {
            // Use the helper method to serialize to base64
            if let Ok(proof_base64) = proof.full_proof.to_base64() {
                let _ = arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.set_text(proof_base64));
            }
        }
    }

    fn save_proof_to_file(&self) {
        // TODO: Implement file save dialog
    }

    fn share_proof(&self) {
        // TODO: Implement proof sharing
    }

    fn browse_for_file(&mut self) {
        // TODO: Implement file browser dialog
    }

    fn fetch_current_state_root(&mut self, _app_context: &AppContext) {
        // TODO: Fetch current state root from platform
    }

    fn copy_verification_result(&self) {
        if let Some(result) = &self.verification_result {
            let text = format!(
                "Verification Result: {}\nContract: {}\nSecurity Level: {}-bit",
                if result.is_valid { "VALID" } else { "INVALID" },
                result.contract_id,
                result.security_level
            );
            let _ = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text));
        }
    }

    fn show_detailed_report(&self) {
        // TODO: Show detailed verification report
    }

    fn truncate_id(id: &str) -> String {
        if id.len() > 16 {
            format!("{}...{}", &id[..6], &id[id.len() - 6..])
        } else {
            id.to_string()
        }
    }

    fn format_timestamp(timestamp: u64) -> String {
        chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn render_generation_ui(&mut self, ui: &mut Ui, app_context: &AppContext) -> Option<AppAction> {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.label(
            RichText::new("Generate Zero-Knowledge Proof")
                .size(Typography::SCALE_XL)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );

        // Add informational description
        Frame::new()
            .inner_margin(Margin::same(Spacing::SM_I8))
            .fill(DashColors::glass_white(dark_mode))
            .stroke(egui::Stroke::new(1.0, DashColors::DASH_BLUE.gamma_multiply(0.3)))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::ui::helpers::info_icon_button(ui,
                        "Zero-Knowledge Proofs allow you to prove ownership of a document without revealing your identity or the document's contents. This cryptographic technique ensures privacy while maintaining verifiability.\n\nRequirement: Only EdDSA (Ed25519) keys are supported for ZK proof generation.");
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("What are ZK Proofs?")
                                .strong()
                                .color(DashColors::text_primary(dark_mode))
                        );
                        ui.label(
                            RichText::new("Prove you own a document in a specific contract without revealing your identity or the document contents.")
                                .size(Typography::SCALE_SM)
                                .color(DashColors::text_secondary(dark_mode))
                        );
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("📝 Requirement:")
                                    .size(Typography::SCALE_XS)
                                    .strong()
                                    .color(DashColors::text_secondary(dark_mode))
                            );
                            ui.label(
                                RichText::new("Only EdDSA keys are supported for ZK proof generation.")
                                    .size(Typography::SCALE_XS)
                                    .color(DashColors::text_secondary(dark_mode))
                            );
                        });
                    });
                });
            });

        ui.add_space(Spacing::MD);
        ui.separator();

        // Step 1: Select Identity
        Frame::new()
            .inner_margin(Margin::same(Spacing::MD_I8))
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Step 1: Select Identity")
                        .size(Typography::SCALE_LG)
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.horizontal(|ui| {
                    ui.label("Identity:");
                    let mut identity_changed = false;
                    ComboBox::from_id_salt("identity_selector")
                        .selected_text(self.selected_identity.as_deref().unwrap_or(
                            if self.available_identities.is_empty() {
                                "No identities available"
                            } else {
                                "Select..."
                            },
                        ))
                        .show_ui(ui, |ui| {
                            if self.available_identities.is_empty() {
                                ui.label("No identities with EdDSA keys found.");
                                ui.label(
                                    RichText::new("ZK proofs require identities with EdDSA (Ed25519) keys. Please add an EdDSA key to an identity.")
                                        .size(Typography::SCALE_XS)
                                        .color(DashColors::text_secondary(dark_mode))
                                );
                            } else {
                                for identity in &self.available_identities {
                                    let id_str = identity.id().to_string(Encoding::Base58);
                                    if ui
                                        .selectable_value(
                                            &mut self.selected_identity,
                                            Some(id_str.clone()),
                                            Self::truncate_id(&id_str),
                                        )
                                        .changed()
                                    {
                                        identity_changed = true;
                                    }
                                }
                            }
                        });

                    // Reset key selection if identity changed
                    if identity_changed {
                        self.selected_key = None;
                    }
                });

                if let Some(id) = &self.selected_identity {
                    ui.label(
                        RichText::new("✅ Identity selected").color(egui::Color32::DARK_GREEN),
                    );

                    // Key selection
                    ui.separator();
                    ui.label(
                        RichText::new("Select Key for Signing:")
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    let available_keys: Vec<IdentityPublicKey> =
                        self.get_available_keys(id).into_iter().cloned().collect();

                    if available_keys.is_empty() {
                        ui.label(
                            RichText::new("⚠️ No EdDSA keys available for ZK proof generation")
                                .color(egui::Color32::YELLOW),
                        );
                        ui.label(
                            RichText::new("ZK proofs require EdDSA (Ed25519) keys. Please add an EdDSA key to this identity.")
                                .size(Typography::SCALE_XS)
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    } else {
                        ComboBox::from_id_salt("key_selector")
                            .selected_text(
                                self.selected_key
                                    .as_ref()
                                    .map(|k| {
                                        format!(
                                            "EdDSA Key {} ({} - {})",
                                            k.id(),
                                            k.purpose(),
                                            k.security_level()
                                        )
                                    })
                                    .unwrap_or_else(|| "Select key...".to_string()),
                            )
                            .show_ui(ui, |ui| {
                                for key in &available_keys {
                                    let key_label = format!(
                                        "EdDSA Key {} ({} - {})",
                                        key.id(),
                                        key.purpose(),
                                        key.security_level()
                                    );
                                    ui.selectable_value(
                                        &mut self.selected_key,
                                        Some(key.clone()),
                                        key_label,
                                    );
                                }
                            });

                        if self.selected_key.is_some() {
                            ui.label(
                                RichText::new("✅ EdDSA key selected").color(egui::Color32::DARK_GREEN),
                            );
                        }
                    }
                }
            });

        ui.add_space(Spacing::MD);

        // Step 2: Select Contract
        Frame::new()
            .inner_margin(Margin::same(Spacing::MD_I8))
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Step 2: Select Contract")
                        .size(Typography::SCALE_LG)
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.horizontal(|ui| {
                    ui.label("Contract:");
                    let mut contract_changed = false;
                    ComboBox::from_id_salt("contract_selector")
                        .selected_text(self.selected_contract.as_deref().unwrap_or(
                            if self.available_contracts.is_empty() {
                                "No contracts available"
                            } else {
                                "Select..."
                            },
                        ))
                        .show_ui(ui, |ui| {
                            if self.available_contracts.is_empty() {
                                ui.label(
                                    "No user contracts found. Please create a contract first.",
                                );
                            } else {
                                for (id, name) in &self.available_contracts {
                                    if ui
                                        .selectable_value(
                                            &mut self.selected_contract,
                                            Some(id.clone()),
                                            name,
                                        )
                                        .changed()
                                    {
                                        contract_changed = true;
                                    }
                                }
                            }
                        });

                    // If contract changed, refresh document types
                    if contract_changed {
                        if let Some(contract_id) = self.selected_contract.clone() {
                            self.refresh_document_types(app_context, &contract_id);
                        }
                    }
                });

                if let Some(contract_id) = &self.selected_contract {
                    ui.label(
                        RichText::new("✅ Contract selected").color(egui::Color32::DARK_GREEN),
                    );

                    // Document Type selection
                    ui.separator();
                    ui.label(
                        RichText::new("Select Document Type:")
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    ui.horizontal(|ui| {
                        ui.label("Document Type:");
                        ComboBox::from_id_salt("document_type_selector")
                            .selected_text(self.selected_document_type.as_deref().unwrap_or(
                                if self.available_document_types.is_empty() {
                                    "No document types available"
                                } else {
                                    "Select..."
                                },
                            ))
                            .show_ui(ui, |ui| {
                                if self.available_document_types.is_empty() {
                                    ui.label("No document types found for this contract.");
                                } else {
                                    for doc_type in &self.available_document_types {
                                        ui.selectable_value(
                                            &mut self.selected_document_type,
                                            Some(doc_type.clone()),
                                            doc_type,
                                        );
                                    }
                                }
                            });
                    });

                    if self.selected_document_type.is_some() {
                        ui.label(
                            RichText::new("✅ Document type selected")
                                .color(egui::Color32::DARK_GREEN),
                        );
                    }
                }
            });

        ui.add_space(Spacing::MD);

        // Step 4: Select Document
        Frame::new()
            .inner_margin(Margin::same(Spacing::MD_I8))
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Step 4: Select Document")
                        .size(Typography::SCALE_LG)
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.horizontal(|ui| {
                    ui.label("Document ID:");
                    let mut document_id =
                        self.selected_document.as_deref().unwrap_or("").to_string();
                    if ui.text_edit_singleline(&mut document_id).changed() {
                        self.selected_document = if document_id.is_empty() {
                            None
                        } else {
                            Some(document_id)
                        };
                    }
                });

                if let Some(doc_id) = &self.selected_document {
                    ui.label(
                        RichText::new("✅ Document selected").color(egui::Color32::DARK_GREEN),
                    );
                }
            });

        ui.separator();

        // Advanced Options
        if render_collapsing_header(ui, "Advanced Options", self.advanced_expanded) {
            self.advanced_expanded = !self.advanced_expanded;
        }

        if self.advanced_expanded {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Security Level:");
                    crate::ui::helpers::info_icon_button(ui,
                        "Security Level determines the cryptographic strength of the zero-knowledge proof:\n\n\
                        • 128-bit: Provides 128 bits of security, suitable for most applications. Faster proof generation and smaller proof size.\n\n\
                        • 192-bit: Provides 192 bits of security for applications requiring higher cryptographic assurance. Slower proof generation and larger proof size.");
                    ui.radio_value(&mut self.security_level, 128, "128-bit");
                    ui.radio_value(&mut self.security_level, 192, "192-bit");
                });

                ui.horizontal(|ui| {
                    ui.label("Grinding Bits:");
                    crate::ui::helpers::info_icon_button(ui,
                        "Grinding Bits control the computational work required for proof generation:\n\n\
                        • Lower values (10-15): Faster proof generation but less security against certain attacks\n\n\
                        • Higher values (16-20): Slower proof generation but increased security\n\n\
                        • Default value of 16 provides a good balance between security and performance\n\n\
                        This parameter affects the 'grinding' process where the prover searches for a nonce that satisfies certain constraints.");
                    ui.add(egui::Slider::new(&mut self.grinding_bits, 10..=20));
                });
            });
        }

        ui.separator();

        // Generate Button
        let can_generate = self.selected_identity.is_some()
            && self.selected_key.is_some()
            && self.selected_contract.is_some()
            && self.selected_document_type.is_some()
            && self.selected_document.is_some();

        let mut action = None;
        ui.horizontal(|ui| {
            if self.is_generating {
                ui.spinner();
                ui.vertical(|ui| {
                    ui.label("🔄 Generating ZK proof...");
                });
            } else {
                if ui
                    .add_enabled(can_generate, Button::new("🔐 Generate Proof"))
                    .clicked()
                {
                    action = Some(self.generate_proof(app_context));
                }
            }
        });
        if action.is_some() {
            return action;
        }

        // Error Display
        if let Some(error) = &self.error_message {
            ui.colored_label(egui::Color32::RED, format!("❌ Error: {}", error));
        }

        // Success Display
        if let Some(proof) = &self.generated_proof {
            ui.separator();
            Frame::new()
                .inner_margin(Margin::same(Spacing::MD_I8))
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, egui::Color32::DARK_GREEN))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("✅ Proof Generated Successfully!")
                            .color(egui::Color32::DARK_GREEN)
                            .strong(),
                    );

                    ui.horizontal(|ui| {
                        if ui.button("📋 Copy Proof").clicked() {
                            self.copy_proof_to_clipboard();
                        }

                        if ui.button("💾 Save to File").clicked() {
                            self.save_proof_to_file();
                        }
                    });
                });
        }
        None
    }

    fn render_verification_ui(
        &mut self,
        ui: &mut Ui,
        app_context: &AppContext,
    ) -> Option<AppAction> {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.label(
            RichText::new("Verify Zero-Knowledge Proof")
                .size(Typography::SCALE_XL)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.separator();

        // Input Method Selection
        Frame::new()
            .inner_margin(Margin::same(Spacing::MD_I8))
            .fill(DashColors::surface(dark_mode))
            .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Select Proof Input Method:")
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            matches!(self.input_method, InputMethod::Paste),
                            "📋 Paste",
                        )
                        .clicked()
                    {
                        self.input_method = InputMethod::Paste;
                    }
                    if ui
                        .selectable_label(matches!(self.input_method, InputMethod::File), "📁 File")
                        .clicked()
                    {
                        self.input_method = InputMethod::File;
                    }
                    if ui
                        .selectable_label(
                            matches!(self.input_method, InputMethod::Platform),
                            "🌐 Platform",
                        )
                        .clicked()
                    {
                        self.input_method = InputMethod::Platform;
                    }
                });
            });

        ui.add_space(Spacing::MD);

        // Proof Input
        match self.input_method {
            InputMethod::Paste => {
                Frame::new()
                    .inner_margin(Margin::same(Spacing::MD_I8))
                    .fill(DashColors::surface(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Paste Proof (Base64 or Hex):")
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add(
                            TextEdit::multiline(&mut self.proof_text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(6),
                        );
                    });
            }
            InputMethod::File => {
                Frame::new()
                    .inner_margin(Margin::same(Spacing::MD_I8))
                    .fill(DashColors::surface(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Proof File:")
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            if ui.button("Browse...").clicked() {
                                self.browse_for_file();
                            }
                        });

                        if let Some(path) = &self.proof_file_path {
                            ui.label(format!("Selected: {}", path.display()));
                        }
                    });
            }
            InputMethod::Platform => {
                Frame::new()
                    .inner_margin(Margin::same(Spacing::MD_I8))
                    .fill(DashColors::surface(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Enter Proof ID from Platform:")
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.text_edit_singleline(&mut self.proof_id);
                    });
            }
        }

        ui.separator();

        // Public Inputs (collapsible)
        if render_collapsing_header(ui, "Public Inputs (Optional)", self.public_inputs_expanded) {
            self.public_inputs_expanded = !self.public_inputs_expanded;
        }

        if self.public_inputs_expanded {
            Frame::new()
                .inner_margin(Margin::same(Spacing::MD_I8))
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Override auto-detected values if needed:")
                            .size(Typography::SCALE_SM)
                            .color(DashColors::text_secondary(dark_mode)),
                    );

                    ui.horizontal(|ui| {
                        ui.label("State Root:");
                        ui.text_edit_singleline(&mut self.state_root);
                        if ui.button("Current").clicked() {
                            self.fetch_current_state_root(app_context);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Contract ID:");
                        ui.text_edit_singleline(&mut self.contract_id);
                    });

                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.check_timestamp, "Verify Timestamp");
                        if self.check_timestamp {
                            ui.label("Expected:");
                            ui.add(egui::DragValue::new(&mut self.expected_timestamp));
                        }
                    });
                });
        }

        ui.separator();

        // Error Display (above the button)
        if let Some(error) = &self.error_message {
            ui.colored_label(egui::Color32::RED, format!("❌ Error: {}", error));
        }

        // Verify Button
        let can_verify = match self.input_method {
            InputMethod::Paste => !self.proof_text.is_empty(),
            InputMethod::File => self.proof_file_path.is_some(),
            InputMethod::Platform => !self.proof_id.is_empty(),
        };

        let mut action = None;
        ui.horizontal(|ui| {
            if self.is_verifying {
                ui.spinner();
                ui.label("Verifying proof...");
            } else {
                if ui
                    .add_enabled(can_verify, Button::new("✅ Verify Proof"))
                    .clicked()
                {
                    action = Some(self.verify_proof(app_context));
                }
            }
        });
        if action.is_some() {
            return action;
        }

        // Verification Result
        if let Some(result) = &self.verification_result {
            ui.separator();

            if result.is_valid {
                Frame::new()
                    .inner_margin(Margin::same(Spacing::MD_I8))
                    .fill(DashColors::surface(dark_mode))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::DARK_GREEN))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .show(ui, |ui| {
                        ui.colored_label(egui::Color32::DARK_GREEN, "✅ PROOF IS VALID");

                        Grid::new("verification_details")
                            .num_columns(2)
                            .show(ui, |ui| {
                                ui.label("Verified At:");
                                ui.label(Self::format_timestamp(result.verified_at));
                                ui.end_row();

                                ui.label("Document Exists:");
                                ui.label("Yes");
                                ui.end_row();

                                ui.label("Key Control:");
                                ui.label("Verified");
                                ui.end_row();

                                ui.label("Contract:");
                                ui.label(Self::truncate_id(&result.contract_id));
                                ui.end_row();

                                ui.label("Security Level:");
                                ui.label(format!("{}-bit", result.security_level));
                                ui.end_row();
                            });

                        ui.horizontal(|ui| {
                            if ui.button("📋 Copy Result").clicked() {
                                self.copy_verification_result();
                            }

                            if ui.button("📊 View Details").clicked() {
                                self.show_detailed_report();
                            }
                        });
                    });
            } else {
                Frame::new()
                    .inner_margin(Margin::same(Spacing::MD_I8))
                    .fill(DashColors::surface(dark_mode))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::RED))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .show(ui, |ui| {
                        ui.colored_label(egui::Color32::RED, "❌ PROOF IS INVALID");
                        if let Some(reason) = &result.error_message {
                            ui.label(format!("Reason: {}", reason));
                        }

                        ui.collapsing("Technical Details", |ui| {
                            ui.monospace(&result.technical_details);
                        });
                    });
            }
        }
        None
    }
}

impl ScreenLike for ZKProofsScreen {
    fn refresh(&mut self) {
        // Refresh implementation if needed
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
        // Reload data in case it changed
        let app_context = self.app_context.clone();
        self.refresh_identities(&app_context);
        self.refresh_contracts(&app_context);

        tracing::info!(
            "ZK Proofs refresh_on_arrival complete: {} identities, {} contracts",
            self.qualified_identities.len(),
            self.available_contracts.len()
        );
    }

    fn display_message(&mut self, message: &str, message_type: crate::ui::MessageType) {
        // Always show error messages
        self.error_message = Some(message.to_string());

        // Reset loading states when an error occurs
        if message_type == crate::ui::MessageType::Error {
            self.is_generating = false;
            self.is_verifying = false;
        }
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::backend_task::BackendTaskSuccessResult,
    ) {
        use crate::backend_task::BackendTaskSuccessResult;

        match backend_task_success_result {
            BackendTaskSuccessResult::GeneratedZKProof(proof_data) => {
                self.is_generating = false;
                let proof_size = proof_data.proof.len();
                self.generated_proof = Some(ProofData {
                    full_proof: proof_data.clone(),
                    hash: hex::encode(&proof_data.public_inputs.state_root[0..8]),
                    size: proof_size,
                    generation_time: std::time::Duration::from_millis(
                        proof_data.metadata.generation_time_ms,
                    ),
                });
                self.proof_size = Some(format!("{} bytes", proof_data.metadata.proof_size));
                self.generation_time = Some(std::time::Duration::from_millis(
                    proof_data.metadata.generation_time_ms,
                ));
                self.error_message = None;
            }
            BackendTaskSuccessResult::VerifiedZKProof(is_valid, proof_data) => {
                self.is_verifying = false;
                // Get contract ID from the proof data itself
                let contract_id = hex::encode(&proof_data.public_inputs.contract_id);
                self.verification_result = Some(VerificationResult {
                    is_valid,
                    verified_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    contract_id,
                    security_level: self.security_level,
                    error_message: if !is_valid {
                        Some("Proof verification failed".to_string())
                    } else {
                        None
                    },
                    technical_details: format!(
                        "Verification result: {}",
                        if is_valid { "VALID" } else { "INVALID" }
                    ),
                });
                self.error_message = None;
            }
            _ => {}
        }
    }

    fn pop_on_success(&mut self) {
        // Pop on success if needed
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel with breadcrumb
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![("Tools", AppAction::None)],
            vec![],
        );

        // Add left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenToolsZKProofsScreen,
        );

        // Add tools subscreen chooser panel
        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        // Add central panel with the main UI
        let panel_action = island_central_panel(ctx, |ui| {
            ui.label(
                RichText::new("Zero-Knowledge Proofs")
                    .size(Typography::SCALE_XL)
                    .strong()
                    .color(DashColors::text_primary(ui.ctx().style().visuals.dark_mode)),
            );
            ui.separator();

            let mut content_action = AppAction::None;
            let available_height = ui.available_height();

            // Mode Toggle at the top
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Mode:")
                        .size(Typography::SCALE_LG)
                        .strong()
                        .color(DashColors::text_primary(ui.ctx().style().visuals.dark_mode)),
                );
                ui.add_space(10.0);

                let dark_mode = ui.ctx().style().visuals.dark_mode;

                // Generate button
                let generate_selected = self.mode == ProofMode::Generate;
                let generate_button = if generate_selected {
                    Button::new(
                        RichText::new("🔐 Generate Proof")
                            .color(DashColors::WHITE)
                            .size(Typography::SCALE_SM),
                    )
                    .fill(DashColors::DASH_BLUE)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .min_size(egui::Vec2::new(150.0, 28.0))
                } else {
                    Button::new(
                        RichText::new("🔐 Generate Proof")
                            .color(DashColors::text_primary(dark_mode))
                            .size(Typography::SCALE_SM),
                    )
                    .fill(DashColors::glass_white(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border(dark_mode)))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .min_size(egui::Vec2::new(150.0, 28.0))
                };

                if ui.add(generate_button).clicked() {
                    self.mode = ProofMode::Generate;
                }

                ui.add_space(5.0);

                // Verify button
                let verify_selected = self.mode == ProofMode::Verify;
                let verify_button = if verify_selected {
                    Button::new(
                        RichText::new("✅ Verify Proof")
                            .color(DashColors::WHITE)
                            .size(Typography::SCALE_SM),
                    )
                    .fill(DashColors::DASH_BLUE)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .min_size(egui::Vec2::new(150.0, 28.0))
                } else {
                    Button::new(
                        RichText::new("✅ Verify Proof")
                            .color(DashColors::text_primary(dark_mode))
                            .size(Typography::SCALE_SM),
                    )
                    .fill(DashColors::glass_white(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border(dark_mode)))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                    .min_size(egui::Vec2::new(150.0, 28.0))
                };

                if ui.add(verify_button).clicked() {
                    self.mode = ProofMode::Verify;
                }
            });

            ui.separator();
            ui.add_space(Spacing::MD);

            // Main content area with scrolling
            ScrollArea::vertical()
                .max_height(available_height - 100.0) // Reserve space for mode toggle and margins
                .show(ui, |ui| {
                    // Clone app_context to avoid borrowing issues
                    let app_context = self.app_context.clone();
                    // Render the appropriate UI based on mode
                    let maybe_action = match self.mode {
                        ProofMode::Generate => self.render_generation_ui(ui, &app_context),
                        ProofMode::Verify => self.render_verification_ui(ui, &app_context),
                    };
                    if let Some(ui_action) = maybe_action {
                        content_action |= ui_action;
                    }
                });

            content_action
        });

        action |= panel_action;

        // Note: Confirmation dialog handling would be done within the UI context if needed

        action
    }
}
