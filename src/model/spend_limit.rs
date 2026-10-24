//! Kinds of backend-quoted spending limits — the amount behind a "Max" button
//! that the upstream builder for that operation is known to accept.

/// Why a wallet's available amount is not offered while another of its
/// transactions is still being built or confirmed.
pub const SPEND_IN_PROGRESS_MESSAGE: &str = "Another transaction from this wallet is still in progress. The available amount is shown again when it finishes.";

/// Which operation a spending limit is quoted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpendLimitKind {
    /// A Core payment to one address, funded from the wallet's pooled
    /// accounts with no change.
    CoreSend,
    /// Shielding credits from the wallet's Platform addresses, under the
    /// upstream shield planner's input rules.
    ShieldFromPlatform,
}
