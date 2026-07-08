use std::collections::BTreeMap;

use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;

use crate::model::wallet::{DerivationPathHelpers, DerivationPathReference};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountCategory {
    Bip44,
    Bip32,
    CoinJoin,
    IdentityRegistration,
    IdentitySystem,
    IdentityTopup,
    IdentityInvitation,
    ProviderVoting,
    ProviderOwner,
    ProviderOperator,
    ProviderPlatform,
    /// DIP-17: Platform Payment Addresses (dash/tdash Bech32m HRP per DIP-18)
    PlatformPayment,
    Other(DerivationPathReference),
}

impl AccountCategory {
    pub fn from_reference(reference: DerivationPathReference) -> Self {
        match reference {
            DerivationPathReference::BIP44 => AccountCategory::Bip44,
            DerivationPathReference::BIP32 => AccountCategory::Bip32,
            DerivationPathReference::BlockchainIdentities => AccountCategory::IdentitySystem,
            DerivationPathReference::BlockchainIdentityCreditRegistrationFunding => {
                AccountCategory::IdentityRegistration
            }
            DerivationPathReference::BlockchainIdentityCreditInvitationFunding => {
                AccountCategory::IdentityInvitation
            }
            DerivationPathReference::BlockchainIdentityCreditTopupFunding => {
                AccountCategory::IdentityTopup
            }
            DerivationPathReference::ProviderVotingKeys => AccountCategory::ProviderVoting,
            DerivationPathReference::ProviderOwnerKeys => AccountCategory::ProviderOwner,
            DerivationPathReference::ProviderOperatorKeys => AccountCategory::ProviderOperator,
            DerivationPathReference::ProviderPlatformNodeKeys => AccountCategory::ProviderPlatform,
            DerivationPathReference::ProviderFunds | DerivationPathReference::CoinJoin => {
                AccountCategory::CoinJoin
            }
            DerivationPathReference::PlatformPayment => AccountCategory::PlatformPayment,
            _ => AccountCategory::Other(reference),
        }
    }

    pub fn label(&self, index: Option<u32>) -> String {
        match self {
            AccountCategory::Bip44 => match index {
                Some(0) => "Dash Core".to_string(),
                Some(idx) => format!("BIP44 Account #{}", idx),
                None => "BIP44 Account".to_string(),
            },
            AccountCategory::Bip32 => match index {
                Some(idx) if idx > 0 => format!("Legacy BIP32 Account #{}", idx),
                _ => "Legacy BIP32 Account".to_string(),
            },
            AccountCategory::CoinJoin => "CoinJoin".to_string(),
            AccountCategory::IdentityRegistration => "Identity Registration".to_string(),
            AccountCategory::IdentitySystem => "Identity System".to_string(),
            AccountCategory::IdentityTopup => "Identity Top-up".to_string(),
            AccountCategory::IdentityInvitation => "Identity Invitation".to_string(),
            AccountCategory::ProviderVoting => "Provider Voting".to_string(),
            AccountCategory::ProviderOwner => "Provider Owner".to_string(),
            AccountCategory::ProviderOperator => "Provider Operator".to_string(),
            AccountCategory::ProviderPlatform => "Provider Platform".to_string(),
            AccountCategory::PlatformPayment => "Platform".to_string(),
            AccountCategory::Other(reference) => format!("{:?}", reference),
        }
    }

    fn sort_key(&self) -> u8 {
        match self {
            AccountCategory::Bip44 => 0,
            AccountCategory::PlatformPayment => 1,
            AccountCategory::Bip32 => 2,
            AccountCategory::CoinJoin => 3,
            AccountCategory::IdentityRegistration => 4,
            AccountCategory::IdentitySystem => 5,
            AccountCategory::IdentityTopup => 6,
            AccountCategory::IdentityInvitation => 7,
            AccountCategory::ProviderOwner => 8,
            AccountCategory::ProviderVoting => 9,
            AccountCategory::ProviderOperator => 10,
            AccountCategory::ProviderPlatform => 11,
            AccountCategory::Other(_) => 12,
        }
    }

    pub fn description(&self) -> Option<&'static str> {
        match self {
            AccountCategory::Bip44 => {
                Some("Standard BIP44 account (m/44'/5'/… ) used for normal wallet funds.")
            }
            AccountCategory::Bip32 => Some(
                "Legacy BIP32 account (m/0'/… ). Funds here were received on older derivation paths.",
            ),
            AccountCategory::CoinJoin => {
                Some("CoinJoin mixing account. Funds here are earmarked for privacy transactions.")
            }
            AccountCategory::IdentityRegistration => Some(
                "Credit funding addresses used to register new identities (DIP‑9). Each identity consumes one hardened address here.",
            ),
            AccountCategory::IdentitySystem => Some(
                "Identity authentication/system addresses. They back the identity keys stored on Platform and usually hold zero balance.",
            ),
            AccountCategory::IdentityTopup => Some(
                "Credit funding addresses used when topping up an existing identity's balance.",
            ),
            AccountCategory::IdentityInvitation => Some(
                "Invitation credit funding addresses. Use these when sponsoring a new identity.",
            ),
            AccountCategory::ProviderVoting => Some(
                "Voting key branch for masternodes (Dash Platform / Core DIP‑3 voting key outputs).",
            ),
            AccountCategory::ProviderOwner => {
                Some("Masternode owner key branch (collateral ownership outputs).")
            }
            AccountCategory::ProviderOperator => {
                Some("Operator key branch for masternode BLS operator keys.")
            }
            AccountCategory::ProviderPlatform => {
                Some("Platform service key branch used by masternode platform nodes.")
            }
            AccountCategory::PlatformPayment => Some(
                "DIP-17 Platform payment addresses (dash/tdash HRP per DIP-18). Hold Dash Credits on Platform, independent of identities.",
            ),
            AccountCategory::Other(_) => None,
        }
    }

    /// Returns a short label suitable for tab headers.
    pub fn tab_label(&self, index: Option<u32>) -> &'static str {
        match self {
            AccountCategory::Bip44 => match index {
                Some(0) => "Dash Core",
                _ => "BIP44",
            },
            AccountCategory::Bip32 => "Legacy BIP32",
            AccountCategory::CoinJoin => "CoinJoin",
            AccountCategory::IdentityRegistration => "Identity Registration",
            AccountCategory::IdentitySystem => "Identity System",
            AccountCategory::IdentityTopup => "Identity Top-up",
            AccountCategory::IdentityInvitation => "Identity Invitation",
            AccountCategory::ProviderVoting
            | AccountCategory::ProviderOwner
            | AccountCategory::ProviderOperator
            | AccountCategory::ProviderPlatform => "Provider",
            AccountCategory::PlatformPayment => "Platform",
            AccountCategory::Other(_) => "Other",
        }
    }

    /// Whether this account tab is visible in default (non-developer) mode.
    pub fn is_visible_in_default_mode(&self) -> bool {
        matches!(
            self,
            AccountCategory::Bip44 | AccountCategory::PlatformPayment
        )
    }

    /// Returns true if this is a "system" account category shown only in
    /// developer mode under the consolidated System tab.
    pub fn is_system_category(&self) -> bool {
        !self.is_visible_in_default_mode()
    }
}

pub(crate) fn categorize_account_path(
    path: &DerivationPath,
    network: Network,
    reference: DerivationPathReference,
) -> (AccountCategory, Option<u32>) {
    // Derivation path shape is authoritative over stored metadata.
    // This prevents stale/misclassified references from surfacing wrong account labels.
    let category = if path.is_bip32() {
        AccountCategory::Bip32
    } else if path.is_bip44(network) {
        AccountCategory::Bip44
    } else if path.is_platform_payment(network) {
        AccountCategory::PlatformPayment
    } else {
        AccountCategory::from_reference(reference)
    };

    let index = match category {
        AccountCategory::Bip44 | AccountCategory::Bip32 => path.bip44_account_index(),
        _ => None,
    };

    (category, index)
}

#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub category: AccountCategory,
    pub index: Option<u32>,
    pub confirmed_balance: u64,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct AccountKey {
    category: AccountCategory,
    index: Option<u32>,
}

struct AccountSummaryBuilder {
    key: AccountKey,
    confirmed_balance: u64,
}

impl AccountSummaryBuilder {
    fn new(category: AccountCategory, index: Option<u32>) -> Self {
        Self {
            key: AccountKey { category, index },
            confirmed_balance: 0,
        }
    }

    fn add_address(&mut self, balance: u64) {
        self.confirmed_balance += balance;
    }

    fn build(self) -> AccountSummary {
        AccountSummary {
            category: self.key.category,
            index: self.key.index,
            confirmed_balance: self.confirmed_balance,
        }
    }
}

/// Build per-account Core (chain) balance summaries for the wallet, sourced
/// entirely from the display-only `WalletBackend` snapshot — never the legacy
/// in-memory `Wallet` model. Platform credits are not part of this breakdown:
/// the Platform tab reads the single authoritative total
/// [`AppContext::platform_balance_duffs`](crate::context::AppContext::platform_balance_duffs)
/// directly, so its figure cannot diverge from the wallet header total.
///
/// Two passes feed the per-category chain totals:
///
/// 1. **Generated addresses** (`address_paths`) — every address the upstream
///    wallet has generated, categorized by derivation-path shape. This seeds the
///    per-category structure (so an unfunded account still renders a
///    zero-balance tab) and folds in each address's chain balance from
///    `address_balances`.
/// 2. **Authoritative funded addresses** — every funded address in
///    `address_balances` that `address_paths` does not cover (funds past DET's
///    bookkeeping window), bucketed as `Other(Unknown)`. This closes the desync
///    where such funds were dropped from the per-category totals. Each chain
///    balance is counted exactly once (pass 2 skips addresses pass 1 covered).
pub fn collect_account_summaries(
    network: Network,
    address_balances: &BTreeMap<Address, u64>,
    address_paths: &BTreeMap<Address, DerivationPath>,
) -> Vec<AccountSummary> {
    let mut builders: BTreeMap<AccountKey, AccountSummaryBuilder> = BTreeMap::new();

    // Pass 1: generated addresses seed the per-category structure and chain
    // balances, categorized by derivation-path shape.
    for (address, path) in address_paths {
        let (category, index) =
            categorize_account_path(path, network, DerivationPathReference::Unknown);
        let balance = address_balances.get(address).copied().unwrap_or_default();
        builders
            .entry(AccountKey {
                category: category.clone(),
                index,
            })
            .or_insert_with(|| AccountSummaryBuilder::new(category, index))
            .add_address(balance);
    }

    // Pass 2: funded addresses the generated-path set does not cover, so no
    // funded chain balance is dropped from the per-category totals.
    for (address, &balance) in address_balances {
        if balance == 0 || address_paths.contains_key(address) {
            continue;
        }
        let category = AccountCategory::Other(DerivationPathReference::Unknown);
        builders
            .entry(AccountKey {
                category: category.clone(),
                index: None,
            })
            .or_insert_with(|| AccountSummaryBuilder::new(category, None))
            .add_address(balance);
    }

    let mut summaries: Vec<_> = builders
        .into_values()
        .map(|builder| builder.build())
        .collect();

    summaries.sort_by(|a, b| {
        (a.category.sort_key(), a.index.unwrap_or(0))
            .cmp(&(b.category.sort_key(), b.index.unwrap_or(0)))
    });

    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::key_wallet::bip32::ChildNumber;

    #[test]
    fn bip44_without_account_index_is_not_dash_core() {
        assert_eq!(AccountCategory::Bip44.label(None), "BIP44 Account");
    }

    #[test]
    fn legacy_path_overrides_incorrect_bip44_reference() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 1 },
            ChildNumber::Normal { index: 3 },
        ]);

        let (category, index) =
            categorize_account_path(&path, Network::Testnet, DerivationPathReference::BIP44);
        assert_eq!(category, AccountCategory::Bip32);
        assert_eq!(index, None);
    }

    #[test]
    fn bip44_path_overrides_incorrect_bip32_reference() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 1 },
        ]);

        let (category, index) =
            categorize_account_path(&path, Network::Testnet, DerivationPathReference::BIP32);
        assert_eq!(category, AccountCategory::Bip44);
        assert_eq!(index, Some(0));
    }

    #[test]
    fn bip44_requires_matching_coin_type_for_network() {
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 5 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 1 },
        ]);

        let (category, _) =
            categorize_account_path(&path, Network::Testnet, DerivationPathReference::Unknown);
        assert_ne!(category, AccountCategory::Bip44);
    }

    /// A P2PKH testnet address derived from a raw secret-key byte — a distinct,
    /// valid address to stand in for a funded-but-unindexed chain address.
    fn addr_from_byte(b: u8) -> Address {
        use dash_sdk::dpp::dashcore::PublicKey;
        use dash_sdk::dpp::dashcore::secp256k1::{
            PublicKey as SecpPublicKey, Secp256k1, SecretKey,
        };
        let secp = Secp256k1::new();
        let mut sk_bytes = [0u8; 32];
        sk_bytes[0] = if b == 0 { 1 } else { b };
        let sk = SecretKey::from_slice(&sk_bytes).expect("valid secret key");
        let pubkey = PublicKey::from_slice(&SecpPublicKey::from_secret_key(&secp, &sk).serialize())
            .expect("valid pubkey");
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    #[test]
    fn platform_payment_path_shape_is_categorized_without_reference() {
        use crate::model::wallet::DerivationPathHelpers;
        // Snapshot-sourced paths carry no DET reference — the DIP-17 shape alone
        // must categorize them, or a Core-funded platform address buckets wrong.
        let path = DerivationPath::platform_payment_path(Network::Testnet, 0, 0, 3);
        let (category, index) =
            categorize_account_path(&path, Network::Testnet, DerivationPathReference::Unknown);
        assert_eq!(category, AccountCategory::PlatformPayment);
        assert_eq!(index, None);
    }

    /// A BIP-44 external path `m/44'/1'/0'/0/idx` on testnet.
    fn bip44_path(idx: u32) -> DerivationPath {
        DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: idx },
        ])
    }

    #[test]
    fn funded_generated_addresses_sum_into_their_category() {
        let a0 = addr_from_byte(41);
        let a5 = addr_from_byte(42);

        let mut address_paths = BTreeMap::new();
        address_paths.insert(a0.clone(), bip44_path(0));
        address_paths.insert(a5.clone(), bip44_path(5));

        let mut address_balances = BTreeMap::new();
        address_balances.insert(a0, 1_000u64);
        address_balances.insert(a5, 5_000u64);

        let summaries =
            collect_account_summaries(Network::Testnet, &address_balances, &address_paths);

        let core_total: u64 = summaries.iter().map(|s| s.confirmed_balance).sum();
        assert_eq!(core_total, 6_000);

        let bip44 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44)
            .expect("bip44 summary");
        assert_eq!(bip44.confirmed_balance, 6_000);
    }

    #[test]
    fn funded_address_missing_from_paths_is_counted_once_as_other() {
        // Funded, generated (in address_paths) address.
        let generated = addr_from_byte(50);
        // Funded address the generated-path set has not indexed yet.
        let unindexed = addr_from_byte(51);

        let mut address_paths = BTreeMap::new();
        address_paths.insert(generated.clone(), bip44_path(0));

        let mut address_balances = BTreeMap::new();
        address_balances.insert(generated, 1_000u64);
        address_balances.insert(unindexed, 5_000u64);

        let summaries =
            collect_account_summaries(Network::Testnet, &address_balances, &address_paths);

        // Neither balance is dropped, and the generated address is not
        // double-counted by the authoritative pass.
        let core_total: u64 = summaries.iter().map(|s| s.confirmed_balance).sum();
        assert_eq!(core_total, 6_000);

        let other = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Other(DerivationPathReference::Unknown))
            .expect("other summary for the unindexed funded address");
        assert_eq!(other.confirmed_balance, 5_000);
    }

    #[test]
    fn generated_address_present_in_both_maps_is_counted_exactly_once() {
        let shared = addr_from_byte(60);

        let mut address_paths = BTreeMap::new();
        address_paths.insert(shared.clone(), bip44_path(0));

        let mut address_balances = BTreeMap::new();
        address_balances.insert(shared, 4_000u64);

        let summaries =
            collect_account_summaries(Network::Testnet, &address_balances, &address_paths);

        let total: u64 = summaries.iter().map(|s| s.confirmed_balance).sum();
        assert_eq!(
            total, 4_000,
            "the shared address is counted once, not twice"
        );
    }

    #[test]
    fn two_funded_bip44_accounts_keep_distinct_per_account_totals() {
        // Account #0: two receive addresses summing to 3.5 DASH.
        let a0 = addr_from_byte(80);
        let a1 = addr_from_byte(81);
        // Account #1: one address holding 0.5 DASH.
        let b0 = addr_from_byte(82);
        let acct1_path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);

        let mut address_paths = BTreeMap::new();
        address_paths.insert(a0.clone(), bip44_path(0));
        address_paths.insert(a1.clone(), bip44_path(1));
        address_paths.insert(b0.clone(), acct1_path);

        let mut address_balances = BTreeMap::new();
        address_balances.insert(a0, 250_000_000u64);
        address_balances.insert(a1, 100_000_000u64);
        address_balances.insert(b0, 50_000_000u64);

        let summaries =
            collect_account_summaries(Network::Testnet, &address_balances, &address_paths);

        let acct0 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44 && s.index == Some(0))
            .expect("account #0 summary");
        assert_eq!(
            acct0.confirmed_balance, 350_000_000,
            "3.5 DASH in account #0"
        );

        let acct1 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44 && s.index == Some(1))
            .expect("account #1 summary");
        assert_eq!(
            acct1.confirmed_balance, 50_000_000,
            "0.5 DASH in account #1"
        );

        // The per-account Core totals sum to the wallet's Core total (4.0 DASH),
        // the figure the wallet header derives from the same snapshot balance.
        let core_total: u64 = summaries.iter().map(|s| s.confirmed_balance).sum();
        assert_eq!(core_total, 400_000_000);
    }

    #[test]
    fn unfunded_generated_account_still_gets_a_zero_balance_summary() {
        // A generated but unfunded BIP-44 account #1 must still surface a tab.
        let acct1_addr = addr_from_byte(70);
        let acct1_path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 0 },
        ]);

        let mut address_paths = BTreeMap::new();
        address_paths.insert(acct1_addr, acct1_path);
        let address_balances = BTreeMap::new();

        let summaries =
            collect_account_summaries(Network::Testnet, &address_balances, &address_paths);

        let acct1 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44 && s.index == Some(1))
            .expect("unfunded BIP44 account #1 summary");
        assert_eq!(acct1.confirmed_balance, 0);
    }
}
