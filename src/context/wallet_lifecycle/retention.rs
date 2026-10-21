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
    /// Delete expired upgrade and legacy backups of every database in the data directory,
    /// keeping each database's newest usable one; attempts all and reports the first failure.
    pub(crate) fn prune_expired_upgrade_backups(&self) -> BackupPruneReport {
        let now = SystemTime::now();
        let mut report = BackupPruneReport::default();
        let retention = match self.backup_retention() {
            Ok(retention) => retention,
            Err(source) => {
                report.failure = Some(TaskError::BackupRetentionRead { source });
                return report;
            }
        };
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
        let bridge = crate::wallet_backend::platform_compatibility::bridge_backup_path;
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
