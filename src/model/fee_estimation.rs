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

use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::address_funds::{AddressFundsFeeStrategyStep, PlatformAddress};
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits};
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::prelude::{AddressNonce, AssetLockProof};
use dash_sdk::dpp::state_transition::StateTransitionEstimatedFeeValidation;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::AddressCreditWithdrawalTransition;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::v0::AddressCreditWithdrawalTransitionV0;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::AddressFundingFromAssetLockTransition;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::v0::AddressFundingFromAssetLockTransitionV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::Pooling;
use std::collections::BTreeMap;

/// Maximum number of platform address inputs allowed per state transition.
pub(crate) const MAX_PLATFORM_INPUTS: usize = 16;

/// Estimated serialized bytes per input (address + signature/witness data).
const ESTIMATED_BYTES_PER_INPUT: usize = 225;

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

/// Component counts describing a data contract, used for detailed fee estimation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContractComponents {
    /// Serialized size of the contract in bytes.
    pub contract_bytes: usize,
    /// Number of document types defined in the contract.
    pub document_type_count: usize,
    /// Number of non-unique indexes across all document types.
    pub non_unique_index_count: usize,
    /// Number of unique indexes across all document types.
    pub unique_index_count: usize,
    /// Number of contested indexes across all document types.
    pub contested_index_count: usize,
    /// Whether the contract defines a token.
    pub has_token: bool,
    /// Whether the token uses perpetual distribution.
    pub has_perpetual_distribution: bool,
    /// Whether the token uses pre-programmed distribution.
    pub has_pre_programmed_distribution: bool,
    /// Number of search keywords registered for the contract.
    pub search_keyword_count: usize,
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
        let fee_duffs = base_fee_credits / CREDITS_PER_DUFF;
        // Add 50% buffer and ensure minimum of 10,000 duffs based on observed behavior
        fee_duffs.saturating_add(fee_duffs / 2).max(10_000)
    }

    /// Estimate fees (in duffs) for a shield-from-core asset lock operation.
    ///
    /// Returns `(platform_fee_duffs, l1_tx_fee_duffs)`:
    /// - Platform fee: `address_funding_asset_lock_cost` with fee multiplier applied,
    ///   converted to duffs, plus 20% buffer
    /// - L1 tx fee: flat estimate covering Core minimum relay fee (~3000 duffs)
    pub fn estimate_shield_from_core_fees_duffs(&self) -> (u64, u64) {
        let platform_fee_credits =
            self.apply_multiplier(self.min_fees.address_funding_asset_lock_cost);
        let platform_fee_duffs =
            (platform_fee_credits / CREDITS_PER_DUFF).saturating_mul(120) / 100;
        let l1_tx_fee_duffs = 3_000_u64;
        (platform_fee_duffs, l1_tx_fee_duffs)
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
        total.saturating_add(total / 5)
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
        total.saturating_add(total / 5)
    }

    /// Resolve the actual fee paid by a wallet-funded identity top-up.
    ///
    /// A top-up converts `amount_duffs` of asset-lock value into
    /// `amount_duffs × CREDITS_PER_DUFF` credits, less the Platform processing
    /// fee. That fee is the shortfall between the credits the asset lock should
    /// have minted and the balance the identity actually gained:
    ///
    /// ```text
    /// actual_fee = expected_credits − (balance_after − balance_before)
    /// ```
    ///
    /// The subtraction is only meaningful when `balance_before` is the
    /// identity's true pre-top-up balance. After a backend reload the caller may
    /// hold a stale cached balance — too low (inflating the apparent increase
    /// and collapsing the delta toward zero) or too high (the apparent increase
    /// shrinks and the delta swells toward the full minted amount). Either skew
    /// drifts the measured fee away from what the top-up actually cost, so the
    /// measured fee is trusted only when it is physically possible **and** lands
    /// in a plausible band relative to the deterministic estimate; otherwise the
    /// estimate — the trustworthy value — is returned.
    pub fn resolve_identity_topup_actual_fee(
        &self,
        amount_duffs: u64,
        balance_before: u64,
        balance_after: u64,
    ) -> u64 {
        let expected_credits = amount_duffs.saturating_mul(CREDITS_PER_DUFF);
        let balance_increase = balance_after.saturating_sub(balance_before);
        let delta_fee = expected_credits.saturating_sub(balance_increase);

        let estimate = self.estimate_identity_topup();

        // Plausibility band for the measured fee. Three conditions must all hold:
        //
        //  • `0 < delta_fee` — a real top-up always pays a non-zero Platform fee.
        //    A stale-LOW `balance_before` inflates the apparent increase to ≥100 %
        //    of the mint and collapses the delta to zero.
        //  • `delta_fee < expected_credits` — the fee can never exceed what the
        //    asset lock minted. A stale-HIGH `balance_before` makes the increase
        //    saturate to zero, swelling the delta to the full minted amount.
        //  • `delta_fee <= plausible_upper` — the deterministic estimate already
        //    over-states the fee (it bills the full asset-lock processing cost),
        //    so a real fee sits at or below it; `×2` leaves headroom for storage
        //    and epoch variance. A *partial*-stale `balance_before` yields a delta
        //    that is non-zero and below the mint yet grossly inflated past the
        //    estimate — caught here where the two boundary checks above miss it.
        //
        // The low side stays at `0 < delta_fee`: the estimate over-predicts, so a
        // legitimately small real fee (well under the estimate) must not be
        // rejected — no tighter lower bound is defensible.
        let plausible_upper = estimate.saturating_mul(2);
        if 0 < delta_fee && delta_fee < expected_credits && delta_fee <= plausible_upper {
            delta_fee
        } else {
            estimate
        }
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
    pub fn estimate_contract_create_detailed(&self, components: ContractComponents) -> u64 {
        const ESTIMATED_SEEKS: usize = 20;

        let ContractComponents {
            contract_bytes,
            document_type_count,
            non_unique_index_count,
            unique_index_count,
            contested_index_count,
            has_token,
            has_perpetual_distribution,
            has_pre_programmed_distribution,
            search_keyword_count,
        } = components;

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

/// Credits per DASH: 1 DASH = 10^DASH_DECIMAL_PLACES credits (100 billion).
pub const CREDITS_PER_DASH: u64 = 10u64.pow(DASH_DECIMAL_PLACES as u32);

/// Format credits as DASH for display
pub fn format_credits_as_dash(credits: u64) -> String {
    Amount::dash_from_credits(credits).to_string()
}

/// Format an amount in duffs as DASH for display.
pub fn format_duffs_as_dash(duffs: u64) -> String {
    Amount::dash_from_duffs(duffs).to_string()
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

/// Calculate the estimated fee for a platform address funds transfer.
///
/// Uses [`PlatformFeeEstimator`] for base costs (input/output fees) plus storage fees.
pub(crate) fn estimate_platform_fee(estimator: &PlatformFeeEstimator, input_count: usize) -> u64 {
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
    total.saturating_add(total / 5)
}

/// Calculate the estimated fee for a Platform address withdrawal using a constructed state transition.
pub(crate) fn estimate_withdrawal_fee_from_transition(
    platform_version: &PlatformVersion,
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
pub(crate) fn estimate_address_funding_fee_from_transition(
    platform_version: &PlatformVersion,
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
pub(crate) struct AddressAllocationResult {
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
pub(crate) fn allocate_platform_addresses_with_fee<F>(
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
pub(crate) fn allocate_platform_addresses(
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

/// Estimate the Core (L1) network fee, in duffs, for a simple wallet send.
///
/// Mirrors the upstream key-wallet `TransactionBuilder` used by
/// `WalletBackend::send_payment`: it builds at the default `FeeRate::normal()`
/// (1 duff per byte) and sizes a non-SegWit P2PKH transaction as
/// `10 + inputs × 148 + outputs × 34` bytes. A "Max" send spends every UTXO
/// into a single recipient output with no change, so pass the wallet's full
/// UTXO count as `num_inputs` and the recipient count as `num_outputs`.
///
/// A 15% safety margin is added on top of the raw size-based fee so the
/// reserved amount comfortably covers the fee the builder actually charges
/// (which rounds up, and may vary slightly with real script sizes). Reserving
/// marginally more than needed leaves a few dust duffs in the wallet — always
/// safe — whereas under-reserving would make the send fail.
///
/// `num_inputs` and `num_outputs` are clamped to a minimum of 1.
pub fn estimate_core_l1_send_fee_duffs(num_inputs: usize, num_outputs: usize) -> u64 {
    const TX_BASE_BYTES: u64 = 10;
    const BYTES_PER_INPUT: u64 = 148;
    const BYTES_PER_OUTPUT: u64 = 34;
    /// Default `FeeRate::normal()` in the upstream builder: 1 duff per byte.
    const DUFFS_PER_BYTE: u64 = 1;
    /// Extra headroom over the raw size estimate, in percent.
    const SAFETY_MARGIN_PERCENT: u64 = 15;

    let inputs = num_inputs.max(1) as u64;
    let outputs = num_outputs.max(1) as u64;

    let size_bytes = TX_BASE_BYTES
        .saturating_add(inputs.saturating_mul(BYTES_PER_INPUT))
        .saturating_add(outputs.saturating_mul(BYTES_PER_OUTPUT));

    let raw_fee = size_bytes.saturating_mul(DUFFS_PER_BYTE);
    raw_fee.saturating_add(raw_fee.saturating_mul(SAFETY_MARGIN_PERCENT) / 100)
}

/// Compute the maximum spendable amount, in duffs, for a Core "Max" send:
/// the whole balance minus the estimated L1 network fee.
///
/// Returns `None` when the balance does not cover the estimated fee (i.e.
/// nothing is left to send). Callers should disable "Max" and show a calm
/// message in that case rather than producing an amount that would fail.
///
/// `num_inputs` is the wallet's UTXO count and `num_outputs` the recipient
/// count; both are passed through to [`estimate_core_l1_send_fee_duffs`].
pub fn core_max_send_amount_duffs(
    balance_duffs: u64,
    num_inputs: usize,
    num_outputs: usize,
) -> Option<u64> {
    let fee = estimate_core_l1_send_fee_duffs(num_inputs, num_outputs);
    let spendable = balance_duffs.checked_sub(fee)?;
    (spendable > 0).then_some(spendable)
}

/// The duffs a Core "Max" send must reserve for the L1 network fee — the
/// difference between the spendable balance and [`core_max_send_amount_duffs`].
///
/// Returns `None` in lockstep with `core_max_send_amount_duffs`: when the
/// spendable balance cannot cover the fee there is no valid Max to reserve
/// against, so callers disable "Max" rather than show a reserve for a send
/// that would fail.
///
/// `spendable_duffs` MUST be the spendable balance (confirmed + unconfirmed),
/// never the headline `total` — `total` counts immature coinbase and locked
/// CoinJoin funds the upstream `CoinSelector` rejects, so reserving against it
/// over-shoots the selectable set and the broadcast fails.
pub fn core_max_send_reserve_duffs(
    spendable_duffs: u64,
    num_inputs: usize,
    num_outputs: usize,
) -> Option<u64> {
    let max = core_max_send_amount_duffs(spendable_duffs, num_inputs, num_outputs)?;
    Some(spendable_duffs.saturating_sub(max))
}

/// Compute the exact shielded fee for a given number of Orchard actions.
///
/// Wraps `compute_minimum_shielded_fee` from `dpp`. Use this to calculate
/// the fee after note selection, when the action count is known.
///
/// Returns the fee in credits, or a boxed [`ProtocolError`] when the active
/// protocol version has no known shielded-fee formula. The error is boxed
/// because `ProtocolError` is large and this sits on a hot `Ok` path.
///
/// [`ProtocolError`]: dash_sdk::dpp::ProtocolError
pub fn shielded_fee_for_actions(
    num_actions: usize,
    platform_version: &PlatformVersion,
) -> Result<u64, Box<dash_sdk::dpp::ProtocolError>> {
    use dash_sdk::dpp::shielded::compute_minimum_shielded_fee;
    compute_minimum_shielded_fee(num_actions, platform_version).map_err(Box::new)
}

/// Fee headroom (credits) to reserve from the platform balance when shielding
/// from it, so a "Max" amount still leaves enough to pay the shield's platform
/// fee. `ShieldFromBalance` needs the shield fee on top of the shielded amount
/// out of the same balance, so this must reserve the two-action shielded fee
/// (scaled by the network multiplier) — not the far smaller plain
/// platform-transfer estimate. Falls back to `0` if the active protocol version
/// has no shielded-fee formula (the backend re-validates before dispatch).
pub fn shield_from_balance_fee_headroom(
    platform_version: &PlatformVersion,
    fee_multiplier_permille: u64,
) -> u64 {
    let base_fee = shielded_fee_for_actions(2, platform_version).unwrap_or(0);
    let multiplier = fee_multiplier_permille.max(1000);
    base_fee.saturating_mul(multiplier) / 1000
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
    fn test_identity_topup_actual_fee_uses_balance_delta_when_consistent() {
        let estimator = PlatformFeeEstimator::new();
        // 500_000 duffs → 500_000_000 credits minted; a real top-up loses some
        // to the processing fee, so the balance gains slightly less.
        let amount_duffs = 500_000u64;
        let balance_before = 1_000_000_000u64;
        let processing_fee = 3_000_000u64;
        let balance_after = balance_before + amount_duffs * CREDITS_PER_DUFF - processing_fee;
        assert_eq!(
            estimator.resolve_identity_topup_actual_fee(
                amount_duffs,
                balance_before,
                balance_after,
            ),
            processing_fee,
            "a consistent balance delta must report the real processing fee"
        );
    }

    #[test]
    fn test_identity_topup_actual_fee_falls_back_to_estimate_on_stale_balance() {
        let estimator = PlatformFeeEstimator::new();
        // Stale (too-low) `balance_before` — e.g. after a backend reload — makes
        // the apparent increase exceed the minted credits, so the naive delta
        // collapses to zero. The helper must fall back to the estimate instead.
        let amount_duffs = 500_000u64;
        let stale_balance_before = 0u64;
        let balance_after = 9_999_999_999u64; // far more than the lock could mint
        let resolved = estimator.resolve_identity_topup_actual_fee(
            amount_duffs,
            stale_balance_before,
            balance_after,
        );
        assert_ne!(resolved, 0, "a top-up must never report a zero fee");
        assert_eq!(
            resolved,
            estimator.estimate_identity_topup(),
            "the stale-balance fallback must be the deterministic estimate"
        );
    }

    /// A stale-HIGH `balance_before` must fall back to the estimate.
    ///
    /// If the cached balance is *higher* than the post-top-up balance (e.g.
    /// because it was read before a spend cleared on-chain), then
    /// `balance_after.saturating_sub(balance_before)` underflows to 0 and
    /// `delta_fee` equals the full minted amount — not a fee, just noise.
    /// The helper must detect this invariant violation and return the estimate.
    #[test]
    fn test_identity_topup_actual_fee_falls_back_to_estimate_on_stale_high_balance() {
        let estimator = PlatformFeeEstimator::new();
        let amount_duffs = 5_000_000u64; // 5M duffs → 5_000_000_000 credits minted
        let expected_credits = amount_duffs * CREDITS_PER_DUFF;
        // balance_before is stale-HIGH: the cached balance is higher than
        // balance_after, so balance_increase saturates to 0 and delta_fee would
        // equal the full minted amount without the guard.
        let stale_balance_before = 10_000_000_000u64;
        let balance_after = 5_000_000_000u64; // lower than before (stale-HIGH)
        assert!(
            balance_after < stale_balance_before,
            "pre-condition: stale-HIGH scenario"
        );
        let resolved = estimator.resolve_identity_topup_actual_fee(
            amount_duffs,
            stale_balance_before,
            balance_after,
        );
        assert_ne!(
            resolved, expected_credits,
            "stale-HIGH must not report the full minted amount as the fee"
        );
        assert_eq!(
            resolved,
            estimator.estimate_identity_topup(),
            "stale-HIGH must fall back to the deterministic estimate"
        );
    }

    /// A *partial*-stale `balance_before` produces a delta that is non-zero and
    /// below the minted amount — so it slips past the two boundary checks — yet
    /// is grossly inflated relative to the real fee. The plausibility cap against
    /// the deterministic estimate must catch it and fall back to the estimate.
    #[test]
    fn test_identity_topup_actual_fee_rejects_partial_stale_inflated_delta() {
        let estimator = PlatformFeeEstimator::new();
        let amount_duffs = 5_000_000u64; // 5M duffs → 5_000_000_000 credits minted
        let expected_credits = amount_duffs * CREDITS_PER_DUFF;

        // Truth: a ~3,000,000-credit processing fee on a large prior balance.
        let true_before = 1_000_000_000u64;
        let real_fee = 3_000_000u64;
        let balance_after = true_before + expected_credits - real_fee; // freshly read

        // `balance_before` is PARTIAL-stale-HIGH: higher than truth by 3 billion,
        // but not high enough to saturate the increase to zero. The naive delta is
        // positive and below the mint, so the boundary checks alone accept it.
        let stale_before = 4_000_000_000u64;
        let naive_increase = balance_after - stale_before;
        let naive_delta = expected_credits - naive_increase;
        assert!(
            naive_delta > 0 && naive_delta < expected_credits,
            "pre-condition: the inflated delta slips past both boundary checks"
        );
        assert!(
            naive_delta > estimator.estimate_identity_topup() * 2,
            "pre-condition: the inflated delta is grossly above the estimate"
        );

        let resolved =
            estimator.resolve_identity_topup_actual_fee(amount_duffs, stale_before, balance_after);
        assert_eq!(
            resolved,
            estimator.estimate_identity_topup(),
            "a partial-stale inflated delta must fall back to the deterministic estimate"
        );
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
        assert_eq!(format_credits_as_dash(fee), "0.000135 DASH");
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
        let fee = estimator.estimate_contract_create_detailed(ContractComponents {
            contract_bytes: 500,
            document_type_count: 1,
            non_unique_index_count: 1,
            unique_index_count: 0,
            contested_index_count: 0,
            has_token: true,
            has_perpetual_distribution: false,
            has_pre_programmed_distribution: false,
            search_keyword_count: 0,
        });
        // Base: 0.1 DASH + Document type: 0.02 DASH + Index: 0.01 DASH + Token: 0.1 DASH
        // = 0.23 DASH + storage fees
        let expected_registration = 10_000_000_000 + 2_000_000_000 + 1_000_000_000 + 10_000_000_000;
        assert!(fee >= expected_registration);
    }

    #[test]
    fn test_format_credits() {
        // 1 DASH = 100,000,000,000 credits
        assert_eq!(format_credits_as_dash(100_000_000_000), "1 DASH");
        assert_eq!(format_credits_as_dash(100_000_000), "0.001 DASH");
        assert_eq!(format_credits_as_dash(100_000), "0.000001 DASH");
    }

    #[test]
    fn test_core_l1_send_fee_matches_builder_size_model() {
        // 1 input, 1 output (Max send: all funds to one recipient, no change).
        // Upstream size = 10 + 148 + 34 = 192 bytes at 1 duff/byte = 192 duffs.
        // With the 15% margin: 192 + floor(192 * 15 / 100) = 192 + 28 = 220.
        assert_eq!(estimate_core_l1_send_fee_duffs(1, 1), 220);

        // 2 inputs, 1 output: 10 + 296 + 34 = 340 bytes → 340 + 51 = 391.
        assert_eq!(estimate_core_l1_send_fee_duffs(2, 1), 391);

        // Fee grows with input count.
        assert!(estimate_core_l1_send_fee_duffs(5, 1) > estimate_core_l1_send_fee_duffs(1, 1));
    }

    #[test]
    fn test_core_l1_send_fee_clamps_to_minimum_one() {
        // Zero inputs/outputs are clamped to 1 each — never a zero-byte tx.
        assert_eq!(
            estimate_core_l1_send_fee_duffs(0, 0),
            estimate_core_l1_send_fee_duffs(1, 1)
        );
    }

    #[test]
    fn test_core_l1_send_fee_covers_actual_builder_fee() {
        // The estimate must be >= the raw size-based fee the builder charges,
        // so reserving it always leaves enough for the real fee.
        for inputs in 1..=10 {
            let raw_size = 10 + inputs as u64 * 148 + 34; // 1 output, no change
            let estimate = estimate_core_l1_send_fee_duffs(inputs, 1);
            assert!(
                estimate >= raw_size,
                "estimate {estimate} must cover raw fee {raw_size} for {inputs} inputs"
            );
        }
    }

    #[test]
    fn test_core_max_send_amount_subtracts_fee() {
        // Balance well above the fee: spendable = balance - fee.
        let balance = 1_000_000_u64;
        let fee = estimate_core_l1_send_fee_duffs(1, 1);
        assert_eq!(
            core_max_send_amount_duffs(balance, 1, 1),
            Some(balance - fee)
        );
    }

    #[test]
    fn test_core_max_send_amount_edge_balance_at_or_below_fee() {
        let fee = estimate_core_l1_send_fee_duffs(1, 1);

        // Balance exactly equal to the fee: nothing left to send.
        assert_eq!(core_max_send_amount_duffs(fee, 1, 1), None);
        // Balance below the fee: nothing left to send.
        assert_eq!(core_max_send_amount_duffs(fee - 1, 1, 1), None);
        // Zero balance: nothing left to send.
        assert_eq!(core_max_send_amount_duffs(0, 1, 1), None);

        // One duff above the fee: exactly one spendable duff.
        assert_eq!(core_max_send_amount_duffs(fee + 1, 1, 1), Some(1));
    }

    #[test]
    fn test_core_max_send_reserve_complements_send_amount() {
        // Reserve + send amount must reconstitute the spendable balance, and the
        // reserve equals the estimated fee whenever a Max exists.
        let spendable = 1_000_000_u64;
        let fee = estimate_core_l1_send_fee_duffs(3, 1);
        let send = core_max_send_amount_duffs(spendable, 3, 1).expect("covers fee");
        let reserve = core_max_send_reserve_duffs(spendable, 3, 1).expect("covers fee");
        assert_eq!(send + reserve, spendable);
        assert_eq!(reserve, fee);
    }

    #[test]
    fn test_core_max_send_reserve_none_when_balance_below_fee() {
        let fee = estimate_core_l1_send_fee_duffs(1, 1);
        // In lockstep with core_max_send_amount_duffs: no Max → no reserve.
        assert_eq!(core_max_send_reserve_duffs(fee, 1, 1), None);
        assert_eq!(core_max_send_reserve_duffs(0, 1, 1), None);
        assert_eq!(core_max_send_reserve_duffs(fee + 1, 1, 1), Some(fee));
    }

    #[test]
    fn shield_from_balance_headroom_reserves_shielded_fee_not_transfer_fee() {
        let platform_version = PlatformVersion::latest();
        let base_fee = shielded_fee_for_actions(2, platform_version).expect("known version");

        // At the minimum (1000‰) multiplier the headroom equals the base fee.
        let headroom = shield_from_balance_fee_headroom(platform_version, 1000);
        assert_eq!(headroom, base_fee);

        // It must reserve the full shielded fee (>50M), an order of magnitude
        // above the plain platform-transfer estimate — under-reserving here is
        // what got a Max shield-from-platform rejected upstream.
        assert!(
            headroom > 50_000_000,
            "shield-from-balance headroom must reserve the shielded fee: {headroom}"
        );
        assert!(headroom > PlatformFeeEstimator::new().estimate_credit_transfer());

        // Headroom scales with the multiplier and a sub-1000 multiplier is
        // clamped up to 1000 so we never under-reserve.
        assert_eq!(
            shield_from_balance_fee_headroom(platform_version, 500),
            base_fee
        );
        assert_eq!(
            shield_from_balance_fee_headroom(platform_version, 2000),
            base_fee.saturating_mul(2000) / 1000
        );
    }

    #[test]
    fn test_shielded_fee_for_actions() {
        let platform_version = PlatformVersion::latest();

        let fee_2 = shielded_fee_for_actions(2, platform_version).expect("known version");
        let fee_3 = shielded_fee_for_actions(3, platform_version).expect("known version");
        let fee_5 = shielded_fee_for_actions(5, platform_version).expect("known version");
        let fee_10 = shielded_fee_for_actions(10, platform_version).expect("known version");

        // Fees should be positive and increase with action count
        assert!(fee_2 > 0, "fee for 2 actions should be positive");
        assert!(fee_3 > fee_2, "fee for 3 actions should exceed fee for 2");
        assert!(fee_5 > fee_3, "fee for 5 actions should exceed fee for 3");
        assert!(fee_10 > fee_5, "fee for 10 actions should exceed fee for 5");

        // Sanity bounds: fee for 2 actions should be in a reasonable range
        assert!(
            fee_2 > 50_000_000,
            "fee for 2 actions should be at least 50M credits"
        );
        assert!(
            fee_2 < 1_000_000_000,
            "fee for 2 actions should be under 1B credits"
        );

        // Fee growth should be roughly linear (per-action cost is constant)
        let per_action_cost_low = (fee_5 - fee_2) / 3;
        let per_action_cost_high = (fee_10 - fee_5) / 5;
        let ratio = per_action_cost_low as f64 / per_action_cost_high as f64;
        assert!(
            (0.8..=1.2).contains(&ratio),
            "per-action cost should be roughly constant, got ratio {ratio}"
        );
    }

    /// A distinct P2PKH platform address for the given seed byte.
    fn pa(byte: u8) -> PlatformAddress {
        PlatformAddress::P2pkh([byte; 20])
    }

    /// A placeholder Core address; the allocation logic passes it through
    /// untouched, so any valid address stands in.
    fn any_core_address() -> Address {
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::dashcore::PublicKey;
        use dash_sdk::dpp::dashcore::secp256k1::{
            PublicKey as SecpPublicKey, Secp256k1, SecretKey,
        };
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1u8; 32]).expect("valid secret key");
        let pubkey = PublicKey::from_slice(&SecpPublicKey::from_secret_key(&secp, &sk).serialize())
            .expect("valid pubkey");
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    fn addrs(balances: &[(u8, u64)]) -> Vec<(PlatformAddress, Address, u64)> {
        let core = any_core_address();
        balances
            .iter()
            .map(|(byte, balance)| (pa(*byte), core.clone(), *balance))
            .collect()
    }

    #[test]
    fn allocate_covers_amount_from_single_address() {
        let addresses = addrs(&[(1, 1000)]);
        let result = allocate_platform_addresses_with_fee(&addresses, 500, None, |_| 100);

        assert_eq!(
            result.shortfall, 0,
            "fully funded transfer has no shortfall"
        );
        assert_eq!(result.estimated_fee, 100);
        assert_eq!(result.inputs.get(&pa(1)).copied(), Some(500));
        assert_eq!(result.fee_payer_index, 0);
    }

    #[test]
    fn allocate_converges_when_fee_depends_on_input_count() {
        // Fee grows with input count, so the allocation loop must iterate until
        // the fee estimate stabilizes rather than under-funding on the first pass.
        let addresses = addrs(&[(1, 300), (2, 300)]);
        let result = allocate_platform_addresses_with_fee(&addresses, 500, None, |inputs| {
            inputs.len() as u64 * 10
        });

        assert_eq!(result.estimated_fee, 20, "fee converged for two inputs");
        assert_eq!(result.inputs.len(), 2);
        assert_eq!(result.inputs.values().sum::<u64>(), 500);
        assert_eq!(result.shortfall, 0);
    }

    #[test]
    fn allocate_reports_shortfall_when_underfunded() {
        let addresses = addrs(&[(1, 100)]);
        let result = allocate_platform_addresses_with_fee(&addresses, 500, None, |_| 50);

        // 100 balance, 50 reserved for fee → only 50 allocatable against a 500 ask.
        assert_eq!(result.estimated_fee, 50);
        assert_eq!(result.inputs.get(&pa(1)).copied(), Some(50));
        assert_eq!(result.shortfall, 450);
    }

    #[test]
    fn allocate_picks_highest_balance_as_fee_payer_and_excludes_destination() {
        let addresses = addrs(&[(1, 100), (2, 1000), (9, 5000)]);
        let destination = pa(9);
        let result =
            allocate_platform_addresses_with_fee(&addresses, 200, Some(&destination), |_| 30);

        assert!(
            !result.inputs.contains_key(&destination),
            "destination must never be used as an input",
        );
        // Highest remaining balance (pa(2)) sorts first and pays the fee.
        assert_eq!(
            result.sorted_addresses.first().map(|(a, _, _)| *a),
            Some(pa(2))
        );
        assert_eq!(result.inputs.get(&pa(2)).copied(), Some(200));
        assert_eq!(result.shortfall, 0);
        let fee_payer_key = result
            .inputs
            .keys()
            .nth(result.fee_payer_index as usize)
            .copied();
        assert_eq!(
            fee_payer_key,
            Some(pa(2)),
            "fee_payer_index locates the fee payer"
        );
    }

    #[test]
    fn allocate_with_no_addresses_reports_full_shortfall() {
        let result = allocate_platform_addresses_with_fee(&[], 500, None, |_| 10);

        assert!(result.inputs.is_empty());
        assert!(result.sorted_addresses.is_empty());
        assert_eq!(result.shortfall, 500);
        assert_eq!(result.estimated_fee, 10);
    }

    #[test]
    fn allocate_with_estimator_uses_worst_case_platform_fee() {
        let estimator = PlatformFeeEstimator::new();
        let addresses = addrs(&[(1, 10_000_000_000)]);
        let result = allocate_platform_addresses(&estimator, &addresses, 1_000_000, None);

        assert_eq!(result.estimated_fee, estimate_platform_fee(&estimator, 1));
        assert_eq!(result.shortfall, 0);
        assert_eq!(result.fee_payer_index, 0);
        assert_eq!(result.inputs.get(&pa(1)).copied(), Some(1_000_000));
    }
}
