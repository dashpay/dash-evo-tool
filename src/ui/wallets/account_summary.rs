use std::collections::BTreeMap;

use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;

use crate::model::wallet::{DerivationPathHelpers, DerivationPathReference, Wallet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountCategory {
    Bip44,
    Bip32,
    CoinJoin,
    IdentityRegistration,
    IdentityTopup,
    IdentityInvitation,
    ProviderVoting,
    ProviderOwner,
    ProviderOperator,
    ProviderPlatform,
    Other(DerivationPathReference),
}

impl AccountCategory {
    pub fn from_reference(reference: DerivationPathReference) -> Self {
        match reference {
            DerivationPathReference::BIP44 => AccountCategory::Bip44,
            DerivationPathReference::BIP32 => AccountCategory::Bip32,
            DerivationPathReference::BlockchainIdentities
            | DerivationPathReference::BlockchainIdentityCreditRegistrationFunding => {
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
            DerivationPathReference::ProviderFunds => AccountCategory::CoinJoin,
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
            AccountCategory::IdentityTopup => "Identity Top-up".to_string(),
            AccountCategory::IdentityInvitation => "Identity Invitation".to_string(),
            AccountCategory::ProviderVoting => "Provider Voting".to_string(),
            AccountCategory::ProviderOwner => "Provider Owner".to_string(),
            AccountCategory::ProviderOperator => "Provider Operator".to_string(),
            AccountCategory::ProviderPlatform => "Provider Platform".to_string(),
            AccountCategory::Other(reference) => format!("{:?}", reference),
        }
    }

    fn sort_key(&self) -> u8 {
        match self {
            AccountCategory::Bip44 => 0,
            AccountCategory::Bip32 => 1,
            AccountCategory::CoinJoin => 2,
            AccountCategory::IdentityRegistration => 3,
            AccountCategory::IdentityTopup => 4,
            AccountCategory::IdentityInvitation => 5,
            AccountCategory::ProviderOwner => 6,
            AccountCategory::ProviderVoting => 7,
            AccountCategory::ProviderOperator => 8,
            AccountCategory::ProviderPlatform => 9,
            AccountCategory::Other(_) => 10,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub category: AccountCategory,
    pub label: String,
    pub index: Option<u32>,
    pub confirmed_balance: u64,
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
    total_addresses: usize,
    external_addresses: usize,
    internal_addresses: usize,
}

impl AccountSummaryBuilder {
    fn new(category: AccountCategory, index: Option<u32>) -> Self {
        Self {
            key: AccountKey { category, index },
            confirmed_balance: 0,
            total_addresses: 0,
            external_addresses: 0,
            internal_addresses: 0,
        }
    }

    fn add_address(&mut self, path: &DerivationPath, balance: u64, network: Network) {
        self.confirmed_balance += balance;
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

        builders
            .entry(AccountKey {
                category: category.clone(),
                index,
            })
            .or_insert_with(|| AccountSummaryBuilder::new(category, index))
            .add_address(path, balance, network);
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
