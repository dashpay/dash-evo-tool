//! Platform address allocation and fee estimation for address-based transfers.
//!
//! This module provides utilities for:
//! - Estimating fees for platform address funds transfers
//! - Estimating fees for withdrawal and funding transitions
//! - Allocating platform addresses for transfers (selecting which addresses
//!   to use and how much from each)

use crate::model::fee_estimation::{PlatformFeeEstimator, apply_fee_safety_margin};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::prelude::AddressNonce;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::StateTransitionEstimatedFeeValidation;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::AddressCreditWithdrawalTransition;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::v0::AddressCreditWithdrawalTransitionV0;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::AddressFundingFromAssetLockTransition;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::v0::AddressFundingFromAssetLockTransitionV0;
use dash_sdk::dpp::withdrawal::Pooling;
use std::collections::BTreeMap;

/// Maximum number of platform address inputs allowed per state transition
pub const MAX_PLATFORM_INPUTS: usize = 16;

/// Estimated serialized bytes per input (address + signature/witness data)
const ESTIMATED_BYTES_PER_INPUT: usize = 225;

/// Calculate the estimated fee for a platform address funds transfer.
///
/// Uses PlatformFeeEstimator for base costs (input/output fees) plus storage fees.
pub fn estimate_platform_fee(estimator: &PlatformFeeEstimator, input_count: usize) -> u64 {
    let inputs = input_count.max(1);

    // Base fee from Platform's min fee structure
    // - 500,000 credits per input (address_funds_transfer_input_cost)
    // - 6,000,000 credits per output (address_funds_transfer_output_cost)
    let base_fee = estimator.estimate_address_funds_transfer(inputs, 1);

    // Add storage fees for serialized input bytes only
    // (outputs don't add significant serialization overhead)
    let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT;
    let storage_fee = estimator.estimate_storage_based_fee(estimated_bytes, inputs);

    // Total with 20% safety buffer
    let total = base_fee.saturating_add(storage_fee);
    apply_fee_safety_margin(total, 20)
}

/// Calculate the estimated fee for a Platform address withdrawal using a constructed state transition.
pub fn estimate_withdrawal_fee_from_transition(
    platform_version: &dash_sdk::dpp::version::PlatformVersion,
    inputs: &BTreeMap<PlatformAddress, u64>,
    output_script: &CoreScript,
) -> u64 {
    let inputs_with_nonce: BTreeMap<PlatformAddress, (AddressNonce, Credits)> = inputs
        .iter()
        .map(|(addr, amount)| (*addr, (0, *amount)))
        .collect();

    let transition = AddressCreditWithdrawalTransition::V0(AddressCreditWithdrawalTransitionV0 {
        inputs: inputs_with_nonce,
        output: None,
        fee_strategy: vec![AddressFundsFeeStrategyStep::DeductFromInput(0)],
        core_fee_per_byte: 1,
        pooling: Pooling::Never,
        output_script: output_script.clone(),
        user_fee_increase: 0,
        input_witnesses: Vec::new(),
    });

    transition
        .calculate_min_required_fee(platform_version)
        .unwrap_or(0)
}

/// Calculate the estimated fee for funding a Platform address from an asset lock.
pub fn estimate_address_funding_fee_from_transition(
    platform_version: &dash_sdk::dpp::version::PlatformVersion,
    destination: &PlatformAddress,
) -> u64 {
    let mut outputs = BTreeMap::new();
    outputs.insert(*destination, None);

    let transition =
        AddressFundingFromAssetLockTransition::V0(AddressFundingFromAssetLockTransitionV0 {
            asset_lock_proof: AssetLockProof::default(),
            inputs: BTreeMap::new(),
            outputs,
            fee_strategy: vec![AddressFundsFeeStrategyStep::ReduceOutput(0)],
            user_fee_increase: 0,
            ..Default::default()
        });

    transition
        .calculate_min_required_fee(platform_version)
        .unwrap_or(0)
}

/// Result of allocating platform addresses for a transfer.
#[derive(Debug, Clone)]
pub struct AddressAllocationResult {
    /// Map of platform address to amount to transfer from each
    pub inputs: BTreeMap<PlatformAddress, u64>,
    /// Index of the fee payer in BTreeMap iteration order
    pub fee_payer_index: u16,
    /// Estimated fee for this transaction
    pub estimated_fee: u64,
    /// Amount that couldn't be covered (0 if fully covered)
    pub shortfall: u64,
    /// Addresses sorted by balance descending (for UI display)
    pub sorted_addresses: Vec<(PlatformAddress, Address, u64)>,
}

/// Allocates platform addresses for a transfer, using a custom fee calculator.
pub fn allocate_platform_addresses_with_fee<F>(
    addresses: &[(PlatformAddress, Address, u64)],
    amount_credits: u64,
    destination: Option<&PlatformAddress>,
    fee_for_inputs: F,
) -> AddressAllocationResult
where
    F: Fn(&BTreeMap<PlatformAddress, u64>) -> u64,
{
    // Filter out the destination address if provided (protocol doesn't allow same address as input and output)
    let filtered: Vec<_> = addresses
        .iter()
        .filter(|(platform_addr, _, _)| destination != Some(platform_addr))
        .cloned()
        .collect();

    // Sort addresses by balance descending so the largest balance is used first
    let mut sorted_addresses = filtered;
    sorted_addresses.sort_by(|a, b| b.2.cmp(&a.2));

    // Early return if no addresses available after filtering
    if sorted_addresses.is_empty() {
        return AddressAllocationResult {
            inputs: BTreeMap::new(),
            fee_payer_index: 0,
            estimated_fee: fee_for_inputs(&BTreeMap::new()),
            shortfall: amount_credits,
            sorted_addresses: vec![],
        };
    }

    // The highest-balance address (first in sorted order) will pay the fee
    let fee_payer_addr = sorted_addresses.first().map(|(addr, _, _)| *addr);

    let mut estimated_fee = fee_for_inputs(&BTreeMap::new());
    let mut inputs: BTreeMap<PlatformAddress, u64> = BTreeMap::new();

    // Iterate until fee estimate stabilizes (input count affects fee)
    for _ in 0..=MAX_PLATFORM_INPUTS {
        inputs.clear();
        let mut remaining = amount_credits;

        for (idx, (platform_addr, _, balance)) in sorted_addresses.iter().enumerate() {
            if remaining == 0 || inputs.len() >= MAX_PLATFORM_INPUTS {
                break;
            }
            let is_fee_payer = idx == 0;
            let available = if is_fee_payer {
                balance.saturating_sub(estimated_fee)
            } else {
                *balance
            };
            let use_amount = remaining.min(available);
            if use_amount > 0 || is_fee_payer {
                inputs.insert(*platform_addr, use_amount);
                remaining = remaining.saturating_sub(use_amount);
            }
        }

        let new_fee = fee_for_inputs(&inputs);
        if new_fee == estimated_fee {
            break;
        }
        estimated_fee = new_fee;
    }

    // Calculate shortfall (amount we couldn't allocate)
    let total_allocated: u64 = inputs.values().sum();
    let allocation_shortfall = amount_credits.saturating_sub(total_allocated);

    // Check if fee payer can actually afford the fee from their remaining balance.
    let fee_deficit = if let Some(fee_payer) = fee_payer_addr {
        let fee_payer_balance = sorted_addresses.first().map(|(_, _, b)| *b).unwrap_or(0);
        let fee_payer_contribution = inputs.get(&fee_payer).copied().unwrap_or(0);
        let fee_payer_remaining = fee_payer_balance.saturating_sub(fee_payer_contribution);
        estimated_fee.saturating_sub(fee_payer_remaining)
    } else {
        estimated_fee
    };

    let shortfall = allocation_shortfall.saturating_add(fee_deficit);

    // Find the index of the fee payer in BTreeMap order (required by backend)
    let fee_payer_index = fee_payer_addr
        .and_then(|payer| {
            inputs
                .keys()
                .enumerate()
                .find(|(_, addr)| **addr == payer)
                .map(|(idx, _)| idx as u16)
        })
        .unwrap_or(0);

    AddressAllocationResult {
        inputs,
        fee_payer_index,
        estimated_fee,
        shortfall,
        sorted_addresses,
    }
}

/// Allocates platform addresses for a transfer, selecting which addresses to use
/// and how much from each.
///
/// Algorithm:
/// 1. Filters out the destination address (can't be both input and output)
/// 2. Sorts addresses by balance descending (largest first)
/// 3. The highest-balance address pays the fee
/// 4. Iteratively allocates until fee estimate converges
/// 5. Fee payer is always included in inputs (even with 0 contribution) so fee can be deducted
///
/// Returns the allocation result with inputs, fee payer index, and any shortfall.
pub fn allocate_platform_addresses(
    estimator: &PlatformFeeEstimator,
    addresses: &[(PlatformAddress, Address, u64)],
    amount_credits: u64,
    destination: Option<&PlatformAddress>,
) -> AddressAllocationResult {
    let max_inputs = addresses
        .iter()
        .filter(|(platform_addr, _, _)| destination != Some(platform_addr))
        .count()
        .min(MAX_PLATFORM_INPUTS);

    allocate_platform_addresses_with_fee(addresses, amount_credits, destination, |_| {
        // Keep the legacy behavior: use a worst-case fee based on max possible inputs.
        estimate_platform_fee(estimator, max_inputs.max(1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dashcore_rpc::dashcore::Network;

    /// Helper: create a (PlatformAddress, Address, balance) tuple for testing.
    fn test_addr(id: u8, balance: u64) -> (PlatformAddress, Address, u64) {
        let mut hash = [0u8; 20];
        hash[0] = id;
        let platform_addr = PlatformAddress::P2pkh(hash);
        let address = platform_addr.to_address_with_network(Network::Testnet);
        (platform_addr, address, balance)
    }

    /// Constant fee function for deterministic testing.
    fn constant_fee(_inputs: &BTreeMap<PlatformAddress, u64>) -> u64 {
        1_000_000
    }

    /// Fee function that scales with the number of inputs.
    fn per_input_fee(inputs: &BTreeMap<PlatformAddress, u64>) -> u64 {
        let count = inputs.len().max(1) as u64;
        count * 500_000
    }

    // ==========================================
    // allocate_platform_addresses_with_fee tests
    // ==========================================

    #[test]
    fn test_single_recipient_sufficient_balance() {
        let addresses = vec![test_addr(1, 10_000_000)];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 5_000_000, None, constant_fee);
        assert_eq!(result.shortfall, 0);
        assert_eq!(result.inputs.len(), 1);
        assert_eq!(result.estimated_fee, 1_000_000);
        let allocated: u64 = result.inputs.values().sum();
        assert_eq!(allocated, 5_000_000);
    }

    #[test]
    fn test_multiple_addresses_picks_largest_first() {
        let addresses = vec![
            test_addr(1, 2_000_000),
            test_addr(2, 8_000_000),
            test_addr(3, 5_000_000),
        ];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 6_000_000, None, constant_fee);
        // Sorted by balance desc: addr2(8M), addr3(5M), addr1(2M)
        // Fee payer is addr2 (highest balance), available = 8M - 1M fee = 7M
        // 7M >= 6M needed, so only addr2 is used
        assert_eq!(result.shortfall, 0);
        assert_eq!(result.inputs.len(), 1);
        assert_eq!(result.estimated_fee, 1_000_000);
    }

    #[test]
    fn test_multiple_addresses_needed() {
        let addresses = vec![
            test_addr(1, 3_000_000),
            test_addr(2, 4_000_000),
            test_addr(3, 2_000_000),
        ];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 7_000_000, None, constant_fee);
        // Sorted: addr2(4M), addr1(3M), addr3(2M)
        // Fee payer addr2: available = 4M - 1M = 3M, allocate 3M, remaining = 4M
        // addr1: available = 3M, allocate 3M, remaining = 1M
        // addr3: available = 2M, allocate 1M, remaining = 0
        assert_eq!(result.shortfall, 0);
        assert!(result.inputs.len() >= 2);
        let allocated: u64 = result.inputs.values().sum();
        assert_eq!(allocated, 7_000_000);
    }

    #[test]
    fn test_insufficient_balance_reports_shortfall() {
        let addresses = vec![test_addr(1, 1_000_000)];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 5_000_000, None, constant_fee);
        // Balance = 1M, fee = 1M, available = 0
        // Fee payer always included, so inputs has addr with 0
        // Shortfall = 5M (amount) - 0 (allocated) = 5M allocation shortfall
        // Fee deficit: balance(1M) - contribution(0) = 1M remaining, fee(1M) - 1M = 0 deficit
        assert!(result.shortfall > 0);
        assert_eq!(result.shortfall, 5_000_000);
    }

    #[test]
    fn test_zero_amount_allocation() {
        let addresses = vec![test_addr(1, 10_000_000)];
        let result = allocate_platform_addresses_with_fee(&addresses, 0, None, constant_fee);
        // When amount is 0, remaining starts at 0 so the inner loop breaks immediately.
        // No inputs are added, but shortfall is also 0 since nothing was requested.
        assert_eq!(result.shortfall, 0);
        assert!(result.inputs.is_empty());
    }

    #[test]
    fn test_destination_address_filtered_out() {
        let addr1 = test_addr(1, 10_000_000);
        let addr2 = test_addr(2, 5_000_000);
        let addresses = vec![addr1.clone(), addr2.clone()];

        // Transfer to addr1's platform address — addr1 should be excluded
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            3_000_000,
            Some(&addr1.0),
            constant_fee,
        );

        // addr1 filtered out, only addr2 available
        assert!(!result.inputs.contains_key(&addr1.0));
        assert_eq!(result.shortfall, 0);
    }

    #[test]
    fn test_destination_filter_causes_shortfall() {
        let addr1 = test_addr(1, 10_000_000);
        let addresses = vec![addr1.clone()];

        // Transfer to addr1 — the only address is filtered out
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            5_000_000,
            Some(&addr1.0),
            constant_fee,
        );

        assert_eq!(result.inputs.len(), 0);
        assert_eq!(result.shortfall, 5_000_000);
        assert!(result.sorted_addresses.is_empty());
    }

    #[test]
    fn test_empty_addresses() {
        let result = allocate_platform_addresses_with_fee(&[], 5_000_000, None, constant_fee);
        assert_eq!(result.inputs.len(), 0);
        assert_eq!(result.shortfall, 5_000_000);
        assert!(result.sorted_addresses.is_empty());
    }

    #[test]
    fn test_zero_balance_addresses() {
        let addresses = vec![test_addr(1, 0), test_addr(2, 0)];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 1_000_000, None, constant_fee);
        // No available balance, shortfall should be the full amount
        assert!(result.shortfall > 0);
    }

    #[test]
    fn test_fee_payer_index_in_btreemap_order() {
        // Create addresses where BTreeMap ordering differs from balance sorting
        let addresses = vec![
            test_addr(1, 2_000_000),
            test_addr(2, 8_000_000),
            test_addr(3, 5_000_000),
        ];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 10_000_000, None, constant_fee);
        // Fee payer is addr2 (highest balance). Its position in the BTreeMap
        // depends on PlatformAddress's Ord implementation. fee_payer_index should
        // correctly reflect the BTreeMap iteration order.
        let fee_payer_platform_addr = test_addr(2, 0).0;
        let expected_index = result
            .inputs
            .keys()
            .enumerate()
            .find(|(_, addr)| **addr == fee_payer_platform_addr)
            .map(|(idx, _)| idx as u16);
        assert_eq!(Some(result.fee_payer_index), expected_index);
    }

    #[test]
    fn test_per_input_fee_converges() {
        // With per-input fees, the algorithm needs multiple iterations to converge
        let addresses = vec![
            test_addr(1, 5_000_000),
            test_addr(2, 3_000_000),
            test_addr(3, 2_000_000),
        ];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 8_000_000, None, per_input_fee);
        // The fee should reflect the actual number of inputs used
        let actual_fee = per_input_fee(&result.inputs);
        assert_eq!(result.estimated_fee, actual_fee);
    }

    #[test]
    fn test_max_platform_inputs_respected() {
        // Create more than MAX_PLATFORM_INPUTS addresses
        let addresses: Vec<_> = (0..20u8).map(|i| test_addr(i, 1_000_000)).collect();
        let result =
            allocate_platform_addresses_with_fee(&addresses, 18_000_000, None, constant_fee);
        assert!(result.inputs.len() <= MAX_PLATFORM_INPUTS);
    }

    #[test]
    fn test_fee_payer_included_when_nonzero_amount() {
        // Fee payer is included as long as a nonzero amount is requested,
        // even if fee payer's contribution is 0 (fee consumes entire balance).
        let addresses = vec![
            test_addr(1, 1_000_000), // balance equals fee, available = 0
            test_addr(2, 5_000_000),
        ];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 3_000_000, None, constant_fee);
        // addr2 is fee payer (higher balance): available = 5M - 1M = 4M, covers 3M
        assert_eq!(result.shortfall, 0);
        assert!(!result.inputs.is_empty());
    }

    #[test]
    fn test_fee_exceeds_balance_reports_deficit() {
        // Fee is larger than the fee payer's total balance
        let addresses = vec![test_addr(1, 500_000)]; // Balance < fee (1M)
        let result = allocate_platform_addresses_with_fee(&addresses, 0, None, constant_fee);
        // Fee payer balance = 500K, fee = 1M
        // Available for allocation = 0 (saturating_sub), contribution = 0
        // Fee deficit = 1M - (500K - 0) = 500K
        assert!(result.shortfall > 0);
    }

    #[test]
    fn test_very_small_amount() {
        let addresses = vec![test_addr(1, 10_000_000)];
        let result = allocate_platform_addresses_with_fee(&addresses, 1, None, constant_fee);
        assert_eq!(result.shortfall, 0);
        let allocated: u64 = result.inputs.values().sum();
        assert_eq!(allocated, 1);
    }

    #[test]
    fn test_amount_close_to_balance_minus_fee() {
        // Amount exactly equals available balance after fee deduction
        let addresses = vec![test_addr(1, 10_000_000)];
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            9_000_000, // balance(10M) - fee(1M) = 9M
            None,
            constant_fee,
        );
        assert_eq!(result.shortfall, 0);
        let allocated: u64 = result.inputs.values().sum();
        assert_eq!(allocated, 9_000_000);
    }

    #[test]
    fn test_amount_exceeds_balance_minus_fee() {
        // Amount is more than what's available after fee
        let addresses = vec![test_addr(1, 10_000_000)];
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            9_500_000, // balance(10M) - fee(1M) = 9M available, need 9.5M
            None,
            constant_fee,
        );
        // Can only allocate 9M, shortfall = 500K
        assert_eq!(result.shortfall, 500_000);
    }

    #[test]
    fn test_sorted_addresses_preserved_in_result() {
        let addresses = vec![
            test_addr(1, 2_000_000),
            test_addr(2, 8_000_000),
            test_addr(3, 5_000_000),
        ];
        let result =
            allocate_platform_addresses_with_fee(&addresses, 1_000_000, None, constant_fee);
        // sorted_addresses should be in descending balance order
        let balances: Vec<u64> = result.sorted_addresses.iter().map(|(_, _, b)| *b).collect();
        for w in balances.windows(2) {
            assert!(
                w[0] >= w[1],
                "sorted_addresses should be descending by balance"
            );
        }
    }

    // ====================================
    // allocate_platform_addresses tests
    // ====================================

    #[test]
    fn test_allocate_with_estimator_single_address() {
        let estimator = PlatformFeeEstimator::default();
        let addresses = vec![test_addr(1, 100_000_000)];
        let result = allocate_platform_addresses(&estimator, &addresses, 50_000_000, None);
        assert_eq!(result.shortfall, 0);
        assert_eq!(result.inputs.len(), 1);
        assert!(result.estimated_fee > 0);
    }

    #[test]
    fn test_allocate_with_estimator_multiple_addresses() {
        let estimator = PlatformFeeEstimator::default();
        let addresses = vec![
            test_addr(1, 30_000_000),
            test_addr(2, 40_000_000),
            test_addr(3, 20_000_000),
        ];
        let result = allocate_platform_addresses(&estimator, &addresses, 50_000_000, None);
        assert_eq!(result.shortfall, 0);
        assert!(result.estimated_fee > 0);
    }

    #[test]
    fn test_allocate_with_estimator_destination_filtered() {
        let estimator = PlatformFeeEstimator::default();
        let addr1 = test_addr(1, 100_000_000);
        let addr2 = test_addr(2, 50_000_000);
        let addresses = vec![addr1.clone(), addr2.clone()];
        let result =
            allocate_platform_addresses(&estimator, &addresses, 30_000_000, Some(&addr1.0));
        // addr1 filtered out, only addr2 used
        assert!(!result.inputs.contains_key(&addr1.0));
        assert_eq!(result.shortfall, 0);
    }

    #[test]
    fn test_allocate_with_estimator_empty() {
        let estimator = PlatformFeeEstimator::default();
        let result = allocate_platform_addresses(&estimator, &[], 5_000_000, None);
        assert_eq!(result.inputs.len(), 0);
        assert_eq!(result.shortfall, 5_000_000);
    }
}
