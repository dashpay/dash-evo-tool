use crate::context::AppContext;
use crate::model::qualified_identity::EncryptedPrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identifier::MasternodeIdentifiers;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::withdraw_from_identity::WithdrawFromIdentity;
use dash_sdk::platform::{Fetch, Identifier, Identity, IdentityPublicKey};
use dash_sdk::Sdk;
use futures::TryFutureExt;
use std::collections::{BTreeMap, HashMap, HashSet};

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
pub(crate) enum IdentityTask {
    LoadIdentity(IdentityInputToLoad),
    AddKeyToIdentity(QualifiedIdentity, IdentityPublicKey, Vec<u8>),
    WithdrawFromIdentity(QualifiedIdentity, Option<Address>, Credits, Option<KeyID>),
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
            IdentityTask::LoadIdentity(input) => {
                let IdentityInputToLoad {
                    identity_id_input,
                    identity_type,
                    voting_private_key_input,
                    alias_input,
                    owner_private_key_input,
                    payout_address_private_key_input,
                    keys_input,
                } = input;

                // Verify the voting private key
                let owner_private_key_bytes = verify_key_input(owner_private_key_input, "Owner")?;

                // Verify the voting private key
                let voting_private_key_bytes =
                    verify_key_input(voting_private_key_input, "Voting")?;

                let payout_address_private_key_bytes =
                    verify_key_input(payout_address_private_key_input, "Payout Address")?;

                // Parse the identity ID
                let identity_id =
                    match Identifier::from_string(&identity_id_input, Encoding::Base58)
                        .or_else(|_| Identifier::from_string(&identity_id_input, Encoding::Hex))
                    {
                        Ok(id) => id,
                        Err(e) => return Err(format!("Identifier error: {}", e)),
                    };

                // Fetch the identity using the SDK
                let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
                    Ok(Some(identity)) => identity,
                    Ok(None) => return Err("Identity not found".to_string()),
                    Err(e) => return Err(format!("Error fetching identity: {}", e)),
                };

                let mut encrypted_private_keys = BTreeMap::new();

                if identity_type != IdentityType::User && owner_private_key_bytes.is_some() {
                    let owner_private_key_bytes = owner_private_key_bytes.unwrap();
                    let key = self.verify_owner_key_exists_on_identity(
                        &identity,
                        owner_private_key_bytes.as_slice(),
                    )?;
                    encrypted_private_keys.insert(
                        (PrivateKeyOnMainIdentity, key.id()),
                        (key.clone(), owner_private_key_bytes),
                    );
                }

                if identity_type != IdentityType::User && payout_address_private_key_bytes.is_some()
                {
                    let payout_address_private_key_bytes =
                        payout_address_private_key_bytes.unwrap();
                    let key = self.verify_payout_address_key_exists_on_identity(
                        &identity,
                        payout_address_private_key_bytes.as_slice(),
                    )?;
                    encrypted_private_keys.insert(
                        (PrivateKeyOnMainIdentity, key.id()),
                        (key.clone(), payout_address_private_key_bytes),
                    );
                }

                // If the identity type is not a User, and we have a voting private key, verify it
                let associated_voter_identity = if identity_type != IdentityType::User
                    && voting_private_key_bytes.is_some()
                {
                    let voting_private_key_bytes = voting_private_key_bytes.unwrap();
                    if let Ok(private_key) =
                        PrivateKey::from_slice(voting_private_key_bytes.as_slice(), self.network)
                    {
                        // Make the vote identifier
                        let address = private_key.public_key(&Secp256k1::new()).pubkey_hash();
                        let voter_identifier = Identifier::create_voter_identifier(
                            identity_id.as_bytes(),
                            address.as_ref(),
                        );

                        // Fetch the voter identifier
                        let voter_identity =
                            match Identity::fetch_by_identifier(sdk, voter_identifier).await {
                                Ok(Some(identity)) => identity,
                                Ok(None) => return Err("Voter Identity not found".to_string()),
                                Err(e) => {
                                    return Err(format!("Error fetching voter identity: {}", e))
                                }
                            };

                        let key = self.verify_voting_key_exists_on_identity(
                            &voter_identity,
                            voting_private_key_bytes.as_slice(),
                        )?;
                        encrypted_private_keys.insert(
                            (PrivateKeyOnVoterIdentity, key.id()),
                            (key.clone(), voting_private_key_bytes),
                        );
                        Some((voter_identity, key))
                    } else {
                        return Err("Voting private key is not valid".to_string());
                    }
                } else {
                    None
                };

                let qualified_identity = QualifiedIdentity {
                    identity,
                    associated_voter_identity,
                    associated_operator_identity: None,
                    associated_owner_key_id: None,
                    identity_type,
                    alias: if alias_input.is_empty() {
                        None
                    } else {
                        Some(alias_input)
                    },
                    encrypted_private_keys,
                };

                // Insert qualified identity into the database
                self.insert_local_qualified_identity(&qualified_identity)
                    .map_err(|e| format!("Database error: {}", e))?;

                Ok(())
            }
            IdentityTask::WithdrawFromIdentity(mut qualified_identity, to_address, credits, id) => {
                let remaining_balance = qualified_identity
                    .identity
                    .clone()
                    .withdraw(
                        &self.sdk,
                        to_address,
                        credits,
                        Some(1),
                        id.and_then(|key_id| {
                            qualified_identity.identity.get_public_key_by_id(key_id)
                        }),
                        qualified_identity.clone(),
                        None,
                    )
                    .await
                    .map_err(|e| format!("Withdrawal error: {}", e))?;
                qualified_identity.identity.set_balance(remaining_balance);
                self.insert_local_qualified_identity(&qualified_identity)
                    .map_err(|e| format!("Database error: {}", e))
            }
            IdentityTask::AddKeyToIdentity(
                mut qualified_identity,
                mut public_key_to_add,
                private_key,
            ) => {
                let new_identity_nonce = sdk
                    .get_identity_nonce(qualified_identity.identity.id(), true, None)
                    .await
                    .map_err(|e| format!("Fetch nonce error: {}", e))?;
                let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
                    return Ok(());
                };
                let master_key_id = master_key.id();
                let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
                    .await
                    .map_err(|e| format!("Fetch nonce error: {}", e))?
                    .unwrap();
                qualified_identity.identity = identity;
                qualified_identity.identity.bump_revision();
                public_key_to_add.set_id(qualified_identity.identity.get_public_key_max_id() + 1);
                qualified_identity.encrypted_private_keys.insert(
                    (PrivateKeyOnMainIdentity, public_key_to_add.id()),
                    (public_key_to_add.clone(), private_key.clone()),
                );
                let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
                    &qualified_identity.identity,
                    &master_key_id,
                    vec![public_key_to_add.clone()],
                    vec![],
                    new_identity_nonce,
                    UserFeeIncrease::default(),
                    &qualified_identity,
                    sdk.version(),
                    None,
                )
                .map_err(|e| format!("IdentityUpdateTransition error: {}", e))?;

                let result = state_transition
                    .broadcast_and_wait(sdk, None)
                    .await
                    .map_err(|e| format!("Broadcasting error: {}", e))?;

                if let StateTransitionProofResult::VerifiedPartialIdentity(identity) = result {
                    for public_key in identity.loaded_public_keys.into_values() {
                        qualified_identity.identity.add_public_key(public_key);
                    }
                }

                self.insert_local_qualified_identity(&qualified_identity)
                    .map_err(|e| format!("Database error: {}", e))
            }
        }
    }
}
