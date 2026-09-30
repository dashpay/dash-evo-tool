//! Time-based retention of storage-upgrade backups.

use super::*;
use std::time::SystemTime;

/// What one retention pass did.
#[derive(Debug, Default)]
pub struct BackupPruneReport {
    /// Expired backups deleted. Undercounts when a location fails part-way.
    pub deleted: usize,
    /// The first failure; the remaining expired backups are retried at the next start.
    pub failure: Option<TaskError>,
}

impl AppContext {
    /// Delete upgrade and legacy migration backups older than the configured retention period.
    ///
    /// Covers the app database and this network's wallet database (bridge and upstream
    /// `pre-migration-*` snapshots), plus legacy `data.db` backups. Call only after those
    /// databases opened successfully: an expired snapshot may be the last one, which is
    /// expendable only once the database it protects is known to work.
    ///
    /// Attempts every location and reports the first failure. Nothing is deleted when the
    /// policy cannot be read.
    pub(crate) fn prune_expired_upgrade_backups(&self) -> BackupPruneReport {
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
        let now = SystemTime::now();
        let outcomes = self
            .upgrade_backup_databases()
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

    /// [`Self::prune_expired_upgrade_backups`] with any failure logged; it is retried on
    /// the next start and never fails the surrounding operation.
    pub(crate) fn prune_expired_upgrade_backups_best_effort(&self) -> BackupPruneReport {
        let report = self.prune_expired_upgrade_backups();
        if let Some(error) = &report.failure {
            tracing::warn!(
                ?error,
                "Expired upgrade backups remain; retrying on next start"
            );
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
        let [app, wallet] = ctx.upgrade_backup_databases();
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
        let fresh = vec![bridge(&app, "new"), bridge(&wallet, "new")];
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
}
