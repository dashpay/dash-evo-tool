mod add_key_to_identity;
mod load_identity;
mod load_identity_from_wallet;
mod refresh_identity;
mod refresh_loaded_identities_dpns_names;
mod register_dpns_name;
mod register_identity;
mod top_up_identity;
mod transfer;
mod withdraw_from_identity;

use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, WalletDerivationPath};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::{Wallet, WalletArcRef, WalletSeedHash};
use dash_sdk::dashcore_rpc::dashcore::bip32::DerivationPath;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey, TxOut};
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{OutPoint, Transaction};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::platform::{Identifier, Identity, IdentityPublicKey};
use dash_sdk::Sdk;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub struct IdentityInputToLoad {
    pub identity_id_input: String,
    pub identity_type: IdentityType,
    pub alias_input: String,
    pub voting_private_key_input: String,
    pub owner_private_key_input: String,
    pub payout_address_private_key_input: String,
    pub keys_input: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdentityKeys {
    pub(crate) master_private_key: Option<(PrivateKey, DerivationPath)>,
    pub(crate) master_private_key_type: KeyType,
    pub(crate) keys_input: Vec<(
        (PrivateKey, DerivationPath),
        KeyType,
        Purpose,
        SecurityLevel,
    )>,
}

impl IdentityKeys {
    pub fn to_key_storage(&self, wallet_seed_hash: WalletSeedHash) -> KeyStorage {
        let Self {
            master_private_key,
            master_private_key_type,
            keys_input,
        } = self;
        let secp = Secp256k1::new();
        let mut key_map = BTreeMap::new();

        if let Some((master_private_key, master_private_key_derivation_path)) = master_private_key {
            let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: 0,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::MASTER,
                contract_bounds: None,
                key_type: *master_private_key_type,
                read_only: false,
                data: master_private_key.public_key(&secp).to_bytes().into(),
                disabled_at: None,
            });

            let wallet_derivation_path = WalletDerivationPath {
                wallet_seed_hash,
                derivation_path: master_private_key_derivation_path.clone(),
            };
            let qualified_identity_public_key =
                QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                    key,
                    Some(wallet_derivation_path.clone()),
                );
            key_map.insert(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
                (qualified_identity_public_key, wallet_derivation_path),
            );
        }

        key_map.extend(keys_input.iter().enumerate().map(
            |(i, ((private_key, derivation_path), key_type, purpose, security_level))| {
                let id = (i + 1) as KeyID;
                let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                    id,
                    purpose: *purpose,
                    security_level: *security_level,
                    contract_bounds: None,
                    key_type: *key_type,
                    read_only: false,
                    data: private_key.public_key(&secp).to_bytes().into(),
                    disabled_at: None,
                });

                let wallet_derivation_path = WalletDerivationPath {
                    wallet_seed_hash,
                    derivation_path: derivation_path.clone(),
                };

                let qualified_identity_public_key =
                    QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                        identity_public_key,
                        Some(wallet_derivation_path.clone()),
                    );
                (
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, id),
                    (qualified_identity_public_key, wallet_derivation_path),
                )
            },
        ));

        key_map.into()
    }
    pub fn to_public_keys_map(&self) -> BTreeMap<KeyID, IdentityPublicKey> {
        let Self {
            master_private_key,
            master_private_key_type,
            keys_input,
            ..
        } = self;
        let secp = Secp256k1::new();
        let mut key_map = BTreeMap::new();
        if let Some((master_private_key, _)) = master_private_key {
            let data = match master_private_key_type {
                KeyType::ECDSA_SECP256K1 => master_private_key.public_key(&secp).to_bytes().into(),
                KeyType::ECDSA_HASH160 => master_private_key
                    .public_key(&secp)
                    .pubkey_hash()
                    .to_byte_array()
                    .to_vec()
                    .into(),
                _ => panic!("need a ECDSA Key for now"),
            };
            let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: 0,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::MASTER,
                contract_bounds: None,
                key_type: *master_private_key_type,
                read_only: false,
                data,
                disabled_at: None,
            });

            key_map.insert(0, key);
        }
        key_map.extend(keys_input.iter().enumerate().map(
            |(i, ((private_key, _), key_type, purpose, security_level))| {
                let id = (i + 1) as KeyID;
                let data = match key_type {
                    KeyType::ECDSA_SECP256K1 => private_key.public_key(&secp).to_bytes().into(),
                    KeyType::ECDSA_HASH160 => private_key
                        .public_key(&secp)
                        .pubkey_hash()
                        .to_byte_array()
                        .to_vec()
                        .into(),
                    _ => panic!("need a ECDSA Key for now"),
                };
                let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                    id,
                    purpose: *purpose,
                    security_level: *security_level,
                    contract_bounds: None,
                    key_type: *key_type,
                    read_only: false,
                    data,
                    disabled_at: None,
                });
                (id, identity_public_key)
            },
        ));

        key_map
    }
}

pub type IdentityIndex = u32;
pub type TopUpIndex = u32;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterIdentityFundingMethod {
    UseAssetLock(Address, Box<AssetLockProof>, Box<Transaction>),
    FundWithUtxo(OutPoint, TxOut, Address, IdentityIndex),
    FundWithWallet(Duffs, IdentityIndex),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopUpIdentityFundingMethod {
    UseAssetLock(Address, Box<AssetLockProof>, Box<Transaction>),
    FundWithUtxo(OutPoint, TxOut, Address, IdentityIndex, TopUpIndex),
    FundWithWallet(Duffs, IdentityIndex, TopUpIndex),
}

#[derive(Debug, Clone)]
pub struct IdentityRegistrationInfo {
    pub alias_input: String,
    pub keys: IdentityKeys,
    pub wallet: Arc<RwLock<Wallet>>,
    pub wallet_identity_index: u32,
    pub identity_funding_method: RegisterIdentityFundingMethod,
}

impl PartialEq for IdentityRegistrationInfo {
    fn eq(&self, other: &Self) -> bool {
        self.alias_input == other.alias_input
            && self.identity_funding_method == other.identity_funding_method
            && self.keys == other.keys
    }
}

#[derive(Debug, Clone)]
pub struct IdentityTopUpInfo {
    pub qualified_identity: QualifiedIdentity,
    pub wallet: Arc<RwLock<Wallet>>,
    pub identity_funding_method: TopUpIdentityFundingMethod,
}

impl PartialEq for IdentityTopUpInfo {
    fn eq(&self, other: &Self) -> bool {
        self.qualified_identity == other.qualified_identity
            && self.identity_funding_method == other.identity_funding_method
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegisterDpnsNameInput {
    pub qualified_identity: QualifiedIdentity,
    pub name_input: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum IdentityTask {
    LoadIdentity(IdentityInputToLoad),
    SearchIdentityFromWallet(WalletArcRef, IdentityIndex),
    RegisterIdentity(IdentityRegistrationInfo),
    TopUpIdentity(IdentityTopUpInfo),
    AddKeyToIdentity(QualifiedIdentity, QualifiedIdentityPublicKey, [u8; 32]),
    WithdrawFromIdentity(QualifiedIdentity, Option<Address>, Credits, Option<KeyID>),
    Transfer(QualifiedIdentity, Identifier, Credits, Option<KeyID>),
    RegisterDpnsName(RegisterDpnsNameInput),
    RefreshIdentity(QualifiedIdentity),
    RefreshLoadedIdentitiesOwnedDPNSNames,
}

fn verify_key_input(
    untrimmed_private_key: String,
    type_key: &str,
) -> Result<Option<[u8; 32]>, String> {
    let private_key = untrimmed_private_key.trim().to_string();
    match private_key.len() {
        64 => {
            // hex
            match hex::decode(private_key.as_str()) {
                Ok(decoded) => Ok(Some(decoded.try_into().unwrap())),
                Err(_) => Err(format!(
                    "{} key is the size of a hex key but isn't hex",
                    type_key
                )),
            }
        }
        51 | 52 => {
            // wif
            match PrivateKey::from_wif(private_key.as_str()) {
                Ok(key) => Ok(Some(key.inner.secret_bytes())),
                Err(_) => Err(format!(
                    "{} key is the length of a WIF key but is invalid",
                    type_key
                )),
            }
        }
        0 => Ok(None),
        _ => Err(format!("{} key is of incorrect size", type_key)),
    }
}

impl AppContext {
    fn verify_voting_key_exists_on_identity(
        &self,
        voting_identity: &Identity,
        private_voting_key: &[u8; 32],
    ) -> Result<IdentityPublicKey, String> {
        // We start by getting all the voting keys
        let voting_keys: Vec<IdentityPublicKey> = voting_identity
            .public_keys()
            .values()
            .filter_map(|key| {
                if key.purpose() != Purpose::VOTING {
                    return None;
                }
                Some(key.clone())
            })
            .collect();
        if voting_keys.is_empty() {
            return Err("This identity does not contain any voting keys".to_string());
        }
        // Then we get all the key types of the voting keys
        let key_types: HashSet<KeyType> = voting_keys.iter().map(|key| key.key_type()).collect();
        // For every key type get the associated public key data
        let public_key_bytes_for_each_key_type = key_types
            .into_iter()
            .map(|key_type| {
                Ok((
                    key_type,
                    key_type
                        .public_key_data_from_private_key_data(private_voting_key, self.network)?,
                ))
            })
            .collect::<Result<HashMap<KeyType, Vec<u8>>, ProtocolError>>()
            .map_err(|e| e.to_string())?;
        let Some(key) = voting_keys.into_iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(
                "Identity does not have a voting public key matching this private key".to_string(),
            );
        };
        Ok(key)
    }

    fn verify_owner_key_exists_on_identity(
        &self,
        identity: &Identity,
        private_voting_key: &[u8; 32],
    ) -> Result<IdentityPublicKey, String> {
        // We start by getting all the voting keys
        let owner_keys: Vec<IdentityPublicKey> = identity
            .public_keys()
            .values()
            .filter_map(|key| {
                if key.purpose() != Purpose::OWNER {
                    return None;
                }
                Some(key.clone())
            })
            .collect();
        if owner_keys.is_empty() {
            return Err("This identity does not contain any owner keys".to_string());
        }
        // Then we get all the key types of the voting keys
        let key_types: HashSet<KeyType> = owner_keys.iter().map(|key| key.key_type()).collect();
        // For every key type get the associated public key data
        let public_key_bytes_for_each_key_type = key_types
            .into_iter()
            .map(|key_type| {
                Ok((
                    key_type,
                    key_type
                        .public_key_data_from_private_key_data(private_voting_key, self.network)?,
                ))
            })
            .collect::<Result<HashMap<KeyType, Vec<u8>>, ProtocolError>>()
            .map_err(|e| e.to_string())?;
        let Some(key) = owner_keys.into_iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(
                "Identity does not have an owner public key matching this private key".to_string(),
            );
        };
        Ok(key)
    }

    fn verify_payout_address_key_exists_on_identity(
        &self,
        identity: &Identity,
        private_voting_key: &[u8; 32],
    ) -> Result<IdentityPublicKey, String> {
        // We start by getting all the voting keys
        let owner_keys: Vec<IdentityPublicKey> = identity
            .public_keys()
            .values()
            .filter_map(|key| {
                if key.purpose() != Purpose::TRANSFER {
                    return None;
                }
                if key.key_type() != KeyType::ECDSA_HASH160 {
                    return None;
                }
                Some(key.clone())
            })
            .collect();
        if owner_keys.is_empty() {
            return Err("This identity does not contain any owner keys".to_string());
        }
        // Then we get all the key types of the voting keys
        let key_types: HashSet<KeyType> = owner_keys.iter().map(|key| key.key_type()).collect();
        // For every key type get the associated public key data
        let public_key_bytes_for_each_key_type = key_types
            .into_iter()
            .map(|key_type| {
                Ok((
                    key_type,
                    key_type
                        .public_key_data_from_private_key_data(private_voting_key, self.network)?,
                ))
            })
            .collect::<Result<HashMap<KeyType, Vec<u8>>, ProtocolError>>()
            .map_err(|e| e.to_string())?;
        let Some(key) = owner_keys.into_iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(
                "Identity does not have a payout address matching this private key".to_string(),
            );
        };
        Ok(key)
    }

    pub async fn run_identity_task(
        &self,
        task: IdentityTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            IdentityTask::LoadIdentity(input) => self.load_identity(sdk, input).await,
            IdentityTask::WithdrawFromIdentity(qualified_identity, to_address, credits, id) => {
                self.withdraw_from_identity(qualified_identity, to_address, credits, id)
                    .await
            }
            IdentityTask::AddKeyToIdentity(qualified_identity, public_key_to_add, private_key) => {
                self.add_key_to_identity(sdk, qualified_identity, public_key_to_add, private_key)
                    .await
            }
            IdentityTask::RegisterIdentity(registration_info) => {
                self.register_identity(registration_info, sender).await
            }
            IdentityTask::RegisterDpnsName(input) => self.register_dpns_name(sdk, input).await,
            IdentityTask::RefreshIdentity(qualified_identity) => self
                .refresh_identity(sdk, qualified_identity, sender)
                .await
                .map_err(|e| format!("Error refreshing identity: {}", e)),
            IdentityTask::Transfer(qualified_identity, to_identifier, credits, id) => {
                self.transfer_to_identity(qualified_identity, to_identifier, credits, id)
                    .await
            }
            IdentityTask::SearchIdentityFromWallet(wallet, identity_index) => {
                self.load_user_identity_from_wallet(sdk, wallet, identity_index)
                    .await
            }
            IdentityTask::TopUpIdentity(top_up_info) => {
                self.top_up_identity(top_up_info, sender).await
            }
            IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames => {
                self.refresh_loaded_identities_dpns_names(sender).await
            }
        }
    }
}
