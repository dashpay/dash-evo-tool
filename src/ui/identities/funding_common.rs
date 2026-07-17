use crate::model::wallet::Wallet;
use crate::ui::state::TrackedAssetLockCache;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::{OutPoint, TxOut};
use eframe::epaint::{Color32, ColorImage};
use egui::{ComboBox, Ui, Vec2};
use image::Luma;
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
use qrcode::QrCode;
use std::fmt;
use std::sync::{Arc, RwLock};

/// How the user chooses to fund an identity operation. Shared by the
/// create-identity and top-up screens (both render the same chooser), so the
/// enum, its labels, and the default pre-selection live in one place.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum FundingMethod {
    NoSelection,
    UseUnusedAssetLock,
    UseWalletBalance,
    /// Use Platform Address credits.
    UsePlatformAddress,
    /// Receive a fresh Dash deposit to a shown address/QR, then fund from it.
    ReceiveDeposit,
}

impl fmt::Display for FundingMethod {
    /// Everyday-user labels. `UseUnusedAssetLock` deliberately avoids "asset
    /// lock" jargon — in the create context it reads as recovering an
    /// interrupted setup. Top-Up uses [`FundingMethod::top_up_label`] instead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = match self {
            FundingMethod::NoSelection => "Select how to fund",
            FundingMethod::UseWalletBalance => "From your wallet (recommended)",
            FundingMethod::UseUnusedAssetLock => "Recover an unfinished funding",
            FundingMethod::UsePlatformAddress => "Use a Platform address",
            FundingMethod::ReceiveDeposit => "Receive a new deposit",
        };
        write!(f, "{}", output)
    }
}

impl FundingMethod {
    /// Top-Up-context label. Shares [`Display`]'s wording except for
    /// `UseUnusedAssetLock`: an existing identity being topped up was never
    /// mid-setup, so "recover an unfinished funding" doesn't fit — it just
    /// reuses an existing funding transaction.
    pub fn top_up_label(&self) -> &'static str {
        match self {
            FundingMethod::NoSelection => "Select how to fund",
            FundingMethod::UseWalletBalance => "From your wallet (recommended)",
            FundingMethod::UseUnusedAssetLock => "Use an existing funding transaction",
            FundingMethod::UsePlatformAddress => "Use a Platform address",
            FundingMethod::ReceiveDeposit => "Receive a new deposit",
        }
    }
}

/// The funding-method chooser's starting state for a wallet that either does
/// or doesn't have spendable balance (§B.9: pre-select `UseWalletBalance`
/// only when it is actually available; otherwise start unselected rather
/// than land on a method the ComboBox itself wouldn't offer). A pure function
/// so the decision is testable without constructing a real wallet/balance
/// snapshot.
pub fn default_funding_state(wallet_has_balance: bool) -> (FundingMethod, WalletFundedScreenStep) {
    if wallet_has_balance {
        (
            FundingMethod::UseWalletBalance,
            WalletFundedScreenStep::ReadyToCreate,
        )
    } else {
        (
            FundingMethod::NoSelection,
            WalletFundedScreenStep::ChooseFundingMethod,
        )
    }
}

/// Resolve the funding method and step after the selected wallet changes.
///
/// While the user has not made an explicit choice, the screen re-applies its
/// default pre-selection for the new wallet, so the chooser always tracks a
/// wallet the current selection can actually be funded from. Once a method has
/// been chosen explicitly, a wallet switch preserves it untouched — a switch
/// must never silently change a funding method the user picked on purpose.
pub fn funding_method_after_switch(
    user_chose_method: bool,
    current: (FundingMethod, WalletFundedScreenStep),
    new_wallet_has_balance: bool,
) -> (FundingMethod, WalletFundedScreenStep) {
    if user_chose_method {
        current
    } else {
        default_funding_state(new_wallet_has_balance)
    }
}

/// Whether a wallet holding `spendable_duffs` can cover `minimum_credits` of
/// platform fees. Shared by the Create-Identity and Top-Up wallet-balance
/// funding gates so the duffs -> credits conversion has one source of truth.
pub fn spendable_covers_minimum(spendable_duffs: u64, minimum_credits: u64) -> bool {
    spendable_duffs.saturating_mul(CREDITS_PER_DUFF) >= minimum_credits
}

/// Resolve a polling snapshot for the address shown by the deposit flow.
/// Advancement and prefill are both bounded by funds at that address, never by
/// unrelated spendable funds elsewhere in the wallet.
pub fn snapshot_deposit_outcome(
    current_step: WalletFundedScreenStep,
    address_balance_duffs: u64,
    minimum_credits: u64,
) -> (WalletFundedScreenStep, Option<u64>) {
    let advance = current_step == WalletFundedScreenStep::WaitingOnFunds
        && spendable_covers_minimum(address_balance_duffs, minimum_credits);
    let next_step = if advance {
        WalletFundedScreenStep::FundsReceived
    } else {
        current_step
    };
    let prefill =
        advance.then(|| max_amount_after_fee_reserve(address_balance_duffs, minimum_credits));
    (next_step, prefill)
}

/// Whether the QR flow should dispatch a new receive-address request.
pub fn should_queue_funding_address(
    has_address: bool,
    request_pending: bool,
    request_in_flight: bool,
    request_failed: bool,
) -> bool {
    !has_address && !request_pending && !request_in_flight && !request_failed
}

/// Restore an amount-entry step after a failed funding task without moving a
/// deposit-address failure away from its retry view.
pub fn step_after_task_failure(current_step: WalletFundedScreenStep) -> WalletFundedScreenStep {
    match current_step {
        WalletFundedScreenStep::WaitingForAssetLock
        | WalletFundedScreenStep::WaitingForPlatformAcceptance => {
            WalletFundedScreenStep::ReadyToCreate
        }
        _ => current_step,
    }
}

/// The largest amount, in credits, a "Max" button can safely offer from a
/// wallet holding `spendable_duffs`, after reserving `fee_credits` for the
/// platform fee. Built on `spendable_duffs` (not the wallet's `total`, which
/// also counts immature/locked funds coin selection cannot touch) so the
/// offered amount never exceeds what the wallet can actually send.
pub fn max_amount_after_fee_reserve(spendable_duffs: u64, fee_credits: u64) -> u64 {
    spendable_duffs
        .saturating_mul(CREDITS_PER_DUFF)
        .saturating_sub(fee_credits)
}

/// Round a DASH amount up to 4 decimal places — the precision of the `dash:`
/// payment URI. Rounding up (never to nearest) guarantees the amount shown in
/// the hint and encoded in the QR never understates the true minimum needed.
pub fn round_up_dash_4dp(dash: f64) -> f64 {
    (dash * 10_000.0).ceil() / 10_000.0
}

/// Duffs received, in this event, by the one address shown to the user as their
/// deposit target. Sums the value of every output paying exactly `funding_address`
/// (single-address equality, not wallet-membership), so a deposit to any other
/// address contributes nothing. Returns `0` when no address is shown yet.
///
/// This decides only whether *this* event touched the shown address; the
/// cumulative available amount comes from that address's UTXO snapshot.
pub fn deposit_matches(
    funding_address: Option<&Address>,
    outputs: &[(OutPoint, TxOut, Address)],
) -> u64 {
    // Saturating fold: output values come from attacker-influenced Core tx data,
    // so a crafted overflow can never wrap the running total.
    outputs
        .iter()
        .filter(|(_, _, address)| Some(address) == funding_address)
        .fold(0u64, |acc, (_, tx_out, _)| acc.saturating_add(tx_out.value))
}

/// Next funding step after a received-UTXO event arrives while awaiting a
/// deposit. Advances to [`WalletFundedScreenStep::FundsReceived`] only when the
/// outputs in this event paid enough to the shown `funding_address` to cover
/// `minimum_credits`; otherwise the step is left unchanged. The per-frame
/// snapshot reconciler handles totals accumulated across multiple events. The
/// step guard prevents another funding method from advancing spuriously.
pub fn deposit_step_after_utxo(
    current_step: WalletFundedScreenStep,
    funding_address: Option<&Address>,
    outputs: &[(OutPoint, TxOut, Address)],
    minimum_credits: u64,
) -> WalletFundedScreenStep {
    if current_step != WalletFundedScreenStep::WaitingOnFunds {
        return current_step;
    }
    if spendable_covers_minimum(deposit_matches(funding_address, outputs), minimum_credits) {
        WalletFundedScreenStep::FundsReceived
    } else {
        current_step
    }
}

/// The next step plus the amount, in credits, to pre-fill into the funding field
/// when a deposit advances the wizard to [`WalletFundedScreenStep::FundsReceived`].
/// The pre-fill is the fee-reserve-capped balance (`Some` only on the advancing
/// event), so the amount and the confirm button are populated on arrival instead
/// of left at zero. Layers the pre-fill decision over [`deposit_step_after_utxo`]
/// so both live in one unit-tested place.
pub fn deposit_event_outcome(
    current_step: WalletFundedScreenStep,
    funding_address: Option<&Address>,
    outputs: &[(OutPoint, TxOut, Address)],
    fee_credits: u64,
) -> (WalletFundedScreenStep, Option<u64>) {
    let deposited_duffs = deposit_matches(funding_address, outputs);
    let next_step = deposit_step_after_utxo(current_step, funding_address, outputs, fee_credits);
    let prefill_credits = (next_step == WalletFundedScreenStep::FundsReceived)
        .then(|| max_amount_after_fee_reserve(deposited_duffs, fee_credits));
    (next_step, prefill_credits)
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub enum WalletFundedScreenStep {
    ChooseFundingMethod,
    WaitingOnFunds,
    FundsReceived,
    ReadyToCreate,
    WaitingForAssetLock,
    WaitingForPlatformAcceptance,
    Success,
}

/// A calm, jargon-free sentence describing where a funding asset lock is in its
/// lifecycle. Shown to the Everyday User when they pick an existing asset lock
/// to fund an identity, so they never see a raw `Debug` enum.
pub fn asset_lock_status_label(status: &AssetLockStatus) -> &'static str {
    match status {
        AssetLockStatus::Built => "Prepared, not yet sent to the network.",
        AssetLockStatus::Broadcast => "Sent to the network. Waiting for confirmation.",
        AssetLockStatus::InstantSendLocked => "Confirmed and ready to use.",
        AssetLockStatus::ChainLocked => "Confirmed and ready to use.",
        AssetLockStatus::Consumed => "Already used to fund an identity.",
    }
}

/// The Dash address that received the locked funds for this asset lock, derived
/// from the lock transaction's credit output. Returns `None` when the address
/// cannot be derived (e.g. a non-standard output). Lets the user tell two asset
/// locks apart by address as well as transaction id.
///
/// Mirrors the upstream recovery derivation, which reads the first credit output
/// of the asset-lock payload (asset locks built here carry a single credit
/// output).
pub fn asset_lock_address(lock: &TrackedAssetLock, network: Network) -> Option<Address> {
    let Some(TransactionPayload::AssetLockPayloadType(payload)) =
        &lock.transaction.special_transaction_payload
    else {
        return None;
    };
    let output = payload.credit_outputs.first()?;
    Address::from_script(&output.script_pubkey, network).ok()
}

/// Render a wallet-picker ComboBox and return the wallet the user clicked this
/// frame, if any. `label_fn` supplies each entry's text and the closed-box text
/// for the current selection; `enabled_fn` greys out wallets that cannot serve
/// the active funding method. Reset-on-change is the caller's responsibility —
/// act on the returned wallet. Shared by the create-identity, top-up, and
/// add-existing-identity screens so the picker scaffolding lives in one place.
pub fn wallet_selection_combo(
    ui: &mut Ui,
    id_salt: &str,
    wallets: &[Arc<RwLock<Wallet>>],
    selected: Option<&Arc<RwLock<Wallet>>>,
    mut label_fn: impl FnMut(&Arc<RwLock<Wallet>>) -> String,
    mut enabled_fn: impl FnMut(&Arc<RwLock<Wallet>>) -> bool,
) -> Option<Arc<RwLock<Wallet>>> {
    let selected_text = match selected {
        Some(wallet) => label_fn(wallet),
        None => "Select".to_string(),
    };

    let mut clicked = None;
    ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for wallet in wallets {
                let is_selected = selected.is_some_and(|s| Arc::ptr_eq(s, wallet));
                let label = label_fn(wallet);
                let enabled = enabled_fn(wallet);
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.selectable_label(is_selected, label).clicked() {
                        clicked = Some(wallet.clone());
                    }
                });
            }
        });
    clicked
}

/// Outcome of the shared unused-funding picker gate.
pub enum FundingAssetLockPicker {
    /// A gate message (no wallet / busy / load failed / loading / none found)
    /// was already rendered; there is nothing for the caller to show.
    Handled,
    /// Asset locks still usable to fund an identity, for the caller to render.
    Available(Vec<TrackedAssetLock>),
}

/// Render the shared load/gate states for the unused-funding picker and return
/// the actionable asset locks (Built / Broadcast / IS-Locked / Chain-Locked —
/// never Consumed) for the selected wallet. The create-identity and top-up
/// screens differ only in how each lock row looks, so this owns the identical
/// wallet-resolution, retry, loading, and empty states.
pub fn actionable_asset_locks(
    ui: &mut Ui,
    cache: &mut TrackedAssetLockCache,
    selected_wallet: Option<&Arc<RwLock<Wallet>>>,
) -> FundingAssetLockPicker {
    let Some(wallet) = selected_wallet else {
        ui.label("No wallet selected.");
        return FundingAssetLockPicker::Handled;
    };

    let seed_hash = match wallet.read() {
        Ok(w) => w.seed_hash(),
        Err(_) => {
            ui.label("Wallet is busy. Try again in a moment.");
            return FundingAssetLockPicker::Handled;
        }
    };

    if cache.is_failed(&seed_hash) {
        ui.label("Couldn't load your unfinished funding.");
        if ui.button("Retry").clicked() {
            cache.invalidate_one(&seed_hash);
        }
        return FundingAssetLockPicker::Handled;
    }

    let Some(all_tracked) = cache.get(&seed_hash) else {
        ui.label("Loading your unfinished funding…");
        return FundingAssetLockPicker::Handled;
    };

    // Consumed locks are tracked for history but cannot fund an identity.
    let tracked: Vec<TrackedAssetLock> = all_tracked
        .iter()
        .filter(|t| !matches!(t.status, AssetLockStatus::Consumed))
        .cloned()
        .collect();

    if tracked.is_empty() {
        ui.label("No unfinished funding was found.");
        return FundingAssetLockPicker::Handled;
    }

    FundingAssetLockPicker::Available(tracked)
}

// Function to generate a QR code image from the address
pub fn generate_qr_code_image(pay_uri: &str) -> Result<ColorImage, qrcode::types::QrError> {
    // Generate the QR code
    let code = QrCode::new(pay_uri.as_bytes())?;

    // Render the QR code into an image buffer
    let image = code.render::<Luma<u8>>().build();

    // Convert the image buffer to ColorImage
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let pixels: Vec<Color32> = pixels
        .into_iter()
        .map(|p| {
            let color = 255 - p; // Invert colors for better visibility
            Color32::from_rgba_unmultiplied(color, color, color, 255)
        })
        .collect();

    Ok(ColorImage {
        size,
        source_size: Vec2::new(size[0] as f32, size[1] as f32),
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_is_user_facing_for_every_variant() {
        // Exhaustive over the enum so a new variant forces a copy decision here
        // instead of silently falling back to a Debug render in the UI.
        for status in [
            AssetLockStatus::Built,
            AssetLockStatus::Broadcast,
            AssetLockStatus::InstantSendLocked,
            AssetLockStatus::ChainLocked,
            AssetLockStatus::Consumed,
        ] {
            let label = asset_lock_status_label(&status);
            assert!(label.ends_with('.'), "label should be a sentence: {label}");
            let debug = format!("{status:?}");
            assert_ne!(label, debug, "label must not be the Debug repr");
            assert!(
                !label.contains("AssetLockStatus") && !label.contains("InstantSendLocked"),
                "label must not leak enum jargon: {label}"
            );
        }
    }

    /// DOC-2: the hint and the QR URI must agree, and never ask for less than
    /// the true minimum — so a value with a 5th decimal digit rounds up, and one
    /// already at 4dp is left unchanged.
    #[test]
    fn round_up_dash_4dp_never_understates_the_minimum() {
        assert_eq!(format!("{:.4}", round_up_dash_4dp(0.00011)), "0.0002");
        assert_eq!(format!("{:.4}", round_up_dash_4dp(0.00019)), "0.0002");
        assert_eq!(format!("{:.4}", round_up_dash_4dp(0.0001)), "0.0001");
        assert_eq!(format!("{:.4}", round_up_dash_4dp(0.0)), "0.0000");
    }

    #[test]
    fn exact_balance_covers_minimum() {
        let minimum_credits = 10 * CREDITS_PER_DUFF;
        assert!(spendable_covers_minimum(10, minimum_credits));
    }

    #[test]
    fn one_credit_short_of_minimum_is_insufficient() {
        let minimum_credits = 10 * CREDITS_PER_DUFF + 1;
        assert!(!spendable_covers_minimum(10, minimum_credits));
    }

    #[test]
    fn one_credit_above_minimum_is_sufficient() {
        let minimum_credits = 10 * CREDITS_PER_DUFF - 1;
        assert!(spendable_covers_minimum(10, minimum_credits));
    }

    #[test]
    fn zero_spendable_never_covers_a_positive_minimum() {
        assert!(!spendable_covers_minimum(0, 1));
    }

    #[test]
    fn conversion_does_not_overflow_on_extreme_values() {
        assert!(spendable_covers_minimum(u64::MAX, u64::MAX));
    }

    #[test]
    fn max_amount_reserves_fee_from_spendable_duffs() {
        let spendable_duffs = 10;
        let fee_credits = 500;
        assert_eq!(
            max_amount_after_fee_reserve(spendable_duffs, fee_credits),
            spendable_duffs * CREDITS_PER_DUFF - fee_credits
        );
    }

    #[test]
    fn max_amount_saturates_to_zero_when_fee_exceeds_spendable() {
        assert_eq!(max_amount_after_fee_reserve(1, u64::MAX), 0);
    }

    #[test]
    fn max_amount_does_not_overflow_on_extreme_values() {
        assert_eq!(max_amount_after_fee_reserve(u64::MAX, 0), u64::MAX);
    }

    /// A wallet with spendable balance defaults to the recommended path,
    /// pre-selected and ready to go.
    #[test]
    fn default_funding_state_prefers_wallet_balance_when_available() {
        assert_eq!(
            default_funding_state(true),
            (
                FundingMethod::UseWalletBalance,
                WalletFundedScreenStep::ReadyToCreate
            )
        );
    }

    /// A wallet with nothing to fund from must not default to a method the
    /// ComboBox itself wouldn't offer — that was the fresh-wallet dead end. It
    /// starts unselected instead.
    #[test]
    fn default_funding_state_falls_back_to_no_selection_without_balance() {
        assert_eq!(
            default_funding_state(false),
            (
                FundingMethod::NoSelection,
                WalletFundedScreenStep::ChooseFundingMethod
            )
        );
    }

    /// Exhaustive over the enum so a new variant forces a copy decision here
    /// instead of silently falling back to a Debug render in the UI.
    #[test]
    fn display_is_jargon_free_for_every_variant() {
        for method in [
            FundingMethod::NoSelection,
            FundingMethod::UseUnusedAssetLock,
            FundingMethod::UseWalletBalance,
            FundingMethod::UsePlatformAddress,
            FundingMethod::ReceiveDeposit,
        ] {
            let label = format!("{method}");
            let debug = format!("{method:?}");
            assert_ne!(label, debug, "label must not be the Debug repr");
            assert!(
                !label.contains("Asset Lock") && !label.contains("asset lock"),
                "label must not leak asset-lock jargon: {label}"
            );
        }
    }

    #[test]
    fn use_wallet_balance_is_the_recommended_primary_path() {
        assert_eq!(
            format!("{}", FundingMethod::UseWalletBalance),
            "From your wallet (recommended)"
        );
    }

    /// Top-Up's asset-lock label must not describe "recovering" a funding
    /// setup — an identity being topped up already exists and was never
    /// mid-creation. Every other variant keeps `Display`'s wording.
    #[test]
    fn top_up_label_differs_only_for_asset_lock() {
        assert_eq!(
            FundingMethod::UseUnusedAssetLock.top_up_label(),
            "Use an existing funding transaction"
        );
        assert_ne!(
            FundingMethod::UseUnusedAssetLock.top_up_label(),
            format!("{}", FundingMethod::UseUnusedAssetLock)
        );

        for method in [
            FundingMethod::NoSelection,
            FundingMethod::UseWalletBalance,
            FundingMethod::UsePlatformAddress,
            FundingMethod::ReceiveDeposit,
        ] {
            assert_eq!(method.top_up_label(), format!("{method}"));
        }
    }

    /// F3 case 1: with no explicit choice yet, a wallet switch re-applies the
    /// screen's default pre-selection for the new wallet. Switching from a
    /// funded wallet to an unfunded one drops the stale `UseWalletBalance`
    /// pre-selection back to `NoSelection` instead of keeping a method the new
    /// wallet can't fund.
    #[test]
    fn switch_without_explicit_choice_recomputes_default() {
        let stale = (
            FundingMethod::UseWalletBalance,
            WalletFundedScreenStep::ReadyToCreate,
        );
        assert_eq!(
            funding_method_after_switch(false, stale, false),
            (
                FundingMethod::NoSelection,
                WalletFundedScreenStep::ChooseFundingMethod
            )
        );
        // And onto a funded wallet it recommends the wallet-balance path.
        assert_eq!(
            funding_method_after_switch(
                false,
                (
                    FundingMethod::NoSelection,
                    WalletFundedScreenStep::ChooseFundingMethod
                ),
                true
            ),
            (
                FundingMethod::UseWalletBalance,
                WalletFundedScreenStep::ReadyToCreate
            )
        );
    }

    /// F3 case 2: once the user has explicitly chosen a method, a wallet switch
    /// preserves it untouched — even when the new wallet's balance would have
    /// produced a different default.
    #[test]
    fn switch_after_explicit_choice_preserves_selection() {
        let chosen = (
            FundingMethod::UseUnusedAssetLock,
            WalletFundedScreenStep::ReadyToCreate,
        );
        assert_eq!(funding_method_after_switch(true, chosen, true), chosen);
        assert_eq!(funding_method_after_switch(true, chosen, false), chosen);
    }

    use dash_sdk::dpp::dashcore::PublicKey;
    use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};

    /// A distinct testnet p2pkh address keyed off `n` (derived from a valid
    /// secret key so the pubkey is a real curve point).
    fn addr(n: u8) -> Address {
        let mut sk_bytes = [1u8; 32];
        sk_bytes[31] = n.max(1);
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&sk_bytes).expect("valid secret key");
        let pubkey = PublicKey::new(sk.public_key(&secp));
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// One received output of `value` duffs paying `address`, with a placeholder
    /// outpoint (the helpers ignore the outpoint).
    fn output(value: u64, address: &Address) -> (OutPoint, TxOut, Address) {
        (
            OutPoint::null(),
            TxOut {
                value,
                script_pubkey: address.script_pubkey(),
            },
            address.clone(),
        )
    }

    /// TC-QRFUND-04: a single output to the shown address is credited in full.
    #[test]
    fn deposit_matches_sums_outputs_to_the_shown_address() {
        let shown = addr(1);
        let outputs = [output(100_000, &shown)];
        assert_eq!(deposit_matches(Some(&shown), &outputs), 100_000);
    }

    /// TC-QRFUND-05: several outputs to the shown address in one event accumulate.
    /// Cross-event accumulation is the wallet snapshot's job, not this helper's.
    #[test]
    fn deposit_matches_accumulates_multiple_matching_outputs() {
        let shown = addr(1);
        let outputs = [output(40_000, &shown), output(60_000, &shown)];
        assert_eq!(deposit_matches(Some(&shown), &outputs), 100_000);
    }

    /// TC-QRFUND-06: a deposit to a different address is not credited — detection
    /// is single-address equality, never wallet-membership.
    #[test]
    fn deposit_matches_ignores_other_addresses() {
        let shown = addr(1);
        let other = addr(2);
        let outputs = [output(100_000, &other)];
        assert_eq!(deposit_matches(Some(&shown), &outputs), 0);
    }

    /// With no address shown yet (address request still in flight), nothing matches.
    #[test]
    fn deposit_matches_returns_zero_without_a_shown_address() {
        let other = addr(2);
        let outputs = [output(100_000, &other)];
        assert_eq!(deposit_matches(None, &outputs), 0);
    }

    /// TC-QRFUND-04: a matching deposit that covers the minimum
    /// advances the wizard to the amount step.
    #[test]
    fn deposit_step_advances_when_matched_and_minimum_covered() {
        let shown = addr(1);
        let outputs = [output(100_000, &shown)];
        let minimum_credits = 100 * CREDITS_PER_DUFF;
        assert_eq!(
            deposit_step_after_utxo(
                WalletFundedScreenStep::WaitingOnFunds,
                Some(&shown),
                &outputs,
                minimum_credits,
            ),
            WalletFundedScreenStep::FundsReceived
        );
    }

    /// TC-QRFUND-05: a matching but still-sub-minimum deposit keeps the wizard
    /// waiting; only crossing the minimum advances it.
    #[test]
    fn deposit_step_stays_waiting_below_minimum() {
        let shown = addr(1);
        let outputs = [output(40, &shown)];
        let minimum_credits = 100 * CREDITS_PER_DUFF;
        assert_eq!(
            deposit_step_after_utxo(
                WalletFundedScreenStep::WaitingOnFunds,
                Some(&shown),
                &outputs,
                minimum_credits,
            ),
            WalletFundedScreenStep::WaitingOnFunds
        );
    }

    /// TC-QRFUND-06: a deposit to a different address never advances the wizard,
    /// even when unrelated wallet funds happen to cover the minimum.
    #[test]
    fn deposit_step_stays_waiting_for_other_address() {
        let shown = addr(1);
        let other = addr(2);
        let outputs = [output(100_000, &other)];
        let minimum_credits = 100 * CREDITS_PER_DUFF;
        assert_eq!(
            deposit_step_after_utxo(
                WalletFundedScreenStep::WaitingOnFunds,
                Some(&shown),
                &outputs,
                minimum_credits,
            ),
            WalletFundedScreenStep::WaitingOnFunds
        );
    }

    /// TC-QRFUND-07: the deposit guard is scoped to the waiting state — a
    /// matching deposit arriving while another method is active (here
    /// `ReadyToCreate`) never spuriously advances to `FundsReceived`.
    #[test]
    fn deposit_step_ignores_events_outside_waiting_state() {
        let shown = addr(1);
        let outputs = [output(100_000, &shown)];
        let minimum_credits = 100 * CREDITS_PER_DUFF;
        for step in [
            WalletFundedScreenStep::ChooseFundingMethod,
            WalletFundedScreenStep::ReadyToCreate,
            WalletFundedScreenStep::FundsReceived,
            WalletFundedScreenStep::WaitingForAssetLock,
        ] {
            assert_eq!(
                deposit_step_after_utxo(step, Some(&shown), &outputs, minimum_credits),
                step,
                "guard must not change step {step:?}"
            );
        }
    }

    /// Bug 1 regression: when a sufficient deposit advances the wizard to
    /// `FundsReceived`, the amount to pre-fill is the fee-reserve-capped balance
    /// and is NON-zero — the bug left the field empty (no Create button until the
    /// user typed or clicked Max). Guards the pre-fill on the advancing event.
    #[test]
    fn advancing_deposit_yields_a_nonzero_prefill_amount() {
        let shown = addr(1);
        let outputs = [output(100_000, &shown)];
        let fee_credits = 100 * CREDITS_PER_DUFF;
        let (next, prefill) = deposit_event_outcome(
            WalletFundedScreenStep::WaitingOnFunds,
            Some(&shown),
            &outputs,
            fee_credits,
        );
        assert_eq!(next, WalletFundedScreenStep::FundsReceived);
        assert_eq!(
            prefill,
            Some(max_amount_after_fee_reserve(100_000, fee_credits))
        );
        assert!(
            prefill.unwrap() > 0,
            "the amount must be pre-filled, not left at zero"
        );
    }

    /// A sub-minimum deposit neither advances nor pre-fills — the amount is
    /// populated only once the wizard actually reaches `FundsReceived`.
    #[test]
    fn below_minimum_deposit_yields_no_prefill() {
        let shown = addr(1);
        let outputs = [output(40, &shown)];
        let fee_credits = 100 * CREDITS_PER_DUFF;
        let (next, prefill) = deposit_event_outcome(
            WalletFundedScreenStep::WaitingOnFunds,
            Some(&shown),
            &outputs,
            fee_credits,
        );
        assert_eq!(next, WalletFundedScreenStep::WaitingOnFunds);
        assert_eq!(prefill, None);
    }

    #[test]
    fn unrelated_wallet_balance_does_not_complete_a_partial_deposit() {
        let shown = addr(1);
        let outputs = [output(1, &shown)];
        let minimum_credits = 50_000_000;

        assert_eq!(
            snapshot_deposit_outcome(WalletFundedScreenStep::WaitingOnFunds, 1, minimum_credits,),
            (WalletFundedScreenStep::WaitingOnFunds, None),
        );
        assert_eq!(
            deposit_event_outcome(
                WalletFundedScreenStep::WaitingOnFunds,
                Some(&shown),
                &outputs,
                minimum_credits,
            ),
            (WalletFundedScreenStep::WaitingOnFunds, None),
        );
    }

    #[test]
    fn shared_address_request_and_failure_helpers_cover_both_identity_flows() {
        assert!(!should_queue_funding_address(false, false, true, false));
        assert!(should_queue_funding_address(false, false, false, false));
        assert_eq!(
            step_after_task_failure(WalletFundedScreenStep::WaitingOnFunds),
            WalletFundedScreenStep::WaitingOnFunds,
        );
        assert_eq!(
            step_after_task_failure(WalletFundedScreenStep::WaitingForAssetLock),
            WalletFundedScreenStep::ReadyToCreate,
        );
    }

    /// A deposit to a different address never advances, so it never pre-fills.
    #[test]
    fn deposit_to_other_address_yields_no_prefill() {
        let shown = addr(1);
        let other = addr(2);
        let outputs = [output(100_000, &other)];
        let fee_credits = 100 * CREDITS_PER_DUFF;
        let (next, prefill) = deposit_event_outcome(
            WalletFundedScreenStep::WaitingOnFunds,
            Some(&shown),
            &outputs,
            fee_credits,
        );
        assert_eq!(next, WalletFundedScreenStep::WaitingOnFunds);
        assert_eq!(prefill, None);
    }
}
