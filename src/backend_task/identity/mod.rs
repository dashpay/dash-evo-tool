mod add_key_to_identity;
mod load_identity;
mod refresh_identity;
mod register_dpns_name;
mod register_identity;
mod withdraw_from_identity;
mod transfer;

use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_identity::{
    EncryptedPrivateKeyTarget, IdentityType, QualifiedIdentity,
};
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey};
use dash_sdk::dpp::balances::credits::Duffs;
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
use dash_sdk::dpp::dashcore::Transaction;
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
    pub(crate) master_private_key: Option<PrivateKey>,
    pub(crate) master_private_key_type: KeyType,
    pub(crate) keys_input: Vec<(PrivateKey, KeyType, Purpose, SecurityLevel)>,
}

impl IdentityKeys {
    pub fn to_encrypted_private_keys(
        &self,
    ) -> BTreeMap<(EncryptedPrivateKeyTarget, KeyID), (IdentityPublicKey, [u8; 32])> {
        let Self {
            master_private_key,
            master_private_key_type,
            keys_input,
        } = self;
        let secp = Secp256k1::new();
        let mut key_map = BTreeMap::new();
        if let Some(master_private_key) = master_private_key {
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

            key_map.insert(
                (EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
                (key, master_private_key.inner.secret_bytes()),
            );
        }
        key_map.extend(keys_input.iter().enumerate().map(
            |(i, (private_key, key_type, purpose, security_level))| {
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
                (
                    (EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity, id),
                    (identity_public_key, private_key.inner.secret_bytes()),
                )
            },
        ));

        key_map
    }
    pub fn to_public_keys_map(&self) -> BTreeMap<KeyID, IdentityPublicKey> {
        let Self {
            master_private_key,
            master_private_key_type,
            keys_input,
        } = self;
        let secp = Secp256k1::new();
        let mut key_map = BTreeMap::new();
        if let Some(master_private_key) = master_private_key {
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

            key_map.insert(0, key);
        }
        key_map.extend(keys_input.iter().enumerate().map(
            |(i, (private_key, key_type, purpose, security_level))| {
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
                (id, identity_public_key)
            },
        ));

        key_map
    }
}

pub type IdentityIndex = u32;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityRegistrationMethod {
    UseAssetLock(Address, AssetLockProof, Transaction),
    FundWithWallet(Duffs, IdentityIndex),
}

#[derive(Debug, Clone)]
pub struct IdentityRegistrationInfo {
    pub alias_input: String,
    pub keys: IdentityKeys,
    pub wallet: Arc<RwLock<Wallet>>,
    pub identity_registration_method: IdentityRegistrationMethod,
}

impl PartialEq for IdentityRegistrationInfo {
    fn eq(&self, other: &Self) -> bool {
        self.alias_input == other.alias_input
            && self.identity_registration_method == other.identity_registration_method
            && self.keys == other.keys
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
    RegisterIdentity(IdentityRegistrationInfo),
    AddKeyToIdentity(QualifiedIdentity, IdentityPublicKey, [u8; 32]),
    WithdrawFromIdentity(QualifiedIdentity, Option<Address>, Credits, Option<KeyID>),
    Transfer(QualifiedIdentity, Identifier, Credits, Option<KeyID>),
    RegisterDpnsName(RegisterDpnsNameInput),
    RefreshIdentity(QualifiedIdentity),
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
    ) -> Result<(), String> {
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
            IdentityTask::RefreshIdentity(qualified_identity) => {
                self.refresh_identity(sdk, qualified_identity, sender).await
            }
            IdentityTask::Transfer(qualified_identity, to_identifier, credits, id) => {
                self.transfer_to_identity(qualified_identity, to_identifier, credits, id)
                    .await
            }
        }
    }
}
