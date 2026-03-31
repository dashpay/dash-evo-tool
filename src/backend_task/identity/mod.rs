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
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
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
    pub fn new(
        master_private_key: Option<(PrivateKey, DerivationPath)>,
        master_private_key_type: KeyType,
        keys_input: Vec<KeyInput>,
    ) -> Self {
        Self {
            master_private_key,
            master_private_key_type,
            keys_input,
        }
    }

    pub fn to_key_storage(&self, wallet_seed_hash: WalletSeedHash) -> KeyStorage {
        let Self {
            master_private_key,
            master_private_key_type,
            keys_input,
        } = self;
        let secp = Secp256k1::new();
        let mut key_map = BTreeMap::new();

        if let Some((master_private_key, master_private_key_derivation_path)) = master_private_key {
            let data = match master_private_key_type {
                KeyType::ECDSA_HASH160 => master_private_key
                    .public_key(&secp)
                    .pubkey_hash()
                    .to_byte_array()
                    .to_vec()
                    .into(),
                _ => master_private_key.public_key(&secp).to_bytes().into(),
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
                let data = match key_type {
                    KeyType::ECDSA_HASH160 => private_key
                        .public_key(&secp)
                        .pubkey_hash()
                        .to_byte_array()
                        .to_vec()
                        .into(),
                    _ => private_key.public_key(&secp).to_bytes().into(),
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

/// Returns the default key specifications for a new identity.
///
/// The returned vector contains tuples of (KeyType, Purpose, SecurityLevel, Option<ContractBounds>):
/// - AUTHENTICATION CRITICAL: General platform operations (actions should require PIN)
/// - AUTHENTICATION HIGH: General platform operations
/// - TRANSFER CRITICAL: Credit transfers
/// - ENCRYPTION MEDIUM with DashPay contactRequest bounds: For contact requests per DIP-15
/// - DECRYPTION MEDIUM with DashPay contactRequest bounds: For contact requests per DIP-15
///
/// Note: ENCRYPTION and DECRYPTION keys must use `SingleContractDocumentType` with "contactRequest"
/// document type, not just `SingleContract`. The platform requires encryption key bounds to specify
/// the exact document type for proper validation.
pub fn default_identity_key_specs(
    dashpay_contract_id: Identifier,
) -> Vec<(KeyType, Purpose, SecurityLevel, Option<ContractBounds>)> {
    let dashpay_bounds = Some(ContractBounds::SingleContractDocumentType {
        id: dashpay_contract_id,
        document_type_name: "contactRequest".to_string(),
    });

    vec![
        (
            KeyType::ECDSA_HASH160,
            Purpose::AUTHENTICATION,
            SecurityLevel::CRITICAL,
            None,
        ),
        (
            KeyType::ECDSA_HASH160,
            Purpose::AUTHENTICATION,
            SecurityLevel::HIGH,
            None,
        ),
        (
            KeyType::ECDSA_HASH160,
            Purpose::TRANSFER,
            SecurityLevel::CRITICAL,
            None,
        ),
        (
            KeyType::ECDSA_SECP256K1, // ECDH requires secp256k1
            Purpose::ENCRYPTION,
            SecurityLevel::MEDIUM, // Platform enforces MEDIUM for ENCRYPTION
            dashpay_bounds.clone(),
        ),
        (
            KeyType::ECDSA_SECP256K1, // ECDH requires secp256k1
            Purpose::DECRYPTION,
            SecurityLevel::MEDIUM,
            dashpay_bounds,
        ),
    ]
}

/// Build an [`IdentityRegistrationInfo`] for a wallet-funded identity.
///
/// Derives the master key and additional keys from the wallet at the given
/// `identity_index`. This is the canonical way to prepare identity
/// registration data from a wallet — used by both UI screens and tests.
#[allow(dead_code)] // Used by backend-e2e tests via pub(crate) visibility
pub(crate) fn build_identity_registration(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    identity_index: u32,
    funding_amount: Duffs,
) -> Result<IdentityRegistrationInfo, TaskError> {
    let dashpay_contract_id = app_context.dashpay_contract.id();
    let key_specs = default_identity_key_specs(dashpay_contract_id);

    let mut wallet = wallet_arc.write()?;

    let (master_private_key, master_derivation_path) = wallet
        .identity_authentication_ecdsa_private_key(
            app_context,
            app_context.network,
            identity_index,
            0,
        )
        .map_err(|e| TaskError::WalletKeyDerivationFailed { source: e.into() })?;

    let mut keys_input: Vec<KeyInput> = Vec::new();
    for (i, (key_type, purpose, security_level, contract_bounds)) in
        key_specs.into_iter().enumerate()
    {
        let key_index = (i + 1) as u32;
        let (private_key, derivation_path) = wallet
            .identity_authentication_ecdsa_private_key(
                app_context,
                app_context.network,
                identity_index,
                key_index,
            )
            .map_err(|e| TaskError::WalletKeyDerivationFailed { source: e.into() })?;
        keys_input.push((
            (private_key, derivation_path),
            key_type,
            purpose,
            security_level,
            contract_bounds,
        ));
    }

    drop(wallet);

    Ok(IdentityRegistrationInfo {
        alias_input: String::new(),
        keys: IdentityKeys::new(
            Some((master_private_key, master_derivation_path)),
            KeyType::ECDSA_HASH160,
            keys_input,
        ),
        wallet: wallet_arc.clone(),
        wallet_identity_index: identity_index,
        identity_funding_method: RegisterIdentityFundingMethod::FundWithWallet(
            funding_amount,
            identity_index,
        ),
    })
}

/// Get a receive address string from a wallet.
#[allow(dead_code)] // Used by backend-e2e tests via pub(crate) visibility
pub(crate) fn get_receive_address(
    app_context: &AppContext,
    wallet_arc: &Arc<RwLock<Wallet>>,
) -> Result<String, TaskError> {
    let mut wallet = wallet_arc.write()?;
    wallet
        .receive_address(app_context.network, false, Some(app_context))
        .map(|addr| addr.to_string())
        .map_err(|e| TaskError::WalletAddressDerivationFailed { detail: e })
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

        // Get the platform wallet for signing (PlatformAddressWallet implements Signer<PlatformAddress>)
        let platform_wallet = self.require_platform_wallet(&wallet_seed_hash)?;

        tracing::info!("Wallet loaded and open, calling top_up_from_addresses...");

        // Get the identity
        let identity = qualified_identity.identity.clone();

        // Execute the top-up
        let (address_infos, new_balance) = identity
            .top_up_from_addresses(sdk, inputs, platform_wallet.platform(), None)
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
            .map_err(|e| TaskError::Database { source: e })?;

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
        // (using proof-verified data from the SDK response).
        // Iterate using the platform_wallets bridge map to collect seed hashes,
        // then update via the old wallets map (which still owns the address info).
        {
            let seed_hashes: Vec<WalletSeedHash> = self
                .platform_wallets
                .lock()
                .map(|pw| pw.keys().copied().collect())
                .unwrap_or_default();

            // Fall back to old wallets map if platform_wallets is empty (e.g. locked wallets)
            let seed_hashes = if seed_hashes.is_empty() {
                self.wallets
                    .read()
                    .map(|w| w.keys().copied().collect())
                    .unwrap_or_default()
            } else {
                seed_hashes
            };

            for seed_hash in seed_hashes {
                if let Err(e) =
                    self.update_wallet_platform_address_info_from_sdk(seed_hash, &address_infos)
                {
                    tracing::warn!("Failed to update wallet platform address info: {}", e);
                }
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
            .map_err(|e| TaskError::Database { source: e })?;

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::TransferredCredits(fee_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that the default identity keys include the correct number of keys
    #[test]
    fn test_default_identity_keys_count() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);
        assert_eq!(keys.len(), 5, "Should have 5 default keys");
    }

    /// Test that AUTHENTICATION keys have correct configuration
    #[test]
    fn test_authentication_keys_configuration() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);

        // First key: AUTHENTICATION CRITICAL
        let (key_type, purpose, security_level, contract_bounds) = &keys[0];
        assert_eq!(*key_type, KeyType::ECDSA_HASH160);
        assert_eq!(*purpose, Purpose::AUTHENTICATION);
        assert_eq!(*security_level, SecurityLevel::CRITICAL);
        assert!(
            contract_bounds.is_none(),
            "AUTHENTICATION keys should have no contract bounds"
        );

        // Second key: AUTHENTICATION HIGH
        let (key_type, purpose, security_level, contract_bounds) = &keys[1];
        assert_eq!(*key_type, KeyType::ECDSA_HASH160);
        assert_eq!(*purpose, Purpose::AUTHENTICATION);
        assert_eq!(*security_level, SecurityLevel::HIGH);
        assert!(
            contract_bounds.is_none(),
            "AUTHENTICATION keys should have no contract bounds"
        );
    }

    /// Test that TRANSFER key has correct configuration
    #[test]
    fn test_transfer_key_configuration() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);

        // Third key: TRANSFER CRITICAL
        let (key_type, purpose, security_level, contract_bounds) = &keys[2];
        assert_eq!(*key_type, KeyType::ECDSA_HASH160);
        assert_eq!(*purpose, Purpose::TRANSFER);
        assert_eq!(*security_level, SecurityLevel::CRITICAL);
        assert!(
            contract_bounds.is_none(),
            "TRANSFER keys should have no contract bounds"
        );
    }

    /// Test that ENCRYPTION key uses SingleContractDocumentType with contactRequest
    ///
    /// This is critical for DashPay compatibility - the platform requires encryption keys
    /// to specify the exact document type (contactRequest) not just the contract ID.
    /// Using SingleContract instead of SingleContractDocumentType will cause:
    /// "key bounds expected but not present error: expected encryption key bounds for encryption"
    #[test]
    fn test_encryption_key_uses_single_contract_document_type() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);

        // Fourth key: ENCRYPTION MEDIUM
        let (key_type, purpose, security_level, contract_bounds) = &keys[3];
        assert_eq!(
            *key_type,
            KeyType::ECDSA_SECP256K1,
            "ENCRYPTION key must use ECDSA_SECP256K1 for ECDH"
        );
        assert_eq!(*purpose, Purpose::ENCRYPTION);
        assert_eq!(
            *security_level,
            SecurityLevel::MEDIUM,
            "Platform enforces MEDIUM for ENCRYPTION"
        );

        // Verify contract bounds uses SingleContractDocumentType, NOT SingleContract
        match contract_bounds {
            Some(ContractBounds::SingleContractDocumentType {
                id,
                document_type_name,
            }) => {
                assert_eq!(
                    *id, contract_id,
                    "Contract ID should match DashPay contract"
                );
                assert_eq!(
                    document_type_name, "contactRequest",
                    "Document type must be 'contactRequest' for DashPay"
                );
            }
            Some(ContractBounds::SingleContract { .. }) => {
                panic!(
                    "ENCRYPTION key must use SingleContractDocumentType, not SingleContract. \
                       Using SingleContract causes 'key bounds expected but not present' error."
                );
            }
            None => {
                panic!("ENCRYPTION key must have DashPay contract bounds for contactRequest");
            }
        }
    }

    /// Test that DECRYPTION key uses SingleContractDocumentType with contactRequest
    ///
    /// This is critical for DashPay compatibility - the platform requires decryption keys
    /// to specify the exact document type (contactRequest) not just the contract ID.
    #[test]
    fn test_decryption_key_uses_single_contract_document_type() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);

        // Fifth key: DECRYPTION MEDIUM
        let (key_type, purpose, security_level, contract_bounds) = &keys[4];
        assert_eq!(
            *key_type,
            KeyType::ECDSA_SECP256K1,
            "DECRYPTION key must use ECDSA_SECP256K1 for ECDH"
        );
        assert_eq!(*purpose, Purpose::DECRYPTION);
        assert_eq!(*security_level, SecurityLevel::MEDIUM);

        // Verify contract bounds uses SingleContractDocumentType, NOT SingleContract
        match contract_bounds {
            Some(ContractBounds::SingleContractDocumentType {
                id,
                document_type_name,
            }) => {
                assert_eq!(
                    *id, contract_id,
                    "Contract ID should match DashPay contract"
                );
                assert_eq!(
                    document_type_name, "contactRequest",
                    "Document type must be 'contactRequest' for DashPay"
                );
            }
            Some(ContractBounds::SingleContract { .. }) => {
                panic!(
                    "DECRYPTION key must use SingleContractDocumentType, not SingleContract. \
                       Using SingleContract causes 'key bounds expected but not present' error."
                );
            }
            None => {
                panic!("DECRYPTION key must have DashPay contract bounds for contactRequest");
            }
        }
    }

    /// Test that encryption and decryption keys have matching contract bounds
    #[test]
    fn test_encryption_decryption_keys_have_matching_bounds() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);

        let encryption_bounds = &keys[3].3;
        let decryption_bounds = &keys[4].3;

        assert_eq!(
            encryption_bounds, decryption_bounds,
            "ENCRYPTION and DECRYPTION keys should have identical contract bounds"
        );
    }

    /// Test that the contract ID is correctly propagated to key bounds
    #[test]
    fn test_contract_id_propagation() {
        let contract_id = Identifier::random();
        let keys = default_identity_key_specs(contract_id);

        for (i, (_, purpose, _, contract_bounds)) in keys.iter().enumerate() {
            if (*purpose == Purpose::ENCRYPTION || *purpose == Purpose::DECRYPTION)
                && let Some(ContractBounds::SingleContractDocumentType { id, .. }) = contract_bounds
            {
                assert_eq!(
                    *id, contract_id,
                    "Key {} contract bounds should use the provided contract ID",
                    i
                );
            }
        }
    }
}
