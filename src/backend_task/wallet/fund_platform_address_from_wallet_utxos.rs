use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::AddressFundsFeeStrategy;
use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::platform::transition::top_up_address::TopUpAddress;
use platform_wallet::AssetLockFundingType;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Output map for a Platform address top-up: each entry maps a recipient to an
/// explicit credit amount, or `None` to absorb the remainder after fees.
type FundingOutputs = BTreeMap<PlatformAddress, Option<u64>>;

/// Build the output map and fee strategy for the fee-from-wallet funding branch.
///
/// The `destination` receives exactly `amount` converted to credits, and a
/// separate `change_address` absorbs the asset-lock surplus minus the Platform
/// fee. The returned `ReduceOutput` step indexes the change output's position in
/// the (sorted) map, so the fee is always deducted from change, never from the
/// destination amount.
fn build_fee_from_wallet_outputs(
    amount: u64,
    destination: PlatformAddress,
    change_address: PlatformAddress,
) -> Result<(FundingOutputs, AddressFundsFeeStrategy), TaskError> {
    let amount_credits =
        amount
            .checked_mul(CREDITS_PER_DUFF)
            .ok_or(TaskError::CreditCalculationOverflow {
                amount,
                credits_per_duff: CREDITS_PER_DUFF,
            })?;

    let mut outputs: FundingOutputs = BTreeMap::new();
    outputs.insert(destination, Some(amount_credits));
    outputs.insert(change_address, None);

    let change_index = outputs.keys().position(|k| *k == change_address).ok_or(
        TaskError::ChangeAddressUnavailable {
            reason: "change address not found in outputs map",
        },
    )? as u16;

    Ok((
        outputs,
        vec![AddressFundsFeeStrategyStep::ReduceOutput(change_index)],
    ))
}

impl AppContext {
    /// Fund a Platform (DIP-17) address directly from wallet UTXOs.
    ///
    /// Branches on upstream pool membership. When the destination is already
    /// revealed in this wallet's upstream platform-payment pool — the exact
    /// recipient set the orchestrator's pre-flight accepts — the funding routes
    /// through the orchestrated `fund_from_asset_lock` pipeline (build/broadcast
    /// the lock, CL-height retry, InstantSend → ChainLock fallback, consume on
    /// acceptance). Any other destination — an advanced footgun users are
    /// trusted to take — keeps the manual asset-lock + `TopUpAddress` path.
    ///
    /// The fee-from-wallet case needs a second (change) recipient, and the
    /// orchestrator's pre-flight requires *every* recipient to be in-pool. So it
    /// reaches the orchestrator only when a distinct in-pool, watched
    /// platform-payment address is available to absorb the change; otherwise it
    /// falls back to the manual path (which derives a fresh change address).
    pub(crate) async fn fund_platform_address_from_wallet_utxos(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: PlatformAddress,
        fee_deduct_from_output: bool,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let backend = self.wallet_backend()?;
        let destination_in_pool = backend
            .platform_address_in_pool(&seed_hash, &destination)
            .await?;

        if destination_in_pool {
            if fee_deduct_from_output {
                // Fee-from-output: a single in-pool destination is the lone
                // `None` recipient, satisfying the orchestrator's one-`None`
                // invariant with no change recipient.
                return self
                    .fund_platform_address_from_wallet_utxos_orchestrated(
                        seed_hash,
                        amount,
                        destination,
                    )
                    .await;
            }

            // Fee-from-wallet: the orchestrator accepts this only when the
            // change recipient is also in-pool. Source one from the wallet's
            // own watched platform-payment addresses; if found, route through
            // the orchestrator for the same recovery guarantees.
            if let Some(change) = self
                .select_in_pool_platform_change(&seed_hash, &destination)
                .await?
            {
                return self
                    .fund_platform_address_from_wallet_utxos_orchestrated_with_change(
                        seed_hash,
                        amount,
                        destination,
                        change,
                    )
                    .await;
            }
        }

        self.fund_platform_address_from_wallet_utxos_manual(
            seed_hash,
            amount,
            destination,
            fee_deduct_from_output,
        )
        .await
    }

    /// Find a watched platform-payment address (distinct from `destination`)
    /// that is already in this wallet's upstream pool, to absorb fee-from-wallet
    /// change.
    ///
    /// Candidates are the wallet's watched platform-payment addresses — every
    /// one is inside DET's synced provider window, so its change credits are
    /// visible and spendable (the funds-safety invariant from `0a64be55`). Each
    /// is gated through [`WalletBackend::platform_address_in_pool`], the exact
    /// membership check the orchestrator's pre-flight runs, so a returned
    /// address is guaranteed to pass `validate_recipient_addresses`. No reveal
    /// or pool advance happens — only already-revealed addresses are inspected.
    /// `None` means no distinct in-pool candidate exists; the caller falls back
    /// to the manual path.
    async fn select_in_pool_platform_change(
        self: &Arc<Self>,
        seed_hash: &WalletSeedHash,
        destination: &PlatformAddress,
    ) -> Result<Option<PlatformAddress>, TaskError> {
        let candidates: Vec<PlatformAddress> = {
            let wallet_arc = self.wallet_arc(seed_hash)?;
            let wallet = wallet_arc.read()?;
            wallet
                .platform_addresses(self.network)
                .into_iter()
                .map(|(_, platform_address)| platform_address)
                .collect()
        };

        // This loop owns ordering and the async in-pool membership gate: it walks
        // candidates in order and returns the first that is both distinct from the
        // destination and confirmed in-pool. The orchestrator needs two distinct
        // recipients, so the destination can never be its own change.
        let backend = self.wallet_backend()?;
        for candidate in candidates {
            if &candidate == destination {
                continue;
            }
            if backend
                .platform_address_in_pool(seed_hash, &candidate)
                .await?
            {
                return Ok(Some(candidate));
            }
        }

        Ok(None)
    }

    /// Orchestrated wallet-UTXO funding for a wallet-owned destination: the
    /// upstream `fund_from_asset_lock` builds and broadcasts the asset lock,
    /// submits the address-funding transition with CL-height retry and IS→CL
    /// fallback, then consumes the lock on acceptance.
    async fn fund_platform_address_from_wallet_utxos_orchestrated(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: PlatformAddress,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use crate::wallet_backend::PlatformPathIndex;
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        let wallet_arc = self.wallet_arc(&seed_hash)?;
        let network = self.network;

        let mut outputs: FundingOutputs = BTreeMap::new();
        outputs.insert(destination, None);
        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        let path_index = {
            let wallet = wallet_arc.read()?;
            PlatformPathIndex::from_wallet(&wallet, network)
        };

        let backend = self.wallet_backend()?;
        backend
            .fund_platform_address(
                &seed_hash,
                AssetLockFunding::FromWalletBalance {
                    amount_duffs: amount,
                    account_index: 0,
                },
                0,
                outputs,
                fee_strategy,
                &path_index,
                None,
            )
            .await?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }

    /// Orchestrated fee-from-wallet funding for a wallet-owned destination with
    /// an in-pool change recipient.
    ///
    /// The `destination` receives exactly `amount` credits; the `change`
    /// recipient (the lone `None` entry) absorbs the asset-lock surplus, from
    /// which the Platform fee is deducted via `ReduceOutput`. Both recipients are
    /// in-pool, so the orchestrator's pre-flight accepts the map and the full
    /// recovery pipeline (CL-height retry, IS→CL fallback, consume-on-accept)
    /// applies. The lock is sized to cover `amount` plus the estimated fee so the
    /// surplus the change absorbs is exactly the fee budget.
    async fn fund_platform_address_from_wallet_utxos_orchestrated_with_change(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: PlatformAddress,
        change: PlatformAddress,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use crate::wallet_backend::PlatformPathIndex;
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        let wallet_arc = self.wallet_arc(&seed_hash)?;
        let network = self.network;

        let (outputs, fee_strategy) = build_fee_from_wallet_outputs(amount, destination, change)?;

        // The lock must also cover the Platform fee, since fees are paid from the
        // wallet (the surplus the change output absorbs), not from `amount`.
        let estimated_platform_fee_duffs = self
            .fee_estimator()
            .estimate_address_funding_from_asset_lock_duffs(2);
        let asset_lock_amount = amount.saturating_add(estimated_platform_fee_duffs);

        let path_index = {
            let wallet = wallet_arc.read()?;
            PlatformPathIndex::from_wallet(&wallet, network)
        };

        let backend = self.wallet_backend()?;
        backend
            .fund_platform_address(
                &seed_hash,
                AssetLockFunding::FromWalletBalance {
                    amount_duffs: asset_lock_amount,
                    account_index: 0,
                },
                0,
                outputs,
                fee_strategy,
                &path_index,
                None,
            )
            .await?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }

    /// Manual wallet-UTXO funding: create the asset lock, then submit the
    /// `TopUpAddress` transition directly. The fallback path — used for non-owned
    /// destinations, and for the fee-from-wallet case only when no distinct
    /// in-pool change address is available (it then derives a fresh one). A
    /// submit failure propagates via `?`, so the flow never reports a false
    /// success.
    async fn fund_platform_address_from_wallet_utxos_manual(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: PlatformAddress,
        fee_deduct_from_output: bool,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex};

        // When fees are paid from the wallet (not the output), the asset lock
        // must be large enough to also cover the estimated Platform fee.
        let asset_lock_amount = if fee_deduct_from_output {
            amount
        } else {
            let estimated_platform_fee_duffs = self
                .fee_estimator()
                .estimate_address_funding_from_asset_lock_duffs(2);
            amount.saturating_add(estimated_platform_fee_duffs)
        };

        let backend = self.wallet_backend()?;
        let (asset_lock_proof, asset_lock_private_key, _tx_id) = backend
            .create_asset_lock_proof(
                &seed_hash,
                asset_lock_amount,
                AssetLockFundingType::AssetLockAddressTopUp,
                0,
            )
            .await?;

        let wallet_arc = self.wallet_arc(&seed_hash)?;
        let sdk = backend.sdk().clone();

        // Derive the change address, build the outputs, and sign — all inside
        // one held-seed scope so the operation prompts at most once and the
        // seed zeroizes on return. The change must be a watched DIP-17
        // platform-payment address (so its credits are synced and spendable),
        // not a BIP-44 change address — hence it is derived and registered via
        // the platform-payment path, then the signer index is rebuilt to cover
        // it.
        let network = self.network;
        backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;

                    let (outputs, fee_strategy) =
                        if fee_deduct_from_output {
                            let mut outputs: FundingOutputs = BTreeMap::new();
                            outputs.insert(destination, None);
                            (outputs, vec![AddressFundsFeeStrategyStep::ReduceOutput(0)])
                        } else {
                            let change_core_addr = {
                                let mut wallet_w = wallet_arc.write()?;
                                wallet_w
                                    .generate_platform_receive_address_with_seed(seed, network)
                                    .map_err(|_| TaskError::WalletPlatformReceiveAddressFailed)?
                            };
                            let change_address = PlatformAddress::try_from(change_core_addr)
                                .map_err(|e| TaskError::AddressConversionFailed {
                                    source: Box::new(e),
                                })?;
                            build_fee_from_wallet_outputs(amount, destination, change_address)?
                        };

                    let path_index = {
                        let wallet = wallet_arc.read()?;
                        PlatformPathIndex::from_wallet(&wallet, network)
                    };
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    outputs
                        .top_up(
                            &sdk,
                            asset_lock_proof,
                            asset_lock_private_key,
                            fee_strategy,
                            &signer,
                            None,
                        )
                        .await
                        .map_err(TaskError::from)
                },
            )
            .await?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p2pkh(byte: u8) -> PlatformAddress {
        PlatformAddress::P2pkh([byte; 20])
    }

    /// The destination receives exactly `amount * CREDITS_PER_DUFF`, and the
    /// `ReduceOutput` index resolves to the change output — never the
    /// destination — regardless of how the addresses sort in the map.
    fn assert_outputs(destination: PlatformAddress, change: PlatformAddress) {
        let amount = 12_345u64;
        let (outputs, fee_strategy) =
            build_fee_from_wallet_outputs(amount, destination, change).expect("builds outputs");

        assert_eq!(
            outputs.get(&destination).copied(),
            Some(Some(amount * CREDITS_PER_DUFF)),
            "destination receives the exact credit amount",
        );
        assert_eq!(
            outputs.get(&change).copied(),
            Some(None),
            "change output absorbs the remainder",
        );

        assert_eq!(fee_strategy.len(), 1);
        let AddressFundsFeeStrategyStep::ReduceOutput(index) = fee_strategy[0] else {
            panic!("fee-from-wallet branch reduces an output");
        };
        let reduced = outputs
            .keys()
            .nth(index as usize)
            .expect("fee index points at an output");
        assert_eq!(
            *reduced, change,
            "the fee is deducted from the change output"
        );
    }

    #[test]
    fn fee_index_resolves_to_change_when_change_sorts_first() {
        // change (0x01) sorts before destination (0x02)
        assert_outputs(p2pkh(0x02), p2pkh(0x01));
    }

    #[test]
    fn fee_index_resolves_to_change_when_change_sorts_last() {
        // change (0x02) sorts after destination (0x01)
        assert_outputs(p2pkh(0x01), p2pkh(0x02));
    }

    #[test]
    fn amount_overflow_is_rejected() {
        let err = build_fee_from_wallet_outputs(u64::MAX, p2pkh(0x01), p2pkh(0x02))
            .expect_err("overflows");
        assert!(matches!(err, TaskError::CreditCalculationOverflow { .. }));
    }
}
