//! "Restore from Previous Version": re-import what the preserved legacy
//! `data.db` still holds but this version's stores lack.
//!
//! User-initiated and repeatable. Wallet seed envelopes are copied only where
//! the vault has no copy of that seed (never overwritten, protected envelopes
//! travel as-is), and identity keys go through the per-identity #889 recovery,
//! which restores only missing items and never resurrects a deleted identity.
//! `data.db` is opened read-only; the one-time drain sentinel is not consulted
//! or changed.

use std::path::Path;
use std::sync::Arc;

use super::finish_unwire::{
    LegacyNetworkRows, MigrationError, migrate_wallet_meta_rows_from_conn,
    migrate_wallet_seeds_rows_from_conn, open_legacy_read_only,
};
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::legacy_recovery::{RecoveryItem, compute_recovery_plan};
use crate::model::legacy_restore::LegacyRestoreSummary;
use crate::model::settings::legacy_network_names;

const LOG_TARGET: &str = "migration::legacy_restore";

/// Run the restore for the active network and report what happened.
///
/// # Errors
///
/// [`TaskError::WalletStorageNotReady`] while the storage update or another
/// restore runs; [`TaskError::LegacyRestoreFailed`] when the legacy database
/// cannot be opened or its wallet table cannot be read. Per-row and
/// per-identity failures are counted in the summary instead.
pub(crate) async fn run(app_context: &Arc<AppContext>) -> Result<LegacyRestoreSummary, TaskError> {
    let mut summary = LegacyRestoreSummary::default();
    let Some(path) = app_context.db.db_file_path().filter(|path| path.exists()) else {
        tracing::info!(
            target = LOG_TARGET,
            "No database from an earlier version exists; nothing to restore"
        );
        return Ok(summary);
    };
    // A fresh install also has a `data.db`, so the file existing says nothing;
    // what counts is whether it holds rows for this network.
    let conn = open_legacy_read_only(&path).map_err(restore_failed)?;
    let has_rows = legacy_rows_present(&conn, app_context.network).map_err(restore_failed)?;
    drop(conn);
    if !has_rows {
        tracing::info!(
            target = LOG_TARGET,
            "The earlier version's database holds no wallets or identities for this network; nothing to restore"
        );
        return Ok(summary);
    }
    summary.legacy_database_found = true;

    restore_wallets(app_context, &path, &mut summary).await?;
    // Wallets first: a wallet-derived identity key is only restorable once the
    // wallet it names is held by this install.
    restore_identity_keys(app_context, &mut summary).await;

    tracing::info!(target = LOG_TARGET, ?summary, network = ?app_context.network, "Restore from the earlier version finished");
    Ok(summary)
}

/// Dispatch wrapper returning the task result envelope.
pub(crate) async fn run_task(
    app_context: &Arc<AppContext>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    run(app_context)
        .await
        .map(BackendTaskSuccessResult::PreviousVersionRestored)
}

/// Copy missing wallet seed envelopes (and their metadata) back, then make the
/// restored wallets live without a restart.
async fn restore_wallets(
    app_context: &Arc<AppContext>,
    path: &Path,
    summary: &mut LegacyRestoreSummary,
) -> Result<(), TaskError> {
    // Same exclusion the drain holds, so a restore never interleaves with it.
    let _gate = app_context
        .try_lock_prepare_gate()
        .map_err(|_| TaskError::WalletStorageNotReady)?;
    if app_context.migration_status().state().is_in_progress() {
        return Err(TaskError::WalletStorageNotReady);
    }

    let backend = app_context.wallet_backend()?;
    let conn = open_legacy_read_only(path).map_err(restore_failed)?;
    let copy = copy_missing_wallets(&backend, &conn, app_context.network.into())
        .map_err(restore_failed)?;

    summary.wallets_restored = copy.seeds_restored;
    summary.wallets_already_present = copy.seeds_already_present;
    summary.wallets_skipped_malformed = copy.skipped_malformed;
    summary.wallets_failed = copy.failed;

    make_copied_wallets_live(app_context, &backend, &copy).await
}

/// Counters of one add-only [`copy_missing_wallets`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct MissingWalletsCopy {
    /// Seed envelopes written because the vault had no copy.
    pub(super) seeds_restored: u32,
    /// Seeds the vault already held, left untouched.
    pub(super) seeds_already_present: u32,
    /// Metadata entries written beside a stored seed that lacked one.
    pub(super) metas_restored: u32,
    /// Damaged rows skipped.
    pub(super) skipped_malformed: u32,
    /// Rows that could not be read or written.
    pub(super) failed: u32,
}

/// Copy the wallet seed envelopes and metadata `rows` selects that this
/// install lacks. Add-only: nothing present is overwritten or removed, and a
/// protected envelope travels as-is, so it stays protected. The legacy
/// connection is only read.
///
/// # Errors
///
/// [`MigrationError`] when a legacy wallet table cannot be read at all;
/// per-row problems are counted instead.
pub(super) fn copy_missing_wallets(
    backend: &crate::wallet_backend::WalletBackend,
    conn: &rusqlite::Connection,
    rows: LegacyNetworkRows,
) -> Result<MissingWalletsCopy, MigrationError> {
    let network = rows.network();
    let seeds = backend.wallet_seeds();
    let metas = backend.wallet_meta();

    let mut seeds_restored = 0u32;
    let seed_outcome = migrate_wallet_seeds_rows_from_conn(
        conn,
        |seed_hash, envelope| {
            if seeds.contains(&seed_hash)? {
                return Ok(());
            }
            seeds.set(&seed_hash, &envelope)?;
            seeds_restored += 1;
            Ok(())
        },
        rows,
    )?;

    // Hydration is driven by wallet metadata, so a seed without it stays
    // invisible. Write it only where missing, and only beside a stored seed.
    let mut metas_restored = 0u32;
    // Callback failures (probe or write) are counted here; the pass's own
    // `failed` also counts undecodable rows the seed pass already counted.
    let mut meta_callback_failures = 0u32;
    migrate_wallet_meta_rows_from_conn(
        conn,
        |seed_hash, meta| {
            let restore_meta = || -> Result<bool, TaskError> {
                if metas.try_get(network, &seed_hash)?.is_some() || !seeds.contains(&seed_hash)? {
                    return Ok(false);
                }
                metas.set_migrated(network, &seed_hash, &meta)?;
                Ok(true)
            };
            match restore_meta() {
                Ok(written) => {
                    metas_restored += u32::from(written);
                    Ok(())
                }
                Err(error) => {
                    meta_callback_failures += 1;
                    Err(error)
                }
            }
        },
        rows,
    )?;

    Ok(MissingWalletsCopy {
        seeds_restored,
        seeds_already_present: seed_outcome.imported.saturating_sub(seeds_restored),
        metas_restored,
        skipped_malformed: seed_outcome.skipped_malformed,
        failed: seed_outcome.failed.saturating_add(meta_callback_failures),
    })
}

/// Make wallets a [`copy_missing_wallets`] pass wrote live without a restart.
/// Open wallets register upstream; a protected one stays closed until the
/// user unlocks it, and that unlock registers it — no prompt here.
pub(super) async fn make_copied_wallets_live(
    app_context: &Arc<AppContext>,
    backend: &crate::wallet_backend::WalletBackend,
    copy: &MissingWalletsCopy,
) -> Result<(), TaskError> {
    if copy.seeds_restored > 0 || copy.metas_restored > 0 {
        backend.hydrate_context_wallets(app_context)?;
        app_context.bootstrap_loaded_wallets().await;
    }
    Ok(())
}

/// Restore every missing key the legacy copy of each local identity still
/// holds. Failures and declined password prompts are counted per identity,
/// never fatal for the run.
async fn restore_identity_keys(app_context: &Arc<AppContext>, summary: &mut LegacyRestoreSummary) {
    let identity_ids = match app_context.local_identity_ids() {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(
                target = LOG_TARGET,
                ?error,
                "Could not list local identities to restore keys into"
            );
            summary.identities_failed = summary.identities_failed.saturating_add(1);
            return;
        }
    };

    for identity_id in identity_ids {
        let approved = match restorable_items(app_context, identity_id) {
            Ok(approved) => approved,
            Err(error) => {
                tracing::warn!(target = LOG_TARGET, identity = %identity_id, ?error, "Could not check an identity for keys saved by the earlier version");
                summary.identities_failed = summary.identities_failed.saturating_add(1);
                continue;
            }
        };
        if approved.is_empty() {
            continue;
        }
        match app_context
            .recover_legacy_identity_data(identity_id, approved)
            .await
        {
            Ok(BackendTaskSuccessResult::LegacyRecoveryCompleted { applied, .. }) => {
                let keys = applied
                    .iter()
                    .filter(|item| matches!(item.item, RecoveryItem::Key { .. }))
                    .count();
                if !applied.is_empty() {
                    summary.identities_updated = summary.identities_updated.saturating_add(1);
                }
                summary.identity_keys_restored = summary
                    .identity_keys_restored
                    .saturating_add(u32::try_from(keys).unwrap_or(u32::MAX));
            }
            Ok(_) => {}
            Err(TaskError::SecretPromptCancelled) => {
                tracing::info!(target = LOG_TARGET, identity = %identity_id, "Skipped restoring an identity's keys because its password prompt was declined");
                summary.identities_skipped = summary.identities_skipped.saturating_add(1);
            }
            Err(error) => {
                tracing::warn!(target = LOG_TARGET, identity = %identity_id, ?error, "Could not restore keys saved by the earlier version into an identity");
                summary.identities_failed = summary.identities_failed.saturating_add(1);
            }
        }
    }
}

/// The full allowlist of items the legacy copy could restore, empty when the
/// legacy file holds nothing for this identity.
fn restorable_items(
    app_context: &AppContext,
    identity_id: dash_sdk::platform::Identifier,
) -> Result<Vec<RecoveryItem>, TaskError> {
    let Some(modern) = app_context.get_local_qualified_identity(&identity_id)? else {
        return Ok(Vec::new());
    };
    let Some(mut legacy) = app_context.legacy_identity_record(identity_id)? else {
        return Ok(Vec::new());
    };
    let plan = compute_recovery_plan(&modern, &legacy);
    // The decoded legacy plaintext is not needed past planning; wipe it.
    let _ = legacy.private_keys.take_plaintext_for_vault();
    Ok(plan.approved_items())
}

/// Whether the legacy database holds any wallet or identity row for `network`.
fn legacy_rows_present(
    conn: &rusqlite::Connection,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<bool, MigrationError> {
    for table in ["wallet", "identity"] {
        let read_error = |source| MigrationError::LegacyDbRead { table, source };
        if !crate::database::table_exists(conn, table).map_err(read_error)? {
            continue;
        }
        let present: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE network IN (?1, ?2))"),
                legacy_network_names(network),
                |row| row.get(0),
            )
            .map_err(read_error)?;
        if present {
            return Ok(true);
        }
    }
    Ok(false)
}

fn restore_failed(source: MigrationError) -> TaskError {
    TaskError::LegacyRestoreFailed {
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::{
        legacy_master_epk_bytes, seed_legacy_unprotected_hd_wallet_row,
    };
    use crate::model::wallet::{ClosedKeyItem, WalletSeedHash};
    use dash_sdk::dpp::dashcore::Network;

    fn app_context(dir: &Path) -> Arc<AppContext> {
        app_context_on(dir, Network::Testnet)
    }

    fn app_context_on(dir: &Path, network: Network) -> Arc<AppContext> {
        crate::app_dir::ensure_env_file(dir);
        let db = Arc::new(crate::database::Database::new(dir.join("data.db")).expect("db"));
        db.create_tables(true).expect("create tables");
        db.set_default_version().expect("set version");
        AppContext::new(
            dir.to_path_buf(),
            network,
            db,
            Default::default(),
            Default::default(),
            egui::Context::default(),
            AppContext::open_app_kv(dir).expect("app kv"),
            AppContext::open_secret_store(dir).expect("secret store"),
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("AppContext")
    }

    async fn wire_backend(ctx: &Arc<AppContext>) {
        let (tx, _rx) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire backend");
    }

    fn stage_wallet(ctx: &AppContext, seed: &[u8; 64], alias: &str) -> WalletSeedHash {
        let seed_hash = ClosedKeyItem::compute_seed_hash(seed);
        let epk = legacy_master_epk_bytes(seed, Network::Testnet);
        seed_legacy_unprotected_hd_wallet_row(
            &ctx.db,
            &seed_hash,
            seed,
            &epk,
            alias,
            Network::Testnet,
        )
        .expect("stage legacy wallet");
        seed_hash
    }

    /// A row hydration would drop (no xpub) and a row whose seed hash is the
    /// wrong size.
    fn stage_broken_rows(ctx: &AppContext) {
        let insert = "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, \
            master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint, \
            network, core_wallet_name) VALUES (?1, ?2, x'', x'', ?3, 'Broken', 0, 0, NULL, 'testnet', NULL)";
        ctx.db
            .execute(
                insert,
                rusqlite::params![
                    [0xE1u8; 32].as_slice(),
                    [0u8; 64].as_slice(),
                    Vec::<u8>::new()
                ],
            )
            .expect("stage malformed row");
        ctx.db
            .execute(
                insert,
                rusqlite::params![
                    [0xE2u8; 31].as_slice(),
                    [0u8; 64].as_slice(),
                    Vec::<u8>::new()
                ],
            )
            .expect("stage undecodable row");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restores_a_missing_wallet_and_never_overwrites_an_existing_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context(dir.path());
        let missing = stage_wallet(&ctx, &[0xA1; 64], "Missing");
        let present = stage_wallet(&ctx, &[0xA2; 64], "Present");
        wire_backend(&ctx).await;
        let backend = ctx.wallet_backend().expect("backend");
        // A sentinel value for the existing seed: an overwrite would replace it.
        backend
            .wallet_seeds()
            .set_raw(&present, &[0x5A; 64])
            .expect("existing seed");

        let summary = run(&ctx).await.expect("restore");

        assert_eq!(
            summary,
            LegacyRestoreSummary {
                legacy_database_found: true,
                wallets_restored: 1,
                wallets_already_present: 1,
                ..Default::default()
            }
        );
        assert!(backend.wallet_seeds().contains(&missing).expect("probe"));
        assert_eq!(
            *backend
                .wallet_seeds()
                .get_raw(&present)
                .expect("read")
                .expect("kept"),
            [0x5A; 64],
            "an existing seed is never overwritten"
        );
        assert!(
            ctx.wallet_context().wallets().contains_key(&missing),
            "the restored wallet is live without a restart"
        );
        assert_eq!(
            backend
                .wallet_meta()
                .get(Network::Testnet, &missing)
                .map(|m| m.alias),
            Some("Missing".to_string())
        );
        assert!(dir.path().join("data.db").exists(), "data.db is kept");
        backend.shutdown().await;
    }

    /// A mainnet wallet saved by v0.9.x carries the pre-v29 `dash` network
    /// spelling; the restore must still find and restore it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restores_a_mainnet_wallet_saved_with_the_dash_network_spelling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context_on(dir.path(), Network::Mainnet);
        let seed = [0xA3; 64];
        let seed_hash = ClosedKeyItem::compute_seed_hash(&seed);
        seed_legacy_unprotected_hd_wallet_row(
            &ctx.db,
            &seed_hash,
            &seed,
            &legacy_master_epk_bytes(&seed, Network::Mainnet),
            "Mainnet",
            Network::Mainnet,
        )
        .expect("stage legacy wallet");
        ctx.db
            .execute("UPDATE wallet SET network = 'dash'", [])
            .expect("use the v0.9.x spelling");
        wire_backend(&ctx).await;

        let summary = run(&ctx).await.expect("restore");

        assert_eq!(
            summary,
            LegacyRestoreSummary {
                legacy_database_found: true,
                wallets_restored: 1,
                ..Default::default()
            }
        );
        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_run_restores_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context(dir.path());
        stage_wallet(&ctx, &[0xB1; 64], "Once");
        wire_backend(&ctx).await;

        assert_eq!(run(&ctx).await.expect("first").wallets_restored, 1);
        let second = run(&ctx).await.expect("second");
        assert_eq!(second.wallets_restored, 0);
        assert_eq!(second.wallets_already_present, 1);
        assert!(!second.restored_anything());
        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn damaged_and_unreadable_rows_are_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context(dir.path());
        stage_broken_rows(&ctx);
        wire_backend(&ctx).await;

        let summary = run(&ctx).await.expect("restore");
        assert_eq!(summary.wallets_skipped_malformed, 1);
        assert_eq!(summary.wallets_failed, 1);
        assert_eq!(summary.wallets_restored, 0);
        assert!(summary.has_problems());
        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture setup outside any production deletion path"
    )]
    async fn no_legacy_database_reports_nothing_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context(dir.path());
        wire_backend(&ctx).await;
        std::fs::remove_file(dir.path().join("data.db")).expect("remove data.db");

        let summary = run(&ctx).await.expect("restore");
        assert_eq!(summary, LegacyRestoreSummary::default());
        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A fresh install's `data.db` exists but holds no earlier-version rows,
    /// which is reported as "nothing found", not "nothing needed".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_fresh_install_database_reports_nothing_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context(dir.path());
        wire_backend(&ctx).await;
        assert!(dir.path().join("data.db").exists());

        let summary = run(&ctx).await.expect("restore");
        assert_eq!(summary, LegacyRestoreSummary::default());
        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_storage_update_blocks_the_restore() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = app_context(dir.path());
        stage_wallet(&ctx, &[0xC1; 64], "Gate");
        wire_backend(&ctx).await;
        let _gate = ctx.try_lock_prepare_gate().expect("hold the gate");

        let error = run(&ctx).await.expect_err("gate held");
        assert!(
            matches!(error, TaskError::WalletStorageNotReady),
            "{error:?}"
        );
        ctx.wallet_backend().expect("backend").shutdown().await;
    }
}
