//! Stateless validation for per-key (single-key import) passphrases.
//!
//! The single-source-of-truth rules live here so the import dialog, the
//! restore dialog, and the backend all agree on what makes a passphrase
//! acceptable (CLAUDE.md validation-placement rule). UI screens call this
//! for instant feedback; the backend re-checks it as the authoritative
//! enforcement layer.

use crate::backend_task::error::TaskError;

/// Minimum length (in characters) for a per-key passphrase. Mirrors
/// NIST 800-63B / OWASP ASVS 6.2.1's minimum recommendation. Both the
/// import/restore dialogs and the backend enforce this single value.
pub const MIN_SINGLE_KEY_PASSPHRASE_LEN: usize = 8;

/// Validate a new single-key passphrase and its confirmation.
///
/// Stateless and dependency-free so it is trivially unit-testable and can
/// run client-side for instant feedback. The backend re-runs the same
/// checks as the authoritative layer, so a callsite that skips this still
/// gets the typed error.
///
/// # Errors
///
/// - [`TaskError::SingleKeyPassphraseTooShort`] when `passphrase` has fewer
///   than [`MIN_SINGLE_KEY_PASSPHRASE_LEN`] characters.
/// - [`TaskError::SingleKeyPassphraseMismatch`] when `passphrase` and
///   `confirm` differ.
pub fn validate_single_key_passphrase(passphrase: &str, confirm: &str) -> Result<(), TaskError> {
    if passphrase.chars().count() < MIN_SINGLE_KEY_PASSPHRASE_LEN {
        return Err(TaskError::SingleKeyPassphraseTooShort {
            min: MIN_SINGLE_KEY_PASSPHRASE_LEN as u32,
        });
    }
    if passphrase != confirm {
        return Err(TaskError::SingleKeyPassphraseMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_passphrase_is_too_short() {
        let err = validate_single_key_passphrase("", "").expect_err("empty rejected");
        match err {
            TaskError::SingleKeyPassphraseTooShort { min } => {
                assert_eq!(min, MIN_SINGLE_KEY_PASSPHRASE_LEN as u32);
            }
            other => panic!("expected SingleKeyPassphraseTooShort, got {other:?}"),
        }
    }

    #[test]
    fn too_short_passphrase_is_rejected() {
        // One character under the limit, matching confirmation — the
        // length check must fire before the mismatch check.
        let short: String = "a".repeat(MIN_SINGLE_KEY_PASSPHRASE_LEN - 1);
        let err = validate_single_key_passphrase(&short, &short).expect_err("short rejected");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseTooShort { .. }),
            "expected SingleKeyPassphraseTooShort, got {err:?}"
        );
    }

    #[test]
    fn length_counts_characters_not_bytes() {
        // Seven multi-byte chars: well over the byte threshold but under
        // the character minimum — must still be rejected as too short.
        let short = "é".repeat(MIN_SINGLE_KEY_PASSPHRASE_LEN - 1);
        let err = validate_single_key_passphrase(&short, &short).expect_err("short rejected");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseTooShort { .. }),
            "expected SingleKeyPassphraseTooShort, got {err:?}"
        );
    }

    #[test]
    fn mismatched_passphrases_are_rejected() {
        let err = validate_single_key_passphrase("longenough1", "longenough2")
            .expect_err("mismatch rejected");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseMismatch),
            "expected SingleKeyPassphraseMismatch, got {err:?}"
        );
    }

    #[test]
    fn valid_matching_passphrase_passes() {
        assert!(validate_single_key_passphrase("longenough123", "longenough123").is_ok());
    }

    #[test]
    fn exactly_minimum_length_passes() {
        let exact: String = "a".repeat(MIN_SINGLE_KEY_PASSPHRASE_LEN);
        assert!(validate_single_key_passphrase(&exact, &exact).is_ok());
    }
}
