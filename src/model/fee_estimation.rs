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
use crate::model::spend_limit::{CoreDestinationScript, CoreSendQuoteFee};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::{AddressFundsFeeStrategyStep, PlatformAddress};
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits};
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::{FeeRate, estimate_tx_size};
use dash_sdk::dpp::prelude::{AddressNonce, AssetLockProof};
use dash_sdk::dpp::state_transition::StateTransitionEstimatedFeeValidation;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::AddressCreditWithdrawalTransition;
use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::v0::AddressCreditWithdrawalTransitionV0;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::v0::AddressFundingFromAssetLockTransitionV0;
use dash_sdk::dpp::state_transition::address_funding_from_asset_lock_transition::{
    AddressFundingFromAssetLockTransition,
    calculate_address_funding_from_asset_lock_min_required_fee,
};
use dash_sdk::dpp::state_transition::address_funds_transfer_transition::AddressFundsTransferTransition;
use dash_sdk::dpp::state_transition::identity_create_from_addresses_transition::IdentityCreateFromAddressesTransition;
use dash_sdk::dpp::state_transition::identity_create_from_addresses_transition::v0::IdentityCreateFromAddressesTransitionV0;
use dash_sdk::dpp::state_transition::identity_create_transition::IdentityCreateTransition;
use dash_sdk::dpp::state_transition::identity_create_transition::v0::IdentityCreateTransitionV0;
use dash_sdk::dpp::state_transition::identity_credit_transfer_to_addresses_transition::IdentityCreditTransferToAddressesTransition;
use dash_sdk::dpp::state_transition::identity_credit_transfer_to_addresses_transition::v0::IdentityCreditTransferToAddressesTransitionV0;
use dash_sdk::dpp::state_transition::identity_credit_transfer_transition::IdentityCreditTransferTransition;
use dash_sdk::dpp::state_transition::identity_credit_withdrawal_transition::IdentityCreditWithdrawalTransition;
use dash_sdk::dpp::state_transition::identity_topup_from_addresses_transition::IdentityTopUpFromAddressesTransition;
use dash_sdk::dpp::state_transition::identity_topup_from_addresses_transition::v0::IdentityTopUpFromAddressesTransitionV0;
use dash_sdk::dpp::state_transition::identity_topup_transition::IdentityTopUpTransition;
use dash_sdk::dpp::state_transition::identity_topup_transition::v0::IdentityTopUpTransitionV0;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::public_key_in_creation::IdentityPublicKeyInCreation;
use dash_sdk::dpp::version::fee::data_contract_registration::FeeDataContractRegistrationVersion;
use dash_sdk::dpp::version::fee::state_transition_min_fees::StateTransitionMinFees;
use dash_sdk::dpp::version::fee::storage::FeeStorageVersion;

/// DET's correction to an upstream estimate: the single place where a fee is
/// raised or a Max lowered beyond what upstream computes.
///
/// Upstream functions are the source of every fee, size and limit; a
/// tolerance only ever makes their result safer — a fee up, a Max down —
/// so a correction for an estimate found to be short is one constant in
/// [`tolerance`], not an edit at each call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstimateTolerance {
    /// Headroom on top of the upstream fee, in percent of it.
    percent: u64,
    /// The smallest fee the estimate may show, in the estimate's unit.
    floor: u64,
}

impl EstimateTolerance {
    /// No correction: upstream's number as is.
    pub const NONE: Self = Self::percent(0);

    /// `percent` of the upstream fee on top of it.
    pub const fn percent(percent: u64) -> Self {
        Self { percent, floor: 0 }
    }

    /// The same, never below `floor`.
    pub const fn with_floor(self, floor: u64) -> Self {
        Self { floor, ..self }
    }

    /// `fee` with the headroom added; never lower than `fee`.
    pub const fn apply_fee(self, fee: u64) -> u64 {
        // Widened so a large fee or percentage saturates the result instead
        // of shrinking the headroom.
        let extra = (fee as u128 * self.percent as u128) / 100;
        let extra = if extra > u64::MAX as u128 {
            u64::MAX
        } else {
            extra as u64
        };
        let raised = fee.saturating_add(extra);
        if raised < self.floor {
            self.floor
        } else {
            raised
        }
    }

    /// `max` lowered by exactly the headroom [`Self::apply_fee`] adds to
    /// `fee`, so a Max and the fee shown beside it still add up to what
    /// upstream says the coins cover.
    pub const fn apply_max(self, max: u64, fee: u64) -> u64 {
        max.saturating_sub(self.apply_fee(fee).saturating_sub(fee))
    }
}

/// Every [`EstimateTolerance`] DET applies, by estimate kind.
///
/// This module is the only place DET corrects an upstream fee, size, limit or
/// Max. Any new estimator calls the upstream function and then one of these
/// constants (add one for a new kind of estimate); call sites never apply a
/// margin of their own. A constant may only make a result safer: a fee up, a
/// Max down.
pub mod tolerance {
    use super::EstimateTolerance;

    /// A pooled Core send. Upstream's pooled Max is an amount its build
    /// accepts, and the fee is priced the same way, so no headroom is needed.
    pub const CORE_SEND: EstimateTolerance = EstimateTolerance::NONE;
    /// The Core fee of an asset lock. No upstream function sizes its payload,
    /// so the plain-transaction size gets this reserve on top.
    pub const ASSET_LOCK_CORE_FEE: EstimateTolerance = EstimateTolerance::percent(15);
    /// The Core fee of shielding from the Core wallet. No upstream function
    /// sizes that transaction, so a flat floor covering Core's minimum relay
    /// fee stands in for it.
    pub const SHIELD_FROM_CORE_L1_FEE: EstimateTolerance =
        EstimateTolerance::NONE.with_floor(3_000);

    /// Upstream's minimum fee for a Platform transition, which is what the
    /// network checks the balance against.
    pub const PLATFORM_MIN_FEE: EstimateTolerance = EstimateTolerance::NONE;
    /// Upstream's shielded fee and the shield planner's fee reserve, which
    /// are what the network and the planner charge and keep back.
    pub const SHIELDED_FEE: EstimateTolerance = EstimateTolerance::NONE;
    /// DET's storage estimate for a document, token or contract write. No
    /// headroom yet; it is floored at upstream's minimum fee.
    pub const PLATFORM_STORAGE: EstimateTolerance = EstimateTolerance::NONE;
    /// Transitions spending Platform address inputs, priced by DET's storage
    /// estimate over upstream's minimum.
    pub const ADDRESS_INPUTS_FEE: EstimateTolerance = EstimateTolerance::percent(20);
    /// The balance a transfer out of an identity must keep. The fitted figures
    /// are for an ECDSA key; a BLS key adds 285,000 credits.
    pub const IDENTITY_TRANSFER: EstimateTolerance = EstimateTolerance::percent(10);
    /// A username registration. The fitted figures were read from Platform's
    /// engine on an empty network; a live network charges somewhat more
    /// processing.
    pub const USERNAME_REGISTRATION: EstimateTolerance = EstimateTolerance::percent(10);
    /// The Platform fee, in duffs, of funding Platform addresses from an asset
    /// lock: half again DET's estimate, and at least 10,000 duffs as observed.
    pub const ADDRESS_FUNDING_FROM_ASSET_LOCK: EstimateTolerance =
        EstimateTolerance::percent(50).with_floor(10_000);
}

/// Keep contested document creation within its contest's base fund — the
/// moderation fund for a moderation election, the contested document fund
/// otherwise — without auto-quoting a larger fund to join a crowded contest.
pub fn document_create_contest_fund_cap(
    document_type: dash_sdk::dpp::data_contract::document_type::DocumentTypeRef<'_>,
    document: &dash_sdk::dpp::document::Document,
    platform_version: &PlatformVersion,
) -> Result<Option<Credits>, Box<dash_sdk::dpp::ProtocolError>> {
    use dash_sdk::dpp::data_contract::document_type::methods::{
        DocumentTypeBasicMethods, DocumentTypeV0Methods,
    };
    use dash_sdk::dpp::document::DocumentV0Getters;
    let mut document = document.clone();
    document_type.regenerate_generated_properties(document.properties_mut(), platform_version)?;
    use dash_sdk::dpp::voting::vote_polls::VotePoll;
    Ok(document_type
        .contested_vote_poll_for_document(&document, platform_version)?
        .map(|VotePoll::ContestedDocumentResourceVotePoll(vote_poll)| {
            vote_poll.required_vote_resolution_fund(platform_version)
        }))
}

/// Subtract an estimated fee from a credit balance without floating-point conversion.
pub const fn max_spendable_credits(balance: u64, estimated_fee: u64) -> u64 {
    balance.saturating_sub(estimated_fee)
}
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::Pooling;
use std::collections::BTreeMap;

/// Most Platform address inputs one state transition may spend under
/// `platform_version`.
pub(crate) fn max_platform_inputs(platform_version: &PlatformVersion) -> usize {
    usize::from(platform_version.dpp.state_transitions.max_address_inputs)
}

/// Estimated serialized bytes per input (address + signature/witness data).
const ESTIMATED_BYTES_PER_INPUT: usize = 225;

// TODO(dashpay/platform#5383): the `USERNAME_*` figures are fitted to how Drive
// bills the DPNS contract; replace them with Platform's own numbers. The charge
// comes from `drive::document::cost::document_create_cost`
// (dashpay/platform#5159), which the pinned Platform revision does not have
// yet. The balance Drive wants before it accepts a document has no function
// yet; dashpay/platform#5265 asks for one.

/// Bytes Platform bills for a username's first registration document and its
/// index entry: 726, plus 428 the first time an identity writes to the name
/// contract.
const USERNAME_FIRST_DOCUMENT_BYTES: usize = 1_154;
/// Processing Platform charges for the first document on top of its storage.
const USERNAME_FIRST_DOCUMENT_PROCESSING_CREDITS: u64 = 3_500_000;
/// Balance Platform wants before it accepts the name document of an empty
/// label, in bytes at the storage rate. It reserves about twice what it then
/// charges, and more for a name that needs a community vote.
const USERNAME_NAME_RESERVED_BYTES: usize = 3_019;
const USERNAME_VOTED_NAME_RESERVED_BYTES: usize = 4_517;
/// Bytes each label character adds to the name document and its index entries.
const USERNAME_NAME_BYTES_PER_CHAR: usize = 4;
const USERNAME_VOTED_NAME_BYTES_PER_CHAR: usize = 5;

// TODO(dashpay/platform#5265): the `*_TRANSFER_RESERVE_*` figures are fitted to
// the balance Drive wants before it accepts a transfer out of an identity;
// replace them with Platform's own number once it offers a function for it.
// Their source is docs/ai-design/2026-10-13-identity-transfer-fee-reserve.

/// Balance Platform wants on an identity, above the amount, before it accepts a
/// transfer to another identity. It then charges about a third of it.
const CREDIT_TRANSFER_RESERVE_CREDITS: u64 = 8_755_720;
/// The same for a transfer to Platform addresses: a part for the transfer and
/// a part for each address that receives.
const ADDRESS_TRANSFER_RESERVE_CREDITS: u64 = 11_261_680;
const ADDRESS_TRANSFER_RESERVE_PER_ADDRESS_CREDITS: u64 = 6_176_980;

/// An upstream minimum fee, or the largest fee there is when the protocol
/// version has no formula for it — an estimate that blocks the operation
/// rather than one that offers more than the network accepts.
fn upstream_min_fee(fee: Result<Credits, ProtocolError>) -> Credits {
    fee.unwrap_or(Credits::MAX)
}

/// `count` distinct Platform addresses standing in for a transition's inputs;
/// the minimum fee counts inputs, not who they are.
fn placeholder_address_inputs(count: usize) -> BTreeMap<PlatformAddress, (AddressNonce, Credits)> {
    (0..count as u64)
        .map(|index| {
            let mut hash = [0u8; 20];
            hash[..8].copy_from_slice(&index.to_le_bytes());
            (PlatformAddress::P2pkh(hash), (0, 0))
        })
        .collect()
}

/// A Platform address output distinct from every placeholder input.
fn placeholder_address_output() -> (PlatformAddress, Credits) {
    (PlatformAddress::P2sh([0xff; 20]), 0)
}

/// Fee estimator for Platform state transitions.
///
/// Every number comes from upstream at the network's protocol version — the
/// fee tables of `platform_version.fee_version` and the minimum-fee functions
/// of each transition — scaled by the epoch fee multiplier and corrected only
/// through [`tolerance`]. Where Platform charges more than its minimum (storage
/// a transition writes) and no upstream function says how much, DET's own
/// estimate is kept, with upstream's minimum as its floor.
#[derive(Debug, Clone, Copy)]
pub struct PlatformFeeEstimator {
    platform_version: &'static PlatformVersion,
    /// Fee multiplier in permille (1000 = 1x, 2000 = 2x, etc.)
    /// This comes from the current epoch's fee_multiplier_permille()
    fee_multiplier_permille: u64,
}

impl PlatformFeeEstimator {
    /// Default fee multiplier (1x = 1000 permille)
    pub const DEFAULT_FEE_MULTIPLIER_PERMILLE: u64 = 1000;

    /// An estimator for `platform_version` at the epoch's fee multiplier.
    pub fn new(platform_version: &'static PlatformVersion, fee_multiplier_permille: u64) -> Self {
        Self {
            platform_version,
            fee_multiplier_permille,
        }
    }

    /// An estimator for the latest protocol version at the default multiplier.
    #[cfg(test)]
    pub(crate) fn latest() -> Self {
        Self::latest_with_fee_multiplier(Self::DEFAULT_FEE_MULTIPLIER_PERMILLE)
    }

    /// An estimator for the latest protocol version at `fee_multiplier_permille`.
    #[cfg(test)]
    pub(crate) fn latest_with_fee_multiplier(fee_multiplier_permille: u64) -> Self {
        Self::new(PlatformVersion::latest(), fee_multiplier_permille)
    }

    /// Most Platform address inputs one state transition may spend.
    pub(crate) fn max_platform_inputs(&self) -> usize {
        max_platform_inputs(self.platform_version)
    }

    fn min_fees(&self) -> &'static StateTransitionMinFees {
        &self.platform_version.fee_version.state_transition_min_fees
    }

    fn storage(&self) -> &'static FeeStorageVersion {
        &self.platform_version.fee_version.storage
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

    /// Upstream's minimum fee for a transition, scaled by the multiplier and
    /// corrected by [`tolerance::PLATFORM_MIN_FEE`].
    fn min_fee(&self, upstream_min: Credits) -> u64 {
        tolerance::PLATFORM_MIN_FEE.apply_fee(self.apply_multiplier(upstream_min))
    }

    /// DET's `estimate` of a charge no upstream function computes, with its
    /// `tolerance`, never below `upstream_min`, scaled by the multiplier.
    fn reserve(&self, tolerance: EstimateTolerance, estimate: u64, upstream_min: Credits) -> u64 {
        self.apply_multiplier(tolerance.apply_fee(estimate).max(upstream_min))
    }

    /// Storage fee for `bytes` of permanent storage.
    fn storage_fee(&self, bytes: usize) -> u64 {
        (bytes as u64).saturating_mul(self.storage().storage_disk_usage_credit_per_byte)
    }

    /// Processing fee for writing `bytes`.
    fn processing_fee(&self, bytes: usize) -> u64 {
        (bytes as u64).saturating_mul(self.storage().storage_processing_credit_per_byte)
    }

    /// Fee for `seek_count` tree seek operations.
    fn seek_fee(&self, seek_count: usize) -> u64 {
        (seek_count as u64).saturating_mul(self.storage().storage_seek_cost)
    }

    /// Storage, processing and seek fees for storing `bytes`, unscaled.
    fn storage_based_fee(&self, bytes: usize, estimated_seeks: usize) -> u64 {
        self.storage_fee(bytes)
            .saturating_add(self.processing_fee(bytes))
            .saturating_add(self.seek_fee(estimated_seeks))
    }

    /// Estimate total storage-based fee for storing data.
    /// Includes storage, processing, and estimated seek costs.
    /// Applies the current fee multiplier.
    pub fn estimate_storage_based_fee(&self, bytes: usize, estimated_seeks: usize) -> u64 {
        self.reserve(
            tolerance::PLATFORM_STORAGE,
            self.storage_based_fee(bytes, estimated_seeks),
            0,
        )
    }

    /// Balance an identity must keep, above the amount, to transfer credits to
    /// another identity.
    ///
    /// Platform accepts the transfer only while the balance holds a reserve
    /// above the fee it then charges, so the fee actually charged is lower.
    pub fn estimate_credit_transfer(&self) -> u64 {
        self.reserve(
            tolerance::IDENTITY_TRANSFER,
            CREDIT_TRANSFER_RESERVE_CREDITS,
            upstream_min_fee(
                IdentityCreditTransferTransition::V0(Default::default())
                    .calculate_min_required_fee(self.platform_version),
            ),
        )
    }

    /// Balance an identity must keep, above the amount, to transfer credits to
    /// `output_count` Platform addresses. As with
    /// [`Self::estimate_credit_transfer`], the fee actually charged is lower.
    pub fn estimate_credit_transfer_to_addresses(&self, output_count: usize) -> u64 {
        let recipients = placeholder_address_inputs(output_count)
            .into_keys()
            .map(|address| (address, 0))
            .collect();
        let upstream = IdentityCreditTransferToAddressesTransition::V0(
            IdentityCreditTransferToAddressesTransitionV0 {
                recipient_addresses: recipients,
                ..Default::default()
            },
        )
        .calculate_min_required_fee(self.platform_version);
        self.reserve(
            tolerance::IDENTITY_TRANSFER,
            ADDRESS_TRANSFER_RESERVE_CREDITS.saturating_add(
                ADDRESS_TRANSFER_RESERVE_PER_ADDRESS_CREDITS.saturating_mul(output_count as u64),
            ),
            upstream_min_fee(upstream),
        )
    }

    /// Estimate fee for credit withdrawal to core chain
    pub fn estimate_credit_withdrawal(&self) -> u64 {
        self.min_fee(upstream_min_fee(
            IdentityCreditWithdrawalTransition::V1(Default::default())
                .calculate_min_required_fee(self.platform_version),
        ))
    }

    /// Estimate fee for a withdrawal from one Platform address.
    pub fn estimate_address_credit_withdrawal(&self) -> u64 {
        self.min_fee(AddressCreditWithdrawalTransition::estimate_min_fee(
            1,
            false,
            self.platform_version,
        ))
    }

    /// Fee, in duffs, to allow for when funding `output_count` Platform
    /// addresses from an asset lock, never below the funding the network
    /// requires before it processes one.
    pub fn estimate_address_funding_from_asset_lock_duffs(&self, output_count: usize) -> u64 {
        let estimate_duffs = self.apply_multiplier(
            self.min_fees().credit_transfer_to_addresses.saturating_add(
                self.min_fees()
                    .address_funds_transfer_output_cost
                    .saturating_mul(output_count as u64),
            ),
        ) / CREDITS_PER_DUFF;
        let required_duffs = calculate_address_funding_from_asset_lock_min_required_fee(
            0,
            output_count,
            self.platform_version,
        )
        .div_ceil(CREDITS_PER_DUFF);
        tolerance::ADDRESS_FUNDING_FROM_ASSET_LOCK
            .apply_fee(estimate_duffs)
            .max(required_duffs)
    }

    /// Smallest amount, in duffs, a user can enter when funding a Platform
    /// address from the wallet without the network refusing the funding.
    ///
    /// The network compares the whole funding with a fee that follows the
    /// protocol version and the number of recipients. With the fee taken from
    /// the amount, the amount is the whole funding. Otherwise the funding is
    /// the amount plus [`Self::estimate_address_funding_from_asset_lock_duffs`]
    /// and is shared with a change recipient, so the amount must make up what
    /// that estimate leaves short, and still meet the smallest amount the
    /// network accepts for a named recipient.
    pub fn address_funding_min_amount_duffs(&self, fee_deduct_from_output: bool) -> u64 {
        let platform_version = self.platform_version;
        let required_duffs = |recipients| {
            calculate_address_funding_from_asset_lock_min_required_fee(
                0,
                recipients,
                platform_version,
            )
            .div_ceil(CREDITS_PER_DUFF)
        };
        if fee_deduct_from_output {
            return required_duffs(1);
        }
        let smallest_named_amount = platform_version
            .dpp
            .state_transitions
            .address_funds
            .min_output_amount
            .div_ceil(CREDITS_PER_DUFF);
        required_duffs(2)
            .saturating_sub(self.estimate_address_funding_from_asset_lock_duffs(2))
            .max(smallest_named_amount)
    }

    /// Fees (in duffs) for shielding from the Core wallet.
    ///
    /// Returns `(platform_fee_duffs, l1_tx_fee_duffs)`:
    /// - Platform fee: what the network takes from the funding before the rest
    ///   reaches the shielded pool, rounded up to whole duffs
    /// - L1 tx fee: flat estimate covering Core minimum relay fee (~3000 duffs)
    ///
    /// `platform_version` is the version the shield is sized at, which may
    /// differ from the estimator's.
    pub fn estimate_shield_from_core_fees_duffs(
        &self,
        platform_version: &PlatformVersion,
    ) -> Result<(u64, u64), Box<ProtocolError>> {
        let platform_fee_duffs =
            shield_from_core_pool_fee_credits(platform_version)?.div_ceil(CREDITS_PER_DUFF);
        let l1_tx_fee_duffs = tolerance::SHIELD_FROM_CORE_L1_FEE.apply_fee(0);
        Ok((platform_fee_duffs, l1_tx_fee_duffs))
    }

    /// What shielding `amount_duffs` from the Core wallet locks, and what
    /// reaches the shielded pool once the network has taken its fee, at the
    /// `platform_version` the shield is sized at.
    pub(crate) fn shield_from_core_funding(
        &self,
        amount_duffs: u64,
        platform_version: &PlatformVersion,
    ) -> Result<ShieldFromCoreFunding, Box<ProtocolError>> {
        let pool_fee_credits = shield_from_core_pool_fee_credits(platform_version)?;
        let lock_duffs = amount_duffs.saturating_add(pool_fee_credits.div_ceil(CREDITS_PER_DUFF));
        Ok(ShieldFromCoreFunding {
            lock_duffs,
            shielded_credits: lock_duffs
                .saturating_mul(CREDITS_PER_DUFF)
                .saturating_sub(pool_fee_credits),
        })
    }

    /// Estimate fee for identity update (adding/disabling keys)
    pub fn estimate_identity_update(&self) -> u64 {
        self.min_fee(upstream_min_fee(
            IdentityUpdateTransition::V0(Default::default())
                .calculate_min_required_fee(self.platform_version),
        ))
    }

    /// Estimate fee for creating an identity with `key_count` keys from an
    /// asset lock: upstream's minimum, which covers the asset-lock processing.
    pub fn estimate_identity_create(&self, key_count: usize) -> u64 {
        self.min_fee(upstream_min_fee(
            identity_create_transition(key_count).calculate_min_required_fee(self.platform_version),
        ))
    }

    /// Estimate fee for identity creation from addresses (asset lock).
    /// This includes base cost, asset lock cost, input/output costs, per-key costs,
    /// storage-based fees, and the [`tolerance::ADDRESS_INPUTS_FEE`] reserve.
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

        let inputs = input_count.max(1);
        let upstream =
            IdentityCreateFromAddressesTransition::V0(IdentityCreateFromAddressesTransitionV0 {
                public_keys: vec![IdentityPublicKeyInCreation::V0(Default::default()); key_count],
                inputs: placeholder_address_inputs(inputs),
                output: has_output.then(placeholder_address_output),
                ..Default::default()
            })
            .calculate_min_required_fee(self.platform_version);
        let upstream = upstream_min_fee(upstream);
        // Identity creation also pays the identity-creation asset-lock
        // processing, which the from-addresses minimum leaves out.
        let asset_lock_cost = self
            .platform_version
            .dpp
            .state_transitions
            .identities
            .asset_locks
            .required_asset_lock_duff_balance_for_processing_start_for_identity_create
            .saturating_mul(CREDITS_PER_DUFF);

        // Add storage-based fees for serialized transaction data
        let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT
            + ESTIMATED_IDENTITY_BASE_BYTES
            + key_count * ESTIMATED_BYTES_PER_KEY;
        let estimated_seeks = ESTIMATED_SEEKS_BASE + inputs;
        let estimate = upstream
            .saturating_add(asset_lock_cost)
            .saturating_add(self.storage_based_fee(estimated_bytes, estimated_seeks));
        tolerance::ADDRESS_INPUTS_FEE.apply_fee(self.reserve(
            EstimateTolerance::NONE,
            estimate,
            upstream,
        ))
    }

    /// Estimate fee for topping up an identity from an asset lock:
    /// upstream's minimum, which covers the asset-lock processing.
    pub fn estimate_identity_topup(&self) -> u64 {
        self.min_fee(upstream_min_fee(
            IdentityTopUpTransition::V0(IdentityTopUpTransitionV0::default())
                .calculate_min_required_fee(self.platform_version),
        ))
    }

    /// Estimate fee for identity top-up from platform addresses.
    /// This includes base cost, asset lock cost, input costs, storage-based
    /// fees, and the [`tolerance::ADDRESS_INPUTS_FEE`] reserve.
    pub fn estimate_identity_topup_from_addresses(&self, input_count: usize) -> u64 {
        // Estimated bytes for top-up transaction structure
        const ESTIMATED_TOPUP_BASE_BYTES: usize = 100;
        // Estimated seek operations for tree traversal
        const ESTIMATED_SEEKS_BASE: usize = 8;

        let inputs = input_count.max(1);
        let upstream = upstream_min_fee(
            IdentityTopUpFromAddressesTransition::V0(IdentityTopUpFromAddressesTransitionV0 {
                inputs: placeholder_address_inputs(inputs),
                ..Default::default()
            })
            .calculate_min_required_fee(self.platform_version),
        );
        // A top-up also pays the address-funding asset-lock processing, which
        // the from-addresses minimum leaves out.
        let asset_lock_cost = self
            .platform_version
            .dpp
            .state_transitions
            .identities
            .asset_locks
            .required_asset_lock_duff_balance_for_processing_start_for_address_funding
            .saturating_mul(CREDITS_PER_DUFF);

        // Add storage-based fees for serialized transaction data
        let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT + ESTIMATED_TOPUP_BASE_BYTES;
        let estimated_seeks = ESTIMATED_SEEKS_BASE + inputs;
        let estimate = upstream
            .saturating_add(asset_lock_cost)
            .saturating_add(self.storage_based_fee(estimated_bytes, estimated_seeks));
        tolerance::ADDRESS_INPUTS_FEE.apply_fee(self.reserve(
            EstimateTolerance::NONE,
            estimate,
            upstream,
        ))
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

    /// Smallest fee Platform takes for a batch of `transition_count` document
    /// or token transitions, from upstream's minimum.
    fn document_batch_min_fee(&self, transition_count: usize) -> Credits {
        self.min_fees()
            .document_batch_sub_transition
            .saturating_mul(transition_count.max(1) as u64)
    }

    /// A document or token transition writing `bytes` with `seeks` tree seeks:
    /// upstream's minimum plus DET's storage estimate.
    fn document_write_fee(&self, bytes: usize, seeks: usize) -> u64 {
        let min = self.document_batch_min_fee(1);
        self.reserve(
            tolerance::PLATFORM_STORAGE,
            min.saturating_add(self.storage_based_fee(bytes, seeks)),
            min,
        )
    }

    /// Estimate fee for document batch transition
    // TODO(dashpay/platform#5383): mirrors Platform's minimum-fee floor; it is
    // not an estimate of the charge.
    pub fn estimate_document_batch(&self, transition_count: usize) -> u64 {
        self.min_fee(self.document_batch_min_fee(transition_count))
    }

    /// Balance an identity needs to register a username of `label_chars`
    /// characters, without the community vote fee.
    ///
    /// Covers both registration documents of an identity's first name. Platform
    /// accepts the second one only while the balance holds a reserve above what
    /// it then charges, so the amount actually charged is lower than this.
    pub fn estimate_username_registration(&self, label_chars: usize, needs_vote: bool) -> u64 {
        let (name_bytes, bytes_per_char) = if needs_vote {
            (
                USERNAME_VOTED_NAME_RESERVED_BYTES,
                USERNAME_VOTED_NAME_BYTES_PER_CHAR,
            )
        } else {
            (USERNAME_NAME_RESERVED_BYTES, USERNAME_NAME_BYTES_PER_CHAR)
        };
        let bytes = USERNAME_FIRST_DOCUMENT_BYTES
            .saturating_add(name_bytes)
            .saturating_add(bytes_per_char.saturating_mul(label_chars));
        self.reserve(
            tolerance::USERNAME_REGISTRATION,
            self.storage_fee(bytes)
                .saturating_add(USERNAME_FIRST_DOCUMENT_PROCESSING_CREDITS),
            // The preorder and the name document, each a batch of one.
            self.document_batch_min_fee(1).saturating_mul(2),
        )
    }

    /// What a username registration takes from the identity balance: the
    /// registration estimate plus the community vote fund (`contest_fund`,
    /// zero for a name that opens no vote). The fund is an exact amount, so
    /// no multiplier or margin applies to it.
    pub fn estimate_username_registration_charge(
        &self,
        label_chars: usize,
        needs_vote: bool,
        contest_fund: u64,
    ) -> u64 {
        self.estimate_username_registration(label_chars, needs_vote)
            .saturating_add(contest_fund)
    }

    /// Estimate fee for document creation with known size.
    /// Documents are stored in the contract's document tree.
    /// Estimated seeks: ~10 for tree traversal and insertion.
    pub fn estimate_document_create_with_size(&self, document_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        self.document_write_fee(document_bytes, ESTIMATED_SEEKS)
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
        self.document_write_fee(0, ESTIMATED_SEEKS)
    }

    /// Estimate fee for document replacement (uses default estimate of ~200 bytes).
    pub fn estimate_document_replace(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        self.document_write_fee(200, ESTIMATED_SEEKS)
    }

    /// Conservative fee estimate for a contact-info write, including all preserved
    /// accounts. The UI does not have their count, so use the contract payload limit.
    pub fn estimate_contact_info_write(&self) -> u64 {
        // encToUserId (32), key indexes (8), document ID, owner and revision (72).
        const FIXED_DOCUMENT_BYTES: usize = 112;
        self.estimate_document_create_with_size(
            FIXED_DOCUMENT_BYTES + crate::model::dashpay::MAX_CONTACT_INFO_ENCRYPTED_BYTES,
        )
    }

    /// Estimate fee for document transfer.
    /// Transfer updates ownership, minimal storage change.
    pub fn estimate_document_transfer(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const OWNERSHIP_UPDATE_BYTES: usize = 64;
        self.document_write_fee(OWNERSHIP_UPDATE_BYTES, ESTIMATED_SEEKS)
    }

    /// Estimate fee for document purchase.
    pub fn estimate_document_purchase(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 10;
        const PURCHASE_UPDATE_BYTES: usize = 100;
        self.document_write_fee(PURCHASE_UPDATE_BYTES, ESTIMATED_SEEKS)
    }

    /// Estimate fee for document set price.
    pub fn estimate_document_set_price(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const PRICE_UPDATE_BYTES: usize = 32;
        self.document_write_fee(PRICE_UPDATE_BYTES, ESTIMATED_SEEKS)
    }

    /// Estimate fee for token transition (mint, burn, transfer, freeze, etc.).
    /// Token operations are relatively small - mainly balance updates.
    pub fn estimate_token_transition(&self) -> u64 {
        const ESTIMATED_SEEKS: usize = 8;
        const TOKEN_OP_BYTES: usize = 100;
        self.document_write_fee(TOKEN_OP_BYTES, ESTIMATED_SEEKS)
    }

    /// Estimate fee for data contract creation with known size: the base
    /// registration fee and Platform's minimum, plus DET's storage estimate.
    fn estimate_contract_create_with_size(&self, contract_bytes: usize) -> u64 {
        const ESTIMATED_SEEKS: usize = 20;
        let min = self
            .platform_version
            .fee_version
            .data_contract_registration
            .base_contract_registration_fee
            .saturating_add(self.min_fees().contract_create);
        self.reserve(
            tolerance::PLATFORM_STORAGE,
            min.saturating_add(self.storage_based_fee(contract_bytes, ESTIMATED_SEEKS)),
            min,
        )
    }

    /// Estimate fee for data contract creation (uses base registration fee only).
    pub fn estimate_contract_create_base(&self) -> u64 {
        // Base registration fee (0.1 DASH) + minimal storage estimate
        self.estimate_contract_create_with_size(500)
    }

    /// Estimate fee for data contract update (uses a ~300 byte change).
    pub fn estimate_contract_update(&self) -> u64 {
        const UPDATE_BYTES: usize = 300;
        const ESTIMATED_SEEKS: usize = 15;
        let min = self.min_fees().contract_update;
        self.reserve(
            tolerance::PLATFORM_STORAGE,
            min.saturating_add(self.storage_based_fee(UPDATE_BYTES, ESTIMATED_SEEKS)),
            min,
        )
    }

    /// Smallest fee Platform takes for a transfer between Platform addresses,
    /// scaled by the multiplier.
    fn estimate_address_funds_transfer(&self, input_count: usize, output_count: usize) -> u64 {
        self.min_fee(AddressFundsTransferTransition::estimate_min_fee(
            input_count,
            output_count,
            self.platform_version,
        ))
    }
}

/// An identity-creation transition with `key_count` keys, for its minimum fee.
fn identity_create_transition(key_count: usize) -> IdentityCreateTransition {
    IdentityCreateTransition::V0(IdentityCreateTransitionV0 {
        public_keys: vec![IdentityPublicKeyInCreation::V0(Default::default()); key_count],
        ..Default::default()
    })
}

/// Community vote fee for registering a contested username, in credits.
///
/// Read from the protocol's vote-resolution fund (0.2 DASH before protocol 14,
/// 0.1 DASH from 14). Platform never returns it, whatever the vote outcome.
pub fn contest_fee_credits(platform_version: &PlatformVersion) -> Credits {
    platform_version
        .fee_version
        .vote_resolution_fund_fees
        .contested_document_vote_resolution_fund_required_amount
}

/// Balance an identity must hold, beyond the fee, for Platform to admit the
/// name document of a username registration, in credits. Never charged.
///
/// A name document type declaring a `chooseFirst` lookup (DPNS from contract
/// version 3) makes Platform scan competing preorders when it writes the name.
/// Before admitting it, Platform reserves the most that scan may bill: the
/// credits per competitor of the fee schedule times the competitors it may
/// read. A balance covering the fee but not this reserve is turned away after
/// the first document was already paid for. Zero for a type without
/// `chooseFirst`.
///
/// The two hashes Platform also reserves for the lookup cost a few thousand
/// credits and sit inside the margin of
/// [`PlatformFeeEstimator::estimate_username_registration`].
pub fn username_name_admission_reserve_credits(
    name_document_type: DocumentTypeRef<'_>,
    platform_version: &PlatformVersion,
) -> Credits {
    let declares_choose_first = name_document_type
        .reference_declarations()
        .filter_map(|(_, reference)| reference.target())
        .flat_map(|target| target.leaves())
        .filter_map(|leaf| leaf.as_any_document_reference())
        .any(|declaration| {
            declaration
                .lookup
                .is_some_and(|lookup| lookup.choose_first.is_some())
        });
    if !declares_choose_first {
        return 0;
    }
    platform_version
        .fee_version
        .document_reference
        .choose_first_credits_per_candidate
        .saturating_mul(u64::from(
            platform_version.system_limits.max_choose_first_candidates,
        ))
}

/// The optional parts of a token contract built by the token creator, each of
/// which adds its own registration fee.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenRegistrationFeatures {
    pub perpetual_distribution: bool,
    pub pre_programmed_distribution: bool,
    pub once_per_identity_distribution: bool,
    /// Contract keywords plus searchable token names; each costs one keyword fee.
    pub search_keyword_count: usize,
}

/// Headroom the token creator adds to the registration fees for processing
/// and storage (0.002 DASH).
const TOKEN_REGISTRATION_HEADROOM_CREDITS: Credits = 200_000_000;

/// Estimated cost of registering a single-token contract with `features`.
///
/// `fees` is the registration fee table of the protocol version the network
/// confirmed: the once-per-identity fee exists only from protocol 14, and a
/// table from another version would price the contract wrongly.
pub fn estimate_token_contract_registration(
    fees: &FeeDataContractRegistrationVersion,
    features: TokenRegistrationFeatures,
) -> Credits {
    let TokenRegistrationFeatures {
        perpetual_distribution,
        pre_programmed_distribution,
        once_per_identity_distribution,
        search_keyword_count,
    } = features;
    let optional_fees = [
        (
            perpetual_distribution,
            fees.token_uses_perpetual_distribution_fee,
        ),
        (
            pre_programmed_distribution,
            fees.token_uses_pre_programmed_distribution_fee,
        ),
        (
            once_per_identity_distribution,
            fees.token_uses_once_per_identity_distribution_fee,
        ),
    ];
    optional_fees
        .into_iter()
        .filter_map(|(used, fee)| used.then_some(fee))
        .fold(
            fees.base_contract_registration_fee
                .saturating_add(fees.token_registration_fee),
            Credits::saturating_add,
        )
        .saturating_add(
            fees.search_keyword_fee
                .saturating_mul(search_keyword_count as u64),
        )
        .saturating_add(TOKEN_REGISTRATION_HEADROOM_CREDITS)
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

/// Calculate the estimated fee for a platform address funds transfer:
/// upstream's minimum plus DET's storage estimate for the inputs, with the
/// [`tolerance::ADDRESS_INPUTS_FEE`] reserve.
pub(crate) fn estimate_platform_fee(
    estimator: &PlatformFeeEstimator,
    input_count: usize,
    output_count: usize,
) -> u64 {
    let inputs = input_count.max(1);
    let outputs = output_count.max(1);
    let base_fee = estimator.estimate_address_funds_transfer(inputs, outputs);
    // Storage fees for serialized input bytes only (outputs don't add
    // significant serialization overhead).
    let storage_fee =
        estimator.estimate_storage_based_fee(inputs * ESTIMATED_BYTES_PER_INPUT, inputs);
    tolerance::ADDRESS_INPUTS_FEE.apply_fee(base_fee.saturating_add(storage_fee))
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

    tolerance::PLATFORM_MIN_FEE.apply_fee(
        transition
            .calculate_min_required_fee(platform_version)
            .unwrap_or(0),
    )
}

/// Fee, in credits, the network takes from a Core wallet funding before the
/// rest reaches the shielded pool: the shielded fee for the two actions of the
/// bundle plus the cost of processing the funding transaction. It follows the
/// protocol version and is not scaled by the epoch fee multiplier.
fn shield_from_core_pool_fee_credits(
    platform_version: &PlatformVersion,
) -> Result<u64, Box<ProtocolError>> {
    let funding_cost_credits = platform_version
        .dpp
        .state_transitions
        .identities
        .asset_locks
        .required_asset_lock_duff_balance_for_processing_start_for_address_funding
        .saturating_mul(CREDITS_PER_DUFF);
    Ok(tolerance::SHIELDED_FEE.apply_fee(
        shielded_fee_for_actions(2, platform_version)?.saturating_add(funding_cost_credits),
    ))
}

/// A shield from the Core wallet: the funding to lock, and the credits that
/// reach the shielded pool from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShieldFromCoreFunding {
    pub lock_duffs: u64,
    pub shielded_credits: u64,
}

/// Smallest identity top-up funding, in duffs, the network accepts.
///
/// The network takes its fee out of the funding and refuses one that cannot
/// cover it. That fee follows the protocol version, is not scaled by the epoch
/// fee multiplier, and is rounded up here to whole duffs.
pub(crate) fn identity_topup_min_funding_duffs(
    platform_version: &PlatformVersion,
) -> Result<u64, Box<ProtocolError>> {
    IdentityTopUpTransition::V0(IdentityTopUpTransitionV0::default())
        .calculate_min_required_fee(platform_version)
        .map(|credits| {
            tolerance::PLATFORM_MIN_FEE
                .apply_fee(credits)
                .div_ceil(CREDITS_PER_DUFF)
        })
        .map_err(Box::new)
}

/// Smallest identity-creation funding, in duffs, the network accepts for an
/// identity created with `key_count` keys.
///
/// Same rule as [`identity_topup_min_funding_duffs`]; the fee the network
/// takes grows with every key the new identity starts with.
pub(crate) fn identity_create_min_funding_duffs(
    key_count: usize,
    platform_version: &PlatformVersion,
) -> Result<u64, Box<ProtocolError>> {
    identity_create_transition(key_count)
        .calculate_min_required_fee(platform_version)
        .map(|credits| {
            tolerance::PLATFORM_MIN_FEE
                .apply_fee(credits)
                .div_ceil(CREDITS_PER_DUFF)
        })
        .map_err(Box::new)
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

    tolerance::PLATFORM_MIN_FEE.apply_fee(
        transition
            .calculate_min_required_fee(platform_version)
            .unwrap_or(0),
    )
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
///
/// At most `max_inputs` addresses are spent; pass
/// [`max_platform_inputs`] of the network's protocol version.
pub(crate) fn allocate_platform_addresses_with_fee<F>(
    addresses: &[(PlatformAddress, Address, u64)],
    amount_credits: u64,
    destination: Option<&PlatformAddress>,
    max_inputs: usize,
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
    sorted_addresses.sort_by_key(|a| std::cmp::Reverse(a.2));

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
    for _ in 0..=max_inputs {
        inputs.clear();
        let mut remaining = amount_credits;

        for (idx, (platform_addr, _, balance)) in sorted_addresses.iter().enumerate() {
            if remaining == 0 || inputs.len() >= max_inputs {
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
    let input_cap = estimator.max_platform_inputs();
    let max_inputs = addresses
        .iter()
        .filter(|(platform_addr, _, _)| destination != Some(platform_addr))
        .count()
        .min(input_cap);

    allocate_platform_addresses_with_fee(addresses, amount_credits, destination, input_cap, |_| {
        // Keep the legacy behavior: use a worst-case fee based on max possible inputs.
        estimate_platform_fee(estimator, max_inputs.max(1), 1)
    })
}

/// The rate DET's Core transactions build at: the upstream builder's default.
fn core_fee_rate() -> FeeRate {
    FeeRate::normal()
}

/// The most coins one standard Core transaction may spend.
///
/// key-wallet enforces this limit but keeps its constant private, and
/// platform-wallet's `pooled_max_sendable` mirrors it as
/// `MAX_STANDARD_TX_INPUTS`; DET mirrors the same value.
// TODO(upstream): import the limit once key-wallet exports it.
pub const CORE_STANDARD_TX_MAX_INPUTS: usize = 500;

/// Fee, in duffs, one more input adds to a Core payment, from key-wallet's
/// own `estimate_tx_size`. A coin worth no more than this costs more to spend
/// than it brings, and a pooled send of the whole balance leaves it out.
pub fn core_send_input_fee_duffs() -> u64 {
    let per_input = estimate_tx_size(1, 1, false).saturating_sub(estimate_tx_size(0, 1, false));
    core_fee_rate().calculate_fee(per_input)
}

/// Fee, in duffs, a pooled send of `input_count` coins into one output paying
/// to a `destination` script costs with no change, priced as platform-wallet's
/// `pooled_max_sendable` prices it (one rounding for the transaction without
/// inputs, one per input), less what a script shorter than P2PKH saves.
// TODO(upstream): call platform-wallet once it exports this pricing; it is
// inline in `CoreWallet::pooled_max_sendable` today.
fn pooled_send_fee_duffs(input_count: usize, destination: CoreDestinationScript) -> u64 {
    core_fee_rate()
        .calculate_fee(estimate_tx_size(0, 1, false))
        .saturating_add(core_send_input_fee_duffs().saturating_mul(input_count as u64))
        .saturating_sub(core_output_script_fee_saved_duffs(
            CoreDestinationScript::PubkeyHash.script_len(),
            destination.script_len(),
        ))
}

/// The most key-wallet's payment build gives the miner on top of the fee of
/// the transaction it builds with change: a remainder no larger than this is
/// cheaper to forgo than to return, so the build keeps no change for it
/// (`CoinSelector::cost_of_change` and the builder's change dust threshold,
/// both 546 duffs at the default rate).
// TODO(upstream): use key-wallet's values once they are exported; both are
// private today.
const CORE_CHANGE_FORGONE_MAX_DUFFS: u64 = 546;

/// Fee, in duffs, a pooled Core send of `input_count` coins into one output
/// paying to a `destination` script is shown with at the Max: the fee the
/// pooled build charges when it sends the Max over the same coins.
/// `input_count` is clamped to at least 1.
pub fn estimate_core_send_fee_duffs(input_count: usize, destination: CoreDestinationScript) -> u64 {
    tolerance::CORE_SEND.apply_fee(pooled_send_fee_duffs(input_count.max(1), destination))
}

/// The Core send Max offered for `proven_max_duffs`, the largest payment the
/// pooled build was proven to accept, charging `fee_at_max_duffs`. It is
/// lowered by the headroom [`core_send_fee_at_max_duffs`] adds, so the Max and
/// the fee shown beside it never add up to more than that build spent.
pub fn core_send_max_duffs(proven_max_duffs: u64, fee_at_max_duffs: u64) -> u64 {
    tolerance::CORE_SEND.apply_max(proven_max_duffs, fee_at_max_duffs)
}

/// Fee, in duffs, shown beside a quoted Core send Max: what its pooled build
/// charged.
pub fn core_send_fee_at_max_duffs(quote: &CoreSendQuoteFee) -> u64 {
    tolerance::CORE_SEND.apply_fee(quote.fee_at_max_duffs)
}

/// Fee, in duffs, shown for a pooled Core payment of `amount_duffs` to a
/// `destination` script. It is never below what the build charges.
///
/// `quote` is the quoted Max and what it was proven against. At that Max the
/// fee is the one its build charged. Otherwise which coins the build picks,
/// and whether it returns change, depends on coin selection, so the fee is
/// bounded: the transaction of every coin the build could spend, with change,
/// plus the remainder the build may give the miner instead of change, and
/// never more than what those coins hold beyond the amount. Without a quote,
/// `fallback_input_count` (every coin the wallet holds in the pooled accounts,
/// reserved ones included) stands in for the coins the build could spend.
// TODO(upstream): show the fee the build would charge once platform-wallet can
// quote a payment's fee without building and reserving it.
pub fn estimate_core_payment_fee_duffs(
    destination: CoreDestinationScript,
    amount_duffs: u64,
    quote: Option<(u64, CoreSendQuoteFee)>,
    fallback_input_count: usize,
) -> u64 {
    if let Some((max_duffs, quote)) = quote
        && amount_duffs == max_duffs
    {
        return core_send_fee_at_max_duffs(&quote);
    }
    let input_count = quote
        .map(|(_, quote)| quote.spendable_input_count)
        .unwrap_or(fallback_input_count)
        .max(1);
    let change_output_fee = core_fee_rate()
        .calculate_fee(estimate_tx_size(0, 2, false).saturating_sub(estimate_tx_size(0, 1, false)));
    let bound = tolerance::CORE_SEND.apply_fee(
        pooled_send_fee_duffs(input_count, destination)
            .saturating_add(change_output_fee)
            .saturating_add(CORE_CHANGE_FORGONE_MAX_DUFFS),
    );
    match quote.and_then(|(_, quote)| quote.spendable_total_duffs) {
        Some(total_duffs) => bound.min(total_duffs.saturating_sub(amount_duffs)),
        None => bound,
    }
}

/// Core (L1) fee, in duffs, of a payment spending `num_inputs` chosen coins
/// into `num_outputs` outputs, sized by key-wallet's `estimate_tx_size` at the
/// rate the send builds with. A change output is counted: a payment from
/// chosen coins returns what it does not spend, and leaving it out understates
/// the fee. Both counts are clamped to at least 1.
pub fn estimate_core_l1_send_fee_duffs(num_inputs: usize, num_outputs: usize) -> u64 {
    let size = estimate_core_l1_send_size_bytes(num_inputs, num_outputs);
    tolerance::CORE_SEND.apply_fee(core_fee_rate().calculate_fee(size))
}

/// Size, in bytes, of the payment [`estimate_core_l1_send_fee_duffs`]
/// prices: `num_inputs` coins into `num_outputs` outputs plus change.
pub fn estimate_core_l1_send_size_bytes(num_inputs: usize, num_outputs: usize) -> usize {
    estimate_tx_size(num_inputs.max(1), num_outputs.max(1), true)
}

/// Core transaction fee, in duffs, to allow for when `num_inputs` payments
/// received by the wallet are spent into one funding transaction.
///
/// What the funding builder can lock is the spent amount less this fee, so a
/// deposit meant to be used whole must bring it on top. Never understates
/// what the builder charges: the transaction is sized as two plain outputs
/// (the burn output and the credit output carried in the payload), and
/// [`tolerance::ASSET_LOCK_CORE_FEE`] covers the payload bytes upstream does
/// not size. A test in `wallet_backend::payments` holds this against the
/// builder.
pub fn estimate_asset_lock_core_fee_duffs(num_inputs: usize) -> u64 {
    let size = estimate_tx_size(num_inputs.max(1), 2, false);
    tolerance::ASSET_LOCK_CORE_FEE.apply_fee(core_fee_rate().calculate_fee(size))
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

/// Fee (credits) shielding from Platform is estimated to cost: the
/// two-action minimum shielded fee the upstream shield claims from its
/// fee-paying input. The planner keeps a larger reserve back
/// ([`shield_from_balance_fee_headroom`]); what the fee does not use stays on
/// the Platform address. `0` when the protocol version has no shielded-fee
/// formula (the backend re-validates before dispatch).
pub fn shield_from_balance_fee_estimate(platform_version: &PlatformVersion) -> u64 {
    tolerance::SHIELDED_FEE.apply_fee(shielded_fee_for_actions(2, platform_version).unwrap_or(0))
}

/// Fee (duffs) a Core payment saves when its output script is `script_len`
/// bytes instead of `priced_script_len`, at the default rate the send builds
/// with. Zero when the script is not smaller.
pub fn core_output_script_fee_saved_duffs(priced_script_len: usize, script_len: usize) -> u64 {
    core_fee_rate().calculate_fee(priced_script_len.saturating_sub(script_len))
}

/// Balance (credits) shielding from Platform keeps back for its fee: the
/// upstream shield planner's versioned reserve, which the fee-paying input
/// retains unclaimed. The planner derives the Max it accepts from this same
/// value, so a hint or preview built on it matches what the build does; the
/// actual fee is lower, and the rest stays on the Platform address. `0` when
/// the protocol version has no shielded-fee formula (the backend re-validates
/// before dispatch).
pub fn shield_from_balance_fee_headroom(platform_version: &PlatformVersion) -> u64 {
    tolerance::SHIELDED_FEE.apply_fee(shield_fee_reserve(platform_version))
}

/// The shield-from-Platform Max offered for `upstream_max_credits`, the
/// largest amount the upstream shield planner accepts. It is lowered by
/// exactly what [`shield_from_balance_fee_headroom`] adds to the planner's
/// reserve, so the two never add up to more than the addresses hold.
pub fn shield_from_balance_max_credits(
    upstream_max_credits: u64,
    platform_version: &PlatformVersion,
) -> u64 {
    tolerance::SHIELDED_FEE.apply_max(upstream_max_credits, shield_fee_reserve(platform_version))
}

/// The upstream shield planner's fee reserve, or `0` when the protocol
/// version has no shielded-fee formula.
fn shield_fee_reserve(platform_version: &PlatformVersion) -> u64 {
    platform_wallet::shield_fee_reserve_credits(platform_version).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upstream-derived Platform and shielded fee helpers go through the
    /// tolerance hook, and a shield Max is lowered by exactly what the hook
    /// adds to the fee it keeps back. Both tolerances are zero today, so the
    /// values are upstream's own.
    #[test]
    fn platform_and_shielded_helpers_go_through_the_tolerance_hook() {
        let version = PlatformVersion::latest();
        let reserve = platform_wallet::shield_fee_reserve_credits(version).expect("reserve");
        assert_eq!(
            shield_from_balance_fee_headroom(version),
            tolerance::SHIELDED_FEE.apply_fee(reserve)
        );
        assert_eq!(
            shield_from_balance_max_credits(1_000_000_000, version),
            tolerance::SHIELDED_FEE.apply_max(1_000_000_000, reserve)
        );
        assert_eq!(shield_from_balance_max_credits(0, version), 0);
        let shielded = shielded_fee_for_actions(2, version).expect("shielded fee");
        assert_eq!(
            shield_from_balance_fee_estimate(version),
            tolerance::SHIELDED_FEE.apply_fee(shielded)
        );
        let top_up = IdentityTopUpTransition::V0(IdentityTopUpTransitionV0::default())
            .calculate_min_required_fee(version)
            .expect("top-up fee");
        assert_eq!(
            identity_topup_min_funding_duffs(version).expect("minimum"),
            tolerance::PLATFORM_MIN_FEE
                .apply_fee(top_up)
                .div_ceil(CREDITS_PER_DUFF)
        );
    }

    /// Every optional token feature adds exactly its own fee from the given
    /// table, on top of the base, token, keyword and headroom terms.
    #[test]
    fn token_contract_registration_adds_each_feature_fee_from_the_given_table() {
        let fees = &PlatformVersion::latest()
            .fee_version
            .data_contract_registration;
        assert!(
            fees.token_uses_once_per_identity_distribution_fee > 0,
            "the protocol 14 table prices the once-per-identity distribution"
        );
        let plain =
            estimate_token_contract_registration(fees, TokenRegistrationFeatures::default());
        assert_eq!(
            plain,
            fees.base_contract_registration_fee
                + fees.token_registration_fee
                + TOKEN_REGISTRATION_HEADROOM_CREDITS
        );

        let with = |features| estimate_token_contract_registration(fees, features) - plain;
        assert_eq!(
            with(TokenRegistrationFeatures {
                perpetual_distribution: true,
                ..Default::default()
            }),
            fees.token_uses_perpetual_distribution_fee
        );
        assert_eq!(
            with(TokenRegistrationFeatures {
                pre_programmed_distribution: true,
                ..Default::default()
            }),
            fees.token_uses_pre_programmed_distribution_fee
        );
        assert_eq!(
            with(TokenRegistrationFeatures {
                once_per_identity_distribution: true,
                ..Default::default()
            }),
            fees.token_uses_once_per_identity_distribution_fee
        );
        assert_eq!(
            with(TokenRegistrationFeatures {
                search_keyword_count: 3,
                ..Default::default()
            }),
            3 * fees.search_keyword_fee
        );
    }

    #[test]
    fn contact_info_write_fee_covers_the_contract_limit_and_fee_multiplier() {
        let estimator = PlatformFeeEstimator::latest();
        assert_eq!(
            estimator.estimate_contact_info_write(),
            estimator.estimate_document_create_with_size(112 + 2048),
        );
        assert_eq!(
            PlatformFeeEstimator::latest_with_fee_multiplier(2000).estimate_contact_info_write(),
            estimator.estimate_contact_info_write().saturating_mul(2),
        );
    }

    #[test]
    fn document_contest_cap_preserves_ordinary_creates_and_caps_contested_names() {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::platform_value::Value;
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use dash_sdk::platform::Identifier;
        for version in [13, 14] {
            let pv = PlatformVersion::get(version).unwrap();
            let contract = load_system_data_contract(SystemDataContract::DPNS, pv).unwrap();
            let owner = Identifier::from([1; 32]);
            let document = crate::context::test_support::dpns_name_document(
                owner,
                "alice",
                Value::Identifier(owner.to_buffer()),
            );
            let domain = contract.document_type_for_name("domain").unwrap();
            assert_eq!(
                document_create_contest_fund_cap(domain, &document, pv).unwrap(),
                Some(contest_fee_credits(pv))
            );
            let preorder = contract.document_type_for_name("preorder").unwrap();
            assert_eq!(
                document_create_contest_fund_cap(preorder, &document, pv).unwrap(),
                None
            );
            let ordinary = crate::context::test_support::dpns_name_document(
                owner,
                "alice-123",
                Value::Identifier(owner.to_buffer()),
            );
            assert_eq!(
                document_create_contest_fund_cap(domain, &ordinary, pv).unwrap(),
                None
            );
        }
    }

    /// A moderation election pays the moderation fund, not the contested
    /// document fund every other contest pays.
    #[test]
    fn document_contest_cap_uses_the_moderation_fund_for_a_moderation_election() {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::document::{Document, DocumentV0};
        use dash_sdk::dpp::platform_value::Value;
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        let pv = PlatformVersion::latest();
        let contract =
            load_system_data_contract(SystemDataContract::ModerationCharters, pv).unwrap();
        let elected_charter = contract.document_type_for_name("electedCharter").unwrap();
        let document = Document::V0(DocumentV0 {
            properties: [("targetContractId".to_string(), Value::Identifier([3; 32]))].into(),
            ..Default::default()
        });
        let fund_fees = &pv.fee_version.vote_resolution_fund_fees;
        assert_ne!(
            fund_fees.moderation_vote_resolution_fund_required_amount,
            fund_fees.contested_document_vote_resolution_fund_required_amount
        );
        assert_eq!(
            document_create_contest_fund_cap(elected_charter, &document, pv).unwrap(),
            Some(fund_fees.moderation_vote_resolution_fund_required_amount)
        );
    }

    #[test]
    fn contest_fee_follows_protocol_version() {
        // USR-TC-005
        let pv13 = PlatformVersion::get(13).expect("PV13");
        let pv14 = PlatformVersion::get(14).expect("PV14");
        assert_eq!(contest_fee_credits(pv13), 20_000_000_000);
        assert_eq!(contest_fee_credits(pv14), 10_000_000_000);
    }

    /// Balance above the amount below which Platform refuses a transfer between
    /// identities, as read from its engine and seen in a refusal on testnet.
    #[test]
    fn credit_transfer_estimate_covers_the_balance_the_network_requires() {
        let required_above_amount = [
            8_555_220_u64, // protocol 11
            8_753_740,     // protocols 12 to 14
            8_754_240,     // signed with a hashed ECDSA key
            8_755_720,     // testnet
            9_038_740,     // signed with a BLS key
        ];
        let estimate = PlatformFeeEstimator::latest().estimate_credit_transfer();
        for required in required_above_amount {
            assert!(estimate >= required, "{estimate} is below {required}");
            assert!(
                estimate <= required / 100 * 115,
                "{estimate} is far above {required}"
            );
        }
    }

    /// The same for a transfer to Platform addresses, by number of addresses.
    /// Each address adds a little more once it receives 2,147,483,648 credits
    /// or more; BLS keys add to that.
    #[test]
    fn credit_transfer_to_addresses_estimate_covers_the_balance_the_network_requires() {
        let required_above_amount = [
            (1, 17_091_200_u64), // protocol 11
            (1, 17_329_060),     // protocols 12 to 14
            (1, 17_331_040),     // testnet
            (1, 17_438_660),     // a larger amount
            (1, 17_723_660),     // a larger amount, signed with a BLS key
            (2, 23_396_440),
            (2, 23_615_640),
            (2, 23_900_640),
            (3, 29_463_820),
            (3, 29_792_620),
            (3, 30_077_620),
        ];
        let estimator = PlatformFeeEstimator::latest();
        for (addresses, required) in required_above_amount {
            let estimate = estimator.estimate_credit_transfer_to_addresses(addresses);
            assert!(
                estimate >= required,
                "{addresses} addresses: {estimate} is below {required}"
            );
            assert!(
                estimate <= required / 100 * 115,
                "{addresses} addresses: {estimate} is far above {required}"
            );
        }
    }

    #[test]
    fn credit_transfer_estimates_follow_the_fee_multiplier() {
        let base = PlatformFeeEstimator::latest();
        let doubled = PlatformFeeEstimator::latest_with_fee_multiplier(2000);
        assert_eq!(
            doubled.estimate_credit_transfer(),
            base.estimate_credit_transfer() * 2
        );
        assert_eq!(
            doubled.estimate_credit_transfer_to_addresses(2),
            base.estimate_credit_transfer_to_addresses(2) * 2
        );
    }

    /// Platform refuses any transfer that leaves less than its own minimum
    /// fee, so an estimate never falls below that minimum, whatever it becomes.
    #[test]
    fn credit_transfer_estimates_never_fall_below_the_minimum_fee_of_the_network() {
        // A DET estimate below upstream's minimum is lifted to the minimum.
        let estimator = PlatformFeeEstimator::latest();
        assert_eq!(
            estimator.reserve(EstimateTolerance::NONE, 5, 50_000_000),
            50_000_000
        );
        assert_eq!(
            estimator.reserve(EstimateTolerance::percent(10), 100, 50),
            110,
            "an estimate above the minimum keeps its tolerance"
        );
        for version in 1..=PlatformVersion::latest().protocol_version {
            let pv = PlatformVersion::get(version).expect("known protocol version");
            let estimator = PlatformFeeEstimator::new(pv, 1000);
            let min_fees = &pv.fee_version.state_transition_min_fees;
            assert!(estimator.estimate_credit_transfer() >= min_fees.credit_transfer);
            assert!(
                estimator.estimate_credit_transfer_to_addresses(3)
                    >= min_fees.credit_transfer_to_addresses
                        + 3 * min_fees.address_funds_transfer_output_cost,
                "protocol {version}"
            );
        }
    }

    /// Funding an address from the wallet allows for at least the funding the
    /// network requires before it processes one: 0.0005 DASH plus 0.00006
    /// DASH per recipient. DET's own estimate (10,000 and 18,750 duffs) is
    /// below that, so the requirement binds.
    #[test]
    fn address_funding_from_the_wallet_allows_for_what_the_network_requires() {
        let estimator = PlatformFeeEstimator::latest();
        assert_eq!(
            estimator.estimate_address_funding_from_asset_lock_duffs(1),
            56_000
        );
        assert_eq!(
            estimator.estimate_address_funding_from_asset_lock_duffs(2),
            62_000
        );
    }

    #[test]
    fn max_spendable_credits_is_exact_above_f64_integer_precision() {
        let balance = (1_u64 << 53) + 17;
        assert_eq!(max_spendable_credits(balance, 11), balance - 11);
        assert_eq!(max_spendable_credits(10, 11), 0);
    }

    /// 0.000505 DASH is the "required" figure the network reports when it
    /// refuses a smaller top-up funding under protocol 13.
    #[test]
    fn identity_topup_minimum_funding_is_the_fee_the_network_requires() {
        let pv13 = PlatformVersion::get(13).expect("PV13");
        assert_eq!(
            identity_topup_min_funding_duffs(pv13).expect("PV13 defines the top-up fee"),
            50_500
        );
    }

    /// Under protocol 13 the network takes 0.00202 DASH plus 0.000065 DASH for
    /// every key from the funding of a new identity.
    #[test]
    fn identity_create_minimum_funding_grows_with_the_number_of_keys() {
        let pv13 = PlatformVersion::get(13).expect("PV13");
        let minimum = |keys| {
            identity_create_min_funding_duffs(keys, pv13).expect("PV13 defines the creation fee")
        };
        assert_eq!(minimum(1), 208_500);
        assert_eq!(minimum(4), 228_000);
    }

    /// With the fee taken from the amount, the amount is the whole funding and
    /// must cover the fee for one recipient: 0.00056 DASH under protocol 13.
    #[test]
    fn address_funding_minimum_is_the_whole_fee_when_taken_from_the_amount() {
        let pv13 = PlatformVersion::get(13).expect("PV13");
        assert_eq!(
            PlatformFeeEstimator::new(pv13, 1000).address_funding_min_amount_duffs(true),
            56_000
        );
    }

    /// With the fee paid on top, the funding is the amount plus the estimate,
    /// which already covers the 62 000 duffs the network requires for two
    /// recipients, so only the smallest named amount binds.
    #[test]
    fn address_funding_minimum_with_change_makes_up_what_the_estimate_leaves_short() {
        let pv13 = PlatformVersion::get(13).expect("PV13");
        let smallest_named = pv13
            .dpp
            .state_transitions
            .address_funds
            .min_output_amount
            .div_ceil(CREDITS_PER_DUFF);
        assert_eq!(
            PlatformFeeEstimator::new(pv13, 1000).address_funding_min_amount_duffs(false),
            smallest_named
        );
    }

    /// Whatever the fee multiplier, an amount at the minimum yields a funding
    /// the network accepts for two recipients, and one duff less does not
    /// unless the smallest named amount is what binds.
    #[test]
    fn address_funding_with_change_at_the_minimum_funds_what_the_network_requires() {
        let pv13 = PlatformVersion::get(13).expect("PV13");
        let required_credits =
            calculate_address_funding_from_asset_lock_min_required_fee(0, 2, pv13);
        let smallest_output_duffs = pv13
            .dpp
            .state_transitions
            .address_funds
            .min_output_amount
            .div_ceil(CREDITS_PER_DUFF);
        for multiplier in [1000, 1500, 2000, 3000, 4000, 10_000] {
            let estimator = PlatformFeeEstimator::new(pv13, multiplier);
            let minimum = estimator.address_funding_min_amount_duffs(false);
            let funding = |amount: u64| {
                (amount + estimator.estimate_address_funding_from_asset_lock_duffs(2))
                    * CREDITS_PER_DUFF
            };
            assert!(
                funding(minimum) >= required_credits,
                "multiplier {multiplier}: minimum {minimum} leaves the funding short"
            );
            assert!(
                minimum >= smallest_output_duffs,
                "multiplier {multiplier}: minimum {minimum} is below the smallest named amount"
            );
            assert!(
                minimum == smallest_output_duffs || funding(minimum - 1) < required_credits,
                "multiplier {multiplier}: minimum {minimum} refuses an amount the network accepts"
            );
        }
    }

    #[test]
    fn test_identity_create_estimate() {
        let estimator = PlatformFeeEstimator::latest();
        // Base cost + asset lock cost + 2 keys
        let fee = estimator.estimate_identity_create(2);
        assert_eq!(fee, 2_000_000 + 200_000_000 + 2 * 6_500_000);
    }

    #[test]
    fn test_identity_topup_actual_fee_uses_balance_delta_when_consistent() {
        let estimator = PlatformFeeEstimator::latest();
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
        let estimator = PlatformFeeEstimator::latest();
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
        let estimator = PlatformFeeEstimator::latest();
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
        let estimator = PlatformFeeEstimator::latest();
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
        let estimator = PlatformFeeEstimator::latest();
        // 3 documents - base fee only
        let fee = estimator.estimate_document_batch(3);
        assert_eq!(fee, 3 * 100_000);
    }

    /// Registering a 29-character name for a newly created identity was
    /// charged 75,646,740 credits on testnet. The estimate must cover that
    /// without being several times larger.
    #[test]
    fn username_registration_estimate_covers_the_charge_measured_on_testnet() {
        const CHARGED_ON_TESTNET: u64 = 75_646_740;
        let estimate = PlatformFeeEstimator::latest().estimate_username_registration(29, false);
        assert!(estimate >= CHARGED_ON_TESTNET, "{estimate}");
        assert!(estimate <= 2 * CHARGED_ON_TESTNET, "{estimate}");
    }

    /// Platform accepts the second document only while the balance covers a
    /// reserve well above what it then charges. These are the smallest balances
    /// its engine (protocol 14) accepted for both documents of an identity's
    /// first name, without the community vote fee. The estimate must reach
    /// each one and stay within 15% of it.
    #[test]
    fn username_registration_estimate_covers_the_balance_the_network_requires() {
        let smallest_accepted_balance = [
            (6, false, 115_965_270_u64),
            (29, false, 118_309_783),
            (63, false, 122_264_862),
            (3, true, 156_187_414),
            (5, true, 156_750_678),
            (19, true, 158_495_902),
        ];
        let estimator = PlatformFeeEstimator::latest();
        for (label_chars, needs_vote, required) in smallest_accepted_balance {
            let estimate = estimator.estimate_username_registration(label_chars, needs_vote);
            assert!(
                estimate >= required,
                "{label_chars} characters, vote {needs_vote}: {estimate} is below {required}"
            );
            assert!(
                estimate <= required / 100 * 115,
                "{label_chars} characters, vote {needs_vote}: {estimate} is far above {required}"
            );
        }
    }

    /// The community vote fund is charged on top of the registration, as is.
    #[test]
    fn username_registration_charge_adds_the_contest_fund_as_is() {
        let estimator = PlatformFeeEstimator::latest();
        let registration = estimator.estimate_username_registration(5, true);
        assert_eq!(
            estimator.estimate_username_registration_charge(5, true, 1_234),
            registration + 1_234
        );
        assert_eq!(
            estimator.estimate_username_registration_charge(9, false, 0),
            estimator.estimate_username_registration(9, false)
        );
    }

    /// On testnet the network refused the name document below these balances,
    /// counting what the first document had cost: 32,090,520 + 83,627,060 for
    /// 20 characters and 32,400,260 + 88,397,980 for 63.
    #[test]
    fn username_registration_estimate_covers_the_balance_testnet_required() {
        let estimator = PlatformFeeEstimator::latest();
        for (label_chars, required) in [(20, 115_717_580_u64), (63, 120_798_240)] {
            let estimate = estimator.estimate_username_registration(label_chars, false);
            assert!(estimate >= required, "{label_chars}: {estimate}");
            assert!(
                estimate <= required / 100 * 115,
                "{label_chars}: {estimate}"
            );
        }
    }

    /// DPNS from contract version 3 picks the first preorder of a name, and
    /// Platform admits the name document only with 0.001 DASH held beyond its
    /// fee for that scan. Earlier contracts need no such reserve.
    #[test]
    fn username_name_admission_reserve_follows_the_contract_choose_first_rule() {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        for (version, expected) in [(14, 0), (15, 100_000_000)] {
            let pv = PlatformVersion::get(version).unwrap();
            let contract = load_system_data_contract(SystemDataContract::DPNS, pv).unwrap();
            let domain = contract.document_type_for_name("domain").unwrap();
            assert_eq!(
                username_name_admission_reserve_credits(domain, pv),
                expected,
                "protocol {version}"
            );
            let preorder = contract.document_type_for_name("preorder").unwrap();
            assert_eq!(
                username_name_admission_reserve_credits(preorder, pv),
                0,
                "protocol {version}: the preorder has no chooseFirst lookup"
            );
        }
    }

    #[test]
    fn username_registration_estimate_follows_the_fee_multiplier() {
        let base = PlatformFeeEstimator::latest().estimate_username_registration(29, false);
        let doubled = PlatformFeeEstimator::latest_with_fee_multiplier(2000)
            .estimate_username_registration(29, false);
        assert_eq!(doubled, 2 * base);
    }

    #[test]
    fn test_storage_fee_calculation() {
        let estimator = PlatformFeeEstimator::latest();
        // 500 bytes at 27,000 credits/byte = 13,500,000 credits
        let fee = estimator.storage_fee(500);
        assert_eq!(fee, 500 * 27_000);
        // 13,500,000 credits = 0.000135 DASH (at 100 billion credits per DASH)
        assert_eq!(format_credits_as_dash(fee), "0.000135 DASH");
    }

    #[test]
    fn test_contract_create_with_size() {
        let estimator = PlatformFeeEstimator::latest();
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
        assert_eq!(format_credits_as_dash(100_000_000_000), "1 DASH");
        assert_eq!(format_credits_as_dash(100_000_000), "0.001 DASH");
        assert_eq!(format_credits_as_dash(100_000), "0.000001 DASH");
    }

    /// The Core fee of a payment from chosen coins is key-wallet's own size
    /// estimate at its default rate, change output included: 10 + 148 per
    /// input + 34 per output + 34 for the change, at 1 duff per byte. A
    /// testnet payment of two coins to two recipients built with change and
    /// was charged 408 duffs.
    #[test]
    fn core_l1_send_fee_is_the_upstream_size_at_the_default_rate() {
        assert_eq!(estimate_core_l1_send_fee_duffs(1, 1), 226);
        assert_eq!(estimate_core_l1_send_fee_duffs(2, 2), 408);
        assert_eq!(estimate_core_l1_send_fee_duffs(2, 3), 442);
        assert_eq!(
            estimate_core_l1_send_fee_duffs(0, 0),
            estimate_core_l1_send_fee_duffs(1, 1),
            "zero counts are clamped to one each"
        );
    }

    /// A pooled send is priced as platform-wallet's `pooled_max_sendable`
    /// prices it, which at a whole duff per byte is the plain size estimate,
    /// less what a shorter P2SH output script saves.
    #[test]
    fn core_send_fee_and_max_follow_the_pooled_pricing() {
        use crate::model::spend_limit::CoreDestinationScript::{PubkeyHash, ScriptHash};
        assert_eq!(core_send_input_fee_duffs(), 148);
        for inputs in 1..=10 {
            let p2pkh = core_fee_rate().calculate_fee(estimate_tx_size(inputs, 1, false));
            assert_eq!(estimate_core_send_fee_duffs(inputs, PubkeyHash), p2pkh);
            assert_eq!(estimate_core_send_fee_duffs(inputs, ScriptHash), p2pkh - 2);
            assert_eq!(core_send_max_duffs(1_000_000, p2pkh), 1_000_000);
        }
        assert_eq!(
            estimate_core_send_fee_duffs(0, PubkeyHash),
            estimate_core_send_fee_duffs(1, PubkeyHash)
        );
    }

    /// With a Max quote, the fee shown at the Max is the fee the quoted build
    /// charged, and below it the cap is what the unreserved coins hold. A Max
    /// cut short by the deadline (699,360 duffs from one free 700,000-duff
    /// coin, built without change and charged 640) shows 640, not the 340 of
    /// a full drain; with a second coin reserved by another build, the Max of
    /// the free coin shows that coin's fee, so Max + fee is the free coin.
    #[test]
    fn core_payment_fee_comes_from_the_quoted_build() {
        use crate::model::spend_limit::CoreDestinationScript::PubkeyHash;
        use crate::model::spend_limit::CoreSendQuoteFee;
        const FREE_DUFFS: u64 = 700_000;

        let partial = CoreSendQuoteFee {
            fee_at_max_duffs: 640,
            spendable_input_count: 1,
            spendable_total_duffs: Some(FREE_DUFFS),
        };
        assert_eq!(core_send_fee_at_max_duffs(&partial), 640);
        assert_eq!(
            core_send_max_duffs(699_360, partial.fee_at_max_duffs),
            699_360
        );
        let at_max =
            estimate_core_payment_fee_duffs(PubkeyHash, 699_360, Some((699_360, partial)), 2);
        assert_eq!(at_max, 640);
        assert!(699_360 + at_max <= FREE_DUFFS);
        assert_eq!(
            estimate_core_payment_fee_duffs(PubkeyHash, 699_000, Some((699_360, partial)), 2),
            226 + 546,
            "below the Max, priced over the one free coin, not the two the snapshot holds"
        );
        assert_eq!(
            estimate_core_payment_fee_duffs(PubkeyHash, 699_500, Some((699_360, partial)), 2),
            500,
            "never more than what the free coin holds beyond the amount"
        );

        let reserved = CoreSendQuoteFee {
            fee_at_max_duffs: 192,
            spendable_input_count: 1,
            spendable_total_duffs: Some(FREE_DUFFS),
        };
        let at_max =
            estimate_core_payment_fee_duffs(PubkeyHash, 699_808, Some((699_808, reserved)), 2);
        assert_eq!(at_max, 192);
        assert_eq!(699_808 + at_max, FREE_DUFFS);
        assert_eq!(
            estimate_core_payment_fee_duffs(PubkeyHash, 100_000, Some((699_808, reserved)), 2),
            226 + 546,
            "far below the Max, one free coin with change plus the forgone remainder"
        );
    }

    /// The fee shown for a pooled Core payment is never below what the build
    /// charges. Below the Max the build may add change (226 duffs for one
    /// coin) or give a remainder of up to 546 duffs to the miner instead, so
    /// that much is shown; it never exceeds what the coins hold beyond the
    /// amount, which at the Max is exactly the Max's fee, for either script.
    #[test]
    fn core_payment_fee_covers_change_and_adds_up_at_the_max() {
        use crate::model::spend_limit::CoreDestinationScript::{PubkeyHash, ScriptHash};
        use crate::model::spend_limit::CoreSendQuoteFee;
        const COINS_DUFFS: u64 = 2_000_000;

        assert_eq!(
            estimate_core_payment_fee_duffs(PubkeyHash, 100_000, None, 1),
            226 + 546,
            "with no Max quoted yet, the bound stands alone"
        );
        assert_eq!(
            estimate_core_payment_fee_duffs(ScriptHash, 100_000, None, 1),
            224 + 546
        );

        for destination in [PubkeyHash, ScriptHash] {
            let max_fee = estimate_core_send_fee_duffs(2, destination);
            let max = COINS_DUFFS - max_fee;
            let quote = CoreSendQuoteFee {
                fee_at_max_duffs: max_fee,
                spendable_input_count: 2,
                spendable_total_duffs: Some(COINS_DUFFS),
            };
            let at_max = estimate_core_payment_fee_duffs(destination, max, Some((max, quote)), 2);
            assert_eq!(at_max, max_fee, "{destination:?}");
            assert_eq!(max + at_max, COINS_DUFFS, "{destination:?}");
            assert_eq!(
                estimate_core_payment_fee_duffs(destination, max - 100, Some((max, quote)), 2),
                max_fee + 100,
                "{destination:?}: just below the Max, what the coins hold beyond the amount"
            );
        }
    }

    /// The asset-lock Core fee keeps its reserve over the plain size: the
    /// builder's payload bytes are not in upstream's estimate.
    #[test]
    fn asset_lock_core_fee_reserves_over_the_upstream_size() {
        // 10 + 148 + 2 × 34 = 226 bytes, plus 15 %.
        assert_eq!(estimate_asset_lock_core_fee_duffs(1), 259);
        // 10 + 296 + 68 = 374 bytes, plus 15 %.
        assert_eq!(estimate_asset_lock_core_fee_duffs(2), 430);
    }

    /// The fees that are upstream's minimum come from upstream's functions at
    /// the estimator's protocol version, scaled by the multiplier.
    #[test]
    fn minimum_fee_estimates_are_upstreams_at_the_estimators_version() {
        for version in [12, 13, PlatformVersion::latest().protocol_version] {
            let pv = PlatformVersion::get(version).expect("known protocol version");
            let estimator = PlatformFeeEstimator::new(pv, 2000);
            let min_fees = &pv.fee_version.state_transition_min_fees;
            let topup = IdentityTopUpTransition::V0(IdentityTopUpTransitionV0::default())
                .calculate_min_required_fee(pv)
                .expect("top-up fee");
            assert_eq!(
                estimator.estimate_identity_topup(),
                topup * 2,
                "protocol {version}"
            );
            let create = identity_create_transition(3)
                .calculate_min_required_fee(pv)
                .expect("creation fee");
            assert_eq!(estimator.estimate_identity_create(3), create * 2);
            assert_eq!(
                estimator.estimate_identity_update(),
                min_fees.identity_update * 2
            );
            assert_eq!(
                estimator.estimate_credit_withdrawal(),
                min_fees.credit_withdrawal * 2
            );
            assert_eq!(
                estimator.estimate_address_credit_withdrawal(),
                AddressCreditWithdrawalTransition::estimate_min_fee(1, false, pv) * 2
            );
            assert_eq!(
                estimator.estimate_document_batch(3),
                min_fees.document_batch_sub_transition * 3 * 2
            );
            assert_eq!(
                estimator.estimate_address_funds_transfer(2, 1),
                AddressFundsTransferTransition::estimate_min_fee(2, 1, pv) * 2
            );
            assert_eq!(
                estimator.max_platform_inputs(),
                usize::from(pv.dpp.state_transitions.max_address_inputs)
            );
        }
    }

    /// The tolerance never overflows: a huge fee or percentage saturates at
    /// `u64::MAX` instead of wrapping to a smaller fee, and a Max smaller than
    /// the headroom is zero, never a wrapped-around large amount.
    #[test]
    fn estimate_tolerance_saturates_at_the_extremes() {
        let huge = EstimateTolerance::percent(u64::MAX).with_floor(u64::MAX);
        assert_eq!(huge.apply_fee(0), u64::MAX);
        assert_eq!(huge.apply_fee(u64::MAX), u64::MAX);
        assert_eq!(huge.apply_max(u64::MAX, 0), 0);
        assert_eq!(huge.apply_max(12_345, 1), 0);

        let margin = EstimateTolerance::percent(50);
        assert_eq!(margin.apply_fee(u64::MAX), u64::MAX);
        // Half of `u64::MAX` plus half again, not a headroom cut short by an
        // intermediate overflow.
        assert_eq!(margin.apply_fee(u64::MAX / 2), u64::MAX / 2 + u64::MAX / 4);
        assert_eq!(
            EstimateTolerance::percent(200).apply_fee(u64::MAX / 2),
            u64::MAX
        );
        assert_eq!(margin.apply_max(0, 1_000), 0);
        assert_eq!(margin.apply_max(10, 1_000), 0);
        assert_eq!(margin.apply_max(u64::MAX, u64::MAX), u64::MAX);
        assert_eq!(EstimateTolerance::NONE.apply_max(0, u64::MAX), 0);
    }

    /// A tolerance only ever raises a fee and lowers a Max, and lowers the Max
    /// by exactly what it adds to the fee.
    #[test]
    fn estimate_tolerance_only_makes_results_safer() {
        let none = EstimateTolerance::NONE;
        assert_eq!(none.apply_fee(1_234), 1_234);
        assert_eq!(none.apply_max(10_000, 1_234), 10_000);

        let margin = EstimateTolerance::percent(15);
        assert_eq!(margin.apply_fee(200), 230);
        assert_eq!(margin.apply_max(10_000, 200), 9_970);

        let floored = EstimateTolerance::percent(50).with_floor(10_000);
        assert_eq!(floored.apply_fee(1_000), 10_000);
        assert_eq!(floored.apply_fee(30_000), 45_000);
        assert_eq!(
            floored.apply_max(5_000, 1_000),
            0,
            "a Max never wraps below zero"
        );

        for fee in [0, 1, 99, 1_000, u64::MAX / 200] {
            for tolerance in [none, margin, floored] {
                let raised = tolerance.apply_fee(fee);
                assert!(raised >= fee);
                assert_eq!(
                    tolerance.apply_max(u64::MAX, fee),
                    u64::MAX - (raised - fee)
                );
            }
        }
    }

    /// What the network takes from a Core wallet funding that is shielded, by
    /// the platform's own functions: the shielded fee for the two actions of
    /// the bundle, plus the cost of processing the funding transaction.
    fn network_shield_from_core_fee_credits(platform_version: &PlatformVersion) -> u64 {
        use dash_sdk::dpp::shielded::compute_minimum_shielded_fee;
        use dash_sdk::dpp::state_transition::shield_from_asset_lock_transition::ShieldFromAssetLockTransition;
        use dash_sdk::dpp::state_transition::shield_from_asset_lock_transition::v0::ShieldFromAssetLockTransitionV0;

        let funding_cost = ShieldFromAssetLockTransition::V0(ShieldFromAssetLockTransitionV0 {
            asset_lock_proof: AssetLockProof::default(),
            actions: Vec::new(),
            value_balance: 1,
            anchor: [0; 32],
            proof: Vec::new(),
            binding_signature: [0; 64],
            surplus_output: None,
            signature: Default::default(),
        })
        .calculate_min_required_fee(platform_version)
        .expect("funding cost");
        compute_minimum_shielded_fee(2, platform_version).expect("shielded fee") + funding_cost
    }

    /// Protocol versions that know the shield from the Core wallet, with the
    /// fee the network takes under each, in credits. Under 12 and 13 the fee is
    /// not a whole number of duffs; under 14 it is.
    const SHIELD_FROM_CORE_FEE_BY_PROTOCOL: [(u32, u64); 3] =
        [(12, 212_851_200), (13, 212_851_200), (14, 164_140_000)];

    /// The fee added to a shield from the Core wallet must be the fee the
    /// network takes, rounded up to whole duffs — not less, and not padded.
    #[test]
    fn shield_from_core_fee_is_what_the_network_takes() {
        for (protocol, fee_credits) in SHIELD_FROM_CORE_FEE_BY_PROTOCOL {
            let platform_version = PlatformVersion::get(protocol).expect("known protocol");
            let network_fee_credits = network_shield_from_core_fee_credits(platform_version);
            assert_eq!(network_fee_credits, fee_credits, "protocol {protocol}");
            for multiplier in [1000, 3000] {
                let (fee_duffs, _) = PlatformFeeEstimator::latest_with_fee_multiplier(multiplier)
                    .estimate_shield_from_core_fees_duffs(platform_version)
                    .expect("known version");
                assert_eq!(
                    fee_duffs,
                    network_fee_credits.div_ceil(CREDITS_PER_DUFF),
                    "protocol {protocol}, multiplier {multiplier}: the network takes \
                     {network_fee_credits} credits"
                );
            }
        }
    }

    /// The network shields what is left of the funding after its fee. That
    /// must be the entered amount (plus what rounding the fee up to whole
    /// duffs adds), and it is the amount to report as shielded.
    #[test]
    fn shield_from_core_delivers_and_reports_the_entered_amount() {
        for (protocol, _) in SHIELD_FROM_CORE_FEE_BY_PROTOCOL {
            let platform_version = PlatformVersion::get(protocol).expect("known protocol");
            let network_fee_credits = network_shield_from_core_fee_credits(platform_version);
            for amount_duffs in [1, 100_000, 100_000_000] {
                let funding = PlatformFeeEstimator::latest()
                    .shield_from_core_funding(amount_duffs, platform_version)
                    .expect("known version");
                let delivered_credits = (funding.lock_duffs * CREDITS_PER_DUFF)
                    .checked_sub(network_fee_credits)
                    .filter(|delivered| *delivered > 0)
                    .unwrap_or_else(|| {
                        panic!(
                            "protocol {protocol}, {amount_duffs} duffs: a funding of {} duffs \
                             does not cover the network fee of {network_fee_credits} credits, \
                             so nothing is shielded",
                            funding.lock_duffs
                        )
                    });
                let entered_credits = amount_duffs * CREDITS_PER_DUFF;
                assert!(
                    (entered_credits..entered_credits + CREDITS_PER_DUFF)
                        .contains(&delivered_credits),
                    "protocol {protocol}, {amount_duffs} duffs: {delivered_credits} credits are \
                     shielded"
                );
                assert_eq!(
                    funding.shielded_credits, delivered_credits,
                    "protocol {protocol}, {amount_duffs} duffs: the reported amount must be \
                     what is shielded"
                );
            }
        }
    }

    /// Rounding the fee up to whole duffs shields a little more than was
    /// entered wherever the fee is not a whole number of duffs. The reported
    /// amount carries that remainder; the entered amount would understate it.
    #[test]
    fn shield_from_core_reports_the_remainder_of_rounding_the_fee_up() {
        for (protocol, remainder_credits) in [(12, 800), (13, 800), (14, 0)] {
            let platform_version = PlatformVersion::get(protocol).expect("known protocol");
            for amount_duffs in [1, 100_000, 100_000_000] {
                let funding = PlatformFeeEstimator::latest()
                    .shield_from_core_funding(amount_duffs, platform_version)
                    .expect("known version");
                assert_eq!(
                    funding.shielded_credits,
                    amount_duffs * CREDITS_PER_DUFF + remainder_credits,
                    "protocol {protocol}, {amount_duffs} duffs"
                );
            }
        }
    }

    /// A fee that cannot be read must stop the shield, not default to a guess.
    #[test]
    fn shield_from_core_fee_is_unavailable_for_an_unknown_fee_version() {
        let mut platform_version = PlatformVersion::latest().clone();
        platform_version.dpp.methods.compute_minimum_shielded_fee = 99;
        let estimator = PlatformFeeEstimator::latest();
        assert!(
            estimator
                .estimate_shield_from_core_fees_duffs(&platform_version)
                .is_err()
        );
        assert!(
            estimator
                .shield_from_core_funding(100_000, &platform_version)
                .is_err()
        );
    }

    /// The shield fee preview states the fee the shield is charged, which is
    /// half the reserve the planner keeps back.
    #[test]
    fn shield_from_balance_fee_estimate_is_the_two_action_fee_not_the_reserve() {
        let pv = PlatformVersion::latest();
        let fee = shield_from_balance_fee_estimate(pv);
        assert_eq!(fee, shielded_fee_for_actions(2, pv).expect("shielded fee"));
        assert_eq!(fee * 2, shield_from_balance_fee_headroom(pv));
    }

    /// A smaller output script saves its bytes' fee at the default rate.
    #[test]
    fn core_output_script_fee_saved_prices_the_saved_bytes() {
        // P2PKH (25 bytes) against P2SH (23 bytes) at 1000 duffs/kB.
        assert_eq!(core_output_script_fee_saved_duffs(25, 23), 2);
        assert_eq!(core_output_script_fee_saved_duffs(23, 25), 0);
    }

    #[test]
    fn shield_from_balance_headroom_is_the_planner_fee_reserve() {
        let platform_version = PlatformVersion::latest();
        let base_fee = shielded_fee_for_actions(2, platform_version).expect("known version");
        let headroom = shield_from_balance_fee_headroom(platform_version);

        // The planner keeps twice the two-action minimum fee on its fee-paying
        // input; reserving less offers a Max the planner refuses.
        assert_eq!(
            headroom,
            platform_wallet::shield_fee_reserve_credits(platform_version).expect("known version")
        );
        assert_eq!(headroom, base_fee * 2);
        assert!(headroom > PlatformFeeEstimator::latest().estimate_credit_transfer());
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
        use dash_sdk::dpp::secp256k1::{PublicKey as SecpPublicKey, SecretKey};

        let sk = SecretKey::from_secret_bytes([1u8; 32]).expect("valid secret key");
        let pubkey = PublicKey::from_slice(&SecpPublicKey::from_secret_key(&sk).serialize())
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
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            500,
            None,
            max_platform_inputs(PlatformVersion::latest()),
            |_| 100,
        );

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
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            500,
            None,
            max_platform_inputs(PlatformVersion::latest()),
            |inputs| inputs.len() as u64 * 10,
        );

        assert_eq!(result.estimated_fee, 20, "fee converged for two inputs");
        assert_eq!(result.inputs.len(), 2);
        assert_eq!(result.inputs.values().sum::<u64>(), 500);
        assert_eq!(result.shortfall, 0);
    }

    #[test]
    fn allocate_reports_shortfall_when_underfunded() {
        let addresses = addrs(&[(1, 100)]);
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            500,
            None,
            max_platform_inputs(PlatformVersion::latest()),
            |_| 50,
        );

        // 100 balance, 50 reserved for fee → only 50 allocatable against a 500 ask.
        assert_eq!(result.estimated_fee, 50);
        assert_eq!(result.inputs.get(&pa(1)).copied(), Some(50));
        assert_eq!(result.shortfall, 450);
    }

    #[test]
    fn allocate_picks_highest_balance_as_fee_payer_and_excludes_destination() {
        let addresses = addrs(&[(1, 100), (2, 1000), (9, 5000)]);
        let destination = pa(9);
        let result = allocate_platform_addresses_with_fee(
            &addresses,
            200,
            Some(&destination),
            max_platform_inputs(PlatformVersion::latest()),
            |_| 30,
        );

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
        let result = allocate_platform_addresses_with_fee(
            &[],
            500,
            None,
            max_platform_inputs(PlatformVersion::latest()),
            |_| 10,
        );

        assert!(result.inputs.is_empty());
        assert!(result.sorted_addresses.is_empty());
        assert_eq!(result.shortfall, 500);
        assert_eq!(result.estimated_fee, 10);
    }

    #[test]
    fn allocate_with_estimator_uses_worst_case_platform_fee() {
        let estimator = PlatformFeeEstimator::latest();
        let addresses = addrs(&[(1, 10_000_000_000)]);
        let result = allocate_platform_addresses(&estimator, &addresses, 1_000_000, None);

        assert_eq!(
            result.estimated_fee,
            estimate_platform_fee(&estimator, 1, 1)
        );
        assert_eq!(result.shortfall, 0);
        assert_eq!(result.fee_payer_index, 0);
        assert_eq!(result.inputs.get(&pa(1)).copied(), Some(1_000_000));
    }

    #[test]
    fn platform_fee_accounts_for_every_output() {
        let estimator = PlatformFeeEstimator::latest();
        let one_output = estimate_platform_fee(&estimator, 1, 1);
        let two_outputs = estimate_platform_fee(&estimator, 1, 2);

        assert!(two_outputs > one_output);
    }
}
