//! Spending limits and expiry of identity authentication keys (protocol
//! version 14): pure validation for adding a key with limits and for raising
//! the limits of an existing key.
//!
//! The rules mirror Platform's so a request it would refuse — and still charge
//! for — is refused here for free:
//! - only AUTHENTICATION keys below MASTER may carry limits;
//! - a budget must be above zero, and an expiry in the future;
//! - an update only raises limits the key already has: a budget grows, an
//!   expiry moves later, and neither can be added to a key without it;
//! - after an update the key must be usable, so an expired key can be
//!   extended but not only topped up.

use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;
use dash_sdk::dpp::identity::{IdentityPublicKey, KeyID, Purpose, SecurityLevel, TimestampMillis};
use thiserror::Error;

/// Milliseconds in one day.
pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The longest validity, in days, a key can be given or extended by at once
/// (about 100 years). Guards against typos, not a Platform rule.
pub const MAX_KEY_VALIDITY_DAYS: u32 = 36_500;

/// Why key limits cannot be set or raised as requested.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyLimitsError {
    #[error(
        "Only authentication keys below the master level can have a spending limit or an expiry. Choose such a key, or remove the limits."
    )]
    NotAllowedForKey {
        purpose: Purpose,
        security_level: SecurityLevel,
    },
    #[error("A spending limit must be above zero. Enter a larger limit, or remove it.")]
    ZeroBudget,
    #[error("Enter the number of days as a whole number, for example 30.")]
    DaysNotANumber,
    #[error("The number of days must be at least 1. Enter a larger number.")]
    ZeroDays,
    #[error("The number of days can be at most {max}. Enter a smaller number.")]
    TooManyDays { max: u32 },
    #[error("This key is disabled, so its limits cannot be changed.")]
    KeyDisabled,
    #[error(
        "Enter an amount to add to the spending limit or a number of days to extend the expiry."
    )]
    NothingToRaise,
    #[error(
        "This key has no spending limit to raise. A spending limit can only be raised, not added to a key."
    )]
    NoBudgetToRaise,
    #[error(
        "This key has no expiry to extend. An expiry can only be extended, not added to a key."
    )]
    NoExpiryToExtend,
    #[error("The amount to add to the spending limit is too large. Enter a smaller amount.")]
    BudgetOverflow,
    #[error("The new expiry is too far in the future. Enter fewer days.")]
    ExpiryOverflow,
    #[error(
        "This key has expired, so raising its spending limit alone would not make it usable. Extend its expiry as well."
    )]
    StillExpired,
    #[error("The key's expiry must be in the future. Choose a later expiry.")]
    ExpiryNotInFuture,
    #[error(
        "This key's limits changed after you reviewed them. Review the new limits and confirm again."
    )]
    LimitsChanged,
}

/// Whether a key of this purpose and security level may carry limits.
pub fn limits_allowed(purpose: Purpose, security_level: SecurityLevel) -> bool {
    purpose == Purpose::AUTHENTICATION && security_level != SecurityLevel::MASTER
}

/// Whether a key of this purpose and security level may be bound to a
/// contract: encryption and decryption keys always (per the contract's own
/// rules), authentication keys below MASTER only where the network admits
/// bound authentication keys (`bound_authentication_supported`).
pub fn contract_bounds_allowed(
    purpose: Purpose,
    security_level: SecurityLevel,
    bound_authentication_supported: bool,
) -> bool {
    match purpose {
        Purpose::ENCRYPTION | Purpose::DECRYPTION => true,
        Purpose::AUTHENTICATION => {
            bound_authentication_supported && security_level != SecurityLevel::MASTER
        }
        _ => false,
    }
}

/// Check the limits a key to be added carries, as Platform will.
pub fn validate_key_limits(
    key: &IdentityPublicKey,
    now_ms: TimestampMillis,
) -> Result<(), KeyLimitsError> {
    if !key.has_limits() {
        return Ok(());
    }
    if !limits_allowed(key.purpose(), key.security_level()) {
        return Err(KeyLimitsError::NotAllowedForKey {
            purpose: key.purpose(),
            security_level: key.security_level(),
        });
    }
    if key.total_budget() == Some(0) {
        return Err(KeyLimitsError::ZeroBudget);
    }
    if key
        .expires_at()
        .is_some_and(|expires_at| expires_at <= now_ms)
    {
        return Err(KeyLimitsError::ExpiryNotInFuture);
    }
    Ok(())
}

/// The key to sign a key limits update with: an enabled MASTER
/// authentication key, else an enabled CRITICAL authentication key with no
/// limits and no contract bounds — among those `can_sign` accepts. Platform
/// refuses any other signer.
pub fn key_limits_update_signer<'a>(
    keys: impl IntoIterator<Item = &'a IdentityPublicKey>,
    can_sign: impl Fn(&IdentityPublicKey) -> bool,
) -> Option<&'a IdentityPublicKey> {
    let candidates: Vec<&IdentityPublicKey> = keys
        .into_iter()
        .filter(|key| {
            key.purpose() == Purpose::AUTHENTICATION && key.disabled_at().is_none() && can_sign(key)
        })
        .collect();
    let master = candidates
        .iter()
        .find(|key| key.security_level() == SecurityLevel::MASTER);
    let critical = candidates.iter().find(|key| {
        key.security_level() == SecurityLevel::CRITICAL
            && !key.has_limits()
            && key.contract_bounds().is_none()
    });
    master.or(critical).copied()
}

/// Parse a whole number of days between 1 and [`MAX_KEY_VALIDITY_DAYS`].
pub fn parse_key_validity_days(input: &str) -> Result<u32, KeyLimitsError> {
    let days: u32 = input
        .trim()
        .parse()
        .map_err(|_| KeyLimitsError::DaysNotANumber)?;
    match days {
        0 => Err(KeyLimitsError::ZeroDays),
        days if days > MAX_KEY_VALIDITY_DAYS => Err(KeyLimitsError::TooManyDays {
            max: MAX_KEY_VALIDITY_DAYS,
        }),
        days => Ok(days),
    }
}

fn days_after(from_ms: TimestampMillis, days: u32) -> Result<TimestampMillis, KeyLimitsError> {
    u64::from(days)
        .checked_mul(DAY_MS)
        .and_then(|span| from_ms.checked_add(span))
        .ok_or(KeyLimitsError::ExpiryOverflow)
}

/// The limits for a new key: `total_budget` credits, and an expiry
/// `validity_days` after `now_ms`. `(None, None)` means no limits.
pub fn new_key_limits(
    purpose: Purpose,
    security_level: SecurityLevel,
    total_budget: Option<Credits>,
    validity_days: Option<u32>,
    now_ms: TimestampMillis,
) -> Result<(Option<Credits>, Option<TimestampMillis>), KeyLimitsError> {
    if total_budget.is_none() && validity_days.is_none() {
        return Ok((None, None));
    }
    if !limits_allowed(purpose, security_level) {
        return Err(KeyLimitsError::NotAllowedForKey {
            purpose,
            security_level,
        });
    }
    if total_budget == Some(0) {
        return Err(KeyLimitsError::ZeroBudget);
    }
    let expires_at = validity_days
        .map(|days| match days {
            0 => Err(KeyLimitsError::ZeroDays),
            days if days > MAX_KEY_VALIDITY_DAYS => Err(KeyLimitsError::TooManyDays {
                max: MAX_KEY_VALIDITY_DAYS,
            }),
            days => days_after(now_ms, days),
        })
        .transpose()?;
    Ok((total_budget, expires_at))
}

/// The absolute limits to request for raising `key`'s: `add_budget` credits
/// more in total, and an expiry `extend_days` later — counted from the
/// current expiry, or from `now_ms` when the key has already expired.
pub fn raised_key_limits(
    key: &IdentityPublicKey,
    add_budget: Option<Credits>,
    extend_days: Option<u32>,
    now_ms: TimestampMillis,
) -> Result<(Option<Credits>, Option<TimestampMillis>), KeyLimitsError> {
    if key.disabled_at().is_some() {
        return Err(KeyLimitsError::KeyDisabled);
    }
    if add_budget.is_none() && extend_days.is_none() {
        return Err(KeyLimitsError::NothingToRaise);
    }

    let total_budget = add_budget
        .map(|amount| {
            let current = key.total_budget().ok_or(KeyLimitsError::NoBudgetToRaise)?;
            if amount == 0 {
                return Err(KeyLimitsError::ZeroBudget);
            }
            current
                .checked_add(amount)
                .ok_or(KeyLimitsError::BudgetOverflow)
        })
        .transpose()?;

    let expires_at = extend_days
        .map(|days| {
            let current = key.expires_at().ok_or(KeyLimitsError::NoExpiryToExtend)?;
            if days == 0 {
                return Err(KeyLimitsError::ZeroDays);
            }
            if days > MAX_KEY_VALIDITY_DAYS {
                return Err(KeyLimitsError::TooManyDays {
                    max: MAX_KEY_VALIDITY_DAYS,
                });
            }
            days_after(current.max(now_ms), days)
        })
        .transpose()?;

    // After the update the key must be usable: the expiry it will have is
    // the new one, or the current one when it is not extended.
    if let Some(expiry_after) = expires_at.or(key.expires_at())
        && expiry_after <= now_ms
    {
        return Err(KeyLimitsError::StillExpired);
    }

    Ok((total_budget, expires_at))
}

/// A limits raise the user reviewed: the key's limits as shown, the
/// absolute limits to set, and the key that signs the change. It is sent as
/// is, never recomputed: if the key's limits moved meanwhile, the raise is
/// refused and quoted again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyLimitsRaise {
    pub key_id: KeyID,
    pub seen_total_budget: Option<Credits>,
    pub seen_expires_at: Option<TimestampMillis>,
    /// The new total budget, `None` to leave it.
    pub total_budget: Option<Credits>,
    /// The new expiry, `None` to leave it.
    pub expires_at: Option<TimestampMillis>,
    pub signing_key_id: KeyID,
}

impl KeyLimitsRaise {
    /// Quote a raise of `key` by `add_budget` credits and/or `extend_days`
    /// days (see [`raised_key_limits`]), signed by `signing_key_id`.
    pub fn quote(
        key: &IdentityPublicKey,
        add_budget: Option<Credits>,
        extend_days: Option<u32>,
        signing_key_id: KeyID,
        now_ms: TimestampMillis,
    ) -> Result<Self, KeyLimitsError> {
        let (total_budget, expires_at) = raised_key_limits(key, add_budget, extend_days, now_ms)?;
        Ok(Self {
            key_id: key.id(),
            seen_total_budget: key.total_budget(),
            seen_expires_at: key.expires_at(),
            total_budget,
            expires_at,
            signing_key_id,
        })
    }

    /// Check the raise against `live`, the key as Platform holds it now: its
    /// limits must still be the ones reviewed, and the key must still be
    /// usable after the change.
    pub fn check_against(
        &self,
        live: &IdentityPublicKey,
        now_ms: TimestampMillis,
    ) -> Result<(), KeyLimitsError> {
        if live.disabled_at().is_some() {
            return Err(KeyLimitsError::KeyDisabled);
        }
        if live.total_budget() != self.seen_total_budget
            || live.expires_at() != self.seen_expires_at
        {
            return Err(KeyLimitsError::LimitsChanged);
        }
        if self.total_budget.is_none() && self.expires_at.is_none() {
            return Err(KeyLimitsError::NothingToRaise);
        }
        if let Some(expiry_after) = self.expires_at.or(self.seen_expires_at)
            && expiry_after <= now_ms
        {
            return Err(KeyLimitsError::StillExpired);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::identity::KeyType;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;

    const NOW: u64 = 1_758_000_000_000;

    fn key(total_budget: Option<Credits>, expires_at: Option<u64>) -> IdentityPublicKey {
        IdentityPublicKey::from(IdentityPublicKeyV0 {
            id: 3,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: vec![2; 33].into(),
            disabled_at: None,
        })
        .with_limits(total_budget, expires_at)
    }

    #[test]
    fn limits_are_only_for_authentication_keys_below_master() {
        assert!(limits_allowed(
            Purpose::AUTHENTICATION,
            SecurityLevel::CRITICAL
        ));
        assert!(limits_allowed(
            Purpose::AUTHENTICATION,
            SecurityLevel::MEDIUM
        ));
        assert!(!limits_allowed(
            Purpose::AUTHENTICATION,
            SecurityLevel::MASTER
        ));
        assert!(!limits_allowed(Purpose::TRANSFER, SecurityLevel::CRITICAL));
        assert!(!limits_allowed(Purpose::ENCRYPTION, SecurityLevel::MEDIUM));
    }

    #[test]
    fn authentication_keys_are_bound_only_where_the_network_admits_it() {
        assert!(contract_bounds_allowed(
            Purpose::ENCRYPTION,
            SecurityLevel::MEDIUM,
            false
        ));
        assert!(!contract_bounds_allowed(
            Purpose::AUTHENTICATION,
            SecurityLevel::HIGH,
            false
        ));
        assert!(contract_bounds_allowed(
            Purpose::AUTHENTICATION,
            SecurityLevel::HIGH,
            true
        ));
        assert!(!contract_bounds_allowed(
            Purpose::AUTHENTICATION,
            SecurityLevel::MASTER,
            true
        ));
        assert!(!contract_bounds_allowed(
            Purpose::TRANSFER,
            SecurityLevel::CRITICAL,
            true
        ));
    }

    #[test]
    fn a_new_key_without_limits_needs_no_checks() {
        assert_eq!(
            new_key_limits(Purpose::TRANSFER, SecurityLevel::CRITICAL, None, None, NOW),
            Ok((None, None))
        );
    }

    #[test]
    fn a_new_key_gets_its_budget_and_an_expiry_counted_from_now() {
        assert_eq!(
            new_key_limits(
                Purpose::AUTHENTICATION,
                SecurityLevel::HIGH,
                Some(500),
                Some(30),
                NOW
            ),
            Ok((Some(500), Some(NOW + 30 * DAY_MS)))
        );
    }

    #[test]
    fn a_new_key_refuses_limits_platform_would_refuse() {
        assert!(matches!(
            new_key_limits(
                Purpose::AUTHENTICATION,
                SecurityLevel::MASTER,
                Some(1),
                None,
                NOW
            ),
            Err(KeyLimitsError::NotAllowedForKey { .. })
        ));
        assert!(matches!(
            new_key_limits(
                Purpose::TRANSFER,
                SecurityLevel::CRITICAL,
                None,
                Some(1),
                NOW
            ),
            Err(KeyLimitsError::NotAllowedForKey { .. })
        ));
        assert_eq!(
            new_key_limits(
                Purpose::AUTHENTICATION,
                SecurityLevel::HIGH,
                Some(0),
                None,
                NOW
            ),
            Err(KeyLimitsError::ZeroBudget)
        );
        assert_eq!(
            new_key_limits(
                Purpose::AUTHENTICATION,
                SecurityLevel::HIGH,
                None,
                Some(0),
                NOW
            ),
            Err(KeyLimitsError::ZeroDays)
        );
    }

    #[test]
    fn validity_days_parse_within_bounds() {
        assert_eq!(parse_key_validity_days(" 30 "), Ok(30));
        assert_eq!(parse_key_validity_days("0"), Err(KeyLimitsError::ZeroDays));
        assert_eq!(
            parse_key_validity_days("thirty"),
            Err(KeyLimitsError::DaysNotANumber)
        );
        assert_eq!(
            parse_key_validity_days("-1"),
            Err(KeyLimitsError::DaysNotANumber)
        );
        assert_eq!(
            parse_key_validity_days(&(MAX_KEY_VALIDITY_DAYS + 1).to_string()),
            Err(KeyLimitsError::TooManyDays {
                max: MAX_KEY_VALIDITY_DAYS
            })
        );
    }

    #[test]
    fn a_top_up_adds_to_the_total_budget() {
        assert_eq!(
            raised_key_limits(&key(Some(1_000), None), Some(500), None, NOW),
            Ok((Some(1_500), None))
        );
    }

    #[test]
    fn an_extension_counts_from_the_current_expiry_while_the_key_is_valid() {
        let expires = NOW + DAY_MS;
        assert_eq!(
            raised_key_limits(&key(None, Some(expires)), None, Some(10), NOW),
            Ok((None, Some(expires + 10 * DAY_MS)))
        );
    }

    #[test]
    fn an_expired_key_is_extended_from_now_and_revived() {
        assert_eq!(
            raised_key_limits(&key(None, Some(NOW - DAY_MS)), None, Some(10), NOW),
            Ok((None, Some(NOW + 10 * DAY_MS)))
        );
    }

    #[test]
    fn an_expired_key_cannot_only_be_topped_up() {
        assert_eq!(
            raised_key_limits(&key(Some(1_000), Some(NOW)), Some(500), None, NOW),
            Err(KeyLimitsError::StillExpired)
        );
    }

    #[test]
    fn limits_are_raised_never_added() {
        let unlimited = key(None, None);
        assert_eq!(
            raised_key_limits(&unlimited, Some(1), None, NOW),
            Err(KeyLimitsError::NoBudgetToRaise)
        );
        assert_eq!(
            raised_key_limits(&unlimited, None, Some(1), NOW),
            Err(KeyLimitsError::NoExpiryToExtend)
        );
        assert_eq!(
            raised_key_limits(&key(Some(1), None), None, None, NOW),
            Err(KeyLimitsError::NothingToRaise)
        );
        assert_eq!(
            raised_key_limits(&key(Some(1), None), Some(0), None, NOW),
            Err(KeyLimitsError::ZeroBudget)
        );
        assert_eq!(
            raised_key_limits(&key(Some(u64::MAX), None), Some(1), None, NOW),
            Err(KeyLimitsError::BudgetOverflow)
        );
    }

    fn signer_key(id: u32, security_level: SecurityLevel) -> IdentityPublicKey {
        let mut key = key(None, None);
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        key.set_id(id);
        key.set_security_level(security_level);
        key
    }

    #[test]
    fn the_update_is_signed_by_master_first_then_an_unlimited_critical_key() {
        let master = signer_key(0, SecurityLevel::MASTER);
        let critical = signer_key(1, SecurityLevel::CRITICAL);
        let limited_critical = signer_key(2, SecurityLevel::CRITICAL).with_limits(Some(5), None);
        let keys = [limited_critical.clone(), critical.clone(), master.clone()];

        let picked = key_limits_update_signer(keys.iter(), |_| true);
        assert_eq!(picked.map(|k| k.id()), Some(0));

        let picked = key_limits_update_signer(keys.iter(), |k| k.id() != 0);
        assert_eq!(picked.map(|k| k.id()), Some(1), "an unlimited CRITICAL key");

        let picked = key_limits_update_signer([&limited_critical], |_| true);
        assert_eq!(picked, None, "a key with limits may not sign the update");
    }

    #[test]
    fn a_new_key_with_limits_is_checked_like_platform_does() {
        assert_eq!(validate_key_limits(&key(None, None), NOW), Ok(()));
        assert_eq!(
            validate_key_limits(&key(Some(1), Some(NOW + 1)), NOW),
            Ok(())
        );
        assert_eq!(
            validate_key_limits(&key(Some(0), None), NOW),
            Err(KeyLimitsError::ZeroBudget)
        );
        assert_eq!(
            validate_key_limits(&key(None, Some(NOW)), NOW),
            Err(KeyLimitsError::ExpiryNotInFuture)
        );
        assert!(matches!(
            validate_key_limits(
                &signer_key(0, SecurityLevel::MASTER).with_limits(Some(1), None),
                NOW
            ),
            Err(KeyLimitsError::NotAllowedForKey { .. })
        ));
    }

    #[test]
    fn a_reviewed_raise_is_sent_only_while_the_limits_are_unchanged() {
        let seen = key(Some(1_000), Some(NOW + DAY_MS));
        let raise = KeyLimitsRaise::quote(&seen, Some(500), Some(10), 0, NOW).expect("quotes");
        assert_eq!(raise.total_budget, Some(1_500));
        assert_eq!(raise.expires_at, Some(NOW + 11 * DAY_MS));
        assert_eq!(raise.check_against(&seen, NOW), Ok(()));

        let raised_elsewhere = key(Some(5_000), Some(NOW + DAY_MS));
        assert_eq!(
            raise.check_against(&raised_elsewhere, NOW),
            Err(KeyLimitsError::LimitsChanged)
        );
    }

    #[test]
    fn a_reviewed_top_up_is_refused_once_the_key_has_expired() {
        let seen = key(Some(1_000), Some(NOW + DAY_MS));
        let raise = KeyLimitsRaise::quote(&seen, Some(500), None, 0, NOW).expect("quotes");
        assert_eq!(
            raise.check_against(&seen, NOW + 2 * DAY_MS),
            Err(KeyLimitsError::StillExpired)
        );
    }

    #[test]
    fn a_disabled_key_is_left_alone() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        let mut disabled = key(Some(1_000), None);
        disabled.set_disabled_at(NOW);
        assert_eq!(
            raised_key_limits(&disabled, Some(1), None, NOW),
            Err(KeyLimitsError::KeyDisabled)
        );
    }
}
