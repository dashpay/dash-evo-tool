//! Each frame the Send form's choices resolve into one concrete source
//! (`selected_source`) and one concrete destination (`validated_destination`).
//! The route functions in the parent module then validate and dispatch them
//! exactly as before; Auto only decides which of them is used.

use super::*;
use crate::model::address::validate_platform_address_for_network;
use crate::model::send_route::{
    AutoDestination, AutoMax, AutoSource, CandidateMax, CandidateMaxima, PayFromKind, RouteRefusal,
    resolve_auto_max, resolve_destination_auto, resolve_source_auto, route_refusal,
    ticked_source_refusal,
};
use crate::model::spend_limit::CoreSendShortfall;
use crate::model::wallet::DerivationPathHelpers;
use crate::ui::state::own_receive_addresses::OwnAddressKind;
use crate::ui::state::send_form::{PayFrom, ToMode};

/// Whether a Max figure is known, still being quoted, or failed to quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuoteState {
    Ready,
    Pending,
    Failed,
    /// Another transaction from the wallet is in progress: no Max is
    /// offered until it finishes, and nothing is asked again meanwhile.
    Busy,
    /// The wallet answered that Core can send nothing, and why.
    Shortfall(CoreSendShortfall),
}

/// What the Amount field may offer for one source and destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AmountLimits {
    /// The Max figure, in credits; `None` while nothing can be offered.
    pub max_credits: Option<u64>,
    /// The fee-reserve caption under the field.
    pub hint: Option<String>,
    pub quote: QuoteState,
    /// No amount the source can fund covers the network fee.
    pub fee_not_covered: bool,
    /// The smallest amount the route accepts, in credits.
    pub min_credits: Option<u64>,
}

impl AmountLimits {
    /// The figure Auto compares against the amount.
    fn candidate_max(&self) -> CandidateMax {
        if self.fee_not_covered {
            return CandidateMax::Ready(0);
        }
        match (self.max_credits, self.quote) {
            (Some(max), _) => CandidateMax::Ready(max),
            (None, QuoteState::Pending) => CandidateMax::Pending,
            (None, QuoteState::Failed) => CandidateMax::Failed,
            // Unknown until the other transaction finishes.
            (None, QuoteState::Busy) => CandidateMax::Withheld,
            (None, QuoteState::Ready | QuoteState::Shortfall(_)) => CandidateMax::Ready(0),
        }
    }
}

/// Where a "My wallets" or typed destination stands this frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum DestinationStatus {
    /// Nothing entered or chosen yet.
    #[default]
    Missing,
    /// The destination is known.
    Ready,
    /// A receiving address of `wallet` is being derived.
    Fetching { kind: PayFromKind },
    /// `wallet` must be unlocked before a receiving address can be derived.
    NeedsUnlock { wallet: WalletSeedHash },
    /// The To wallet holds no identity that can receive.
    NoIdentity,
}

/// This frame's resolution, for rendering and for the requests it needs.
#[derive(Debug, Clone, Default)]
pub(super) struct RouteView {
    /// Source Auto's outcome, when Pay from is Auto.
    pub auto: Option<AutoSource>,
    /// Max-with-Auto's outcome, when Pay from is Auto and Max is on.
    pub auto_max: Option<AutoMax>,
    /// Why the wallet said Core can send nothing, when it did.
    pub auto_shortfall: Option<CoreSendShortfall>,
    /// The destination Auto resolved against.
    pub destination: Option<AutoDestination>,
    pub destination_status: DestinationStatus,
    /// The kind a "My wallets" destination receives into.
    pub receive_kind: Option<PayFromKind>,
    /// Why the explicit source cannot pay this destination.
    pub refusal: Option<RouteRefusal>,
    /// Sources whose Max quotes this frame needs.
    pub quote_sources: Vec<SourceSelection>,
    /// Own receiving addresses this frame needs.
    pub address_requests: Vec<(WalletSeedHash, OwnAddressKind)>,
}

impl WalletSendScreen {
    /// Balance kind of a resolved source.
    pub(super) fn source_kind(source: &SourceSelection) -> PayFromKind {
        match source {
            SourceSelection::CoreWallet => PayFromKind::Core,
            SourceSelection::PlatformAddresses(_) => PayFromKind::Platform,
            SourceSelection::Identity(_) => PayFromKind::Identity,
            SourceSelection::Shielded(..) => PayFromKind::Shielded,
        }
    }

    /// Platform addresses with balances, in the shape the route functions take.
    pub(super) fn platform_source_addresses(&self) -> Vec<(PlatformAddress, Address, u64)> {
        self.get_platform_addresses()
            .into_iter()
            .map(|(core, platform, balance)| (platform, core, balance))
            .collect()
    }

    /// The private balance this form may spend, if shielded operations are
    /// available and it holds anything.
    pub(super) fn spendable_shielded(&self) -> Option<(WalletSeedHash, u64)> {
        if !FeatureGate::ShieldedOperations.is_available(&self.app_context) {
            return None;
        }
        self.get_shielded_balance()
    }

    /// What the Amount field may offer for paying `destination` from `source`.
    /// The same figures back Max, the field's limits and Auto's "covers" test.
    pub(super) fn amount_limits_for(
        &self,
        source: &SourceSelection,
        destination: Option<&ValidatedAddress>,
    ) -> AmountLimits {
        let fee_estimator = self.app_context.fee_estimator();
        let dest_kind = destination.map(ValidatedAddress::kind);
        let mut quote = QuoteState::Ready;
        let (max_credits, hint) = match source {
            SourceSelection::CoreWallet => {
                let seed_hash = self.selected_wallet_seed_hash;
                let mut max = seed_hash.and_then(|seed_hash| {
                    // Only an asset-lock route starts from the builder ceiling:
                    // the same current-composition accessor validation uses, so
                    // Max can never offer a stale ceiling. A Core send takes the
                    // backend quote below.
                    let max_duffs = if Self::funds_asset_lock(source, dest_kind) {
                        let max = self.asset_lock_max_amount(&seed_hash).ok();
                        if max.is_none() {
                            quote = if self.asset_lock_balance.is_failed(&seed_hash) {
                                QuoteState::Failed
                            } else if self.asset_lock_balance.is_busy(&seed_hash) {
                                QuoteState::Busy
                            } else {
                                QuoteState::Pending
                            };
                        }
                        max
                    } else {
                        None
                    };
                    max_duffs.map(|amount| amount.saturating_mul(CREDITS_PER_DUFF))
                });
                let hint = match dest_kind {
                    Some(AddressKind::Platform) => {
                        match destination.and_then(|v| v.as_platform().copied()) {
                            Some(destination) => {
                                // With the fee on top, dispatch locks the amount
                                // plus this figure, so Max leaves room for it.
                                let on_top = self.funding_fee_on_top();
                                let estimated_fee = if on_top {
                                    fee_estimator
                                        .estimate_address_funding_from_asset_lock_duffs(2)
                                        .saturating_mul(CREDITS_PER_DUFF)
                                } else {
                                    estimate_address_funding_fee_from_transition(
                                        self.app_context.platform_version(),
                                        &destination,
                                    )
                                };
                                max = max.map(|amount| amount.saturating_sub(estimated_fee));
                                let fee = format_credits_as_dash(estimated_fee);
                                let minimum = format_duffs_as_dash(
                                    fee_estimator.address_funding_min_amount_duffs(!on_top),
                                );
                                Some(if on_top {
                                    format!(
                                        "A Platform fee of approximately {fee} is paid from your balance on top of the amount, so the amount must be at least {minimum}."
                                    )
                                } else {
                                    format!(
                                        "A Platform fee of approximately {fee} is reserved from your balance and deducted from the amount, so the amount must be at least {minimum}."
                                    )
                                })
                            }
                            None => None,
                        }
                    }
                    Some(AddressKind::Shielded) => match self.shield_from_core_fees_duffs() {
                        Some((platform_fee_duffs, l1_tx_fee_duffs)) => {
                            let platform_fee_credits =
                                platform_fee_duffs.saturating_mul(CREDITS_PER_DUFF);
                            let total_fee_credits = platform_fee_duffs
                                .saturating_add(l1_tx_fee_duffs)
                                .saturating_mul(CREDITS_PER_DUFF);
                            max = max.map(|amount| {
                                asset_lock_user_max_amount(amount, platform_fee_credits)
                            });
                            let fee = format_credits_as_dash(total_fee_credits);
                            Some(format!(
                                "Shielding fees of approximately {fee} are reserved from your balance."
                            ))
                        }
                        None => {
                            // Without a fee to reserve, Max must offer nothing.
                            max = None;
                            quote = QuoteState::Ready;
                            None
                        }
                    },
                    Some(AddressKind::Identity) => {
                        let estimated_fee = fee_estimator.estimate_identity_topup();
                        max = max.map(|amount| amount.saturating_sub(estimated_fee));
                        let fee = format_credits_as_dash(estimated_fee);
                        Some(
                            match self
                                .wallet_funding_minimum_duffs(AddressKind::Identity)
                                .map(format_duffs_as_dash)
                            {
                                Some(minimum) => format!(
                                    "An identity top-up fee of approximately {fee} is reserved from your balance. The network fee is taken from the amount, so it must be at least {minimum}."
                                ),
                                None => format!(
                                    "An identity top-up fee of approximately {fee} is reserved from your balance."
                                ),
                            },
                        )
                    }
                    Some(AddressKind::Core) | None => {
                        // Core-to-Core "Max": the backend's quote of the largest
                        // payment the pooled build accepts, net of its fee;
                        // with no destination yet, the P2PKH quote.
                        // Nothing is offered until it arrives.
                        match self.spend_limit_for(source, dest_kind, self.core_send_limit_kind()) {
                            // The backend states a zero as its reason; this
                            // only keeps a zero from being offered.
                            Some(0) => {
                                max = None;
                                None
                            }
                            Some(max_credits) => {
                                max = Some(max_credits);
                                let fee_duffs = self
                                    .core_send_quote_fee_for(source, dest_kind)
                                    .map(|quote| core_send_fee_at_max_duffs(&quote))
                                    .unwrap_or_default();
                                let fee = format_credits_as_dash(fee_duffs * CREDITS_PER_DUFF);
                                Some(format!(
                                    "A network fee of approximately {fee} is reserved from your balance."
                                ))
                            }
                            None => {
                                max = None;
                                quote = self.spend_limit_quote_state(source, dest_kind);
                                None
                            }
                        }
                    }
                };
                (max, hint)
            }
            SourceSelection::PlatformAddresses(_) if dest_kind == Some(AddressKind::Shielded) => {
                // Shield-from-Platform: the upstream planner picks the input
                // addresses in its own order, skips the ones below the minimum
                // input and caps their count, so only its quote is a Max it
                // accepts. Nothing is offered until that quote arrives.
                let headroom =
                    shield_from_balance_fee_headroom(self.app_context.platform_version());
                let fee = format_credits_as_dash(headroom);
                match self.spend_limit_for(source, dest_kind, SpendLimitKind::ShieldFromPlatform) {
                    // No address can keep the fee back: a zero Max would only
                    // fill in an amount the form refuses.
                    Some(0) => (
                        None,
                        Some(format!(
                            "None of your Platform addresses can keep back {fee} to pay the shielding fee, so nothing can be shielded right now."
                        )),
                    ),
                    max => {
                        if max.is_none() {
                            quote = self.spend_limit_quote_state(source, dest_kind);
                        }
                        (
                            max,
                            Some(format!(
                                "Up to {fee} is kept back from your balance to pay the shielding fee. What the fee does not use stays on your Platform address."
                            )),
                        )
                    }
                }
            }
            SourceSelection::PlatformAddresses(addresses) => {
                // Recipients never pay: several recipients exclude every one of
                // them and each adds an output, as their dispatch does.
                let recipients: Vec<PlatformAddress> = if self.form.has_several_recipients() {
                    // Distinct, as the dispatch merges repeated recipients.
                    self.form
                        .recipients()
                        .iter()
                        .filter_map(|row| {
                            PlatformAddress::from_bech32m_string(row.address.trim()).ok()
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect()
                } else {
                    destination
                        .and_then(|v| v.as_platform().copied())
                        .into_iter()
                        .collect()
                };
                let mut sorted_addresses: Vec<_> = addresses
                    .iter()
                    .filter(|(addr, _, _)| !recipients.contains(addr))
                    .cloned()
                    .collect();
                sorted_addresses.sort_by_key(|a| std::cmp::Reverse(a.2));

                // Sum balances from top addresses, limited by the protocol's
                // input limit.
                let max_inputs = fee_estimator.max_platform_inputs();
                let total: u64 = sorted_addresses
                    .iter()
                    .take(max_inputs)
                    .map(|(_, _, balance)| *balance)
                    .sum();
                let max_fee = self.estimate_max_fee_for_platform_send(
                    &fee_estimator,
                    &sorted_addresses,
                    &recipients,
                );
                let fee = format_credits_as_dash(max_fee);
                let hint = if sorted_addresses.len() > max_inputs {
                    format!(
                        "This transaction is limited to {max_inputs} input addresses, and fees of approximately {fee} are reserved from your balance."
                    )
                } else {
                    format!("Fees of approximately {fee} are reserved from your balance.")
                };
                (Some(total.saturating_sub(max_fee)), Some(hint))
            }
            SourceSelection::Shielded(_, balance) => (
                Some(*balance),
                Some("The maximum is based on your private balance.".to_string()),
            ),
            SourceSelection::Identity(qi) => {
                let estimated_fee = self.identity_source_fee_reserve_for(dest_kind);
                let available = max_spendable_credits(qi.identity.balance(), estimated_fee);
                let fee = format_credits_as_dash(estimated_fee);
                (
                    Some(available),
                    Some(format!(
                        "Fees of approximately {fee} are reserved from your balance."
                    )),
                )
            }
        };

        let input_kind = match source {
            SourceSelection::CoreWallet => Some(AddressKind::Core),
            // Identity credits behave like Platform credits.
            SourceSelection::PlatformAddresses(_) | SourceSelection::Identity(_) => {
                Some(AddressKind::Platform)
            }
            SourceSelection::Shielded(..) => Some(AddressKind::Shielded),
        };
        let min_credits = self.min_output_amount(input_kind, dest_kind);
        // No amount the wallet can fund covers the network fee, so Max must
        // not propose one the network would refuse.
        let fee_not_covered = input_kind == Some(AddressKind::Core)
            && matches!(
                dest_kind,
                Some(AddressKind::Platform | AddressKind::Identity | AddressKind::Shielded)
            )
            && matches!((max_credits, min_credits), (Some(max), Some(min))
                if asset_lock_user_amount_range(max, 0, min).is_none());
        AmountLimits {
            max_credits,
            hint,
            quote,
            fee_not_covered,
            min_credits,
        }
    }

    fn spend_limit_quote_state(
        &self,
        source: &SourceSelection,
        destination: Option<AddressKind>,
    ) -> QuoteState {
        let Some((seed_hash, kind, revision)) = self.spend_limit_target_for(source, destination)
        else {
            return QuoteState::Ready;
        };
        if self.spend_limits.is_failed(&seed_hash, kind, revision) {
            return QuoteState::Failed;
        }
        if self.spend_limits.is_busy(&seed_hash, kind, revision) {
            return QuoteState::Busy;
        }
        match self.spend_limits.unavailable(&seed_hash, kind, revision) {
            // Held funds come back once the other transaction finishes.
            Some(CoreSendShortfall::HeldByPendingTransaction) => QuoteState::Busy,
            // A settled zero: Auto passes over this balance instead of waiting.
            Some(reason) => QuoteState::Shortfall(reason),
            None => QuoteState::Pending,
        }
    }

    /// The identities of `seed_hash`'s wallet, loaded once per form refresh.
    pub(super) fn identities_of(&mut self, seed_hash: &WalletSeedHash) -> Vec<QualifiedIdentity> {
        if self.identities_by_wallet.is_none() {
            #[cfg(test)]
            self.identity_load_count
                .set(self.identity_load_count.get() + 1);
            let mut by_wallet: BTreeMap<WalletSeedHash, Vec<QualifiedIdentity>> = BTreeMap::new();
            for identity in self
                .app_context
                .load_local_qualified_identities()
                .unwrap_or_default()
            {
                if let Some(owner) = self.app_context.identity_owner_wallet(&identity) {
                    by_wallet.entry(owner).or_default().push(identity);
                }
            }
            self.identities_by_wallet = Some(by_wallet);
        }
        self.identities_by_wallet
            .as_ref()
            .and_then(|by_wallet| by_wallet.get(seed_hash).cloned())
            .unwrap_or_default()
    }

    /// Identities the From wallet can pay from: all of its identities, less a
    /// Top up's recipient.
    pub(super) fn paying_identities(&mut self) -> Vec<QualifiedIdentity> {
        let Some(seed_hash) = self.selected_wallet_seed_hash else {
            return Vec::new();
        };
        let mut identities = self.identities_of(&seed_hash);
        if let Some((recipient, IdentitySendMode::TopUp)) = &self.identity_flow {
            identities.retain(|source| source.identity.id() != recipient.identity.id());
        }
        identities
    }

    /// Identities of the To wallet that can receive: all of them, less the
    /// identity that pays.
    pub(super) fn receiving_identities(
        &mut self,
        to_wallet: &WalletSeedHash,
    ) -> Vec<QualifiedIdentity> {
        let paying = match &self.selected_source {
            Some(SourceSelection::Identity(qi)) => Some(qi.identity.id()),
            _ => None,
        };
        let mut identities = self.identities_of(to_wallet);
        identities.retain(|qi| Some(qi.identity.id()) != paying);
        identities
    }

    /// The identity Pay from › Identity spends: the user's pick, else the
    /// app-selected identity when it is in this wallet, else the richest.
    pub(super) fn chosen_paying_identity(&mut self) -> Option<QualifiedIdentity> {
        let identities = self.paying_identities();
        let preferred = self
            .form
            .pay_identity()
            .or_else(|| self.app_context.selected_identity_id());
        identities
            .iter()
            .find(|qi| Some(qi.identity.id()) == preferred)
            .or_else(|| identities.iter().max_by_key(|qi| qi.identity.balance()))
            .cloned()
    }

    /// The identity Receive into › Identity tops up.
    pub(super) fn chosen_receiving_identity(
        &mut self,
        to_wallet: &WalletSeedHash,
    ) -> Option<QualifiedIdentity> {
        let identities = self.receiving_identities(to_wallet);
        let preferred = self.form.receive_identity();
        identities
            .iter()
            .find(|qi| Some(qi.identity.id()) == preferred)
            .or_else(|| identities.iter().max_by_key(|qi| qi.identity.balance()))
            .cloned()
    }

    /// The paying wallet's balance a typed destination belongs to, if any.
    pub(super) fn own_account_of(&self, destination: &ValidatedAddress) -> Option<PayFromKind> {
        let seed_hash = self.selected_wallet_seed_hash?;
        match destination {
            ValidatedAddress::Core(address) => self
                .app_context
                .snapshot_address_paths(&seed_hash)
                .get(address)
                .filter(|path| !path.is_platform_payment(self.app_context.network))
                .map(|_| PayFromKind::Core),
            ValidatedAddress::Platform { address, .. } => self
                .is_own_platform_address(&seed_hash, address)
                .then_some(PayFromKind::Platform),
            ValidatedAddress::Shielded(_) => self
                .shielded_destination_is_own_pool()
                .then_some(PayFromKind::Shielded),
            ValidatedAddress::Identity { .. } => None,
        }
    }

    /// Whether `address` belongs to the paying wallet, whatever it holds: the
    /// derivation inventory (as for Core), the watched addresses, and every
    /// address with recorded Platform state, not only those with a balance.
    fn is_own_platform_address(
        &self,
        seed_hash: &WalletSeedHash,
        address: &PlatformAddress,
    ) -> bool {
        let network = self.app_context.network;
        let matches =
            |core: &Address| PlatformAddress::try_from(core.clone()).is_ok_and(|p| p == *address);
        if self
            .platform_source_addresses()
            .iter()
            .any(|(platform, _, _)| platform == address)
            || self
                .app_context
                .snapshot_address_paths(seed_hash)
                .iter()
                .any(|(core, path)| path.is_platform_payment(network) && matches(core))
        {
            return true;
        }
        self.selected_wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok())
            .is_some_and(|wallet| {
                wallet
                    .platform_addresses(network)
                    .iter()
                    .any(|(_, p)| p == address)
                    || wallet.platform_address_info.keys().any(matches)
            })
    }

    /// An unused watched Platform address of `wallet`: no balance, no history.
    fn unused_platform_address(&self, seed_hash: &WalletSeedHash) -> Option<PlatformAddress> {
        let wallet = self.app_context.wallet_context().hd_wallet(seed_hash)?;
        let wallet = wallet.read().ok()?;
        let network = self.app_context.network;
        wallet
            .platform_addresses(network)
            .into_iter()
            .find(|(core, _)| {
                wallet
                    .get_platform_address_info(core)
                    .is_none_or(|info| info.balance == 0 && info.nonce == 0)
            })
            .map(|(_, platform)| platform)
    }

    fn wallet_is_open(&self, seed_hash: &WalletSeedHash) -> bool {
        self.app_context
            .wallet_context()
            .hd_wallet(seed_hash)
            .is_some_and(|wallet| wallet.read().map(|w| w.is_open()).unwrap_or(false))
    }

    /// Kind a "My wallets" destination receives into this frame.
    fn receive_kind(&self, same_wallet: bool) -> PayFromKind {
        match self.form.receive_into().kind() {
            Some(kind) => kind,
            None => {
                let source = self
                    .form
                    .explicit_source_kind()
                    .or_else(|| self.selected_source.as_ref().map(Self::source_kind))
                    .unwrap_or(PayFromKind::Core);
                let mut kind = resolve_destination_auto(source, same_wallet);
                // A preset that fixes the destination kinds (Unshield) keeps
                // Auto within them, whichever wallet receives.
                if let Some(allowed) = self.flow.preset_destination_kinds()
                    && !allowed.contains(&kind)
                    && let Some(first) = allowed.first()
                {
                    kind = *first;
                }
                match kind {
                    AddressKind::Core => PayFromKind::Core,
                    AddressKind::Platform => PayFromKind::Platform,
                    AddressKind::Shielded => PayFromKind::Shielded,
                    AddressKind::Identity => PayFromKind::Identity,
                }
            }
        }
    }

    /// Resolve a "My wallets" destination, asking for addresses it needs.
    fn resolve_own_destination(&mut self, view: &mut RouteView) -> AutoDestination {
        let Some(to_wallet) = self.form.to_wallet().or(self.selected_wallet_seed_hash) else {
            self.set_destination(None);
            return AutoDestination::external(None);
        };
        let same_wallet = Some(to_wallet) == self.selected_wallet_seed_hash;
        let kind = self.receive_kind(same_wallet);
        view.receive_kind = Some(kind);
        let network = self.app_context.network;
        let (destination, status) = match kind {
            PayFromKind::Core => match self.own_addresses.get(&to_wallet, OwnAddressKind::Core) {
                Some(address) => match address
                    .parse::<Address<NetworkUnchecked>>()
                    .ok()
                    .and_then(|address| address.require_network(network).ok())
                {
                    Some(address) => (
                        Some(ValidatedAddress::Core(address)),
                        DestinationStatus::Ready,
                    ),
                    None => (None, DestinationStatus::Fetching { kind }),
                },
                None => {
                    view.address_requests
                        .push((to_wallet, OwnAddressKind::Core));
                    (None, DestinationStatus::Fetching { kind })
                }
            },
            PayFromKind::Platform => {
                let known = self.unused_platform_address(&to_wallet).or_else(|| {
                    self.own_addresses
                        .get(&to_wallet, OwnAddressKind::Platform)
                        .and_then(|address| PlatformAddress::from_bech32m_string(address).ok())
                });
                match known {
                    Some(address) => (
                        Some(ValidatedAddress::Platform {
                            address,
                            bech32m: address.to_bech32m_string(network),
                        }),
                        DestinationStatus::Ready,
                    ),
                    None if self.wallet_is_open(&to_wallet) => {
                        view.address_requests
                            .push((to_wallet, OwnAddressKind::Platform));
                        (None, DestinationStatus::Fetching { kind })
                    }
                    None => (None, DestinationStatus::NeedsUnlock { wallet: to_wallet }),
                }
            }
            PayFromKind::Shielded if same_wallet => (
                // The paying wallet's own pool: Core and Platform shield into
                // it whatever the address, so the form marks it as its own.
                Some(ValidatedAddress::Shielded(String::new())),
                DestinationStatus::Ready,
            ),
            PayFromKind::Shielded => match self.app_context.shielded_receive_address(&to_wallet) {
                Some(address) => (
                    Some(ValidatedAddress::Shielded(address)),
                    DestinationStatus::Ready,
                ),
                None => (None, DestinationStatus::NeedsUnlock { wallet: to_wallet }),
            },
            PayFromKind::Identity => match self.chosen_receiving_identity(&to_wallet) {
                Some(identity) => (
                    Some(ValidatedAddress::Identity {
                        id: identity.identity.id(),
                        dpns_name: self.app_context.main_username(&identity),
                    }),
                    DestinationStatus::Ready,
                ),
                None => (None, DestinationStatus::NoIdentity),
            },
        };
        view.destination_status = status;
        let shielded_is_own_pool = kind == PayFromKind::Shielded && same_wallet;
        self.set_destination(destination);
        AutoDestination {
            kind: self.destination_kind(),
            shielded_is_own_pool,
            same_account: same_wallet.then_some(kind),
        }
    }

    /// Replace the destination only when it changed, so an intact one is
    /// never rebuilt.
    fn set_destination(&mut self, destination: Option<ValidatedAddress>) {
        if self.validated_destination != destination {
            self.validated_destination = destination;
        }
    }

    /// Resolve the destination for this frame.
    fn resolve_destination(&mut self, view: &mut RouteView) -> AutoDestination {
        if self.form.locks().to && matches!(self.identity_flow, Some((_, IdentitySendMode::TopUp)))
        {
            // The bound recipient: kept by the identity constraint.
            view.destination_status = DestinationStatus::Ready;
            return AutoDestination::external(self.destination_kind());
        }
        match self.form.to_mode() {
            ToMode::MyWallets => self.resolve_own_destination(view),
            ToMode::SomeoneElse => {
                let typed = if self.form.has_several_recipients() {
                    self.rows_destination()
                } else {
                    self.typed_destination.clone()
                };
                self.set_destination(typed);
                view.destination_status = if self.validated_destination.is_some() {
                    DestinationStatus::Ready
                } else {
                    DestinationStatus::Missing
                };
                let same_account = self
                    .validated_destination
                    .as_ref()
                    .and_then(|destination| self.own_account_of(destination));
                AutoDestination {
                    kind: self.destination_kind(),
                    shielded_is_own_pool: self.shielded_destination_is_own_pool(),
                    same_account,
                }
            }
        }
    }

    /// With several recipients, the destination Auto resolves against: the
    /// first row, once every filled-in row is a valid address of its kind.
    fn rows_destination(&self) -> Option<ValidatedAddress> {
        let mut rows = self
            .form
            .recipients()
            .iter()
            .map(|row| row.address.trim())
            .filter(|address| !address.is_empty())
            .map(|address| self.row_address(address));
        let first = rows.next()??;
        rows.all(|row| row.is_some_and(|row| row.kind() == first.kind()))
            .then_some(first)
    }

    /// A recipient row's address, when it is a valid Core or Platform address
    /// on the active network.
    pub(super) fn row_address(&self, address: &str) -> Option<ValidatedAddress> {
        let address = address.trim();
        match AddressKind::detect(address)? {
            AddressKind::Core => address
                .parse::<Address<NetworkUnchecked>>()
                .ok()?
                .require_network(self.app_context.network)
                .ok()
                .map(ValidatedAddress::Core),
            AddressKind::Platform => {
                validate_platform_address_for_network(address, self.app_context.network).ok()?;
                PlatformAddress::from_bech32m_string(address)
                    .ok()
                    .map(|platform| ValidatedAddress::Platform {
                        address: platform,
                        bech32m: address.to_string(),
                    })
            }
            _ => None,
        }
    }

    /// Whether every recipient row names an address and a positive amount.
    /// A row missing either would be left out of the payment.
    pub(super) fn recipient_rows_complete(&self) -> bool {
        self.form.recipients().iter().all(|row| {
            !row.address.trim().is_empty()
                && Self::parse_amount_to_credits(&row.amount).is_ok_and(|credits| credits > 0)
        })
    }

    /// The amount the source must cover, in credits: the single amount, or
    /// the recipients' total.
    pub(super) fn requested_credits(&self) -> Option<u64> {
        if self.form.has_several_recipients() {
            let total = self
                .form
                .recipients()
                .iter()
                .filter_map(|row| Self::parse_amount_to_credits(&row.amount).ok())
                .fold(0u64, u64::saturating_add);
            return (total > 0).then_some(total);
        }
        self.amount.as_ref().map(Amount::value)
    }

    /// The concrete source of an explicit Pay from choice.
    fn explicit_source(&mut self, choice: PayFrom) -> Option<SourceSelection> {
        self.selected_wallet.as_ref()?;
        match choice {
            PayFrom::Auto => None,
            PayFrom::Core => Some(SourceSelection::CoreWallet),
            PayFrom::Platform => {
                let addresses = self.platform_source_addresses();
                (!addresses.is_empty()).then_some(SourceSelection::PlatformAddresses(addresses))
            }
            PayFrom::Shielded => {
                let seed_hash = self.selected_wallet_seed_hash?;
                // A Shielded preset spends the private balance whatever it
                // holds; the free form offers it only when it holds anything.
                if self.form.locks().pay_from {
                    let balance = self.app_context.shielded_balance_credits(&seed_hash);
                    Some(SourceSelection::Shielded(seed_hash, balance))
                } else {
                    self.spendable_shielded()
                        .map(|(seed_hash, balance)| SourceSelection::Shielded(seed_hash, balance))
                }
            }
            PayFrom::Identity => self
                .chosen_paying_identity()
                .map(|identity| SourceSelection::Identity(Box::new(identity))),
            PayFrom::Advanced => match self.form.advanced_type() {
                AdvancedSourceType::Core => Some(SourceSelection::CoreWallet),
                AdvancedSourceType::Platform => {
                    let ticked: Vec<String> = self
                        .form
                        .ticked_advanced()
                        .into_iter()
                        .map(|(address, _)| address)
                        .collect();
                    let network = self.app_context.network;
                    let addresses: Vec<_> = self
                        .platform_source_addresses()
                        .into_iter()
                        .filter(|(platform, _, _)| {
                            ticked.contains(&platform.to_bech32m_string(network))
                        })
                        .collect();
                    (!addresses.is_empty()).then_some(SourceSelection::PlatformAddresses(addresses))
                }
            },
        }
    }

    /// Spendable maxima of the balances Auto may choose from, and why the
    /// wallet said Core can send nothing, if it did.
    fn candidate_maxima(
        &self,
        destination: &AutoDestination,
    ) -> (CandidateMaxima, Option<CoreSendShortfall>) {
        if self.selected_wallet.is_none() {
            return (CandidateMaxima::default(), None);
        }
        let platform = self.platform_source_addresses();
        if destination.kind.is_none() {
            // No destination yet: a preview from the balances.
            let maxima = CandidateMaxima {
                core: CandidateMax::Ready(self.get_core_balance().saturating_mul(CREDITS_PER_DUFF)),
                platform: CandidateMax::Ready(platform.iter().map(|(_, _, b)| *b).sum()),
                shielded: CandidateMax::Unavailable,
            };
            return (maxima, None);
        }
        let several = self.form.has_several_recipients();
        let destination_value = self.validated_destination.as_ref();
        let shortfall = std::cell::Cell::new(None);
        let max_of = |source: &SourceSelection| {
            let limits = self.amount_limits_for(source, destination_value);
            if let QuoteState::Shortfall(reason) = limits.quote {
                shortfall.set(Some(reason));
            }
            limits.candidate_max()
        };
        // Several recipients: Core rows are paid from Core, Platform rows from
        // Platform; no other route carries several outputs.
        let core = if several && destination.kind != Some(AddressKind::Core) {
            CandidateMax::Unavailable
        } else {
            max_of(&SourceSelection::CoreWallet)
        };
        let platform = if platform.is_empty()
            || (several && destination.kind != Some(AddressKind::Platform))
        {
            CandidateMax::Unavailable
        } else {
            max_of(&SourceSelection::PlatformAddresses(platform))
        };
        let shielded = match self.spendable_shielded() {
            Some((seed_hash, balance)) if !several => {
                max_of(&SourceSelection::Shielded(seed_hash, balance))
            }
            _ => CandidateMax::Unavailable,
        };
        let maxima = CandidateMaxima {
            core,
            platform,
            shielded,
        };
        (maxima, shortfall.get())
    }

    /// The source for a balance kind Auto chose.
    fn auto_source(&self, kind: PayFromKind) -> Option<SourceSelection> {
        match kind {
            PayFromKind::Core => Some(SourceSelection::CoreWallet),
            PayFromKind::Platform => Some(SourceSelection::PlatformAddresses(
                self.platform_source_addresses(),
            )),
            PayFromKind::Shielded => self
                .spendable_shielded()
                .map(|(seed_hash, balance)| SourceSelection::Shielded(seed_hash, balance)),
            PayFromKind::Identity => None,
        }
    }

    /// Resolve the form into this frame's source and destination.
    pub(super) fn sync_route(&mut self) {
        self.keep_identity_constraint();
        let mut view = RouteView::default();
        let destination = self.resolve_destination(&mut view);

        let identity_sender = matches!(
            self.identity_flow,
            Some((_, IdentitySendMode::Send | IdentitySendMode::MoveToWallet))
        );
        if !identity_sender {
            let source = match self.form.pay_from() {
                PayFrom::Auto => {
                    let (maxima, shortfall) = self.candidate_maxima(&destination);
                    view.auto_shortfall = shortfall;
                    if self.form.max_intent() && destination.kind.is_some() {
                        let outcome = resolve_auto_max(&destination, &maxima);
                        let source = match &outcome {
                            AutoMax::Chosen { kind, .. } => self.auto_source(*kind),
                            AutoMax::Checking { pending } => {
                                view.quote_sources.extend(
                                    pending.iter().filter_map(|kind| self.auto_source(*kind)),
                                );
                                None
                            }
                            AutoMax::QuoteFailed { .. }
                            | AutoMax::Withheld { .. }
                            | AutoMax::NoFunds => None,
                        };
                        view.auto_max = Some(outcome);
                        source
                    } else {
                        let outcome =
                            resolve_source_auto(&destination, self.requested_credits(), &maxima);
                        let source = match &outcome {
                            AutoSource::Chosen { kind, .. } => self.auto_source(*kind),
                            AutoSource::Checking { kind } => {
                                view.quote_sources.extend(self.auto_source(*kind));
                                None
                            }
                            _ => None,
                        };
                        view.auto = Some(outcome);
                        source
                    }
                }
                choice => self.explicit_source(choice),
            };
            if self.selected_source != source {
                self.selected_source = source;
            }
        }

        if let (Some(source), Some(kind)) = (&self.selected_source, self.destination_kind()) {
            view.refusal = route_refusal(
                Self::source_kind(source),
                kind,
                destination.shielded_is_own_pool,
            );
            if self.form.pay_from() != PayFrom::Auto || identity_sender {
                view.quote_sources.push(source.clone());
            }
        } else if let Some(source) = &self.selected_source
            && self.form.pay_from() != PayFrom::Auto
        {
            view.quote_sources.push(source.clone());
        }
        // Refused before anything is ticked, so no ticking is wasted on it.
        if self.form.pay_from() == PayFrom::Advanced
            && self.form.advanced_type() == AdvancedSourceType::Platform
            && let Some(kind) = self.destination_kind()
        {
            view.refusal = view
                .refusal
                .or_else(|| ticked_source_refusal(PayFromKind::Platform, kind));
        }
        view.destination = Some(destination);
        self.route = view;
    }

    /// Requests this frame's resolution needs: Max quotes and own receiving
    /// addresses.
    pub(super) fn route_requests(&mut self) -> Vec<BackendTask> {
        let destination = self.destination_kind();
        let mut tasks = Vec::new();
        for source in self.route.quote_sources.clone() {
            tasks.extend(self.request_asset_lock_max_amount_for(&source, destination));
            tasks.extend(self.request_spend_limit_for(&source, destination));
        }
        for (seed_hash, kind) in self.route.address_requests.clone() {
            tasks.extend(self.own_addresses.ensure_requested(seed_hash, kind));
        }
        tasks
    }
}
