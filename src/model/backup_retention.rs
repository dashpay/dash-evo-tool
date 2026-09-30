//! Time-based retention policy for storage-upgrade backups.
//!
//! Upgrade backups are copies of the app and wallet databases taken before a
//! storage upgrade. They exist for recovery only, so they are deleted once
//! they are older than the retention period. Pure data and validation — the
//! filesystem work lives in `wallet_backend::platform_compatibility` and
//! `database::legacy_backups`.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// Seconds in one retention day.
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// How long storage-upgrade backups are kept before they are deleted automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackupRetention {
    /// Never delete upgrade backups automatically.
    KeepForever,
    /// Delete upgrade backups older than this many days.
    DeleteAfterDays(u16),
}

impl BackupRetention {
    /// Global k/v key of the persisted policy. A dedicated key, not an
    /// `AppSettings` field: that blob has a positional wire layout, so a new
    /// field would break decoding for existing installs and older builds.
    pub const KV_KEY: &'static str = "det:upgrade_backup_retention:v1";

    /// Default retention period: roughly three months.
    pub const DEFAULT_DAYS: u16 = 90;

    /// Shortest accepted retention period, in days.
    pub const MIN_DAYS: u16 = 1;

    /// Longest accepted retention period, in days (about ten years).
    pub const MAX_DAYS: u16 = 3650;

    /// Maximum backup age, or `None` when backups are kept forever.
    pub fn max_age(self) -> Option<Duration> {
        match self {
            Self::KeepForever => None,
            Self::DeleteAfterDays(days) => Some(Duration::from_secs(
                u64::from(days).saturating_mul(SECONDS_PER_DAY),
            )),
        }
    }

    /// The policy itself when valid, otherwise the default.
    ///
    /// Guards against a persisted value outside the accepted range, which
    /// would otherwise delete backups sooner than any user could choose.
    pub fn sanitized(self) -> Self {
        match self {
            Self::DeleteAfterDays(days) if validate_retention_days(u32::from(days)).is_err() => {
                Self::default()
            }
            policy => policy,
        }
    }
}

impl Default for BackupRetention {
    fn default() -> Self {
        Self::DeleteAfterDays(Self::DEFAULT_DAYS)
    }
}

/// A retention period outside the accepted range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Enter a number of days between {min} and {max}.")]
pub struct InvalidRetentionDays {
    /// Shortest accepted period.
    pub min: u16,
    /// Longest accepted period.
    pub max: u16,
}

/// Validate a retention period in days.
pub fn validate_retention_days(days: u32) -> Result<u16, InvalidRetentionDays> {
    u16::try_from(days)
        .ok()
        .filter(|days| (BackupRetention::MIN_DAYS..=BackupRetention::MAX_DAYS).contains(days))
        .ok_or(InvalidRetentionDays {
            min: BackupRetention::MIN_DAYS,
            max: BackupRetention::MAX_DAYS,
        })
}

/// Whether a backup created at `created` is older than `max_age` at `now`.
///
/// `created` must be the latest of every creation signal available (file
/// modification time, a timestamp in the file name), so a reset or skewed
/// signal only postpones deletion. A creation time in the future is never
/// expired.
pub fn backup_expired(created: SystemTime, now: SystemTime, max_age: Duration) -> bool {
    now.duration_since(created).is_ok_and(|age| age > max_age)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(SECONDS_PER_DAY);

    #[test]
    fn default_retention_is_ninety_days() {
        assert_eq!(
            BackupRetention::default(),
            BackupRetention::DeleteAfterDays(90)
        );
        assert_eq!(BackupRetention::default().max_age(), Some(DAY * 90));
        assert_eq!(BackupRetention::KeepForever.max_age(), None);
    }

    #[test]
    fn retention_days_are_bounded() {
        assert_eq!(validate_retention_days(1), Ok(1));
        assert_eq!(validate_retention_days(3650), Ok(3650));
        for invalid in [0, 3651, u32::from(u16::MAX) + 1] {
            let error = validate_retention_days(invalid).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Enter a number of days between 1 and 3650."
            );
        }
    }

    #[test]
    fn out_of_range_persisted_policy_falls_back_to_default() {
        assert_eq!(
            BackupRetention::DeleteAfterDays(0).sanitized(),
            BackupRetention::default()
        );
        assert_eq!(
            BackupRetention::DeleteAfterDays(30).sanitized(),
            BackupRetention::DeleteAfterDays(30)
        );
        assert_eq!(
            BackupRetention::KeepForever.sanitized(),
            BackupRetention::KeepForever
        );
    }

    #[test]
    fn backup_expires_only_after_max_age() {
        let now = SystemTime::UNIX_EPOCH + DAY * 1000;
        assert!(backup_expired(now - DAY * 91, now, DAY * 90));
        assert!(!backup_expired(now - DAY * 90, now, DAY * 90));
        assert!(!backup_expired(now - DAY, now, DAY * 90));
        assert!(
            !backup_expired(now + DAY * 365, now, DAY * 90),
            "a future creation time is never expired"
        );
    }
}
