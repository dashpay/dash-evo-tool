//! Which loaded wallet (if any) controls an on-screen object, e.g. a masternode.
//!
//! Pure decision logic (module-placement policy → `model/`): the UI shows a real
//! wallet name only when a wallet currently loaded in this app genuinely owns the
//! object; otherwise it shows the read-only [`NOT_IN_WALLET_LABEL`] indicator so a
//! wallet-less object never appears to belong to some arbitrary wallet.

use crate::model::wallet::WalletSeedHash;

/// Read-only indicator shown when no loaded wallet controls the object. A single
/// i18n translation unit; kept short and jargon-free.
pub const NOT_IN_WALLET_LABEL: &str = "Not in a wallet";

/// Which loaded wallet, if any, controls an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalletAssociation {
    /// A wallet loaded in this app controls the object; carries its display name.
    InWallet(String),
    /// No wallet loaded in this app controls the object.
    NotInWallet,
}

impl WalletAssociation {
    /// Resolve an object's wallet association from the wallet its keys derive
    /// from and the wallets currently loaded.
    ///
    /// The object is [`InWallet`] only when it has an owning wallet hash AND that
    /// wallet is currently loaded. A `None` owning hash (no wallet-derived key —
    /// e.g. a masternode loaded from raw keys) or an owning hash no loaded wallet
    /// matches both resolve to [`NotInWallet`], so the UI never names a wallet
    /// that does not genuinely own the object.
    ///
    /// [`InWallet`]: WalletAssociation::InWallet
    /// [`NotInWallet`]: WalletAssociation::NotInWallet
    pub fn resolve(
        owning: Option<WalletSeedHash>,
        loaded_wallets: &[(WalletSeedHash, String)],
    ) -> Self {
        match owning {
            Some(hash) => loaded_wallets
                .iter()
                .find(|(h, _)| *h == hash)
                .map(|(_, name)| Self::InWallet(name.clone()))
                .unwrap_or(Self::NotInWallet),
            None => Self::NotInWallet,
        }
    }

    /// Whether a loaded wallet controls the object.
    pub fn is_in_wallet(&self) -> bool {
        matches!(self, Self::InWallet(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallets() -> Vec<(WalletSeedHash, String)> {
        vec![([0x11; 32], "Main Wallet".to_string())]
    }

    /// An object with no wallet-derived key (owning hash `None`) — the masternode
    /// case — is never shown as belonging to a wallet.
    #[test]
    fn no_owning_hash_resolves_to_not_in_wallet() {
        assert_eq!(
            WalletAssociation::resolve(None, &wallets()),
            WalletAssociation::NotInWallet
        );
    }

    /// The reported bug: an owning wallet that is NOT among the loaded wallets
    /// must not fall back to naming some other loaded wallet.
    #[test]
    fn unmatched_owning_hash_resolves_to_not_in_wallet() {
        assert_eq!(
            WalletAssociation::resolve(Some([0x99; 32]), &wallets()),
            WalletAssociation::NotInWallet
        );
    }

    /// A genuinely owned object names its wallet.
    #[test]
    fn matched_owning_hash_resolves_to_the_wallet_name() {
        assert_eq!(
            WalletAssociation::resolve(Some([0x11; 32]), &wallets()),
            WalletAssociation::InWallet("Main Wallet".to_string())
        );
        assert!(
            WalletAssociation::resolve(Some([0x11; 32]), &wallets()).is_in_wallet(),
            "a matched object reports as in a wallet"
        );
        assert!(!WalletAssociation::NotInWallet.is_in_wallet());
    }
}
