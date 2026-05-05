use dash_sdk::platform::DataContract;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedContract {
    pub contract: DataContract,
    pub alias: Option<String>,
}

/// Arc-backed counterpart to [`QualifiedContract`] for read-only listing
/// paths.
///
/// Use this whenever the caller only needs to iterate / read contracts
/// (e.g. per-frame UI panels). For cached system contracts the underlying
/// `DataContract` is shared via `Arc::clone`, avoiding the deep clone that
/// `QualifiedContract` would incur.
#[derive(Debug, Clone)]
pub struct QualifiedContractRef {
    pub contract: Arc<DataContract>,
    pub alias: Option<String>,
}

impl QualifiedContractRef {
    /// Materializes a fully-owned [`QualifiedContract`]. Call this only at
    /// the moment a caller needs to store an owned copy (e.g. when the user
    /// selects a contract); read-only paths should keep using the ref form.
    pub fn to_owned_qualified(&self) -> QualifiedContract {
        QualifiedContract {
            contract: (*self.contract).clone(),
            alias: self.alias.clone(),
        }
    }
}
