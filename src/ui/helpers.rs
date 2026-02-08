use std::sync::Arc;

use crate::{
    app::AppAction,
    context::AppContext,
    model::{qualified_contract::QualifiedContract, qualified_identity::QualifiedIdentity},
    ui::contracts_documents::group_actions_screen::GroupActionsScreen,
    ui::{RootScreenType, Screen, identities::keys::add_key_screen::AddKeyScreen},
};
use arboard::Clipboard;
use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters,
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

use super::theme::DashColors;
use super::tokens::tokens_screen::IdentityTokenInfo;

/// Translates a raw backend error string into a user-friendly summary and technical details.
///
/// Returns `(user_friendly_summary, technical_details)`. The summary is a short, plain-language
/// description of what went wrong. The details contain the original raw error string for
/// debugging purposes.
///
/// # Examples
/// ```ignore
/// let (summary, details) = translate_backend_error("Transport(Status { code: Unavailable })");
/// assert_eq!(summary, "Connection to Platform failed. Check your network connection.");
/// ```
pub fn translate_backend_error(raw: &str) -> (String, String) {
    let raw_lower = raw.to_lowercase();

    // Connection / transport errors
    if raw_lower.contains("transport(status") {
        if raw_lower.contains("unavailable") {
            return (
                "Connection to Platform failed. Check your network connection.".to_string(),
                raw.to_string(),
            );
        }
        if raw_lower.contains("deadlineexceeded") || raw_lower.contains("deadline exceeded") {
            return (
                "Request timed out. The Platform may be busy — try again later.".to_string(),
                raw.to_string(),
            );
        }
        if raw_lower.contains("internal") {
            return (
                "Platform returned an internal error. Try again later.".to_string(),
                raw.to_string(),
            );
        }
        return (
            "Connection error while communicating with Platform.".to_string(),
            raw.to_string(),
        );
    }

    // Generic connection / network errors
    if raw_lower.contains("connection refused")
        || raw_lower.contains("connect error")
        || raw_lower.contains("connection reset")
    {
        return (
            "Could not connect to the server. Check that Dash Core is running and your network connection is active.".to_string(),
            raw.to_string(),
        );
    }

    if raw_lower.contains("timed out") || raw_lower.contains("timeout") {
        return (
            "The operation timed out. The server may be busy — try again later.".to_string(),
            raw.to_string(),
        );
    }

    // Insufficient funds / balance errors
    if raw_lower.contains("insufficient") && raw_lower.contains("balance") {
        return (
            "Insufficient balance for this operation.".to_string(),
            raw.to_string(),
        );
    }
    if raw_lower.contains("insufficient") {
        return (
            "Insufficient funds for this operation.".to_string(),
            raw.to_string(),
        );
    }

    // Identity not found
    if raw_lower.contains("identity") && raw_lower.contains("not found") {
        return (
            "The requested identity was not found on Platform.".to_string(),
            raw.to_string(),
        );
    }
    if raw_lower.contains("identity at revision") {
        return (
            "The identity could not be found at the expected revision.".to_string(),
            raw.to_string(),
        );
    }

    // Document / contract not found
    if raw_lower.contains("document type not found") {
        return (
            "The requested document type was not found in the contract.".to_string(),
            raw.to_string(),
        );
    }
    if raw_lower.contains("contract not found") || raw_lower.contains("contractnotfound") {
        return (
            "The data contract was not found on Platform.".to_string(),
            raw.to_string(),
        );
    }
    if raw_lower.contains("notfound")
        || (raw_lower.contains("not found") && raw_lower.contains("query"))
    {
        return (
            "The requested item was not found on Platform.".to_string(),
            raw.to_string(),
        );
    }

    // Already exists
    if raw_lower.contains("already exists") {
        return (
            "This item already exists on Platform.".to_string(),
            raw.to_string(),
        );
    }

    // Invalid contract / protocol errors from DPP
    if raw_lower.contains("invalid contract") || raw_lower.contains("protocolerror") {
        return (
            "The contract data is invalid or has been modified on Platform.".to_string(),
            raw.to_string(),
        );
    }

    // State transition broadcast errors
    if raw_lower.contains("state transition broadcast error")
        || raw_lower.contains("statetransition")
    {
        if raw_lower.contains("identity balance is not enough") {
            return (
                "Insufficient identity balance to cover this transaction's fees.".to_string(),
                raw.to_string(),
            );
        }
        return (
            "Failed to broadcast the transaction to Platform.".to_string(),
            raw.to_string(),
        );
    }

    // RPC / Core errors
    if raw_lower.contains("wallet file not specified") {
        return (
            "Dash Core has multiple wallets loaded. Please ensure only one wallet is active."
                .to_string(),
            raw.to_string(),
        );
    }
    if raw_lower.contains("rpc") && raw_lower.contains("error") {
        return (
            "Error communicating with Dash Core. Check that Core is running and configured correctly.".to_string(),
            raw.to_string(),
        );
    }

    // Authentication / cookie errors
    if raw_lower.contains("cookie")
        || raw_lower.contains("authentication")
        || raw_lower.contains("unauthorized")
    {
        return (
            "Authentication with Dash Core failed. Check your RPC credentials or cookie file."
                .to_string(),
            raw.to_string(),
        );
    }

    // Database errors
    if raw_lower.contains("database")
        || raw_lower.contains("rusqlite")
        || raw_lower.contains("sqlite")
    {
        return (
            "A database error occurred. The application data may be corrupted.".to_string(),
            raw.to_string(),
        );
    }

    // Fee-related errors
    if raw_lower.contains("min relay fee not met") || raw_lower.contains("fee rate") {
        return (
            "The transaction fee is too low. Try increasing the fee.".to_string(),
            raw.to_string(),
        );
    }

    // Consensus / validation errors
    if raw_lower.contains("consensus") && raw_lower.contains("error") {
        return (
            "The transaction failed Platform validation.".to_string(),
            raw.to_string(),
        );
    }

    // Frozen token errors
    if raw_lower.contains("frozen") {
        return (
            "This token or identity is currently frozen.".to_string(),
            raw.to_string(),
        );
    }

    // Paused token errors
    if raw_lower.contains("paused") {
        return (
            "This token is currently paused.".to_string(),
            raw.to_string(),
        );
    }

    // If the raw error is already user-readable (short, no nested parens/braces),
    // use it directly as the summary.
    if raw.len() < 120 && !raw.contains("(Status") && !raw.contains("Protocol(") {
        return (raw.to_string(), String::new());
    }

    // Default: generic message with raw error as details
    ("Operation failed.".to_string(), raw.to_string())
}

/// Layout of labels and buttons in the UI fails to vertically align properly containers that contain buttons and other items (labels, text fields, etc.).
/// This constant provides a constant padding to be used in such cases to ensure proper alignment.
pub const BUTTON_ADJUSTMENT_PADDING_TOP: f32 = 15.0;

/// Renders a standardized "Wallet is locked" warning overlay with an Unlock button.
/// Returns `true` if the "Unlock Wallet" button was clicked (caller should open wallet unlock popup).
pub fn render_wallet_locked_overlay(ui: &mut Ui, action_description: &str) -> bool {
    ui.add_space(10.0);
    ui.colored_label(
        DashColors::WARNING_ORANGE,
        format!("Wallet is locked. Please unlock {}.", action_description),
    );
    ui.add_space(8.0);
    ui.button("Unlock Wallet").clicked()
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

    response
        .on_hover_text(hover_text)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
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
    let allowed_security_levels: Vec<SecurityLevel> = match (transaction_type, document_type) {
        (TransactionType::DocumentAction, Some(doc_type)) => {
            let required_level = doc_type.security_level_requirement();
            let allowed_levels = SecurityLevel::CRITICAL as u8..=required_level as u8;
            [
                SecurityLevel::CRITICAL,
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
            ]
            .into_iter()
            .filter(|level| allowed_levels.contains(&(*level as u8)))
            .collect()
        }
        _ => transaction_type.allowed_security_levels(),
    };

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
                        .map(|k| {
                            format!(
                                "Key {} | {} | {} | {}",
                                k.id(),
                                k.purpose(),
                                k.security_level(),
                                k.key_type()
                            )
                        })
                        .unwrap_or_else(|| "Select Key...".into()),
                )
                .show_ui(ui, |kui| {
                    for key_ref in identity.private_keys.identity_public_keys() {
                        let key = &key_ref.1.identity_public_key;

                        let is_allowed = if is_dev_mode {
                            true
                        } else {
                            allowed_purposes.contains(&key.purpose())
                                && allowed_security_levels.contains(&key.security_level())
                        };

                        if is_allowed {
                            let label = if is_dev_mode
                                && (!allowed_purposes.contains(&key.purpose())
                                    || !allowed_security_levels.contains(&key.security_level()))
                            {
                                format!(
                                    "Key {} | {} | {} | {} [DEV]",
                                    key.id(),
                                    key.purpose(),
                                    key.security_level(),
                                    key.key_type()
                                )
                            } else {
                                format!(
                                    "Key {} | {} | {} | {}",
                                    key.id(),
                                    key.purpose(),
                                    key.security_level(),
                                    key.key_type()
                                )
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
                    .selected_text(match selected_identity {
                        Some(qi) => qi
                            .alias
                            .clone()
                            .unwrap_or_else(|| qi.identity.id().to_string(Encoding::Base58)),
                        None => "Select Identity…".into(),
                    })
                    .show_ui(ui, |iui| {
                        for qi in identities {
                            let label = qi
                                .alias
                                .clone()
                                .unwrap_or_else(|| qi.identity.id().to_string(Encoding::Base58));
                            if iui
                                .selectable_label(
                                    selected_identity.as_ref() == Some(qi),
                                    label.clone(),
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
                    let allowed_security_levels: Vec<SecurityLevel> = match (transaction_type, document_type) {
                        (TransactionType::DocumentAction, Some(doc_type)) => {
                            let required_level = doc_type.security_level_requirement();
                            let allowed_levels = SecurityLevel::CRITICAL as u8..=required_level as u8;
                            [SecurityLevel::CRITICAL, SecurityLevel::HIGH, SecurityLevel::MEDIUM]
                                .into_iter()
                                .filter(|level| allowed_levels.contains(&(*level as u8)))
                                .collect()
                        }
                        _ => transaction_type.allowed_security_levels(),
                    };

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
                            .map(|k| {
                                format!(
                                    "Key {} | {} | {} | {}",
                                    k.id(),
                                    k.purpose(),
                                    k.security_level(),
                                    k.key_type()
                                )
                            })
                            .unwrap_or_else(|| "Select Key…".into()),
                    )
                    .show_ui(ui, |kui| {
                        if let Some(qi) = selected_identity {
                            let allowed_purposes = transaction_type.allowed_purposes();
                            let allowed_security_levels = if transaction_type
                                == TransactionType::DocumentAction
                            {
                                if let Some(document_type) = document_type {
                                    // For document actions with a specific document type, use its security requirement
                                    let required_level = document_type.security_level_requirement();
                                    let allowed_levels =
                                        SecurityLevel::CRITICAL as u8..=required_level as u8;
                                    let allowed_levels: Vec<SecurityLevel> = [
                                        SecurityLevel::CRITICAL,
                                        SecurityLevel::HIGH,
                                        SecurityLevel::MEDIUM,
                                    ]
                                    .iter()
                                    .cloned()
                                    .filter(|level| allowed_levels.contains(&(*level as u8)))
                                    .collect();
                                    allowed_levels
                                } else {
                                    transaction_type.allowed_security_levels()
                                }
                            } else {
                                transaction_type.allowed_security_levels()
                            };

                            for key_ref in qi.private_keys.identity_public_keys() {
                                let key = &key_ref.1.identity_public_key;

                                // In dev mode, show all keys
                                // In production mode, filter by transaction requirements
                                let is_allowed = if is_dev_mode {
                                    true
                                } else {
                                    allowed_purposes
                                        .contains(&key.purpose())
                                        && allowed_security_levels.contains(&key.security_level())
                                };

                                if is_allowed {
                                    let label = if is_dev_mode
                                        && (!allowed_purposes.contains(&key.purpose())
                                            || !allowed_security_levels
                                                .contains(&key.security_level()))
                                    {
                                        // In dev mode, mark keys that wouldn't normally be allowed
                                        format!(
                                            "Key {} | {} | {} | {} [DEV]",
                                            key.id(),
                                            key.purpose(),
                                            key.security_level(),
                                            key.key_type()
                                        )
                                    } else {
                                        format!(
                                            "Key {} | {} | {} | {}",
                                            key.id(),
                                            key.purpose(),
                                            key.security_level(),
                                            key.key_type()
                                        )
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
        let key = qc
            .alias
            .clone()
            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
        key.to_lowercase().contains(&search_term_lowercase)
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
                    .selected_text(match selected_contract {
                        Some(qc) => qc
                            .alias
                            .clone()
                            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58)),
                        None => "Select Contract…".into(),
                    })
                    .show_ui(ui, |cui| {
                        for qc in filtered_contracts {
                            let label = qc
                                .alias
                                .clone()
                                .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
                            if cui
                                .selectable_label(
                                    selected_contract.as_ref() == Some(qc),
                                    label.clone(),
                                )
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
                .selected_text(match selected_contract {
                    Some(qc) => qc
                        .alias
                        .clone()
                        .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58)),
                    None => "Select Contract…".into(),
                })
                .show_ui(ui, |cui| {
                    for qc in filtered_contracts {
                        let label = qc
                            .alias
                            .clone()
                            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
                        if cui
                            .selectable_label(selected_contract.as_ref() == Some(qc), label.clone())
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
                            .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(description)
                            .size(14.0)
                            .color(crate::ui::theme::DashColors::text_secondary(dark_mode)),
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
                            .color(crate::ui::theme::DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(description)
                            .size(14.0)
                            .color(crate::ui::theme::DashColors::text_secondary(dark_mode)),
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
