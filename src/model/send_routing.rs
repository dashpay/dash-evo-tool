use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::fee_estimation::{PlatformFeeEstimator, format_credits_as_dash};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::version::PlatformVersion;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use super::address::ValidatedAddress;

/// Maximum number of platform address inputs allowed per state transition.
pub const MAX_PLATFORM_INPUTS: usize = 16;

/// The funding source for a send operation.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SendSource {
    /// Core wallet UTXOs. Balance is the confirmed spendable balance in duffs.
    CoreWallet {
        wallet: Arc<RwLock<Wallet>>,
        balance_duffs: u64,
        seed_hash: WalletSeedHash,
    },
    /// Platform L2 addresses with their balances (in credits).
    PlatformAddresses {
        seed_hash: WalletSeedHash,
        addresses: Vec<(PlatformAddress, Address, u64)>,
    },
    /// An identity's credit balance.
    Identity {
        qualified_identity: QualifiedIdentity,
    },
    /// Shielded pool balance (in credits).
    Shielded {
        seed_hash: WalletSeedHash,
        balance_credits: u64,
    },
}

/// Complete send request with all parameters needed for routing.
pub struct SendRequest {
    /// Funding source.
    pub source: SendSource,
    /// Validated destination address.
    pub destination: ValidatedAddress,
    /// Amount in the source's native unit (duffs for Core, credits for others).
    pub amount: u64,
    /// Pre-resolved destination identity (required for *-to-Identity routes).
    pub resolved_identity: Option<QualifiedIdentity>,
    /// Fee context for Platform allocation routes.
    pub fee_context: Option<FeeContext>,
}

/// Fee estimation context for routes that need platform address allocation.
#[allow(clippy::large_enum_variant)]
pub enum FeeContext {
    /// For Platform-to-Platform and Platform-to-Identity: fee estimator with multiplier.
    PlatformTransfer(PlatformFeeEstimator),
    /// For Platform-to-Core: withdrawal fee via state transition estimation.
    Withdrawal {
        output_script: CoreScript,
        platform_version: &'static PlatformVersion,
    },
    /// For Platform-to-Shielded: per-operation fee in credits.
    Shield { per_op_fee_credits: u64 },
}

/// Result of a successful routing decision.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SendResult {
    /// A single backend task to execute.
    Single(BackendTask),
    /// Multiple tasks to execute sequentially (e.g., Platform-to-Shielded multi-address).
    Sequential(Vec<BackendTask>),
}

/// Errors from the send routing function.
#[derive(Debug, thiserror::Error)]
pub enum SendRoutingError {
    #[error("Enter an amount greater than zero.")]
    ZeroAmount,

    #[error("Not enough funds. Your available balance is {available_formatted}.")]
    InsufficientBalance {
        available: u64,
        required: u64,
        available_formatted: String,
    },

    #[error(
        "Direct transfers from an identity to a private address are not available yet. \
         You can withdraw to a Platform address first, then transfer to a private address from there."
    )]
    IdentityToShieldedUnsupported,

    #[error(
        "Direct transfers from a private address to an identity are not available yet. \
         You can transfer to a Platform address first, then top up the identity from there."
    )]
    ShieldedToIdentityUnsupported,

    #[error("You cannot send to the same address or identity you are sending from.")]
    SelfSend,

    #[error("The destination identity could not be found. Check the ID and try again.")]
    IdentityNotResolved,

    #[error(
        "Requested amount exceeds the maximum for a single transaction. \
         Maximum sendable is {max_formatted}. Try reducing the amount."
    )]
    TooManyInputs {
        max_sendable: u64,
        max_formatted: String,
    },

    #[error("Insufficient balance after fees. Available after fees: {available_formatted}.")]
    InsufficientAfterFees {
        available_after_fees: u64,
        available_formatted: String,
    },

    #[error("No fee context provided for this route. This is a programming error.")]
    MissingFeeContext,

    #[error("Invalid shielded address. Check the address and try again.")]
    InvalidShieldedAddress,
}

/// Result of allocating platform addresses for a transfer.
#[derive(Debug, Clone)]
pub struct AddressAllocationResult {
    /// Map of platform address to amount to transfer from each.
    pub inputs: BTreeMap<PlatformAddress, u64>,
    /// Index of the fee payer in BTreeMap iteration order.
    pub fee_payer_index: u16,
    /// Estimated fee for this transaction.
    pub estimated_fee: u64,
    /// Amount that couldn't be covered (0 if fully covered).
    pub shortfall: u64,
    /// Addresses sorted by balance descending (for UI display).
    pub sorted_addresses: Vec<(PlatformAddress, Address, u64)>,
}

/// Allocates platform addresses for a transfer, using a custom fee calculator.
///
/// Iterates until the fee estimate stabilizes (input count affects fee).
pub fn allocate_platform_addresses_with_fee<F>(
    addresses: &[(PlatformAddress, Address, u64)],
    amount_credits: u64,
    destination: Option<&PlatformAddress>,
    fee_for_inputs: F,
) -> AddressAllocationResult
where
    F: Fn(&BTreeMap<PlatformAddress, u64>) -> u64,
{
    let filtered: Vec<_> = addresses
        .iter()
        .filter(|(platform_addr, _, _)| destination != Some(platform_addr))
        .cloned()
        .collect();

    let mut sorted_addresses = filtered;
    sorted_addresses.sort_by(|a, b| b.2.cmp(&a.2));

    if sorted_addresses.is_empty() {
        return AddressAllocationResult {
            inputs: BTreeMap::new(),
            fee_payer_index: 0,
            estimated_fee: fee_for_inputs(&BTreeMap::new()),
            shortfall: amount_credits,
            sorted_addresses: vec![],
        };
    }

    let fee_payer_addr = sorted_addresses.first().map(|(addr, _, _)| *addr);

    let mut estimated_fee = fee_for_inputs(&BTreeMap::new());
    let mut inputs: BTreeMap<PlatformAddress, u64> = BTreeMap::new();

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

    let total_allocated: u64 = inputs.values().sum();
    let allocation_shortfall = amount_credits.saturating_sub(total_allocated);

    let fee_deficit = if let Some(fee_payer) = fee_payer_addr {
        let fee_payer_balance = sorted_addresses.first().map(|(_, _, b)| *b).unwrap_or(0);
        let fee_payer_contribution = inputs.get(&fee_payer).copied().unwrap_or(0);
        let fee_payer_remaining = fee_payer_balance.saturating_sub(fee_payer_contribution);
        estimated_fee.saturating_sub(fee_payer_remaining)
    } else {
        estimated_fee
    };

    let shortfall = allocation_shortfall.saturating_add(fee_deficit);

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

/// Allocates platform addresses for a transfer using the standard platform fee estimator.
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
        estimate_platform_fee(estimator, max_inputs.max(1))
    })
}

/// Estimated serialized bytes per input (address + signature/witness data).
const ESTIMATED_BYTES_PER_INPUT: usize = 225;

/// Calculate the estimated fee for a platform address funds transfer.
pub fn estimate_platform_fee(estimator: &PlatformFeeEstimator, input_count: usize) -> u64 {
    let inputs = input_count.max(1);
    let base_fee = estimator.estimate_address_funds_transfer(inputs, 1);
    let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT;
    let storage_fee = estimator.estimate_storage_based_fee(estimated_bytes, inputs);
    let total = base_fee.saturating_add(storage_fee);
    total.saturating_mul(120) / 100
}

/// Calculate the estimated fee for a Platform address withdrawal using a
/// constructed state transition.
pub fn estimate_withdrawal_fee(
    platform_version: &PlatformVersion,
    inputs: &BTreeMap<PlatformAddress, u64>,
    output_script: &CoreScript,
) -> u64 {
    use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
    use dash_sdk::dpp::fee::Credits;
    use dash_sdk::dpp::state_transition::StateTransitionEstimatedFeeValidation;
    use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::AddressCreditWithdrawalTransition;
    use dash_sdk::dpp::state_transition::address_credit_withdrawal_transition::v0::AddressCreditWithdrawalTransitionV0;
    use dash_sdk::dpp::withdrawal::Pooling;

    type AddressNonce = u32;

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

/// Route a send request to the correct `BackendTask` variant(s).
///
/// This function is pure and synchronous — no network calls, no async, no side effects.
/// The caller is responsible for:
/// - Resolving destination identities (populate `resolved_identity`)
/// - Providing fee context for Platform-source routes (populate `fee_context`)
/// - Converting amounts to the correct unit before calling
pub fn resolve_send(request: SendRequest) -> Result<SendResult, SendRoutingError> {
    let SendRequest {
        source,
        destination,
        amount,
        resolved_identity,
        fee_context,
    } = request;

    if amount == 0 {
        return Err(SendRoutingError::ZeroAmount);
    }

    match (source, destination) {
        // =====================================================================
        // Core Wallet source (amounts in duffs)
        // =====================================================================
        (
            SendSource::CoreWallet {
                wallet,
                balance_duffs,
                ..
            },
            ValidatedAddress::Core(addr),
        ) => {
            check_balance(balance_duffs, amount, true)?;

            let recipient = PaymentRecipient {
                address: addr.to_string(),
                amount_duffs: amount,
            };

            Ok(SendResult::Single(BackendTask::CoreTask(
                CoreTask::SendWalletPayment {
                    wallet,
                    request: WalletPaymentRequest {
                        recipients: vec![recipient],
                        subtract_fee_from_amount: false,
                        memo: None,
                        override_fee: None,
                    },
                },
            )))
        }

        (
            SendSource::CoreWallet {
                balance_duffs,
                seed_hash,
                ..
            },
            ValidatedAddress::Platform {
                address: destination,
                ..
            },
        ) => {
            check_balance(balance_duffs, amount, true)?;

            Ok(SendResult::Single(BackendTask::WalletTask(
                WalletTask::FundPlatformAddressFromWalletUtxos {
                    seed_hash,
                    amount,
                    destination,
                    fee_deduct_from_output: true,
                },
            )))
        }

        (
            SendSource::CoreWallet {
                balance_duffs,
                seed_hash,
                ..
            },
            ValidatedAddress::Shielded(_),
        ) => {
            check_balance(balance_duffs, amount, true)?;

            Ok(SendResult::Single(BackendTask::ShieldedTask(
                ShieldedTask::ShieldFromAssetLock {
                    seed_hash,
                    amount_duffs: amount,
                    source_address: None,
                },
            )))
        }

        (
            SendSource::CoreWallet {
                wallet,
                balance_duffs,
                ..
            },
            ValidatedAddress::Identity { .. },
        ) => {
            check_balance(balance_duffs, amount, true)?;

            let qi = resolved_identity.ok_or(SendRoutingError::IdentityNotResolved)?;
            let identity_index = qi.wallet_index.unwrap_or(0);
            let top_up_index = qi.top_ups.len() as u32;

            Ok(SendResult::Single(BackendTask::IdentityTask(
                IdentityTask::TopUpIdentity(IdentityTopUpInfo {
                    qualified_identity: qi,
                    wallet,
                    identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                        amount,
                        identity_index,
                        top_up_index,
                    ),
                }),
            )))
        }

        // =====================================================================
        // Platform Addresses source (amounts in credits)
        // =====================================================================
        (
            SendSource::PlatformAddresses {
                seed_hash,
                addresses,
            },
            ValidatedAddress::Platform {
                address: dest_addr, ..
            },
        ) => {
            let total_balance: u64 = addresses.iter().map(|(_, _, b)| *b).sum();
            check_balance(total_balance, amount, false)?;

            let estimator = match fee_context {
                Some(FeeContext::PlatformTransfer(est)) => est,
                _ => return Err(SendRoutingError::MissingFeeContext),
            };

            let allocation =
                allocate_platform_addresses(&estimator, &addresses, amount, Some(&dest_addr));

            if allocation.sorted_addresses.is_empty() {
                return Err(SendRoutingError::SelfSend);
            }

            if allocation.shortfall > 0 {
                return platform_shortfall_error(&allocation, amount);
            }

            let mut outputs = BTreeMap::new();
            outputs.insert(dest_addr, amount);

            Ok(SendResult::Single(BackendTask::WalletTask(
                WalletTask::TransferPlatformCredits {
                    seed_hash,
                    inputs: allocation.inputs,
                    outputs,
                    fee_payer_index: allocation.fee_payer_index,
                },
            )))
        }

        (
            SendSource::PlatformAddresses {
                seed_hash,
                addresses,
            },
            ValidatedAddress::Core(addr),
        ) => {
            let total_balance: u64 = addresses.iter().map(|(_, _, b)| *b).sum();
            check_balance(total_balance, amount, false)?;

            let (output_script, platform_version) = match fee_context {
                Some(FeeContext::Withdrawal {
                    output_script,
                    platform_version,
                }) => (output_script, platform_version),
                _ => {
                    let script = CoreScript::new(addr.script_pubkey());
                    let pv = PlatformVersion::latest();
                    (script, pv)
                }
            };

            let allocation =
                allocate_platform_addresses_with_fee(&addresses, amount, None, |inputs| {
                    estimate_withdrawal_fee(platform_version, inputs, &output_script)
                });

            if allocation.shortfall > 0 {
                return platform_shortfall_error(&allocation, amount);
            }

            Ok(SendResult::Single(BackendTask::WalletTask(
                WalletTask::WithdrawFromPlatformAddress {
                    seed_hash,
                    inputs: allocation.inputs,
                    output_script,
                    core_fee_per_byte: 1,
                    fee_payer_index: allocation.fee_payer_index,
                },
            )))
        }

        (
            SendSource::PlatformAddresses {
                seed_hash,
                addresses,
            },
            ValidatedAddress::Shielded(_),
        ) => {
            let total_available: u64 = addresses.iter().map(|(_, _, b)| *b).sum();
            check_balance(total_available, amount, false)?;

            let per_op_fee = match fee_context {
                Some(FeeContext::Shield { per_op_fee_credits }) => per_op_fee_credits,
                _ => return Err(SendRoutingError::MissingFeeContext),
            };

            let mut sorted_addrs = addresses;
            sorted_addrs.sort_by(|a, b| b.2.cmp(&a.2));

            let mut remaining = amount;
            let mut tasks: Vec<BackendTask> = Vec::new();

            for (platform_addr, _, balance) in &sorted_addrs {
                if remaining == 0 {
                    break;
                }
                let available = balance.saturating_sub(per_op_fee);
                if available == 0 {
                    continue;
                }
                let spend = remaining.min(available);
                tasks.push(BackendTask::ShieldedTask(ShieldedTask::ShieldCredits {
                    seed_hash,
                    amount: spend,
                    from_address: *platform_addr,
                    nonce_override: None,
                }));
                remaining -= spend;
            }

            if tasks.is_empty() || remaining > 0 {
                let max_sendable = amount.saturating_sub(remaining);
                return Err(SendRoutingError::InsufficientAfterFees {
                    available_after_fees: max_sendable,
                    available_formatted: format_credits_as_dash(max_sendable),
                });
            }

            if tasks.len() == 1 {
                Ok(SendResult::Single(tasks.into_iter().next().unwrap()))
            } else {
                Ok(SendResult::Sequential(tasks))
            }
        }

        (
            SendSource::PlatformAddresses {
                seed_hash,
                addresses,
            },
            ValidatedAddress::Identity { .. },
        ) => {
            let total_balance: u64 = addresses.iter().map(|(_, _, b)| *b).sum();
            check_balance(total_balance, amount, false)?;

            let qi = resolved_identity.ok_or(SendRoutingError::IdentityNotResolved)?;

            let estimator = match fee_context {
                Some(FeeContext::PlatformTransfer(est)) => est,
                _ => return Err(SendRoutingError::MissingFeeContext),
            };

            let allocation = allocate_platform_addresses(&estimator, &addresses, amount, None);

            if allocation.shortfall > 0 {
                return platform_shortfall_error(&allocation, amount);
            }

            Ok(SendResult::Single(BackendTask::IdentityTask(
                IdentityTask::TopUpIdentityFromPlatformAddresses {
                    identity: qi,
                    inputs: allocation.inputs,
                    wallet_seed_hash: seed_hash,
                },
            )))
        }

        // =====================================================================
        // Identity source (amounts in credits)
        // =====================================================================
        (SendSource::Identity { qualified_identity }, ValidatedAddress::Core(addr)) => {
            let balance = qualified_identity.identity.balance();
            check_balance(balance, amount, false)?;

            Ok(SendResult::Single(BackendTask::IdentityTask(
                IdentityTask::WithdrawFromIdentity(qualified_identity, Some(addr), amount, None),
            )))
        }

        (
            SendSource::Identity { qualified_identity },
            ValidatedAddress::Platform {
                address: dest_addr, ..
            },
        ) => {
            let balance = qualified_identity.identity.balance();
            check_balance(balance, amount, false)?;

            let mut outputs = BTreeMap::new();
            outputs.insert(dest_addr, amount);

            Ok(SendResult::Single(BackendTask::IdentityTask(
                IdentityTask::TransferToAddresses {
                    identity: qualified_identity,
                    outputs,
                    key_id: None,
                },
            )))
        }

        (
            SendSource::Identity { qualified_identity },
            ValidatedAddress::Identity { id: to_id, .. },
        ) => {
            let balance = qualified_identity.identity.balance();
            check_balance(balance, amount, false)?;

            if to_id == qualified_identity.identity.id() {
                return Err(SendRoutingError::SelfSend);
            }

            Ok(SendResult::Single(BackendTask::IdentityTask(
                IdentityTask::Transfer(qualified_identity, to_id, amount, None),
            )))
        }

        (SendSource::Identity { .. }, ValidatedAddress::Shielded(_)) => {
            Err(SendRoutingError::IdentityToShieldedUnsupported)
        }

        // =====================================================================
        // Shielded source (amounts in credits)
        // =====================================================================
        (
            SendSource::Shielded {
                seed_hash,
                balance_credits,
            },
            ValidatedAddress::Shielded(addr_string),
        ) => {
            check_balance(balance_credits, amount, false)?;

            let (orchard_addr, _) =
                dash_sdk::dpp::address_funds::OrchardAddress::from_bech32m_string(&addr_string)
                    .map_err(|_| SendRoutingError::InvalidShieldedAddress)?;

            Ok(SendResult::Single(BackendTask::ShieldedTask(
                ShieldedTask::ShieldedTransfer {
                    seed_hash,
                    amount,
                    recipient_address_bytes: orchard_addr.to_raw_bytes().to_vec(),
                },
            )))
        }

        (
            SendSource::Shielded {
                seed_hash,
                balance_credits,
            },
            ValidatedAddress::Platform {
                address: dest_addr, ..
            },
        ) => {
            check_balance(balance_credits, amount, false)?;

            Ok(SendResult::Single(BackendTask::ShieldedTask(
                ShieldedTask::UnshieldCredits {
                    seed_hash,
                    amount,
                    to_platform_address: dest_addr,
                },
            )))
        }

        (
            SendSource::Shielded {
                seed_hash,
                balance_credits,
            },
            ValidatedAddress::Core(addr),
        ) => {
            check_balance(balance_credits, amount, false)?;

            Ok(SendResult::Single(BackendTask::ShieldedTask(
                ShieldedTask::ShieldedWithdrawal {
                    seed_hash,
                    amount,
                    to_core_address: addr,
                },
            )))
        }

        (SendSource::Shielded { .. }, ValidatedAddress::Identity { .. }) => {
            Err(SendRoutingError::ShieldedToIdentityUnsupported)
        }
    }
}

/// Check that the available balance covers the required amount.
fn check_balance(available: u64, required: u64, is_duffs: bool) -> Result<(), SendRoutingError> {
    if required > available {
        let available_formatted = if is_duffs {
            format_duffs_as_dash(available)
        } else {
            format_credits_as_dash(available)
        };
        return Err(SendRoutingError::InsufficientBalance {
            available,
            required,
            available_formatted,
        });
    }
    Ok(())
}

/// Format duffs as DASH for display.
fn format_duffs_as_dash(duffs: u64) -> String {
    let dash = duffs as f64 / 100_000_000.0;
    format!("{:.8} DASH", dash)
}

/// Produce a shortfall error for platform allocation routes.
fn platform_shortfall_error(
    allocation: &AddressAllocationResult,
    _amount: u64,
) -> Result<SendResult, SendRoutingError> {
    let addresses_available = allocation.sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
    let max_balance: u64 = allocation
        .sorted_addresses
        .iter()
        .take(MAX_PLATFORM_INPUTS)
        .map(|(_, _, b)| *b)
        .sum();
    let max_sendable = max_balance.saturating_sub(allocation.estimated_fee);

    Err(SendRoutingError::TooManyInputs {
        max_sendable,
        max_formatted: format!(
            "{} (from {} addresses, fee ~{})",
            format_credits_as_dash(max_sendable),
            addresses_available,
            format_credits_as_dash(allocation.estimated_fee),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::address::AddressKind;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::platform::Identifier;

    fn testnet_core_address() -> Address {
        use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dpp::dashcore::{PrivateKey, PublicKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let privkey = PrivateKey::new(sk, Network::Testnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    fn testnet_core_address_2() -> Address {
        use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dpp::dashcore::{PrivateKey, PublicKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let privkey = PrivateKey::new(sk, Network::Testnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    fn test_platform_address() -> PlatformAddress {
        PlatformAddress::P2pkh([1u8; 20])
    }

    fn test_platform_address_2() -> PlatformAddress {
        PlatformAddress::P2pkh([2u8; 20])
    }

    fn test_identity_id() -> Identifier {
        Identifier::random()
    }

    fn test_seed_hash() -> WalletSeedHash {
        [42u8; 32]
    }

    // =========================================================================
    // Zero amount tests (TC-SR-017..020)
    // =========================================================================
    mod zero_amount {
        use super::*;

        #[test]
        fn core_source_zero_amount() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 100_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: 0,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::ZeroAmount)));
        }

        #[test]
        fn platform_source_zero_amount() {
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(test_platform_address(), testnet_core_address(), 5_000_000)],
                },
                destination: ValidatedAddress::Platform {
                    address: test_platform_address_2(),
                    bech32m: "tdash1test".to_string(),
                },
                amount: 0,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::ZeroAmount)));
        }

        #[test]
        fn identity_source_zero_amount() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: 0,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::ZeroAmount)));
        }

        #[test]
        fn shielded_source_zero_amount() {
            let result = resolve_send(SendRequest {
                source: SendSource::Shielded {
                    seed_hash: test_seed_hash(),
                    balance_credits: 5_000_000,
                },
                destination: ValidatedAddress::Shielded("tdash1z_test".to_string()),
                amount: 0,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::ZeroAmount)));
        }
    }

    // =========================================================================
    // Unsupported routes (TC-SR-015..016)
    // =========================================================================
    mod unsupported_routes {
        use super::*;

        #[test]
        fn identity_to_shielded() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Shielded("tdash1z_test".to_string()),
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Err(SendRoutingError::IdentityToShieldedUnsupported)
            ));
        }

        #[test]
        fn shielded_to_identity() {
            let result = resolve_send(SendRequest {
                source: SendSource::Shielded {
                    seed_hash: test_seed_hash(),
                    balance_credits: 5_000_000,
                },
                destination: ValidatedAddress::Identity {
                    id: test_identity_id(),
                    dpns_name: None,
                },
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Err(SendRoutingError::ShieldedToIdentityUnsupported)
            ));
        }
    }

    // =========================================================================
    // Insufficient balance (TC-SR-021..024)
    // =========================================================================
    mod insufficient_balance {
        use super::*;

        #[test]
        fn core_to_core_insufficient() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 10_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: 50_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Err(SendRoutingError::InsufficientBalance {
                    available,
                    required,
                    ..
                }) => {
                    assert_eq!(available, 10_000);
                    assert_eq!(required, 50_000);
                }
                other => panic!("Expected InsufficientBalance, got {other:?}"),
            }
        }

        #[test]
        fn platform_to_platform_insufficient() {
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(test_platform_address(), testnet_core_address(), 500_000)],
                },
                destination: ValidatedAddress::Platform {
                    address: test_platform_address_2(),
                    bech32m: "tdash1test".to_string(),
                },
                amount: 2_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Err(SendRoutingError::InsufficientBalance { .. })
            ));
        }

        #[test]
        fn identity_to_core_insufficient() {
            let qi = QualifiedIdentity::new_test_identity(1_000_000);
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: 5_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Err(SendRoutingError::InsufficientBalance { .. })
            ));
        }

        #[test]
        fn identity_to_identity_insufficient() {
            let qi = QualifiedIdentity::new_test_identity(1_000);
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Identity {
                    id: test_identity_id(),
                    dpns_name: None,
                },
                amount: 5_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Err(SendRoutingError::InsufficientBalance { .. })
            ));
        }
    }

    // =========================================================================
    // Self-send prevention (TC-SR-025..026)
    // =========================================================================
    mod self_send {
        use super::*;

        #[test]
        fn identity_to_same_identity() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let self_id = qi.identity.id();
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Identity {
                    id: self_id,
                    dpns_name: None,
                },
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::SelfSend)));
        }

        #[test]
        fn platform_to_same_platform_address() {
            let addr = test_platform_address();
            let core_addr = testnet_core_address();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(addr, core_addr, 5_000_000)],
                },
                destination: ValidatedAddress::Platform {
                    address: addr,
                    bech32m: "tdash1self".to_string(),
                },
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: Some(FeeContext::PlatformTransfer(PlatformFeeEstimator::default())),
            });
            assert!(matches!(result, Err(SendRoutingError::SelfSend)));
        }
    }

    // =========================================================================
    // Happy path: Identity source routes (TC-SR-009..011)
    // =========================================================================
    mod identity_source {
        use super::*;

        #[test]
        fn identity_to_core() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let addr = testnet_core_address();
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi.clone(),
                },
                destination: ValidatedAddress::Core(addr.clone()),
                amount: 3_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::IdentityTask(
                    IdentityTask::WithdrawFromIdentity(_, Some(to_addr), credits, None),
                ))) => {
                    assert_eq!(credits, 3_000_000);
                    assert_eq!(to_addr, addr);
                }
                other => panic!("Expected WithdrawFromIdentity, got {other:?}"),
            }
        }

        #[test]
        fn identity_to_platform() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let dest = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Platform {
                    address: dest,
                    bech32m: "tdash1dest".to_string(),
                },
                amount: 4_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::IdentityTask(
                    IdentityTask::TransferToAddresses {
                        outputs, key_id, ..
                    },
                ))) => {
                    assert_eq!(*outputs.get(&dest).unwrap(), 4_000_000);
                    assert!(key_id.is_none());
                }
                other => panic!("Expected TransferToAddresses, got {other:?}"),
            }
        }

        #[test]
        fn identity_to_identity() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let to_id = test_identity_id();
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Identity {
                    id: to_id,
                    dpns_name: None,
                },
                amount: 5_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::IdentityTask(IdentityTask::Transfer(
                    _,
                    dest_id,
                    credits,
                    None,
                )))) => {
                    assert_eq!(dest_id, to_id);
                    assert_eq!(credits, 5_000_000);
                }
                other => panic!("Expected Transfer, got {other:?}"),
            }
        }
    }

    // =========================================================================
    // Happy path: Shielded source routes (TC-SR-012..014)
    // =========================================================================
    mod shielded_source {
        use super::*;

        #[test]
        fn shielded_to_platform() {
            let dest = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::Shielded {
                    seed_hash: test_seed_hash(),
                    balance_credits: 5_000_000,
                },
                destination: ValidatedAddress::Platform {
                    address: dest,
                    bech32m: "tdash1dest".to_string(),
                },
                amount: 2_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::ShieldedTask(
                    ShieldedTask::UnshieldCredits {
                        amount,
                        to_platform_address,
                        ..
                    },
                ))) => {
                    assert_eq!(amount, 2_000_000);
                    assert_eq!(to_platform_address, dest);
                }
                other => panic!("Expected UnshieldCredits, got {other:?}"),
            }
        }

        #[test]
        fn shielded_to_core() {
            let addr = testnet_core_address();
            let result = resolve_send(SendRequest {
                source: SendSource::Shielded {
                    seed_hash: test_seed_hash(),
                    balance_credits: 5_000_000,
                },
                destination: ValidatedAddress::Core(addr.clone()),
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::ShieldedTask(
                    ShieldedTask::ShieldedWithdrawal {
                        amount,
                        to_core_address,
                        ..
                    },
                ))) => {
                    assert_eq!(amount, 1_000_000);
                    assert_eq!(to_core_address, addr);
                }
                other => panic!("Expected ShieldedWithdrawal, got {other:?}"),
            }
        }

        // TC-SR-012: Shielded -> Shielded requires a valid Orchard address
        // which is hard to construct in tests. Test the error path instead.
        #[test]
        fn shielded_to_shielded_invalid_address() {
            let result = resolve_send(SendRequest {
                source: SendSource::Shielded {
                    seed_hash: test_seed_hash(),
                    balance_credits: 5_000_000,
                },
                destination: ValidatedAddress::Shielded("tdash1z_invalid".to_string()),
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Err(SendRoutingError::InvalidShieldedAddress)
            ));
        }
    }

    // =========================================================================
    // Happy path: Core source (TC-SR-001)
    // =========================================================================
    mod core_source {
        use super::*;

        #[test]
        fn core_to_core() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let addr = testnet_core_address();
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 100_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Core(addr.clone()),
                amount: 50_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::CoreTask(CoreTask::SendWalletPayment {
                    request,
                    ..
                }))) => {
                    assert_eq!(request.recipients.len(), 1);
                    assert_eq!(request.recipients[0].amount_duffs, 50_000);
                    assert_eq!(request.recipients[0].address, addr.to_string());
                }
                other => panic!("Expected SendWalletPayment, got {other:?}"),
            }
        }

        // TC-SR-027: Core -> Platform amount stays in duffs
        #[test]
        fn core_to_platform_amount_in_duffs() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let dest = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 10_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Platform {
                    address: dest,
                    bech32m: "tdash1test".to_string(),
                },
                amount: 1,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::WalletTask(
                    WalletTask::FundPlatformAddressFromWalletUtxos { amount, .. },
                ))) => {
                    assert_eq!(amount, 1); // 1 duff, NOT 1000 credits
                }
                other => panic!("Expected FundPlatformAddressFromWalletUtxos, got {other:?}"),
            }
        }

        // TC-SR-028: Core -> Shielded amount in duffs
        #[test]
        fn core_to_shielded_amount_in_duffs() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 100_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Shielded("tdash1z_test".to_string()),
                amount: 1,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::ShieldedTask(
                    ShieldedTask::ShieldFromAssetLock { amount_duffs, .. },
                ))) => {
                    assert_eq!(amount_duffs, 1);
                }
                other => panic!("Expected ShieldFromAssetLock, got {other:?}"),
            }
        }

        // TC-SR-004: Core -> Identity requires resolved_identity
        #[test]
        fn core_to_identity_missing_resolved_identity() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 100_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Identity {
                    id: test_identity_id(),
                    dpns_name: None,
                },
                amount: 50_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::IdentityNotResolved)));
        }

        // TC-SR-004: Core -> Identity happy path
        #[test]
        fn core_to_identity_happy_path() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let qi = QualifiedIdentity::new_test_identity(5_000_000);
            let id = qi.identity.id();
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 100_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Identity {
                    id,
                    dpns_name: None,
                },
                amount: 50_000,
                resolved_identity: Some(qi),
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                    info,
                )))) => {
                    assert!(matches!(
                        info.identity_funding_method,
                        TopUpIdentityFundingMethod::FundWithWallet(50_000, _, _)
                    ));
                }
                other => panic!("Expected TopUpIdentity, got {other:?}"),
            }
        }
    }

    // =========================================================================
    // Minimum amounts (TC-SR-031..032)
    // =========================================================================
    mod min_amounts {
        use super::*;

        #[test]
        fn minimum_core_to_core_1_duff() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 1,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: 1,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::CoreTask(CoreTask::SendWalletPayment {
                    request,
                    ..
                }))) => {
                    assert_eq!(request.recipients[0].amount_duffs, 1);
                }
                other => panic!("Expected SendWalletPayment with 1 duff, got {other:?}"),
            }
        }
    }

    // =========================================================================
    // Large amounts / overflow safety (TC-SR-033)
    // =========================================================================
    mod overflow_safety {
        use super::*;

        #[test]
        fn large_core_to_core_no_panic() {
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: u64::MAX,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: u64::MAX - 1,
                resolved_identity: None,
                fee_context: None,
            });
            // Should not panic; should succeed since balance >= amount
            assert!(result.is_ok());
        }

        #[test]
        fn large_identity_balance_no_panic() {
            let qi = QualifiedIdentity::new_test_identity(u64::MAX);
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Core(testnet_core_address()),
                amount: u64::MAX - 1,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(result.is_ok());
        }
    }

    // =========================================================================
    // Address detection (TC-SR-039..040)
    // =========================================================================
    mod address_detection {
        use super::*;

        #[test]
        fn validated_address_kind_returns_correct_kind() {
            let core = ValidatedAddress::Core(testnet_core_address());
            assert_eq!(core.kind(), AddressKind::Core);

            let platform = ValidatedAddress::Platform {
                address: test_platform_address(),
                bech32m: "tdash1test".to_string(),
            };
            assert_eq!(platform.kind(), AddressKind::Platform);

            let shielded = ValidatedAddress::Shielded("tdash1z_test".to_string());
            assert_eq!(shielded.kind(), AddressKind::Shielded);

            let identity = ValidatedAddress::Identity {
                id: test_identity_id(),
                dpns_name: None,
            };
            assert_eq!(identity.kind(), AddressKind::Identity);
        }

        #[test]
        fn routing_uses_validated_address_variant_not_string() {
            // A Platform address whose bech32m string contains "dash1z" — should NOT
            // route to shielded, because the ValidatedAddress is Platform.
            let wallet = Arc::new(RwLock::new(Wallet::new_test_wallet()));
            let dest = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::CoreWallet {
                    wallet,
                    balance_duffs: 100_000,
                    seed_hash: [0u8; 32],
                },
                destination: ValidatedAddress::Platform {
                    address: dest,
                    bech32m: "tdash1z_looks_like_shielded_but_is_platform".to_string(),
                },
                amount: 1_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(
                result,
                Ok(SendResult::Single(BackendTask::WalletTask(
                    WalletTask::FundPlatformAddressFromWalletUtxos { .. }
                )))
            ));
        }
    }

    // =========================================================================
    // Platform source happy paths (TC-SR-005..008) - require fee context
    // =========================================================================
    mod platform_source {
        use super::*;

        #[test]
        fn platform_to_platform_missing_fee_context() {
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(test_platform_address(), testnet_core_address(), 10_000_000)],
                },
                destination: ValidatedAddress::Platform {
                    address: test_platform_address_2(),
                    bech32m: "tdash1dest".to_string(),
                },
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            assert!(matches!(result, Err(SendRoutingError::MissingFeeContext)));
        }

        #[test]
        fn platform_to_platform_happy_path() {
            let src_addr = test_platform_address();
            let dest_addr = test_platform_address_2();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(src_addr, testnet_core_address(), 100_000_000)],
                },
                destination: ValidatedAddress::Platform {
                    address: dest_addr,
                    bech32m: "tdash1dest".to_string(),
                },
                amount: 1_000_000,
                resolved_identity: None,
                fee_context: Some(FeeContext::PlatformTransfer(PlatformFeeEstimator::default())),
            });
            match result {
                Ok(SendResult::Single(BackendTask::WalletTask(
                    WalletTask::TransferPlatformCredits { outputs, .. },
                ))) => {
                    assert_eq!(*outputs.get(&dest_addr).unwrap(), 1_000_000);
                }
                other => panic!("Expected TransferPlatformCredits, got {other:?}"),
            }
        }

        #[test]
        fn platform_to_core_happy_path() {
            let src_addr = test_platform_address();
            let dest_core = testnet_core_address();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(src_addr, testnet_core_address_2(), 10_000_000_000)],
                },
                destination: ValidatedAddress::Core(dest_core.clone()),
                amount: 5_000_000,
                resolved_identity: None,
                fee_context: Some(FeeContext::Withdrawal {
                    output_script: CoreScript::new(dest_core.script_pubkey()),
                    platform_version: PlatformVersion::latest(),
                }),
            });
            match result {
                Ok(SendResult::Single(BackendTask::WalletTask(
                    WalletTask::WithdrawFromPlatformAddress {
                        core_fee_per_byte, ..
                    },
                ))) => {
                    assert_eq!(core_fee_per_byte, 1);
                }
                other => panic!("Expected WithdrawFromPlatformAddress, got {other:?}"),
            }
        }

        #[test]
        fn platform_to_shielded_single_address() {
            let src_addr = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(src_addr, testnet_core_address(), 100_000_000)],
                },
                destination: ValidatedAddress::Shielded("tdash1z_recipient".to_string()),
                amount: 2_000_000,
                resolved_identity: None,
                fee_context: Some(FeeContext::Shield {
                    per_op_fee_credits: 500_000,
                }),
            });
            match result {
                Ok(SendResult::Single(BackendTask::ShieldedTask(
                    ShieldedTask::ShieldCredits {
                        amount,
                        from_address,
                        ..
                    },
                ))) => {
                    assert_eq!(amount, 2_000_000);
                    assert_eq!(from_address, src_addr);
                }
                other => panic!("Expected ShieldCredits, got {other:?}"),
            }
        }

        #[test]
        fn platform_to_shielded_multi_address_sequential() {
            let addr1 = test_platform_address();
            let addr2 = test_platform_address_2();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![
                        (addr1, testnet_core_address(), 2_000_000),
                        (addr2, testnet_core_address_2(), 3_000_000),
                    ],
                },
                destination: ValidatedAddress::Shielded("tdash1z_recipient".to_string()),
                amount: 4_000_000,
                resolved_identity: None,
                fee_context: Some(FeeContext::Shield {
                    per_op_fee_credits: 100_000,
                }),
            });
            match result {
                Ok(SendResult::Sequential(tasks)) => {
                    assert!(tasks.len() >= 2);
                    for task in &tasks {
                        assert!(matches!(
                            task,
                            BackendTask::ShieldedTask(ShieldedTask::ShieldCredits { .. })
                        ));
                    }
                }
                other => panic!("Expected Sequential ShieldCredits tasks, got {other:?}"),
            }
        }

        #[test]
        fn platform_to_identity_happy_path() {
            let src_addr = test_platform_address();
            let qi = QualifiedIdentity::new_test_identity(5_000_000);
            let id = qi.identity.id();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(src_addr, testnet_core_address(), 100_000_000)],
                },
                destination: ValidatedAddress::Identity {
                    id,
                    dpns_name: None,
                },
                amount: 2_000_000,
                resolved_identity: Some(qi),
                fee_context: Some(FeeContext::PlatformTransfer(PlatformFeeEstimator::default())),
            });
            match result {
                Ok(SendResult::Single(BackendTask::IdentityTask(
                    IdentityTask::TopUpIdentityFromPlatformAddresses { inputs, .. },
                ))) => {
                    assert!(!inputs.is_empty());
                }
                other => panic!("Expected TopUpIdentityFromPlatformAddresses, got {other:?}"),
            }
        }

        #[test]
        fn platform_to_identity_missing_resolved_identity() {
            let src_addr = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::PlatformAddresses {
                    seed_hash: test_seed_hash(),
                    addresses: vec![(src_addr, testnet_core_address(), 100_000_000)],
                },
                destination: ValidatedAddress::Identity {
                    id: test_identity_id(),
                    dpns_name: None,
                },
                amount: 2_000_000,
                resolved_identity: None,
                fee_context: Some(FeeContext::PlatformTransfer(PlatformFeeEstimator::default())),
            });
            assert!(matches!(result, Err(SendRoutingError::IdentityNotResolved)));
        }
    }

    // =========================================================================
    // Amount conversion verification (TC-SR-029..030)
    // =========================================================================
    mod amount_conversion {
        use super::*;

        #[test]
        fn identity_to_platform_amount_in_credits() {
            let qi = QualifiedIdentity::new_test_identity(10_000_000);
            let dest = test_platform_address();
            let result = resolve_send(SendRequest {
                source: SendSource::Identity {
                    qualified_identity: qi,
                },
                destination: ValidatedAddress::Platform {
                    address: dest,
                    bech32m: "tdash1dest".to_string(),
                },
                amount: 3_000_000,
                resolved_identity: None,
                fee_context: None,
            });
            match result {
                Ok(SendResult::Single(BackendTask::IdentityTask(
                    IdentityTask::TransferToAddresses { outputs, .. },
                ))) => {
                    assert_eq!(*outputs.get(&dest).unwrap(), 3_000_000);
                }
                other => panic!("Expected TransferToAddresses, got {other:?}"),
            }
        }
    }
}
