//! Contract-related DTOs.
//!
//! Replaces `DataContract`, `QualifiedContract`, and related SDK types.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::common::IdentifierDto;

/// Serializable version of `QualifiedContract`.
/// Represents a data contract loaded in the application.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DataContractDto {
    /// Platform contract identifier as hex string.
    pub id: IdentifierDto,
    /// Owner identity identifier as hex string.
    pub owner_id: IdentifierDto,
    /// User-assigned alias for this contract.
    pub alias: Option<String>,
    /// Contract version.
    pub version: u32,
    /// Document type names defined in this contract.
    pub document_type_names: Vec<String>,
    /// Number of token configurations in this contract.
    pub token_count: u16,
    /// Full contract JSON schema (for display/editing).
    pub schema_json: serde_json::Value,
}

/// A document type within a data contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTypeDto {
    /// Document type name (e.g., "domain", "profile").
    pub name: String,
    /// Available query indexes.
    pub indexes: Vec<DocumentIndexDto>,
    /// Whether this document type requires a purchase.
    pub requires_identity_encryption_bounded_key: bool,
    /// Whether this document type requires a purchase.
    pub requires_identity_decryption_bounded_key: bool,
}

/// An index on a document type (used for querying).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentIndexDto {
    /// Index name.
    pub name: String,
    /// Index properties (field name + sort order).
    pub properties: Vec<IndexPropertyDto>,
    /// Whether this index is unique.
    pub unique: bool,
}

/// A property within a document index.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexPropertyDto {
    /// Field name.
    pub name: String,
    /// Sort order ("asc" or "desc").
    pub sort_order: String,
}

/// Brief contract info for list views.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContractSummaryDto {
    /// Platform contract identifier as hex string.
    pub id: IdentifierDto,
    /// User-assigned alias.
    pub alias: Option<String>,
    /// Number of document types.
    pub document_type_count: usize,
    /// Number of tokens.
    pub token_count: u16,
}

/// Description info for a contract (used in search results).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContractDescriptionInfoDto {
    /// Platform contract identifier as hex string.
    pub data_contract_id: IdentifierDto,
    /// Description text.
    pub description: String,
}

/// A group action on a contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupActionDto {
    /// The group contract position.
    pub group_position: u16,
    /// Action ID.
    pub action_id: String,
    /// Action type description.
    pub action_type: String,
    /// Current signers count.
    pub signers_count: u32,
    /// Required signatures.
    pub required_signatures: u32,
    /// JSON representation of the action details.
    pub details: serde_json::Value,
}
