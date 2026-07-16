//! Stateless password-format validation for identity-key protection (Tier-2).
//!
//! The single source of truth for the identity-key protection password policy,
//! reused by the backend seal path (authoritative enforcement) and by any UI
//! that wants instant feedback (FR-8 / §10.3). Delegates to the shared
//! single-key passphrase length rule so the minimum lives in one place.

use crate::backend_task::error::TaskError;
use crate::model::secret::Secret;
use crate::model::wallet::passphrase::validate_single_key_passphrase;

/// Validate an identity-key protection password against the backend policy.
///
/// Reuses the single-key passphrase rule (the same minimum length the UI
/// shows). The confirmation match is a UI concern, so the password is passed as
/// its own confirmation here — only the length check is meaningful at this
/// layer.
///
/// # Errors
///
/// [`TaskError::SingleKeyPassphraseTooShort`] when the password is shorter than
/// the shared minimum length.
pub fn validate_protection_password(password: &Secret) -> Result<(), TaskError> {
    let pw = password.expose_secret();
    validate_single_key_passphrase(pw, pw).map_err(TaskError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A too-short password is rejected with the typed error; a compliant one
    /// passes — the same policy the backend seal path enforces.
    #[test]
    fn weak_password_is_rejected_compliant_accepted() {
        let err = validate_protection_password(&Secret::new("short")).expect_err("too short");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseTooShort { .. }),
            "expected SingleKeyPassphraseTooShort, got {err:?}"
        );
        validate_protection_password(&Secret::new("long-enough-password"))
            .expect("compliant password accepted");
    }
}
