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
    /// Length, in bytes, of an output script of this kind.
    pub const fn script_len(self) -> usize {
        match self {
            Self::PubkeyHash => 25,
            Self::ScriptHash => 23,
        }
    }

    /// Smallest output, in duffs, the network relays to a script of this
    /// kind: upstream's dust rule, which a Core payment amount and its Max
    /// are both held to.
    pub fn dust_threshold_duffs(self) -> u64 {
        use dash_sdk::dpp::dashcore::hashes::Hash;
        use dash_sdk::dpp::dashcore::{PubkeyHash, ScriptBuf, ScriptHash};
        // Only the script's length sets the threshold, not its hash.
        let script = match self {
            Self::PubkeyHash => ScriptBuf::new_p2pkh(&PubkeyHash::all_zeros()),
            Self::ScriptHash => ScriptBuf::new_p2sh(&ScriptHash::all_zeros()),
        };
        script.dust_value().to_sat()
    }

    /// The script kind `address` pays to.
    pub fn of(address: &dash_sdk::dpp::dashcore::Address) -> Self {
        if address.script_pubkey().is_p2sh() {
            Self::ScriptHash
        } else {
            Self::PubkeyHash
        }
    }
}

/// What a Core send's Max was proven against: the fee the pooled build of
/// that Max charged, and the coins the pooled build could spend at the time
/// (spendable and not reserved by another build). The fee shown for a Core
/// send is priced from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSendQuoteFee {
    /// Fee, in duffs, the pooled build of the quoted Max charged.
    pub fee_at_max_duffs: u64,
    /// How many coins the pooled build could spend.
    pub spendable_input_count: usize,
    /// What those coins hold, in duffs; `None` when there were more of them
    /// than one transaction can spend, so upstream did not total them.
    pub spendable_total_duffs: Option<u64>,
}

/// A spending limit the backend quoted: the Max, and for a Core send the
/// basis of the fee shown beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendLimitQuote {
    /// The Max, in credits.
    pub max_credits: u64,
    /// For a Core send, what the Max was proven against.
    pub core_fee: Option<CoreSendQuoteFee>,
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

impl SpendLimitKind {
    /// Smallest amount, in credits, the operation behind this Max accepts,
    /// so the amount field never refuses a Max the backend quotes.
    pub fn min_amount_credits(self) -> dash_sdk::dpp::balances::credits::Credits {
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        match self {
            // The payment output must clear its script's dust threshold.
            Self::CoreSend { destination } => destination.dust_threshold_duffs() * CREDITS_PER_DUFF,
            // The shield transition and planner only require a positive
            // amount; the Platform-address output minimum does not apply.
            Self::ShieldFromPlatform => 1,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay dust thresholds of the pinned dashcore: a P2SH output's
    /// shorter script lowers it.
    #[test]
    fn dust_threshold_follows_the_output_script() {
        assert_eq!(
            CoreDestinationScript::PubkeyHash.dust_threshold_duffs(),
            546
        );
        assert_eq!(
            CoreDestinationScript::ScriptHash.dust_threshold_duffs(),
            540
        );
    }

    /// Each Max's operation has its own minimum: a Core payment's dust
    /// threshold, and any positive amount for a shield from Platform.
    #[test]
    fn min_amount_follows_the_operation_behind_the_max() {
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        let core = |destination| SpendLimitKind::CoreSend { destination }.min_amount_credits();
        assert_eq!(
            core(CoreDestinationScript::PubkeyHash),
            546 * CREDITS_PER_DUFF
        );
        assert_eq!(
            core(CoreDestinationScript::ScriptHash),
            540 * CREDITS_PER_DUFF
        );
        assert_eq!(SpendLimitKind::ShieldFromPlatform.min_amount_credits(), 1);
    }
}
