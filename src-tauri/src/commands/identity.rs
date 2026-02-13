//! Identity-related Tauri IPC commands.
//!
//! Maps all IdentityTask variants plus direct database methods to
//! Tauri commands. Long-running operations are dispatched asynchronously
//! via `task_dispatcher::dispatch_task` and results arrive as events.
//! Short reads (DB queries) return directly.

use crate::dto::common::{CreditsDto, IdentifierDto, WalletSeedHashDto};
use crate::dto::identity::{
    ContractBoundsDto, DpnsNameInfoDto, IdentityKeyDto, IdentityStatusDto, IdentitySummaryDto,
    IdentityTypeDto, QualifiedIdentityDto, TopUpEntryDto,
};
use crate::dto::NetworkDto;
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::identity::{
    IdentityInputToLoad, IdentityKeys, IdentityRegistrationInfo, IdentityTask, IdentityTopUpInfo,
    RegisterDpnsNameInput, RegisterIdentityFundingMethod, TopUpIdentityFundingMethod,
};
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::lock_helper::RwLockExt;
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use dash_evo_tool::model::wallet::WalletSeedHash;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs — serializable command parameters from the frontend
// ---------------------------------------------------------------------------

/// Input for loading an existing identity by ID or from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadIdentityInput {
    /// Identity ID as hex or base58 string.
    pub identity_id: String,
    /// Type of identity (User, Masternode, Evonode).
    pub identity_type: IdentityTypeDto,
    /// User alias to assign.
    pub alias: String,
    /// Voting private key (hex or WIF, empty string if none).
    pub voting_private_key: String,
    /// Owner private key (hex or WIF, empty string if none).
    pub owner_private_key: String,
    /// Payout address private key (hex or WIF, empty string if none).
    pub payout_address_private_key: String,
    /// Additional private keys (hex or WIF).
    pub keys: Vec<String>,
    /// Whether to try deriving keys from loaded wallets.
    pub derive_keys_from_wallets: bool,
    /// Optional wallet seed hash to associate with.
    pub selected_wallet_seed_hash: Option<WalletSeedHashDto>,
}

/// Input for searching identity by DPNS name.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchIdentityByDpnsNameInput {
    /// DPNS name without .dash suffix.
    pub name: String,
    /// Optional wallet seed hash for key derivation.
    pub wallet_seed_hash: Option<WalletSeedHashDto>,
}

/// Input for searching identities from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchIdentityFromWalletInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Identity index to search at.
    pub identity_index: u32,
}

/// Input for batch search of identities from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchIdentitiesUpToIndexInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Maximum identity index to search up to.
    pub max_identity_index: u32,
}

/// Input for registering a DPNS name.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDpnsNameCommandInput {
    /// Identity ID (hex) that will own the name.
    pub identity_id: IdentifierDto,
    /// The DPNS name to register (without .dash suffix).
    pub name: String,
}

/// Input for refreshing a single identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefreshIdentityInput {
    /// Identity ID (hex) to refresh.
    pub identity_id: IdentifierDto,
}

/// Input for withdrawing credits from an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawFromIdentityInput {
    /// Identity ID (hex) to withdraw from.
    pub identity_id: IdentifierDto,
    /// Optional destination Core address.
    pub to_address: Option<String>,
    /// Amount to withdraw in credits.
    pub credits: CreditsDto,
    /// Optional key ID to use for signing.
    pub key_id: Option<u32>,
}

/// Input for transferring credits to another identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferCreditsInput {
    /// Source identity ID (hex).
    pub from_identity_id: IdentifierDto,
    /// Destination identity ID (hex).
    pub to_identity_id: IdentifierDto,
    /// Amount to transfer in credits.
    pub credits: CreditsDto,
    /// Optional key ID to use for signing.
    pub key_id: Option<u32>,
}

/// Input for adding a key to an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddKeyToIdentityInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key type (e.g., "ECDSA_SECP256K1", "BLS12_381").
    pub key_type: String,
    /// Key purpose (e.g., "AUTHENTICATION", "VOTING", "TRANSFER").
    pub purpose: String,
    /// Security level (e.g., "HIGH", "MEDIUM").
    pub security_level: String,
    /// Private key as hex string (32 bytes).
    pub private_key_hex: String,
    /// Optional contract bounds for this key.
    pub contract_bounds: Option<ContractBoundsDto>,
}

/// Input for disabling keys on an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DisableKeysInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key IDs to disable.
    pub key_ids: Vec<u32>,
}

/// Input for replacing a key on an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceKeyInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID to replace (old key).
    pub old_key_id: u32,
    /// New key type.
    pub new_key_type: String,
    /// New key purpose.
    pub new_purpose: String,
    /// New security level.
    pub new_security_level: String,
    /// New private key as hex string (32 bytes).
    pub new_private_key_hex: String,
}

/// Input for setting identity alias.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentityAliasInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// New alias (None to clear).
    pub alias: Option<String>,
}

/// Input for saving custom identity ordering.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveIdentityOrderInput {
    /// Ordered list of identity IDs (hex).
    pub identity_ids: Vec<IdentifierDto>,
}

/// Input for deleting a local identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteIdentityInput {
    /// Identity ID (hex) to delete.
    pub identity_id: IdentifierDto,
}

/// Input for adding a private key to local storage for an identity key.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddPrivateKeyToStorageInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID on the identity to associate the private key with.
    pub key_id: u32,
    /// Private key as hex string (64 hex chars = 32 bytes).
    pub private_key_hex: String,
}

/// Input for removing a private key from local storage for an identity key.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemovePrivateKeyFromStorageInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID on the identity whose private key should be removed.
    pub key_id: u32,
}

/// A key specification for identity registration.
///
/// Each key has a type, purpose, security level, and optional contract bounds.
/// The actual private key is derived from the wallet at registration time.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeySpecDto {
    /// Key type (e.g., "ECDSA_SECP256K1", "ECDSA_HASH160").
    pub key_type: String,
    /// Key purpose (e.g., "AUTHENTICATION", "TRANSFER", "ENCRYPTION", "DECRYPTION").
    pub purpose: String,
    /// Security level (e.g., "MASTER", "CRITICAL", "HIGH", "MEDIUM").
    pub security_level: String,
    /// Optional contract bounds for this key.
    pub contract_bounds: Option<ContractBoundsDto>,
}

/// Funding method for identity registration.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "method")]
pub enum RegisterIdentityFundingMethodDto {
    /// Use an existing asset lock (from a prior create_registration_asset_lock task).
    #[serde(rename_all = "camelCase")]
    UseAssetLock {
        /// Asset lock proof as hex-encoded bytes.
        asset_lock_proof_hex: String,
        /// The full transaction as hex-encoded bytes.
        transaction_hex: String,
        /// The pay-to address string.
        address: String,
    },
    /// Fund from wallet balance by creating a new asset lock transaction.
    #[serde(rename_all = "camelCase")]
    FundWithWallet {
        /// Amount in duffs.
        amount_duffs: u64,
    },
    /// Fund from a specific UTXO.
    #[serde(rename_all = "camelCase")]
    FundWithUtxo {
        /// Previous transaction hash (hex).
        txid: String,
        /// Output index in the previous transaction.
        vout: u32,
        /// Output value in satoshis/duffs.
        value: u64,
        /// Output script as hex.
        script_pub_key_hex: String,
        /// The address string.
        address: String,
    },
    /// Fund from Platform addresses.
    #[serde(rename_all = "camelCase")]
    FundWithPlatformAddresses {
        /// Platform address → credits amount pairs.
        inputs: Vec<PlatformAddressCreditsPair>,
    },
}

/// A platform address and credits pair for funding operations.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlatformAddressCreditsPair {
    /// Platform address string.
    pub address: String,
    /// Amount in credits.
    pub amount: CreditsDto,
}

/// Input for registering a new identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterIdentityInput {
    /// Wallet seed hash (hex) — the wallet that will own and derive keys for this identity.
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Identity index within the wallet.
    pub identity_index: u32,
    /// Alias for this identity.
    pub alias: String,
    /// Master key type (e.g., "ECDSA_HASH160", "ECDSA_SECP256K1").
    pub master_key_type: String,
    /// Additional key specifications beyond the master key.
    /// If empty, default key specs are used (AUTH CRITICAL, AUTH HIGH,
    /// TRANSFER CRITICAL, ENCRYPTION MEDIUM w/ DashPay, DECRYPTION MEDIUM w/ DashPay).
    pub key_specs: Vec<KeySpecDto>,
    /// Whether to use default key specs (ignores key_specs if true).
    pub use_default_keys: bool,
    /// Funding method for identity creation.
    pub funding_method: RegisterIdentityFundingMethodDto,
}

/// Funding method for identity top-up.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "method")]
pub enum TopUpIdentityFundingMethodDto {
    /// Use an existing asset lock.
    #[serde(rename_all = "camelCase")]
    UseAssetLock {
        /// Asset lock proof as hex-encoded bytes.
        asset_lock_proof_hex: String,
        /// The full transaction as hex-encoded bytes.
        transaction_hex: String,
        /// The pay-to address string.
        address: String,
    },
    /// Fund from wallet balance.
    #[serde(rename_all = "camelCase")]
    FundWithWallet {
        /// Amount in duffs.
        amount_duffs: u64,
        /// Top-up index for this identity.
        top_up_index: u32,
    },
    /// Fund from a specific UTXO.
    #[serde(rename_all = "camelCase")]
    FundWithUtxo {
        /// Previous transaction hash (hex).
        txid: String,
        /// Output index.
        vout: u32,
        /// Output value in satoshis/duffs.
        value: u64,
        /// Output script as hex.
        script_pub_key_hex: String,
        /// Address string.
        address: String,
        /// Top-up index for this identity.
        top_up_index: u32,
    },
}

/// Input for topping up an existing identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopUpIdentityInput {
    /// Identity ID (hex) to top up.
    pub identity_id: IdentifierDto,
    /// Wallet seed hash (hex) — the wallet used for funding.
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Identity index within the wallet.
    pub identity_index: u32,
    /// Funding method.
    pub funding_method: TopUpIdentityFundingMethodDto,
}

/// Input for topping up identity from Platform addresses.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopUpIdentityFromPlatformAddressesInput {
    /// Identity ID (hex) to top up.
    pub identity_id: IdentifierDto,
    /// Wallet seed hash (hex) for signing.
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Platform address → credits amount pairs.
    pub inputs: Vec<PlatformAddressCreditsPair>,
}

/// Input for transferring credits from identity to Platform addresses.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferToAddressesInput {
    /// Source identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Platform address → credits amount pairs (destinations).
    pub outputs: Vec<PlatformAddressCreditsPair>,
    /// Optional key ID to use for signing.
    pub key_id: Option<u32>,
}

// ---------------------------------------------------------------------------
// Helper: Convert QualifiedIdentity → QualifiedIdentityDto
// ---------------------------------------------------------------------------

/// Convert a backend `QualifiedIdentity` into a serializable DTO.
///
/// The `network` parameter is required since `QualifiedIdentity` does not
/// carry network information.
pub fn qualified_identity_to_dto(qi: &QualifiedIdentity, network: Network) -> QualifiedIdentityDto {
    let identity = &qi.identity;
    let id_hex = hex::encode(identity.id().to_vec());

    let keys: Vec<IdentityKeyDto> = identity
        .public_keys()
        .values()
        .map(|key| {
            let has_private = qi
                .private_keys
                .private_keys
                .values()
                .any(|qualified_key_pair| {
                    qualified_key_pair.0.identity_public_key.id() == key.id()
                });

            IdentityKeyDto {
                key_id: key.id(),
                key_type: format!("{:?}", key.key_type()),
                purpose: format!("{:?}", key.purpose()),
                security_level: format!("{:?}", key.security_level()),
                data: hex::encode(key.data().as_slice()),
                is_disabled: key.is_disabled(),
                disabled_at: key.disabled_at(),
                has_private_key: has_private,
                contract_bounds: key.contract_bounds().map(|cb| match cb {
                    ContractBounds::SingleContract { id } => ContractBoundsDto::SingleContract {
                        contract_id: hex::encode(id.to_vec()),
                    },
                    ContractBounds::SingleContractDocumentType {
                        id,
                        document_type_name,
                    } => ContractBoundsDto::SingleContractDocumentType {
                        contract_id: hex::encode(id.to_vec()),
                        document_type_name: document_type_name.clone(),
                    },
                }),
            }
        })
        .collect();

    let dpns_names: Vec<DpnsNameInfoDto> = qi
        .dpns_names
        .iter()
        .map(|info| DpnsNameInfoDto {
            name: info.name.clone(),
            acquired_at: info.acquired_at,
        })
        .collect();

    let associated_wallet_hashes: Vec<WalletSeedHashDto> =
        qi.associated_wallets.keys().map(hex::encode).collect();

    let top_ups: Vec<TopUpEntryDto> = qi
        .top_ups
        .iter()
        .map(|(index, amount)| TopUpEntryDto {
            index: *index,
            amount: *amount,
        })
        .collect();

    let identity_type = match qi.identity_type {
        IdentityType::User => IdentityTypeDto::User,
        IdentityType::Masternode => IdentityTypeDto::Masternode,
        IdentityType::Evonode => IdentityTypeDto::Evonode,
    };

    QualifiedIdentityDto {
        id: id_hex,
        identity_type,
        alias: qi.alias.clone(),
        balance: identity.balance(),
        keys,
        dpns_names,
        associated_wallet_hashes,
        wallet_index: qi.wallet_index,
        top_ups,
        status: IdentityStatusDto::Active, // If loaded, it's active
        network: NetworkDto::from_network(network),
        voter_identity_id: qi
            .associated_voter_identity
            .as_ref()
            .map(|(voter_identity, _)| hex::encode(voter_identity.id().to_vec())),
        operator_identity_id: qi
            .associated_operator_identity
            .as_ref()
            .map(|(operator_identity, _)| hex::encode(operator_identity.id().to_vec())),
        masternode_payout_address: qi.masternode_payout_address(network).map(|a| a.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Helper: Parse identifier from hex string
// ---------------------------------------------------------------------------

pub fn parse_identifier(input: &str) -> Result<Identifier, String> {
    let trimmed = input.trim();
    if let Ok(bytes) = hex::decode(trimmed) {
        Identifier::from_bytes(&bytes).map_err(|e| format!("Invalid identifier: {e}"))
    } else {
        Identifier::from_string(trimmed, Encoding::Base58)
            .map_err(|e| format!("Invalid identifier: {e}"))
    }
}

fn parse_wallet_seed_hash(hex_str: &str) -> Result<WalletSeedHash, String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid wallet seed hash hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Wallet seed hash must be 32 bytes, got {}",
            bytes.len()
        ));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

fn parse_private_key_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid private key hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("Private key must be 32 bytes, got {}", bytes.len()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Look up a `QualifiedIdentity` from the local database by identifier.
fn lookup_identity(state: &AppState, identity_id: &str) -> Result<QualifiedIdentity, String> {
    let identifier = parse_identifier(identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_by_id(&identifier)
        .map_err(|e| format!("Database error loading identity: {e}"))?
        .ok_or_else(|| format!("Identity {} not found in local database", identity_id))
}

/// Look up a wallet Arc ref by seed hash.
fn lookup_wallet_arc_ref(
    state: &AppState,
    wallet_seed_hash_hex: &str,
) -> Result<dash_evo_tool::model::wallet::WalletArcRef, String> {
    use dash_evo_tool::model::wallet::WalletArcRef;
    let seed_hash = parse_wallet_seed_hash(wallet_seed_hash_hex)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", wallet_seed_hash_hex))?;
    Ok(WalletArcRef::from(wallet_arc))
}

// ---------------------------------------------------------------------------
// Helpers for identity registration / top-up
// ---------------------------------------------------------------------------

/// Parse a `ContractBoundsDto` into the SDK `ContractBounds` type.
fn parse_contract_bounds(dto: &ContractBoundsDto) -> Result<ContractBounds, String> {
    match dto {
        ContractBoundsDto::SingleContract { contract_id } => {
            let id = parse_identifier(contract_id)?;
            Ok(ContractBounds::SingleContract { id })
        }
        ContractBoundsDto::SingleContractDocumentType {
            contract_id,
            document_type_name,
        } => {
            let id = parse_identifier(contract_id)?;
            Ok(ContractBounds::SingleContractDocumentType {
                id,
                document_type_name: document_type_name.clone(),
            })
        }
    }
}

/// Build `IdentityKeys` from wallet derivation. Derives the master key and
/// additional keys using the wallet's HD key derivation at the given identity index.
fn build_identity_keys(
    state: &AppState,
    wallet_seed_hash_hex: &str,
    identity_index: u32,
    master_key_type: &str,
    key_specs: &[KeySpecDto],
    use_default_keys: bool,
) -> Result<IdentityKeys, String> {
    let seed_hash = parse_wallet_seed_hash(wallet_seed_hash_hex)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", wallet_seed_hash_hex))?;
    let mut wallet = wallet_arc.write_or_recover();

    let master_key_type_parsed = parse_key_type(master_key_type)?;

    // Derive master key (key index 0)
    let network = ctx.network();
    let master_key = wallet
        .identity_authentication_ecdsa_private_key(network, identity_index, 0, Some(ctx))
        .map_err(|e| format!("Failed to derive master key: {e}"))?;

    // Build additional key inputs
    let keys_input = if use_default_keys {
        // Use default key specs (same as egui default_identity_key_specs)
        let dashpay_contract_id = ctx.dashpay_contract_id();
        let dashpay_bounds = Some(ContractBounds::SingleContractDocumentType {
            id: dashpay_contract_id,
            document_type_name: "contactRequest".to_string(),
        });

        let default_specs: Vec<(KeyType, Purpose, SecurityLevel, Option<ContractBounds>)> = vec![
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
                KeyType::ECDSA_SECP256K1,
                Purpose::ENCRYPTION,
                SecurityLevel::MEDIUM,
                dashpay_bounds.clone(),
            ),
            (
                KeyType::ECDSA_SECP256K1,
                Purpose::DECRYPTION,
                SecurityLevel::MEDIUM,
                dashpay_bounds,
            ),
        ];

        default_specs
            .into_iter()
            .enumerate()
            .map(|(i, (kt, purpose, sl, cb))| {
                let key_index = (i + 1) as u32;
                let (private_key, derivation_path) = wallet
                    .identity_authentication_ecdsa_private_key(
                        ctx.network(),
                        identity_index,
                        key_index,
                        Some(ctx),
                    )
                    .map_err(|e| format!("Failed to derive key {}: {e}", key_index))?;
                Ok(((private_key, derivation_path), kt, purpose, sl, cb))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        // Use custom key specs from frontend
        key_specs
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let key_index = (i + 1) as u32;
                let kt = parse_key_type(&spec.key_type)?;
                let purpose = parse_purpose(&spec.purpose)?;
                let sl = parse_security_level(&spec.security_level)?;
                let cb = spec
                    .contract_bounds
                    .as_ref()
                    .map(parse_contract_bounds)
                    .transpose()?;

                let (private_key, derivation_path) = wallet
                    .identity_authentication_ecdsa_private_key(
                        ctx.network(),
                        identity_index,
                        key_index,
                        Some(ctx),
                    )
                    .map_err(|e| format!("Failed to derive key {}: {e}", key_index))?;
                Ok(((private_key, derivation_path), kt, purpose, sl, cb))
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    Ok(IdentityKeys::new(
        Some(master_key),
        master_key_type_parsed,
        keys_input,
    ))
}

/// Parse an address string with network validation.
fn parse_address_checked(
    addr_str: &str,
    network: Network,
) -> Result<dash_sdk::dpp::dashcore::Address, String> {
    addr_str
        .parse::<dash_sdk::dpp::dashcore::Address<
            dash_sdk::dpp::dashcore::address::NetworkUnchecked,
        >>()
        .map_err(|e| format!("Invalid address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("Address network mismatch: {e}"))
}

/// Parse a `RegisterIdentityFundingMethodDto` into the backend `RegisterIdentityFundingMethod`.
fn parse_register_funding_method(
    dto: &RegisterIdentityFundingMethodDto,
    identity_index: u32,
    wallet_seed_hash_hex: &str,
    network: Network,
) -> Result<RegisterIdentityFundingMethod, String> {
    use dash_sdk::dpp::dashcore::consensus::Decodable;

    match dto {
        RegisterIdentityFundingMethodDto::UseAssetLock {
            asset_lock_proof_hex,
            transaction_hex,
            address,
        } => {
            let proof_bytes = hex::decode(asset_lock_proof_hex)
                .map_err(|e| format!("Invalid asset lock proof hex: {e}"))?;
            let asset_lock_proof: dash_sdk::dpp::prelude::AssetLockProof =
                dash_sdk::platform::dpp::bincode::decode_from_slice(
                    &proof_bytes,
                    dash_sdk::platform::dpp::bincode::config::standard()
                        .with_big_endian()
                        .with_no_limit(),
                )
                .map_err(|e| format!("Failed to decode asset lock proof: {e}"))?
                .0;

            let tx_bytes = hex::decode(transaction_hex)
                .map_err(|e| format!("Invalid transaction hex: {e}"))?;
            let transaction =
                dash_sdk::dpp::dashcore::Transaction::consensus_decode(&mut tx_bytes.as_slice())
                    .map_err(|e| format!("Failed to decode transaction: {e}"))?;

            let addr = parse_address_checked(address, network)?;

            Ok(RegisterIdentityFundingMethod::UseAssetLock(
                addr,
                Box::new(asset_lock_proof),
                Box::new(transaction),
            ))
        }
        RegisterIdentityFundingMethodDto::FundWithWallet { amount_duffs } => Ok(
            RegisterIdentityFundingMethod::FundWithWallet(*amount_duffs, identity_index),
        ),
        RegisterIdentityFundingMethodDto::FundWithUtxo {
            txid,
            vout,
            value,
            script_pub_key_hex,
            address,
        } => {
            let txid_hash = txid
                .parse::<dash_sdk::dpp::dashcore::Txid>()
                .map_err(|e| format!("Invalid txid: {e}"))?;
            let outpoint = dash_sdk::dpp::dashcore::OutPoint::new(txid_hash, *vout);

            let script_bytes = hex::decode(script_pub_key_hex)
                .map_err(|e| format!("Invalid script_pub_key hex: {e}"))?;
            let script_pubkey = dash_sdk::dpp::dashcore::ScriptBuf::from_bytes(script_bytes);
            let txout = dash_sdk::dpp::dashcore::TxOut {
                value: *value,
                script_pubkey,
            };

            let addr = parse_address_checked(address, network)?;

            Ok(RegisterIdentityFundingMethod::FundWithUtxo(
                outpoint,
                txout,
                addr,
                identity_index,
            ))
        }
        RegisterIdentityFundingMethodDto::FundWithPlatformAddresses { inputs } => {
            use dash_sdk::dpp::address_funds::PlatformAddress;

            let seed_hash = parse_wallet_seed_hash(wallet_seed_hash_hex)?;
            let mut platform_inputs = std::collections::BTreeMap::new();
            for pair in inputs {
                let addr = parse_address_checked(&pair.address, network)?;
                let platform_addr = PlatformAddress::try_from(addr)
                    .map_err(|e| format!("Invalid platform address: {e}"))?;
                platform_inputs.insert(platform_addr, pair.amount as Credits);
            }
            Ok(RegisterIdentityFundingMethod::FundWithPlatformAddresses {
                inputs: platform_inputs,
                wallet_seed_hash: seed_hash,
            })
        }
    }
}

/// Parse a `TopUpIdentityFundingMethodDto` into the backend `TopUpIdentityFundingMethod`.
fn parse_top_up_funding_method(
    dto: &TopUpIdentityFundingMethodDto,
    identity_index: u32,
    network: Network,
) -> Result<TopUpIdentityFundingMethod, String> {
    use dash_sdk::dpp::dashcore::consensus::Decodable;

    match dto {
        TopUpIdentityFundingMethodDto::UseAssetLock {
            asset_lock_proof_hex,
            transaction_hex,
            address,
        } => {
            let proof_bytes = hex::decode(asset_lock_proof_hex)
                .map_err(|e| format!("Invalid asset lock proof hex: {e}"))?;
            let asset_lock_proof: dash_sdk::dpp::prelude::AssetLockProof =
                dash_sdk::platform::dpp::bincode::decode_from_slice(
                    &proof_bytes,
                    dash_sdk::platform::dpp::bincode::config::standard()
                        .with_big_endian()
                        .with_no_limit(),
                )
                .map_err(|e| format!("Failed to decode asset lock proof: {e}"))?
                .0;

            let tx_bytes = hex::decode(transaction_hex)
                .map_err(|e| format!("Invalid transaction hex: {e}"))?;
            let transaction =
                dash_sdk::dpp::dashcore::Transaction::consensus_decode(&mut tx_bytes.as_slice())
                    .map_err(|e| format!("Failed to decode transaction: {e}"))?;

            let addr = parse_address_checked(address, network)?;

            Ok(TopUpIdentityFundingMethod::UseAssetLock(
                addr,
                Box::new(asset_lock_proof),
                Box::new(transaction),
            ))
        }
        TopUpIdentityFundingMethodDto::FundWithWallet {
            amount_duffs,
            top_up_index,
        } => Ok(TopUpIdentityFundingMethod::FundWithWallet(
            *amount_duffs,
            identity_index,
            *top_up_index,
        )),
        TopUpIdentityFundingMethodDto::FundWithUtxo {
            txid,
            vout,
            value,
            script_pub_key_hex,
            address,
            top_up_index,
        } => {
            let txid_hash = txid
                .parse::<dash_sdk::dpp::dashcore::Txid>()
                .map_err(|e| format!("Invalid txid: {e}"))?;
            let outpoint = dash_sdk::dpp::dashcore::OutPoint::new(txid_hash, *vout);

            let script_bytes = hex::decode(script_pub_key_hex)
                .map_err(|e| format!("Invalid script_pub_key hex: {e}"))?;
            let script_pubkey = dash_sdk::dpp::dashcore::ScriptBuf::from_bytes(script_bytes);
            let txout = dash_sdk::dpp::dashcore::TxOut {
                value: *value,
                script_pubkey,
            };

            let addr = parse_address_checked(address, network)?;

            Ok(TopUpIdentityFundingMethod::FundWithUtxo(
                outpoint,
                txout,
                addr,
                identity_index,
                *top_up_index,
            ))
        }
    }
}

/// Parse platform address pairs into a BTreeMap for backend consumption.
fn parse_platform_address_credits(
    pairs: &[PlatformAddressCreditsPair],
    network: Network,
) -> Result<
    std::collections::BTreeMap<dash_sdk::dpp::address_funds::PlatformAddress, Credits>,
    String,
> {
    use dash_sdk::dpp::address_funds::PlatformAddress;

    let mut map = std::collections::BTreeMap::new();
    for pair in pairs {
        let addr = parse_address_checked(&pair.address, network)?;
        let platform_addr = PlatformAddress::try_from(addr)
            .map_err(|e| format!("Invalid platform address: {e}"))?;
        map.insert(platform_addr, pair.amount as Credits);
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Load an existing identity by ID, optionally associating with wallets.
///
/// Dispatches `IdentityTask::LoadIdentity`. Result arrives via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn identity_load(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: LoadIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let identity_type = match input.identity_type {
        IdentityTypeDto::User => IdentityType::User,
        IdentityTypeDto::Masternode => IdentityType::Masternode,
        IdentityTypeDto::Evonode => IdentityType::Evonode,
    };

    let selected_wallet_seed_hash = input
        .selected_wallet_seed_hash
        .map(|h| parse_wallet_seed_hash(&h))
        .transpose()?;

    let load_input = IdentityInputToLoad {
        identity_id_input: input.identity_id,
        identity_type,
        alias_input: input.alias,
        voting_private_key_input: input.voting_private_key,
        owner_private_key_input: input.owner_private_key,
        payout_address_private_key_input: input.payout_address_private_key,
        keys_input: input.keys,
        derive_keys_from_wallets: input.derive_keys_from_wallets,
        selected_wallet_seed_hash,
    };

    let task = BackendTask::IdentityTask(IdentityTask::LoadIdentity(load_input));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Search for an identity by DPNS name.
///
/// Dispatches `IdentityTask::SearchIdentityByDpnsName`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_search_by_dpns_name(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchIdentityByDpnsNameInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet_seed_hash = input
        .wallet_seed_hash
        .map(|h| parse_wallet_seed_hash(&h))
        .transpose()?;

    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
        input.name,
        wallet_seed_hash,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Search for an identity from a wallet at a specific index.
///
/// Dispatches `IdentityTask::SearchIdentityFromWallet`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_search_from_wallet(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchIdentityFromWalletInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet_ref = lookup_wallet_arc_ref(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityFromWallet(
        wallet_ref,
        input.identity_index,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Batch search for identities from a wallet up to a max index.
///
/// Dispatches `IdentityTask::SearchIdentitiesUpToIndex`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_search_up_to_index(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchIdentitiesUpToIndexInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet_ref = lookup_wallet_arc_ref(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentitiesUpToIndex(
        wallet_ref,
        input.max_identity_index,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Register a DPNS name for an identity.
///
/// Dispatches `IdentityTask::RegisterDpnsName`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_register_dpns_name(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RegisterDpnsNameCommandInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
        qualified_identity: qi,
        name_input: input.name,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Refresh a single identity's state from Platform.
///
/// Dispatches `IdentityTask::RefreshIdentity`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_refresh(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RefreshIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Refresh all loaded identities' DPNS names.
///
/// Dispatches `IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_refresh_dpns_names(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::IdentityTask(IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Withdraw credits from an identity to a Core address.
///
/// Dispatches `IdentityTask::WithdrawFromIdentity`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_withdraw(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: WithdrawFromIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let network = state.current_context().network();
    let to_address = input
        .to_address
        .map(|addr_str| parse_address_checked(&addr_str, network))
        .transpose()?;
    let key_id: Option<KeyID> = input.key_id.map(|id| id as KeyID);

    let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
        qi,
        to_address,
        input.credits as Credits,
        key_id,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Transfer credits from one identity to another.
///
/// Dispatches `IdentityTask::Transfer`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_transfer(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TransferCreditsInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.from_identity_id)?;
    let to_id = parse_identifier(&input.to_identity_id)?;
    let key_id: Option<KeyID> = input.key_id.map(|id| id as KeyID);

    let task = BackendTask::IdentityTask(IdentityTask::Transfer(
        qi,
        to_id,
        input.credits as Credits,
        key_id,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Add a key to an identity.
///
/// Dispatches `IdentityTask::AddKeyToIdentity`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_add_key(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: AddKeyToIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let private_key = parse_private_key_hex(&input.private_key_hex)?;

    let key_type = parse_key_type(&input.key_type)?;
    let purpose = parse_purpose(&input.purpose)?;
    let security_level = parse_security_level(&input.security_level)?;

    // Derive the public key data from the private key
    let public_key_data = derive_public_key_data(key_type, &private_key)?;

    // Parse optional contract bounds
    let contract_bounds = input
        .contract_bounds
        .as_ref()
        .map(parse_contract_bounds)
        .transpose()?;

    // Construct the IdentityPublicKey with a temporary ID (backend will reassign)
    let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0, // Backend will set the correct ID
        key_type,
        purpose,
        security_level,
        contract_bounds,
        read_only: false,
        data: public_key_data.into(),
        disabled_at: None,
    });

    let qualified_public_key = QualifiedIdentityPublicKey {
        identity_public_key,
        in_wallet_at_derivation_path: None,
    };

    let task = BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(
        qi,
        qualified_public_key,
        private_key,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Disable one or more keys on an identity.
///
/// Dispatches `IdentityTask::DisableKeys`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_disable_keys(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: DisableKeysInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let key_ids: Vec<KeyID> = input.key_ids.iter().map(|&id| id as KeyID).collect();

    let task = BackendTask::IdentityTask(IdentityTask::DisableKeys(qi, key_ids));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Replace a key on an identity (disable old + add new atomically).
///
/// Dispatches `IdentityTask::ReplaceKey`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_replace_key(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: ReplaceKeyInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let new_private_key = parse_private_key_hex(&input.new_private_key_hex)?;

    let key_type = parse_key_type(&input.new_key_type)?;
    let purpose = parse_purpose(&input.new_purpose)?;
    let security_level = parse_security_level(&input.new_security_level)?;

    // Derive the public key data from the new private key
    let public_key_data = derive_public_key_data(key_type, &new_private_key)?;

    let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0, // Backend will set the correct ID
        key_type,
        purpose,
        security_level,
        contract_bounds: None,
        read_only: false,
        data: public_key_data.into(),
        disabled_at: None,
    });

    let new_qualified_key = QualifiedIdentityPublicKey {
        identity_public_key,
        in_wallet_at_derivation_path: None,
    };

    let task = BackendTask::IdentityTask(IdentityTask::ReplaceKey(
        qi,
        input.old_key_id as KeyID,
        new_qualified_key,
        new_private_key,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Register a new identity on the platform.
///
/// Derives keys from the wallet, constructs the identity registration info,
/// and dispatches `IdentityTask::RegisterIdentity`. Supports all 4 funding methods:
/// UseAssetLock, FundWithWallet, FundWithUtxo, FundWithPlatformAddresses.
///
/// Result arrives via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn identity_register(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RegisterIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    // Build identity keys from wallet derivation
    let identity_keys = build_identity_keys(
        &state,
        &input.wallet_seed_hash,
        input.identity_index,
        &input.master_key_type,
        &input.key_specs,
        input.use_default_keys,
    )?;

    // Parse funding method
    let network = state.current_context().network();
    let funding_method = parse_register_funding_method(
        &input.funding_method,
        input.identity_index,
        &input.wallet_seed_hash,
        network,
    )?;

    // Look up wallet Arc reference
    let wallet_arc_ref = lookup_wallet_arc_ref(&state, &input.wallet_seed_hash)?;

    let registration_info = IdentityRegistrationInfo {
        alias_input: input.alias,
        keys: identity_keys,
        wallet: wallet_arc_ref.wallet,
        wallet_identity_index: input.identity_index,
        identity_funding_method: funding_method,
    };

    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(registration_info));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Top up an existing identity with additional credits.
///
/// Dispatches `IdentityTask::TopUpIdentity`. Supports UseAssetLock, FundWithWallet,
/// and FundWithUtxo funding methods.
///
/// Result arrives via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn identity_top_up(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TopUpIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let wallet_arc_ref = lookup_wallet_arc_ref(&state, &input.wallet_seed_hash)?;
    let network = state.current_context().network();

    let funding_method =
        parse_top_up_funding_method(&input.funding_method, input.identity_index, network)?;

    let top_up_info = IdentityTopUpInfo {
        qualified_identity: qi,
        wallet: wallet_arc_ref.wallet,
        identity_funding_method: funding_method,
    };

    let task = BackendTask::IdentityTask(IdentityTask::TopUpIdentity(top_up_info));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Top up an existing identity from Platform addresses.
///
/// Dispatches `IdentityTask::TopUpIdentityFromPlatformAddresses`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_top_up_from_platform_addresses(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TopUpIdentityFromPlatformAddressesInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let network = state.current_context().network();
    let inputs = parse_platform_address_credits(&input.inputs, network)?;

    let task = BackendTask::IdentityTask(IdentityTask::TopUpIdentityFromPlatformAddresses {
        identity: qi,
        inputs,
        wallet_seed_hash: seed_hash,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Transfer credits from an identity to Platform addresses.
///
/// Dispatches `IdentityTask::TransferToAddresses`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_transfer_to_addresses(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TransferToAddressesInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let network = state.current_context().network();
    let outputs = parse_platform_address_credits(&input.outputs, network)?;
    let key_id: Option<KeyID> = input.key_id.map(|id| id as KeyID);

    let task = BackendTask::IdentityTask(IdentityTask::TransferToAddresses {
        identity: qi,
        outputs,
        key_id,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Direct database read commands (synchronous, return immediately)
// ---------------------------------------------------------------------------

/// Load all local qualified identities from the database.
#[tauri::command]
#[specta::specta]
pub fn identity_list_local(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<QualifiedIdentityDto>, String> {
    let ctx = state.current_context();
    let network = ctx.network();
    let identities = ctx
        .load_local_qualified_identities()
        .map_err(|e| format!("Failed to load identities: {e}"))?;

    Ok(identities
        .iter()
        .map(|qi| qualified_identity_to_dto(qi, network))
        .collect())
}

/// Load all local user identities (non-masternode/evonode).
#[tauri::command]
#[specta::specta]
pub fn identity_list_user(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<QualifiedIdentityDto>, String> {
    let ctx = state.current_context();
    let network = ctx.network();
    let identities = ctx
        .load_local_user_identities()
        .map_err(|e| format!("Failed to load user identities: {e}"))?;

    Ok(identities
        .iter()
        .map(|qi| qualified_identity_to_dto(qi, network))
        .collect())
}

/// Load all local voting identities.
#[tauri::command]
#[specta::specta]
pub fn identity_list_voting(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<QualifiedIdentityDto>, String> {
    let ctx = state.current_context();
    let network = ctx.network();
    let identities = ctx
        .load_local_voting_identities()
        .map_err(|e| format!("Failed to load voting identities: {e}"))?;

    Ok(identities
        .iter()
        .map(|qi| qualified_identity_to_dto(qi, network))
        .collect())
}

/// Get a single identity by its ID.
#[tauri::command]
#[specta::specta]
pub fn identity_get_by_id(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
) -> Result<Option<QualifiedIdentityDto>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let ctx = state.current_context();
    let network = ctx.network();
    let identity = ctx
        .get_identity_by_id(&identifier)
        .map_err(|e| format!("Failed to get identity: {e}"))?;

    Ok(identity
        .as_ref()
        .map(|qi| qualified_identity_to_dto(qi, network)))
}

/// Set the alias for an identity.
#[tauri::command]
#[specta::specta]
pub fn identity_set_alias(
    state: tauri::State<'_, Arc<AppState>>,
    input: SetIdentityAliasInput,
) -> Result<(), String> {
    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    ctx.set_identity_alias(&identifier, input.alias.as_deref())
        .map_err(|e| format!("Failed to set identity alias: {e}"))
}

/// Get the alias for an identity.
#[tauri::command]
#[specta::specta]
pub fn identity_get_alias(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
) -> Result<Option<String>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_alias(&identifier)
        .map_err(|e| format!("Failed to get identity alias: {e}"))
}

/// Load the custom identity ordering.
#[tauri::command]
#[specta::specta]
pub fn identity_load_order(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<IdentifierDto>, String> {
    let db = state.db();
    let ids = db
        .load_identity_order()
        .map_err(|e| format!("Failed to load identity order: {e}"))?;
    Ok(ids.iter().map(|id| hex::encode(id.to_vec())).collect())
}

/// Save the custom identity ordering.
#[tauri::command]
#[specta::specta]
pub fn identity_save_order(
    state: tauri::State<'_, Arc<AppState>>,
    input: SaveIdentityOrderInput,
) -> Result<(), String> {
    let ids: Vec<Identifier> = input
        .identity_ids
        .iter()
        .map(|hex_str| parse_identifier(hex_str))
        .collect::<Result<Vec<_>, _>>()?;
    let db = state.db();
    db.save_identity_order(ids)
        .map_err(|e| format!("Failed to save identity order: {e}"))
}

/// Delete a local identity from the database.
#[tauri::command]
#[specta::specta]
pub fn identity_delete(
    state: tauri::State<'_, Arc<AppState>>,
    input: DeleteIdentityInput,
) -> Result<(), String> {
    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    let db = state.db();

    // Load the identity first so we can check for an associated voter identity
    let qi = ctx
        .get_identity_by_id(&identifier)
        .map_err(|e| format!("Failed to load identity: {e}"))?;

    db.delete_local_qualified_identity(&identifier, ctx)
        .map_err(|e| format!("Failed to delete identity: {e}"))?;

    // Cascade delete the associated voter identity if present
    if let Some(qi) = qi {
        if let Some((voter_identity, _)) = &qi.associated_voter_identity {
            let voter_id = voter_identity.id();
            if let Err(e) = db.delete_local_qualified_identity(&voter_id, ctx) {
                tracing::warn!("Failed to delete voter identity: {}", e);
            }
        }
    }

    Ok(())
}

/// Get identity summaries suitable for dropdown selectors.
#[tauri::command]
#[specta::specta]
pub fn identity_list_summaries(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<IdentitySummaryDto>, String> {
    let ctx = state.current_context();
    let identities = ctx
        .load_local_qualified_identities()
        .map_err(|e| format!("Failed to load identities: {e}"))?;

    Ok(identities
        .iter()
        .map(|qi| {
            let id_hex = hex::encode(qi.identity.id().to_vec());
            let display_name = qi.alias.clone().unwrap_or_else(|| truncate_id(&id_hex));

            IdentitySummaryDto {
                id: id_hex,
                display_name,
                identity_type: match qi.identity_type {
                    IdentityType::User => IdentityTypeDto::User,
                    IdentityType::Masternode => IdentityTypeDto::Masternode,
                    IdentityType::Evonode => IdentityTypeDto::Evonode,
                },
                balance: qi.identity.balance(),
            }
        })
        .collect())
}

/// Get all local DPNS names across all loaded identities.
#[tauri::command]
#[specta::specta]
pub fn identity_local_dpns_names(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<DpnsNameEntryDto>, String> {
    let ctx = state.current_context();
    let names = ctx
        .local_dpns_names()
        .map_err(|e| format!("Failed to load DPNS names: {e}"))?;

    Ok(names
        .iter()
        .map(|(id, info)| DpnsNameEntryDto {
            identity_id: hex::encode(id.to_vec()),
            name: info.name.clone(),
            acquired_at: info.acquired_at,
        })
        .collect())
}

/// A DPNS name entry with its owning identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DpnsNameEntryDto {
    /// Identity ID (hex) that owns this name.
    pub identity_id: IdentifierDto,
    /// The DPNS name (e.g., "alice.dash").
    pub name: String,
    /// Timestamp when the name was acquired.
    pub acquired_at: u64,
}

// ---------------------------------------------------------------------------
// Message signing
// ---------------------------------------------------------------------------

/// Input for signing a message with an identity key.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID on the identity to sign with.
    pub key_id: u32,
    /// The message text to sign.
    pub message: String,
}

/// Sign a message using Dash's signed message protocol.
///
/// Uses the private key associated with the given key ID on the identity.
/// Supports ECDSA_SECP256K1 and ECDSA_HASH160 key types.
/// Returns the signature as a Base64-encoded string (65 bytes: 1 recovery flag + 64 compact sig).
#[tauri::command]
#[specta::specta]
pub fn identity_sign_message(
    state: tauri::State<'_, Arc<AppState>>,
    input: SignMessageInput,
) -> Result<String, String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::key::Secp256k1;
    use dash_sdk::dpp::dashcore::secp256k1::{Message, SecretKey};
    use dash_sdk::dpp::dashcore::sign_message::signed_msg_hash;

    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    let qi = ctx
        .get_identity_by_id(&identifier)
        .map_err(|e| format!("Failed to get identity: {e}"))?
        .ok_or("Identity not found")?;

    // Determine the private key target from the key's purpose
    let key_id = input.key_id as KeyID;
    let identity_key = qi
        .identity
        .get_public_key_by_id(key_id)
        .ok_or(format!("Key with ID {} not found on identity", key_id))?;

    let target = PrivateKeyTarget::from(identity_key.purpose());

    // Resolve the private key bytes (handles Clear, AlwaysClear, and AtWalletDerivationPath)
    let wallets: Vec<_> = ctx.loaded_wallets().into_iter().map(|(_, w)| w).collect();
    let (_pub_key, private_key_bytes) = qi
        .private_keys
        .get_resolve(&(target, key_id), &wallets, ctx.network())
        .map_err(|e| format!("Failed to resolve private key: {e}"))?
        .ok_or("Private key not available for this key")?;

    // Only ECDSA key types support message signing
    let key_type = identity_key.key_type();
    match key_type {
        KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
            let secp = Secp256k1::new();

            let message_hash = signed_msg_hash(&input.message);
            let message = Message::from_digest(*message_hash.as_byte_array());

            let secret_key = SecretKey::from_byte_array(&private_key_bytes)
                .map_err(|e| format!("Invalid private key: {e}"))?;

            let signature = secp.sign_ecdsa(&message, &secret_key);

            // Compact signature (64 bytes) with recovery flag (0x20) prepended
            let mut serialized = signature.serialize_compact().to_vec();
            serialized.insert(0, 32u8);

            Ok(STANDARD.encode(serialized))
        }
        _ => Err(format!(
            "Unsupported key type for signing: {:?}. Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
            key_type
        )),
    }
}

// ---------------------------------------------------------------------------
// Private key storage management
// ---------------------------------------------------------------------------

/// Add a private key to local storage for an identity key.
///
/// Parses the hex private key, validates it against the identity's public key,
/// stores it in the qualified identity's key storage, and persists to the database.
/// Returns the updated `QualifiedIdentityDto`.
#[tauri::command]
#[specta::specta]
pub fn identity_add_private_key_to_storage(
    state: tauri::State<'_, Arc<AppState>>,
    input: AddPrivateKeyToStorageInput,
) -> Result<QualifiedIdentityDto, String> {
    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    let mut qi = ctx
        .get_identity_by_id(&identifier)
        .map_err(|e| format!("Failed to get identity: {e}"))?
        .ok_or("Identity not found")?;

    let key_id = input.key_id as KeyID;
    let identity_key = qi
        .identity
        .get_public_key_by_id(key_id)
        .ok_or(format!("Key with ID {} not found on identity", key_id))?
        .clone();

    // Parse the hex private key
    let private_key_bytes = parse_private_key_hex(&input.private_key_hex)?;

    // Validate the private key matches the public key
    let network = ctx.network();
    let validation_result = identity_key
        .validate_private_key_bytes(&private_key_bytes, network)
        .map_err(|e| format!("Issue verifying private key: {e}"))?;
    if !validation_result {
        return Err("Private key does not match the public key.".to_string());
    }

    // Store in qualified identity
    let target = PrivateKeyTarget::from(identity_key.purpose());
    let qualified_pub_key = QualifiedIdentityPublicKey {
        identity_public_key: identity_key.clone(),
        in_wallet_at_derivation_path: None,
    };
    qi.private_keys
        .insert_non_encrypted((target, key_id), (qualified_pub_key, private_key_bytes));

    // Persist
    ctx.update_local_qualified_identity(&qi)
        .map_err(|e| format!("Failed to save identity: {e}"))?;

    Ok(qualified_identity_to_dto(&qi, network))
}

/// Remove a private key from local storage for an identity key.
///
/// Removes the private key from the qualified identity's key storage and
/// persists the change to the database. Returns the updated `QualifiedIdentityDto`.
#[tauri::command]
#[specta::specta]
pub fn identity_remove_private_key_from_storage(
    state: tauri::State<'_, Arc<AppState>>,
    input: RemovePrivateKeyFromStorageInput,
) -> Result<QualifiedIdentityDto, String> {
    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    let mut qi = ctx
        .get_identity_by_id(&identifier)
        .map_err(|e| format!("Failed to get identity: {e}"))?
        .ok_or("Identity not found")?;

    let key_id = input.key_id as KeyID;
    let identity_key = qi
        .identity
        .get_public_key_by_id(key_id)
        .ok_or(format!("Key with ID {} not found on identity", key_id))?;

    let target = PrivateKeyTarget::from(identity_key.purpose());
    qi.private_keys.private_keys.remove(&(target, key_id));

    // Persist
    let network = ctx.network();
    ctx.update_local_qualified_identity(&qi)
        .map_err(|e| format!("Failed to save identity: {e}"))?;

    Ok(qualified_identity_to_dto(&qi, network))
}

// ---------------------------------------------------------------------------
// Helper: String parsing for key types / purposes / security levels
// ---------------------------------------------------------------------------

/// Derive public key data from a private key for the given key type.
fn derive_public_key_data(key_type: KeyType, private_key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::key::Secp256k1;
    use dash_sdk::dpp::dashcore::PrivateKey;

    match key_type {
        KeyType::ECDSA_SECP256K1 => {
            let secp = Secp256k1::new();
            let secret_key = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(private_key)
                .map_err(|e| format!("Invalid private key: {e}"))?;
            let pk = PrivateKey::new(secret_key, dash_sdk::dpp::dashcore::Network::Dash);
            Ok(pk.public_key(&secp).to_bytes().to_vec())
        }
        KeyType::ECDSA_HASH160 => {
            let secp = Secp256k1::new();
            let secret_key = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(private_key)
                .map_err(|e| format!("Invalid private key: {e}"))?;
            let pk = PrivateKey::new(secret_key, dash_sdk::dpp::dashcore::Network::Dash);
            Ok(pk.public_key(&secp).pubkey_hash().to_byte_array().to_vec())
        }
        _ => Err(format!(
            "Cannot derive public key for key type {:?} from raw private key bytes. \
            Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
            key_type
        )),
    }
}

fn parse_key_type(s: &str) -> Result<KeyType, String> {
    match s.to_uppercase().as_str() {
        "ECDSA_SECP256K1" => Ok(KeyType::ECDSA_SECP256K1),
        "BLS12_381" => Ok(KeyType::BLS12_381),
        "ECDSA_HASH160" => Ok(KeyType::ECDSA_HASH160),
        "BIP13_SCRIPT_HASH" => Ok(KeyType::BIP13_SCRIPT_HASH),
        "EDDSA_25519_HASH160" => Ok(KeyType::EDDSA_25519_HASH160),
        other => Err(format!("Unknown key type: {}", other)),
    }
}

fn parse_purpose(s: &str) -> Result<Purpose, String> {
    match s.to_uppercase().as_str() {
        "AUTHENTICATION" => Ok(Purpose::AUTHENTICATION),
        "ENCRYPTION" => Ok(Purpose::ENCRYPTION),
        "DECRYPTION" => Ok(Purpose::DECRYPTION),
        "TRANSFER" => Ok(Purpose::TRANSFER),
        "VOTING" => Ok(Purpose::VOTING),
        "OWNER" => Ok(Purpose::OWNER),
        other => Err(format!("Unknown key purpose: {}", other)),
    }
}

fn parse_security_level(s: &str) -> Result<SecurityLevel, String> {
    match s.to_uppercase().as_str() {
        "MASTER" => Ok(SecurityLevel::MASTER),
        "CRITICAL" => Ok(SecurityLevel::CRITICAL),
        "HIGH" => Ok(SecurityLevel::HIGH),
        "MEDIUM" => Ok(SecurityLevel::MEDIUM),
        other => Err(format!("Unknown security level: {}", other)),
    }
}

/// Truncate an identifier hex string for display (first 8 + last 4 chars).
fn truncate_id(hex: &str) -> String {
    if hex.len() <= 16 {
        hex.to_string()
    } else {
        format!("{}...{}", &hex[..8], &hex[hex.len() - 4..])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_identity_input_serializes_with_camel_case() {
        let input = LoadIdentityInput {
            identity_id: "abc123".into(),
            identity_type: IdentityTypeDto::User,
            alias: "Alice".into(),
            voting_private_key: String::new(),
            owner_private_key: String::new(),
            payout_address_private_key: String::new(),
            keys: vec![],
            derive_keys_from_wallets: true,
            selected_wallet_seed_hash: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"identityType\":\"user\""));
        assert!(json.contains("\"deriveKeysFromWallets\":true"));
        assert!(json.contains("\"selectedWalletSeedHash\":null"));
    }

    #[test]
    fn search_by_dpns_name_input_serializes() {
        let input = SearchIdentityByDpnsNameInput {
            name: "alice".into(),
            wallet_seed_hash: Some("deadbeef".repeat(4)),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"name\":\"alice\""));
        assert!(json.contains("\"walletSeedHash\""));
    }

    #[test]
    fn register_dpns_name_input_serializes() {
        let input = RegisterDpnsNameCommandInput {
            identity_id: "abc123".into(),
            name: "bob".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"name\":\"bob\""));
    }

    #[test]
    fn withdraw_input_serializes() {
        let input = WithdrawFromIdentityInput {
            identity_id: "abc123".into(),
            to_address: Some("XpYvN123".into()),
            credits: 50000,
            key_id: Some(2),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"toAddress\":\"XpYvN123\""));
        assert!(json.contains("\"credits\":50000"));
        assert!(json.contains("\"keyId\":2"));
    }

    #[test]
    fn transfer_input_serializes() {
        let input = TransferCreditsInput {
            from_identity_id: "aaa".into(),
            to_identity_id: "bbb".into(),
            credits: 100000,
            key_id: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"fromIdentityId\":\"aaa\""));
        assert!(json.contains("\"toIdentityId\":\"bbb\""));
        assert!(json.contains("\"keyId\":null"));
    }

    #[test]
    fn add_key_input_serializes() {
        let input = AddKeyToIdentityInput {
            identity_id: "abc".into(),
            key_type: "ECDSA_SECP256K1".into(),
            purpose: "AUTHENTICATION".into(),
            security_level: "HIGH".into(),
            private_key_hex: "aa".repeat(32),
            contract_bounds: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyType\":\"ECDSA_SECP256K1\""));
        assert!(json.contains("\"purpose\":\"AUTHENTICATION\""));
        assert!(json.contains("\"securityLevel\":\"HIGH\""));
        assert!(json.contains("\"contractBounds\":null"));
    }

    #[test]
    fn add_key_input_with_contract_bounds_serializes() {
        let input = AddKeyToIdentityInput {
            identity_id: "abc".into(),
            key_type: "ECDSA_SECP256K1".into(),
            purpose: "AUTHENTICATION".into(),
            security_level: "HIGH".into(),
            private_key_hex: "aa".repeat(32),
            contract_bounds: Some(ContractBoundsDto::SingleContractDocumentType {
                contract_id: "deadbeef".into(),
                document_type_name: "profile".into(),
            }),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"contractBounds\":{"));
        assert!(json.contains("\"type\":\"singleContractDocumentType\""));
        assert!(json.contains("\"contractId\":\"deadbeef\""));
        assert!(json.contains("\"documentTypeName\":\"profile\""));
    }

    #[test]
    fn disable_keys_input_serializes() {
        let input = DisableKeysInput {
            identity_id: "abc".into(),
            key_ids: vec![1, 3, 5],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyIds\":[1,3,5]"));
    }

    #[test]
    fn replace_key_input_serializes() {
        let input = ReplaceKeyInput {
            identity_id: "abc".into(),
            old_key_id: 2,
            new_key_type: "BLS12_381".into(),
            new_purpose: "VOTING".into(),
            new_security_level: "MEDIUM".into(),
            new_private_key_hex: "bb".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"oldKeyId\":2"));
        assert!(json.contains("\"newKeyType\":\"BLS12_381\""));
    }

    #[test]
    fn set_alias_input_serializes() {
        let input = SetIdentityAliasInput {
            identity_id: "abc".into(),
            alias: Some("My Identity".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":\"My Identity\""));
    }

    #[test]
    fn save_order_input_serializes() {
        let input = SaveIdentityOrderInput {
            identity_ids: vec!["aaa".into(), "bbb".into()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityIds\":[\"aaa\",\"bbb\"]"));
    }

    #[test]
    fn delete_identity_input_serializes() {
        let input = DeleteIdentityInput {
            identity_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
    }

    #[test]
    fn dpns_name_entry_serializes() {
        let entry = DpnsNameEntryDto {
            identity_id: "abc".into(),
            name: "alice.dash".into(),
            acquired_at: 1234567890,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"name\":\"alice.dash\""));
        assert!(json.contains("\"acquiredAt\":1234567890"));
    }

    #[test]
    fn parse_identifier_valid_hex() {
        let hex_str = "a".repeat(64); // 32 bytes as hex
        let result = parse_identifier(&hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_identifier_invalid_hex() {
        let result = parse_identifier("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_wallet_seed_hash_valid() {
        let hex_str = "b".repeat(64);
        let result = parse_wallet_seed_hash(&hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_wallet_seed_hash_wrong_length() {
        let hex_str = "bb".repeat(16); // 16 bytes, not 32
        let result = parse_wallet_seed_hash(&hex_str);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("32 bytes"));
    }

    #[test]
    fn parse_private_key_hex_valid() {
        let hex_str = "cc".repeat(32);
        let result = parse_private_key_hex(&hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_key_type_variants() {
        assert!(parse_key_type("ECDSA_SECP256K1").is_ok());
        assert!(parse_key_type("BLS12_381").is_ok());
        assert!(parse_key_type("ECDSA_HASH160").is_ok());
        assert!(parse_key_type("BIP13_SCRIPT_HASH").is_ok());
        assert!(parse_key_type("EDDSA_25519_HASH160").is_ok());
        assert!(parse_key_type("ecdsa_secp256k1").is_ok()); // case insensitive
        assert!(parse_key_type("UNKNOWN").is_err());
    }

    #[test]
    fn parse_purpose_variants() {
        assert!(parse_purpose("AUTHENTICATION").is_ok());
        assert!(parse_purpose("ENCRYPTION").is_ok());
        assert!(parse_purpose("DECRYPTION").is_ok());
        assert!(parse_purpose("TRANSFER").is_ok());
        assert!(parse_purpose("VOTING").is_ok());
        assert!(parse_purpose("OWNER").is_ok());
        assert!(parse_purpose("voting").is_ok()); // case insensitive
        assert!(parse_purpose("UNKNOWN").is_err());
    }

    #[test]
    fn parse_security_level_variants() {
        assert!(parse_security_level("MASTER").is_ok());
        assert!(parse_security_level("CRITICAL").is_ok());
        assert!(parse_security_level("HIGH").is_ok());
        assert!(parse_security_level("MEDIUM").is_ok());
        assert!(parse_security_level("medium").is_ok()); // case insensitive
        assert!(parse_security_level("UNKNOWN").is_err());
    }

    #[test]
    fn truncate_id_short() {
        assert_eq!(truncate_id("abcdef"), "abcdef");
    }

    #[test]
    fn truncate_id_long() {
        let long_id = "a".repeat(64);
        let truncated = truncate_id(&long_id);
        assert_eq!(truncated, "aaaaaaaa...aaaa");
    }

    #[test]
    fn search_identity_from_wallet_input_serializes() {
        let input = SearchIdentityFromWalletInput {
            wallet_seed_hash: "aa".repeat(32),
            identity_index: 5,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
        assert!(json.contains("\"identityIndex\":5"));
    }

    #[test]
    fn search_identities_up_to_index_input_serializes() {
        let input = SearchIdentitiesUpToIndexInput {
            wallet_seed_hash: "bb".repeat(32),
            max_identity_index: 10,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"maxIdentityIndex\":10"));
    }

    #[test]
    fn refresh_identity_input_serializes() {
        let input = RefreshIdentityInput {
            identity_id: "abc123".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
    }

    #[test]
    fn identity_roundtrip_deserialization() {
        let json = r#"{
            "identityId": "abc123",
            "identityType": "masternode",
            "alias": "My MN",
            "votingPrivateKey": "",
            "ownerPrivateKey": "",
            "payoutAddressPrivateKey": "",
            "keys": [],
            "deriveKeysFromWallets": false,
            "selectedWalletSeedHash": null
        }"#;
        let input: LoadIdentityInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.identity_id, "abc123");
        assert_eq!(input.identity_type, IdentityTypeDto::Masternode);
        assert_eq!(input.alias, "My MN");
        assert!(!input.derive_keys_from_wallets);
    }

    // --- Tests for new registration/top-up/transfer DTOs ---

    #[test]
    fn key_spec_dto_serializes_with_camel_case() {
        let spec = KeySpecDto {
            key_type: "ECDSA_HASH160".into(),
            purpose: "AUTHENTICATION".into(),
            security_level: "CRITICAL".into(),
            contract_bounds: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"keyType\":\"ECDSA_HASH160\""));
        assert!(json.contains("\"purpose\":\"AUTHENTICATION\""));
        assert!(json.contains("\"securityLevel\":\"CRITICAL\""));
        assert!(json.contains("\"contractBounds\":null"));
    }

    #[test]
    fn key_spec_dto_with_contract_bounds_serializes() {
        let spec = KeySpecDto {
            key_type: "ECDSA_SECP256K1".into(),
            purpose: "ENCRYPTION".into(),
            security_level: "MEDIUM".into(),
            contract_bounds: Some(ContractBoundsDto::SingleContractDocumentType {
                contract_id: "abc123".into(),
                document_type_name: "contactRequest".into(),
            }),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"contractBounds\":{"));
        assert!(json.contains("\"type\":\"singleContractDocumentType\""));
        assert!(json.contains("\"contractId\":\"abc123\""));
        assert!(json.contains("\"documentTypeName\":\"contactRequest\""));
    }

    #[test]
    fn contract_bounds_single_contract_serializes() {
        let bounds = ContractBoundsDto::SingleContract {
            contract_id: "def456".into(),
        };
        let json = serde_json::to_string(&bounds).unwrap();
        assert!(json.contains("\"type\":\"singleContract\""));
        assert!(json.contains("\"contractId\":\"def456\""));
    }

    #[test]
    fn register_identity_funding_fund_with_wallet_serializes() {
        let method = RegisterIdentityFundingMethodDto::FundWithWallet {
            amount_duffs: 50000,
        };
        let json = serde_json::to_string(&method).unwrap();
        assert!(json.contains("\"method\":\"fundWithWallet\""));
        assert!(json.contains("\"amountDuffs\":50000"));
    }

    #[test]
    fn register_identity_funding_use_asset_lock_serializes() {
        let method = RegisterIdentityFundingMethodDto::UseAssetLock {
            asset_lock_proof_hex: "aabb".into(),
            transaction_hex: "ccdd".into(),
            address: "XpYvN123".into(),
        };
        let json = serde_json::to_string(&method).unwrap();
        assert!(json.contains("\"method\":\"useAssetLock\""));
        assert!(json.contains("\"assetLockProofHex\":\"aabb\""));
        assert!(json.contains("\"transactionHex\":\"ccdd\""));
    }

    #[test]
    fn register_identity_funding_fund_with_utxo_serializes() {
        let method = RegisterIdentityFundingMethodDto::FundWithUtxo {
            txid: "aa".repeat(32),
            vout: 0,
            value: 100000,
            script_pub_key_hex: "76a914".into(),
            address: "XpYvN456".into(),
        };
        let json = serde_json::to_string(&method).unwrap();
        assert!(json.contains("\"method\":\"fundWithUtxo\""));
        assert!(json.contains("\"vout\":0"));
        assert!(json.contains("\"value\":100000"));
        assert!(json.contains("\"scriptPubKeyHex\":\"76a914\""));
    }

    #[test]
    fn register_identity_funding_platform_addresses_serializes() {
        let method = RegisterIdentityFundingMethodDto::FundWithPlatformAddresses {
            inputs: vec![PlatformAddressCreditsPair {
                address: "XpYvN789".into(),
                amount: 250000,
            }],
        };
        let json = serde_json::to_string(&method).unwrap();
        assert!(json.contains("\"method\":\"fundWithPlatformAddresses\""));
        assert!(json.contains("\"address\":\"XpYvN789\""));
        assert!(json.contains("\"amount\":250000"));
    }

    #[test]
    fn register_identity_input_serializes_with_camel_case() {
        let input = RegisterIdentityInput {
            wallet_seed_hash: "aa".repeat(32),
            identity_index: 0,
            alias: "My Identity".into(),
            master_key_type: "ECDSA_HASH160".into(),
            key_specs: vec![],
            use_default_keys: true,
            funding_method: RegisterIdentityFundingMethodDto::FundWithWallet {
                amount_duffs: 100000,
            },
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
        assert!(json.contains("\"identityIndex\":0"));
        assert!(json.contains("\"alias\":\"My Identity\""));
        assert!(json.contains("\"masterKeyType\":\"ECDSA_HASH160\""));
        assert!(json.contains("\"useDefaultKeys\":true"));
        assert!(json.contains("\"fundingMethod\":{"));
    }

    #[test]
    fn register_identity_input_roundtrip() {
        let json = r#"{
            "walletSeedHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "identityIndex": 3,
            "alias": "Test",
            "masterKeyType": "ECDSA_HASH160",
            "keySpecs": [
                {
                    "keyType": "ECDSA_HASH160",
                    "purpose": "AUTHENTICATION",
                    "securityLevel": "CRITICAL",
                    "contractBounds": null
                }
            ],
            "useDefaultKeys": false,
            "fundingMethod": {
                "method": "fundWithWallet",
                "amountDuffs": 50000
            }
        }"#;
        let input: RegisterIdentityInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.identity_index, 3);
        assert_eq!(input.alias, "Test");
        assert!(!input.use_default_keys);
        assert_eq!(input.key_specs.len(), 1);
        assert_eq!(input.key_specs[0].key_type, "ECDSA_HASH160");
    }

    #[test]
    fn top_up_identity_input_serializes() {
        let input = TopUpIdentityInput {
            identity_id: "abc123".into(),
            wallet_seed_hash: "bb".repeat(32),
            identity_index: 1,
            funding_method: TopUpIdentityFundingMethodDto::FundWithWallet {
                amount_duffs: 75000,
                top_up_index: 0,
            },
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"identityIndex\":1"));
        assert!(json.contains("\"method\":\"fundWithWallet\""));
        assert!(json.contains("\"amountDuffs\":75000"));
        assert!(json.contains("\"topUpIndex\":0"));
    }

    #[test]
    fn top_up_from_platform_addresses_input_serializes() {
        let input = TopUpIdentityFromPlatformAddressesInput {
            identity_id: "def456".into(),
            wallet_seed_hash: "cc".repeat(32),
            inputs: vec![PlatformAddressCreditsPair {
                address: "XpAddr".into(),
                amount: 500000,
            }],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"def456\""));
        assert!(json.contains("\"walletSeedHash\""));
        assert!(json.contains("\"address\":\"XpAddr\""));
        assert!(json.contains("\"amount\":500000"));
    }

    #[test]
    fn transfer_to_addresses_input_serializes() {
        let input = TransferToAddressesInput {
            identity_id: "abc123".into(),
            outputs: vec![
                PlatformAddressCreditsPair {
                    address: "XpAddr1".into(),
                    amount: 100000,
                },
                PlatformAddressCreditsPair {
                    address: "XpAddr2".into(),
                    amount: 200000,
                },
            ],
            key_id: Some(5),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"keyId\":5"));
        assert!(json.contains("\"XpAddr1\""));
        assert!(json.contains("\"XpAddr2\""));
    }

    #[test]
    fn transfer_to_addresses_input_without_key_id() {
        let input = TransferToAddressesInput {
            identity_id: "abc123".into(),
            outputs: vec![],
            key_id: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyId\":null"));
    }

    #[test]
    fn platform_address_credits_pair_serializes() {
        let pair = PlatformAddressCreditsPair {
            address: "XpYvN123".into(),
            amount: 42000,
        };
        let json = serde_json::to_string(&pair).unwrap();
        assert!(json.contains("\"address\":\"XpYvN123\""));
        assert!(json.contains("\"amount\":42000"));
    }

    #[test]
    fn top_up_funding_use_asset_lock_serializes() {
        let method = TopUpIdentityFundingMethodDto::UseAssetLock {
            asset_lock_proof_hex: "aabb".into(),
            transaction_hex: "ccdd".into(),
            address: "XpAddr".into(),
        };
        let json = serde_json::to_string(&method).unwrap();
        assert!(json.contains("\"method\":\"useAssetLock\""));
        assert!(json.contains("\"assetLockProofHex\":\"aabb\""));
    }

    #[test]
    fn top_up_funding_fund_with_utxo_serializes() {
        let method = TopUpIdentityFundingMethodDto::FundWithUtxo {
            txid: "bb".repeat(32),
            vout: 2,
            value: 50000,
            script_pub_key_hex: "76a914".into(),
            address: "XpAddr".into(),
            top_up_index: 1,
        };
        let json = serde_json::to_string(&method).unwrap();
        assert!(json.contains("\"method\":\"fundWithUtxo\""));
        assert!(json.contains("\"vout\":2"));
        assert!(json.contains("\"topUpIndex\":1"));
    }

    #[test]
    fn sign_message_input_serializes_with_camel_case() {
        let input = SignMessageInput {
            identity_id: "abc123".into(),
            key_id: 2,
            message: "Hello, Dash!".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"keyId\":2"));
        assert!(json.contains("\"message\":\"Hello, Dash!\""));
    }

    #[test]
    fn sign_message_input_roundtrip() {
        let input = SignMessageInput {
            identity_id: "ff".repeat(32),
            key_id: 0,
            message: "test message".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: SignMessageInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.identity_id, input.identity_id);
        assert_eq!(parsed.key_id, input.key_id);
        assert_eq!(parsed.message, input.message);
    }
}
