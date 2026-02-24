//! Fee estimation utilities for Dash transactions.
//!
//! ## Core (L1) transaction fees
//!
//! Size-based fee estimation for P2PKH and asset-lock transactions broadcast to
//! the Dash Core network.  Provides [`estimate_p2pkh_tx_size`],
//! [`estimate_asset_lock_tx_size`], [`calculate_relay_fee`], and the iterative
//! [`calculate_asset_lock_fee`] helper used by the wallet.
//!
//! ## Platform (L2) state-transition fees
//!
//! Fee estimation for various state transition types using the fee structure
//! from the platform version.
//!
//! Fee calculation is based on:
//! - Storage fees: Bytes stored × storage_disk_usage_credit_per_byte (27,000)
//! - Processing fees: Bytes processed × storage_processing_credit_per_byte (400)
//! - Seek costs: Number of tree operations × storage_seek_cost (2,000)
//!
//! Note: These are estimates. Actual fees depend on exact storage operations
//! performed by Platform. For accurate fees, use Platform's EstimateStateTransitionFee
//! endpoint (when available).

use dash_sdk::dpp::address_funds::{AddressFundsFeeStrategyStep, PlatformAddress};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeLevel;
use dash_sdk::dpp::prelude::{AddressNonce, AssetLockProof, Identifier};
use dash_sdk::dpp::state_transition::StateTransitionEstimatedFeeValidation;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::AddressCreditWithdrawalTransition;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::v0::AddressCreditWithdrawalTransitionV0;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::AddressFundingFromAssetLockTransition;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::v0::AddressFundingFromAssetLockTransitionV0;
use dash_sdk::dpp::state_transition::address_funds_transfer_transition::AddressFundsTransferTransition;
use dash_sdk::dpp::state_transition::address_funds_transfer_transition::v0::AddressFundsTransferTransitionV0;
use dash_sdk::dpp::state_transition::identity_create_from_addresses_transition::IdentityCreateFromAddressesTransition;
use dash_sdk::dpp::state_transition::identity_create_from_addresses_transition::v0::IdentityCreateFromAddressesTransitionV0;
use dash_sdk::dpp::state_transition::identity_topup_from_addresses_transition::IdentityTopUpFromAddressesTransition;
use dash_sdk::dpp::state_transition::identity_topup_from_addresses_transition::v0::IdentityTopUpFromAddressesTransitionV0;
use dash_sdk::dpp::state_transition::public_key_in_creation::IdentityPublicKeyInCreation;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::Pooling;
use std::collections::BTreeMap;

// ── Core (L1) transaction fee estimation ────────────────────────────────────

/// Dust threshold for P2PKH outputs (duffs). Outputs below this are
/// rejected by the network as non-standard.
pub const DUST_THRESHOLD: u64 = 546;

/// Minimum fee for asset lock transactions (duffs).
pub const MIN_ASSET_LOCK_FEE: u64 = 3_000;

/// Size of the asset-lock special transaction payload (bytes).
const ASSET_LOCK_PAYLOAD_SIZE: usize = 60;

/// Estimate varint encoding size.
fn varint_size(value: usize) -> usize {
    match value {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Estimate P2PKH transaction size in bytes.
///
/// Layout: 8-byte header (version + lock_time) + varint input/output counts
/// + 148 bytes per P2PKH input + 34 bytes per output.
pub fn estimate_p2pkh_tx_size(inputs: usize, outputs: usize) -> usize {
    8 + varint_size(inputs) + varint_size(outputs) + inputs * 148 + outputs * 34
}

/// Estimate asset-lock transaction size (P2PKH inputs + special payload).
pub fn estimate_asset_lock_tx_size(inputs: usize, outputs: usize) -> usize {
    // Asset-lock txs use a 10-byte header variant (version/type/lock_time)
    // instead of the standard 8-byte P2PKH header.
    10 + varint_size(inputs)
        + varint_size(outputs)
        + inputs * 148
        + outputs * 34
        + ASSET_LOCK_PAYLOAD_SIZE
}

/// Calculate the relay fee for a given transaction size using the SDK's
/// normal fee rate.
pub fn calculate_relay_fee(tx_size: usize) -> u64 {
    FeeLevel::Normal.fee_rate().calculate_fee(tx_size)
}

/// Result of asset lock fee calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetLockFeeResult {
    /// Transaction fee in duffs.
    pub fee: u64,
    /// Actual amount for the asset lock output (may differ from requested if
    /// `allow_take_fee_from_amount` is true).
    pub actual_amount: u64,
    /// Change to return, or `None` if no change output is needed.
    pub change: Option<u64>,
}

/// Calculate fee, actual amount, and change for an asset lock transaction.
///
/// Uses an iterative approach: starts assuming a change output exists,
/// then recomputes if the change disappears under the real fee.
///
/// # Errors
///
/// Returns an error string if there are insufficient funds.
pub fn calculate_asset_lock_fee(
    total_input_value: u64,
    requested_amount: u64,
    num_inputs: usize,
    allow_take_fee_from_amount: bool,
) -> Result<AssetLockFeeResult, String> {
    // First pass: assume 2 outputs (1 burn + 1 change).
    let fee_with_change = std::cmp::max(
        MIN_ASSET_LOCK_FEE,
        estimate_asset_lock_tx_size(num_inputs, 2) as u64,
    );

    let required_with_change = requested_amount
        .checked_add(fee_with_change)
        .ok_or("Overflow computing required amount + fee")?;
    let tentative_change = total_input_value.checked_sub(required_with_change);

    // If change exceeds dust threshold, include it as an output.
    if let Some(change) = tentative_change
        && change >= DUST_THRESHOLD
    {
        return Ok(AssetLockFeeResult {
            fee: fee_with_change,
            actual_amount: requested_amount,
            change: Some(change),
        });
    }

    // Change is zero or negative under the 2-output fee.
    // Recompute with 1 output (no change).
    let fee_no_change = std::cmp::max(
        MIN_ASSET_LOCK_FEE,
        estimate_asset_lock_tx_size(num_inputs, 1) as u64,
    );

    let required_no_change = requested_amount
        .checked_add(fee_no_change)
        .ok_or("Overflow computing required amount + fee")?;

    if total_input_value >= required_no_change {
        // Enough funds without a change output. Any leftover becomes
        // additional fee (it is too small for a viable change output).
        return Ok(AssetLockFeeResult {
            fee: total_input_value - requested_amount,
            actual_amount: requested_amount,
            change: None,
        });
    }

    // Insufficient for the requested amount at the 1-output fee rate.
    if allow_take_fee_from_amount {
        let adjusted = total_input_value.saturating_sub(fee_no_change);
        if adjusted == 0 {
            return Err("Insufficient funds for transaction fee".to_string());
        }
        Ok(AssetLockFeeResult {
            fee: fee_no_change,
            actual_amount: adjusted,
            change: None,
        })
    } else {
        Err(format!(
            "Insufficient funds: need {} + {} fee, have {}",
            requested_amount, fee_no_change, total_input_value
        ))
    }
}

// ── Platform (L2) state-transition fee estimation ───────────────────────────

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
    fn calculate_storage_fee(&self, bytes: usize) -> u64 {
        (bytes as u64).saturating_mul(self.storage_fees.storage_disk_usage_credit_per_byte)
    }

    /// Calculate processing fee for writing data.
    fn calculate_processing_fee(&self, bytes: usize) -> u64 {
        (bytes as u64).saturating_mul(self.storage_fees.storage_processing_credit_per_byte)
    }

    /// Calculate fee for tree seek operations.
    /// Contracts and documents require multiple seeks for tree traversal.
    fn calculate_seek_fee(&self, seek_count: usize) -> u64 {
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

    /// Legacy fee estimate for identity creation from addresses.
    ///
    /// TODO(dashpay/platform#3040): Remove once transition-based estimation is
    /// accurate.  Use [`Self::estimate_identity_create_from_addresses_fee`] instead.
    fn estimate_identity_create_from_addresses(
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

    /// Legacy fee estimate for identity top-up from platform addresses.
    ///
    /// TODO(dashpay/platform#3040): Remove once transition-based estimation is
    /// accurate.  Use [`Self::estimate_identity_topup_from_addresses_fee`] instead.
    fn estimate_identity_topup_from_addresses(&self, input_count: usize) -> u64 {
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
        total.saturating_add(total / 5)
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
    fn estimate_document_create_with_size(&self, document_bytes: usize) -> u64 {
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
    fn estimate_contract_create_with_size(&self, contract_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 20;
        let base_fee = self
            .registration_fees
            .base_contract_registration_fee
            .saturating_add(self.min_fees.contract_create)
            .saturating_add(self.calculate_storage_based_fee(contract_bytes, ESTIMATED_SEEKS));
        self.apply_multiplier(base_fee)
    }

    /// Estimate fee for data contract creation (uses base registration fee only).
    /// For more accurate estimates, use estimate_contract_create_with_size.
    pub fn estimate_contract_create_base(&self) -> u64 {
        // Base registration fee (0.1 DASH) + minimal storage estimate
        self.estimate_contract_create_with_size(500)
    }

    /// Estimate fee for data contract update with known size of changes.
    fn estimate_contract_update_with_size(&self, update_bytes: usize) -> u64 {
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

    // ── Unified fee estimation (max of legacy + transition-based) ────────
    //
    // Each method below computes fees using both the legacy heuristic and a
    // constructed state transition via `calculate_min_required_fee()`.  The
    // transition-based approach is more accurate but currently underestimates
    // (dashpay/platform#3040).  We return `max(legacy, transition)` until
    // that is fixed, at which point the legacy paths can be removed.

    /// Estimate fee from an already-constructed state transition.
    /// Returns 0 if estimation fails.
    fn estimate_fee_from_transition<T: StateTransitionEstimatedFeeValidation>(
        transition: &T,
        platform_version: &PlatformVersion,
    ) -> u64 {
        transition
            .calculate_min_required_fee(platform_version)
            .unwrap_or(0)
    }

    /// Legacy platform transfer/withdrawal fee estimate (base + storage + 20%
    /// buffer).  Mirrors the old `estimate_platform_fee()` free function from
    /// `send_screen.rs`.
    ///
    /// TODO(dashpay/platform#3040): Remove once transition-based estimation is
    /// accurate and this fallback is no longer needed.
    fn legacy_platform_transfer_fee(&self, input_count: usize) -> u64 {
        /// Estimated serialized bytes per input (address + signature/witness data).
        const ESTIMATED_BYTES_PER_INPUT: usize = 225;

        let inputs = input_count.max(1);

        // Base fee from Platform's min fee structure
        let base_fee = self.estimate_address_funds_transfer(inputs, 1);

        // Add storage fees for serialized input bytes
        let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT;
        let storage_fee = self.estimate_storage_based_fee(estimated_bytes, inputs);

        // Total with 20% safety buffer
        let total = base_fee.saturating_add(storage_fee);
        total.saturating_add(total / 5)
    }

    /// Estimate fee for a Platform → Core withdrawal.
    ///
    /// Computes `max(legacy, transition)` internally.
    pub fn estimate_platform_to_core_fee(
        &self,
        platform_version: &PlatformVersion,
        inputs: &BTreeMap<PlatformAddress, u64>,
        output_script: &CoreScript,
    ) -> u64 {
        let legacy = self.legacy_platform_transfer_fee(inputs.len());

        let inputs_with_nonce: BTreeMap<PlatformAddress, (AddressNonce, Credits)> = inputs
            .iter()
            .map(|(addr, amount)| (*addr, (0, *amount)))
            .collect();

        let transition =
            AddressCreditWithdrawalTransition::V0(AddressCreditWithdrawalTransitionV0 {
                inputs: inputs_with_nonce,
                output: None,
                fee_strategy: vec![AddressFundsFeeStrategyStep::DeductFromInput(0)],
                core_fee_per_byte: 1,
                pooling: Pooling::Never,
                output_script: output_script.clone(),
                user_fee_increase: 0,
                input_witnesses: Vec::new(),
            });

        let transition_fee = Self::estimate_fee_from_transition(&transition, platform_version);

        // TODO(dashpay/platform#3040): Remove legacy estimation once transition-based is accurate.
        legacy.max(transition_fee)
    }

    /// Estimate fee for a Platform → Platform transfer.
    ///
    /// Computes `max(legacy, transition)` internally.
    pub fn estimate_platform_to_platform_fee(
        &self,
        platform_version: &PlatformVersion,
        inputs: &BTreeMap<PlatformAddress, u64>,
        destination: &PlatformAddress,
    ) -> u64 {
        let legacy = self.legacy_platform_transfer_fee(inputs.len());

        let inputs_with_nonce: BTreeMap<PlatformAddress, (AddressNonce, Credits)> = inputs
            .iter()
            .map(|(addr, amount)| (*addr, (0, *amount)))
            .collect();

        let mut outputs = BTreeMap::new();
        outputs.insert(*destination, 1);

        let transition = AddressFundsTransferTransition::V0(AddressFundsTransferTransitionV0 {
            inputs: inputs_with_nonce,
            outputs,
            fee_strategy: vec![AddressFundsFeeStrategyStep::DeductFromInput(0)],
            user_fee_increase: 0,
            input_witnesses: Vec::new(),
        });

        let transition_fee = Self::estimate_fee_from_transition(&transition, platform_version);

        // TODO(dashpay/platform#3040): Remove legacy estimation once transition-based is accurate.
        legacy.max(transition_fee)
    }

    /// Estimate fee for a Core → Platform address funding (via asset lock).
    ///
    /// Computes `max(legacy, transition)` internally.
    pub fn estimate_core_to_platform_fee(
        &self,
        platform_version: &PlatformVersion,
        destination: &PlatformAddress,
    ) -> u64 {
        // Legacy estimate
        let legacy = self.estimate_address_funding_from_asset_lock_duffs(1);

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

        let transition_fee = Self::estimate_fee_from_transition(&transition, platform_version);

        // TODO(dashpay/platform#3040): Remove legacy estimation once transition-based is accurate.
        legacy.max(transition_fee)
    }

    /// Estimate fee for identity creation from Platform addresses.
    ///
    /// Computes `max(legacy, transition)` internally.
    pub fn estimate_identity_create_from_addresses_fee(
        &self,
        platform_version: &PlatformVersion,
        inputs: &BTreeMap<PlatformAddress, (AddressNonce, Credits)>,
        output: Option<(PlatformAddress, Credits)>,
        key_count: usize,
    ) -> u64 {
        let legacy =
            self.estimate_identity_create_from_addresses(inputs.len(), output.is_some(), key_count);

        let public_keys = match IdentityPublicKeyInCreation::default_versioned(platform_version) {
            Ok(key) => std::iter::repeat_n(key, key_count).collect(),
            Err(_) => Vec::new(),
        };

        let transition =
            IdentityCreateFromAddressesTransition::V0(IdentityCreateFromAddressesTransitionV0 {
                public_keys,
                inputs: inputs.clone(),
                output,
                fee_strategy: vec![AddressFundsFeeStrategyStep::DeductFromInput(0)],
                user_fee_increase: 0,
                input_witnesses: Vec::new(),
            });

        let transition_fee = Self::estimate_fee_from_transition(&transition, platform_version);

        // TODO(dashpay/platform#3040): Remove legacy estimation once transition-based is accurate.
        legacy.max(transition_fee)
    }

    /// Estimate fee for identity top-up from Platform addresses.
    ///
    /// Computes `max(legacy, transition)` internally.
    pub fn estimate_identity_topup_from_addresses_fee(
        &self,
        platform_version: &PlatformVersion,
        inputs: &BTreeMap<PlatformAddress, (AddressNonce, Credits)>,
        output: Option<(PlatformAddress, Credits)>,
        identity_id: Identifier,
    ) -> u64 {
        let legacy = self.estimate_identity_topup_from_addresses(inputs.len());

        let transition =
            IdentityTopUpFromAddressesTransition::V0(IdentityTopUpFromAddressesTransitionV0 {
                inputs: inputs.clone(),
                output,
                identity_id,
                fee_strategy: vec![AddressFundsFeeStrategyStep::DeductFromInput(0)],
                user_fee_increase: 0,
                input_witnesses: Vec::new(),
            });

        let transition_fee = Self::estimate_fee_from_transition(&transition, platform_version);

        // TODO(dashpay/platform#3040): Remove legacy estimation once transition-based is accurate.
        legacy.max(transition_fee)
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

    // ── Core (L1) fee tests ─────────────────────────────────────────────

    #[test]
    fn p2pkh_tx_size_single_input() {
        // 1 input, 2 outputs: 8 + 1 + 1 + 148 + 68 = 226
        assert_eq!(estimate_p2pkh_tx_size(1, 2), 226);
    }

    #[test]
    fn p2pkh_tx_size_many_inputs() {
        // 10 inputs, 2 outputs: 8 + 1 + 1 + 1480 + 68 = 1558
        assert_eq!(estimate_p2pkh_tx_size(10, 2), 1558);
    }

    #[test]
    fn asset_lock_tx_size_single_input() {
        // 1 input, 2 outputs: 10 + 1 + 1 + 148 + 68 + 60 = 288
        assert_eq!(estimate_asset_lock_tx_size(1, 2), 288);
    }

    #[test]
    fn relay_fee_exceeds_1000_for_large_tx() {
        // Regression: the bug was a hardcoded 1000-duffs fee for any tx size.
        // 10 inputs × 2 outputs → ~1558 bytes → relay fee must exceed 1000.
        let size = estimate_p2pkh_tx_size(10, 2);
        let fee = calculate_relay_fee(size);
        assert!(
            fee > 1_000,
            "relay fee for 10-input tx should exceed 1000 duffs, got {fee}"
        );
    }

    #[test]
    fn asset_lock_fee_single_input_minimum() {
        // 1 input, amount=10000, total=50000.
        // With 2 outputs: size = 288 -> fee = max(3000, 288) = 3000.
        // Change = 50000 - 10000 - 3000 = 37000.
        let result = calculate_asset_lock_fee(50_000, 10_000, 1, false).expect("should succeed");
        assert_eq!(result.fee, 3_000);
        assert_eq!(result.actual_amount, 10_000);
        assert_eq!(result.change, Some(37_000));
    }

    #[test]
    fn asset_lock_fee_scales_with_inputs() {
        // 21 inputs, amount=50000, total=200000.
        // With 2 outputs: size = 10 + varint(21)+varint(2) + 21*148 + 2*34 + 60 = 3248.
        // fee = max(3000, 3248) = 3248.
        let result = calculate_asset_lock_fee(200_000, 50_000, 21, false).expect("should succeed");
        assert!(
            result.fee > MIN_ASSET_LOCK_FEE,
            "fee should exceed minimum for many inputs"
        );
        assert_eq!(result.fee, 3_248);
        assert_eq!(result.actual_amount, 50_000);
        assert_eq!(result.change, Some(146_752));
    }

    #[test]
    fn asset_lock_fee_exact_no_change() {
        // 1 input, amount=47000, total=50000.
        // With 2 outputs: fee = 3000. change = 50000 - 47000 - 3000 = 0 -> None.
        // Re-check with 1 output: fee = 3000 (minimum).
        // 50000 >= 47000 + 3000 -> actual fee = 50000 - 47000 = 3000.
        let result = calculate_asset_lock_fee(50_000, 47_000, 1, false).expect("should succeed");
        assert_eq!(result.actual_amount, 47_000);
        assert_eq!(result.change, None);
    }

    #[test]
    fn asset_lock_fee_take_from_amount() {
        // 1 input, amount=50000, total=50000, allow_take_fee=true.
        // Take from amount: actual = 50000 - 3000 = 47000.
        let result = calculate_asset_lock_fee(50_000, 50_000, 1, true).expect("should succeed");
        assert_eq!(result.actual_amount, 47_000);
        assert_eq!(result.change, None);
    }

    #[test]
    fn asset_lock_fee_insufficient_funds() {
        let result = calculate_asset_lock_fee(50_000, 50_000, 1, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient funds"));
    }

    #[test]
    fn asset_lock_fee_insufficient_for_fee_alone() {
        // Fee = 3000 > total 2000 -> error.
        let result = calculate_asset_lock_fee(2_000, 2_000, 1, true);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("Insufficient funds"),
            "should error when total cannot even cover the fee"
        );
    }

    #[test]
    fn asset_lock_fee_change_eliminated_by_real_fee_should_succeed() {
        // 21 inputs, amount=50000, total=53220.
        // Correct code recalculates with 1 output when change disappears.
        let result = calculate_asset_lock_fee(53_220, 50_000, 21, false);
        assert!(
            result.is_ok(),
            "should NOT fail with insufficient funds; got: {:?}",
            result
        );
        let result = result.unwrap();
        assert_eq!(result.actual_amount, 50_000);
        assert_eq!(result.change, None);
        assert_eq!(result.fee, 3_220);
    }

    #[test]
    fn asset_lock_fee_overestimate_when_change_disappears() {
        // 21 inputs, amount=50000, total=53240.
        // 1-output path succeeds; fee = 3240, no change.
        let result = calculate_asset_lock_fee(53_240, 50_000, 21, false);
        assert!(
            result.is_ok(),
            "should succeed when recalculated without change output; got: {:?}",
            result
        );
        let result = result.unwrap();
        assert_eq!(result.actual_amount, 50_000);
        assert_eq!(result.change, None);
        assert!(
            result.fee < 3_246,
            "fee should be less than the 2-output estimate of 3246, got {}",
            result.fee
        );
        assert_eq!(result.fee, 3_240);
    }

    #[test]
    fn asset_lock_fee_dust_change_absorbed_into_fee() {
        // 1 input, amount=46500, total=50000.
        // change = 500 < DUST_THRESHOLD → absorbed into fee.
        let result = calculate_asset_lock_fee(50_000, 46_500, 1, false).expect("should succeed");
        assert_eq!(result.actual_amount, 46_500);
        assert_eq!(
            result.change, None,
            "dust change should not create an output"
        );
        assert_eq!(result.fee, 3_500, "dust should be absorbed into fee");
    }

    // ── Platform (L2) fee tests ─────────────────────────────────────────

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
    fn test_format_credits() {
        // 1 DASH = 100,000,000,000 credits
        assert_eq!(format_credits_as_dash(100_000_000_000), "1.00000000 DASH");
        assert_eq!(format_credits_as_dash(100_000_000), "0.00100000 DASH");
        assert_eq!(format_credits_as_dash(100_000), "0.00000100 DASH");
    }
}
