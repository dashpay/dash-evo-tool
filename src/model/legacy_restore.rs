//! Outcome of "Restore from Previous Version": what was copied back from the
//! earlier version's read-only database and what could not be.

/// Counters reported by one restore run. Pure data; the backend fills it and
/// the UI shows [`Self::user_message`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyRestoreSummary {
    /// Whether the earlier version's database holds wallets or identities for
    /// the active network.
    pub legacy_database_found: bool,
    /// Wallet secrets copied back into this version's secure storage.
    pub wallets_restored: u32,
    /// Wallets on this network that were already present and left untouched.
    pub wallets_already_present: u32,
    /// Wallet rows that are damaged and cannot be used (skipped).
    pub wallets_skipped_malformed: u32,
    /// Wallet rows that could not be read or written.
    pub wallets_failed: u32,
    /// Imported (single) keys copied back into this version's secure storage.
    pub imported_keys_restored: u32,
    /// Imported-key rows that could not be read or written.
    pub imported_keys_failed: u32,
    /// Identities that received at least one restored key or link.
    pub identities_updated: u32,
    /// Identity private keys restored.
    pub identity_keys_restored: u32,
    /// Identities whose saved copy could not be read or restored.
    pub identities_failed: u32,
    /// Identities skipped because the user declined their password prompt.
    pub identities_skipped: u32,
}

impl LegacyRestoreSummary {
    /// Whether anything was written by this run.
    pub fn restored_anything(&self) -> bool {
        self.wallets_restored > 0 || self.imported_keys_restored > 0 || self.identities_updated > 0
    }

    /// Whether some saved data could not be restored.
    pub fn has_problems(&self) -> bool {
        self.wallets_skipped_malformed > 0
            || self.wallets_failed > 0
            || self.imported_keys_failed > 0
            || self.identities_failed > 0
    }

    /// The user-facing summary, built from complete sentences only.
    pub fn user_message(&self) -> String {
        if !self.legacy_database_found {
            return "No data from an earlier version of Dash Evo Tool was found for this network. Nothing was restored.".to_string();
        }
        if !self.restored_anything() && !self.has_problems() && self.identities_skipped == 0 {
            return "Everything saved by the earlier version is already available. Nothing needed to be restored.".to_string();
        }

        let mut sentences = vec!["Restoring data from the earlier version finished.".to_string()];
        if !self.restored_anything() {
            sentences.push("Nothing new was restored.".to_string());
        }
        if self.wallets_restored > 0 {
            sentences.push(format!(
                "Wallets restored: {count}.",
                count = self.wallets_restored
            ));
        }
        if self.imported_keys_restored > 0 {
            sentences.push(format!(
                "Imported keys restored: {count}.",
                count = self.imported_keys_restored
            ));
        }
        if self.identities_updated > 0 {
            sentences.push(format!(
                "Identities updated: {count}.",
                count = self.identities_updated
            ));
        }
        if self.identity_keys_restored > 0 {
            sentences.push(format!(
                "Identity keys restored: {count}.",
                count = self.identity_keys_restored
            ));
        }
        if self.wallets_skipped_malformed > 0 {
            sentences.push(format!(
                "Damaged wallet records skipped: {count}.",
                count = self.wallets_skipped_malformed
            ));
        }
        if self.wallets_failed > 0 {
            sentences.push(format!(
                "Wallet records that could not be restored: {count}.",
                count = self.wallets_failed
            ));
        }
        if self.wallets_skipped_malformed > 0 || self.wallets_failed > 0 {
            sentences.push(
                "Import those wallets again from their recovery phrases to keep using them."
                    .to_string(),
            );
        }
        if self.imported_keys_failed > 0 {
            sentences.push(format!(
                "Imported key records that could not be restored: {count}.",
                count = self.imported_keys_failed
            ));
            sentences
                .push("Try again, or import those keys again from their private keys.".to_string());
        }
        if self.identities_failed > 0 {
            sentences.push(format!(
                "Identities that could not be restored: {count}.",
                count = self.identities_failed
            ));
            sentences.push("Try again, or import the keys of those identities again.".to_string());
        }
        if self.identities_skipped > 0 {
            sentences.push(format!(
                "Identities skipped because their password was not entered: {count}.",
                count = self.identities_skipped
            ));
            sentences.push("Run the restore again to include them.".to_string());
        }
        sentences.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found() -> LegacyRestoreSummary {
        LegacyRestoreSummary {
            legacy_database_found: true,
            ..Default::default()
        }
    }

    #[test]
    fn no_legacy_database_says_nothing_was_found() {
        let summary = LegacyRestoreSummary::default();
        assert!(!summary.restored_anything());
        assert!(!summary.has_problems());
        assert!(
            summary
                .user_message()
                .contains("No data from an earlier version")
        );
    }

    #[test]
    fn nothing_missing_says_nothing_needed_restoring() {
        let summary = LegacyRestoreSummary {
            wallets_already_present: 2,
            ..found()
        };
        assert!(
            summary
                .user_message()
                .contains("Nothing needed to be restored")
        );
    }

    #[test]
    fn restored_items_are_counted() {
        let summary = LegacyRestoreSummary {
            wallets_restored: 2,
            identities_updated: 1,
            identity_keys_restored: 3,
            ..found()
        };
        let message = summary.user_message();
        assert!(summary.restored_anything());
        assert!(!summary.has_problems());
        assert!(message.contains("Wallets restored: 2."), "{message}");
        assert!(message.contains("Identity keys restored: 3."), "{message}");
        assert!(!message.contains("recovery phrases"), "{message}");
    }

    /// Restoring only identity links reports the identities updated, never a
    /// misleading "0 wallets, 0 keys".
    #[test]
    fn a_link_only_restore_reports_identities_updated() {
        let summary = LegacyRestoreSummary {
            identities_updated: 1,
            ..found()
        };
        let message = summary.user_message();
        assert!(message.contains("Identities updated: 1."), "{message}");
        assert!(!message.contains("restored: 0"), "{message}");
        assert!(!message.contains("Nothing new was restored."), "{message}");
    }

    /// Declining a password prompt is the user's choice, not a failure: it is
    /// reported calmly, with how to include the identity later.
    #[test]
    fn declined_identities_are_reported_as_skipped_not_failed() {
        let summary = LegacyRestoreSummary {
            identities_skipped: 2,
            ..found()
        };
        let message = summary.user_message();
        assert!(!summary.has_problems(), "{message}");
        assert!(
            message.contains("Identities skipped because their password was not entered: 2."),
            "{message}"
        );
        assert!(message.contains("Run the restore again"), "{message}");
        assert!(!message.contains("could not be restored"), "{message}");
        assert!(
            !message.contains("Nothing needed to be restored"),
            "{message}"
        );
    }

    #[test]
    fn skipped_and_failed_rows_are_surfaced_with_an_action() {
        let summary = LegacyRestoreSummary {
            wallets_skipped_malformed: 1,
            wallets_failed: 2,
            identities_failed: 1,
            ..found()
        };
        let message = summary.user_message();
        assert!(summary.has_problems());
        assert!(message.contains("Nothing new was restored."), "{message}");
        assert!(
            message.contains("Damaged wallet records skipped: 1."),
            "{message}"
        );
        assert!(
            message.contains("Wallet records that could not be restored: 2."),
            "{message}"
        );
        assert!(message.contains("recovery phrases"), "{message}");
        assert!(
            message.contains("Identities that could not be restored: 1."),
            "{message}"
        );
    }

    #[test]
    fn imported_keys_are_counted_and_failures_surfaced() {
        let summary = LegacyRestoreSummary {
            imported_keys_restored: 2,
            imported_keys_failed: 1,
            ..found()
        };
        let message = summary.user_message();
        assert!(summary.restored_anything());
        assert!(summary.has_problems());
        assert!(message.contains("Imported keys restored: 2."), "{message}");
        assert!(
            message.contains("Imported key records that could not be restored: 1."),
            "{message}"
        );
        assert!(!message.contains("Nothing new was restored."), "{message}");
    }
}
