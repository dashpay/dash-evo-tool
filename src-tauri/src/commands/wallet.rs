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

// NOTE: FundPlatformAddressFromAssetLock is not exposed yet because
// AssetLockProof requires dedicated DTO serialization. The proof typically
// comes from a prior CreateRegistrationAssetLock/CreateTopUpAssetLock task
// result and would be passed through the event system. This will be
// implemented when the full asset lock workflow is wired up.

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
}
