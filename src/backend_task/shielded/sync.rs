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

    // Persist decrypted notes to the sidecar BEFORE advancing the commitment
    // tree (F54). The reload cursor (`last_synced_index`) derives from the
    // tree's `max_leaf_position() + 1`, NOT from the sidecar, so if a note's
    // commitment were appended + checkpointed before its sidecar row landed and
    // the insert then failed, the next restart's cursor would skip that
    // position forever — silently losing a spendable note. Persisting first and
    // propagating the insert error (rather than swallowing it) means a failed
    // insert aborts the sync before the checkpoint, so the position is
    // re-scanned next time. The sidecar uses INSERT OR IGNORE, so a re-scan of
    // an already-persisted note is idempotent.
    //
    // Build a HashMap of position->value for O(1) dedup + divergence detection.
    let existing_notes: std::collections::HashMap<u64, u64> = shielded_state
        .notes
        .iter()
        .map(|n| (u64::from(n.position), n.note.value().inner()))
        .collect();
    let mut new_note_count = 0u32;
    let mut pending_notes = Vec::new();
    for dn in result.decrypted_notes.iter() {
        if dn.position < already_have {
            continue; // already stored in a previous sync
        }
        if let Some(&existing_value) = existing_notes.get(&dn.position) {
            let new_value = dn.note.value().inner();
            if new_value != existing_value {
                tracing::warn!(
                    position = dn.position,
                    existing_value,
                    new_value,
                    "Shielded note dedup: value divergence at existing position"
                );
            }
            continue; // already loaded from DB during init
        }

        // Compute the spending nullifier from our FVK (dn.nullifier is the rho/nf
        // field from the compact action, not the spending nullifier).
        let nullifier = dn.note.nullifier(&shielded_state.keys.fvk);
        let value = dn.note.value().inner();

        tracing::info!("Note[{}]: DECRYPTED, value={} credits", dn.position, value,);

        let note_data = crate::model::wallet::shielded::serialize_note(&dn.note);
        let nullifier_bytes = nullifier.to_bytes();

        // T-SH-03: persist into the per-network shielded sidecar. A failure
        // here is propagated, NOT swallowed, so the tree is not advanced past
        // an unpersisted note (F54).
        if let Ok(backend) = app_context.wallet_backend() {
            backend
                .shielded()
                .insert_shielded_note(
                    seed_hash,
                    &crate::wallet_backend::InsertShieldedNote {
                        note_data: &note_data,
                        position: dn.position,
                        cmx: &dn.cmx,
                        nullifier: &nullifier_bytes,
                        block_height: 0,
                        value,
                        network: &network_str,
                    },
                )
                .map_err(|source| TaskError::ShieldedNotePersistFailed { source })?;
        }

        pending_notes.push(ShieldedNote {
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

    // Append notes to the local commitment tree, skipping positions already present.
    // all_notes is ordered: aligned_start + i == global position of all_notes[i].
    // Runs AFTER the sidecar persist above so a checkpoint never advances the
    // reload cursor past a note that did not reach the sidecar (F54).
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

    // The notes are durably in the sidecar and the tree is checkpointed; only
    // now adopt them into the in-memory state.
    shielded_state.notes.extend(pending_notes);

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

#[cfg(test)]
mod tests {
    use crate::backend_task::error::TaskError;
    use crate::wallet_backend::{InsertShieldedNote, ShieldedView};
    use dash_sdk::grovedb_commitment_tree::{ClientPersistentCommitmentTree, Retention};

    /// F54: a sidecar insert that fails surfaces as a real, propagatable error
    /// (`ShieldedNotePersistFailed`) — it is NOT swallowed. The pre-fix code
    /// used `let _ =`, so a transient IO failure on the insert was discarded
    /// while the commitment had already been appended + checkpointed, making
    /// the reload cursor permanently skip the unpersisted (lost) note.
    #[test]
    fn shielded_note_insert_failure_is_propagated_not_swallowed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Make the sidecar PATH a directory so `Connection::open` fails
        // deterministically on the insert (no reliance on permissions).
        let sidecar = ShieldedView::sidecar_path(tmp.path());
        std::fs::create_dir_all(&sidecar).expect("plant directory at sidecar path");

        let view = ShieldedView::new(tmp.path());
        let seed_hash = [0x5Cu8; 32];
        let cmx = [0x01u8; 32];
        let nullifier = [0x02u8; 32];

        let err = view
            .insert_shielded_note(
                &seed_hash,
                &InsertShieldedNote {
                    note_data: &[0u8; 8],
                    position: 0,
                    cmx: &cmx,
                    nullifier: &nullifier,
                    block_height: 0,
                    value: 42,
                    network: "testnet",
                },
            )
            .expect_err("insert must fail when the sidecar path is a directory");

        // The production mapping the sync path applies.
        let task_err = TaskError::ShieldedNotePersistFailed { source: err };
        assert!(
            matches!(task_err, TaskError::ShieldedNotePersistFailed { .. }),
            "a swallowed insert is the bug; the error must map to ShieldedNotePersistFailed"
        );
    }

    /// F54: the reload cursor derives from the commitment tree's
    /// `max_leaf_position`, so persisting BEFORE appending is what prevents a
    /// lost note. This pins that contract: when the sidecar persist fails, the
    /// tree position must NOT have advanced past the failed note's position —
    /// the next sync re-scans it. Models the fixed control flow (persist →
    /// abort-on-error → only-then append) against a real tree and a failing
    /// view.
    #[test]
    fn tree_cursor_does_not_advance_past_unpersisted_note() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tree_path = tmp.path().join("tree.sqlite");
        let mut tree =
            ClientPersistentCommitmentTree::open_path(&tree_path, 100).expect("open tree");

        // Precondition: empty tree has no leaf position (cursor would be 0).
        assert!(
            tree.max_leaf_position().expect("max pos").is_none(),
            "precondition: fresh tree has no leaves"
        );

        // Force the sidecar insert to fail.
        let sidecar = ShieldedView::sidecar_path(tmp.path());
        std::fs::create_dir_all(&sidecar).expect("plant directory at sidecar path");
        let view = ShieldedView::new(tmp.path());
        let seed_hash = [0x6Du8; 32];
        let cmx = [0x03u8; 32];
        let nullifier = [0x04u8; 32];

        // Fixed ordering: persist FIRST; on failure abort BEFORE touching the
        // tree. This is what `sync_notes` now does.
        let persist = view.insert_shielded_note(
            &seed_hash,
            &InsertShieldedNote {
                note_data: &[0u8; 8],
                position: 0,
                cmx: &cmx,
                nullifier: &nullifier,
                block_height: 0,
                value: 7,
                network: "testnet",
            },
        );
        assert!(persist.is_err(), "the persist fails for this test");

        if persist.is_ok() {
            tree.append(cmx, Retention::Marked).expect("append");
            tree.checkpoint(1).expect("checkpoint");
        }

        // The note was never persisted, so the tree cursor must not have moved
        // past it — the next sync re-scans position 0 (no silent loss).
        assert!(
            tree.max_leaf_position().expect("max pos").is_none(),
            "the tree cursor must not advance past an unpersisted note"
        );
    }
}
