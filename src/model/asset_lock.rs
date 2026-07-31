//! Stateless asset-lock amount calculations shared by UI flows.

/// Why an asset-lock amount is outside the builder-derived ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetLockAmountError {
    /// Adding the operation-specific reserve exceeded the amount range.
    Overflow,
    /// The requested amount plus its reserve exceeds the builder ceiling.
    ExceedsMaximum { maximum_amount_duffs: u64 },
}

/// Largest user-entered amount after reserving operation-specific fees.
///
/// Both arguments must use the same unit (duffs or Platform credits).
pub fn asset_lock_user_max_amount(builder_max: u64, reserve: u64) -> u64 {
    builder_max.saturating_sub(reserve)
}

/// Validate a user-entered amount against the live builder-derived ceiling.
pub fn validate_asset_lock_amount(
    amount_duffs: u64,
    reserve_duffs: u64,
    builder_max_duffs: u64,
) -> Result<(), AssetLockAmountError> {
    let required_duffs = amount_duffs
        .checked_add(reserve_duffs)
        .ok_or(AssetLockAmountError::Overflow)?;
    if required_duffs > builder_max_duffs {
        return Err(AssetLockAmountError::ExceedsMaximum {
            maximum_amount_duffs: asset_lock_user_max_amount(builder_max_duffs, reserve_duffs),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AssetLockAmountError, asset_lock_user_max_amount, validate_asset_lock_amount};

    #[test]
    fn builder_ceiling_validation_reserves_operation_fee() {
        assert_eq!(asset_lock_user_max_amount(10_000, 1_000), 9_000);
        assert_eq!(validate_asset_lock_amount(9_000, 1_000, 10_000), Ok(()));
        assert_eq!(
            validate_asset_lock_amount(9_001, 1_000, 10_000),
            Err(AssetLockAmountError::ExceedsMaximum {
                maximum_amount_duffs: 9_000,
            })
        );
    }
}
