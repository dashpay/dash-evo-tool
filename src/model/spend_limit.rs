//! Kinds of backend-quoted spending limits — the amount behind a "Max" button
//! that the upstream builder for that operation is known to accept.

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
