mod add_key_to_identity;
mod load_identity;
mod register_dpns_name;
mod register_identity;
mod withdraw_from_identity;

use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose};
use dash_sdk::dpp::ProtocolError;
use dash_sdk::platform::{Identifier, Identity, IdentityPublicKey};
use dash_sdk::Sdk;
use std::collections::{HashMap, HashSet};

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
pub struct IdentityRegistrationInfo {
    pub identity_id_input: String,
    pub alias_input: String,
    pub master_private_key_input: String,
    pub master_private_key_type: KeyType,
    pub keys_input: Vec<(String, KeyType)>,
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
    AddKeyToIdentity(QualifiedIdentity, IdentityPublicKey, Vec<u8>),
    WithdrawFromIdentity(QualifiedIdentity, Option<Address>, Credits, Option<KeyID>),
    RegisterDpnsName(RegisterDpnsNameInput),
}

fn verify_key_input(
    untrimmed_private_key: String,
    type_key: &str,
) -> Result<Option<Vec<u8>>, String> {
    let private_key = untrimmed_private_key.trim().to_string();
    match private_key.len() {
        64 => {
            // hex
            match hex::decode(private_key.as_str()) {
                Ok(decoded) => Ok(Some(decoded)),
                Err(_) => Err(format!(
                    "{} key is the size of a hex key but isn't hex",
                    type_key
                )),
            }
        }
        51 | 52 => {
            // wif
            match PrivateKey::from_wif(private_key.as_str()) {
                Ok(key) => Ok(Some(key.to_bytes())),
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
        private_voting_key: &[u8],
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
        private_voting_key: &[u8],
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
        private_voting_key: &[u8],
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

    pub async fn run_identity_task(&self, task: IdentityTask, sdk: &Sdk) -> Result<(), String> {
        match task {
            IdentityTask::LoadIdentity(input) => self.load_identity(sdk, input).await,
            IdentityTask::WithdrawFromIdentity(qualified_identity, to_address, credits, id) => {
                self.withdraw_from_identity(sdk, qualified_identity, to_address, credits, id)
                    .await
            }
            IdentityTask::AddKeyToIdentity(qualified_identity, public_key_to_add, private_key) => {
                self.add_key_to_identity(sdk, qualified_identity, public_key_to_add, private_key)
                    .await
            }
            IdentityTask::RegisterIdentity(registration_info) => {
                self.register_identity(sdk, registration_info).await
            }
            IdentityTask::RegisterDpnsName(input) => self.register_dpns_name(sdk, input).await,
        }
    }
}
