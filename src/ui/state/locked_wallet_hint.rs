//! The Identities hub's hint that locked wallets are not searched for identities.
//!
//! Automatic identity discovery skips password-protected wallets until they are
//! unlocked, so their identities stay hidden until then.

use crate::model::wallet::WalletSeedHash;

/// The label of the hint's link that opens the wallet unlock prompt.
pub const UNLOCK_LINK_LABEL: &str = "Unlock";

const UNNAMED_WALLET_TEXT: &str =
    "A wallet is locked. Unlock it to look for identities created with it.";
const SEVERAL_WALLETS_TEXT: &str =
    "Some wallets are locked. Unlock them to look for identities created with them.";

/// What the hub tells the user about locked wallets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedWalletHint {
    /// The sentence shown to the user.
    pub text: String,
    /// The wallet the unlock link opens.
    pub unlock_target: WalletSeedHash,
}

/// Decide the hint from the locked wallets, each with its name if it has one.
///
/// With `wallet_filter` set, the hub lists only that wallet's identities, so
/// only that wallet counts. Returns `None` when no counted wallet is locked.
pub fn locked_wallet_hint(
    locked: &[(WalletSeedHash, Option<String>)],
    wallet_filter: Option<&WalletSeedHash>,
) -> Option<LockedWalletHint> {
    let mut relevant = locked
        .iter()
        .filter(|(seed_hash, _)| wallet_filter.is_none_or(|filter| filter == seed_hash));
    let (unlock_target, alias) = relevant.next()?;
    let text = if relevant.next().is_some() {
        SEVERAL_WALLETS_TEXT.to_string()
    } else {
        match alias.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(wallet) => format!(
                "The wallet \"{wallet}\" is locked. Unlock it to look for identities created with it."
            ),
            None => UNNAMED_WALLET_TEXT.to_string(),
        }
    };
    Some(LockedWalletHint {
        text,
        unlock_target: *unlock_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WalletSeedHash = [0xA1; 32];
    const B: WalletSeedHash = [0xB2; 32];

    fn named(hash: WalletSeedHash, alias: &str) -> (WalletSeedHash, Option<String>) {
        (hash, Some(alias.to_string()))
    }

    #[test]
    fn no_locked_wallet_shows_no_hint() {
        assert_eq!(locked_wallet_hint(&[], None), None);
        assert_eq!(locked_wallet_hint(&[], Some(&A)), None);
    }

    #[test]
    fn one_locked_wallet_is_named_and_unlockable() {
        let hint = locked_wallet_hint(&[named(A, "Savings")], None).expect("hint");
        assert_eq!(
            hint.text,
            "The wallet \"Savings\" is locked. Unlock it to look for identities created with it."
        );
        assert_eq!(hint.unlock_target, A);
    }

    #[test]
    fn a_locked_wallet_without_a_name_gets_the_generic_sentence() {
        for alias in [None, Some(String::new()), Some("   ".to_string())] {
            let hint = locked_wallet_hint(&[(A, alias)], None).expect("hint");
            assert_eq!(hint.text, UNNAMED_WALLET_TEXT);
            assert_eq!(hint.unlock_target, A);
        }
    }

    #[test]
    fn several_locked_wallets_share_one_hint_that_unlocks_the_first() {
        let hint =
            locked_wallet_hint(&[named(A, "Savings"), named(B, "Spending")], None).expect("hint");
        assert_eq!(hint.text, SEVERAL_WALLETS_TEXT);
        assert_eq!(hint.unlock_target, A);
    }

    #[test]
    fn a_wallet_filter_counts_only_that_wallet() {
        let locked = [named(A, "Savings"), named(B, "Spending")];
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
            locked_wallet_hint(&[named(A, "Savings")], Some(&open_wallet)),
            None
        );
    }
}
