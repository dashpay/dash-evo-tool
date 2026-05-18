//! Platform-wallet one-time migration — Stage B (post-unlock async engine).
//!
//! Stage A (SQL v35, sync, pre-unlock; `initialization.rs`) only arms
//! `settings.platform_wallet_migration_pending` and writes the retained
//! `<db>.premigration` recovery floor. Stage B (here) does the actual data
//! migration once, post-unlock, async, marker-gated, behind the
//! `AppContext`-owned `tokio::sync::Mutex` (invariant C6).
//!
//! Back-compat for legacy DashPay DIP-14 derivation is DROPPED
//! (Decision #6 — `dip14-migration-hardstop.md` SUPERSEDED). Contacts are
//! re-established on UPSTREAM derivation unconditionally; there is no
//! DET re-derivation, no comparison, no quarantine. The accepted
//! fund-accessibility trade-off is covered by the mandatory one-time notice
//! (P3e), the sole compensating control.
//!
//! Each step is idempotent so a crash-and-relaunch re-runs cleanly. The
//! legacy-table DROP is the strictly-last destructive step and is performed
//! only after a durable upstream flush. Restore-from-`premigration` is a
//! launch-time concern (`Database::recover_from_premigration_if_corrupt`)
//! that fires ONLY on injected corruption — never on a healthy-but-pending
//! DB. These ordering & recovery invariants are pinned by the `p3d` test
//! module in `database::initialization`.

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::wallet_backend::WalletBackend;

/// Default DashPay account index. DET established contacts have always used
/// account 0 (the legacy DIP-14 path hardcoded it); upstream
/// `register_contact_account` re-derives on account 0 to match.
const DEFAULT_DASHPAY_ACCOUNT: u32 = 0;

/// Run the one-time Stage-B migration. Idempotent; marker-gated by the
/// caller. On success (and once [`LEGACY_DROP_ENABLED`]) clears
/// `platform_wallet_migration_pending`; on any error the marker is left set
/// (caller logs; next launch retries). Restore-from-`premigration` is the
/// caller/launch responsibility and happens ONLY on an exceptional path,
/// never here.
pub async fn run_stage_b(ctx: &Arc<AppContext>, backend: &WalletBackend) -> Result<(), TaskError> {
    tracing::info!("Platform-wallet Stage-B migration: starting");

    // Step 1 — backup precondition. The retained recovery floor must exist
    // before any later (eventually destructive) step. `initialize()`
    // (re)creates it idempotently on every post-marker launch; re-assert it
    // here so Stage B never proceeds without the floor.
    if let Some(path) = ctx.db.db_file_path() {
        let backup = crate::database::Database::premigration_backup_path(&path);
        if !backup.exists() {
            return Err(TaskError::WalletBackend {
                source: Box::new(platform_wallet::error::PlatformWalletError::WalletCreation(
                    "pre-migration backup missing; refusing to migrate without recovery floor"
                        .to_string(),
                )),
            });
        }
    }

    // Step 2 — re-register every wallet from seed (idempotent; skips
    // already-registered, tolerates upstream WalletAlreadyExists). Upstream
    // `create_wallet_from_seed_bytes` also runs identity discovery from
    // chain, which is Step 3's mechanism.
    backend.ensure_wallets_registered(ctx).await?;

    // Step 3 — identities. The DET `QualifiedIdentity` blob and the
    // identity/platform-address/token tables are RETAINED (upstream "Outside
    // scope", data-model-and-migration.md conversion surface). Upstream
    // `IdentityManager` is repopulated by the chain-driven identity sync that
    // step 2's wallet registration already performed — no separate
    // low-level `add_identity` push is needed (and it would risk
    // IdentityAlreadyExists against that sync). Intentional no-op.

    // Step 4 — re-establish every accepted DashPay contact on UPSTREAM
    // derivation only. Idempotent: upstream `register_contact_account`
    // no-ops if the contact account already exists. No DET re-derivation,
    // no comparison, no quarantine (Decision #6).
    //
    // The owning wallet for a contact is the one whose seed derives the
    // contact's owner identity. DET tracks that linkage as
    // (QualifiedIdentity, owner-wallet seed_hash); build owner-id →
    // seed_hash so each contact account is registered on the right wallet.
    let owner_seed: std::collections::HashMap<Identifier, [u8; 32]> = ctx
        .db
        .get_local_user_identities(ctx)
        .map_err(|source| TaskError::Database { source })?
        .into_iter()
        .filter_map(|(qi, seed)| seed.map(|s| (qi.identity.id(), s)))
        .collect();

    let contacts = ctx
        .db
        .load_all_accepted_dashpay_contacts()
        .map_err(|source| TaskError::Database { source })?;
    tracing::info!(
        count = contacts.len(),
        "Stage-B: re-establishing DashPay contacts on upstream derivation"
    );
    for c in &contacts {
        let owner = Identifier::from_bytes(&c.owner_identity_id).map_err(|_| {
            invalid_identity("stored DashPay owner identity id is not a valid Identifier")
        })?;
        let contact = Identifier::from_bytes(&c.contact_identity_id).map_err(|_| {
            invalid_identity("stored DashPay contact identity id is not a valid Identifier")
        })?;
        let Some(seed_hash) = owner_seed.get(&owner) else {
            // Owner identity not linked to any local wallet (e.g. an
            // imported-by-id identity without its seed). The contact cannot
            // be re-established on upstream derivation; skip it — the
            // accepted-trade-off notice (P3e) already informs the user that
            // some DashPay state may not carry over.
            tracing::warn!(
                owner = %owner,
                "Stage-B: accepted DashPay contact has no local owner wallet; skipping"
            );
            continue;
        };
        // A per-contact derivation/registration failure aborts Stage B with
        // the marker left set (idempotent re-run next launch) rather than
        // silently skipping — DashPay correctness is not best-effort.
        backend
            .register_dashpay_contact(seed_hash, &owner, &contact, DEFAULT_DASHPAY_ACCOUNT)
            .await?;
    }

    // Step 5 — finalize (SINGLE fork: success). The exception fork is
    // implicit: any `?` above returns Err with the marker still set, so the
    // next launch re-runs Stage B (idempotent) and the launch-time recovery
    // (`Database::recover_from_premigration_if_corrupt`) restores from
    // `premigration` only if the new state is corrupt.
    //
    // Ordering is the P3d-proven invariant: durable upstream flush BEFORE the
    // destructive drop, then DROP as the strictly-last step, then clear the
    // marker. If we crash after the flush but before the drop, the next
    // launch re-runs Stage B: registration/contacts are idempotent no-ops and
    // the drop is `DROP TABLE IF EXISTS`. If we crash after the drop but
    // before clearing the marker, the next launch re-runs and the idempotent
    // drop is a clean no-op, then the marker clears.
    backend.flush_persister().await?;
    ctx.db
        .drop_legacy_migrated_tables()
        .map_err(|source| TaskError::Database { source })?;
    // Record completion BEFORE clearing the pending marker so that, if we
    // crash between the two writes, the next launch still re-runs Stage B
    // (pending stays set) and the idempotent finalise re-asserts both bits.
    // `completed` distinguishes a migrated user from a fresh install for the
    // one-time post-migration notice.
    ctx.db
        .set_platform_wallet_migration_completed(true)
        .map_err(|source| TaskError::Database { source })?;
    ctx.db
        .set_platform_wallet_migration_pending(false)
        .map_err(|source| TaskError::Database { source })?;
    tracing::info!("Platform-wallet Stage-B migration: complete (legacy tables dropped)");

    Ok(())
}

fn invalid_identity(msg: &str) -> TaskError {
    TaskError::WalletBackend {
        source: Box::new(
            platform_wallet::error::PlatformWalletError::InvalidIdentityData(msg.to_string()),
        ),
    }
}
