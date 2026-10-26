//! Kinds of backend-quoted spending limits — the amount behind a "Max" button
//! that the upstream builder for that operation is known to accept.

/// Why a wallet's available amount is not offered while another of its
/// transactions is still being built or confirmed.
pub const SPEND_IN_PROGRESS_MESSAGE: &str = "Another transaction from this wallet is still in progress. The available amount is shown again when it finishes.";

/// The output script a Core payment pays to. Its size is part of the
/// transaction the fee is charged on, so a Max is quoted for one kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoreDestinationScript {
    /// Pay-to-public-key-hash. The larger of the two output scripts Dash
    /// addresses encode, so it is also the conservative bound quoted before
    /// a destination is chosen.
    #[default]
    PubkeyHash,
    /// Pay-to-script-hash.
    ScriptHash,
}

impl CoreDestinationScript {
    /// The script kind `address` pays to.
    pub fn of(address: &dash_sdk::dpp::dashcore::Address) -> Self {
        if address.script_pubkey().is_p2sh() {
            Self::ScriptHash
        } else {
            Self::PubkeyHash
        }
    }
}

/// Which operation a spending limit is quoted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpendLimitKind {
    /// A Core payment to one address of the `destination` kind, funded from
    /// the wallet's pooled accounts with no change.
    CoreSend { destination: CoreDestinationScript },
    /// Shielding credits from the wallet's Platform addresses, under the
    /// upstream shield planner's input rules.
    ShieldFromPlatform,
}

/// Why a Core send has no Max to offer although the wallet answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSendShortfall {
    /// No coin in the accounts a send spends from can be spent yet: the
    /// funds are mixed (CoinJoin), recent rewards still maturing, or none.
    NothingSpendable,
    /// Spendable coins exist, but none brings in more than the network fee
    /// its own spend costs.
    BelowFee,
    /// Every spendable coin is held by a transaction the wallet has not
    /// finished processing yet.
    HeldByPendingTransaction,
}

impl CoreSendShortfall {
    /// What the form tells the user instead of offering Max.
    pub fn message(self) -> &'static str {
        match self {
            Self::NothingSpendable => {
                "None of your funds can be sent from this form yet. CoinJoin funds cannot be sent here, and recent masternode or mining rewards become available a few hours after they arrive."
            }
            Self::BelowFee => {
                "Your spendable balance is too low to cover the network fee. Add more Dash to this wallet, then try again."
            }
            Self::HeldByPendingTransaction => {
                "Your funds are held by a recent transaction that has not finished yet. The available amount is shown again when it completes."
            }
        }
    }
}
