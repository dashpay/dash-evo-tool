mod add_key_to_identity;
mod auth_pubkey_resolve;
mod discover_identities;
mod load_identity;
mod load_identity_by_dpns_name;
mod load_identity_from_wallet;
mod protect_identity_keys;
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
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey};
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Network, OutPoint, PublicKey};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
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

/// One chosen identity key, public-only.
///
/// Carries the derived **public** key plus the wallet derivation path it came
/// from. The matching private key is never held here — it is materialized
/// just-in-time at signing time from `derivation_path` through the JIT
/// chokepoint (`QualifiedIdentity` signer + `get_resolve_with_seed`). The
/// public key and the path are derived from the same
/// `(network, identity_index, key_index)`, so the key submitted to Platform
/// and the key that signs are the two faces of one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityKeyEntry {
    pub public_key: PublicKey,
    pub derivation_path: DerivationPath,
    pub key_type: KeyType,
    pub purpose: Purpose,
    pub security_level: SecurityLevel,
    pub contract_bounds: Option<ContractBounds>,
}

impl IdentityKeyEntry {
    /// Build an entry from a cached public key, deriving the matching path
    /// from the **same** `(network, identity_index, key_index)`.
    ///
    /// RK-1 single-constructor rule: the public key and the path always come
    /// from one coordinate, so the key submitted to Platform and the key the
    /// chokepoint signs with at registration are byte-for-byte the same key.
    #[allow(clippy::too_many_arguments)]
    pub fn from_cached_public_key(
        public_key: PublicKey,
        network: Network,
        identity_index: u32,
        key_index: u32,
        key_type: KeyType,
        purpose: Purpose,
        security_level: SecurityLevel,
        contract_bounds: Option<ContractBounds>,
    ) -> Self {
        let derivation_path = DerivationPath::identity_authentication_path(
            network,
            KeyDerivationType::ECDSA,
            identity_index,
            key_index,
        );
        Self {
            public_key,
            derivation_path,
            key_type,
            purpose,
            security_level,
            contract_bounds,
        }
    }

    /// Build an entry by deriving the public key from a borrowed HD seed.
    ///
    /// Seed-param form used by the seed-driven registration prep (production
    /// `build_identity_registration` and the e2e helper). The public key is
    /// derived from the same coordinate as the path, satisfying the RK-1
    /// single-constructor rule. The seed is borrowed for the derivation only
    /// and never retained.
    #[allow(clippy::too_many_arguments)]
    pub fn from_seed(
        wallet: &Wallet,
        seed: &[u8; 64],
        network: Network,
        identity_index: u32,
        key_index: u32,
        key_type: KeyType,
        purpose: Purpose,
        security_level: SecurityLevel,
        contract_bounds: Option<ContractBounds>,
    ) -> Result<Self, TaskError> {
        let public_key = wallet
            .identity_authentication_ecdsa_public_key_from_seed(
                seed,
                network,
                identity_index,
                key_index,
            )
            .map_err(|e| TaskError::WalletKeyDerivationFailed { source: e.into() })?;
        Ok(Self::from_cached_public_key(
            public_key,
            network,
            identity_index,
            key_index,
            key_type,
            purpose,
            security_level,
            contract_bounds,
        ))
    }
}

/// The chosen key set for a new identity, public-only.
///
/// Holds public material + derivation paths (key *specs*), not private keys:
/// the chooser reads public keys from the
/// [`AuthPubkeyCache`](crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache)
/// and signing derives the private keys just-in-time at registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityKeySpecs {
    /// The master AUTHENTICATION/MASTER key, when present. `None` until the
    /// chooser has populated keys (a cold auth-pubkey cache, before warming).
    pub(crate) master: Option<IdentityKeyEntry>,
    /// The additional non-master keys, in id order (id = index + 1).
    pub(crate) others: Vec<IdentityKeyEntry>,
}

impl IdentityKeySpecs {
    pub fn new(master: Option<IdentityKeyEntry>, others: Vec<IdentityKeyEntry>) -> Self {
        Self { master, others }
    }

    /// An empty, not-yet-populated key set (cold auth-pubkey cache).
    pub fn empty() -> Self {
        Self {
            master: None,
            others: Vec::new(),
        }
    }

    /// Whether the master key has been populated. Registration is gated on
    /// this — a cold cache leaves it `None` and the create button stays
    /// disabled (fail-closed).
    pub fn has_master(&self) -> bool {
        self.master.is_some()
    }

    fn key_data(entry: &IdentityKeyEntry) -> dash_sdk::dpp::platform_value::BinaryData {
        match entry.key_type {
            KeyType::ECDSA_HASH160 => entry
                .public_key
                .pubkey_hash()
                .to_byte_array()
                .to_vec()
                .into(),
            _ => entry.public_key.to_bytes().into(),
        }
    }

    pub fn to_key_storage(&self, wallet_seed_hash: WalletSeedHash) -> KeyStorage {
        let mut key_map = BTreeMap::new();

        if let Some(master) = &self.master {
            let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: 0,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::MASTER,
                contract_bounds: None,
                key_type: master.key_type,
                read_only: false,
                data: Self::key_data(master),
                disabled_at: None,
            });

            let wallet_derivation_path = WalletDerivationPath {
                wallet_seed_hash,
                derivation_path: master.derivation_path.clone(),
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

        key_map.extend(self.others.iter().enumerate().map(|(i, entry)| {
            let id = (i + 1) as KeyID;
            let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id,
                purpose: entry.purpose,
                security_level: entry.security_level,
                contract_bounds: entry.contract_bounds.clone(),
                key_type: entry.key_type,
                read_only: false,
                data: Self::key_data(entry),
                disabled_at: None,
            });

            let wallet_derivation_path = WalletDerivationPath {
                wallet_seed_hash,
                derivation_path: entry.derivation_path.clone(),
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
        }));

        key_map.into()
    }
    pub fn to_public_keys_map(&self) -> Result<BTreeMap<KeyID, IdentityPublicKey>, String> {
        let mut key_map = BTreeMap::new();
        if let Some(master) = &self.master {
            match master.key_type {
                KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {}
                other => {
                    return Err(format!(
                        "Unsupported master key type: {:?}. Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
                        other
                    ));
                }
            }
            let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: 0,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::MASTER,
                contract_bounds: None,
                key_type: master.key_type,
                read_only: false,
                data: Self::key_data(master),
                disabled_at: None,
            });

            key_map.insert(0, key);
        }
        for (i, entry) in self.others.iter().enumerate() {
            let id = (i + 1) as KeyID;
            let key_type = &entry.key_type;
            let purpose = &entry.purpose;
            let security_level = &entry.security_level;

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

            match key_type {
                KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {}
                other => {
                    return Err(format!(
                        "Unsupported key type for key {}: {:?}. Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
                        id, other
                    ));
                }
            }
            let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id,
                purpose: *purpose,
                security_level: *security_level,
                contract_bounds: entry.contract_bounds.clone(),
                key_type: *key_type,
                read_only: false,
                data: Self::key_data(entry),
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
    /// Resume from an asset lock already tracked by the upstream
    /// `AssetLockManager` (identified by its credit-output outpoint).
    /// Upstream re-derives the credit-output key from the seed and
    /// drives the IS→CL fallback + identity-create submission.
    UseAssetLock {
        out_point: OutPoint,
        identity_index: IdentityIndex,
    },
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
    /// Resume from an asset lock already tracked by the upstream
    /// `AssetLockManager` (identified by its credit-output outpoint).
    UseAssetLock {
        out_point: OutPoint,
        identity_index: IdentityIndex,
        top_up_index: TopUpIndex,
    },
    FundWithWallet(Duffs, IdentityIndex, TopUpIndex),
}

#[derive(Debug, Clone)]
pub struct IdentityRegistrationInfo {
    pub alias_input: String,
    pub keys: IdentityKeySpecs,
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
    /// Opt-in: seal every keyless (Tier-1) vault-stored key of this
    /// identity under ONE per-identity object `password` (Tier-2), and store
    /// `hint` for the sign-time prompt copy. Idempotent (an already-protected
    /// key is skipped) and crash-safe (same-label in-place upsert, vault before
    /// sidecar). After this, signing with this identity prompts for the
    /// password; headless/MCP signing yields `SecretPromptUnavailable`.
    ProtectIdentityKeys {
        /// The identity whose keys to protect.
        identity_id: Identifier,
        /// The per-identity password the user chose. Isolated per secret —
        /// never the wallet's object password.
        password: Secret,
        /// Optional user-set hint shown next to the sign-time prompt.
        hint: Option<String>,
    },
    /// Opt-out: revert every password-protected (Tier-2) vault-stored
    /// key of this identity back to keyless (Tier-1), after verifying
    /// `password`. Idempotent (an already-keyless key is skipped) and crash-safe
    /// (vault downgrade before sidecar delete). After this, signing is
    /// prompt-free again, including headless/MCP.
    UnprotectIdentityKeys {
        /// The identity whose key protection to remove.
        identity_id: Identifier,
        /// The current per-identity password, verified before downgrading.
        password: Secret,
    },
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

/// Build an [`IdentityRegistrationInfo`] for a wallet-funded identity from a
/// borrowed HD seed.
///
/// Derives the **public** master key and additional keys from `seed` at the
/// given `identity_index`. Seed-param form: the caller resolves the seed once
/// through the JIT chokepoint and passes it by borrow — the private keys are
/// never materialized here (signing derives them JIT at registration). The
/// public key and the wallet derivation path of each entry come from the same
/// coordinate (RK-1), so the keys submitted to Platform and the keys the
/// chokepoint signs with are identical. This is the canonical way to prepare
/// identity registration data — used by the async UI/registration path and by
/// tests.
pub fn build_identity_registration_with_seed(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    seed: &[u8; 64],
    identity_index: u32,
    funding_amount: Duffs,
) -> Result<IdentityRegistrationInfo, TaskError> {
    let dashpay_contract_id = app_context.dashpay_contract.id();
    let key_specs = default_identity_key_specs(dashpay_contract_id);
    let network = app_context.network;

    let wallet = wallet_arc.read()?;

    let master = IdentityKeyEntry::from_seed(
        &wallet,
        seed,
        network,
        identity_index,
        0,
        KeyType::ECDSA_HASH160,
        Purpose::AUTHENTICATION,
        SecurityLevel::MASTER,
        None,
    )?;

    let mut others: Vec<IdentityKeyEntry> = Vec::with_capacity(key_specs.len());
    for (i, (key_type, purpose, security_level, contract_bounds)) in
        key_specs.into_iter().enumerate()
    {
        let key_index = (i + 1) as u32;
        others.push(IdentityKeyEntry::from_seed(
            &wallet,
            seed,
            network,
            identity_index,
            key_index,
            key_type,
            purpose,
            security_level,
            contract_bounds,
        )?);
    }

    drop(wallet);

    Ok(IdentityRegistrationInfo {
        alias_input: String::new(),
        keys: IdentityKeySpecs::new(Some(master), others),
        wallet: wallet_arc.clone(),
        wallet_identity_index: identity_index,
        identity_funding_method: RegisterIdentityFundingMethod::FundWithWallet(
            funding_amount,
            identity_index,
        ),
    })
}

/// Async wrapper over [`build_identity_registration_with_seed`].
///
/// Resolves the wallet's HD seed once through the JIT chokepoint and delegates;
/// callers that can `await` use this and never read the wallet's parked seed.
#[allow(dead_code)] // Used by backend-e2e tests
pub async fn build_identity_registration(
    app_context: &Arc<AppContext>,
    wallet_arc: &Arc<RwLock<Wallet>>,
    identity_index: u32,
    funding_amount: Duffs,
) -> Result<IdentityRegistrationInfo, TaskError> {
    let seed_hash = wallet_arc.read()?.seed_hash();
    let backend = app_context.wallet_backend()?;
    backend
        .secret_access()
        .with_secret(
            &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
            |plaintext| {
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                build_identity_registration_with_seed(
                    app_context,
                    wallet_arc,
                    seed,
                    identity_index,
                    funding_amount,
                )
            },
        )
        .await
}

/// Failure while checking that an identity carries a public key matching a
/// user-supplied private key.
#[derive(Debug, thiserror::Error)]
pub enum KeyVerificationError {
    /// The identity has no keys of the required purpose.
    #[error("This identity does not contain any {purpose} keys.")]
    NoKeysForPurpose { purpose: &'static str },

    /// No public key on the identity matches the supplied private key.
    #[error("This identity has no {purpose} public key matching this private key.")]
    NoMatchingKey { purpose: &'static str },

    /// The public key could not be derived from the supplied private key.
    #[error("The public key could not be derived from the supplied private key.")]
    PublicKeyDerivation(#[source] Box<ProtocolError>),
}

impl AppContext {
    fn verify_voting_key_exists_on_identity(
        &self,
        voting_identity: &Identity,
        private_voting_key: &[u8; 32],
    ) -> Result<IdentityPublicKey, KeyVerificationError> {
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
            return Err(KeyVerificationError::NoKeysForPurpose { purpose: "voting" });
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
            .map_err(|e| KeyVerificationError::PublicKeyDerivation(Box::new(e)))?;
        let Some(key) = voting_keys.into_iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(KeyVerificationError::NoMatchingKey { purpose: "voting" });
        };
        Ok(key)
    }

    fn verify_owner_key_exists_on_identity(
        &self,
        identity: &Identity,
        private_voting_key: &[u8; 32],
    ) -> Result<IdentityPublicKey, KeyVerificationError> {
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
            return Err(KeyVerificationError::NoKeysForPurpose { purpose: "owner" });
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
            .map_err(|e| KeyVerificationError::PublicKeyDerivation(Box::new(e)))?;
        let Some(key) = owner_keys.into_iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(KeyVerificationError::NoMatchingKey { purpose: "owner" });
        };
        Ok(key)
    }

    fn verify_payout_address_key_exists_on_identity(
        &self,
        identity: &Identity,
        private_voting_key: &[u8; 32],
    ) -> Result<IdentityPublicKey, KeyVerificationError> {
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
            return Err(KeyVerificationError::NoKeysForPurpose {
                purpose: "payout address",
            });
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
            .map_err(|e| KeyVerificationError::PublicKeyDerivation(Box::new(e)))?;
        let Some(key) = owner_keys.into_iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(KeyVerificationError::NoMatchingKey {
                purpose: "payout address",
            });
        };
        Ok(key)
    }

    pub async fn run_identity_task(
        self: &Arc<Self>,
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
                .load_user_identities_up_to_index(wallet, max_identity_index, sender)
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
            IdentityTask::ProtectIdentityKeys {
                identity_id,
                password,
                hint,
            } => self.protect_identity_keys(identity_id, password, hint),
            IdentityTask::UnprotectIdentityKeys {
                identity_id,
                password,
            } => self.unprotect_identity_keys(identity_id, password),
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
        use dash_sdk::platform::transition::top_up_identity_from_addresses::TopUpIdentityFromAddresses;

        // Estimate the top-up fee with the active network fee multiplier
        // (context estimator) so the figure shown to the user is accurate.
        let estimated_fee = self.fee_estimator().estimate_identity_topup();

        tracing::info!(
            "top_up_identity_from_platform_addresses: identity={}, inputs={:?}",
            qualified_identity.identity.id(),
            inputs
        );

        // Clone the wallet for the pure address→path index (needed across the
        // async boundary). The signing key never lives in this snapshot — it is
        // derived JIT from the borrowed HD seed inside the secret scope below.
        // No `is_open()` gate: the chokepoint resolves an unprotected or
        // session-cached wallet without a prompt, and prompts a locked protected
        // one — returning `WalletLocked` only if the seed is truly unavailable.
        let wallet_snapshot = {
            let wallet = {
                let wallets = self.wallets.read()?;
                wallets
                    .get(&wallet_seed_hash)
                    .cloned()
                    .ok_or(TaskError::WalletNotFound)?
            };
            let wallet_guard = wallet.read()?;
            wallet_guard.clone()
        };

        tracing::info!("Wallet loaded, calling top_up_from_addresses...");

        // Get the identity
        let identity = qualified_identity.identity.clone();

        // Sign each top-up input through a JIT platform signer that borrows the
        // HD seed only for the duration of the SDK call. The pure path index is
        // built before the secret scope; the seed zeroizes on return and never
        // enters this layer by value.
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex, SecretScope};
        let network = self.network;
        let path_index = PlatformPathIndex::from_wallet(&wallet_snapshot, network);
        let backend = self.wallet_backend()?;

        // Execute the top-up
        let (address_infos, new_balance, _height) = backend
            .secret_access()
            .with_secret_session(
                &SecretScope::HdSeed {
                    seed_hash: wallet_seed_hash,
                },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    identity
                        .top_up_from_addresses(sdk, inputs, &signer, None)
                        .await
                        .map_err(TaskError::from)
                },
            )
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
        self.update_local_qualified_identity(&updated_identity)?;

        let fee_result = FeeResult::estimated_only(estimated_fee);
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
        use dash_sdk::platform::transition::transfer_to_addresses::TransferToAddresses;

        // Get the identity
        let identity = qualified_identity.identity.clone();

        // Get the signing key if specified
        let signing_key = key_id.and_then(|id| identity.get_public_key_by_id(id));

        // Track balance before transfer for fee calculation
        let balance_before = identity.balance();
        let fee_estimator = self.fee_estimator();
        let estimated_fee = fee_estimator.estimate_credit_transfer_to_addresses(outputs.len());

        // Execute the transfer - qualified_identity is consumed here as the signer
        let (address_infos, new_balance, _height) = identity
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
            let wallets = self.wallets.read()?;
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
        self.update_local_qualified_identity(&updated_identity)?;

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

    /// RK-1 / no-drift: the public key the chooser shows (from the cache,
    /// reconstructed via `from_cached_public_key`) and the private key the
    /// chokepoint derives at registration (the path's BIP-32 private
    /// derivation) are the two faces of one `(network, identity_index,
    /// key_index)` coordinate. If these ever diverge, the key submitted to
    /// Platform would not match the key that signs — identity-create would be
    /// rejected or the wallet could not sign for its own identity.
    #[test]
    fn identity_key_entry_public_matches_private_derivation() {
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;

        let seed = [0x42u8; 64];
        let secp = Secp256k1::new();

        for network in [Network::Testnet, Network::Mainnet] {
            let wallet = Wallet::new_from_seed(seed, network, None, None).expect("build wallet");

            for identity_index in [0u32, 7] {
                for key_index in 0u32..6 {
                    // The public key the chooser caches/shows.
                    let cached_pk = wallet
                        .identity_authentication_ecdsa_public_key_from_seed(
                            &seed,
                            network,
                            identity_index,
                            key_index,
                        )
                        .expect("public derive");

                    // The entry the chooser builds from that cached key.
                    let entry = IdentityKeyEntry::from_cached_public_key(
                        cached_pk,
                        network,
                        identity_index,
                        key_index,
                        KeyType::ECDSA_HASH160,
                        Purpose::AUTHENTICATION,
                        SecurityLevel::HIGH,
                        None,
                    );

                    // The private key the chokepoint derives at the entry's
                    // path at signing time.
                    let private_key = entry
                        .derivation_path
                        .derive_priv_ecdsa_for_master_seed(&seed, network)
                        .expect("private derive")
                        .to_priv();

                    assert_eq!(
                        entry.public_key.inner.serialize(),
                        private_key.public_key(&secp).inner.serialize(),
                        "public/private key drift at ({network:?}, id={identity_index}, key={key_index})"
                    );

                    // The seed-param entry constructor must produce the same
                    // entry (same public key + same path).
                    let from_seed_entry = IdentityKeyEntry::from_seed(
                        &wallet,
                        &seed,
                        network,
                        identity_index,
                        key_index,
                        KeyType::ECDSA_HASH160,
                        Purpose::AUTHENTICATION,
                        SecurityLevel::HIGH,
                        None,
                    )
                    .expect("from_seed");
                    assert_eq!(
                        entry, from_seed_entry,
                        "from_cached_public_key and from_seed diverged at ({network:?}, id={identity_index}, key={key_index})"
                    );
                }
            }
        }
    }

    /// The reshaped `to_public_keys_map` / `to_key_storage` produce the same
    /// public key data and stored derivation paths the registration flow
    /// expects, with no private key held anywhere in the spec set.
    #[test]
    fn identity_key_specs_roundtrip_public_and_paths() {
        let seed = [0x11u8; 64];
        let network = Network::Testnet;
        let wallet = Wallet::new_from_seed(seed, network, None, None).expect("build wallet");
        let seed_hash = wallet.seed_hash();

        let master = IdentityKeyEntry::from_seed(
            &wallet,
            &seed,
            network,
            0,
            0,
            KeyType::ECDSA_HASH160,
            Purpose::AUTHENTICATION,
            SecurityLevel::MASTER,
            None,
        )
        .expect("master");
        let other = IdentityKeyEntry::from_seed(
            &wallet,
            &seed,
            network,
            0,
            1,
            KeyType::ECDSA_SECP256K1,
            Purpose::AUTHENTICATION,
            SecurityLevel::HIGH,
            None,
        )
        .expect("other");

        let specs = IdentityKeySpecs::new(Some(master.clone()), vec![other.clone()]);

        // Public keys map: id 0 (master) + id 1 (other), both present.
        let public_map = specs.to_public_keys_map().expect("public map");
        assert_eq!(public_map.len(), 2);
        assert!(public_map.contains_key(&0));
        assert!(public_map.contains_key(&1));

        // Key storage carries the entries' derivation paths verbatim, keyed by
        // the same wallet seed hash the chokepoint resolves from. Resolve via
        // the seed-param path (the JIT chokepoint's resolver) with a borrowed
        // seed — the legacy parked-seed `get_resolve` is gone.
        let storage = specs.to_key_storage(seed_hash);
        let stored = storage
            .get_resolve_with_seed(
                &(PrivateKeyTarget::PrivateKeyOnMainIdentity, 0),
                std::slice::from_ref(&std::sync::Arc::new(std::sync::RwLock::new(
                    Wallet::new_from_seed(seed, network, None, None).expect("wallet"),
                ))),
                &seed,
                network,
            )
            .expect("resolve master")
            .expect("master present");
        // The resolved private key's public key must match the spec's public key.
        let secp = dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new();
        let resolved_pub = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(&stored.1[..])
            .expect("secret")
            .public_key(&secp);
        assert_eq!(
            master.public_key.inner.serialize(),
            resolved_pub.serialize(),
            "stored path resolves to the spec's public key"
        );
    }
}
