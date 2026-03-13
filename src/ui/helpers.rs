use crate::ui::theme::ResponseExt;
use dash_sdk::dpp::address_funds::{PLATFORM_HRP_MAINNET, PLATFORM_HRP_TESTNET};
use std::sync::Arc;

/// Returns true if the user left-clicked outside the given window rect this frame.
/// Use after painting a modal overlay and showing the dialog window.
pub fn clicked_outside_window(ctx: &egui::Context, window_rect: egui::Rect) -> bool {
    ctx.input(|i| {
        i.pointer.primary_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| !window_rect.contains(pos))
    })
}

/// Checks if a string looks like a Platform address (bech32m with dash/tdash HRP per DIP-18).
///
/// This checks whether the string starts with a known Platform HRP followed by the
/// bech32 separator '1'. It does NOT fully validate the address — use
/// `PlatformAddress::from_bech32m_string()` for that.
pub fn is_platform_address_string(s: &str) -> bool {
    let s = s.to_lowercase();
    for hrp in [PLATFORM_HRP_MAINNET, PLATFORM_HRP_TESTNET] {
        if s.starts_with(hrp) && s.get(hrp.len()..hrp.len() + 1) == Some("1") {
            return true;
        }
    }
    false
}

use crate::{
    app::AppAction,
    context::AppContext,
    model::{qualified_contract::QualifiedContract, qualified_identity::QualifiedIdentity},
    ui::contracts_documents::group_actions_screen::GroupActionsScreen,
    ui::theme::DashColors,
    ui::{RootScreenType, Screen, identities::keys::add_key_screen::AddKeyScreen},
};
use arboard::Clipboard;
use dash_sdk::{
    dpp::{
        data_contract::{
            GroupContractPosition,
            accessors::v0::DataContractV0Getters,
            accessors::v1::DataContractV1Getters,
            associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters,
            change_control_rules::authorized_action_takers::AuthorizedActionTakers,
            document_type::{DocumentType, accessors::DocumentTypeV0Getters},
            group::{Group, accessors::v0::GroupV0Getters},
        },
        identity::{
            Purpose, SecurityLevel, accessors::IdentityGettersV0,
            identity_public_key::accessors::v0::IdentityPublicKeyGettersV0,
        },
        platform_value::string_encoding::Encoding,
    },
    platform::{Identifier, IdentityPublicKey},
};
use egui::{Color32, ComboBox, Response, Ui};

use super::tokens::tokens_screen::IdentityTokenInfo;

/// Layout of labels and buttons in the UI fails to vertically align properly containers that contain buttons and other items (labels, text fields, etc.).
/// This constant provides a constant padding to be used in such cases to ensure proper alignment.
pub const BUTTON_ADJUSTMENT_PADDING_TOP: f32 = 15.0;

/// Formats a key label for display in combo boxes and lists.
/// Returns a string like "Key 0 | AUTHENTICATION | CRITICAL | ECDSA_SECP256K1"
pub fn format_key_label(key: &IdentityPublicKey) -> String {
    format!(
        "Key {} | {} | {} | {}",
        key.id(),
        key.purpose(),
        key.security_level(),
        key.key_type()
    )
}

/// Formats a key label with a [DEV] suffix for dev mode display.
pub fn format_key_label_dev(key: &IdentityPublicKey) -> String {
    format!(
        "Key {} | {} | {} | {} [DEV]",
        key.id(),
        key.purpose(),
        key.security_level(),
        key.key_type()
    )
}

/// Returns the display label for a QualifiedIdentity (alias or Base58 ID).
pub fn identity_display_label(identity: &QualifiedIdentity) -> String {
    identity
        .alias
        .clone()
        .unwrap_or_else(|| identity.identity.id().to_string(Encoding::Base58))
}

/// Returns the display label for a QualifiedContract (alias or Base58 ID).
pub fn contract_display_label(contract: &QualifiedContract) -> String {
    contract
        .alias
        .clone()
        .unwrap_or_else(|| contract.contract.id().to_string(Encoding::Base58))
}

/// Computes the allowed security levels for a given transaction type and optional document type.
/// This centralizes the logic that was previously duplicated in multiple places.
pub fn compute_allowed_security_levels(
    transaction_type: TransactionType,
    document_type: Option<&DocumentType>,
) -> Vec<SecurityLevel> {
    match (transaction_type, document_type) {
        (TransactionType::DocumentAction, Some(doc_type)) => {
            let required_level = doc_type.security_level_requirement();
            let allowed_range = SecurityLevel::CRITICAL as u8..=required_level as u8;
            [
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]
            .into_iter()
            .filter(|level| allowed_range.contains(&(*level as u8)))
            .collect()
        }
        _ => transaction_type.allowed_security_levels(),
    }
}

/// Result of checking token action authorization.
/// Contains the group if applicable and any error message.
pub struct TokenAuthorizationResult {
    pub group: Option<(GroupContractPosition, Group)>,
    pub error_message: Option<String>,
    pub is_unilateral_group_member: bool,
}

/// Checks if the given identity is authorized to perform a token action.
/// This centralizes the authorization checking logic used across token screens (mint, burn, pause, etc.).
///
/// # Arguments
/// * `action_takers` - The authorized action takers for the operation
/// * `identity_token_info` - The token and identity information
/// * `action_name` - Human-readable name of the action (e.g., "mint", "burn")
///
/// # Returns
/// A `TokenAuthorizationResult` containing the group (if applicable), any error message,
/// and whether the user is a unilateral group member.
pub fn check_token_authorization(
    action_takers: &AuthorizedActionTakers,
    identity_token_info: &IdentityTokenInfo,
    action_name: &str,
) -> TokenAuthorizationResult {
    let mut error_message = None;

    let group = match action_takers {
        AuthorizedActionTakers::NoOne => {
            error_message = Some(format!("{} is not allowed on this token", action_name));
            None
        }
        AuthorizedActionTakers::ContractOwner => {
            if identity_token_info.data_contract.contract.owner_id()
                != identity_token_info.identity.identity.id()
            {
                error_message = Some(format!(
                    "You are not allowed to {} this token. Only the contract owner is.",
                    action_name.to_lowercase()
                ));
            }
            None
        }
        AuthorizedActionTakers::Identity(identifier) => {
            if identifier != &identity_token_info.identity.identity.id() {
                error_message = Some(format!(
                    "You are not allowed to {} this token",
                    action_name.to_lowercase()
                ));
            }
            None
        }
        AuthorizedActionTakers::MainGroup => {
            match identity_token_info.token_config.main_control_group() {
                None => {
                    error_message = Some(
                        "Invalid contract: No main control group, though one should exist"
                            .to_string(),
                    );
                    None
                }
                Some(group_pos) => {
                    match identity_token_info
                        .data_contract
                        .contract
                        .expected_group(group_pos)
                    {
                        Ok(group) => Some((group_pos, group.clone())),
                        Err(e) => {
                            error_message = Some(format!("Invalid contract: {}", e));
                            None
                        }
                    }
                }
            }
        }
        AuthorizedActionTakers::Group(group_pos) => {
            match identity_token_info
                .data_contract
                .contract
                .expected_group(*group_pos)
            {
                Ok(group) => Some((*group_pos, group.clone())),
                Err(e) => {
                    error_message = Some(format!("Invalid contract: {}", e));
                    None
                }
            }
        }
    };

    let is_unilateral_group_member = if let Some((_, ref g)) = group {
        g.members()
            .get(&identity_token_info.identity.identity.id())
            .map(|power| *power >= g.required_power())
            .unwrap_or(false)
    } else {
        false
    };

    TokenAuthorizationResult {
        group,
        error_message,
        is_unilateral_group_member,
    }
}

/// Helper function to create a styled info icon button with a circle and "i"
/// Returns a Response that can be checked for .clicked() to show an info popup
pub fn info_icon_button(ui: &mut egui::Ui, hover_text: &str) -> Response {
    let size = 16.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let is_hovered = response.hovered();
        let color = if is_hovered {
            Color32::from_rgb(100, 180, 255) // Brighter blue on hover
        } else {
            Color32::from_rgb(70, 130, 180) // Steel blue
        };

        let center = rect.center();
        let radius = size / 2.0 - 1.0;

        // Draw circle outline
        ui.painter()
            .circle_stroke(center, radius, egui::Stroke::new(1.5, color));

        // Draw "i" text in the center
        ui.painter().text(
            center,
            egui::Align2::CENTER_CENTER,
            "i",
            egui::FontId::proportional(11.0),
            color,
        );
    }

    response.clickable_tooltip(hover_text)
}

pub fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// Returns the newly selected key (if changed), otherwise the existing one.
// Allow dead_code: This function provides UI for key selection within identities,
// useful for identity-based operations and key management interfaces
pub fn render_key_selector(
    ui: &mut Ui,
    selected_identity: &QualifiedIdentity,
    selected_key: &Option<IdentityPublicKey>,
) -> Option<IdentityPublicKey> {
    let mut new_selected_key = selected_key.clone();

    ui.horizontal(|ui| {
        ui.label("Key:");
        ComboBox::from_id_salt("key_selector")
            .selected_text(
                selected_key
                    .as_ref()
                    .map(|k| format!("Key {} Security {}", k.id(), k.security_level()))
                    .unwrap_or_else(|| "Choose key…".into()),
            )
            .show_ui(ui, |cb| {
                for key_ref in selected_identity.available_authentication_keys_non_master() {
                    let key = &key_ref.identity_public_key;
                    let label = format!("Key {} Security {}", key.id(), key.security_level());
                    if cb
                        .selectable_label(Some(key) == selected_key.as_ref(), label)
                        .clicked()
                    {
                        new_selected_key = Some(key.clone());
                    }
                }
            });
    });

    new_selected_key
}

/// Transaction types that require specific key filtering
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransactionType {
    /// Register a new data contract - requires Authentication keys with High or Critical security level
    RegisterContract,
    /// Update an existing data contract - requires Authentication keys with Critical security level only
    UpdateContract,
    /// Transfer credits - requires Transfer purpose keys
    Transfer,
    /// Withdraw from identity - requires Transfer or Owner purpose keys
    Withdraw,
    /// Generic document creation/update - security level depends on document type
    DocumentAction,
    /// Token actions such as minting - requires Critical Authentication keys
    TokenAction,
    /// Token action of transferring tokens
    TokenTransfer,
    /// Token action of claiming
    TokenClaim,
    /// DashPay contact request - requires Authentication keys for signing (ENCRYPTION key for ECDH is auto-selected)
    ContactRequest,
}

impl TransactionType {
    /// Returns the allowed purposes for this transaction type
    pub fn allowed_purposes(&self) -> Vec<Purpose> {
        match self {
            TransactionType::RegisterContract | TransactionType::UpdateContract => {
                vec![Purpose::AUTHENTICATION]
            }
            TransactionType::Transfer => vec![Purpose::TRANSFER],
            TransactionType::Withdraw => vec![Purpose::TRANSFER, Purpose::OWNER], // Owner keys handled separately
            TransactionType::DocumentAction | TransactionType::TokenAction => {
                vec![Purpose::AUTHENTICATION]
            }
            TransactionType::TokenTransfer | TransactionType::TokenClaim => {
                vec![Purpose::TRANSFER, Purpose::AUTHENTICATION]
            }
            TransactionType::ContactRequest => vec![Purpose::AUTHENTICATION],
        }
    }

    /// Returns the allowed security levels for this transaction type
    pub fn allowed_security_levels(&self) -> Vec<SecurityLevel> {
        match self {
            TransactionType::RegisterContract => vec![SecurityLevel::CRITICAL, SecurityLevel::HIGH],
            TransactionType::UpdateContract => vec![SecurityLevel::CRITICAL],
            TransactionType::Transfer => vec![SecurityLevel::CRITICAL],
            TransactionType::Withdraw => vec![SecurityLevel::CRITICAL],
            TransactionType::DocumentAction => vec![
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ],
            TransactionType::TokenAction
            | TransactionType::TokenTransfer
            | TransactionType::TokenClaim => vec![SecurityLevel::CRITICAL],
            TransactionType::ContactRequest => vec![SecurityLevel::CRITICAL, SecurityLevel::HIGH],
        }
    }

    /// Returns a descriptive label for the transaction type
    pub fn label(&self) -> &'static str {
        match self {
            TransactionType::RegisterContract => "Register Contract",
            TransactionType::UpdateContract => "Update Contract",
            TransactionType::Transfer => "Transfer",
            TransactionType::Withdraw => "Withdraw",
            TransactionType::DocumentAction => "Document Action",
            TransactionType::TokenAction => "Token Action",
            TransactionType::TokenTransfer => "Token Transfer",
            TransactionType::TokenClaim => "Token Claim",
            TransactionType::ContactRequest => "Contact Request",
        }
    }
}

/// Key chooser that filters keys based on transaction type and dev mode.
/// Use this when you already have a specific identity and just need to select a key.
pub fn add_key_chooser(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    selected_key: &mut Option<IdentityPublicKey>,
    transaction_type: TransactionType,
) -> AppAction {
    add_key_chooser_with_doc_type(
        ui,
        app_context,
        identity,
        selected_key,
        transaction_type,
        None,
    )
}

/// Key chooser that filters keys based on transaction type, document type and dev mode.
/// Use this when you already have a specific identity and just need to select a key.
pub fn add_key_chooser_with_doc_type(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    selected_key: &mut Option<IdentityPublicKey>,
    transaction_type: TransactionType,
    document_type: Option<&DocumentType>,
) -> AppAction {
    let is_dev_mode = app_context.is_developer_mode();
    let mut action = AppAction::None;

    let allowed_purposes = transaction_type.allowed_purposes();
    let allowed_security_levels = compute_allowed_security_levels(transaction_type, document_type);

    // Check for keys with private keys loaded
    let has_suitable_keys_with_private =
        identity
            .private_keys
            .identity_public_keys()
            .iter()
            .any(|key_ref| {
                let key = &key_ref.1.identity_public_key;

                allowed_purposes.contains(&key.purpose())
                    && allowed_security_levels.contains(&key.security_level())
            });

    // Check if there are eligible public keys without private keys
    let has_eligible_public_keys_without_private =
        identity.identity.public_keys().iter().any(|(_, pub_key)| {
            let basic_ok = allowed_purposes.contains(&pub_key.purpose())
                && allowed_security_levels.contains(&pub_key.security_level());

            let has_private = identity
                .private_keys
                .identity_public_keys()
                .iter()
                .any(|key_ref| key_ref.1.identity_public_key.id() == pub_key.id());

            basic_ok && !has_private
        });

    if !is_dev_mode && !has_suitable_keys_with_private {
        // Show message and buttons when no suitable keys
        ui.group(|ui| {
            ui.set_min_width(220.0);
            ui.vertical(|ui| {
                ui.label("No eligible key. This transaction type requires:");
                ui.label(format!("{} key", transaction_type.label()));

                if has_eligible_public_keys_without_private {
                    ui.label(
                        "This Identity has an eligible public key but the private key isn't loaded.",
                    );
                }

                ui.add_space(5.0);

                if ui.button("Add New Key to Identity").clicked() {
                    action = AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        identity.clone(),
                        app_context,
                    )));
                }
            });
        });
    } else {
        // Show key combo box
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(15.0);
                ui.label("Key:");
            });
            ComboBox::from_id_salt("key_chooser_combo")
                .width(300.0)
                .selected_text(
                    selected_key
                        .as_ref()
                        .map(format_key_label)
                        .unwrap_or_else(|| "Select Key...".into()),
                )
                .show_ui(ui, |kui| {
                    for key_ref in identity.private_keys.identity_public_keys() {
                        let key = &key_ref.1.identity_public_key;

                        let is_allowed = is_dev_mode
                            || (allowed_purposes.contains(&key.purpose())
                                && allowed_security_levels.contains(&key.security_level()));

                        if is_allowed {
                            let is_dev_override = is_dev_mode
                                && (!allowed_purposes.contains(&key.purpose())
                                    || !allowed_security_levels.contains(&key.security_level()));
                            let label = if is_dev_override {
                                format_key_label_dev(key)
                            } else {
                                format_key_label(key)
                            };

                            if kui
                                .selectable_label(selected_key.as_ref() == Some(key), label)
                                .clicked()
                            {
                                *selected_key = Some(key.clone());
                            }
                        }
                    }
                });
        });
    }

    action
}

/// Identity key chooser that filters keys based on transaction type and dev mode
pub fn add_identity_key_chooser<'a, T>(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identities: T,
    selected_identity: &mut Option<QualifiedIdentity>,
    selected_key: &mut Option<IdentityPublicKey>,
    transaction_type: TransactionType,
) -> AppAction
where
    T: Iterator<Item = &'a QualifiedIdentity>,
{
    add_identity_key_chooser_with_doc_type(
        ui,
        app_context,
        identities,
        selected_identity,
        selected_key,
        transaction_type,
        None,
    )
}

/// Identity key chooser that filters keys based on transaction type, document type and dev mode
pub fn add_identity_key_chooser_with_doc_type<'a, T>(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identities: T,
    selected_identity: &mut Option<QualifiedIdentity>,
    selected_key: &mut Option<IdentityPublicKey>,
    transaction_type: TransactionType,
    document_type: Option<&DocumentType>,
) -> AppAction
where
    T: Iterator<Item = &'a QualifiedIdentity>,
{
    let is_dev_mode = app_context.is_developer_mode();
    let mut action = AppAction::None;

    egui::Grid::new("identity_key_chooser_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .striped(false)
        .show(ui, |ui| {
            ui.label("Identity:");
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ComboBox::from_id_salt("identity_combo")
                    .width(220.0)
                    .selected_text(
                        selected_identity
                            .as_ref()
                            .map(identity_display_label)
                            .unwrap_or_else(|| "Select Identity…".into()),
                    )
                    .show_ui(ui, |iui| {
                        for qi in identities {
                            let label = identity_display_label(qi);
                            if iui
                                .selectable_label(
                                    selected_identity.as_ref() == Some(qi),
                                    label,
                                )
                                .clicked()
                            {
                                *selected_identity = Some(qi.clone());
                                *selected_key = None; // Clear key selection when identity changes
                            }
                        }
                    });
            });

            ui.end_row();

            ui.label("Key:");

            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                // Check if selected identity has suitable keys
                let mut show_combo = true;
                if let Some(qi) = selected_identity {
                    let allowed_purposes = transaction_type.allowed_purposes();
                    let allowed_security_levels =
                        compute_allowed_security_levels(transaction_type, document_type);

                    // Check for keys with private keys loaded
                    let has_suitable_keys_with_private = qi
                        .private_keys
                        .identity_public_keys()
                        .iter()
                        .any(|key_ref| {
                            let key = &key_ref.1.identity_public_key;

                            allowed_purposes.contains(&key.purpose())
                                && allowed_security_levels.contains(&key.security_level())
                        });

                    // Check if there are eligible public keys without private keys
                    let has_eligible_public_keys_without_private = qi
                        .identity
                        .public_keys()
                        .iter()
                        .any(|(_, pub_key)| {
                            // Check if this public key meets the criteria
                            let basic_ok = allowed_purposes.contains(&pub_key.purpose())
                                && allowed_security_levels.contains(&pub_key.security_level());

                            // Check if we don't have the private key for this public key
                            let has_private = qi.private_keys
                                .identity_public_keys()
                                .iter()
                                .any(|key_ref| key_ref.1.identity_public_key.id() == pub_key.id());

                            basic_ok && !has_private
                        });

                    if !is_dev_mode && !has_suitable_keys_with_private {
                        show_combo = false;
                        // Show message and buttons in a proper group/frame
                        ui.group(|ui| {
                            ui.set_min_width(220.0); // Match the combo box width
                            ui.vertical(|ui| {
                                // Identity has eligible keys but private keys not loaded
                                ui.label("⚠ No eligible key. This transaction type requires:");
                                ui.label(format!("• {} key", transaction_type.label()));

                                if has_eligible_public_keys_without_private {
                                    ui.label(
                                        "This Identity already has an eligible public key but the private key isn't loaded into Dash Evo Tool yet.",
                                    );
                                    ui.label("Go to the Identities screen to load an existing private key, or use the button below to add a new key:");
                                }

                                ui.add_space(5.0);

                                // Always show option to add new key
                                if ui.button("Add New Key to Identity").clicked() {
                                    action = AppAction::AddScreen(Screen::AddKeyScreen(
                                        AddKeyScreen::new(
                                            qi.clone(),
                                            app_context,
                                        ),
                                    ));
                                }
                            });
                        });
                    }
                }

                if show_combo {
                    ComboBox::from_id_salt("key_combo")
                        .width(220.0)
                        .selected_text(
                            selected_key
                                .as_ref()
                                .map(format_key_label)
                                .unwrap_or_else(|| "Select Key…".into()),
                        )
                        .show_ui(ui, |kui| {
                            if let Some(qi) = selected_identity {
                                let allowed_purposes = transaction_type.allowed_purposes();
                                let allowed_security_levels =
                                    compute_allowed_security_levels(transaction_type, document_type);

                                for key_ref in qi.private_keys.identity_public_keys() {
                                    let key = &key_ref.1.identity_public_key;

                                    let is_allowed = is_dev_mode
                                        || (allowed_purposes.contains(&key.purpose())
                                            && allowed_security_levels.contains(&key.security_level()));

                                    if is_allowed {
                                        let is_dev_override = is_dev_mode
                                            && (!allowed_purposes.contains(&key.purpose())
                                                || !allowed_security_levels.contains(&key.security_level()));
                                        let label = if is_dev_override {
                                            format_key_label_dev(key)
                                        } else {
                                            format_key_label(key)
                                        };

                                        if kui
                                            .selectable_label(selected_key.as_ref() == Some(key), label)
                                            .clicked()
                                        {
                                            *selected_key = Some(key.clone());
                                        }
                                    }
                                }
                            } else {
                                kui.label("Pick an identity first");
                            }
                        });
                }
            });
            ui.end_row();
        });

    action
}

pub fn add_contract_doc_type_chooser_with_filtering(
    ui: &mut Ui,
    search_term: &mut String,
    app_context: &Arc<AppContext>,
    selected_contract: &mut Option<QualifiedContract>,
    selected_doc_type: &mut Option<DocumentType>,
) {
    let contracts = app_context.get_contracts(None, None).unwrap_or_default();
    let search_term_lowercase = search_term.to_lowercase();
    let filtered = contracts.iter().filter(|qc| {
        contract_display_label(qc)
            .to_lowercase()
            .contains(&search_term_lowercase)
    });

    add_contract_doc_type_chooser_pre_filtered(
        ui,
        search_term,
        filtered,
        selected_contract,
        selected_doc_type,
    );
}

/// Extremely compact chooser: just two combo-boxes (Contract ▸ Doc-Type)
///
/// * No collapsible tree.
/// * Optional search box on top.
/// * Emits `ContractTask::RemoveContract` via a small “🗑” button next to the contract picker.
pub fn add_contract_doc_type_chooser_pre_filtered<'a, T>(
    ui: &mut Ui,
    search_term: &mut String,
    filtered_contracts: T,
    selected_contract: &mut Option<QualifiedContract>,
    selected_doc_type: &mut Option<DocumentType>,
) where
    T: Iterator<Item = &'a QualifiedContract>,
{
    egui::Grid::new("contract_doc_type_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .striped(false)
        .show(ui, |ui| {
            ui.label("Filter contracts:");
            ui.text_edit_singleline(search_term);
            ui.end_row();

            ui.label("Contract:");
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ComboBox::from_id_salt("contract_combo")
                    .width(220.0)
                    .selected_text(
                        selected_contract
                            .as_ref()
                            .map(contract_display_label)
                            .unwrap_or_else(|| "Select Contract…".into()),
                    )
                    .show_ui(ui, |cui| {
                        for qc in filtered_contracts {
                            let label = contract_display_label(qc);
                            if cui
                                .selectable_label(selected_contract.as_ref() == Some(qc), label)
                                .clicked()
                            {
                                *selected_contract = Some(qc.clone());
                            }
                        }
                    });
            });

            ui.end_row();

            ui.label("Doc Type:");
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ComboBox::from_id_salt("doctype_combo")
                    .width(220.0)
                    .selected_text(
                        selected_doc_type
                            .as_ref()
                            .map(|d| d.name().to_owned())
                            .unwrap_or_else(|| "Select Doc Type…".into()),
                    )
                    .show_ui(ui, |dui| {
                        if let Some(qc) = selected_contract {
                            for name in qc.contract.document_types().keys() {
                                if dui
                                    .selectable_label(
                                        selected_doc_type
                                            .as_ref()
                                            .map(|cur| cur.name() == name)
                                            .unwrap_or(false),
                                        name,
                                    )
                                    .clicked()
                                {
                                    *selected_doc_type =
                                        qc.contract.document_type_cloned_for_name(name).ok();
                                }
                            }
                        } else {
                            dui.label("Pick a contract first");
                        }
                    });
            });
            ui.end_row();
        });
}

/// Extremely compact chooser: just two combo-boxes (Contract ▸ Doc-Type)
///
/// * No collapsible tree.
/// * Optional search box on top.
pub fn add_contract_chooser_pre_filtered<'a, T>(
    ui: &mut Ui,
    search_term: &mut String,
    filtered_contracts: T,
    selected_contract: &mut Option<QualifiedContract>,
) where
    T: Iterator<Item = &'a QualifiedContract>,
{
    egui::Grid::new("contract_doc_type_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .striped(false)
        .show(ui, |ui| {
            ui.label("Filter contracts:");
            ui.text_edit_singleline(search_term);
            ui.end_row();

            ui.label("Contract:");
            ComboBox::from_id_salt("contract_chooser")
                .width(220.0)
                .selected_text(
                    selected_contract
                        .as_ref()
                        .map(contract_display_label)
                        .unwrap_or_else(|| "Select Contract…".into()),
                )
                .show_ui(ui, |cui| {
                    for qc in filtered_contracts {
                        let label = contract_display_label(qc);
                        if cui
                            .selectable_label(selected_contract.as_ref() == Some(qc), label)
                            .clicked()
                        {
                            *selected_contract = Some(qc.clone());
                        }
                    }
                });

            ui.end_row();
        });
}

pub fn render_group_action_text(
    ui: &mut Ui,
    group: &Option<(u16, Group)>,
    identity_token_info: &IdentityTokenInfo,
    group_action_type_str: &str,
    group_action_id: &Option<Identifier>,
) -> String {
    if let Some(group_action_id) = group_action_id {
        ui.add_space(20.0);
        ui.add(egui::Label::new(
            egui::RichText::new("This is a group action.")
                .heading()
                .color(egui::Color32::DARK_RED),
        ));

        ui.add_space(10.0);
        ui.label(format!(
            "You are signing an active {} group action (Action ID {})",
            group_action_type_str,
            group_action_id.to_string(Encoding::Base58)
        ));
        format!("Sign {}", group_action_type_str)
    } else if let Some((_, group)) = group.as_ref() {
        let your_power = group
            .members()
            .get(&identity_token_info.identity.identity.id());

        ui.add_space(20.0);
        ui.add(egui::Label::new(
            egui::RichText::new("This is a group action.")
                .heading()
                .color(egui::Color32::DARK_RED),
        ));

        if your_power.is_none() {
            ui.add_space(10.0);
            ui.colored_label(
                Color32::DARK_RED,
                format!(
                    "You are not a valid group member for {} on this token",
                    group_action_type_str
                ),
            );
            return format!("Test {} (Should fail)", group_action_type_str);
        }

        ui.add_space(10.0);
        if let Some(your_power) = your_power {
            if *your_power >= group.required_power() {
                ui.label("You are a unilateral group member.\nYou do not need other group members to sign off on this action for it to process.".to_string());
                group_action_type_str.to_string()
            } else {
                ui.label(format!("You are not a unilateral group member.\nYou can initiate the {group_action_type_str} action but will need other group members to sign off on it for it to process.\nThis action requires a total power of {}.\nYour power is {your_power}.", group.required_power()));

                ui.add_space(10.0);
                ui.label(format!(
                    "Other group members are : \n{}",
                    group
                        .members()
                        .iter()
                        .filter_map(|(member, power)| {
                            if member != &identity_token_info.identity.identity.id() {
                                Some(format!(" - {} with power {}", member, power))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", \n")
                ));
                format!("Initiate Group {}", group_action_type_str)
            }
        } else {
            format!("Test {} (It should fail)", group_action_type_str)
        }
    } else {
        group_action_type_str.to_string()
    }
}

pub fn show_success_screen(
    ui: &mut Ui,
    success_message: String,
    action_buttons: Vec<(String, AppAction)>,
) -> AppAction {
    show_success_screen_with_info(ui, success_message, action_buttons, None)
}

/// Shows a success screen with an optional info section above the buttons.
/// The info section takes a title and description that will be displayed in a centered box.
pub fn show_success_screen_with_info(
    ui: &mut Ui,
    success_message: String,
    action_buttons: Vec<(String, AppAction)>,
    info_section: Option<(&str, &str)>,
) -> AppAction {
    let mut action = AppAction::None;
    let dark_mode = ui.ctx().style().visuals.dark_mode;

    ui.vertical_centered(|ui| {
        ui.add_space(if info_section.is_some() { 60.0 } else { 100.0 });
        ui.heading("🎉");
        ui.heading(success_message);

        // Optional info section (above buttons)
        if let Some((title, description)) = info_section {
            ui.add_space(24.0);

            let description_width = 500.0_f32.min(ui.available_width() - 40.0);
            ui.allocate_ui_with_layout(
                egui::Vec2::new(description_width, 0.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .size(16.0)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(description)
                            .size(14.0)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                },
            );
        }

        ui.add_space(20.0);
        for button in action_buttons {
            if ui.button(button.0).clicked() {
                action = button.1;
            }
        }

        ui.add_space(if info_section.is_some() { 60.0 } else { 100.0 });
    });
    action
}

/// Shows a success screen for group token actions (mint, burn, pause, resume, freeze, unfreeze, etc.)
/// Handles the three cases:
/// 1. Group action signing (group_action_id is Some) - shows "Back to Group Actions" and "Back to Tokens"
/// 2. Group action initiated (has_group && !is_unilateral) - shows "Back to Tokens" and "Go to Group Actions"
/// 3. Normal action - shows just "Back to Tokens"
pub fn show_group_token_success_screen(
    ui: &mut Ui,
    action_name: &str,
    is_group_action_signing: bool,
    is_unilateral_group_member: bool,
    has_group: bool,
    app_context: &Arc<AppContext>,
) -> AppAction {
    show_group_token_success_screen_with_fee(
        ui,
        action_name,
        is_group_action_signing,
        is_unilateral_group_member,
        has_group,
        app_context,
        None,
    )
}

/// Shows a success screen for group token actions with optional fee info display.
/// Handles the three cases:
/// 1. Group action signing (group_action_id is Some) - shows "Back to Group Actions" and "Back to Tokens"
/// 2. Group action initiated (has_group && !is_unilateral) - shows "Back to Tokens" and "Go to Group Actions"
/// 3. Normal action - shows just "Back to Tokens"
pub fn show_group_token_success_screen_with_fee(
    ui: &mut Ui,
    action_name: &str,
    is_group_action_signing: bool,
    is_unilateral_group_member: bool,
    has_group: bool,
    app_context: &Arc<AppContext>,
    fee_info: Option<(&str, &str)>,
) -> AppAction {
    let mut action = AppAction::None;
    let dark_mode = ui.ctx().style().visuals.dark_mode;

    ui.vertical_centered(|ui| {
        ui.add_space(if fee_info.is_some() { 60.0 } else { 100.0 });
        ui.heading("🎉");

        // Determine the success message based on the action type
        if is_group_action_signing {
            ui.heading(format!("Group {} Signing Successful.", action_name));
        } else if !is_unilateral_group_member && has_group {
            ui.heading(format!("Group {} Initiated.", action_name));
        } else {
            ui.heading(format!("{} Successful.", action_name));
        }

        // Optional fee info section
        if let Some((title, description)) = fee_info {
            ui.add_space(24.0);

            let description_width = 500.0_f32.min(ui.available_width() - 40.0);
            ui.allocate_ui_with_layout(
                egui::Vec2::new(description_width, 0.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .size(16.0)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(description)
                            .size(14.0)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                },
            );
        }

        ui.add_space(20.0);

        // Show appropriate buttons based on the action type
        if is_group_action_signing {
            if ui.button("Back to Group Actions").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            if ui.button("Back to Tokens").clicked() {
                action = AppAction::SetMainScreenThenGoToMainScreen(
                    RootScreenType::RootScreenMyTokenBalances,
                );
            }
        } else {
            if ui.button("Back to Tokens").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }

            if !is_unilateral_group_member
                && has_group
                && ui.button("Go to Group Actions").clicked()
            {
                action = AppAction::PopThenAddScreenToMainScreen(
                    RootScreenType::RootScreenDocumentQuery,
                    Screen::GroupActionsScreen(GroupActionsScreen::new(app_context)),
                );
            }
        }
        ui.add_space(if fee_info.is_some() { 60.0 } else { 100.0 });
    });
    action
}
