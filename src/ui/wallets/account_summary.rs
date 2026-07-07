use std::collections::BTreeMap;

use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;

use crate::model::wallet::{DerivationPathHelpers, DerivationPathReference, Wallet};

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
    /// Platform credits balance for Platform Payment addresses
    pub platform_credits: Credits,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct AccountKey {
    category: AccountCategory,
    index: Option<u32>,
}

struct AccountSummaryBuilder {
    key: AccountKey,
    confirmed_balance: u64,
    platform_credits: Credits,
}

impl AccountSummaryBuilder {
    fn new(category: AccountCategory, index: Option<u32>) -> Self {
        Self {
            key: AccountKey { category, index },
            confirmed_balance: 0,
            platform_credits: 0,
        }
    }

    fn add_address(&mut self, balance: u64, platform_credits: Credits) {
        self.confirmed_balance += balance;
        self.platform_credits += platform_credits;
    }

    fn build(self) -> AccountSummary {
        AccountSummary {
            category: self.key.category,
            index: self.key.index,
            confirmed_balance: self.confirmed_balance,
            platform_credits: self.platform_credits,
        }
    }
}

/// Categorize a funded chain address the watched set hasn't indexed yet, using
/// the authoritative derivation path from the `WalletBackend` snapshot.
///
/// Falls back to an `Other(Unknown)` bucket when no path is known, so a funded
/// address is still counted in the tab totals rather than silently dropped.
fn categorize_snapshot_address(
    network: Network,
    address: &Address,
    address_paths: &BTreeMap<Address, DerivationPath>,
) -> (AccountCategory, Option<u32>) {
    match address_paths.get(address) {
        Some(path) => categorize_account_path(path, network, DerivationPathReference::Unknown),
        None => (
            AccountCategory::Other(DerivationPathReference::Unknown),
            None,
        ),
    }
}

/// Build per-account summaries for the wallet.
///
/// Two passes feed the per-category totals:
///
/// 1. **Watched addresses** — the source of per-category structure, address
///    labels (via each address's stored `path_reference`), and Platform credits
///    from `Wallet.platform_address_info` (kept current by the coordinator-push
///    path `AppContext::apply_platform_address_push` and the manual
///    `FetchPlatformAddressBalances` task). Chain balances come from the
///    display-only `WalletBackend` snapshot `address_balances` (P4a — replaces
///    the dropped `Wallet.address_balances`).
/// 2. **Authoritative funded addresses** — every address in `address_balances`
///    the watched set has NOT indexed yet, categorized via the snapshot's
///    `address_paths`. This closes the desync where funds on addresses past
///    DET's bookkeeping window were dropped from the tab totals. Each chain
///    balance is counted exactly once (pass 2 skips addresses pass 1 covered).
pub fn collect_account_summaries(
    wallet: &Wallet,
    network: Network,
    address_balances: &BTreeMap<Address, u64>,
    address_paths: &BTreeMap<Address, DerivationPath>,
) -> Vec<AccountSummary> {
    let mut builders: BTreeMap<AccountKey, AccountSummaryBuilder> = BTreeMap::new();

    // Pass 1: watched addresses (structure + labels + Platform credits).
    let mut counted: std::collections::BTreeSet<Address> = std::collections::BTreeSet::new();
    for (path, info) in &wallet.watched_addresses {
        let (category, index) = categorize_account_path(path, network, info.path_reference);

        let balance = address_balances
            .get(&info.address)
            .copied()
            .unwrap_or_default();

        // Get Platform credits balance for Platform Payment addresses
        // Use canonical lookup to handle potential Address key mismatches
        let platform_credits = wallet
            .get_platform_address_info(&info.address)
            .map(|info| info.balance)
            .unwrap_or_default();

        builders
            .entry(AccountKey {
                category: category.clone(),
                index,
            })
            .or_insert_with(|| AccountSummaryBuilder::new(category, index))
            .add_address(balance, platform_credits);
        counted.insert(info.address.clone());
    }

    // Pass 2: authoritative funded addresses the watched set missed.
    for (address, &balance) in address_balances {
        if balance == 0 || counted.contains(address) {
            continue;
        }
        let (category, index) = categorize_snapshot_address(network, address, address_paths);
        builders
            .entry(AccountKey {
                category: category.clone(),
                index,
            })
            .or_insert_with(|| AccountSummaryBuilder::new(category, index))
            .add_address(balance, 0);
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

    #[test]
    fn snapshot_address_without_a_known_path_still_gets_a_bucket() {
        let addr = addr_from_byte(9);
        let paths = BTreeMap::new();
        let (category, index) = categorize_snapshot_address(Network::Testnet, &addr, &paths);
        assert_eq!(
            category,
            AccountCategory::Other(DerivationPathReference::Unknown)
        );
        assert_eq!(index, None);
    }

    #[test]
    fn funded_address_missing_from_watched_is_counted_once() {
        let wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, None, None).expect("test wallet");

        // The single bootstrapped watched receive address (m/44'/1'/0'/0/0).
        let watched = wallet
            .watched_addresses
            .values()
            .next()
            .expect("bootstrapped receive address")
            .address
            .clone();

        // A funded address the watched set has not indexed yet (m/44'/1'/0'/0/5).
        let unwatched = addr_from_byte(42);
        let unwatched_path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index: 5 },
        ]);

        let mut address_balances = BTreeMap::new();
        address_balances.insert(watched.clone(), 1_000u64);
        address_balances.insert(unwatched.clone(), 5_000u64);

        let mut address_paths = BTreeMap::new();
        address_paths.insert(unwatched.clone(), unwatched_path);

        let summaries =
            collect_account_summaries(&wallet, Network::Testnet, &address_balances, &address_paths);

        // Neither balance is dropped, and the watched address is not
        // double-counted by the authoritative pass.
        let core_total: u64 = summaries.iter().map(|s| s.confirmed_balance).sum();
        assert_eq!(core_total, 6_000);

        let bip44 = summaries
            .iter()
            .find(|s| s.category == AccountCategory::Bip44)
            .expect("bip44 summary");
        assert_eq!(bip44.confirmed_balance, 6_000);
    }
}
