//! Fee estimation utilities for Dash Platform state transitions.
//!
//! This module provides fee estimation for various state transition types,
//! using the fee structure from the platform version.
//!
//! Fee calculation is based on:
//! - Storage fees: Bytes stored × storage_disk_usage_credit_per_byte (27,000)
//! - Processing fees: Bytes processed × storage_processing_credit_per_byte (400)
//! - Seek costs: Number of tree operations × storage_seek_cost (2,000)
//!
//! Note: These are estimates. Actual fees depend on exact storage operations
//! performed by Platform. For accurate fees, use Platform's EstimateStateTransitionFee
//! endpoint (when available).

use dash_sdk::dpp::version::PlatformVersion;

/// Storage fee constants from FEE_STORAGE_VERSION1 in rs-platform-version.
/// These determine the cost of storing and processing data on Platform.
#[derive(Debug, Clone, Copy)]
pub struct StorageFeeConstants {
    /// Credits charged per byte of permanent storage (27,000 credits/byte = 0.00027 DASH/byte)
    pub storage_disk_usage_credit_per_byte: u64,
    /// Credits charged per byte for write processing
    pub storage_processing_credit_per_byte: u64,
    /// Credits charged per byte for read processing
    pub storage_load_credit_per_byte: u64,
    /// Credits charged per seek/tree operation
    pub storage_seek_cost: u64,
}

impl Default for StorageFeeConstants {
    fn default() -> Self {
        // Values from FEE_STORAGE_VERSION1 in rs-platform-version
        Self {
            storage_disk_usage_credit_per_byte: 27_000,
            storage_processing_credit_per_byte: 400,
            storage_load_credit_per_byte: 20,
            storage_seek_cost: 2_000,
        }
    }
}

/// Data contract registration fees from FEE_DATA_CONTRACT_REGISTRATION_VERSION2.
/// These are fixed fees charged for registering contracts and their components.
#[derive(Debug, Clone, Copy)]
pub struct DataContractRegistrationFees {
    /// Base fee for registering any contract (0.1 DASH)
    pub base_contract_registration_fee: u64,
    /// Fee per document type in the contract (0.02 DASH)
    pub document_type_registration_fee: u64,
    /// Fee per non-unique index (0.01 DASH)
    pub document_type_base_non_unique_index_registration_fee: u64,
    /// Fee per unique index (0.01 DASH)
    pub document_type_base_unique_index_registration_fee: u64,
    /// Fee per contested index (1 DASH)
    pub document_type_base_contested_index_registration_fee: u64,
    /// Fee for token registration (0.1 DASH)
    pub token_registration_fee: u64,
    /// Fee for perpetual distribution feature (0.1 DASH)
    pub token_uses_perpetual_distribution_fee: u64,
    /// Fee for pre-programmed distribution feature (0.1 DASH)
    pub token_uses_pre_programmed_distribution_fee: u64,
    /// Fee per search keyword (0.1 DASH)
    pub search_keyword_fee: u64,
}

impl Default for DataContractRegistrationFees {
    fn default() -> Self {
        // Values from FEE_DATA_CONTRACT_REGISTRATION_VERSION2
        Self {
            base_contract_registration_fee: 10_000_000_000,      // 0.1 DASH
            document_type_registration_fee: 2_000_000_000,       // 0.02 DASH
            document_type_base_non_unique_index_registration_fee: 1_000_000_000, // 0.01 DASH
            document_type_base_unique_index_registration_fee: 1_000_000_000,     // 0.01 DASH
            document_type_base_contested_index_registration_fee: 100_000_000_000, // 1 DASH
            token_registration_fee: 10_000_000_000,              // 0.1 DASH
            token_uses_perpetual_distribution_fee: 10_000_000_000, // 0.1 DASH
            token_uses_pre_programmed_distribution_fee: 10_000_000_000, // 0.1 DASH
            search_keyword_fee: 10_000_000_000,                  // 0.1 DASH
        }
    }
}

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
    /// Asset lock cost for identity creation (200,000 duffs × 1000 credits/duff)
    pub identity_create_asset_lock_cost: u64,
    /// Asset lock cost for identity top-up (50,000 duffs × 1000 credits/duff)
    pub identity_topup_asset_lock_cost: u64,
    /// Asset lock cost for address funding (50,000 duffs × 1000 credits/duff)
    pub address_funding_asset_lock_cost: u64,
}

impl Default for StateTransitionMinFees {
    fn default() -> Self {
        // Values from STATE_TRANSITION_MIN_FEES_VERSION1
        // Asset lock costs from IdentityTransitionAssetLockVersions (duffs × CREDITS_PER_DUFF)
        // CREDITS_PER_DUFF = 1000
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
            // Asset lock costs (duffs × 1000)
            identity_create_asset_lock_cost: 200_000_000, // 200,000 duffs × 1000 = 0.002 DASH
            identity_topup_asset_lock_cost: 50_000_000,   // 50,000 duffs × 1000 = 0.0005 DASH
            address_funding_asset_lock_cost: 50_000_000,  // 50,000 duffs × 1000 = 0.0005 DASH
        }
    }
}

/// Fee estimator for platform state transitions.
#[derive(Debug, Clone)]
pub struct PlatformFeeEstimator {
    min_fees: StateTransitionMinFees,
    storage_fees: StorageFeeConstants,
    registration_fees: DataContractRegistrationFees,
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
            storage_fees: StorageFeeConstants::default(),
            registration_fees: DataContractRegistrationFees::default(),
        }
    }

    /// Try to create from platform version (for future dynamic fee support)
    pub fn from_platform_version(_platform_version: &PlatformVersion) -> Self {
        // For now, use default fees. In future, could read from platform_version
        Self::new()
    }

    /// Calculate storage fee for a given number of bytes.
    /// This is the main cost component for storing data on Platform.
    pub fn calculate_storage_fee(&self, bytes: usize) -> u64 {
        (bytes as u64).saturating_mul(self.storage_fees.storage_disk_usage_credit_per_byte)
    }

    /// Calculate processing fee for writing data.
    pub fn calculate_processing_fee(&self, bytes: usize) -> u64 {
        (bytes as u64).saturating_mul(self.storage_fees.storage_processing_credit_per_byte)
    }

    /// Calculate fee for tree seek operations.
    /// Contracts and documents require multiple seeks for tree traversal.
    pub fn calculate_seek_fee(&self, seek_count: usize) -> u64 {
        (seek_count as u64).saturating_mul(self.storage_fees.storage_seek_cost)
    }

    /// Estimate total storage-based fee for storing data.
    /// Includes storage, processing, and estimated seek costs.
    pub fn estimate_storage_based_fee(&self, bytes: usize, estimated_seeks: usize) -> u64 {
        self.calculate_storage_fee(bytes)
            .saturating_add(self.calculate_processing_fee(bytes))
            .saturating_add(self.calculate_seek_fee(estimated_seeks))
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

    /// Estimate fee for identity creation.
    /// This includes base cost, asset lock cost, and per-key costs.
    pub fn estimate_identity_create(&self, key_count: usize) -> u64 {
        self.min_fees
            .identity_create_base_cost
            .saturating_add(self.min_fees.identity_create_asset_lock_cost)
            .saturating_add(
                self.min_fees
                    .identity_key_in_creation_cost
                    .saturating_mul(key_count as u64),
            )
    }

    /// Estimate fee for identity creation from addresses (asset lock).
    /// This includes base cost, asset lock cost, input/output costs, and per-key costs.
    pub fn estimate_identity_create_from_addresses(
        &self,
        input_count: usize,
        has_output: bool,
        key_count: usize,
    ) -> u64 {
        let output_count = if has_output { 1 } else { 0 };
        self.min_fees
            .identity_create_base_cost
            .saturating_add(self.min_fees.address_funding_asset_lock_cost)
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

    /// Estimate fee for identity top-up.
    /// This includes base cost and asset lock cost.
    pub fn estimate_identity_topup(&self) -> u64 {
        self.min_fees
            .identity_topup_base_cost
            .saturating_add(self.min_fees.identity_topup_asset_lock_cost)
    }

    /// Estimate fee for document batch transition
    pub fn estimate_document_batch(&self, transition_count: usize) -> u64 {
        self.min_fees
            .document_batch_sub_transition
            .saturating_mul(transition_count.max(1) as u64)
    }

    /// Estimate fee for document creation with known size.
    /// Documents are stored in the contract's document tree.
    /// Estimated seeks: ~10 for tree traversal and insertion.
    pub fn estimate_document_create_with_size(&self, document_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.estimate_storage_based_fee(document_bytes, ESTIMATED_SEEKS))
    }

    /// Estimate fee for document creation (uses default estimate of ~200 bytes).
    pub fn estimate_document_create(&self) -> u64 {
        self.estimate_document_create_with_size(200)
    }

    /// Estimate fee for document deletion.
    /// Deletion is cheaper - mainly processing, no new storage.
    pub fn estimate_document_delete(&self) -> u64 {
        // Deletion involves seeks but no storage addition
        const ESTIMATED_SEEKS: usize = 8;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.calculate_seek_fee(ESTIMATED_SEEKS))
    }

    /// Estimate fee for document replacement with known size.
    pub fn estimate_document_replace_with_size(&self, document_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.estimate_storage_based_fee(document_bytes, ESTIMATED_SEEKS))
    }

    /// Estimate fee for document replacement (uses default estimate of ~200 bytes).
    pub fn estimate_document_replace(&self) -> u64 {
        self.estimate_document_replace_with_size(200)
    }

    /// Estimate fee for document transfer.
    /// Transfer updates ownership, minimal storage change.
    pub fn estimate_document_transfer(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const OWNERSHIP_UPDATE_BYTES: usize = 64;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.estimate_storage_based_fee(OWNERSHIP_UPDATE_BYTES, ESTIMATED_SEEKS))
    }

    /// Estimate fee for document purchase.
    pub fn estimate_document_purchase(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        const PURCHASE_UPDATE_BYTES: usize = 100;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.estimate_storage_based_fee(PURCHASE_UPDATE_BYTES, ESTIMATED_SEEKS))
    }

    /// Estimate fee for document set price.
    pub fn estimate_document_set_price(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const PRICE_UPDATE_BYTES: usize = 32;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.estimate_storage_based_fee(PRICE_UPDATE_BYTES, ESTIMATED_SEEKS))
    }

    /// Estimate fee for token transition (mint, burn, transfer, freeze, etc.).
    /// Token operations are relatively small - mainly balance updates.
    pub fn estimate_token_transition(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const TOKEN_OP_BYTES: usize = 100;
        self.min_fees
            .document_batch_sub_transition
            .saturating_add(self.estimate_storage_based_fee(TOKEN_OP_BYTES, ESTIMATED_SEEKS))
    }

    /// Estimate fee for data contract creation with known size.
    /// Includes base registration fee (0.1 DASH) plus storage costs.
    /// For contracts with tokens, document types, or indexes, use the detailed method.
    pub fn estimate_contract_create_with_size(&self, contract_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 20;
        self.registration_fees
            .base_contract_registration_fee
            .saturating_add(self.min_fees.contract_create)
            .saturating_add(self.estimate_storage_based_fee(contract_bytes, ESTIMATED_SEEKS))
    }

    /// Estimate fee for data contract creation with detailed component counts.
    /// This provides the most accurate estimate by accounting for all registration fees.
    pub fn estimate_contract_create_detailed(
        &self,
        contract_bytes: usize,
        document_type_count: usize,
        non_unique_index_count: usize,
        unique_index_count: usize,
        contested_index_count: usize,
        has_token: bool,
        has_perpetual_distribution: bool,
        has_pre_programmed_distribution: bool,
        search_keyword_count: usize,
    ) -> u64 {
        const ESTIMATED_SEEKS: usize = 20;

        let mut fee = self.registration_fees.base_contract_registration_fee;

        // Document type fees
        fee = fee.saturating_add(
            self.registration_fees
                .document_type_registration_fee
                .saturating_mul(document_type_count as u64),
        );

        // Index fees
        fee = fee.saturating_add(
            self.registration_fees
                .document_type_base_non_unique_index_registration_fee
                .saturating_mul(non_unique_index_count as u64),
        );
        fee = fee.saturating_add(
            self.registration_fees
                .document_type_base_unique_index_registration_fee
                .saturating_mul(unique_index_count as u64),
        );
        fee = fee.saturating_add(
            self.registration_fees
                .document_type_base_contested_index_registration_fee
                .saturating_mul(contested_index_count as u64),
        );

        // Token fees
        if has_token {
            fee = fee.saturating_add(self.registration_fees.token_registration_fee);
        }
        if has_perpetual_distribution {
            fee = fee.saturating_add(self.registration_fees.token_uses_perpetual_distribution_fee);
        }
        if has_pre_programmed_distribution {
            fee = fee.saturating_add(self.registration_fees.token_uses_pre_programmed_distribution_fee);
        }

        // Search keyword fees
        fee = fee.saturating_add(
            self.registration_fees
                .search_keyword_fee
                .saturating_mul(search_keyword_count as u64),
        );

        // Add state transition minimum and storage fees
        fee = fee.saturating_add(self.min_fees.contract_create);
        fee = fee.saturating_add(self.estimate_storage_based_fee(contract_bytes, ESTIMATED_SEEKS));

        fee
    }

    /// Estimate fee for data contract creation (uses base registration fee only).
    /// For more accurate estimates, use estimate_contract_create_with_size or
    /// estimate_contract_create_detailed.
    pub fn estimate_contract_create_base(&self) -> u64 {
        // Base registration fee (0.1 DASH) + minimal storage estimate
        self.estimate_contract_create_with_size(500)
    }

    /// Estimate fee for data contract update with known size of changes.
    pub fn estimate_contract_update_with_size(&self, update_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 15;
        self.min_fees
            .contract_update
            .saturating_add(self.estimate_storage_based_fee(update_bytes, ESTIMATED_SEEKS))
    }

    /// Estimate fee for data contract update (uses default estimate).
    pub fn estimate_contract_update(&self) -> u64 {
        self.estimate_contract_update_with_size(300)
    }

    /// Get the registration fees structure
    pub fn registration_fees(&self) -> &DataContractRegistrationFees {
        &self.registration_fees
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

    /// Get the storage fee constants
    pub fn storage_fees(&self) -> &StorageFeeConstants {
        &self.storage_fees
    }
}

/// Credits per DASH constant
/// 1 DASH = 100,000,000,000 credits (100 billion)
pub const CREDITS_PER_DASH: u64 = 100_000_000_000;

/// Format credits as DASH for display
pub fn format_credits_as_dash(credits: u64) -> String {
    let dash = credits as f64 / CREDITS_PER_DASH as f64;
    format!("{:.8} DASH", dash)
}

/// Format credits for display (with both credits and DASH)
pub fn format_credits(credits: u64) -> String {
    let dash = credits as f64 / CREDITS_PER_DASH as f64;
    if credits >= 1_000_000_000 {
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
        // 3 documents - base fee only
        let fee = estimator.estimate_document_batch(3);
        assert_eq!(fee, 3 * 100_000);
    }

    #[test]
    fn test_storage_fee_calculation() {
        let estimator = PlatformFeeEstimator::new();
        // 500 bytes at 27,000 credits/byte = 13,500,000 credits
        let fee = estimator.calculate_storage_fee(500);
        assert_eq!(fee, 500 * 27_000);
        // 13,500,000 credits = 0.000135 DASH (at 100 billion credits per DASH)
        assert_eq!(format_credits_as_dash(fee), "0.00013500 DASH");
    }

    #[test]
    fn test_contract_create_with_size() {
        let estimator = PlatformFeeEstimator::new();
        // 500 byte contract
        let fee = estimator.estimate_contract_create_with_size(500);
        // Should be: base_registration_fee + min_fee + storage + processing + seeks
        // 10,000,000,000 + 100,000 + (500 * 27,000) + (500 * 400) + (20 * 2,000)
        // = 10,000,000,000 + 100,000 + 13,500,000 + 200,000 + 40,000
        // = 10,013,840,000 credits = ~0.1 DASH
        let base_registration = 10_000_000_000u64; // 0.1 DASH
        let min_fee = 100_000u64;
        let storage = 500 * 27_000;
        let processing = 500 * 400;
        let seeks = 20 * 2_000;
        let expected = base_registration + min_fee + storage + processing + seeks;
        assert_eq!(fee, expected);
        // ~0.1 DASH for a simple contract (base registration fee dominates)
    }

    #[test]
    fn test_contract_create_detailed_with_token() {
        let estimator = PlatformFeeEstimator::new();
        // Contract with a token
        let fee = estimator.estimate_contract_create_detailed(
            500,   // contract bytes
            1,     // 1 document type
            1,     // 1 non-unique index
            0,     // 0 unique indexes
            0,     // 0 contested indexes
            true,  // has token
            false, // no perpetual distribution
            false, // no pre-programmed distribution
            0,     // 0 search keywords
        );
        // Base: 0.1 DASH + Document type: 0.02 DASH + Index: 0.01 DASH + Token: 0.1 DASH
        // = 0.23 DASH + storage fees
        let expected_registration = 10_000_000_000 + 2_000_000_000 + 1_000_000_000 + 10_000_000_000;
        assert!(fee >= expected_registration);
    }

    #[test]
    fn test_format_credits() {
        // 1 DASH = 100,000,000,000 credits
        assert_eq!(format_credits_as_dash(100_000_000_000), "1.00000000 DASH");
        assert_eq!(format_credits_as_dash(100_000_000), "0.00100000 DASH");
        assert_eq!(format_credits_as_dash(100_000), "0.00000100 DASH");
    }
}
