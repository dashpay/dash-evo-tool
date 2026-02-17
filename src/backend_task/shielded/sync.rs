use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::{ShieldedNote, ShieldedWalletState};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::grovedb_commitment_tree::{
    COMPACT_NOTE_SIZE, CompactAction, DashMemo, EphemeralKeyBytes, ExtractedNoteCommitment, Note,
    Nullifier, OrchardDomain, Position, PreparedIncomingViewingKey, Retention,
    try_compact_note_decryption,
};
use dash_sdk::platform::Fetch;
use dash_sdk::query_types::{
    ShieldedEncryptedNote, ShieldedEncryptedNotes, ShieldedEncryptedNotesQuery,
};
use std::sync::Arc;

const SYNC_BATCH_SIZE: u32 = 1000;

/// Minimum size of the `encrypted_note` field from the RPC response.
///
/// The stored value format in GroveDB is: `cmx(32) || nullifier(32) || encrypted_note(216)`.
/// After proof verification splits at 32 bytes, the `encrypted_note` field contains:
///   nullifier(32) || epk(32) || enc_ciphertext(104) || out_ciphertext(80) = 248 bytes.
const MIN_ENCRYPTED_NOTE_LEN: usize = 32 + 32 + COMPACT_NOTE_SIZE;

/// Sync encrypted notes from the platform's BulkAppendTree.
///
/// For each encrypted note:
/// 1. Trial-decrypt with the wallet's IVK using compact note decryption
/// 2. If decrypted, compute nullifier and store as ShieldedNote
/// 3. Append ALL cmx values to the ClientCommitmentTree
///    (Retention::Marked for our notes, Retention::Ephemeral for others)
pub async fn sync_notes(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    shielded_state: &mut ShieldedWalletState,
    network: Network,
) -> Result<(u32, u64), String> {
    let sdk = {
        let guard = app_context.sdk.read().unwrap();
        guard.clone()
    };

    let network_str = network.to_string();
    let prepared_ivk = shielded_state.keys.ivk.prepare();
    let mut new_note_count = 0u32;
    let mut start_index = shielded_state.last_synced_index;
    let mut checkpoint_id = start_index as u32;

    loop {
        let query = ShieldedEncryptedNotesQuery {
            start_index,
            count: SYNC_BATCH_SIZE,
        };

        let notes_result: Option<ShieldedEncryptedNotes> =
            ShieldedEncryptedNotes::fetch(&sdk, query)
                .await
                .map_err(|e| format!("Failed to fetch encrypted notes: {e}"))?;

        let notes = match notes_result {
            Some(n) => n.0,
            None => break,
        };

        if notes.is_empty() {
            break;
        }

        let batch_len = notes.len() as u64;

        for (i, encrypted_note) in notes.into_iter().enumerate() {
            let global_position = start_index + i as u64;
            let position = Position::from(global_position);

            let cmx_bytes: [u8; 32] = encrypted_note
                .cmx
                .clone()
                .try_into()
                .map_err(|_| "Invalid cmx length")?;

            // Try to trial-decrypt with our IVK
            if let Some(note) = try_decrypt_note(&prepared_ivk, &encrypted_note, &cmx_bytes) {
                let nullifier = note.nullifier(&shielded_state.keys.fvk);
                let value = note.value().inner();
                let note_data = crate::model::wallet::shielded::serialize_note(&note);

                let shielded_note = ShieldedNote {
                    note,
                    position,
                    cmx: cmx_bytes,
                    nullifier,
                    block_height: 0, // Not available from encrypted notes query
                    is_spent: false,
                    value,
                };

                // Persist to DB
                let nullifier_bytes = nullifier.to_bytes();
                let _ = app_context.db.insert_shielded_note(
                    seed_hash,
                    &crate::database::shielded::InsertShieldedNote {
                        note_data: &note_data,
                        position: global_position,
                        cmx: &cmx_bytes,
                        nullifier: &nullifier_bytes,
                        block_height: 0,
                        value,
                        network: &network_str,
                    },
                );

                shielded_state.notes.push(shielded_note);
                new_note_count += 1;

                // Mark in commitment tree (we need witness for this note)
                shielded_state
                    .commitment_tree
                    .append(cmx_bytes, Retention::Marked)
                    .map_err(|e| format!("Failed to append marked note to tree: {e}"))?;
            } else {
                // Not our note, but still need to track in tree for correct Merkle paths
                shielded_state
                    .commitment_tree
                    .append(cmx_bytes, Retention::Ephemeral)
                    .map_err(|e| format!("Failed to append ephemeral note to tree: {e}"))?;
            }
        }

        start_index += batch_len;
        checkpoint_id += 1;

        // Checkpoint the tree
        shielded_state
            .commitment_tree
            .checkpoint(checkpoint_id)
            .map_err(|e| format!("Failed to checkpoint tree: {e}"))?;

        // If we got fewer notes than the batch size, we've caught up
        if batch_len < SYNC_BATCH_SIZE as u64 {
            break;
        }
    }

    shielded_state.last_synced_index = start_index;
    shielded_state.recalculate_balance();

    // Save tree state to DB
    let _ = app_context.db.save_shielded_tree_state(
        seed_hash,
        &[],
        shielded_state.last_synced_index,
        &network_str,
    );

    Ok((new_note_count, shielded_state.shielded_balance))
}

/// Attempt compact trial decryption on an encrypted note from the RPC.
///
/// The `encrypted_note.encrypted_note` field contains (after proof verification):
///   `nullifier(32) || epk(32) || enc_ciphertext(104) || out_ciphertext(80)`
///
/// For compact decryption we only need the first 116 bytes:
///   - nullifier(32): derives Rho via `Rho::from_nf_old(nullifier)` for OrchardDomain
///   - epk(32): ephemeral public key for key agreement
///   - enc_compact(52): first COMPACT_NOTE_SIZE bytes of enc_ciphertext
///     (version + diversifier + value + rseed)
fn try_decrypt_note(
    ivk: &PreparedIncomingViewingKey,
    encrypted_note: &ShieldedEncryptedNote,
    cmx_bytes: &[u8; 32],
) -> Option<Note> {
    let data = &encrypted_note.encrypted_note;
    if data.len() < MIN_ENCRYPTED_NOTE_LEN {
        return None;
    }

    // Parse nullifier (first 32 bytes)
    let nf_bytes: [u8; 32] = data[0..32].try_into().ok()?;
    let nf = Nullifier::from_bytes(&nf_bytes).into_option()?;

    // Parse cmx
    let cmx = ExtractedNoteCommitment::from_bytes(cmx_bytes).into_option()?;

    // Parse ephemeral public key (next 32 bytes)
    let epk_bytes: [u8; 32] = data[32..64].try_into().ok()?;

    // Parse compact ciphertext (first COMPACT_NOTE_SIZE bytes of enc_ciphertext)
    let enc_compact: [u8; COMPACT_NOTE_SIZE] = data[64..64 + COMPACT_NOTE_SIZE].try_into().ok()?;

    // Build CompactAction and OrchardDomain for trial decryption
    let compact = CompactAction::from_parts(nf, cmx, EphemeralKeyBytes(epk_bytes), enc_compact);
    let domain = OrchardDomain::<DashMemo>::for_compact_action(&compact);

    // Attempt decryption — returns (Note, PaymentAddress) if this note belongs to us
    let (note, _address) = try_compact_note_decryption(&domain, ivk, &compact)?;
    Some(note)
}
