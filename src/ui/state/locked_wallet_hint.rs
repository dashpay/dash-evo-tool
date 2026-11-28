//! The Identities hub's hint that locked wallets are not searched for identities.
//!
//! Automatic identity discovery skips password-protected wallets until they are
//! unlocked: identities already loaded stay listed, but new ones created with
//! a locked wallet are not looked for until then. The hint covers only locked
//! wallets with no listed identity, where nothing shows the wallet is in use.

use crate::model::wallet::WalletSeedHash;

/// The label of the hint's link that opens the wallet unlock prompt.
pub const UNLOCK_LINK_LABEL: &str = "Unlock";

const UNNAMED_WALLET_TEXT: &str =
    "A wallet is locked. Unlock it to look for identities created with it.";
const SEVERAL_WALLETS_TEXT: &str =
    "Some wallets are locked. Unlock them to look for identities created with them.";

/// A locked, password-protected wallet as the hub sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedWallet {
    pub seed_hash: WalletSeedHash,
    /// The wallet's name, if it has one.
    pub alias: Option<String>,
    /// Whether the hub already lists an identity created with this wallet.
    pub has_listed_identity: bool,
}

/// What the hub tells the user about locked wallets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedWalletHint {
    /// The sentence shown to the user.
    pub text: String,
    /// The wallet the unlock link opens.
    pub unlock_target: WalletSeedHash,
}

/// Decide the hint from the locked wallets.
///
/// Only locked wallets without a listed identity count. With `wallet_filter`
/// set, the hub lists only that wallet's identities, so only that wallet
/// counts. Returns `None` when no wallet counts.
pub fn locked_wallet_hint(
    locked: &[LockedWallet],
    wallet_filter: Option<&WalletSeedHash>,
) -> Option<LockedWalletHint> {
    let mut relevant = locked.iter().filter(|wallet| {
        !wallet.has_listed_identity
            && wallet_filter.is_none_or(|filter| *filter == wallet.seed_hash)
    });
    let first = relevant.next()?;
    let text = if relevant.next().is_some() {
        SEVERAL_WALLETS_TEXT.to_string()
    } else {
        match first
            .alias
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            Some(wallet) => format!(
                "The wallet \"{wallet}\" is locked. Unlock it to look for identities created with it."
            ),
            None => UNNAMED_WALLET_TEXT.to_string(),
        }
    };
    Some(LockedWalletHint {
        text,
        unlock_target: first.seed_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WalletSeedHash = [0xA1; 32];
    const B: WalletSeedHash = [0xB2; 32];

    fn without_identity(seed_hash: WalletSeedHash, alias: &str) -> LockedWallet {
        LockedWallet {
            seed_hash,
            alias: Some(alias.to_string()),
            has_listed_identity: false,
        }
    }

    fn with_identity(seed_hash: WalletSeedHash, alias: &str) -> LockedWallet {
        LockedWallet {
            has_listed_identity: true,
            ..without_identity(seed_hash, alias)
        }
    }

    #[test]
    fn no_locked_wallet_shows_no_hint() {
        assert_eq!(locked_wallet_hint(&[], None), None);
        assert_eq!(locked_wallet_hint(&[], Some(&A)), None);
    }

    #[test]
    fn one_locked_wallet_is_named_and_unlockable() {
        let hint = locked_wallet_hint(&[without_identity(A, "Savings")], None).expect("hint");
        assert_eq!(
            hint.text,
            "The wallet \"Savings\" is locked. Unlock it to look for identities created with it."
        );
        assert_eq!(hint.unlock_target, A);
    }

    #[test]
    fn a_locked_wallet_without_a_name_gets_the_generic_sentence() {
        for alias in [None, Some(String::new()), Some("   ".to_string())] {
            let wallet = LockedWallet {
                seed_hash: A,
                alias,
                has_listed_identity: false,
            };
            let hint = locked_wallet_hint(&[wallet], None).expect("hint");
            assert_eq!(hint.text, UNNAMED_WALLET_TEXT);
            assert_eq!(hint.unlock_target, A);
        }
    }

    #[test]
    fn several_locked_wallets_share_one_hint_that_unlocks_the_first() {
        let locked = [
            without_identity(A, "Savings"),
            without_identity(B, "Spending"),
        ];
        let hint = locked_wallet_hint(&locked, None).expect("hint");
        assert_eq!(hint.text, SEVERAL_WALLETS_TEXT);
        assert_eq!(hint.unlock_target, A);
    }

    #[test]
    fn a_wallet_filter_counts_only_that_wallet() {
        let locked = [
            without_identity(A, "Savings"),
            without_identity(B, "Spending"),
        ];
        let hint = locked_wallet_hint(&locked, Some(&B)).expect("hint");
        assert_eq!(
            hint.text,
            "The wallet \"Spending\" is locked. Unlock it to look for identities created with it."
        );
        assert_eq!(hint.unlock_target, B);
    }

    #[test]
    fn filtering_to_an_open_wallet_hides_the_hint() {
        let open_wallet: WalletSeedHash = [0xC3; 32];
        assert_eq!(
            locked_wallet_hint(&[without_identity(A, "Savings")], Some(&open_wallet)),
            None
        );
    }

    #[test]
    fn a_locked_wallet_with_a_listed_identity_shows_no_hint() {
        let locked = [with_identity(A, "Savings")];
        assert_eq!(locked_wallet_hint(&locked, None), None);
        assert_eq!(locked_wallet_hint(&locked, Some(&A)), None);
    }

    #[test]
    fn only_locked_wallets_without_a_listed_identity_count() {
        let locked = [with_identity(A, "Savings"), without_identity(B, "Spending")];
        let hint = locked_wallet_hint(&locked, None).expect("hint");
        assert_eq!(
            hint.text,
            "The wallet \"Spending\" is locked. Unlock it to look for identities created with it.",
            "a wallet whose identities are listed must not turn the hint plural"
        );
        assert_eq!(hint.unlock_target, B);
    }
}
