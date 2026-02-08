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
