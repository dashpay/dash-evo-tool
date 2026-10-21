//! Stateless asset-lock amount rules shared by screens and backend tasks.

use std::ops::RangeInclusive;

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

/// A funding amount is smaller than the network fee taken from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetLockAmountBelowMinimum {
    pub minimum_amount_duffs: u64,
}

/// Refuse a funding amount the network would reject for not covering its fee.
///
/// The network takes the fee out of the funding itself and accepts an amount
/// equal to `minimum_duffs`.
pub fn validate_asset_lock_minimum(
    amount_duffs: u64,
    minimum_duffs: u64,
) -> Result<(), AssetLockAmountBelowMinimum> {
    if amount_duffs < minimum_duffs {
        return Err(AssetLockAmountBelowMinimum {
            minimum_amount_duffs: minimum_duffs,
        });
    }
    Ok(())
}

/// Amounts a user can enter: from the network minimum up to the builder
/// ceiling less the reserve. `None` when no amount satisfies both, so a "Max"
/// built on this range never offers one the network would refuse.
///
/// All arguments must use the same unit (duffs or Platform credits).
pub fn asset_lock_user_amount_range(
    builder_max: u64,
    reserve: u64,
    minimum: u64,
) -> Option<RangeInclusive<u64>> {
    let maximum = asset_lock_user_max_amount(builder_max, reserve);
    (minimum <= maximum).then_some(minimum..=maximum)
}

/// A confirmed funding transaction of `amount_duffs`, for the tests of the
/// lists that offer one.
#[cfg(test)]
pub(crate) fn confirmed_funding_for_test(
    txid_byte: u8,
    amount_duffs: u64,
) -> platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock {
    use dash_sdk::dpp::dashcore::{OutPoint, Transaction, Txid, hashes::Hash};
    use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
    use dash_sdk::dpp::prelude::AssetLockProof;
    use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
    let out_point = OutPoint::new(Txid::from_byte_array([txid_byte; 32]), 0);
    TrackedAssetLock {
        out_point,
        transaction: Transaction {
            version: 3,
            lock_time: 0,
            input: Vec::new(),
            output: Vec::new(),
            special_transaction_payload: None,
        },
        account_index: 0,
        funding_type: platform_wallet::AssetLockFundingType::IdentityTopUp,
        identity_index: 0,
        amount: amount_duffs,
        status: AssetLockStatus::ChainLocked,
        proof: Some(AssetLockProof::Chain(ChainAssetLockProof {
            core_chain_locked_height: 1,
            out_point,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssetLockAmountBelowMinimum, AssetLockAmountError, asset_lock_user_amount_range,
        asset_lock_user_max_amount, validate_asset_lock_amount, validate_asset_lock_minimum,
    };

    /// 0.000505 DASH — what the network requires an identity top-up to cover.
    const NETWORK_FEE_DUFFS: u64 = 50_500;

    #[test]
    fn funding_below_the_network_fee_is_refused_with_the_minimum() {
        let refused = Err(AssetLockAmountBelowMinimum {
            minimum_amount_duffs: NETWORK_FEE_DUFFS,
        });
        assert_eq!(
            validate_asset_lock_minimum(5_237, NETWORK_FEE_DUFFS),
            refused
        );
        assert_eq!(
            validate_asset_lock_minimum(NETWORK_FEE_DUFFS - 1, NETWORK_FEE_DUFFS),
            refused
        );
    }

    #[test]
    fn funding_equal_to_the_network_fee_is_accepted() {
        assert_eq!(
            validate_asset_lock_minimum(NETWORK_FEE_DUFFS, NETWORK_FEE_DUFFS),
            Ok(())
        );
        assert_eq!(
            validate_asset_lock_minimum(NETWORK_FEE_DUFFS + 1, NETWORK_FEE_DUFFS),
            Ok(())
        );
    }

    /// A wallet that can build at most 55 737 duffs leaves 5 237 after the
    /// reserve — an amount the network refuses, so nothing may be offered.
    #[test]
    fn no_amount_is_offered_when_the_ceiling_cannot_cover_the_network_fee() {
        assert_eq!(
            asset_lock_user_amount_range(55_737, NETWORK_FEE_DUFFS, NETWORK_FEE_DUFFS),
            None
        );
        assert_eq!(
            asset_lock_user_amount_range(100_999, NETWORK_FEE_DUFFS, NETWORK_FEE_DUFFS),
            None
        );
    }

    #[test]
    fn offered_amounts_span_the_network_fee_to_the_reserved_ceiling() {
        assert_eq!(
            asset_lock_user_amount_range(101_000, NETWORK_FEE_DUFFS, NETWORK_FEE_DUFFS),
            Some(NETWORK_FEE_DUFFS..=NETWORK_FEE_DUFFS)
        );
        assert_eq!(
            asset_lock_user_amount_range(1_000_000, NETWORK_FEE_DUFFS, NETWORK_FEE_DUFFS),
            Some(NETWORK_FEE_DUFFS..=949_500)
        );
    }

    /// Both ends of an offered range must pass the validators a dispatch runs.
    #[test]
    fn both_ends_of_an_offered_range_pass_validation() {
        let (ceiling, reserve) = (1_000_000, NETWORK_FEE_DUFFS);
        let range = asset_lock_user_amount_range(ceiling, reserve, NETWORK_FEE_DUFFS)
            .expect("the ceiling covers the network fee");
        for amount in [*range.start(), *range.end()] {
            assert_eq!(validate_asset_lock_amount(amount, reserve, ceiling), Ok(()));
            assert_eq!(
                validate_asset_lock_minimum(amount, NETWORK_FEE_DUFFS),
                Ok(())
            );
        }
    }

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
