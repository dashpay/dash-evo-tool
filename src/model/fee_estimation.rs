//! Fee estimation utilities for Dash Platform state transitions.
//!
//! This module provides fee estimation for various state transition types,
//! using the fee structure from the platform version.

use dash_sdk::dpp::version::PlatformVersion;

/// Minimum fees for state transitions (in credits).
/// Based on STATE_TRANSITION_MIN_FEES_VERSION1 from rs-platform-version.
#[derive(Debug, Clone, Copy)]
pub struct StateTransitionMinFees {
    pub credit_transfer: u64,
    pub credit_transfer_to_addresses: u64,
    pub credit_withdrawal: u64,
    pub identity_update: u64,
    pub document_batch_sub_transition: u64,
    pub contract_create: u64,
    pub contract_update: u64,
    pub masternode_vote: u64,
    pub address_credit_withdrawal: u64,
    pub address_funds_transfer_input_cost: u64,
    pub address_funds_transfer_output_cost: u64,
    pub identity_create_base_cost: u64,
    pub identity_topup_base_cost: u64,
    pub identity_key_in_creation_cost: u64,
}

impl Default for StateTransitionMinFees {
    fn default() -> Self {
        // Values from STATE_TRANSITION_MIN_FEES_VERSION1
        Self {
            credit_transfer: 100_000,
            credit_transfer_to_addresses: 500_000,
            credit_withdrawal: 400_000_000,
            identity_update: 100_000,
            document_batch_sub_transition: 100_000,
            contract_create: 100_000,
            contract_update: 100_000,
            masternode_vote: 100_000,
            address_credit_withdrawal: 400_000_000,
            address_funds_transfer_input_cost: 500_000,
            address_funds_transfer_output_cost: 6_000_000,
            identity_create_base_cost: 2_000_000,
            identity_topup_base_cost: 500_000,
            identity_key_in_creation_cost: 6_500_000,
        }
    }
}

/// Fee estimator for platform state transitions.
#[derive(Debug, Clone)]
pub struct PlatformFeeEstimator {
    min_fees: StateTransitionMinFees,
}

impl Default for PlatformFeeEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformFeeEstimator {
    pub fn new() -> Self {
        Self {
            min_fees: StateTransitionMinFees::default(),
        }
    }

    /// Try to create from platform version (for future dynamic fee support)
    pub fn from_platform_version(_platform_version: &PlatformVersion) -> Self {
        // For now, use default fees. In future, could read from platform_version
        Self::new()
    }

    /// Estimate fee for credit transfer between identities
    pub fn estimate_credit_transfer(&self) -> u64 {
        self.min_fees.credit_transfer
    }

    /// Estimate fee for credit transfer to platform addresses
    pub fn estimate_credit_transfer_to_addresses(&self, output_count: usize) -> u64 {
        self.min_fees
            .credit_transfer_to_addresses
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_output_cost
                    .saturating_mul(output_count as u64),
            )
    }

    /// Estimate fee for credit withdrawal to core chain
    pub fn estimate_credit_withdrawal(&self) -> u64 {
        self.min_fees.credit_withdrawal
    }

    /// Estimate fee for address-based credit withdrawal
    pub fn estimate_address_credit_withdrawal(&self) -> u64 {
        self.min_fees.address_credit_withdrawal
    }

    /// Estimate fee for identity update (adding/disabling keys)
    pub fn estimate_identity_update(&self) -> u64 {
        self.min_fees.identity_update
    }

    /// Estimate fee for identity creation
    pub fn estimate_identity_create(&self, key_count: usize) -> u64 {
        self.min_fees
            .identity_create_base_cost
            .saturating_add(
                self.min_fees
                    .identity_key_in_creation_cost
                    .saturating_mul(key_count as u64),
            )
    }

    /// Estimate fee for identity creation from addresses (asset lock)
    pub fn estimate_identity_create_from_addresses(
        &self,
        input_count: usize,
        has_output: bool,
        key_count: usize,
    ) -> u64 {
        let output_count = if has_output { 1 } else { 0 };
        self.min_fees
            .identity_create_base_cost
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_input_cost
                    .saturating_mul(input_count as u64),
            )
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_output_cost
                    .saturating_mul(output_count),
            )
            .saturating_add(
                self.min_fees
                    .identity_key_in_creation_cost
                    .saturating_mul(key_count as u64),
            )
    }

    /// Estimate fee for identity top-up
    pub fn estimate_identity_topup(&self) -> u64 {
        self.min_fees.identity_topup_base_cost
    }

    /// Estimate fee for document batch transition
    pub fn estimate_document_batch(&self, transition_count: usize) -> u64 {
        self.min_fees
            .document_batch_sub_transition
            .saturating_mul(transition_count.max(1) as u64)
    }

    /// Estimate fee for document creation
    pub fn estimate_document_create(&self) -> u64 {
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for document deletion
    pub fn estimate_document_delete(&self) -> u64 {
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for document replacement
    pub fn estimate_document_replace(&self) -> u64 {
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for document transfer
    pub fn estimate_document_transfer(&self) -> u64 {
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for document purchase
    pub fn estimate_document_purchase(&self) -> u64 {
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for document set price
    pub fn estimate_document_set_price(&self) -> u64 {
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for token transition (mint, burn, transfer, freeze, etc.)
    pub fn estimate_token_transition(&self) -> u64 {
        // Token transitions use the document batch sub-transition fee
        self.min_fees.document_batch_sub_transition
    }

    /// Estimate fee for data contract creation (base fee only, excludes registration cost)
    pub fn estimate_contract_create_base(&self) -> u64 {
        self.min_fees.contract_create
    }

    /// Estimate fee for data contract update
    pub fn estimate_contract_update(&self) -> u64 {
        self.min_fees.contract_update
    }

    /// Estimate fee for masternode vote
    pub fn estimate_masternode_vote(&self) -> u64 {
        self.min_fees.masternode_vote
    }

    /// Estimate fee for address funds transfer
    pub fn estimate_address_funds_transfer(&self, input_count: usize, output_count: usize) -> u64 {
        self.min_fees
            .address_funds_transfer_input_cost
            .saturating_mul(input_count as u64)
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_output_cost
                    .saturating_mul(output_count.max(1) as u64),
            )
    }

    /// Get the raw minimum fees structure
    pub fn min_fees(&self) -> &StateTransitionMinFees {
        &self.min_fees
    }
}

/// Format credits as DASH for display
pub fn format_credits_as_dash(credits: u64) -> String {
    // 1 DASH = 100_000_000 credits (same as duffs)
    let dash = credits as f64 / 100_000_000.0;
    format!("{:.8} DASH", dash)
}

/// Format credits for display (with both credits and DASH)
pub fn format_credits(credits: u64) -> String {
    let dash = credits as f64 / 100_000_000.0;
    if credits >= 1_000_000 {
        format!("{} credits ({:.8} DASH)", credits, dash)
    } else {
        format!("{} credits ({:.10} DASH)", credits, dash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credit_transfer_estimate() {
        let estimator = PlatformFeeEstimator::new();
        assert_eq!(estimator.estimate_credit_transfer(), 100_000);
    }

    #[test]
    fn test_identity_create_estimate() {
        let estimator = PlatformFeeEstimator::new();
        // Base cost + 2 keys
        let fee = estimator.estimate_identity_create(2);
        assert_eq!(fee, 2_000_000 + 2 * 6_500_000);
    }

    #[test]
    fn test_document_batch_estimate() {
        let estimator = PlatformFeeEstimator::new();
        // 3 documents
        let fee = estimator.estimate_document_batch(3);
        assert_eq!(fee, 3 * 100_000);
    }

    #[test]
    fn test_format_credits() {
        assert_eq!(format_credits_as_dash(100_000_000), "1.00000000 DASH");
        assert_eq!(format_credits_as_dash(100_000), "0.00100000 DASH");
    }
}
