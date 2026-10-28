//! How the Identity Hub's Tokens tab lists tokens: remembered per device.

use serde::{Deserialize, Serialize};

/// The order of the Tokens tab list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenListSort {
    /// Tokens with something to claim first, then by balance, zero balances last.
    #[default]
    ClaimableFirst,
    /// Highest balance first; unknown balances after known ones, zeros last.
    BalanceHighToLow,
    /// Alphabetical by token name.
    NameAToZ,
}

impl TokenListSort {
    /// Every sort, in the order the sort menu lists them.
    pub const ALL: [TokenListSort; 3] = [
        TokenListSort::ClaimableFirst,
        TokenListSort::BalanceHighToLow,
        TokenListSort::NameAToZ,
    ];

    /// The menu label.
    pub fn label(self) -> &'static str {
        match self {
            TokenListSort::ClaimableFirst => "Claimable first, then balance",
            TokenListSort::BalanceHighToLow => "Balance, high to low",
            TokenListSort::NameAToZ => "Name, A to Z",
        }
    }
}

/// The Tokens tab's list controls, persisted at
/// [`IdentityTokensViewPrefs::KV_KEY`] so they survive a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityTokensViewPrefs {
    /// Hide tokens whose fetched balance is zero. Never hides a token whose
    /// balance is still unknown.
    pub hide_zero_balances: bool,
    /// The list order.
    pub sort: TokenListSort,
    /// The name / symbol / ID filter text.
    pub filter: String,
}

impl IdentityTokensViewPrefs {
    /// Global-scope k/v key of the persisted blob.
    pub const KV_KEY: &'static str = "det:identity_tokens_view:v1";

    /// The longest filter kept: a Base58 token ID is 44 characters, so this
    /// fits any ID with room, and a stray paste never lands in the database.
    pub const MAX_FILTER_CHARS: usize = 64;

    /// Cut the filter to [`Self::MAX_FILTER_CHARS`] characters.
    pub fn clamp_filter(&mut self) {
        if let Some((end, _)) = self.filter.char_indices().nth(Self::MAX_FILTER_CHARS) {
            self.filter.truncate(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_is_cut_at_the_cap_on_a_character_boundary() {
        let mut prefs = IdentityTokensViewPrefs {
            filter: "é".repeat(IdentityTokensViewPrefs::MAX_FILTER_CHARS + 3),
            ..Default::default()
        };
        prefs.clamp_filter();
        assert_eq!(
            prefs.filter.chars().count(),
            IdentityTokensViewPrefs::MAX_FILTER_CHARS
        );
        let mut short = IdentityTokensViewPrefs {
            filter: "dcf".into(),
            ..Default::default()
        };
        short.clamp_filter();
        assert_eq!(short.filter, "dcf");
    }
}
