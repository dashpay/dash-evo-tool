use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded::ShieldedWalletState;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Fetch;
use dash_sdk::query_types::{ShieldedNullifierStatuses, ShieldedNullifiersQuery};
use std::sync::Arc;

/// Check which unspent notes have been spent on-chain by querying their nullifiers.
///
/// For each unspent note, queries the platform for the nullifier status.
/// Notes whose nullifiers are found spent are marked as such in both
/// the in-memory state and the database.
pub async fn check_nullifiers(
    app_context: &Arc<AppContext>,
    seed_hash: &WalletSeedHash,
    shielded_state: &mut ShieldedWalletState,
    network: Network,
) -> Result<u32, String> {
    let sdk = {
        let guard = app_context.sdk.read().unwrap();
        guard.clone()
    };

    let network_str = network.to_string();

    // Collect nullifiers of unspent notes
    let unspent_nullifiers: Vec<Vec<u8>> = shielded_state
        .notes
        .iter()
        .filter(|n| !n.is_spent)
        .map(|n| n.nullifier.to_bytes().to_vec())
        .collect();

    if unspent_nullifiers.is_empty() {
        return Ok(0);
    }

    let query = ShieldedNullifiersQuery(unspent_nullifiers);

    let statuses: Option<ShieldedNullifierStatuses> = ShieldedNullifierStatuses::fetch(&sdk, query)
        .await
        .map_err(|e| format!("Failed to fetch nullifier statuses: {e}"))?;

    let statuses = match statuses {
        Some(s) => s.0,
        None => return Ok(0),
    };

    let mut spent_count = 0u32;

    for status in statuses {
        if status.is_spent {
            let nullifier_bytes: [u8; 32] = status
                .nullifier
                .try_into()
                .map_err(|_| "Invalid nullifier length")?;

            // Mark as spent in memory
            for note in &mut shielded_state.notes {
                if !note.is_spent && note.nullifier.to_bytes() == nullifier_bytes {
                    note.is_spent = true;
                    spent_count += 1;

                    // Mark as spent in DB
                    let _ = app_context.db.mark_shielded_note_spent(
                        seed_hash,
                        &nullifier_bytes,
                        &network_str,
                    );
                }
            }
        }
    }

    if spent_count > 0 {
        shielded_state.recalculate_balance();
    }

    Ok(spent_count)
}
