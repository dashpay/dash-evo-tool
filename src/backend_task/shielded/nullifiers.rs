use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::ShieldedWalletState;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::drive::drive::shielded::paths::SHIELDED_NOTES_CHUNK_POWER;
use dash_sdk::platform::shielded::sync_shielded_notes;
use std::collections::HashSet;
use std::sync::Arc;

/// Server-enforced chunk size — the scan start must be a multiple of this.
/// Derived from the upstream consensus constant so the value can never drift.
const CHUNK_SIZE: u64 = 1u64 << SHIELDED_NOTES_CHUNK_POWER;

/// Detect which of our unspent notes have been spent on-chain via scan-based
/// nullifier matching.
///
/// In Orchard every action reveals the nullifier of the note it SPENT (the
/// output note's `rho` equals the input note's nullifier), so the set of
/// nullifiers carried on the scanned on-chain notes is exactly the set of
/// nullifiers that went on-chain over the scanned range. We fetch those notes,
/// build that set, and flip any owned note whose computed spending nullifier
/// appears in it. This mirrors the upstream wallet's `sync_notes_across`
/// spend-detection, which folds the same match into its note scan — there is no
/// dedicated nullifier-sync round-trip in the API anymore.
///
/// The scan resumes from `last_nullifier_sync_height` (a note-tree position,
/// rounded down to a chunk boundary): spend markers only ever land at new
/// positions on the append-only tree, so scanning `[watermark, tip]` each pass
/// observes every spend exactly once and never skips one. A note already marked
/// spent stays spent, so re-observing a nullifier is idempotent.
pub async fn check_nullifiers(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    shielded_state: &mut ShieldedWalletState,
    network: Network,
) -> Result<u32, TaskError> {
    if shielded_state.notes.iter().all(|n| n.is_spent) {
        return Ok(0);
    }

    let sdk = { app_context.sdk.load().as_ref().clone() };
    let network_str = network.to_string();
    let prepared_ivk = shielded_state.keys.ivk.prepare();

    // Round the resume position down to a chunk boundary so the server accepts
    // the query; the partial chunk re-fetch is harmless (idempotent matching).
    let aligned_start = (shielded_state.last_nullifier_sync_height / CHUNK_SIZE) * CHUNK_SIZE;

    let result = sync_shielded_notes(&sdk, &prepared_ivk, aligned_start, None)
        .await
        .map_err(|e| TaskError::ShieldedNullifierSyncFailed {
            detail: e.to_string(),
        })?;

    // The spend-marker set: every scanned on-chain note's `nullifier` field.
    let onchain_nullifiers: HashSet<[u8; 32]> = result
        .all_notes
        .iter()
        .filter_map(|n| <[u8; 32]>::try_from(n.nullifier.as_slice()).ok())
        .collect();

    let backend = app_context.wallet_backend().ok();
    let mut spent_count = 0u32;
    for note in &mut shielded_state.notes {
        if note.is_spent {
            continue;
        }
        if onchain_nullifiers.contains(&note.nullifier.to_bytes()) {
            note.is_spent = true;
            spent_count += 1;
            if let Some(backend) = backend.as_ref() {
                let _ = backend.shielded().mark_shielded_note_spent(
                    seed_hash,
                    &note.nullifier.to_bytes(),
                    &network_str,
                );
            }
        }
    }

    // Advance the resume cursor to the scanned tip (note position), so the next
    // pass only walks newly appended positions. `block_height` rides along for
    // diagnostics / display continuity.
    shielded_state.last_nullifier_sync_height =
        aligned_start.saturating_add(result.total_notes_scanned);
    shielded_state.last_nullifier_sync_timestamp = result.block_height;
    if let Some(backend) = backend.as_ref() {
        let _ = backend.shielded().set_nullifier_sync_info(
            seed_hash,
            &network_str,
            shielded_state.last_nullifier_sync_height,
            shielded_state.last_nullifier_sync_timestamp,
        );
    }

    if spent_count > 0 {
        shielded_state.recalculate_balance();
    }

    Ok(spent_count)
}
