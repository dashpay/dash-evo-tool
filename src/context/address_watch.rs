//! Typed address-watch registration for `AppContext`.
//!
//! The previous pair `ensure_address_imported` / `try_import_address` silently
//! no-opped in SPV mode and relied on an undocumented precondition — that the
//! address was already covered by the wallet's account-level watch. That was
//! true for standard BIP44 receive addresses derived from the wallet xprv, but
//! not for DashPay DIP-15 contact paths, imported single-key addresses, or
//! P2SH multisig outputs.
//!
//! [`AddressCoverage`] forces callers to declare how SPV coverage is provided
//! for an address so [`AppContext::ensure_address_watched`] can dispatch to
//! the right subsystem and never silently drop coverage.
//!
//! [`AppContext::ensure_address_watched`]: crate::context::AppContext::ensure_address_watched

use crate::model::wallet::DerivationPathReference;

/// How SPV coverage is provided for an address being registered.
///
/// Callers must pick the variant that matches how the address was obtained.
/// The wrong variant can cause incoming transactions to be missed (off-tree
/// address classified as BIP44) or waste a runtime SPV registration slot
/// (BIP44 address classified as off-tree).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressCoverage {
    /// Address derived inside the wallet's standard BIP44 account
    /// (`m/44'/coin'/0'/change/index`).
    ///
    /// SPV coverage is automatic via the account-level watch set up at wallet
    /// load; no explicit SPV registration is required. In Core RPC mode the
    /// address is imported into the wallet as watch-only.
    StandardBip44Account,

    /// Address derived outside the standard BIP44 account.
    ///
    /// Covers DashPay DIP-15 contact paths, imported single-key wallets,
    /// Blockchain-Identities paths (`m/9'/…`), platform-payment addresses
    /// (DIP-17), multisig outputs, and anything else that does not belong to
    /// the wallet's BIP44 receive/change chains.
    ///
    /// In Core RPC mode the address is imported into the wallet as watch-only.
    /// In SPV mode the running subsystem currently has no runtime registration
    /// hook — see the `TODO(refactor)` note in
    /// [`AppContext::ensure_address_watched`] and [`TaskError::SpvOffTreeAddressRegistrationUnsupported`].
    ///
    /// [`AppContext::ensure_address_watched`]: crate::context::AppContext::ensure_address_watched
    /// [`TaskError::SpvOffTreeAddressRegistrationUnsupported`]: crate::backend_task::error::TaskError::SpvOffTreeAddressRegistrationUnsupported
    OffTree,
}

impl AddressCoverage {
    /// Map a [`DerivationPathReference`] to the appropriate coverage variant.
    ///
    /// Used by the wallet's internal address-registration helpers to pick the
    /// correct variant without burdening every caller with the classification.
    pub fn from_derivation_path_reference(reference: DerivationPathReference) -> Self {
        match reference {
            DerivationPathReference::BIP44 => Self::StandardBip44Account,
            _ => Self::OffTree,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bip44_maps_to_standard_account() {
        assert_eq!(
            AddressCoverage::from_derivation_path_reference(DerivationPathReference::BIP44),
            AddressCoverage::StandardBip44Account,
        );
    }

    #[test]
    fn non_bip44_references_map_to_off_tree() {
        let off_tree_refs = [
            DerivationPathReference::BIP32,
            DerivationPathReference::BlockchainIdentities,
            DerivationPathReference::ProviderFunds,
            DerivationPathReference::ContactBasedFunds,
            DerivationPathReference::ContactBasedFundsRoot,
            DerivationPathReference::ContactBasedFundsExternal,
            DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
            DerivationPathReference::BlockchainIdentityCreditTopupFunding,
            DerivationPathReference::BlockchainIdentityCreditInvitationFunding,
            DerivationPathReference::ProviderPlatformNodeKeys,
            DerivationPathReference::CoinJoin,
            DerivationPathReference::PlatformPayment,
            DerivationPathReference::Root,
            DerivationPathReference::Unknown,
        ];
        for reference in off_tree_refs {
            assert_eq!(
                AddressCoverage::from_derivation_path_reference(reference),
                AddressCoverage::OffTree,
                "reference {:?} should map to OffTree",
                reference,
            );
        }
    }
}
