//! Time-based retention of storage-upgrade backups.

use super::*;
use std::time::SystemTime;

/// What one retention pass did.
#[derive(Debug, Default)]
pub struct BackupPruneReport {
    /// Expired backups deleted. Undercounts when a location fails part-way.
    pub deleted: usize,
    /// The first failure; the remaining expired backups are retried the next time
    /// wallet data opens.
    pub failure: Option<TaskError>,
}

/// Every database in `data_dir` whose upgrade backups the global retention covers:
/// the shared app database and each network's wallet database.
pub(crate) fn retained_backup_databases(data_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::iter::once(crate::wallet_backend::app_database_path(data_dir))
        .chain(
            crate::wallet_backend::all_networks()
                .into_iter()
                .map(|network| crate::wallet_backend::wallet_database_path(data_dir, network)),
        )
        .collect()
}

impl AppContext {
    /// Delete upgrade and legacy migration backups older than the configured retention period.
    ///
    /// The policy is global: it covers every database in the data directory (the app
    /// database and every network's wallet database, bridge and upstream
    /// `pre-migration-*` snapshots), plus legacy `data.db` backups, whichever network
    /// this context serves. The newest usable backup of each database is always kept,
    /// so the pass is safe for databases this process has not opened. A database
    /// another context is opening or upgrading is skipped until the next pass.
    ///
    /// Every policy, including keeping backups forever, also removes legacy copies a
    /// crash left unpublished (see
    /// [`sweep_pending`](crate::database::legacy_backups::sweep_pending)); those are not
    /// counted as deleted backups.
    ///
    /// Attempts every location and reports the first failure. Nothing is deleted when the
    /// policy cannot be read.
    pub(crate) fn prune_expired_upgrade_backups(&self) -> BackupPruneReport {
        self.prune_expired_upgrade_backups_at(SystemTime::now())
    }

    /// [`Self::prune_expired_upgrade_backups`] as if the clock read `now`.
    pub(crate) fn prune_expired_upgrade_backups_at(&self, now: SystemTime) -> BackupPruneReport {
        let mut report = BackupPruneReport::default();
        let retention = match self.backup_retention() {
            Ok(retention) => retention,
            Err(source) => {
                report.failure = Some(TaskError::BackupRetentionRead { source });
                return report;
            }
        };
        // Crash-left legacy copies were never published backups, so every policy
        // removes them.
        match crate::database::legacy_backups::sweep_pending(self.data_dir(), now) {
            Ok(0) => {}
            Ok(swept) => tracing::info!(swept, "Removed unfinished legacy backup copies"),
            Err(source) => {
                report
                    .failure
                    .get_or_insert(TaskError::UpgradeBackupCleanup { source });
            }
        }
        let Some(max_age) = retention.max_age() else {
            return report;
        };
        let outcomes = retained_backup_databases(self.data_dir())
            .into_iter()
            .map(|database| {
                crate::wallet_backend::platform_compatibility::prune_expired_backups(
                    &database, max_age, now,
                )
            })
            .chain(std::iter::once(
                crate::database::legacy_backups::prune_expired(self.data_dir(), max_age, now),
            ));
        for outcome in outcomes {
            match outcome {
                Ok(count) => report.deleted += count,
                Err(source) => {
                    report
                        .failure
                        .get_or_insert(TaskError::UpgradeBackupCleanup { source });
                }
            }
        }
        if report.deleted > 0 {
            tracing::info!(
                deleted = report.deleted,
                ?retention,
                "Deleted expired upgrade backups"
            );
        }
        report
    }

    /// [`Self::prune_expired_upgrade_backups`] with any failure logged; it never fails
    /// the surrounding operation.
    pub(crate) fn prune_expired_upgrade_backups_best_effort(&self) -> BackupPruneReport {
        let report = self.prune_expired_upgrade_backups();
        match &report.failure {
            Some(error @ TaskError::BackupRetentionRead { .. }) => tracing::warn!(
                ?error,
                "Backup retention setting unreadable; no upgrade backups are deleted until the setting is saved again"
            ),
            Some(error) => tracing::warn!(
                ?error,
                "Expired upgrade backups remain; retrying when wallet data next opens"
            ),
            None => {}
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use crate::context::test_support::test_app_context;
    use crate::model::backup_retention::BackupRetention;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn pruned(ctx: &crate::context::AppContext) -> usize {
        let report = ctx.prune_expired_upgrade_backups();
        assert!(report.failure.is_none(), "{:?}", report.failure);
        report.deleted
    }

    fn write_aged(path: &Path, age: Duration) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"backup").unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    struct Fixture {
        expired: Vec<std::path::PathBuf>,
        fresh: Vec<std::path::PathBuf>,
    }

    fn backups(ctx: &crate::context::AppContext) -> Fixture {
        let dir = ctx.data_dir();
        let [app, wallet] = [
            crate::wallet_backend::app_database_path(dir),
            crate::wallet_backend::wallet_database_path(dir, ctx.network),
        ];
        let bridge = |database: &Path, suffix: &str| {
            database.with_file_name(format!(
                "{}.platform-67d4ef3-backup-{suffix}.sqlite",
                database.file_name().unwrap().to_string_lossy()
            ))
        };
        let expired = vec![
            bridge(&app, "old"),
            bridge(&wallet, "old"),
            dir.join("backups/data_backup_20000101_000000.db"),
        ];
        let fresh = vec![
            bridge(&app, "new"),
            bridge(&wallet, "new"),
            dir.join("backups")
                .join(crate::database::legacy_backups::backup_file_name(
                    chrono::Utc::now(),
                )),
        ];
        for path in &expired {
            write_aged(path, DAY * 120);
        }
        for path in &fresh {
            write_aged(path, DAY);
        }
        Fixture { expired, fresh }
    }

    #[test]
    fn startup_retention_deletes_only_expired_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        let fixture = backups(&ctx);

        assert_eq!(pruned(&ctx), 3);

        assert!(fixture.expired.iter().all(|path| !path.exists()));
        assert!(fixture.fresh.iter().all(|path| path.exists()));
    }

    /// A legacy copy left unpublished by a crash is removed by every retention pass,
    /// whatever the policy, once it is clearly not a copy still being written. It
    /// is not a backup, so it is not counted as a deleted one.
    #[test]
    fn crash_left_legacy_copies_are_swept_under_any_policy() {
        for policy in [BackupRetention::KeepForever, BackupRetention::default()] {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            ctx.set_backup_retention(policy).unwrap();
            let backups = ctx.data_dir().join("backups");
            let crashed = backups.join("data_backup_20200101_000000.db.a1B2c3.pending");
            let in_progress = backups.join("data_backup_20200102_000000.db.Z9y8X7.pending");
            write_aged(&crashed, DAY * 2);
            write_aged(&in_progress, Duration::from_secs(5));

            assert_eq!(pruned(&ctx), 0, "{policy:?}");

            assert!(!crashed.exists(), "{policy:?}: a crash-left copy is swept");
            assert!(
                in_progress.exists(),
                "{policy:?}: a recent copy may still be written"
            );
        }
    }

    #[test]
    fn keep_forever_deletes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_backup_retention(BackupRetention::KeepForever)
            .unwrap();
        let fixture = backups(&ctx);

        assert_eq!(pruned(&ctx), 0);

        assert!(fixture.expired.iter().all(|path| path.exists()));
    }

    #[test]
    fn shorter_retention_deletes_more() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        ctx.set_backup_retention(BackupRetention::DeleteAfterDays(200))
            .unwrap();
        let fixture = backups(&ctx);
        assert_eq!(pruned(&ctx), 0);

        ctx.set_backup_retention(BackupRetention::DeleteAfterDays(30))
            .unwrap();
        assert_eq!(pruned(&ctx), 3);
        assert!(fixture.fresh.iter().all(|path| path.exists()));
    }

    #[test]
    fn retention_is_global_across_networks() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        let other = dash_sdk::dpp::dashcore::Network::Mainnet;
        assert_ne!(ctx.network, other);
        let database = crate::wallet_backend::wallet_database_path(ctx.data_dir(), other);
        let auto = platform_wallet_storage::default_auto_backup_dir(&database);
        let expired = auto.join("pre-migration-det-mainnet-1-to-2-20000101T000000Z.db");
        let newest = auto.join("pre-migration-det-mainnet-2-to-3-20000102T000000Z.db");
        write_aged(&expired, DAY * 400);
        write_aged(&newest, DAY * 300);

        assert_eq!(pruned(&ctx), 1);
        assert!(
            !expired.exists(),
            "another network's expired backup is pruned"
        );
        assert!(
            newest.exists(),
            "the newest backup of each database is kept"
        );
        assert!(
            !crate::wallet_backend::platform_compatibility::backup_lock_path(
                &crate::wallet_backend::wallet_database_path(
                    ctx.data_dir(),
                    dash_sdk::dpp::dashcore::Network::Devnet
                )
            )
            .unwrap()
            .exists(),
            "a database without backups is not locked or touched"
        );
    }

    #[test]
    fn newest_backup_of_each_database_survives_a_far_future_clock() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        let fixture = backups(&ctx);
        let far_future = SystemTime::now() + DAY * 365 * 100;

        let report = ctx.prune_expired_upgrade_backups_at(far_future);
        assert!(report.failure.is_none(), "{:?}", report.failure);
        assert_eq!(report.deleted, 3);
        assert!(fixture.fresh.iter().all(|path| path.exists()));
        assert!(fixture.expired.iter().all(|path| !path.exists()));
        let again = ctx.prune_expired_upgrade_backups_at(far_future);
        assert_eq!(again.deleted, 0, "the last backups are never deleted");
        assert!(fixture.fresh.iter().all(|path| path.exists()));
    }

    #[test]
    fn database_in_use_is_skipped_not_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = test_app_context(tmp.path());
        let fixture = backups(&ctx);
        let wallet = crate::wallet_backend::wallet_database_path(ctx.data_dir(), ctx.network);
        let lock =
            crate::wallet_backend::platform_compatibility::backup_lock_path(&wallet).unwrap();
        let held = std::fs::File::create(&lock).unwrap();
        held.try_lock().unwrap();

        let report = ctx.prune_expired_upgrade_backups();
        assert!(report.failure.is_none(), "{:?}", report.failure);
        assert_eq!(
            report.deleted, 2,
            "app and legacy backups; the locked wallet is skipped"
        );
        assert!(fixture.expired[1].exists());
    }
}
