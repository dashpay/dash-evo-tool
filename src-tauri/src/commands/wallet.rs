//! Wallet-related Tauri IPC commands.
//!
//! Maps all 6 `WalletTask` variants plus direct database/context methods
//! for wallet management to Tauri commands. Handles HD wallets, single-key
//! wallets, platform addresses, SPV management, and wallet lifecycle.

use crate::dto::common::{CreditsDto, SingleKeyHashDto, WalletSeedHashDto};
use crate::dto::wallet::{
    AssetLockDto, PlatformAddressDto, SingleKeyWalletDto, WalletAddressDto, WalletDto,
    WalletListDto, WalletRefDto, WalletTransactionDto,
};
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::model::wallet::single_key::SingleKeyWallet;
use dash_evo_tool::model::wallet::Wallet;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

use super::core::PlatformSyncModeDto;

// ---------------------------------------------------------------------------
// Input DTOs
// ---------------------------------------------------------------------------

/// Input for generating a receive address.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenerateReceiveAddressInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
}

/// Input for fetching platform address balances.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchPlatformAddressBalancesInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Sync mode to use.
    pub sync_mode: PlatformSyncModeDto,
}

/// A platform address amount pair for transfer inputs/outputs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlatformAddressAmountDto {
    /// The address string.
    pub address: String,
    /// Amount in credits.
    pub amount: CreditsDto,
}

/// Input for transferring credits between platform addresses.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlatformCreditsInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Source addresses with amounts to transfer.
    pub inputs: Vec<PlatformAddressAmountDto>,
    /// Destination addresses with amounts.
    pub outputs: Vec<PlatformAddressAmountDto>,
    /// Index of the input to deduct fees from (in sorted order).
    pub fee_payer_index: u16,
}

/// Input for funding a platform address from an existing asset lock.
///
/// Uses an index into the wallet's `unused_asset_locks` array rather than
/// passing the full `AssetLockProof` across IPC (proofs are large and complex).
/// The frontend obtains the asset lock list via `wallet_list_all` or
/// `wallet_get_hd` and displays them in a dropdown.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FundPlatformFromAssetLockInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Index into the wallet's `unused_asset_locks` array.
    pub asset_lock_index: usize,
    /// Destination platform address (Bech32m format: tevo1.../evo1... or base58).
    pub destination_address: String,
    /// Optional amount in credits. `None` means use the full asset lock amount.
    pub amount: Option<CreditsDto>,
}

/// Input for withdrawing from a platform address to Core.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawFromPlatformAddressInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Platform addresses and amounts to withdraw.
    pub inputs: Vec<PlatformAddressAmountDto>,
    /// Core script (hex) to receive the withdrawal.
    pub output_script_hex: String,
    /// Core fee per byte.
    pub core_fee_per_byte: u32,
    /// Index of the input to deduct fees from.
    pub fee_payer_index: u16,
}

/// Input for funding a platform address from wallet UTXOs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FundPlatformAddressFromWalletUtxosInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Amount in duffs to lock.
    pub amount: u64,
    /// Destination platform address.
    pub destination: String,
    /// Whether fees are deducted from the output amount.
    pub fee_deduct_from_output: bool,
}

/// Input for setting a wallet alias.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetWalletAliasInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// New alias (None to clear).
    pub alias: Option<String>,
}

/// Input for setting a single-key wallet alias.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetSingleKeyWalletAliasInput {
    /// Key hash (hex).
    pub key_hash: SingleKeyHashDto,
    /// New alias (None to clear).
    pub alias: Option<String>,
}

/// Input for removing a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWalletInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
}

/// Input for removing a single-key wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSingleKeyWalletInput {
    /// Key hash (hex).
    pub key_hash: SingleKeyHashDto,
}

/// Input for selecting a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SelectWalletInput {
    /// Which wallet to select: HD (by seed hash), SingleKey (by key hash), or None (deselect).
    pub selected: Option<WalletRefDto>,
}

/// Input for creating a new HD wallet.
///
/// The mnemonic is generated client-side (including entropy gathering from the user)
/// and passed to this command. The backend handles: seed derivation, optional
/// encryption, key derivation, database persistence, in-memory registration,
/// address bootstrapping, and SPV loading.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateWalletInput {
    /// BIP39 mnemonic phrase (space-separated words).
    pub mnemonic: String,
    /// Optional password to encrypt the wallet seed (empty string = no encryption).
    pub password: String,
    /// Optional alias / display name for the wallet (max 64 chars).
    /// If empty, auto-generates "Wallet N".
    pub alias: String,
    /// Whether to also set this password as the application main password.
    pub use_password_for_app: bool,
}

/// Input for importing an HD wallet from an existing BIP39 mnemonic.
///
/// Unlike `CreateWalletInput`, this does NOT pre-derive the first receive address
/// (address maps start empty, populated during bootstrap). It also supports
/// optional identity auto-discovery via `identity_scan_count`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportMnemonicInput {
    /// BIP39 mnemonic phrase (space-separated words, 12/15/18/21/24).
    pub mnemonic: String,
    /// Optional password to encrypt the wallet seed (empty string = no encryption).
    pub password: String,
    /// Optional alias / display name for the wallet (max 64 chars).
    /// If empty, auto-generates "Wallet N".
    pub alias: String,
    /// Whether to also set this password as the application main password.
    pub use_password_for_app: bool,
    /// Number of identity indices to scan (0 = skip identity discovery).
    /// When > 0, the backend will check indices 0..identity_scan_count-1
    /// for existing identities on the network (useful for mobile recovery).
    pub identity_scan_count: u32,
}

/// Input for importing a single-key wallet from a private key.
///
/// Supports WIF-encoded private keys (51-52 chars, network auto-detected)
/// and hex-encoded raw private keys (64 hex chars, uses active network).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportPrivateKeyInput {
    /// Private key string (WIF format or 64-char hex).
    pub private_key: String,
    /// Optional password to encrypt the private key (empty string = no encryption).
    pub password: String,
    /// Optional alias / display name (max 64 chars).
    /// If empty, auto-generates "Key N".
    pub alias: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_wallet_seed_hash(hex_str: &str) -> Result<[u8; 32], String> {
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

fn parse_single_key_hash(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid single-key hash hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Single-key hash must be 32 bytes, got {}",
            bytes.len()
        ));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

/// Convert an HD `Wallet` to its DTO representation (no private key data).
fn wallet_to_dto(wallet: &Wallet) -> WalletDto {
    let seed_hash_hex = hex::encode(wallet.seed_hash());

    // Build address list from watched_addresses (which have AddressInfo with path info)
    // and look up balances from address_balances.
    let addresses: Vec<WalletAddressDto> = wallet
        .watched_addresses
        .iter()
        .map(|(path, info)| {
            let balance = wallet
                .address_balances
                .get(&info.address)
                .copied()
                .unwrap_or(0);
            let total_received = wallet
                .address_total_received
                .get(&info.address)
                .copied()
                .unwrap_or(0);
            WalletAddressDto {
                address: info.address.to_string(),
                balance,
                total_received,
                derivation_path: format!("{}", path),
            }
        })
        .collect();

    let transactions: Vec<WalletTransactionDto> = wallet
        .transactions
        .iter()
        .map(|tx| WalletTransactionDto {
            txid: format!("{}", tx.txid),
            timestamp: tx.timestamp,
            height: tx.height,
            block_hash: tx.block_hash.map(|h| format!("{}", h)),
            net_amount: tx.net_amount,
            fee: tx.fee,
            label: tx.label.clone(),
            is_ours: tx.is_ours,
        })
        .collect();

    // unused_asset_locks is Vec<(Transaction, Address, Credits, Option<InstantLock>, Option<AssetLockProof>)>
    let unused_asset_locks: Vec<AssetLockDto> = wallet
        .unused_asset_locks
        .iter()
        .map(
            |(tx, addr, credits, instant_lock, asset_lock_proof)| AssetLockDto {
                txid: format!("{}", tx.txid()),
                address: addr.to_string(),
                amount: *credits,
                has_instant_lock: instant_lock.is_some(),
                has_asset_lock_proof: asset_lock_proof.is_some(),
            },
        )
        .collect();

    let platform_addresses: Vec<PlatformAddressDto> = wallet
        .platform_address_info
        .iter()
        .map(|(addr, info)| PlatformAddressDto {
            address: addr.to_string(),
            balance: info.balance,
            nonce: info.nonce as u64,
        })
        .collect();

    // Identity indexes from the identities map
    let identity_indexes: Vec<u32> = wallet.identities.keys().copied().collect();

    WalletDto {
        seed_hash: seed_hash_hex,
        uses_password: wallet.uses_password,
        alias: wallet.alias.clone(),
        is_main: wallet.is_main,
        confirmed_balance: wallet.confirmed_balance,
        unconfirmed_balance: wallet.unconfirmed_balance,
        total_balance: wallet.max_balance(),
        addresses,
        transactions,
        unused_asset_locks,
        platform_addresses,
        identity_indexes,
        password_hint: wallet.password_hint().clone(),
    }
}

/// Convert a `SingleKeyWallet` to its DTO representation.
fn single_key_wallet_to_dto(wallet: &SingleKeyWallet) -> SingleKeyWalletDto {
    SingleKeyWalletDto {
        key_hash: hex::encode(wallet.key_hash),
        uses_password: wallet.uses_password,
        public_key: wallet.public_key.to_string(),
        address: wallet.address.to_string(),
        alias: wallet.alias.clone(),
        confirmed_balance: wallet.confirmed_balance,
        unconfirmed_balance: wallet.unconfirmed_balance,
        total_balance: wallet.total_balance,
        utxo_count: wallet.utxos.len(),
    }
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Generate a new receive address for an HD wallet.
///
/// Dispatches `WalletTask::GenerateReceiveAddress`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn wallet_generate_receive_address(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: GenerateReceiveAddressInput,
) -> Result<DispatchTaskResponse, String> {
    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    // Verify the wallet exists
    let ctx = state.current_context();
    ctx.wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", input.wallet_seed_hash))?;

    let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch platform address balances for a wallet.
///
/// Dispatches `WalletTask::FetchPlatformAddressBalances`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn wallet_fetch_platform_address_balances(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchPlatformAddressBalancesInput,
) -> Result<DispatchTaskResponse, String> {
    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    ctx.wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", input.wallet_seed_hash))?;

    let sync_mode = input.sync_mode.to_backend();
    let task = BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances {
        seed_hash,
        sync_mode,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Transfer credits between platform addresses.
///
/// Dispatches `WalletTask::TransferPlatformCredits`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn wallet_transfer_platform_credits(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TransferPlatformCreditsInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::address_funds::PlatformAddress;
    use dash_sdk::dpp::fee::Credits;
    use std::collections::BTreeMap;

    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    ctx.wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", input.wallet_seed_hash))?;

    let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
    for item in &input.inputs {
        let addr = item
            .address
            .parse::<dash_sdk::dpp::dashcore::Address<dash_sdk::dpp::dashcore::address::NetworkUnchecked>>()
            .map_err(|e| format!("Invalid input address {}: {e}", item.address))?
            .assume_checked();
        let platform_addr = PlatformAddress::try_from(addr)
            .map_err(|e| format!("Invalid platform address {}: {e}", item.address))?;
        inputs.insert(platform_addr, item.amount);
    }

    let mut outputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
    for item in &input.outputs {
        let addr = item
            .address
            .parse::<dash_sdk::dpp::dashcore::Address<dash_sdk::dpp::dashcore::address::NetworkUnchecked>>()
            .map_err(|e| format!("Invalid output address {}: {e}", item.address))?
            .assume_checked();
        let platform_addr = PlatformAddress::try_from(addr)
            .map_err(|e| format!("Invalid platform address {}: {e}", item.address))?;
        outputs.insert(platform_addr, item.amount);
    }

    let task = BackendTask::WalletTask(WalletTask::TransferPlatformCredits {
        seed_hash,
        inputs,
        outputs,
        fee_payer_index: input.fee_payer_index,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Withdraw from platform addresses to a Core script.
///
/// Dispatches `WalletTask::WithdrawFromPlatformAddress`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn wallet_withdraw_from_platform_address(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: WithdrawFromPlatformAddressInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::address_funds::PlatformAddress;
    use dash_sdk::dpp::fee::Credits;
    use dash_sdk::dpp::identity::core_script::CoreScript;
    use std::collections::BTreeMap;

    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    ctx.wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", input.wallet_seed_hash))?;

    let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
    for item in &input.inputs {
        let addr = item
            .address
            .parse::<dash_sdk::dpp::dashcore::Address<dash_sdk::dpp::dashcore::address::NetworkUnchecked>>()
            .map_err(|e| format!("Invalid input address {}: {e}", item.address))?
            .assume_checked();
        let platform_addr = PlatformAddress::try_from(addr)
            .map_err(|e| format!("Invalid platform address {}: {e}", item.address))?;
        inputs.insert(platform_addr, item.amount);
    }

    let script_bytes = hex::decode(&input.output_script_hex)
        .map_err(|e| format!("Invalid output script hex: {e}"))?;
    let output_script = CoreScript::from_bytes(script_bytes);

    let task = BackendTask::WalletTask(WalletTask::WithdrawFromPlatformAddress {
        seed_hash,
        inputs,
        output_script,
        core_fee_per_byte: input.core_fee_per_byte,
        fee_payer_index: input.fee_payer_index,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fund a platform address from wallet UTXOs.
///
/// Dispatches `WalletTask::FundPlatformAddressFromWalletUtxos`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn wallet_fund_platform_address_from_utxos(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FundPlatformAddressFromWalletUtxosInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::address_funds::PlatformAddress;

    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    ctx.wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", input.wallet_seed_hash))?;

    let dest_addr = input
        .destination
        .parse::<dash_sdk::dpp::dashcore::Address<dash_sdk::dpp::dashcore::address::NetworkUnchecked>>()
        .map_err(|e| format!("Invalid destination address: {e}"))?
        .assume_checked();
    let destination = PlatformAddress::try_from(dest_addr)
        .map_err(|e| format!("Invalid platform address: {e}"))?;

    let task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
        seed_hash,
        amount: input.amount,
        destination,
        fee_deduct_from_output: input.fee_deduct_from_output,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fund a platform address from an existing asset lock.
///
/// Dispatches `WalletTask::FundPlatformAddressFromAssetLock`. The asset lock
/// proof is looked up from the wallet's in-memory state using the provided
/// index. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn wallet_fund_platform_from_asset_lock(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FundPlatformFromAssetLockInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::address_funds::PlatformAddress;
    use dash_sdk::dpp::dashcore::address::NetworkUnchecked;
    use std::collections::BTreeMap;

    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", input.wallet_seed_hash))?;

    // Read the asset lock proof and address from wallet state
    let (asset_lock_proof, asset_lock_address) = {
        let wallet = wallet_arc
            .read()
            .map_err(|e| format!("Failed to read wallet: {e}"))?;
        let asset_lock = wallet
            .unused_asset_locks
            .get(input.asset_lock_index)
            .ok_or_else(|| {
                format!(
                    "Asset lock index {} out of range (wallet has {} asset locks)",
                    input.asset_lock_index,
                    wallet.unused_asset_locks.len()
                )
            })?;
        let (_, addr, _, _, proof_opt) = asset_lock;
        let proof = proof_opt
            .as_ref()
            .ok_or_else(|| "Asset lock proof not yet available for this lock".to_string())?;
        (Box::new(proof.clone()), addr.clone())
    };

    // Parse the destination platform address
    let platform_addr = if input.destination_address.starts_with("evo1")
        || input.destination_address.starts_with("tevo1")
    {
        let (addr, _network) = PlatformAddress::from_bech32m_string(&input.destination_address)
            .map_err(|e| format!("Invalid Bech32m address: {e}"))?;
        addr
    } else {
        let addr = input
            .destination_address
            .parse::<dash_sdk::dpp::dashcore::Address<NetworkUnchecked>>()
            .map_err(|e| format!("Invalid address {}: {e}", input.destination_address))?
            .assume_checked();
        PlatformAddress::try_from(addr).map_err(|e| format!("Invalid platform address: {e}"))?
    };

    // Build outputs map
    let mut outputs: BTreeMap<PlatformAddress, Option<u64>> = BTreeMap::new();
    outputs.insert(platform_addr, input.amount);

    let task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromAssetLock {
        seed_hash,
        asset_lock_proof,
        asset_lock_address,
        outputs,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Wallet creation commands
// ---------------------------------------------------------------------------

/// Create a new HD wallet from a BIP39 mnemonic.
///
/// This replicates the full wallet creation flow from `AddNewWalletScreen::save_wallet()`:
/// 1. Parse and validate the mnemonic
/// 2. Derive seed from mnemonic
/// 3. Optionally encrypt seed with password (AES-256-GCM + Argon2)
/// 4. Optionally set app main password
/// 5. Derive master BIP44 ECDSA extended public key
/// 6. Compute seed hash (SHA-256)
/// 7. Derive first receive address (m/44'/coin'/0'/0/0)
/// 8. Create Wallet struct and persist to database
/// 9. Register in-memory and set as pending selection
/// 10. Save first address to database
/// 11. Bootstrap wallet addresses and start SPV if applicable
///
/// Returns the created wallet as a DTO.
#[tauri::command]
#[specta::specta]
pub fn wallet_create(
    state: tauri::State<'_, Arc<AppState>>,
    input: CreateWalletInput,
) -> Result<WalletDto, String> {
    use dash_evo_tool::model::wallet::encryption::{encrypt_message, DASH_SECRET_MESSAGE};
    use dash_evo_tool::model::wallet::{
        AddressInfo as WalletAddressInfo, ClosedKeyItem, DerivationPathReference,
        DerivationPathType, OpenWalletSeed, WalletSeed,
    };
    use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
    use dash_sdk::dpp::dashcore::{Address, Network};
    use dash_sdk::dpp::key_wallet::bip32::{
        ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
    };
    use std::collections::BTreeMap;

    // BIP44 account 0 paths
    let bip44_account_0_mainnet: [ChildNumber; 3] = [
        ChildNumber::Hardened { index: 44 },
        ChildNumber::Hardened { index: 5 },
        ChildNumber::Hardened { index: 0 },
    ];
    let bip44_account_0_testnet: [ChildNumber; 3] = [
        ChildNumber::Hardened { index: 44 },
        ChildNumber::Hardened { index: 1 },
        ChildNumber::Hardened { index: 0 },
    ];

    // 1. Parse and validate mnemonic
    let mnemonic: bip39::Mnemonic = input
        .mnemonic
        .parse()
        .map_err(|e| format!("Invalid mnemonic: {e}"))?;

    // 2. Derive seed
    let seed = mnemonic.to_seed("");

    // 3. Encrypt seed if password provided
    let (encrypted_seed, salt, nonce, uses_password) = if input.password.is_empty() {
        (seed.to_vec(), vec![], vec![], false)
    } else {
        let (encrypted_seed, salt, nonce) = ClosedKeyItem::encrypt_seed(&seed, &input.password)?;
        (encrypted_seed, salt, nonce, true)
    };

    let ctx = state.current_context();
    let network = ctx.network();

    // 4. Set app main password if requested
    if uses_password && input.use_password_for_app {
        let (encrypted_message, pw_salt, pw_nonce) =
            encrypt_message(DASH_SECRET_MESSAGE, &input.password)?;
        ctx.update_main_password(&pw_salt, &pw_nonce, &encrypted_message)
            .map_err(|e| format!("Failed to set app password: {e}"))?;
    }

    // 5. Derive master BIP44 ECDSA extended public key
    let master_ecdsa_extended_private_key = ExtendedPrivKey::new_master(network, &seed)
        .map_err(|e| format!("Failed to create master key: {e}"))?;

    let bip44_root = match network {
        Network::Dash => &bip44_account_0_mainnet,
        _ => &bip44_account_0_testnet,
    };
    let bip44_root_derivation_path = DerivationPath::from(bip44_root.as_slice());

    let secp = Secp256k1::new();
    let derived_priv = master_ecdsa_extended_private_key
        .derive_priv(&secp, &bip44_root_derivation_path)
        .map_err(|e| format!("Failed to derive BIP44 key: {e}"))?;
    let master_bip44_ecdsa_extended_public_key = ExtendedPubKey::from_priv(&secp, &derived_priv);

    // 6. Compute seed hash
    let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);

    // 7. Derive first receive address (m/44'/coin'/0'/0/0)
    let address_path_ext = DerivationPath::from(
        [
            ChildNumber::Normal { index: 0 }, // receive (not change)
            ChildNumber::Normal { index: 0 }, // first address
        ]
        .as_slice(),
    );
    let first_address = master_bip44_ecdsa_extended_public_key
        .derive_pub(&secp, &address_path_ext)
        .ok()
        .map(|pk| Address::p2pkh(&pk.to_pub(), network));

    // Build known_addresses and watched_addresses
    let mut known_addresses = BTreeMap::new();
    let mut watched_addresses = BTreeMap::new();

    if let Some(ref address) = first_address {
        let full_derivation_path = DerivationPath::from(
            [
                bip44_root[0],
                bip44_root[1],
                bip44_root[2],
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index: 0 },
            ]
            .as_slice(),
        );
        known_addresses.insert(address.clone(), full_derivation_path.clone());
        watched_addresses.insert(
            full_derivation_path,
            WalletAddressInfo {
                address: address.clone(),
                path_type: DerivationPathType::CLEAR_FUNDS,
                path_reference: DerivationPathReference::BIP44,
            },
        );
    }

    // 8. Generate wallet alias
    let trimmed_alias = input.alias.trim();
    let wallet_alias = if trimmed_alias.is_empty() {
        let existing_count = ctx.loaded_wallets().len();
        format!("Wallet {}", existing_count + 1)
    } else {
        trimmed_alias.chars().take(64).collect()
    };

    // 9. Create Wallet struct
    let wallet = Wallet {
        wallet_seed: WalletSeed::Open(OpenWalletSeed {
            seed,
            wallet_info: ClosedKeyItem {
                seed_hash,
                encrypted_seed,
                salt,
                nonce,
                password_hint: None,
            },
        }),
        uses_password,
        master_bip44_ecdsa_extended_public_key,
        address_balances: Default::default(),
        address_total_received: Default::default(),
        known_addresses,
        watched_addresses,
        unused_asset_locks: Default::default(),
        alias: Some(wallet_alias),
        identities: Default::default(),
        utxos: Default::default(),
        transactions: Vec::new(),
        is_main: true,
        confirmed_balance: 0,
        unconfirmed_balance: 0,
        total_balance: 0,
        platform_address_info: Default::default(),
    };

    let new_seed_hash = wallet.seed_hash();

    // 10. Register wallet: persist to DB and add to in-memory map
    let wallet_arc = ctx.register_new_wallet(wallet)?;

    // 11. Save first address to database
    if let Some(ref address) = first_address {
        let full_derivation_path = DerivationPath::from(
            [
                bip44_root[0],
                bip44_root[1],
                bip44_root[2],
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index: 0 },
            ]
            .as_slice(),
        );
        let _ = ctx.save_address_if_not_exists(
            &new_seed_hash,
            address,
            &full_derivation_path,
            DerivationPathReference::BIP44,
            DerivationPathType::CLEAR_FUNDS,
        );
    }

    // 12. Bootstrap addresses and start SPV if applicable
    ctx.bootstrap_wallet_addresses(&wallet_arc);
    if ctx.core_backend_mode() == dash_evo_tool::spv::CoreBackendMode::Spv {
        ctx.handle_wallet_unlocked(&wallet_arc);
    }

    // Return DTO
    let guard = wallet_arc
        .read()
        .map_err(|e| format!("Failed to read wallet: {e}"))?;
    Ok(wallet_to_dto(&guard))
}

/// Import an HD wallet from an existing BIP39 mnemonic.
///
/// This replicates the import flow from `ImportMnemonicScreen::save_wallet()`:
/// 1. Parse and validate the mnemonic
/// 2. Derive seed from mnemonic
/// 3. Optionally encrypt seed with password (AES-256-GCM + Argon2)
/// 4. Optionally set app main password
/// 5. Derive master BIP44 ECDSA extended public key
/// 6. Compute seed hash (SHA-256)
/// 7. Create Wallet struct with EMPTY address maps (no pre-derived addresses)
/// 8. Persist to database (with duplicate detection)
/// 9. Register in-memory and set as pending selection
/// 10. Bootstrap wallet addresses and start SPV if applicable
/// 11. Queue identity discovery if identity_scan_count > 0
///
/// Key differences from `wallet_create`:
/// - Address maps start empty (no first address pre-derived)
/// - Supports identity auto-discovery via `identity_scan_count`
/// - Provides "already imported" error for duplicate wallets
///
/// Returns the imported wallet as a DTO.
#[tauri::command]
#[specta::specta]
pub fn wallet_import_mnemonic(
    state: tauri::State<'_, Arc<AppState>>,
    input: ImportMnemonicInput,
) -> Result<WalletDto, String> {
    use dash_evo_tool::model::wallet::encryption::{encrypt_message, DASH_SECRET_MESSAGE};
    use dash_evo_tool::model::wallet::{ClosedKeyItem, OpenWalletSeed, WalletSeed};
    use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::key_wallet::bip32::{
        ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
    };

    // BIP44 account 0 paths
    let bip44_account_0_mainnet: [ChildNumber; 3] = [
        ChildNumber::Hardened { index: 44 },
        ChildNumber::Hardened { index: 5 },
        ChildNumber::Hardened { index: 0 },
    ];
    let bip44_account_0_testnet: [ChildNumber; 3] = [
        ChildNumber::Hardened { index: 44 },
        ChildNumber::Hardened { index: 1 },
        ChildNumber::Hardened { index: 0 },
    ];

    // 1. Parse and validate mnemonic
    let mnemonic: bip39::Mnemonic = input
        .mnemonic
        .parse()
        .map_err(|e| format!("Invalid mnemonic: {e}"))?;

    // 2. Derive seed
    let seed = mnemonic.to_seed("");

    // 3. Encrypt seed if password provided
    let (encrypted_seed, salt, nonce, uses_password) = if input.password.is_empty() {
        (seed.to_vec(), vec![], vec![], false)
    } else {
        let (encrypted_seed, salt, nonce) = ClosedKeyItem::encrypt_seed(&seed, &input.password)?;
        (encrypted_seed, salt, nonce, true)
    };

    let ctx = state.current_context();
    let network = ctx.network();

    // 4. Set app main password if requested
    if uses_password && input.use_password_for_app {
        let (encrypted_message, pw_salt, pw_nonce) =
            encrypt_message(DASH_SECRET_MESSAGE, &input.password)?;
        ctx.update_main_password(&pw_salt, &pw_nonce, &encrypted_message)
            .map_err(|e| format!("Failed to set app password: {e}"))?;
    }

    // 5. Derive master BIP44 ECDSA extended public key
    let master_ecdsa_extended_private_key = ExtendedPrivKey::new_master(network, &seed)
        .map_err(|e| format!("Failed to create master key: {e}"))?;

    let bip44_root = match network {
        Network::Dash => &bip44_account_0_mainnet,
        _ => &bip44_account_0_testnet,
    };
    let bip44_root_derivation_path = DerivationPath::from(bip44_root.as_slice());

    let secp = Secp256k1::new();
    let derived_priv = master_ecdsa_extended_private_key
        .derive_priv(&secp, &bip44_root_derivation_path)
        .map_err(|e| format!("Failed to derive BIP44 key: {e}"))?;
    let master_bip44_ecdsa_extended_public_key = ExtendedPubKey::from_priv(&secp, &derived_priv);

    // 6. Compute seed hash
    let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);

    // 7. Generate wallet alias
    let trimmed_alias = input.alias.trim();
    let wallet_alias = if trimmed_alias.is_empty() {
        let existing_count = ctx.loaded_wallets().len();
        format!("Wallet {}", existing_count + 1)
    } else {
        trimmed_alias.chars().take(64).collect()
    };

    // 8. Create Wallet struct with EMPTY address maps (key difference from create)
    let wallet = Wallet {
        wallet_seed: WalletSeed::Open(OpenWalletSeed {
            seed,
            wallet_info: ClosedKeyItem {
                seed_hash,
                encrypted_seed,
                salt,
                nonce,
                password_hint: None,
            },
        }),
        uses_password,
        master_bip44_ecdsa_extended_public_key,
        address_balances: Default::default(),
        address_total_received: Default::default(),
        known_addresses: Default::default(), // Empty — differs from wallet_create
        watched_addresses: Default::default(), // Empty — differs from wallet_create
        unused_asset_locks: Default::default(),
        alias: Some(wallet_alias),
        identities: Default::default(),
        utxos: Default::default(),
        transactions: Vec::new(),
        is_main: true,
        confirmed_balance: 0,
        unconfirmed_balance: 0,
        total_balance: 0,
        platform_address_info: Default::default(),
    };

    // 9. Register wallet: persist to DB and add to in-memory map
    // Uses descriptive "already imported" error for UNIQUE constraint violations
    let wallet_arc = ctx.register_new_wallet(wallet).map_err(|e| {
        if e.contains("UNIQUE constraint failed") {
            "This wallet has already been imported for another network. Each wallet can only be imported once per network. If you want to use this wallet on a different network, please switch networks first.".to_string()
        } else {
            e
        }
    })?;

    // 10. Bootstrap addresses and start SPV if applicable
    ctx.bootstrap_wallet_addresses(&wallet_arc);
    if ctx.core_backend_mode() == dash_evo_tool::spv::CoreBackendMode::Spv {
        ctx.handle_wallet_unlocked(&wallet_arc);
    }

    // 11. Queue identity discovery if requested
    if input.identity_scan_count > 0 {
        ctx.queue_wallet_identity_discovery(&wallet_arc, input.identity_scan_count - 1);
    }

    // Return DTO
    let guard = wallet_arc
        .read()
        .map_err(|e| format!("Failed to read wallet: {e}"))?;
    Ok(wallet_to_dto(&guard))
}

/// Import a single-key wallet from a private key (WIF or hex).
///
/// This replicates the import flow from `ImportMnemonicScreen::save_private_key_wallet()`:
/// 1. Parse private key (WIF format auto-detects network; hex uses active network)
/// 2. Create SingleKeyWallet with optional password encryption
/// 3. Persist to database (with duplicate detection)
/// 4. Register in-memory
///
/// Returns the imported single-key wallet as a DTO.
#[tauri::command]
#[specta::specta]
pub fn wallet_import_private_key(
    state: tauri::State<'_, Arc<AppState>>,
    input: ImportPrivateKeyInput,
) -> Result<SingleKeyWalletDto, String> {
    let trimmed_key = input.private_key.trim();
    if trimmed_key.is_empty() {
        return Err("Please enter a private key".to_string());
    }

    let password = if input.password.is_empty() {
        None
    } else {
        Some(input.password.as_str())
    };

    let ctx = state.current_context();
    let network = ctx.network();

    // Generate alias
    let trimmed_alias = input.alias.trim();
    let alias = if trimmed_alias.is_empty() {
        let existing_count = ctx.loaded_single_key_wallets().len();
        Some(format!("Key {}", existing_count + 1))
    } else {
        Some(trimmed_alias.chars().take(64).collect())
    };

    // Try WIF first (auto-detects network), then hex (uses active network)
    let wallet = SingleKeyWallet::from_wif(trimmed_key, password, alias.clone())
        .or_else(|_| SingleKeyWallet::from_hex(trimmed_key, network, password, alias))?;

    // Register: persist to DB and add to in-memory map
    let wallet_arc = ctx.register_new_single_key_wallet(wallet)?;

    // Return DTO
    let guard = wallet_arc
        .read()
        .map_err(|e| format!("Failed to read wallet: {e}"))?;
    Ok(single_key_wallet_to_dto(&guard))
}

// ---------------------------------------------------------------------------
// Direct read/write commands (synchronous)
// ---------------------------------------------------------------------------

/// Get all loaded wallets (HD + single-key) with current state.
#[tauri::command]
#[specta::specta]
pub fn wallet_list_all(state: tauri::State<'_, Arc<AppState>>) -> Result<WalletListDto, String> {
    let ctx = state.current_context();

    let hd_wallets: Vec<WalletDto> = ctx
        .loaded_wallets()
        .iter()
        .filter_map(|(_, arc)| {
            let guard = arc.read().ok()?;
            Some(wallet_to_dto(&guard))
        })
        .collect();

    let single_key_wallets: Vec<SingleKeyWalletDto> = ctx
        .loaded_single_key_wallets()
        .iter()
        .filter_map(|(_, arc)| {
            let guard = arc.read().ok()?;
            Some(single_key_wallet_to_dto(&guard))
        })
        .collect();

    // Determine which wallet is selected
    let selected = ctx
        .selected_wallet_hash()
        .map(|seed_hash| WalletRefDto::Hd {
            seed_hash: hex::encode(seed_hash),
        })
        .or_else(|| {
            ctx.selected_single_key_hash()
                .map(|key_hash| WalletRefDto::SingleKey {
                    key_hash: hex::encode(key_hash),
                })
        });

    Ok(WalletListDto {
        hd_wallets,
        single_key_wallets,
        selected,
    })
}

/// Get a single HD wallet by seed hash.
#[tauri::command]
#[specta::specta]
pub fn wallet_get_hd(
    state: tauri::State<'_, Arc<AppState>>,
    wallet_seed_hash: WalletSeedHashDto,
) -> Result<WalletDto, String> {
    let seed_hash = parse_wallet_seed_hash(&wallet_seed_hash)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", wallet_seed_hash))?;
    let guard = wallet_arc
        .read()
        .map_err(|e| format!("Failed to read wallet: {e}"))?;
    Ok(wallet_to_dto(&guard))
}

/// Get a single-key wallet by key hash.
#[tauri::command]
#[specta::specta]
pub fn wallet_get_single_key(
    state: tauri::State<'_, Arc<AppState>>,
    key_hash: SingleKeyHashDto,
) -> Result<SingleKeyWalletDto, String> {
    let hash = parse_single_key_hash(&key_hash)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .single_key_wallet_by_hash(&hash)
        .ok_or_else(|| format!("Single-key wallet not found for hash {}", key_hash))?;
    let guard = wallet_arc
        .read()
        .map_err(|e| format!("Failed to read wallet: {e}"))?;
    Ok(single_key_wallet_to_dto(&guard))
}

/// Select a wallet (HD or single-key) as the active wallet.
#[tauri::command]
#[specta::specta]
pub fn wallet_select(
    state: tauri::State<'_, Arc<AppState>>,
    input: SelectWalletInput,
) -> Result<(), String> {
    let ctx = state.current_context();
    match input.selected {
        Some(WalletRefDto::Hd { seed_hash }) => {
            let hash = parse_wallet_seed_hash(&seed_hash)?;
            ctx.set_selected_wallet_hash(Some(hash));
            ctx.set_selected_single_key_hash(None);
        }
        Some(WalletRefDto::SingleKey { key_hash }) => {
            let hash = parse_single_key_hash(&key_hash)?;
            ctx.set_selected_single_key_hash(Some(hash));
            ctx.set_selected_wallet_hash(None);
        }
        None => {
            ctx.set_selected_wallet_hash(None);
            ctx.set_selected_single_key_hash(None);
        }
    }
    Ok(())
}

/// Set the alias for an HD wallet.
#[tauri::command]
#[specta::specta]
pub fn wallet_set_alias(
    state: tauri::State<'_, Arc<AppState>>,
    input: SetWalletAliasInput,
) -> Result<(), String> {
    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| "Wallet not found".to_string())?;

    // Update in-memory
    {
        let mut guard = wallet_arc
            .write()
            .map_err(|e| format!("Failed to lock wallet: {e}"))?;
        guard.alias = input.alias.clone();
    }

    // Persist to database
    let db = state.db();
    db.set_wallet_alias(&seed_hash, input.alias)
        .map_err(|e| format!("Failed to persist alias: {e}"))
}

/// Set the alias for a single-key wallet.
#[tauri::command]
#[specta::specta]
pub fn wallet_set_single_key_alias(
    state: tauri::State<'_, Arc<AppState>>,
    input: SetSingleKeyWalletAliasInput,
) -> Result<(), String> {
    let key_hash = parse_single_key_hash(&input.key_hash)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .single_key_wallet_by_hash(&key_hash)
        .ok_or_else(|| "Single-key wallet not found".to_string())?;

    // Update in-memory
    {
        let mut guard = wallet_arc
            .write()
            .map_err(|e| format!("Failed to lock wallet: {e}"))?;
        guard.alias = input.alias.clone();
    }

    // Persist to database
    let db = state.db();
    db.update_single_key_wallet_alias(&key_hash, input.alias.as_deref())
        .map_err(|e| format!("Failed to persist alias: {e}"))
}

/// Remove an HD wallet from the application.
#[tauri::command]
#[specta::specta]
pub fn wallet_remove(
    state: tauri::State<'_, Arc<AppState>>,
    input: RemoveWalletInput,
) -> Result<(), String> {
    let seed_hash = parse_wallet_seed_hash(&input.wallet_seed_hash)?;
    let ctx = state.current_context();
    ctx.remove_wallet(&seed_hash)
}

/// Remove a single-key wallet from the application.
#[tauri::command]
#[specta::specta]
pub fn wallet_remove_single_key(
    state: tauri::State<'_, Arc<AppState>>,
    input: RemoveSingleKeyWalletInput,
) -> Result<(), String> {
    let key_hash = parse_single_key_hash(&input.key_hash)?;
    let ctx = state.current_context();
    ctx.remove_single_key_wallet(&key_hash)
}

/// Start SPV for the current network.
#[tauri::command]
#[specta::specta]
pub fn wallet_start_spv(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    let ctx = state.current_context().clone();
    ctx.start_spv()
}

/// Stop SPV for the current network.
#[tauri::command]
#[specta::specta]
pub fn wallet_stop_spv(state: tauri::State<'_, Arc<AppState>>) {
    let ctx = state.current_context();
    ctx.stop_spv();
}

/// Clear SPV data for the current network.
#[tauri::command]
#[specta::specta]
pub fn wallet_clear_spv_data(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    let ctx = state.current_context();
    ctx.clear_spv_data()
}

/// Bootstrap known addresses for a wallet.
///
/// Populates the wallet's `known_addresses` and `watched_addresses` maps
/// from derivation paths if they are empty. This is typically called
/// automatically during wallet creation/import, but can be invoked
/// manually if address maps need re-initialization.
#[tauri::command]
#[specta::specta]
pub fn wallet_bootstrap_addresses(
    state: tauri::State<'_, Arc<AppState>>,
    wallet_seed_hash: WalletSeedHashDto,
) -> Result<(), String> {
    let seed_hash = parse_wallet_seed_hash(&wallet_seed_hash)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", wallet_seed_hash))?;
    ctx.bootstrap_wallet_addresses(&wallet_arc);
    Ok(())
}

/// Notify the backend that a wallet has been unlocked (triggers SPV wallet load).
#[tauri::command]
#[specta::specta]
pub fn wallet_notify_unlocked(
    state: tauri::State<'_, Arc<AppState>>,
    wallet_seed_hash: WalletSeedHashDto,
) -> Result<(), String> {
    let seed_hash = parse_wallet_seed_hash(&wallet_seed_hash)?;
    let ctx = state.current_context().clone();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| "Wallet not found".to_string())?;
    ctx.handle_wallet_unlocked(&wallet_arc);
    Ok(())
}

/// Notify the backend that a wallet has been locked (triggers SPV wallet unload).
#[tauri::command]
#[specta::specta]
pub fn wallet_notify_locked(
    state: tauri::State<'_, Arc<AppState>>,
    wallet_seed_hash: WalletSeedHashDto,
) -> Result<(), String> {
    let seed_hash = parse_wallet_seed_hash(&wallet_seed_hash)?;
    let ctx = state.current_context().clone();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| "Wallet not found".to_string())?;
    ctx.handle_wallet_locked(&wallet_arc);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_receive_address_input_serializes() {
        let input = GenerateReceiveAddressInput {
            wallet_seed_hash: "aa".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
    }

    #[test]
    fn fetch_platform_balances_input_serializes() {
        let input = FetchPlatformAddressBalancesInput {
            wallet_seed_hash: "bb".repeat(32),
            sync_mode: PlatformSyncModeDto::ForceFull,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"syncMode\":\"forceFull\""));
    }

    #[test]
    fn platform_address_amount_dto_serializes() {
        let dto = PlatformAddressAmountDto {
            address: "XpYvN123".into(),
            amount: 50000,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"address\":\"XpYvN123\""));
        assert!(json.contains("\"amount\":50000"));
    }

    #[test]
    fn transfer_platform_credits_input_serializes() {
        let input = TransferPlatformCreditsInput {
            wallet_seed_hash: "cc".repeat(32),
            inputs: vec![PlatformAddressAmountDto {
                address: "addr1".into(),
                amount: 100000,
            }],
            outputs: vec![PlatformAddressAmountDto {
                address: "addr2".into(),
                amount: 100000,
            }],
            fee_payer_index: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"feePayerIndex\":0"));
    }

    #[test]
    fn withdraw_from_platform_input_serializes() {
        let input = WithdrawFromPlatformAddressInput {
            wallet_seed_hash: "dd".repeat(32),
            inputs: vec![PlatformAddressAmountDto {
                address: "addr1".into(),
                amount: 50000,
            }],
            output_script_hex: "76a914".into(),
            core_fee_per_byte: 1,
            fee_payer_index: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"outputScriptHex\":\"76a914\""));
        assert!(json.contains("\"coreFeePerByte\":1"));
    }

    #[test]
    fn fund_from_utxos_input_serializes() {
        let input = FundPlatformAddressFromWalletUtxosInput {
            wallet_seed_hash: "ee".repeat(32),
            amount: 1000000,
            destination: "XpYvN123".into(),
            fee_deduct_from_output: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"feeDeductFromOutput\":true"));
        assert!(json.contains("\"amount\":1000000"));
    }

    #[test]
    fn fund_from_asset_lock_input_serializes() {
        let input = FundPlatformFromAssetLockInput {
            wallet_seed_hash: "ff".repeat(32),
            asset_lock_index: 2,
            destination_address: "tevo1abc123".into(),
            amount: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
        assert!(json.contains("\"assetLockIndex\":2"));
        assert!(json.contains("\"destinationAddress\":\"tevo1abc123\""));
        assert!(json.contains("\"amount\":null"));
    }

    #[test]
    fn fund_from_asset_lock_input_with_amount_serializes() {
        let input = FundPlatformFromAssetLockInput {
            wallet_seed_hash: "ff".repeat(32),
            asset_lock_index: 0,
            destination_address: "XpYvN123".into(),
            amount: Some(500000),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"amount\":500000"));
        assert!(json.contains("\"assetLockIndex\":0"));
    }

    #[test]
    fn fund_from_asset_lock_input_roundtrip() {
        let json = r#"{"walletSeedHash":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","assetLockIndex":1,"destinationAddress":"tevo1xyz","amount":null}"#;
        let input: FundPlatformFromAssetLockInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.wallet_seed_hash, "ff".repeat(32));
        assert_eq!(input.asset_lock_index, 1);
        assert_eq!(input.destination_address, "tevo1xyz");
        assert!(input.amount.is_none());
    }

    #[test]
    fn set_wallet_alias_input_serializes() {
        let input = SetWalletAliasInput {
            wallet_seed_hash: "aa".repeat(32),
            alias: Some("My Wallet".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":\"My Wallet\""));
    }

    #[test]
    fn set_wallet_alias_none_serializes() {
        let input = SetWalletAliasInput {
            wallet_seed_hash: "aa".repeat(32),
            alias: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":null"));
    }

    #[test]
    fn set_single_key_alias_input_serializes() {
        let input = SetSingleKeyWalletAliasInput {
            key_hash: "bb".repeat(32),
            alias: Some("Single Key 1".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":\"Single Key 1\""));
    }

    #[test]
    fn remove_wallet_input_serializes() {
        let input = RemoveWalletInput {
            wallet_seed_hash: "cc".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
    }

    #[test]
    fn select_wallet_hd_serializes() {
        let input = SelectWalletInput {
            selected: Some(WalletRefDto::Hd {
                seed_hash: "aa".repeat(32),
            }),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"type\":\"hd\""));
        assert!(json.contains("\"seedHash\""));
    }

    #[test]
    fn select_wallet_single_key_serializes() {
        let input = SelectWalletInput {
            selected: Some(WalletRefDto::SingleKey {
                key_hash: "bb".repeat(32),
            }),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"type\":\"singleKey\""));
        assert!(json.contains("\"keyHash\""));
    }

    #[test]
    fn select_wallet_none_serializes() {
        let input = SelectWalletInput { selected: None };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"selected\":null"));
    }

    #[test]
    fn remove_single_key_input_serializes() {
        let input = RemoveSingleKeyWalletInput {
            key_hash: "dd".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyHash\""));
    }

    #[test]
    fn select_wallet_roundtrip() {
        let json = r#"{"selected":{"type":"hd","seedHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        let input: SelectWalletInput = serde_json::from_str(json).unwrap();
        match input.selected.unwrap() {
            WalletRefDto::Hd { seed_hash } => {
                assert_eq!(seed_hash, "aa".repeat(32));
            }
            _ => panic!("Expected HD variant"),
        }
    }

    #[test]
    fn create_wallet_input_serializes() {
        let input = CreateWalletInput {
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
            password: "mypassword".into(),
            alias: "My Wallet".into(),
            use_password_for_app: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"mnemonic\":\"abandon abandon"));
        assert!(json.contains("\"password\":\"mypassword\""));
        assert!(json.contains("\"alias\":\"My Wallet\""));
        assert!(json.contains("\"usePasswordForApp\":true"));
    }

    #[test]
    fn create_wallet_input_with_empty_password_serializes() {
        let input = CreateWalletInput {
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
            password: String::new(),
            alias: String::new(),
            use_password_for_app: false,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"password\":\"\""));
        assert!(json.contains("\"alias\":\"\""));
        assert!(json.contains("\"usePasswordForApp\":false"));
    }

    #[test]
    fn create_wallet_input_roundtrip() {
        let json = r#"{"mnemonic":"abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about","password":"secret","alias":"Test Wallet","usePasswordForApp":false}"#;
        let input: CreateWalletInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.mnemonic, "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
        assert_eq!(input.password, "secret");
        assert_eq!(input.alias, "Test Wallet");
        assert!(!input.use_password_for_app);
    }

    #[test]
    fn create_wallet_input_uses_camel_case() {
        let input = CreateWalletInput {
            mnemonic: "test".into(),
            password: "pw".into(),
            alias: "w".into(),
            use_password_for_app: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        // Verify camelCase field names
        assert!(!json.contains("use_password_for_app"));
        assert!(json.contains("usePasswordForApp"));
    }

    #[test]
    fn import_mnemonic_input_serializes() {
        let input = ImportMnemonicInput {
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
            password: "mypassword".into(),
            alias: "Imported Wallet".into(),
            use_password_for_app: false,
            identity_scan_count: 10,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"mnemonic\":\"abandon abandon"));
        assert!(json.contains("\"password\":\"mypassword\""));
        assert!(json.contains("\"alias\":\"Imported Wallet\""));
        assert!(json.contains("\"usePasswordForApp\":false"));
        assert!(json.contains("\"identityScanCount\":10"));
    }

    #[test]
    fn import_mnemonic_input_with_no_password_serializes() {
        let input = ImportMnemonicInput {
            mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong".into(),
            password: String::new(),
            alias: String::new(),
            use_password_for_app: false,
            identity_scan_count: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"password\":\"\""));
        assert!(json.contains("\"alias\":\"\""));
        assert!(json.contains("\"identityScanCount\":0"));
    }

    #[test]
    fn import_mnemonic_input_roundtrip() {
        let json = r#"{"mnemonic":"abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about","password":"secret","alias":"Test","usePasswordForApp":true,"identityScanCount":5}"#;
        let input: ImportMnemonicInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.mnemonic, "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
        assert_eq!(input.password, "secret");
        assert_eq!(input.alias, "Test");
        assert!(input.use_password_for_app);
        assert_eq!(input.identity_scan_count, 5);
    }

    #[test]
    fn import_mnemonic_input_uses_camel_case() {
        let input = ImportMnemonicInput {
            mnemonic: "test".into(),
            password: "pw".into(),
            alias: "w".into(),
            use_password_for_app: true,
            identity_scan_count: 10,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(!json.contains("use_password_for_app"));
        assert!(json.contains("usePasswordForApp"));
        assert!(!json.contains("identity_scan_count"));
        assert!(json.contains("identityScanCount"));
    }

    #[test]
    fn import_private_key_input_serializes() {
        let input = ImportPrivateKeyInput {
            private_key: "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ".into(),
            password: "mypassword".into(),
            alias: "My Key".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"privateKey\":\"5HueCGU8rMjxEXx"));
        assert!(json.contains("\"password\":\"mypassword\""));
        assert!(json.contains("\"alias\":\"My Key\""));
    }

    #[test]
    fn import_private_key_input_with_empty_fields_serializes() {
        let input = ImportPrivateKeyInput {
            private_key: "a".repeat(64),
            password: String::new(),
            alias: String::new(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"password\":\"\""));
        assert!(json.contains("\"alias\":\"\""));
    }

    #[test]
    fn import_private_key_input_roundtrip() {
        let json = r#"{"privateKey":"5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ","password":"secret","alias":"My Key"}"#;
        let input: ImportPrivateKeyInput = serde_json::from_str(json).unwrap();
        assert_eq!(
            input.private_key,
            "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ"
        );
        assert_eq!(input.password, "secret");
        assert_eq!(input.alias, "My Key");
    }

    #[test]
    fn import_private_key_input_uses_camel_case() {
        let input = ImportPrivateKeyInput {
            private_key: "abc123".into(),
            password: "pw".into(),
            alias: "k".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(!json.contains("private_key"));
        assert!(json.contains("privateKey"));
    }
}
