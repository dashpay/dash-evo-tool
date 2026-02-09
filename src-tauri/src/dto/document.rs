//! Document-related DTOs.
//!
//! Replaces `Document` and related SDK types.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::common::IdentifierDto;

/// Serializable representation of a platform Document.
/// The original SDK `Document` is an opaque type; we convert to JSON for display.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDto {
    /// Document identifier as hex string.
    pub id: IdentifierDto,
    /// Owner identity identifier as hex string.
    pub owner_id: IdentifierDto,
    /// Document type name.
    pub document_type: String,
    /// The document's data as a JSON value (the actual properties).
    pub data: serde_json::Value,
    /// Revision number.
    pub revision: u64,
    /// Created-at timestamp (if present).
    pub created_at: Option<u64>,
    /// Updated-at timestamp (if present).
    pub updated_at: Option<u64>,
    /// Transferred-at timestamp (if present).
    pub transferred_at: Option<u64>,
}

/// A page of document query results.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPageDto {
    /// Documents in this page (ID → optional document).
    pub documents: Vec<DocumentPageEntryDto>,
    /// Whether there are more pages available.
    pub has_more: bool,
}

/// An entry in a document page (some may be missing/deleted).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPageEntryDto {
    /// Document identifier as hex string.
    pub id: IdentifierDto,
    /// The document (None if deleted or not found).
    pub document: Option<DocumentDto>,
}
