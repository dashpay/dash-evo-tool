mod add_key_to_identity;
mod discover_identities;
mod load_identity;
mod load_identity_by_dpns_name;
mod load_identity_from_wallet;
mod refresh_identity;
mod refresh_loaded_identities_dpns_names;
mod register_dpns_name;
mod register_identity;
mod top_up_identity;
mod transfer;
mod withdraw_from_identity;

use super::{BackendTaskSuccessResult, FeeResult, TaskError};
use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, WalletDerivationPath};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use crate::model::secret::Secret;
use crate::model::wallet::{Wallet, WalletArcRef, WalletSeedHash};
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey, TxOut};
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{OutPoint, Transaction};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::{Identifier, Identity, IdentityPublicKey};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq)]
pub struct IdentityInputToLoad {
    pub identity_id_input: String,
    pub identity_type: IdentityType,
    pub alias_input: String,
    pub voting_private_key_input: Secret,
    pub owner_private_key_input: Secret,
    pub payout_address_private_key_input: Secret,
    pub keys_input: Vec<Secret>,
    pub derive_keys_from_wallets: bool,
    pub selected_wallet_seed_hash: Option<WalletSeedHash>,
}

/// A key input tuple containing the private key with derivation path, key type, purpose,
/// security level, and optional contract bounds.
pub type KeyInput = (
    (PrivateKey, DerivationPath),
    KeyType,
    Purpose,
    SecurityLevel,
    Option<ContractBounds>,
);

#[derive(Debug, Clone, PartialEq)]
pub struct IdentityKeys {
    pub(crate) master_private_key: Option<(PrivateKey, DerivationPath)>,
    pub(crate) master_private_key_type: KeyType,
    pub(crate) keys_input: Vec<KeyInput>,
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
            |(
                i,
                (
                    (private_key, derivation_path),
                    key_type,
                    purpose,
                    security_level,
                    contract_bounds,
                ),
            )| {
                let id = (i + 1) as KeyID;
                let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                    id,
                    purpose: *purpose,
                    security_level: *security_level,
                    contract_bounds: contract_bounds.clone(),
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
    pub fn to_public_keys_map(&self) -> Result<BTreeMap<KeyID, IdentityPublicKey>, String> {
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
                other => {
                    return Err(format!(
                        "Unsupported master key type: {:?}. Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
                        other
                    ));
                }
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
        for (i, ((private_key, _), key_type, purpose, security_level, contract_bounds)) in
            keys_input.iter().enumerate()
        {
            let id = (i + 1) as KeyID;

            // Validate security level matches key purpose (defense-in-depth)
            match purpose {
                Purpose::TRANSFER => {
                    if *security_level != SecurityLevel::CRITICAL {
                        return Err(format!(
                            "Key {}: TRANSFER purpose requires CRITICAL security level, got {:?}",
                            id, security_level
                        ));
                    }
                }
                Purpose::ENCRYPTION | Purpose::DECRYPTION => {
                    if *security_level != SecurityLevel::MEDIUM {
                        return Err(format!(
                            "Key {}: {:?} purpose requires MEDIUM security level, got {:?}",
                            id, purpose, security_level
                        ));
                    }
                }
                Purpose::AUTHENTICATION => {
                    if *security_level != SecurityLevel::CRITICAL
                        && *security_level != SecurityLevel::HIGH
                        && *security_level != SecurityLevel::MEDIUM
                    {
                        return Err(format!(
                            "Key {}: AUTHENTICATION purpose requires CRITICAL, HIGH, or MEDIUM security level, got {:?}",
                            id, security_level
                        ));
                    }
                }
                _ => {}
            }

            let data = match key_type {
                KeyType::ECDSA_SECP256K1 => private_key.public_key(&secp).to_bytes().into(),
                KeyType::ECDSA_HASH160 => private_key
                    .public_key(&secp)
                    .pubkey_hash()
                    .to_byte_array()
                    .to_vec()
                    .into(),
                other => {
                    return Err(format!(
                        "Unsupported key type for key {}: {:?}. Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
                        id, other
                    ));
                }
            };
            let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id,
                purpose: *purpose,
                security_level: *security_level,
                contract_bounds: contract_bounds.clone(),
                key_type: *key_type,
                read_only: false,
                data,
                disabled_at: None,
            });
            key_map.insert(id, identity_public_key);
        }

        Ok(key_map)
    }
}

pub type IdentityIndex = u32;
pub type TopUpIndex = u32;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterIdentityFundingMethod {
    UseAssetLock(Address, Box<AssetLockProof>, Box<Transaction>),
    FundWithUtxo(OutPoint, TxOut, Address, IdentityIndex),
    FundWithWallet(Duffs, IdentityIndex),
    /// Fund identity creation from Platform addresses
    FundWithPlatformAddresses {
        /// Platform addresses and credits to use
        inputs: BTreeMap<dash_sdk::dpp::address_funds::PlatformAddress, Credits>,
        /// Wallet seed hash for signing
        wallet_seed_hash: WalletSeedHash,
    },
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
pub enum IdentityTask {
    LoadIdentity(IdentityInputToLoad),
    #[allow(dead_code)] // May be used for finding identities in wallets
    SearchIdentityFromWallet(WalletArcRef, IdentityIndex),
    SearchIdentitiesUpToIndex(WalletArcRef, IdentityIndex),
    /// Search for an identity by its DPNS name (without .dash suffix)
    /// Second parameter is optional wallet seed hash for key derivation
    SearchIdentityByDpnsName(String, Option<WalletSeedHash>),
    RegisterIdentity(IdentityRegistrationInfo),
    TopUpIdentity(IdentityTopUpInfo),
    /// Top up an identity from Platform addresses
    TopUpIdentityFromPlatformAddresses {
        identity: QualifiedIdentity,
        /// Platform addresses and amounts to use for top-up
        inputs: BTreeMap<dash_sdk::dpp::address_funds::PlatformAddress, Credits>,
        /// Wallet seed hash for signing
        wallet_seed_hash: WalletSeedHash,
    },
    AddKeyToIdentity(QualifiedIdentity, QualifiedIdentityPublicKey, [u8; 32]),
    WithdrawFromIdentity(QualifiedIdentity, Option<Address>, Credits, Option<KeyID>),
    Transfer(QualifiedIdentity, Identifier, Credits, Option<KeyID>),
    /// Transfer credits from identity to Platform addresses
    TransferToAddresses {
        identity: QualifiedIdentity,
        /// Platform addresses and amounts to receive credits
        outputs: BTreeMap<dash_sdk::dpp::address_funds::PlatformAddress, Credits>,
        /// Key ID to use for signing (if any)
        key_id: Option<KeyID>,
    },
    RegisterDpnsName(RegisterDpnsNameInput),
    RefreshIdentity(QualifiedIdentity),
    RefreshLoadedIdentitiesOwnedDPNSNames,
}

fn verify_key_input(
    untrimmed_private_key: Secret,
    type_key: &str,
) -> Result<Option<[u8; 32]>, String> {
    let private_key = untrimmed_private_key.expose_secret().trim();
    match private_key.len() {
        64 => {
            // hex
            match hex::decode(private_key) {
                Ok(decoded) => Ok(Some(decoded.try_into().unwrap())),
                Err(_) => Err(format!(
                    "{} key is the size of a hex key but isn't hex",
                    type_key
                )),
            }
        }
        51 | 52 => {
            // wif
            match PrivateKey::from_wif(private_key) {
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
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            IdentityTask::LoadIdentity(input) => Ok(self.load_identity(sdk, input).await?),
            IdentityTask::WithdrawFromIdentity(qualified_identity, to_address, credits, id) => {
                Ok(self
                    .withdraw_from_identity(qualified_identity, to_address, credits, id)
                    .await?)
            }
            IdentityTask::AddKeyToIdentity(qualified_identity, public_key_to_add, private_key) => {
                self.add_key_to_identity(sdk, qualified_identity, public_key_to_add, private_key)
                    .await
            }
            IdentityTask::RegisterIdentity(registration_info) => {
                Ok(self.register_identity(registration_info).await?)
            }
            IdentityTask::RegisterDpnsName(input) => {
                Ok(self.register_dpns_name(sdk, input).await?)
            }
            IdentityTask::RefreshIdentity(qualified_identity) => {
                self.refresh_identity(sdk, qualified_identity, sender).await
            }
            IdentityTask::Transfer(qualified_identity, to_identifier, credits, id) => Ok(self
                .transfer_to_identity(qualified_identity, to_identifier, credits, id)
                .await?),
            IdentityTask::SearchIdentityFromWallet(wallet, identity_index) => Ok(self
                .load_user_identity_from_wallet(sdk, wallet, identity_index, sender)
                .await?),
            IdentityTask::SearchIdentitiesUpToIndex(wallet, max_identity_index) => Ok(self
                .load_user_identities_up_to_index(sdk, wallet, max_identity_index, sender)
                .await?),
            IdentityTask::SearchIdentityByDpnsName(dpns_name, wallet_seed_hash) => Ok(self
                .load_identity_by_dpns_name(sdk, dpns_name, wallet_seed_hash)
                .await?),
            IdentityTask::TopUpIdentity(top_up_info) => {
                Ok(self.top_up_identity(top_up_info).await?)
            }
            IdentityTask::TopUpIdentityFromPlatformAddresses {
                identity,
                inputs,
                wallet_seed_hash,
            } => {
                self.top_up_identity_from_platform_addresses(
                    sdk,
                    identity,
                    inputs,
                    wallet_seed_hash,
                )
                .await
            }
            IdentityTask::TransferToAddresses {
                identity,
                outputs,
                key_id,
            } => {
                self.transfer_to_addresses(sdk, identity, outputs, key_id)
                    .await
            }
            IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames => {
                Ok(self.refresh_loaded_identities_dpns_names(sender).await?)
            }
        }
    }

    /// Top up an identity using credits from Platform addresses
    async fn top_up_identity_from_platform_addresses(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        inputs: BTreeMap<dash_sdk::dpp::address_funds::PlatformAddress, Credits>,
        wallet_seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use crate::model::fee_estimation::PlatformFeeEstimator;
        use dash_sdk::platform::transition::top_up_identity_from_addresses::TopUpIdentityFromAddresses;

        // Estimate fee for top-up from platform addresses
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_topup();

        tracing::info!(
            "top_up_identity_from_platform_addresses: identity={}, inputs={:?}",
            qualified_identity.identity.id(),
            inputs
        );

        // Get the wallet for signing - clone it to avoid holding guard across await
        let wallet_clone = {
            let wallet = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&wallet_seed_hash)
                    .cloned()
                    .ok_or_else(|| TaskError::Generic("Wallet not found".into()))?
            };

            // TODO: Replace Generic with a dedicated TaskError::LockPoisoned variant
            //       that preserves the PoisonError as #[source] with a user-friendly Display.
            let wallet_guard = wallet
                .read()
                .map_err(|e| TaskError::Generic(e.to_string()))?;

            // Ensure wallet is open
            if !wallet_guard.is_open() {
                return Err(TaskError::Generic(
                    "Wallet must be unlocked to sign Platform transactions".into(),
                ));
            }

            wallet_guard.clone()
        };

        tracing::info!("Wallet loaded and open, calling top_up_from_addresses...");

        // Get the identity
        let identity = qualified_identity.identity.clone();

        // Execute the top-up
        let (address_infos, new_balance) = identity
            .top_up_from_addresses(sdk, inputs, &wallet_clone, None)
            .await?;

        tracing::info!(
            "top_up_from_addresses succeeded, new_balance={}",
            new_balance
        );

        // Update source address balances using proof-verified data from SDK response
        if let Err(e) =
            self.update_wallet_platform_address_info_from_sdk(wallet_seed_hash, &address_infos)
        {
            tracing::warn!("Failed to update wallet platform address info: {}", e);
        }

        // Update the identity balance in memory
        let mut updated_identity = qualified_identity.clone();
        updated_identity.identity.set_balance(new_balance);

        // Store the updated identity (use update to preserve wallet association)
        self.update_local_qualified_identity(&updated_identity)
            .map_err(|e| TaskError::IdentitySaveError { source: e })?;

        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::ToppedUpIdentity(
            updated_identity,
            fee_result,
        ))
    }

    /// Transfer credits from an identity to Platform addresses
    async fn transfer_to_addresses(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        outputs: BTreeMap<dash_sdk::dpp::address_funds::PlatformAddress, Credits>,
        key_id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use crate::model::fee_estimation::PlatformFeeEstimator;
        use dash_sdk::platform::transition::transfer_to_addresses::TransferToAddresses;

        // Get the identity
        let identity = qualified_identity.identity.clone();

        // Get the signing key if specified
        let signing_key = key_id.and_then(|id| identity.get_public_key_by_id(id));

        // Track balance before transfer for fee calculation
        let balance_before = identity.balance();
        let fee_estimator = PlatformFeeEstimator::new();
        let estimated_fee = fee_estimator.estimate_credit_transfer_to_addresses(outputs.len());

        // Execute the transfer - qualified_identity is consumed here as the signer
        let (address_infos, new_balance) = identity
            .transfer_credits_to_addresses(
                sdk,
                outputs.clone(),
                signing_key,
                &qualified_identity,
                None,
            )
            .await?;

        // Update destination address balances in any wallets that contain them
        // (using proof-verified data from the SDK response)
        {
            let wallets = self.wallets.read().unwrap();
            for (seed_hash, wallet_arc) in wallets.iter() {
                if let Err(e) =
                    self.update_wallet_platform_address_info_from_sdk(*seed_hash, &address_infos)
                {
                    tracing::warn!("Failed to update wallet platform address info: {}", e);
                }
                // Break early since all wallets share the same network addresses
                let _ = wallet_arc; // silence unused warning
            }
        }

        // Update the identity balance in memory
        let mut updated_identity = qualified_identity;
        updated_identity.identity.set_balance(new_balance);

        // Calculate actual fee
        let total_outputs: Credits = outputs.values().sum();
        let actual_fee = balance_before
            .saturating_sub(new_balance)
            .saturating_sub(total_outputs);

        tracing::info!(
            "Credit transfer to addresses complete: estimated fee {} credits, actual fee {} credits",
            estimated_fee,
            actual_fee
        );
        if actual_fee != estimated_fee {
            tracing::warn!(
                "Fee mismatch: estimated {} vs actual {} (diff: {})",
                estimated_fee,
                actual_fee,
                actual_fee as i64 - estimated_fee as i64
            );
        }

        // Store the updated identity (use update to preserve wallet association)
        self.update_local_qualified_identity(&updated_identity)
            .map_err(|e| TaskError::IdentitySaveError { source: e })?;

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::TransferredCredits(fee_result))
    }
}
