use crate::model::wallet::{DerivationPathReference, Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::{Address, Network};
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath, ExtendedPubKey};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct WatchOnlyAccount {
    pub seed_hash: WalletSeedHash,
    pub account_index: u32,
    pub xpub: ExtendedPubKey,
}

#[derive(Debug, Clone)]
pub struct WatchOnlyAccountAddresses {
    pub account_type: AccountType,
    pub addresses: Vec<Address>,
}

#[derive(Debug, Clone)]
pub struct WatchOnlyAddressRegistration {
    pub seed_hash: WalletSeedHash,
    pub entries: Vec<WatchOnlyAccountAddresses>,
}

#[derive(Debug, Clone)]
pub struct WatchOnlyWalletAttachment {
    pub network: Network,
    pub accounts: Vec<WatchOnlyAccount>,
    pub identity_registrations: Vec<WatchOnlyAddressRegistration>,
}

impl WatchOnlyWalletAttachment {
    pub fn from_wallets(
        network: Network,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
    ) -> Self {
        let mut accounts = Vec::new();
        let mut identity_registrations = Vec::new();

        for (seed_hash, wallet_arc) in wallets.iter() {
            let wallet = wallet_arc.read().expect("wallet lock poisoned");
            let xpub = wallet.master_bip44_ecdsa_extended_public_key;
            accounts.push(WatchOnlyAccount {
                seed_hash: *seed_hash,
                account_index: 0,
                xpub,
            });

            let mut reference_counts: BTreeMap<DerivationPathReference, usize> = BTreeMap::new();
            for reference in wallet
                .watched_addresses
                .values()
                .map(|info| info.path_reference)
            {
                *reference_counts.entry(reference).or_default() += 1;
            }

            tracing::debug!(
                seed = %hex::encode(seed_hash),
                total = wallet.watched_addresses.len(),
                ?reference_counts,
                "Wallet watched-address breakdown",
            );

            let mut batches: BTreeMap<IdentityAccountKind, Vec<Address>> = BTreeMap::new();
            for (path, info) in &wallet.watched_addresses {
                if let Some(kind) = Self::identity_account_kind(path, info.path_reference) {
                    batches.entry(kind).or_default().push(info.address.clone());
                }
            }

            if !batches.is_empty() {
                tracing::info!(
                    seed = %hex::encode(seed_hash),
                    categories = batches.len(),
                    "Preparing identity watch-only addresses",
                );
                let entries = batches
                    .into_iter()
                    .map(|(kind, mut list)| {
                        list.sort_by_key(|addr| addr.to_string());
                        list.dedup();
                        WatchOnlyAccountAddresses {
                            account_type: kind.to_account_type(),
                            addresses: list,
                        }
                    })
                    .collect();

                identity_registrations.push(WatchOnlyAddressRegistration {
                    seed_hash: *seed_hash,
                    entries,
                });
            }
        }

        Self {
            network,
            accounts,
            identity_registrations,
        }
    }

    fn identity_account_kind(
        path: &DerivationPath,
        reference: DerivationPathReference,
    ) -> Option<IdentityAccountKind> {
        match reference {
            DerivationPathReference::BlockchainIdentityCreditRegistrationFunding => {
                Some(IdentityAccountKind::Registration)
            }
            DerivationPathReference::BlockchainIdentityCreditInvitationFunding => {
                Some(IdentityAccountKind::Invitation)
            }
            DerivationPathReference::BlockchainIdentityCreditTopupFunding => {
                let components = path.as_ref();
                if components.len() > 4 {
                    let registration_index = Self::child_number_index(components[4]);
                    Some(IdentityAccountKind::TopUp {
                        registration_index: Some(registration_index),
                    })
                } else {
                    Some(IdentityAccountKind::TopUp {
                        registration_index: None,
                    })
                }
            }
            _ => None,
        }
    }

    fn child_number_index(child: ChildNumber) -> u32 {
        match child {
            ChildNumber::Normal { index } | ChildNumber::Hardened { index } => index,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IdentityAccountKind {
    Registration,
    Invitation,
    TopUp { registration_index: Option<u32> },
}

impl IdentityAccountKind {
    fn to_account_type(self) -> AccountType {
        match self {
            IdentityAccountKind::Registration => AccountType::IdentityRegistration,
            IdentityAccountKind::Invitation => AccountType::IdentityInvitation,
            IdentityAccountKind::TopUp { registration_index } => match registration_index {
                Some(idx) => AccountType::IdentityTopUp {
                    registration_index: idx,
                },
                None => AccountType::IdentityTopUpNotBoundToIdentity,
            },
        }
    }
}
