//! System, MnList, QRInfo, GroveSTARK, and BroadcastStateTransition Tauri IPC commands.

use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::grovestark::GroveSTARKTask;
use dash_evo_tool::backend_task::mnlist::MnListTask;
use dash_evo_tool::backend_task::system_task::SystemTask;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::model::qualified_identity::PrivateKeyTarget;
use dash_evo_tool::model::settings::ThemeMode;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Theme mode DTO.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ThemeModeDto {
    Light,
    Dark,
    System,
}

impl ThemeModeDto {
    pub fn to_backend(self) -> ThemeMode {
        match self {
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
            Self::System => ThemeMode::System,
        }
    }
    pub fn from_backend(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::Light,
            ThemeMode::Dark => Self::Dark,
            ThemeMode::System => Self::System,
        }
    }
}

/// Input for fetching MnList diff.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchMnListDiffInput {
    pub base_block_height: u32,
    pub base_block_hash: String,
    pub block_height: u32,
    pub block_hash: String,
    pub validate_quorums: bool,
}

/// Input for fetching QR info.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchQrInfoInput {
    pub known_block_hashes: Vec<String>,
    pub block_hash: String,
}

/// Input for fetching chain locks over a range.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchChainLocksInput {
    pub base_block_height: u32,
    pub block_height: u32,
}

/// A single diff chain entry.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiffChainEntry {
    pub base_height: u32,
    pub base_hash: String,
    pub height: u32,
    pub hash: String,
}

/// Input for fetching a chain of diffs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchDiffsChainInput {
    pub chain: Vec<DiffChainEntry>,
}

/// Input for generating a GroveSTARK proof.
///
/// The private and public keys are resolved on the backend from the identity
/// and key ID, so the frontend never handles raw private key material.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenerateGroveStarkProofInput {
    pub identity_id: String,
    pub contract_id: String,
    pub document_type: String,
    pub document_id: String,
    pub key_id: u32,
}

/// Input for verifying a GroveSTARK proof.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyGroveStarkProofInput {
    /// Serialized proof bytes as hex.
    pub proof_hex: String,
    /// Public inputs.
    pub state_root_hex: String,
    pub contract_id_hex: String,
    pub message_hash_hex: String,
    pub timestamp: u64,
    /// Metadata.
    pub created_at: u64,
    pub proof_size: u64,
    pub generation_time_ms: u64,
    pub security_level: u32,
}

/// Input for broadcasting a state transition.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastStateTransitionInput {
    /// Hex-encoded state transition bytes.
    pub state_transition_hex: String,
}

// ---------------------------------------------------------------------------
// Helpers: parse BlockHash
// ---------------------------------------------------------------------------

fn parse_block_hash(hex_str: &str) -> Result<dash_sdk::dpp::dashcore::BlockHash, String> {
    use dash_sdk::dpp::dashcore::hashes::Hash;
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid block hash hex: {e}"))?;
    dash_sdk::dpp::dashcore::BlockHash::from_slice(&bytes)
        .map_err(|e| format!("Invalid block hash: {e}"))
}

fn parse_32_bytes(hex_str: &str, name: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid {} hex: {e}", name))?;
    if bytes.len() != 32 {
        return Err(format!("{} must be 32 bytes, got {}", name, bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

// ---------------------------------------------------------------------------
// System commands
// ---------------------------------------------------------------------------

/// Wipe all platform data (devnet only).
#[tauri::command]
#[specta::specta]
pub fn system_wipe_platform_data(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::SystemTask(SystemTask::WipePlatformData);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Update the theme preference.
#[tauri::command]
#[specta::specta]
pub fn system_update_theme(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    theme: ThemeModeDto,
) -> DispatchTaskResponse {
    let task = BackendTask::SystemTask(SystemTask::UpdateThemePreference(theme.to_backend()));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

// ---------------------------------------------------------------------------
// MnList commands
// ---------------------------------------------------------------------------

/// Fetch a masternode list diff between two block heights.
#[tauri::command]
#[specta::specta]
pub fn mnlist_fetch_diff(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchMnListDiffInput,
) -> Result<DispatchTaskResponse, String> {
    let base_block_hash = parse_block_hash(&input.base_block_hash)?;
    let block_hash = parse_block_hash(&input.block_hash)?;
    let task = BackendTask::MnListTask(MnListTask::FetchEndDmlDiff {
        base_block_height: input.base_block_height,
        base_block_hash,
        block_height: input.block_height,
        block_hash,
        validate_quorums: input.validate_quorums,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch quorum rotation info.
#[tauri::command]
#[specta::specta]
pub fn mnlist_fetch_qr_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchQrInfoInput,
) -> Result<DispatchTaskResponse, String> {
    let known_hashes: Vec<dash_sdk::dpp::dashcore::BlockHash> = input
        .known_block_hashes
        .iter()
        .map(|h| parse_block_hash(h))
        .collect::<Result<Vec<_>, String>>()?;
    let block_hash = parse_block_hash(&input.block_hash)?;
    let task = BackendTask::MnListTask(MnListTask::FetchEndQrInfo {
        known_block_hashes: known_hashes,
        block_hash,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch quorum rotation info with included DMLs.
#[tauri::command]
#[specta::specta]
pub fn mnlist_fetch_qr_info_with_dmls(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchQrInfoInput,
) -> Result<DispatchTaskResponse, String> {
    let known_hashes: Vec<dash_sdk::dpp::dashcore::BlockHash> = input
        .known_block_hashes
        .iter()
        .map(|h| parse_block_hash(h))
        .collect::<Result<Vec<_>, String>>()?;
    let block_hash = parse_block_hash(&input.block_hash)?;
    let task = BackendTask::MnListTask(MnListTask::FetchEndQrInfoWithDmls {
        known_block_hashes: known_hashes,
        block_hash,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch chain lock signatures over a block height range.
#[tauri::command]
#[specta::specta]
pub fn mnlist_fetch_chain_locks(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchChainLocksInput,
) -> DispatchTaskResponse {
    let task = BackendTask::MnListTask(MnListTask::FetchChainLocks {
        base_block_height: input.base_block_height,
        block_height: input.block_height,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch a chain of MnList diffs for validation.
#[tauri::command]
#[specta::specta]
pub fn mnlist_fetch_diffs_chain(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchDiffsChainInput,
) -> Result<DispatchTaskResponse, String> {
    let chain: Vec<(
        u32,
        dash_sdk::dpp::dashcore::BlockHash,
        u32,
        dash_sdk::dpp::dashcore::BlockHash,
    )> = input
        .chain
        .iter()
        .map(|entry| {
            let base_hash = parse_block_hash(&entry.base_hash)?;
            let hash = parse_block_hash(&entry.hash)?;
            Ok((entry.base_height, base_hash, entry.height, hash))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let task = BackendTask::MnListTask(MnListTask::FetchDiffsChain { chain });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// QRInfo DTOs
// ---------------------------------------------------------------------------

/// Serializable representation of a QuorumSnapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QuorumSnapshotDto {
    pub skip_list_mode: u8,
    pub active_quorum_members: Vec<bool>,
    pub skip_list: Vec<u32>,
}

/// Serializable qualified quorum entry (from QRInfo — includes verification status).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QuorumEntryDto {
    pub version: u16,
    pub llmq_type: u8,
    pub quorum_hash: String,
    pub quorum_index: Option<i16>,
    pub signers: Vec<bool>,
    pub valid_members: Vec<bool>,
    pub quorum_public_key: String,
    pub quorum_vvec_hash: String,
    pub threshold_sig: String,
    pub all_commitment_aggregated_signature: String,
    pub commitment_hash: String,
    pub entry_hash: String,
    pub verification_status: String,
}

/// Serializable quorum entry from MnListDiff (unqualified — no verification status).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SimpleQuorumEntryDto {
    pub version: u16,
    pub llmq_type: u8,
    pub quorum_hash: String,
    pub quorum_index: Option<i16>,
    pub signers: Vec<bool>,
    pub valid_members: Vec<bool>,
    pub quorum_public_key: String,
    pub quorum_vvec_hash: String,
    pub threshold_sig: String,
    pub all_commitment_aggregated_signature: String,
}

/// Deleted quorum entry (type + hash only).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletedQuorumDto {
    pub llmq_type: u8,
    pub quorum_hash: String,
}

/// Serializable representation of a MnListDiff.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MnListDiffDto {
    pub version: u16,
    pub base_block_hash: String,
    pub block_hash: String,
    pub total_transactions: u32,
    pub merkle_hashes: Vec<String>,
    pub merkle_flags_len: usize,
    pub coinbase_txid: String,
    pub coinbase_size: usize,
    pub new_masternodes: Vec<MasternodeEntryDto>,
    pub deleted_masternodes: Vec<String>,
    pub new_quorums: Vec<SimpleQuorumEntryDto>,
    pub deleted_quorums: Vec<DeletedQuorumDto>,
    pub chainlock_sig_count: usize,
    pub chainlock_signatures: Vec<ChainLockSigEntryDto>,
}

/// A masternode list entry.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MasternodeEntryDto {
    pub pro_reg_tx_hash: String,
    pub address: String,
}

/// A chainlock signature entry from MnListDiff.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChainLockSigEntryDto {
    pub signature: String,
    pub index_set: Vec<u16>,
}

/// Serializable representation of a loaded QRInfo file.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QrInfoDto {
    /// Block hash used as the key for this QRInfo.
    pub tip_block_hash: String,
    /// Named snapshots at specific heights relative to head.
    pub snapshot_h_minus_c: QuorumSnapshotDto,
    pub snapshot_h_minus_2c: QuorumSnapshotDto,
    pub snapshot_h_minus_3c: QuorumSnapshotDto,
    pub snapshot_h_minus_4c: Option<QuorumSnapshotDto>,
    /// Named diffs at specific heights.
    pub diff_h_minus_3c: MnListDiffDto,
    pub diff_h_minus_2c: MnListDiffDto,
    pub diff_h_minus_c: MnListDiffDto,
    pub diff_h: MnListDiffDto,
    pub diff_tip: MnListDiffDto,
    pub diff_h_minus_4c: Option<MnListDiffDto>,
    /// Rotated quorum commitments (unqualified — from raw QRInfo).
    pub last_commitments: Vec<SimpleQuorumEntryDto>,
    /// Additional snapshot and diff lists.
    pub quorum_snapshot_list: Vec<QuorumSnapshotDto>,
    pub mn_list_diff_list: Vec<MnListDiffDto>,
}

// ---------------------------------------------------------------------------
// QRInfo conversion helpers
// ---------------------------------------------------------------------------

fn convert_quorum_snapshot(
    snapshot: &dash_sdk::dpp::dashcore::network::message_qrinfo::QuorumSnapshot,
) -> QuorumSnapshotDto {
    QuorumSnapshotDto {
        skip_list_mode: snapshot.skip_list_mode.index() as u8,
        active_quorum_members: snapshot.active_quorum_members.clone(),
        skip_list: snapshot.skip_list.iter().map(|&v| v as u32).collect(),
    }
}

/// Convert a QuorumEntry to a SimpleQuorumEntryDto.
fn convert_simple_quorum_entry(
    e: &dash_sdk::dpp::dashcore::transaction::special_transaction::quorum_commitment::QuorumEntry,
) -> SimpleQuorumEntryDto {
    SimpleQuorumEntryDto {
        version: e.version,
        llmq_type: u8::from(e.llmq_type),
        quorum_hash: e.quorum_hash.to_string(),
        quorum_index: e.quorum_index,
        signers: e.signers.clone(),
        valid_members: e.valid_members.clone(),
        quorum_public_key: e.quorum_public_key.to_string(),
        quorum_vvec_hash: e.quorum_vvec_hash.to_string(),
        threshold_sig: hex::encode(e.threshold_sig.as_bytes()),
        all_commitment_aggregated_signature: hex::encode(
            e.all_commitment_aggregated_signature.as_bytes(),
        ),
    }
}

pub fn convert_mn_list_diff(
    diff: &dash_sdk::dpp::dashcore::network::message_sml::MnListDiff,
) -> MnListDiffDto {
    let new_masternodes = diff
        .new_masternodes
        .iter()
        .map(|mn| {
            let addr = format!("{}", mn.service_address);
            MasternodeEntryDto {
                pro_reg_tx_hash: mn.pro_reg_tx_hash.to_string(),
                address: addr,
            }
        })
        .collect();

    let deleted_masternodes = diff
        .deleted_masternodes
        .iter()
        .map(|h| h.to_string())
        .collect();

    let new_quorums = diff
        .new_quorums
        .iter()
        .map(convert_simple_quorum_entry)
        .collect();

    let deleted_quorums = diff
        .deleted_quorums
        .iter()
        .map(|dq| DeletedQuorumDto {
            llmq_type: u8::from(dq.llmq_type),
            quorum_hash: dq.quorum_hash.to_string(),
        })
        .collect();

    let chainlock_signatures = diff
        .quorums_chainlock_signatures
        .iter()
        .map(|obj| ChainLockSigEntryDto {
            signature: hex::encode(obj.signature.as_bytes()),
            index_set: obj.index_set.clone(),
        })
        .collect();

    MnListDiffDto {
        version: diff.version,
        base_block_hash: diff.base_block_hash.to_string(),
        block_hash: diff.block_hash.to_string(),
        total_transactions: diff.total_transactions,
        merkle_hashes: diff.merkle_hashes.iter().map(|h| h.to_string()).collect(),
        merkle_flags_len: diff.merkle_flags.len(),
        coinbase_txid: diff.coinbase_tx.txid().to_string(),
        coinbase_size: diff.coinbase_tx.size(),
        new_masternodes,
        deleted_masternodes,
        new_quorums,
        deleted_quorums,
        chainlock_sig_count: diff.quorums_chainlock_signatures.len(),
        chainlock_signatures,
    }
}

pub fn convert_qr_info(
    qr_info: &dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo,
) -> QrInfoDto {
    let (snapshot_h_minus_4c, diff_h_minus_4c) =
        if let Some((snap, diff)) = &qr_info.quorum_snapshot_and_mn_list_diff_at_h_minus_4c {
            (
                Some(convert_quorum_snapshot(snap)),
                Some(convert_mn_list_diff(diff)),
            )
        } else {
            (None, None)
        };

    QrInfoDto {
        tip_block_hash: qr_info.mn_list_diff_tip.block_hash.to_string(),
        snapshot_h_minus_c: convert_quorum_snapshot(&qr_info.quorum_snapshot_at_h_minus_c),
        snapshot_h_minus_2c: convert_quorum_snapshot(&qr_info.quorum_snapshot_at_h_minus_2c),
        snapshot_h_minus_3c: convert_quorum_snapshot(&qr_info.quorum_snapshot_at_h_minus_3c),
        snapshot_h_minus_4c,
        diff_h_minus_3c: convert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_3c),
        diff_h_minus_2c: convert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_2c),
        diff_h_minus_c: convert_mn_list_diff(&qr_info.mn_list_diff_at_h_minus_c),
        diff_h: convert_mn_list_diff(&qr_info.mn_list_diff_h),
        diff_tip: convert_mn_list_diff(&qr_info.mn_list_diff_tip),
        diff_h_minus_4c,
        last_commitments: qr_info
            .last_commitment_per_index
            .iter()
            .map(convert_simple_quorum_entry)
            .collect(),
        quorum_snapshot_list: qr_info
            .quorum_snapshot_list
            .iter()
            .map(convert_quorum_snapshot)
            .collect(),
        mn_list_diff_list: qr_info
            .mn_list_diff_list
            .iter()
            .map(convert_mn_list_diff)
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// QRInfo commands
// ---------------------------------------------------------------------------

/// Open a native file dialog and load a QRInfo `.dat` file.
///
/// Attempts consensus decode first, then falls back to bincode.
/// Returns the parsed QRInfo as a DTO.
#[tauri::command]
#[specta::specta]
pub async fn qrinfo_load_file() -> Result<QrInfoDto, String> {
    use dash_sdk::dpp::dashcore::consensus::Decodable;
    use dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo;

    let file_path = rfd::AsyncFileDialog::new()
        .add_filter("Data Files", &["dat"])
        .pick_file()
        .await
        .ok_or_else(|| "File selection cancelled".to_string())?;

    let data = file_path.read().await;

    // Try consensus decode first
    let qr_info = match QRInfo::consensus_decode(&mut data.as_slice()) {
        Ok(qi) => qi,
        Err(_) => {
            // Fallback to bincode
            let (qi, _): (QRInfo, _) =
                bincode::decode_from_slice(&data, bincode::config::standard())
                    .map_err(|e| format!("Failed to decode QRInfo file: {e}"))?;
            qi
        }
    };

    Ok(convert_qr_info(&qr_info))
}

/// Open a native save dialog and save QRInfo data as a `.dat` file.
///
/// Accepts a serialized QRInfo DTO key (tip block hash) and raw bytes to write.
/// The caller provides the raw bytes from a previously loaded file.
#[tauri::command]
#[specta::specta]
pub async fn qrinfo_save_file(
    #[allow(unused)] default_filename: String,
    data_hex: String,
) -> Result<(), String> {
    let data = hex::decode(&data_hex).map_err(|e| format!("Invalid hex data: {e}"))?;

    let file_path = rfd::AsyncFileDialog::new()
        .set_file_name(&default_filename)
        .add_filter("Data Files", &["dat"])
        .save_file()
        .await
        .ok_or_else(|| "Save cancelled".to_string())?;

    // rfd::FileHandle doesn't directly expose the path for writing on all platforms,
    // so we use write() which is supported.
    file_path
        .write(&data)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// GroveSTARK commands
// ---------------------------------------------------------------------------

/// Generate a GroveSTARK proof.
///
/// Resolves the private and public keys from the identity on the backend,
/// keeping key material off the frontend.
#[tauri::command]
#[specta::specta]
pub fn grovestark_generate_proof(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: GenerateGroveStarkProofInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::identity::KeyType;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;

    let ctx = state.current_context();

    // Load qualified identities and find the one matching the input identity_id
    let qualified_identities = ctx
        .load_local_qualified_identities()
        .map_err(|e| format!("Failed to load identities: {e}"))?;

    let qualified_identity = qualified_identities
        .iter()
        .find(|qi| qi.identity.id().to_string(Encoding::Base58) == input.identity_id)
        .ok_or_else(|| format!("Identity not found: {}", input.identity_id))?;

    // Verify the key exists and is EdDSA
    let _identity_key = qualified_identity
        .identity
        .public_keys()
        .iter()
        .find(|(_, k)| k.id() == input.key_id)
        .map(|(_, k)| k)
        .ok_or_else(|| format!("Key {} not found on identity", input.key_id))?;

    if _identity_key.key_type() != KeyType::EDDSA_25519_HASH160 {
        return Err(format!(
            "Key {} is not EdDSA (EDDSA_25519_HASH160). Got {:?}",
            input.key_id,
            _identity_key.key_type()
        ));
    }

    // Resolve the private key from the qualified identity
    let wallet_vec: Vec<_> = ctx
        .loaded_wallets()
        .into_iter()
        .map(|(_, arc)| arc)
        .collect();

    let private_key = qualified_identity
        .private_keys
        .get_resolve(
            &(PrivateKeyTarget::PrivateKeyOnMainIdentity, input.key_id),
            &wallet_vec,
            ctx.network(),
        )
        .map_err(|e| format!("Failed to resolve private key: {e}"))?
        .map(|(_, bytes)| bytes)
        .ok_or_else(|| "Private key not found in storage".to_string())?;

    // Derive the public key from the private key using ed25519-dalek
    let public_key = {
        use ed25519_dalek::SigningKey;
        let signing_key = SigningKey::from_bytes(&private_key);
        *signing_key.verifying_key().as_bytes()
    };

    let task = BackendTask::GroveSTARKTask(GroveSTARKTask::GenerateProof {
        identity_id: input.identity_id,
        contract_id: input.contract_id,
        document_type: input.document_type,
        document_id: input.document_id,
        key_id: input.key_id,
        private_key,
        public_key,
    });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Verify a GroveSTARK proof.
#[tauri::command]
#[specta::specta]
pub fn grovestark_verify_proof(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: VerifyGroveStarkProofInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_evo_tool::model::grovestark_prover::{
        ProofDataOutput, ProofMetadata, PublicInputsData,
    };

    let proof = hex::decode(&input.proof_hex).map_err(|e| format!("Invalid proof hex: {e}"))?;
    let state_root = parse_32_bytes(&input.state_root_hex, "state root")?;
    let contract_id = parse_32_bytes(&input.contract_id_hex, "contract ID")?;
    let message_hash = parse_32_bytes(&input.message_hash_hex, "message hash")?;

    let proof_data = ProofDataOutput {
        proof,
        public_inputs: PublicInputsData {
            state_root,
            contract_id,
            message_hash,
            timestamp: input.timestamp,
        },
        metadata: ProofMetadata {
            created_at: input.created_at,
            proof_size: input.proof_size as usize,
            generation_time_ms: input.generation_time_ms,
            security_level: input.security_level,
        },
    };

    let task = BackendTask::GroveSTARKTask(GroveSTARKTask::VerifyProof { proof_data });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// BroadcastStateTransition
// ---------------------------------------------------------------------------

/// Broadcast a state transition to Platform.
#[tauri::command]
#[specta::specta]
pub fn broadcast_state_transition(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: BroadcastStateTransitionInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_sdk::dpp::state_transition::StateTransition;

    let bytes = hex::decode(&input.state_transition_hex)
        .map_err(|e| format!("Invalid state transition hex: {e}"))?;

    let st: StateTransition = dash_sdk::dpp::bincode::decode_from_slice(
        &bytes,
        dash_sdk::dpp::bincode::config::standard()
            .with_big_endian()
            .with_no_limit(),
    )
    .map_err(|e| format!("Failed to decode state transition: {e}"))?
    .0;

    let task = BackendTask::BroadcastStateTransition(st);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_mode_dto_serializes() {
        let light = ThemeModeDto::Light;
        let json = serde_json::to_string(&light).unwrap();
        assert!(json.contains("\"light\""));

        let dark = ThemeModeDto::Dark;
        let json = serde_json::to_string(&dark).unwrap();
        assert!(json.contains("\"dark\""));

        let system = ThemeModeDto::System;
        let json = serde_json::to_string(&system).unwrap();
        assert!(json.contains("\"system\""));
    }

    #[test]
    fn theme_mode_dto_roundtrip() {
        let original = ThemeModeDto::Dark;
        let backend = original.to_backend();
        let recovered = ThemeModeDto::from_backend(backend);
        let json_orig = serde_json::to_string(&original).unwrap();
        let json_recovered = serde_json::to_string(&recovered).unwrap();
        assert_eq!(json_orig, json_recovered);
    }

    #[test]
    fn fetch_mn_list_diff_input_serializes() {
        let input = FetchMnListDiffInput {
            base_block_height: 100,
            base_block_hash: "aa".repeat(32),
            block_height: 200,
            block_hash: "bb".repeat(32),
            validate_quorums: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"baseBlockHeight\":100"));
        assert!(json.contains("\"blockHeight\":200"));
        assert!(json.contains("\"validateQuorums\":true"));
    }

    #[test]
    fn fetch_qr_info_input_serializes() {
        let input = FetchQrInfoInput {
            known_block_hashes: vec!["aa".repeat(32)],
            block_hash: "bb".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"knownBlockHashes\""));
        assert!(json.contains("\"blockHash\""));
    }

    #[test]
    fn fetch_chain_locks_input_serializes() {
        let input = FetchChainLocksInput {
            base_block_height: 100,
            block_height: 200,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"baseBlockHeight\":100"));
    }

    #[test]
    fn diff_chain_entry_serializes() {
        let entry = DiffChainEntry {
            base_height: 1,
            base_hash: "aa".repeat(32),
            height: 2,
            hash: "bb".repeat(32),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"baseHeight\":1"));
        assert!(json.contains("\"height\":2"));
    }

    #[test]
    fn generate_grovestark_proof_input_serializes() {
        let input = GenerateGroveStarkProofInput {
            identity_id: "abc".into(),
            contract_id: "def".into(),
            document_type: "note".into(),
            document_id: "ghi".into(),
            key_id: 1,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"documentType\":\"note\""));
        assert!(json.contains("\"keyId\":1"));
    }

    #[test]
    fn verify_grovestark_proof_input_serializes() {
        let input = VerifyGroveStarkProofInput {
            proof_hex: "deadbeef".into(),
            state_root_hex: "aa".repeat(32),
            contract_id_hex: "bb".repeat(32),
            message_hash_hex: "cc".repeat(32),
            timestamp: 1000,
            created_at: 2000,
            proof_size: 512,
            generation_time_ms: 100,
            security_level: 3,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"proofHex\":\"deadbeef\""));
        assert!(json.contains("\"timestamp\":1000"));
        assert!(json.contains("\"securityLevel\":3"));
    }

    #[test]
    fn broadcast_state_transition_input_serializes() {
        let input = BroadcastStateTransitionInput {
            state_transition_hex: "cafebabe".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"stateTransitionHex\":\"cafebabe\""));
    }

    #[test]
    fn parse_32_bytes_valid() {
        let hex = "aa".repeat(32);
        let result = parse_32_bytes(&hex, "test");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [0xaa; 32]);
    }

    #[test]
    fn parse_32_bytes_wrong_length() {
        let hex = "aa".repeat(16);
        let result = parse_32_bytes(&hex, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("32 bytes"));
    }

    #[test]
    fn parse_32_bytes_invalid_hex() {
        let result = parse_32_bytes("not-hex", "test");
        assert!(result.is_err());
    }
}
