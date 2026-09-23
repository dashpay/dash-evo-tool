//! Stateless validation for per-key (single-key import) passphrases.
//!
//! The single-source-of-truth rules live here so the import dialog, the
//! restore dialog, and the backend all agree on what makes a passphrase
//! acceptable (CLAUDE.md validation-placement rule). UI screens call this
//! for instant feedback; the backend re-checks it as the authoritative
//! enforcement layer.

use platform_wallet_storage::secrets::MAX_PASSPHRASE_LEN;
use thiserror::Error;

/// Minimum length (in characters) for a per-key passphrase. Mirrors
/// NIST 800-63B / OWASP ASVS 6.2.1's minimum recommendation. Both the
/// import/restore dialogs and the backend enforce this single value.
pub const MIN_SINGLE_KEY_PASSPHRASE_LEN: usize = 8;

/// Why a single-key passphrase failed validation.
///
/// Model-local so this pure validator carries no dependency on the
/// backend-task layer; `TaskError` provides `From` conversions to the
/// user-facing variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PassphraseError {
    /// The passphrase is shorter than [`MIN_SINGLE_KEY_PASSPHRASE_LEN`].
    #[error("Passphrases must be at least {min} characters. Pick a longer one and try again.")]
    TooShort { min: u32 },
    /// The passphrase is longer than the vault's `MAX_PASSPHRASE_LEN`, so
    /// sealing under it fails — and, were it ever enrolled, a later build
    /// would refuse to unseal with it too.
    ///
    /// Counted in bytes while [`Self::TooShort`] counts characters: each
    /// mirrors the unit its enforcing layer uses.
    #[error("This key passphrase is too long. Pick a shorter passphrase and try again.")]
    TooLong { max: usize },
    /// The passphrase and its confirmation differ.
    #[error("The two passphrases do not match. Type them again carefully.")]
    Mismatch,
}

/// Validate a new single-key passphrase and its confirmation.
///
/// Stateless and dependency-free so it is trivially unit-testable and can
/// run client-side for instant feedback. The backend re-runs the same
/// checks as the authoritative layer, so a callsite that skips this still
/// gets the typed error.
///
/// # Errors
///
/// - [`PassphraseError::TooShort`] when `passphrase` has fewer than
///   [`MIN_SINGLE_KEY_PASSPHRASE_LEN`] characters.
/// - [`PassphraseError::TooLong`] when `passphrase` is more than
///   `MAX_PASSPHRASE_LEN` bytes.
/// - [`PassphraseError::Mismatch`] when `passphrase` and `confirm` differ.
pub fn validate_single_key_passphrase(
    passphrase: &str,
    confirm: &str,
) -> Result<(), PassphraseError> {
    // Untrimmed bytes, matching upstream `exceeds_maximum_passphrase_len`: the
    // ceiling bounds the guarded page a resident passphrase occupies,
    // whitespace included. Trimming here would pass values the vault refuses.
    if passphrase.len() > MAX_PASSPHRASE_LEN {
        return Err(PassphraseError::TooLong {
            max: MAX_PASSPHRASE_LEN,
        });
    }
    if passphrase.chars().count() < MIN_SINGLE_KEY_PASSPHRASE_LEN {
        return Err(PassphraseError::TooShort {
            min: MIN_SINGLE_KEY_PASSPHRASE_LEN as u32,
        });
    }
    if passphrase != confirm {
        return Err(PassphraseError::Mismatch);
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
            PassphraseError::TooShort { min } => {
                assert_eq!(min, MIN_SINGLE_KEY_PASSPHRASE_LEN as u32);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn too_short_passphrase_is_rejected() {
        // One character under the limit, matching confirmation — the
        // length check must fire before the mismatch check.
        let short: String = "a".repeat(MIN_SINGLE_KEY_PASSPHRASE_LEN - 1);
        let err = validate_single_key_passphrase(&short, &short).expect_err("short rejected");
        assert!(
            matches!(err, PassphraseError::TooShort { .. }),
            "expected TooShort, got {err:?}"
        );
    }

    #[test]
    fn length_counts_characters_not_bytes() {
        // Seven multi-byte chars: well over the byte threshold but under
        // the character minimum — must still be rejected as too short.
        let short = "é".repeat(MIN_SINGLE_KEY_PASSPHRASE_LEN - 1);
        let err = validate_single_key_passphrase(&short, &short).expect_err("short rejected");
        assert!(
            matches!(err, PassphraseError::TooShort { .. }),
            "expected TooShort, got {err:?}"
        );
    }

    #[test]
    fn mismatched_passphrases_are_rejected() {
        let err = validate_single_key_passphrase("longenough1", "longenough2")
            .expect_err("mismatch rejected");
        assert!(
            matches!(err, PassphraseError::Mismatch),
            "expected Mismatch, got {err:?}"
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

    #[test]
    fn exactly_maximum_length_passes() {
        let exact: String = "a".repeat(MAX_PASSPHRASE_LEN);
        assert!(
            validate_single_key_passphrase(&exact, &exact).is_ok(),
            "the vault seals at the ceiling, so this validator must not refuse it"
        );
    }

    #[test]
    fn one_byte_over_the_maximum_is_rejected() {
        let over: String = "a".repeat(MAX_PASSPHRASE_LEN + 1);
        let err = validate_single_key_passphrase(&over, &over).expect_err("over cap rejected");
        assert!(
            matches!(err, PassphraseError::TooLong { max } if max == MAX_PASSPHRASE_LEN),
            "expected TooLong, got {err:?}"
        );
    }

    /// The ceiling counts UTF-8 bytes, not characters — a character-counted
    /// check would let 4 080 four-byte characters (16 320 bytes) through and
    /// enrol a password the vault can never unseal.
    #[test]
    fn ceiling_counts_bytes_not_characters() {
        // "𝄞" is 4 bytes: exactly at the ceiling in bytes, far under in chars.
        let at_cap = "𝄞".repeat(MAX_PASSPHRASE_LEN / 4);
        assert_eq!(at_cap.len(), MAX_PASSPHRASE_LEN);
        assert!(validate_single_key_passphrase(&at_cap, &at_cap).is_ok());

        let over = "𝄞".repeat(MAX_PASSPHRASE_LEN / 4 + 1);
        assert!(over.chars().count() < MAX_PASSPHRASE_LEN, "under in chars");
        let err = validate_single_key_passphrase(&over, &over).expect_err("over cap in bytes");
        assert!(
            matches!(err, PassphraseError::TooLong { .. }),
            "expected TooLong, got {err:?}"
        );
    }

    /// The ceiling does NOT trim, unlike the floor: upstream bounds the whole
    /// resident value, whitespace included. A trimming check would accept this
    /// and hand the vault a passphrase it refuses.
    #[test]
    fn ceiling_does_not_trim_surrounding_whitespace() {
        let padded = format!("{}password", " ".repeat(MAX_PASSPHRASE_LEN));
        assert!(
            padded.trim().len() < MAX_PASSPHRASE_LEN,
            "trims to well under"
        );
        let err = validate_single_key_passphrase(&padded, &padded).expect_err("over cap untrimmed");
        assert!(
            matches!(err, PassphraseError::TooLong { .. }),
            "expected TooLong, got {err:?}"
        );
    }

    #[test]
    fn byte_ceiling_wins_when_trimmed_password_is_also_too_short() {
        let padded = format!("{}x", " ".repeat(MAX_PASSPHRASE_LEN));

        let err = validate_single_key_passphrase(&padded, &padded).expect_err("over cap");
        assert!(
            matches!(err, PassphraseError::TooLong { .. }),
            "got {err:?}"
        );
    }

    /// Length is checked before the confirmation match, so an over-long
    /// passphrase reports the actionable problem rather than a mismatch.
    #[test]
    fn ceiling_is_checked_before_the_confirmation_match() {
        let over: String = "a".repeat(MAX_PASSPHRASE_LEN + 1);
        let err = validate_single_key_passphrase(&over, "different").expect_err("over cap");
        assert!(
            matches!(err, PassphraseError::TooLong { .. }),
            "expected TooLong, got {err:?}"
        );
    }
}
