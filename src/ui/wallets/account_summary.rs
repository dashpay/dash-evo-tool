use std::collections::BTreeMap;

use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::balances::credits::Credits;
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
    /// DIP-17: Platform Payment Addresses (dashevo/tdashevo Bech32m prefix per DIP-18)
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
            AccountCategory::Bip44 => match index.unwrap_or(0) {
                0 => "Main Account".to_string(),
                idx => format!("BIP44 Account #{}", idx),
            },
            AccountCategory::Bip32 => format!("BIP32 Account {:?}", index.unwrap_or(0)),
            AccountCategory::CoinJoin => "CoinJoin".to_string(),
            AccountCategory::IdentityRegistration => "Identity Registration".to_string(),
            AccountCategory::IdentitySystem => "Identity System".to_string(),
            AccountCategory::IdentityTopup => "Identity Top-up".to_string(),
            AccountCategory::IdentityInvitation => "Identity Invitation".to_string(),
            AccountCategory::ProviderVoting => "Provider Voting".to_string(),
            AccountCategory::ProviderOwner => "Provider Owner".to_string(),
            AccountCategory::ProviderOperator => "Provider Operator".to_string(),
            AccountCategory::ProviderPlatform => "Provider Platform".to_string(),
            AccountCategory::PlatformPayment => "Platform Account".to_string(),
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
            AccountCategory::Bip32 => {
                Some("Legacy BIP32 branch reserved for custom derivations or advanced tools.")
            }
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
                "DIP-17 Platform payment addresses (dashevo/tdashevo prefix). Hold Dash Credits on Platform, independent of identities.",
            ),
            AccountCategory::Other(_) => None,
        }
    }

    /// Returns true if this account category is for key derivation/proofs only
    /// and does not hold funds (balance is always N/A).
    pub fn is_key_only(&self) -> bool {
        matches!(
            self,
            AccountCategory::IdentityRegistration
                | AccountCategory::IdentityTopup
                | AccountCategory::IdentityInvitation
                | AccountCategory::IdentitySystem
                | AccountCategory::ProviderVoting
                | AccountCategory::ProviderOwner
                | AccountCategory::ProviderOperator
                | AccountCategory::ProviderPlatform
        )
    }
}

#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub category: AccountCategory,
    pub label: String,
    pub index: Option<u32>,
    pub confirmed_balance: u64,
    /// Platform credits balance for Platform Payment addresses
    pub platform_credits: Credits,
    pub total_addresses: usize,
    pub external_addresses: usize,
    pub internal_addresses: usize,
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
    total_addresses: usize,
    external_addresses: usize,
    internal_addresses: usize,
}

impl AccountSummaryBuilder {
    fn new(category: AccountCategory, index: Option<u32>) -> Self {
        Self {
            key: AccountKey { category, index },
            confirmed_balance: 0,
            platform_credits: 0,
            total_addresses: 0,
            external_addresses: 0,
            internal_addresses: 0,
        }
    }

    fn add_address(
        &mut self,
        path: &DerivationPath,
        balance: u64,
        platform_credits: Credits,
        network: Network,
    ) {
        self.confirmed_balance += balance;
        self.platform_credits += platform_credits;
        self.total_addresses += 1;

        if path.is_bip44_external(network) {
            self.external_addresses += 1;
        } else if path.is_bip44_change(network) {
            self.internal_addresses += 1;
        }
    }

    fn build(self) -> AccountSummary {
        let label = self.key.category.label(self.key.index);

        AccountSummary {
            category: self.key.category,
            label,
            index: self.key.index,
            confirmed_balance: self.confirmed_balance,
            platform_credits: self.platform_credits,
            total_addresses: self.total_addresses,
            external_addresses: self.external_addresses,
            internal_addresses: self.internal_addresses,
        }
    }
}

pub fn collect_account_summaries(wallet: &Wallet, network: Network) -> Vec<AccountSummary> {
    let mut builders: BTreeMap<AccountKey, AccountSummaryBuilder> = BTreeMap::new();

    for (path, info) in &wallet.watched_addresses {
        let category = AccountCategory::from_reference(info.path_reference);
        let index = match category {
            AccountCategory::Bip44 | AccountCategory::Bip32 => path.bip44_account_index(),
            _ => None,
        };

        let balance = wallet
            .address_balances
            .get(&info.address)
            .cloned()
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
            .add_address(path, balance, platform_credits, network);
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
