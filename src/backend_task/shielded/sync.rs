use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::{ShieldedNote, ShieldedWalletState};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::grovedb_commitment_tree::{Position, Retention};
use dash_sdk::platform::shielded::sync_shielded_notes;
use std::sync::Arc;

/// Server-enforced chunk size — start_index must be a multiple of this.
const CHUNK_SIZE: u64 = 2048;

/// Sync encrypted notes from the platform using the SDK's parallel chunk fetcher.
///
/// On resume the raw `last_synced_index` may fall mid-chunk; we round down to
/// the nearest chunk boundary and re-fetch that partial chunk, skipping any
/// positions already present in the local tree.
pub async fn sync_notes(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    shielded_state: &mut ShieldedWalletState,
    network: Network,
) -> Result<(u32, u64), TaskError> {
    let sdk = { app_context.sdk.load().as_ref().clone() };

    let network_str = network.to_string();
    let prepared_ivk = shielded_state.keys.ivk.prepare();

    // Round down to nearest chunk boundary so the server accepts the query.
    let already_have = shielded_state.last_synced_index;
    let aligned_start = (already_have / CHUNK_SIZE) * CHUNK_SIZE;

    tracing::info!(
        "Starting shielded note sync: last_synced={}, aligned_start={}",
        already_have,
        aligned_start,
    );

    let result = sync_shielded_notes(&sdk, &prepared_ivk, aligned_start, None)
        .await
        .map_err(|e| TaskError::ShieldedSyncFailed {
            detail: e.to_string(),
        })?;

    tracing::info!(
        "Sync complete: total_scanned={}, decrypted={}, next_start_index={}",
        result.total_notes_scanned,
        result.decrypted_notes.len(),
        result.next_start_index,
    );

    if result.next_start_index == 0 && result.total_notes_scanned > 0 {
        tracing::warn!(
            "Shielded sync: next_start_index is 0 after scanning {} notes — next sync will rescan everything from the beginning",
            result.total_notes_scanned,
        );
    }

    // Append notes to the local commitment tree, skipping positions already present.
    // all_notes is ordered: aligned_start + i == global position of all_notes[i].
    let mut appended = 0u32;
    for (i, raw_note) in result.all_notes.iter().enumerate() {
        let global_pos = aligned_start + i as u64;
        if global_pos < already_have {
            continue; // already appended in a previous sync
        }

        let cmx_bytes: [u8; 32] =
            raw_note
                .cmx
                .as_slice()
                .try_into()
                .map_err(|_| TaskError::ShieldedSyncFailed {
                    detail: "Invalid cmx length".into(),
                })?;

        let is_ours = result
            .decrypted_notes
            .iter()
            .any(|dn| dn.position == global_pos);
        let retention = if is_ours {
            Retention::Marked
        } else {
            Retention::Ephemeral
        };

        shielded_state
            .commitment_tree
            .lock()
            .unwrap()
            .append(cmx_bytes, retention)
            .map_err(|e| TaskError::ShieldedTreeUpdateFailed {
                detail: e.to_string(),
            })?;

        appended += 1;
    }

    if appended > 0 {
        let checkpoint_id = result.next_start_index as u32;
        shielded_state
            .commitment_tree
            .lock()
            .unwrap()
            .checkpoint(checkpoint_id)
            .map_err(|e| TaskError::ShieldedTreeUpdateFailed {
                detail: e.to_string(),
            })?;
    }

    // Persist and record decrypted notes that are new (position >= already_have).
    let mut new_note_count = 0u32;
    for dn in result.decrypted_notes {
        if dn.position < already_have {
            continue; // already stored in a previous sync
        }

        // Compute the spending nullifier from our FVK (dn.nullifier is the rho/nf
        // field from the compact action, not the spending nullifier).
        let nullifier = dn.note.nullifier(&shielded_state.keys.fvk);
        let value = dn.note.value().inner();

        tracing::info!("Note[{}]: DECRYPTED, value={} credits", dn.position, value,);

        let note_data = crate::model::wallet::shielded::serialize_note(&dn.note);
        let nullifier_bytes = nullifier.to_bytes();

        let _ = app_context.db.insert_shielded_note(
            seed_hash,
            &crate::database::shielded::InsertShieldedNote {
                note_data: &note_data,
                position: dn.position,
                cmx: &dn.cmx,
                nullifier: &nullifier_bytes,
                block_height: 0,
                value,
                network: &network_str,
            },
        );

        shielded_state.notes.push(ShieldedNote {
            note: dn.note,
            position: Position::from(dn.position),
            cmx: dn.cmx,
            nullifier,
            block_height: 0,
            is_spent: false,
            value,
        });

        new_note_count += 1;
    }

    // Store the actual number of notes seen, not the chunk-rounded next_start_index.
    // The SDK rounds next_start_index UP to the next 2048 boundary, which would
    // make the display show 2048 even when only e.g. 150 notes exist.
    // On the next sync we round down again, re-fetch the partial chunk, and skip
    // positions already in the tree — so correctness is maintained.
    shielded_state.last_synced_index = aligned_start + result.total_notes_scanned;
    shielded_state.recalculate_balance();

    let notes_total = shielded_state.notes.len();
    let notes_unspent = shielded_state.unspent_notes().len();

    tracing::info!(
        "Shielded sync finished: {} new note(s), total notes={} (unspent={}), spendable balance: {} ({} credits)",
        new_note_count,
        notes_total,
        notes_unspent,
        format_credits_as_dash(shielded_state.shielded_balance),
        shielded_state.shielded_balance,
    );

    Ok((new_note_count, shielded_state.shielded_balance))
}
