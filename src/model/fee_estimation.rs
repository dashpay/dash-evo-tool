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
            base_contract_registration_fee: 10_000_000_000, // 0.1 DASH
            document_type_registration_fee: 2_000_000_000,  // 0.02 DASH
            document_type_base_non_unique_index_registration_fee: 1_000_000_000, // 0.01 DASH
            document_type_base_unique_index_registration_fee: 1_000_000_000, // 0.01 DASH
            document_type_base_contested_index_registration_fee: 100_000_000_000, // 1 DASH
            token_registration_fee: 10_000_000_000,         // 0.1 DASH
            token_uses_perpetual_distribution_fee: 10_000_000_000, // 0.1 DASH
            token_uses_pre_programmed_distribution_fee: 10_000_000_000, // 0.1 DASH
            search_keyword_fee: 10_000_000_000,             // 0.1 DASH
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
    /// Fee multiplier in permille (1000 = 1x, 2000 = 2x, etc.)
    /// This comes from the current epoch's fee_multiplier_permille()
    fee_multiplier_permille: u64,
}

impl Default for PlatformFeeEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformFeeEstimator {
    /// Default fee multiplier (1x = 1000 permille)
    pub const DEFAULT_FEE_MULTIPLIER_PERMILLE: u64 = 1000;

    pub fn new() -> Self {
        Self {
            min_fees: StateTransitionMinFees::default(),
            storage_fees: StorageFeeConstants::default(),
            registration_fees: DataContractRegistrationFees::default(),
            fee_multiplier_permille: Self::DEFAULT_FEE_MULTIPLIER_PERMILLE,
        }
    }

    /// Create an estimator with a specific fee multiplier (from epoch info)
    pub fn with_fee_multiplier(fee_multiplier_permille: u64) -> Self {
        Self {
            min_fees: StateTransitionMinFees::default(),
            storage_fees: StorageFeeConstants::default(),
            registration_fees: DataContractRegistrationFees::default(),
            fee_multiplier_permille,
        }
    }

    /// Try to create from platform version (for future dynamic fee support)
    pub fn from_platform_version(_platform_version: &PlatformVersion) -> Self {
        // For now, use default fees. In future, could read from platform_version
        Self::new()
    }

    /// Apply the fee multiplier to a base fee amount.
    /// Multiplier is in permille: 1000 = 1x, 1500 = 1.5x, 2000 = 2x
    fn apply_multiplier(&self, base_fee: u64) -> u64 {
        base_fee
            .saturating_mul(self.fee_multiplier_permille)
            .saturating_div(1000)
    }

    /// Get the current fee multiplier permille
    pub fn fee_multiplier_permille(&self) -> u64 {
        self.fee_multiplier_permille
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

    /// Calculate total storage-based fee for storing data (without fee multiplier).
    /// Includes storage, processing, and estimated seek costs.
    /// This is a building block used by other estimation functions.
    fn calculate_storage_based_fee(&self, bytes: usize, estimated_seeks: usize) -> u64 {
        self.calculate_storage_fee(bytes)
            .saturating_add(self.calculate_processing_fee(bytes))
            .saturating_add(self.calculate_seek_fee(estimated_seeks))
    }

    /// Estimate total storage-based fee for storing data.
    /// Includes storage, processing, and estimated seek costs.
    /// Applies the current fee multiplier.
    pub fn estimate_storage_based_fee(&self, bytes: usize, estimated_seeks: usize) -> u64 {
        self.apply_multiplier(self.calculate_storage_based_fee(bytes, estimated_seeks))
    }

    /// Estimate fee for credit transfer between identities
    pub fn estimate_credit_transfer(&self) -> u64 {
        self.apply_multiplier(self.min_fees.credit_transfer)
    }

    /// Estimate fee for credit transfer to platform addresses
    pub fn estimate_credit_transfer_to_addresses(&self, output_count: usize) -> u64 {
        let base_fee = self.min_fees.credit_transfer_to_addresses.saturating_add(
            self.min_fees
                .address_funds_transfer_output_cost
                .saturating_mul(output_count as u64),
        );
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for credit withdrawal to core chain
    pub fn estimate_credit_withdrawal(&self) -> u64 {
        self.apply_multiplier(self.min_fees.credit_withdrawal)
    }

    /// Estimate fee for address-based credit withdrawal
    pub fn estimate_address_credit_withdrawal(&self) -> u64 {
        self.apply_multiplier(self.min_fees.address_credit_withdrawal)
    }

    /// Estimate fee for funding a platform address from an asset lock.
    /// This includes the asset lock processing cost and transfer costs.
    /// Returns fee in duffs (not credits).
    pub fn estimate_address_funding_from_asset_lock_duffs(&self, output_count: usize) -> u64 {
        // The fee includes:
        // - Base transfer cost to addresses
        // - Per-output costs
        // We add a 50% buffer to account for any additional costs
        let base_fee_credits = self.estimate_credit_transfer_to_addresses(output_count);
        let fee_duffs = base_fee_credits / 1000; // Convert credits to duffs
        // Add 50% buffer and ensure minimum of 10,000 duffs based on observed behavior
        fee_duffs.saturating_add(fee_duffs / 2).max(10_000)
    }

    /// Estimate fee for identity update (adding/disabling keys)
    pub fn estimate_identity_update(&self) -> u64 {
        self.apply_multiplier(self.min_fees.identity_update)
    }

    /// Estimate fee for identity creation.
    /// This includes base cost, asset lock cost, and per-key costs.
    pub fn estimate_identity_create(&self, key_count: usize) -> u64 {
        let base_fee = self
            .min_fees
            .identity_create_base_cost
            .saturating_add(self.min_fees.identity_create_asset_lock_cost)
            .saturating_add(
                self.min_fees
                    .identity_key_in_creation_cost
                    .saturating_mul(key_count as u64),
            );
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for identity creation from addresses (asset lock).
    /// This includes base cost, asset lock cost, input/output costs, per-key costs,
    /// storage-based fees, and a 20% safety buffer to account for fee variability.
    pub fn estimate_identity_create_from_addresses(
        &self,
        input_count: usize,
        has_output: bool,
        key_count: usize,
    ) -> u64 {
        // Estimated serialized bytes per input (address + signature/witness data)
        const ESTIMATED_BYTES_PER_INPUT: usize = 225;
        // Estimated bytes for identity structure + keys
        const ESTIMATED_IDENTITY_BASE_BYTES: usize = 100;
        const ESTIMATED_BYTES_PER_KEY: usize = 50;
        // Estimated seek operations for tree traversal
        const ESTIMATED_SEEKS_BASE: usize = 10;

        let output_count = if has_output { 1 } else { 0 };
        let inputs = input_count.max(1);

        // Base fee from min fee structure
        // Note: identity creation requires the full identity_create_asset_lock_cost,
        // not the smaller address_funding_asset_lock_cost used for simple transfers
        let base_fee = self
            .min_fees
            .identity_create_base_cost
            .saturating_add(self.min_fees.identity_create_asset_lock_cost)
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_input_cost
                    .saturating_mul(inputs as u64),
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
            );

        // Add storage-based fees for serialized transaction data
        let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT
            + ESTIMATED_IDENTITY_BASE_BYTES
            + key_count * ESTIMATED_BYTES_PER_KEY;
        let estimated_seeks = ESTIMATED_SEEKS_BASE + inputs;
        let storage_fee = self.calculate_storage_based_fee(estimated_bytes, estimated_seeks);

        // Total with fee multiplier
        let total = self.apply_multiplier(base_fee.saturating_add(storage_fee));

        // Add 20% safety buffer to account for fee variability
        apply_fee_safety_margin(total, 20)
    }

    /// Estimate fee for identity top-up.
    /// This includes base cost and asset lock cost.
    pub fn estimate_identity_topup(&self) -> u64 {
        let base_fee = self
            .min_fees
            .identity_topup_base_cost
            .saturating_add(self.min_fees.identity_topup_asset_lock_cost);
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for identity top-up from platform addresses.
    /// This includes base cost, asset lock cost, input costs, storage-based fees,
    /// and a 20% safety buffer to account for fee variability.
    pub fn estimate_identity_topup_from_addresses(&self, input_count: usize) -> u64 {
        // Estimated serialized bytes per input (address + signature/witness data)
        const ESTIMATED_BYTES_PER_INPUT: usize = 225;
        // Estimated bytes for top-up transaction structure
        const ESTIMATED_TOPUP_BASE_BYTES: usize = 100;
        // Estimated seek operations for tree traversal
        const ESTIMATED_SEEKS_BASE: usize = 8;

        let inputs = input_count.max(1);

        // Base fee from min fee structure
        let base_fee = self
            .min_fees
            .identity_topup_base_cost
            .saturating_add(self.min_fees.address_funding_asset_lock_cost)
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_input_cost
                    .saturating_mul(inputs as u64),
            );

        // Add storage-based fees for serialized transaction data
        let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT + ESTIMATED_TOPUP_BASE_BYTES;
        let estimated_seeks = ESTIMATED_SEEKS_BASE + inputs;
        let storage_fee = self.calculate_storage_based_fee(estimated_bytes, estimated_seeks);

        // Total with fee multiplier
        let total = self.apply_multiplier(base_fee.saturating_add(storage_fee));

        // Add 20% safety buffer to account for fee variability
        apply_fee_safety_margin(total, 20)
    }

    /// Estimate fee for document batch transition
    pub fn estimate_document_batch(&self, transition_count: usize) -> u64 {
        let base_fee = self
            .min_fees
            .document_batch_sub_transition
            .saturating_mul(transition_count.max(1) as u64);
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for document creation with known size.
    /// Documents are stored in the contract's document tree.
    /// Estimated seeks: ~10 for tree traversal and insertion.
    pub fn estimate_document_create_with_size(&self, document_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        let base_fee = self
            .min_fees
            .document_batch_sub_transition
            .saturating_add(self.calculate_storage_based_fee(document_bytes, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
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
        let base_fee = self
            .min_fees
            .document_batch_sub_transition
            .saturating_add(self.calculate_seek_fee(ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for document replacement with known size.
    pub fn estimate_document_replace_with_size(&self, document_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        let base_fee = self
            .min_fees
            .document_batch_sub_transition
            .saturating_add(self.calculate_storage_based_fee(document_bytes, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
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
        let base_fee = self.min_fees.document_batch_sub_transition.saturating_add(
            self.calculate_storage_based_fee(OWNERSHIP_UPDATE_BYTES, ESTIMATED_SEEKS),
        );
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for document purchase.
    pub fn estimate_document_purchase(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        const PURCHASE_UPDATE_BYTES: usize = 100;
        let base_fee = self.min_fees.document_batch_sub_transition.saturating_add(
            self.calculate_storage_based_fee(PURCHASE_UPDATE_BYTES, ESTIMATED_SEEKS),
        );
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for document set price.
    pub fn estimate_document_set_price(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const PRICE_UPDATE_BYTES: usize = 32;
        let base_fee = self
            .min_fees
            .document_batch_sub_transition
            .saturating_add(self.calculate_storage_based_fee(PRICE_UPDATE_BYTES, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for token transition (mint, burn, transfer, freeze, etc.).
    /// Token operations are relatively small - mainly balance updates.
    pub fn estimate_token_transition(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const TOKEN_OP_BYTES: usize = 100;
        let base_fee = self
            .min_fees
            .document_batch_sub_transition
            .saturating_add(self.calculate_storage_based_fee(TOKEN_OP_BYTES, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for data contract creation with known size.
    /// Includes base registration fee (0.1 DASH) plus storage costs.
    /// For contracts with tokens, document types, or indexes, use the detailed method.
    pub fn estimate_contract_create_with_size(&self, contract_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 20;
        let base_fee = self
            .registration_fees
            .base_contract_registration_fee
            .saturating_add(self.min_fees.contract_create)
            .saturating_add(self.calculate_storage_based_fee(contract_bytes, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for data contract creation with detailed component counts.
    /// This provides the most accurate estimate by accounting for all registration fees.
    #[allow(clippy::too_many_arguments)]
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

        let mut base_fee = self.registration_fees.base_contract_registration_fee;

        // Document type fees
        base_fee = base_fee.saturating_add(
            self.registration_fees
                .document_type_registration_fee
                .saturating_mul(document_type_count as u64),
        );

        // Index fees
        base_fee = base_fee.saturating_add(
            self.registration_fees
                .document_type_base_non_unique_index_registration_fee
                .saturating_mul(non_unique_index_count as u64),
        );
        base_fee = base_fee.saturating_add(
            self.registration_fees
                .document_type_base_unique_index_registration_fee
                .saturating_mul(unique_index_count as u64),
        );
        base_fee = base_fee.saturating_add(
            self.registration_fees
                .document_type_base_contested_index_registration_fee
                .saturating_mul(contested_index_count as u64),
        );

        // Token fees
        if has_token {
            base_fee = base_fee.saturating_add(self.registration_fees.token_registration_fee);
        }
        if has_perpetual_distribution {
            base_fee = base_fee
                .saturating_add(self.registration_fees.token_uses_perpetual_distribution_fee);
        }
        if has_pre_programmed_distribution {
            base_fee = base_fee.saturating_add(
                self.registration_fees
                    .token_uses_pre_programmed_distribution_fee,
            );
        }

        // Search keyword fees
        base_fee = base_fee.saturating_add(
            self.registration_fees
                .search_keyword_fee
                .saturating_mul(search_keyword_count as u64),
        );

        // Add state transition minimum and storage fees
        base_fee = base_fee.saturating_add(self.min_fees.contract_create);
        base_fee = base_fee
            .saturating_add(self.calculate_storage_based_fee(contract_bytes, ESTIMATED_SEEKS));

        self.apply_multiplier(base_fee)
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
        let base_fee = self
            .min_fees
            .contract_update
            .saturating_add(self.calculate_storage_based_fee(update_bytes, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
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
        self.apply_multiplier(self.min_fees.masternode_vote)
    }

    /// Estimate fee for address funds transfer.
    /// Applies the current fee multiplier.
    pub fn estimate_address_funds_transfer(&self, input_count: usize, output_count: usize) -> u64 {
        let base_fee = self
            .min_fees
            .address_funds_transfer_input_cost
            .saturating_mul(input_count as u64)
            .saturating_add(
                self.min_fees
                    .address_funds_transfer_output_cost
                    .saturating_mul(output_count.max(1) as u64),
            );
        self.apply_multiplier(base_fee)
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

/// Estimate the byte size of a P2PKH transaction given the number of inputs and outputs.
///
/// Uses standard sizes: 148 bytes per input, 34 bytes per output, plus overhead.
pub fn estimate_p2pkh_tx_size(inputs: usize, outputs: usize) -> usize {
    fn varint_size(value: usize) -> usize {
        match value {
            0..=0xfc => 1,
            0xfd..=0xffff => 3,
            0x1_0000..=0xffff_ffff => 5,
            _ => 9,
        }
    }

    let mut size = 8; // version/type/lock_time
    size += varint_size(inputs);
    size += varint_size(outputs);
    size += inputs * 148; // P2PKH input size
    size += outputs * 34; // P2PKH output size
    size
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

/// Apply a safety margin to a fee estimate.
///
/// Adds the specified percentage as a buffer to account for fee variability
/// between estimation and actual platform execution. Uses saturating arithmetic
/// to prevent overflow.
///
/// # Arguments
/// * `fee` - The base fee estimate
/// * `percent` - The safety margin percentage (e.g., 20 for 20%)
pub fn apply_fee_safety_margin(fee: u64, percent: u64) -> u64 {
    fee.saturating_add(fee.saturating_mul(percent) / 100)
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
        // Base cost + asset lock cost + 2 keys
        let fee = estimator.estimate_identity_create(2);
        assert_eq!(fee, 2_000_000 + 200_000_000 + 2 * 6_500_000);
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

    // =====================================================
    // apply_fee_safety_margin tests
    // =====================================================

    #[test]
    fn test_apply_fee_safety_margin_20_percent() {
        // 100 + 20% = 120
        assert_eq!(apply_fee_safety_margin(100, 20), 120);
        // 1000 + 20% = 1200
        assert_eq!(apply_fee_safety_margin(1000, 20), 1200);
        // 1_000_000 + 20% = 1_200_000
        assert_eq!(apply_fee_safety_margin(1_000_000, 20), 1_200_000);
    }

    #[test]
    fn test_apply_fee_safety_margin_zero_percent() {
        // 0% margin should return the original fee
        assert_eq!(apply_fee_safety_margin(100, 0), 100);
        assert_eq!(apply_fee_safety_margin(0, 0), 0);
        assert_eq!(apply_fee_safety_margin(u64::MAX, 0), u64::MAX);
    }

    #[test]
    fn test_apply_fee_safety_margin_zero_fee() {
        // Zero fee should always return zero regardless of percent
        assert_eq!(apply_fee_safety_margin(0, 20), 0);
        assert_eq!(apply_fee_safety_margin(0, 100), 0);
        assert_eq!(apply_fee_safety_margin(0, u64::MAX), 0);
    }

    #[test]
    fn test_apply_fee_safety_margin_100_percent() {
        // 100% margin should double the fee
        assert_eq!(apply_fee_safety_margin(100, 100), 200);
        assert_eq!(apply_fee_safety_margin(500_000, 100), 1_000_000);
    }

    #[test]
    fn test_apply_fee_safety_margin_overflow_protection() {
        // Very large fee * large percent uses saturating arithmetic — must not panic
        let result = apply_fee_safety_margin(u64::MAX, 20);
        // u64::MAX.saturating_mul(20) = u64::MAX, / 100 = 184467440737095516
        // u64::MAX.saturating_add(184467440737095516) = u64::MAX
        assert_eq!(result, u64::MAX);

        // u64::MAX / 2 with 200%: saturating_mul(200) = u64::MAX, /100 = 184467440737095516
        // (u64::MAX/2).saturating_add(184467440737095516) does not overflow
        let result = apply_fee_safety_margin(u64::MAX / 2, 200);
        assert!(result > u64::MAX / 2); // margin was added, no panic

        // Small percent on near-max: no panic
        let result = apply_fee_safety_margin(u64::MAX - 1, 1);
        assert!(result > u64::MAX - 1); // margin was added
    }

    #[test]
    fn test_apply_fee_safety_margin_small_values() {
        // Small fee where percent rounds down due to integer division
        // 1 + (1 * 20 / 100) = 1 + 0 = 1 (integer division truncates)
        assert_eq!(apply_fee_safety_margin(1, 20), 1);
        // 4 + (4 * 20 / 100) = 4 + 0 = 4 (80/100 = 0 in integer division)
        assert_eq!(apply_fee_safety_margin(4, 20), 4);
        // 5 + (5 * 20 / 100) = 5 + 1 = 6 (100/100 = 1)
        assert_eq!(apply_fee_safety_margin(5, 20), 6);
    }

    // =====================================================
    // estimate_p2pkh_tx_size tests
    // =====================================================

    #[test]
    fn test_estimate_p2pkh_tx_size_single_input_single_output() {
        // 1 input, 1 output: 8 + 1 + 1 + 148 + 34 = 192
        let size = estimate_p2pkh_tx_size(1, 1);
        assert_eq!(size, 192);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_single_input_two_outputs() {
        // Standard: 1 input, 2 outputs (recipient + change)
        // 8 + 1 + 1 + 148 + 2*34 = 226
        let size = estimate_p2pkh_tx_size(1, 2);
        assert_eq!(size, 226);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_two_inputs_two_outputs() {
        // 2 inputs, 2 outputs: 8 + 1 + 1 + 2*148 + 2*34 = 374
        let size = estimate_p2pkh_tx_size(2, 2);
        assert_eq!(size, 374);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_zero_inputs() {
        // Edge case: 0 inputs (shouldn't happen in practice but shouldn't panic)
        // 8 + 1 + 1 + 0 + 34 = 44
        let size = estimate_p2pkh_tx_size(0, 1);
        assert_eq!(size, 44);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_zero_outputs() {
        // Edge case: 0 outputs (shouldn't happen in practice but shouldn't panic)
        // 8 + 1 + 1 + 148 + 0 = 158
        let size = estimate_p2pkh_tx_size(1, 0);
        assert_eq!(size, 158);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_zero_both() {
        // Edge case: 0 inputs and 0 outputs
        // 8 + 1 + 1 = 10
        let size = estimate_p2pkh_tx_size(0, 0);
        assert_eq!(size, 10);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_many_inputs() {
        // 10 inputs, 2 outputs: 8 + 1 + 1 + 10*148 + 2*34 = 1558
        let size = estimate_p2pkh_tx_size(10, 2);
        assert_eq!(size, 1558);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_varint_boundary() {
        // At 253 inputs, varint switches from 1 byte to 3 bytes
        let size_252 = estimate_p2pkh_tx_size(252, 1);
        let size_253 = estimate_p2pkh_tx_size(253, 1);
        // 252 inputs: 8 + 1 + 1 + 252*148 + 34 = 37340
        assert_eq!(size_252, 8 + 1 + 1 + 252 * 148 + 34);
        // 253 inputs: 8 + 3 + 1 + 253*148 + 34 = 37490
        assert_eq!(size_253, 8 + 3 + 1 + 253 * 148 + 34);
        // The 253rd input adds 148 bytes for the input + 2 bytes for varint growth
        assert_eq!(size_253 - size_252, 148 + 2);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_large_transaction() {
        // Very large: 500 inputs, 100 outputs
        let size = estimate_p2pkh_tx_size(500, 100);
        // 8 + 3 + 1 + 500*148 + 100*34 = 8 + 3 + 1 + 74000 + 3400 = 77412
        assert_eq!(size, 77412);
    }

    #[test]
    fn test_estimate_p2pkh_tx_size_scales_linearly_with_inputs() {
        let size_1 = estimate_p2pkh_tx_size(1, 2);
        let size_2 = estimate_p2pkh_tx_size(2, 2);
        let size_3 = estimate_p2pkh_tx_size(3, 2);
        // Each additional input adds exactly 148 bytes (within same varint range)
        assert_eq!(size_2 - size_1, 148);
        assert_eq!(size_3 - size_2, 148);
    }

    // =====================================================
    // Fee multiplier tests
    // =====================================================

    #[test]
    fn test_fee_multiplier_doubles_fees() {
        let estimator_1x = PlatformFeeEstimator::new();
        let estimator_2x = PlatformFeeEstimator::with_fee_multiplier(2000);
        assert_eq!(
            estimator_2x.estimate_credit_transfer(),
            estimator_1x.estimate_credit_transfer() * 2
        );
    }

    #[test]
    fn test_fee_multiplier_fractional() {
        let estimator = PlatformFeeEstimator::with_fee_multiplier(1500); // 1.5x
        // 100,000 * 1500 / 1000 = 150,000
        assert_eq!(estimator.estimate_credit_transfer(), 150_000);
    }

    #[test]
    fn test_fee_multiplier_zero() {
        let estimator = PlatformFeeEstimator::with_fee_multiplier(0);
        assert_eq!(estimator.estimate_credit_transfer(), 0);
    }

    // =====================================================
    // Platform fee estimation edge cases
    // =====================================================

    #[test]
    fn test_identity_create_zero_keys() {
        let estimator = PlatformFeeEstimator::new();
        let fee = estimator.estimate_identity_create(0);
        // Base cost + asset lock cost + 0 keys
        assert_eq!(fee, 2_000_000 + 200_000_000);
    }

    #[test]
    fn test_identity_create_many_keys() {
        let estimator = PlatformFeeEstimator::new();
        let fee_5 = estimator.estimate_identity_create(5);
        let fee_10 = estimator.estimate_identity_create(10);
        // Each additional key adds 6,500,000 credits
        assert_eq!(fee_10 - fee_5, 5 * 6_500_000);
    }

    #[test]
    fn test_identity_topup_estimate() {
        let estimator = PlatformFeeEstimator::new();
        let fee = estimator.estimate_identity_topup();
        // Base cost + asset lock cost
        assert_eq!(fee, 500_000 + 50_000_000);
    }

    #[test]
    fn test_identity_create_from_addresses_includes_safety_margin() {
        let estimator = PlatformFeeEstimator::new();
        let fee = estimator.estimate_identity_create_from_addresses(1, false, 2);
        // The fee should be > 0 and include the 20% safety margin
        assert!(fee > 0);
        // Verify safety margin: fee should be approximately 1.2x the base
        // We can verify by checking that fee is larger than the base identity create
        let base = estimator.estimate_identity_create(2);
        assert!(fee > base); // From-addresses version includes storage fees + safety margin
    }

    #[test]
    fn test_identity_topup_from_addresses_includes_safety_margin() {
        let estimator = PlatformFeeEstimator::new();
        let fee = estimator.estimate_identity_topup_from_addresses(1);
        assert!(fee > 0);
        // Should be more than basic topup due to address processing + safety margin
        let basic_topup = estimator.estimate_identity_topup();
        assert!(fee > basic_topup);
    }

    #[test]
    fn test_identity_create_from_addresses_zero_inputs_uses_one() {
        let estimator = PlatformFeeEstimator::new();
        // 0 inputs should be treated as 1 (via .max(1))
        let fee_0 = estimator.estimate_identity_create_from_addresses(0, false, 2);
        let fee_1 = estimator.estimate_identity_create_from_addresses(1, false, 2);
        assert_eq!(fee_0, fee_1);
    }

    #[test]
    fn test_document_batch_zero_transitions_uses_one() {
        let estimator = PlatformFeeEstimator::new();
        // 0 transitions should be treated as 1 (via .max(1))
        let fee_0 = estimator.estimate_document_batch(0);
        let fee_1 = estimator.estimate_document_batch(1);
        assert_eq!(fee_0, fee_1);
    }

    #[test]
    fn test_address_funding_from_asset_lock_minimum() {
        let estimator = PlatformFeeEstimator::new();
        // Should have a minimum of 10,000 duffs
        let fee = estimator.estimate_address_funding_from_asset_lock_duffs(1);
        assert!(fee >= 10_000);
    }

    #[test]
    fn test_credit_transfer_to_addresses_scales_with_outputs() {
        let estimator = PlatformFeeEstimator::new();
        let fee_1 = estimator.estimate_credit_transfer_to_addresses(1);
        let fee_3 = estimator.estimate_credit_transfer_to_addresses(3);
        // Each additional output adds address_funds_transfer_output_cost (6,000,000)
        assert_eq!(fee_3 - fee_1, 2 * 6_000_000);
    }

    #[test]
    fn test_address_funds_transfer_scales_with_inputs_and_outputs() {
        let estimator = PlatformFeeEstimator::new();
        let fee_1_1 = estimator.estimate_address_funds_transfer(1, 1);
        let fee_2_1 = estimator.estimate_address_funds_transfer(2, 1);
        let fee_1_2 = estimator.estimate_address_funds_transfer(1, 2);
        // Adding 1 input adds 500,000 credits
        assert_eq!(fee_2_1 - fee_1_1, 500_000);
        // Adding 1 output adds 6,000,000 credits
        assert_eq!(fee_1_2 - fee_1_1, 6_000_000);
    }

    #[test]
    fn test_address_funds_transfer_zero_outputs_uses_one() {
        let estimator = PlatformFeeEstimator::new();
        // 0 outputs should be treated as 1 (via .max(1))
        let fee_0 = estimator.estimate_address_funds_transfer(1, 0);
        let fee_1 = estimator.estimate_address_funds_transfer(1, 1);
        assert_eq!(fee_0, fee_1);
    }

    // =====================================================
    // format_credits tests
    // =====================================================

    #[test]
    fn test_format_credits_as_dash_zero() {
        assert_eq!(format_credits_as_dash(0), "0.00000000 DASH");
    }

    #[test]
    fn test_format_credits_as_dash_one_credit() {
        // 1 credit is very small
        assert_eq!(format_credits_as_dash(1), "0.00000000 DASH");
    }

    #[test]
    fn test_format_credits_large_vs_small_formatting() {
        // format_credits uses 8 decimal places for >= 1 billion credits
        let large = format_credits(1_000_000_000);
        assert!(large.contains("credits"));
        assert!(large.contains("DASH"));

        // Uses 10 decimal places for < 1 billion credits
        let small = format_credits(999_999_999);
        assert!(small.contains("credits"));
        assert!(small.contains("DASH"));
    }

    #[test]
    fn test_format_credits_zero() {
        let result = format_credits(0);
        assert!(result.starts_with("0 credits"));
    }

    // =====================================================
    // Storage fee calculation tests
    // =====================================================

    #[test]
    fn test_storage_fee_zero_bytes() {
        let estimator = PlatformFeeEstimator::new();
        assert_eq!(estimator.calculate_storage_fee(0), 0);
        assert_eq!(estimator.calculate_processing_fee(0), 0);
        assert_eq!(estimator.calculate_seek_fee(0), 0);
    }

    #[test]
    fn test_storage_based_fee_zero_everything() {
        let estimator = PlatformFeeEstimator::new();
        assert_eq!(estimator.estimate_storage_based_fee(0, 0), 0);
    }

    #[test]
    fn test_storage_based_fee_components() {
        let estimator = PlatformFeeEstimator::new();
        let bytes = 100;
        let seeks = 5;
        let fee = estimator.estimate_storage_based_fee(bytes, seeks);
        // At 1x multiplier: storage + processing + seeks
        let expected = 100 * 27_000 + 100 * 400 + 5 * 2_000;
        assert_eq!(fee, expected);
    }

    // =====================================================
    // Document estimation tests
    // =====================================================

    #[test]
    fn test_document_create_default_size() {
        let estimator = PlatformFeeEstimator::new();
        let fee_default = estimator.estimate_document_create();
        let fee_200 = estimator.estimate_document_create_with_size(200);
        // Default uses 200 bytes
        assert_eq!(fee_default, fee_200);
    }

    #[test]
    fn test_document_delete_cheaper_than_create() {
        let estimator = PlatformFeeEstimator::new();
        let create_fee = estimator.estimate_document_create();
        let delete_fee = estimator.estimate_document_delete();
        // Deletion should be cheaper (no storage addition)
        assert!(delete_fee < create_fee);
    }

    #[test]
    fn test_document_replace_default_size() {
        let estimator = PlatformFeeEstimator::new();
        let fee_default = estimator.estimate_document_replace();
        let fee_200 = estimator.estimate_document_replace_with_size(200);
        assert_eq!(fee_default, fee_200);
    }

    // =====================================================
    // Contract registration detailed tests
    // =====================================================

    #[test]
    fn test_contract_create_base_uses_500_bytes() {
        let estimator = PlatformFeeEstimator::new();
        let base = estimator.estimate_contract_create_base();
        let with_500 = estimator.estimate_contract_create_with_size(500);
        assert_eq!(base, with_500);
    }

    #[test]
    fn test_contract_create_detailed_all_features() {
        let estimator = PlatformFeeEstimator::new();
        let fee = estimator.estimate_contract_create_detailed(
            1000, // contract bytes
            3,    // 3 document types
            2,    // 2 non-unique indexes
            1,    // 1 unique index
            1,    // 1 contested index
            true, // has token
            true, // has perpetual distribution
            true, // has pre-programmed distribution
            2,    // 2 search keywords
        );
        // Registration fees:
        // base: 10B + doc_types: 3*2B + non_unique: 2*1B + unique: 1*1B + contested: 1*100B
        // + token: 10B + perpetual: 10B + pre_programmed: 10B + keywords: 2*10B
        let expected_registration: u64 = 10_000_000_000
            + 3 * 2_000_000_000
            + 2 * 1_000_000_000
            + 1_000_000_000
            + 100_000_000_000
            + 10_000_000_000
            + 10_000_000_000
            + 10_000_000_000
            + 2 * 10_000_000_000;
        // Plus min fee + storage
        assert!(fee >= expected_registration);
    }

    #[test]
    fn test_contract_create_contested_index_is_expensive() {
        let estimator = PlatformFeeEstimator::new();
        let fee_no_contested =
            estimator.estimate_contract_create_detailed(500, 1, 1, 0, 0, false, false, false, 0);
        let fee_contested =
            estimator.estimate_contract_create_detailed(500, 1, 1, 0, 1, false, false, false, 0);
        // Contested index adds 1 DASH (100,000,000,000 credits)
        assert_eq!(fee_contested - fee_no_contested, 100_000_000_000);
    }

    #[test]
    fn test_contract_update_default_size() {
        let estimator = PlatformFeeEstimator::new();
        let fee_default = estimator.estimate_contract_update();
        let fee_300 = estimator.estimate_contract_update_with_size(300);
        assert_eq!(fee_default, fee_300);
    }
}
