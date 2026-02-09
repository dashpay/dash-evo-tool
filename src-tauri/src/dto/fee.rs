//! Fee-related DTOs.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Serializable version of `backend_task::FeeResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FeeResultDto {
    /// The fee that was estimated before the operation (in credits).
    pub estimated_fee: u64,
    /// The actual fee that was paid (in credits).
    pub actual_fee: u64,
}
