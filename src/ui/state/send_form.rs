//! Choices on the Send form: who pays, who receives, and how.
//!
//! Renders nothing. The Send screen owns one [`SendFormState`], renders it,
//! and each frame resolves it — with the pure rules in
//! [`crate::model::send_route`] — into the one concrete source and destination
//! its route functions validate and dispatch.
//!
//! Invariants:
//! - Changing a From choice never clears the To value, and the reverse
//!   (spec SR-16; fixes the old screen's lost-destination bug UX-001).
//! - Auto is on at most one side. Choosing Auto on one side while the other
//!   side is on Auto pins the other side to a concrete choice and leaves a
//!   one-shot notice there (spec §4.3); the side touched last wins.
//! - "Max" is an intent that holds until the amount is edited.

use crate::model::send_route::PayFromKind;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::platform::Identifier;
use std::collections::BTreeMap;

/// Fee strategy for payments built from explicit Platform inputs or outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlatformFeeStrategy {
    /// Deduct fee from first input
    #[default]
    DeductFromFirstInput,
    /// Deduct fee from last input
    DeductFromLastInput,
    /// Reduce first output by fee amount
    ReduceFirstOutput,
    /// Reduce last output by fee amount
    ReduceLastOutput,
}

impl PlatformFeeStrategy {
    /// Every strategy, in the order the form lists them.
    pub const ALL: [Self; 4] = [
        Self::DeductFromFirstInput,
        Self::DeductFromLastInput,
        Self::ReduceFirstOutput,
        Self::ReduceLastOutput,
    ];

    /// Whether the recipient receives less than the entered amount.
    pub fn reduces_output(self) -> bool {
        matches!(self, Self::ReduceFirstOutput | Self::ReduceLastOutput)
    }
}

impl std::fmt::Display for PlatformFeeStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeductFromFirstInput => write!(f, "Deduct from first input"),
            Self::DeductFromLastInput => write!(f, "Deduct from last input"),
            Self::ReduceFirstOutput => write!(f, "Reduce first output"),
            Self::ReduceLastOutput => write!(f, "Reduce last output"),
        }
    }
}

/// Which kind of addresses the Advanced source spends from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdvancedSourceType {
    Core,
    Platform,
}

impl AdvancedSourceType {
    /// The balance kind these addresses belong to.
    pub fn pay_from_kind(self) -> PayFromKind {
        match self {
            Self::Core => PayFromKind::Core,
            Self::Platform => PayFromKind::Platform,
        }
    }
}

impl std::fmt::Display for AdvancedSourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core => write!(f, "Core addresses"),
            Self::Platform => write!(f, "Platform addresses"),
        }
    }
}

/// The "Pay from" choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayFrom {
    Auto,
    Core,
    Platform,
    Identity,
    Shielded,
    /// Spend only from addresses the user ticks.
    Advanced,
}

impl PayFrom {
    /// The concrete choice for a balance kind.
    pub fn from_kind(kind: PayFromKind) -> Self {
        match kind {
            PayFromKind::Core => Self::Core,
            PayFromKind::Platform => Self::Platform,
            PayFromKind::Identity => Self::Identity,
            PayFromKind::Shielded => Self::Shielded,
        }
    }
}

/// Whether To pays someone else or one of the user's own wallets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToMode {
    SomeoneElse,
    MyWallets,
}

/// The "Receive into" choice under My wallets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveInto {
    Auto,
    Core,
    Platform,
    Identity,
    Shielded,
}

impl ReceiveInto {
    /// The concrete choice for a balance kind.
    pub fn from_kind(kind: PayFromKind) -> Self {
        match kind {
            PayFromKind::Core => Self::Core,
            PayFromKind::Platform => Self::Platform,
            PayFromKind::Identity => Self::Identity,
            PayFromKind::Shielded => Self::Shielded,
        }
    }

    /// The balance kind of a concrete choice; `None` for Auto.
    pub fn kind(self) -> Option<PayFromKind> {
        match self {
            Self::Auto => None,
            Self::Core => Some(PayFromKind::Core),
            Self::Platform => Some(PayFromKind::Platform),
            Self::Identity => Some(PayFromKind::Identity),
            Self::Shielded => Some(PayFromKind::Shielded),
        }
    }
}

/// The one-shot line left under a side that Auto on the other side pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinNotice {
    /// Pay from was pinned to this balance so Receive into can choose.
    PayFromPinned(PayFromKind),
    /// Receive into was pinned to this balance so Pay from can choose.
    ReceiveIntoPinned(PayFromKind),
}

/// One ticked-or-not address of the Advanced source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdvancedRow {
    pub ticked: bool,
    /// Optional amount to take from this address, as typed. Blank everywhere
    /// spreads the payment over the ticked addresses.
    pub amount: String,
}

/// One recipient row when paying several recipients.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientRow {
    pub address: String,
    pub amount: String,
}

/// Which parts of the form an entry point fixes (spec §11).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormLocks {
    /// Pay from is set by the entry point and shown read-only.
    pub pay_from: bool,
    /// To is set by the entry point and shown read-only.
    pub to: bool,
    /// Receive into is fixed (the To mode may still change).
    pub receive_into: bool,
    /// The To mode (Someone else / My wallets) is fixed.
    pub to_mode: bool,
}

/// Where an entry point opens the form (spec §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormStart {
    pub pay_from: PayFrom,
    pub to_mode: ToMode,
    pub receive_into: ReceiveInto,
    pub locks: FormLocks,
}

impl Default for FormStart {
    /// Wallet › Send: Auto, paying someone else.
    fn default() -> Self {
        Self {
            pay_from: PayFrom::Auto,
            to_mode: ToMode::SomeoneElse,
            receive_into: ReceiveInto::Core,
            locks: FormLocks::default(),
        }
    }
}

/// The Send form's choices.
#[derive(Debug, Clone)]
pub struct SendFormState {
    start: FormStart,
    pay_from: PayFrom,
    pay_identity: Option<Identifier>,
    to_mode: ToMode,
    receive_into: ReceiveInto,
    receive_into_chosen: bool,
    receive_identity: Option<Identifier>,
    to_wallet: Option<WalletSeedHash>,
    pin_notice: Option<PinNotice>,
    /// Pay from holds a pin set by Auto under Receive into, not a user choice.
    pay_from_pinned: bool,
    /// Receive into holds a pin set by Auto under Pay from, not a user choice.
    receive_into_pinned: bool,
    max_intent: bool,
    recipients: Vec<RecipientRow>,
    advanced_type: AdvancedSourceType,
    advanced_rows: BTreeMap<(AdvancedSourceType, String), AdvancedRow>,
    fee_strategy: PlatformFeeStrategy,
}

impl Default for SendFormState {
    fn default() -> Self {
        Self::new(FormStart::default())
    }
}

impl SendFormState {
    pub fn new(start: FormStart) -> Self {
        Self {
            start,
            pay_from: start.pay_from,
            pay_identity: None,
            to_mode: start.to_mode,
            receive_into: start.receive_into,
            receive_into_chosen: start.locks.receive_into || start.to_mode == ToMode::MyWallets,
            receive_identity: None,
            to_wallet: None,
            pin_notice: None,
            pay_from_pinned: false,
            receive_into_pinned: false,
            max_intent: false,
            recipients: Vec::new(),
            advanced_type: AdvancedSourceType::Core,
            advanced_rows: BTreeMap::new(),
            fee_strategy: PlatformFeeStrategy::default(),
        }
    }

    /// Back to where the entry point opened the form.
    pub fn reset(&mut self) {
        *self = Self::new(self.start);
    }

    pub fn locks(&self) -> FormLocks {
        self.start.locks
    }

    pub fn pay_from(&self) -> PayFrom {
        self.pay_from
    }

    pub fn pay_identity(&self) -> Option<Identifier> {
        self.pay_identity
    }

    pub fn to_mode(&self) -> ToMode {
        self.to_mode
    }

    pub fn receive_into(&self) -> ReceiveInto {
        self.receive_into
    }

    pub fn receive_identity(&self) -> Option<Identifier> {
        self.receive_identity
    }

    pub fn to_wallet(&self) -> Option<WalletSeedHash> {
        self.to_wallet
    }

    pub fn pin_notice(&self) -> Option<PinNotice> {
        self.pin_notice
    }

    pub fn max_intent(&self) -> bool {
        self.max_intent
    }

    pub fn fee_strategy(&self) -> PlatformFeeStrategy {
        self.fee_strategy
    }

    pub fn advanced_type(&self) -> AdvancedSourceType {
        self.advanced_type
    }

    /// The balance an explicit Pay from choice spends; `None` for Auto.
    pub fn explicit_source_kind(&self) -> Option<PayFromKind> {
        match self.pay_from {
            PayFrom::Auto => None,
            PayFrom::Core => Some(PayFromKind::Core),
            PayFrom::Platform => Some(PayFromKind::Platform),
            PayFrom::Identity => Some(PayFromKind::Identity),
            PayFrom::Shielded => Some(PayFromKind::Shielded),
            PayFrom::Advanced => Some(self.advanced_type.pay_from_kind()),
        }
    }

    /// Whether Receive into is on Auto for a My wallets destination.
    pub fn destination_is_auto(&self) -> bool {
        self.to_mode == ToMode::MyWallets && self.receive_into == ReceiveInto::Auto
    }

    /// Choose Pay from. Choosing Auto while Receive into is on Auto pins
    /// Receive into to `pin_receive_into` — what its Auto had resolved to, or
    /// its default. Leaving Auto releases such a pin, since a pin is never the
    /// user's choice. Never touches the To value.
    pub fn choose_pay_from(&mut self, choice: PayFrom, pin_receive_into: PayFromKind) {
        if self.start.locks.pay_from {
            return;
        }
        self.pin_notice = None;
        self.pay_from_pinned = false;
        if choice == PayFrom::Auto && self.destination_is_auto() && !self.start.locks.receive_into {
            self.receive_into = ReceiveInto::from_kind(pin_receive_into);
            self.receive_into_pinned = true;
            self.pin_notice = Some(PinNotice::ReceiveIntoPinned(pin_receive_into));
        } else if choice != PayFrom::Auto && self.receive_into_pinned {
            self.receive_into = ReceiveInto::Auto;
            self.receive_into_pinned = false;
        }
        self.pay_from = choice;
    }

    /// Choose the identity that pays.
    pub fn choose_pay_identity(&mut self, identity: Identifier) {
        self.pin_notice = None;
        self.pay_identity = Some(identity);
    }

    /// Switch between Someone else and My wallets. The first switch to My
    /// wallets picks `default_receive_into` and the From wallet; later
    /// switches keep what the user chose.
    pub fn choose_to_mode(
        &mut self,
        mode: ToMode,
        from_wallet: Option<WalletSeedHash>,
        default_receive_into: PayFromKind,
    ) {
        if self.start.locks.to || self.start.locks.to_mode {
            return;
        }
        self.pin_notice = None;
        self.to_mode = mode;
        // A Pay from pin only lets Receive into choose under My wallets.
        // Releasing it puts Pay from back on Auto, so Receive into takes its
        // default again on the way back instead of a second Auto.
        if mode == ToMode::SomeoneElse && self.pay_from_pinned {
            self.pay_from = PayFrom::Auto;
            self.pay_from_pinned = false;
            self.receive_into_chosen = false;
        }
        if mode == ToMode::MyWallets {
            if self.to_wallet.is_none() {
                self.to_wallet = from_wallet;
            }
            if !self.receive_into_chosen {
                self.receive_into = ReceiveInto::from_kind(default_receive_into);
                self.receive_into_chosen = true;
            }
        }
    }

    /// Choose the wallet that receives under My wallets.
    pub fn choose_to_wallet(&mut self, wallet: WalletSeedHash) {
        if self.start.locks.to {
            return;
        }
        self.pin_notice = None;
        self.to_wallet = Some(wallet);
    }

    /// Choose Receive into. Choosing Auto while Pay from is on Auto pins Pay
    /// from to `pin_pay_from` — what its Auto had resolved to, or its default.
    /// Leaving Auto releases such a pin.
    pub fn choose_receive_into(&mut self, choice: ReceiveInto, pin_pay_from: PayFromKind) {
        if self.start.locks.to || self.start.locks.receive_into {
            return;
        }
        self.pin_notice = None;
        self.receive_into_chosen = true;
        self.receive_into_pinned = false;
        if choice == ReceiveInto::Auto
            && self.pay_from == PayFrom::Auto
            && !self.start.locks.pay_from
        {
            self.pay_from = PayFrom::from_kind(pin_pay_from);
            self.pay_from_pinned = true;
            self.pin_notice = Some(PinNotice::PayFromPinned(pin_pay_from));
        } else if choice != ReceiveInto::Auto && self.pay_from_pinned {
            self.pay_from = PayFrom::Auto;
            self.pay_from_pinned = false;
        }
        self.receive_into = choice;
    }

    /// Choose the identity that receives under My wallets.
    pub fn choose_receive_identity(&mut self, identity: Identifier) {
        self.pin_notice = None;
        self.receive_identity = Some(identity);
    }

    /// Set the identity choices without counting as a user change, e.g. to
    /// seed the default identity.
    pub fn seed_identities(&mut self, pay: Option<Identifier>, receive: Option<Identifier>) {
        if self.pay_identity.is_none() {
            self.pay_identity = pay;
        }
        if self.receive_identity.is_none() {
            self.receive_identity = receive;
        }
    }

    /// Note any other edit of the form: the one-shot pin notice goes away.
    pub fn touched(&mut self) {
        self.pin_notice = None;
    }

    /// "Max" was pressed.
    pub fn press_max(&mut self) {
        self.pin_notice = None;
        self.max_intent = true;
    }

    /// The amount was edited, so it is exact again.
    pub fn amount_edited(&mut self) {
        self.pin_notice = None;
        self.max_intent = false;
    }

    // --- Several recipients -------------------------------------------------

    pub fn recipients(&self) -> &[RecipientRow] {
        &self.recipients
    }

    pub fn recipients_mut(&mut self) -> &mut [RecipientRow] {
        &mut self.recipients
    }

    /// Whether the form pays several recipients (two or more rows).
    pub fn has_several_recipients(&self) -> bool {
        self.recipients.len() >= 2
    }

    /// Add a recipient row. From the single-recipient layout, the current
    /// recipient and amount become the first row.
    pub fn add_recipient(&mut self, current_address: String, current_amount: String) {
        self.pin_notice = None;
        self.max_intent = false;
        if self.recipients.is_empty() {
            self.recipients.push(RecipientRow {
                address: current_address,
                amount: current_amount,
            });
        }
        self.recipients.push(RecipientRow::default());
    }

    /// Remove a recipient row. When one row is left the form returns to the
    /// single-recipient layout and the survivor is returned so its address
    /// and amount carry over.
    pub fn remove_recipient(&mut self, index: usize) -> Option<RecipientRow> {
        self.pin_notice = None;
        if index < self.recipients.len() {
            self.recipients.remove(index);
        }
        if self.recipients.len() == 1 {
            return self.recipients.pop();
        }
        None
    }

    // --- Advanced source ----------------------------------------------------

    pub fn choose_advanced_type(&mut self, kind: AdvancedSourceType) {
        self.pin_notice = None;
        self.advanced_type = kind;
    }

    /// The Advanced row for `address` of the current type.
    pub fn advanced_row(&self, address: &str) -> AdvancedRow {
        self.advanced_rows
            .get(&(self.advanced_type, address.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Tick or untick `address` of the current type.
    pub fn set_advanced_ticked(&mut self, address: &str, ticked: bool) {
        self.pin_notice = None;
        self.max_intent = false;
        self.advanced_rows
            .entry((self.advanced_type, address.to_string()))
            .or_default()
            .ticked = ticked;
    }

    /// Set the amount typed for `address` of the current type.
    pub fn set_advanced_amount(&mut self, address: &str, amount: String) {
        self.pin_notice = None;
        self.advanced_rows
            .entry((self.advanced_type, address.to_string()))
            .or_default()
            .amount = amount;
    }

    /// Ticked addresses of the current type, with their typed amounts.
    pub fn ticked_advanced(&self) -> Vec<(String, String)> {
        self.advanced_rows
            .iter()
            .filter(|((kind, _), row)| *kind == self.advanced_type && row.ticked)
            .map(|((_, address), row)| (address.clone(), row.amount.clone()))
            .collect()
    }

    /// Forget ticks of addresses no longer offered (spent or gone).
    pub fn retain_advanced(&mut self, offered: &[String]) {
        let kind = self.advanced_type;
        self.advanced_rows
            .retain(|(row_kind, address), _| *row_kind != kind || offered.contains(address));
    }

    pub fn choose_fee_strategy(&mut self, strategy: PlatformFeeStrategy) {
        self.pin_notice = None;
        self.fee_strategy = strategy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: WalletSeedHash = [1; 32];
    const SAVINGS: WalletSeedHash = [2; 32];

    #[test]
    fn opens_on_auto_paying_someone_else() {
        let form = SendFormState::default();
        assert_eq!(form.pay_from(), PayFrom::Auto);
        assert_eq!(form.to_mode(), ToMode::SomeoneElse);
        assert_eq!(form.pin_notice(), None);
        assert!(!form.max_intent());
    }

    #[test]
    fn switching_to_my_wallets_defaults_the_wallet_and_kind_once() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Platform);
        assert_eq!(form.to_wallet(), Some(MAIN));
        assert_eq!(form.receive_into(), ReceiveInto::Platform);

        form.choose_receive_into(ReceiveInto::Core, PayFromKind::Core);
        form.choose_to_wallet(SAVINGS);
        form.choose_to_mode(ToMode::SomeoneElse, Some(MAIN), PayFromKind::Platform);
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Platform);
        assert_eq!(
            form.receive_into(),
            ReceiveInto::Core,
            "the user's choice survives"
        );
        assert_eq!(form.to_wallet(), Some(SAVINGS));
    }

    #[test]
    fn auto_under_receive_into_pins_pay_from() {
        // AC-04 / W6.
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Core);
        assert_eq!(form.receive_into(), ReceiveInto::Auto);
        assert_eq!(
            form.pin_notice(),
            Some(PinNotice::PayFromPinned(PayFromKind::Core))
        );
    }

    #[test]
    fn auto_under_pay_from_pins_receive_into_back() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        // The last-touched side wins: Auto on Pay from flips the pin.
        form.choose_pay_from(PayFrom::Auto, PayFromKind::Platform);
        assert_eq!(form.pay_from(), PayFrom::Auto);
        assert_eq!(form.receive_into(), ReceiveInto::Platform);
        assert_eq!(
            form.pin_notice(),
            Some(PinNotice::ReceiveIntoPinned(PayFromKind::Platform))
        );
    }

    #[test]
    fn auto_is_never_pinned_while_paying_someone_else() {
        let mut form = SendFormState::default();
        form.choose_pay_from(PayFrom::Core, PayFromKind::Core);
        form.choose_pay_from(PayFrom::Auto, PayFromKind::Platform);
        assert_eq!(form.pay_from(), PayFrom::Auto);
        assert_eq!(form.pin_notice(), None);
    }

    #[test]
    fn the_pin_notice_goes_away_on_the_next_change() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        assert!(form.pin_notice().is_some());
        form.amount_edited();
        assert_eq!(form.pin_notice(), None);
    }

    #[test]
    fn changing_pay_from_keeps_the_destination_choices() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_to_wallet(SAVINGS);
        form.choose_receive_into(ReceiveInto::Platform, PayFromKind::Core);
        for choice in [
            PayFrom::Core,
            PayFrom::Platform,
            PayFrom::Identity,
            PayFrom::Advanced,
        ] {
            form.choose_pay_from(choice, PayFromKind::Core);
            assert_eq!(form.to_mode(), ToMode::MyWallets);
            assert_eq!(form.to_wallet(), Some(SAVINGS));
            assert_eq!(form.receive_into(), ReceiveInto::Platform);
        }
    }

    #[test]
    fn max_holds_until_the_amount_is_edited() {
        let mut form = SendFormState::default();
        form.press_max();
        form.choose_pay_from(PayFrom::Core, PayFromKind::Core);
        assert!(form.max_intent(), "changing the source keeps the intent");
        form.amount_edited();
        assert!(!form.max_intent());
    }

    #[test]
    fn a_fixed_side_ignores_changes() {
        let mut form = SendFormState::new(FormStart {
            pay_from: PayFrom::Shielded,
            to_mode: ToMode::SomeoneElse,
            receive_into: ReceiveInto::Core,
            locks: FormLocks {
                pay_from: true,
                ..FormLocks::default()
            },
        });
        form.choose_pay_from(PayFrom::Core, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Shielded);

        let mut form = SendFormState::new(FormStart {
            pay_from: PayFrom::Auto,
            to_mode: ToMode::MyWallets,
            receive_into: ReceiveInto::Shielded,
            locks: FormLocks {
                to: true,
                ..FormLocks::default()
            },
        });
        form.choose_to_mode(ToMode::SomeoneElse, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        assert_eq!(form.to_mode(), ToMode::MyWallets);
        assert_eq!(form.receive_into(), ReceiveInto::Shielded);
        assert_eq!(
            form.pay_from(),
            PayFrom::Auto,
            "a fixed To never pins Pay from"
        );
    }

    #[test]
    fn a_fixed_pay_from_is_never_pinned() {
        let mut form = SendFormState::new(FormStart {
            pay_from: PayFrom::Shielded,
            to_mode: ToMode::MyWallets,
            receive_into: ReceiveInto::Platform,
            locks: FormLocks {
                pay_from: true,
                ..FormLocks::default()
            },
        });
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Shielded);
        assert_eq!(form.receive_into(), ReceiveInto::Auto);
        assert_eq!(form.pin_notice(), None);
    }

    #[test]
    fn reset_returns_to_the_entry_point() {
        let start = FormStart {
            pay_from: PayFrom::Shielded,
            to_mode: ToMode::MyWallets,
            receive_into: ReceiveInto::Auto,
            locks: FormLocks {
                pay_from: true,
                ..FormLocks::default()
            },
        };
        let mut form = SendFormState::new(start);
        form.choose_to_mode(ToMode::SomeoneElse, Some(MAIN), PayFromKind::Core);
        form.press_max();
        form.reset();
        assert_eq!(form.to_mode(), ToMode::MyWallets);
        assert_eq!(form.receive_into(), ReceiveInto::Auto);
        assert!(!form.max_intent());
    }

    #[test]
    fn several_recipients_start_from_the_current_one_and_collapse_back() {
        let mut form = SendFormState::default();
        form.add_recipient("yX12".into(), "0.2".into());
        assert!(form.has_several_recipients());
        assert_eq!(
            form.recipients()[0],
            RecipientRow {
                address: "yX12".into(),
                amount: "0.2".into()
            }
        );
        form.add_recipient(String::new(), String::new());
        assert_eq!(form.recipients().len(), 3);
        assert_eq!(form.remove_recipient(1), None);
        let survivor = form.remove_recipient(1);
        assert_eq!(
            survivor,
            Some(RecipientRow {
                address: "yX12".into(),
                amount: "0.2".into()
            })
        );
        assert!(form.recipients().is_empty());
        assert!(!form.has_several_recipients());
    }

    #[test]
    fn advanced_ticks_are_kept_per_address_kind() {
        let mut form = SendFormState::default();
        form.set_advanced_ticked("yXa1", true);
        form.set_advanced_amount("yXa1", "0.5".into());
        form.choose_advanced_type(AdvancedSourceType::Platform);
        assert!(form.ticked_advanced().is_empty());
        form.set_advanced_ticked("tdash1q", true);
        form.choose_advanced_type(AdvancedSourceType::Core);
        assert_eq!(
            form.ticked_advanced(),
            vec![("yXa1".to_string(), "0.5".to_string())]
        );
        form.set_advanced_ticked("yXa1", false);
        assert!(form.ticked_advanced().is_empty());
        assert_eq!(
            form.advanced_row("yXa1").amount,
            "0.5",
            "unticking keeps the amount"
        );
    }

    #[test]
    fn advanced_forgets_addresses_no_longer_offered() {
        let mut form = SendFormState::default();
        form.set_advanced_ticked("gone", true);
        form.set_advanced_ticked("kept", true);
        form.retain_advanced(&["kept".to_string()]);
        assert_eq!(
            form.ticked_advanced(),
            vec![("kept".to_string(), String::new())]
        );
    }

    #[test]
    fn advanced_spends_the_kind_of_its_addresses() {
        let mut form = SendFormState::default();
        form.choose_pay_from(PayFrom::Advanced, PayFromKind::Core);
        assert_eq!(form.explicit_source_kind(), Some(PayFromKind::Core));
        form.choose_advanced_type(AdvancedSourceType::Platform);
        assert_eq!(form.explicit_source_kind(), Some(PayFromKind::Platform));
    }

    #[test]
    fn fee_strategy_defaults_to_the_first_input_and_survives_section_changes() {
        let mut form = SendFormState::default();
        assert_eq!(
            form.fee_strategy(),
            PlatformFeeStrategy::DeductFromFirstInput
        );
        form.choose_fee_strategy(PlatformFeeStrategy::ReduceLastOutput);
        form.choose_pay_from(PayFrom::Platform, PayFromKind::Core);
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        assert_eq!(form.fee_strategy(), PlatformFeeStrategy::ReduceLastOutput);
        assert!(form.fee_strategy().reduces_output());
    }

    #[test]
    fn a_pinned_pay_from_returns_to_auto_when_receive_into_leaves_auto() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Core, "pinned");

        form.choose_receive_into(ReceiveInto::Platform, PayFromKind::Core);
        assert_eq!(
            form.pay_from(),
            PayFrom::Auto,
            "the pin was the app's choice, not the user's"
        );
        assert_eq!(form.receive_into(), ReceiveInto::Platform);
    }

    #[test]
    fn a_pinned_receive_into_returns_to_auto_when_pay_from_leaves_auto() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        form.choose_pay_from(PayFrom::Auto, PayFromKind::Platform);
        assert_eq!(form.receive_into(), ReceiveInto::Platform, "pinned");

        form.choose_pay_from(PayFrom::Core, PayFromKind::Core);
        assert_eq!(form.receive_into(), ReceiveInto::Auto);
        assert_eq!(form.pay_from(), PayFrom::Core);
    }

    #[test]
    fn an_explicitly_chosen_side_is_never_switched_back() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_pay_from(PayFrom::Platform, PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Core, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Platform);

        // Picking another option on the pinned side makes it the user's own.
        form.choose_pay_from(PayFrom::Auto, PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Core, "pinned");
        form.choose_pay_from(PayFrom::Platform, PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Core, PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Platform);
    }

    /// A Pay from pin exists only so Receive into can choose under My
    /// wallets; leaving My wallets releases it.
    #[test]
    fn switching_to_someone_else_releases_the_pay_from_pin() {
        let mut form = SendFormState::default();
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        form.choose_receive_into(ReceiveInto::Auto, PayFromKind::Platform);
        assert_eq!(
            form.pay_from(),
            PayFrom::Platform,
            "sanity: Pay from is pinned"
        );

        form.choose_to_mode(ToMode::SomeoneElse, Some(MAIN), PayFromKind::Core);

        assert_eq!(form.pay_from(), PayFrom::Auto);

        // Back to My wallets: Receive into takes its default again, never
        // Auto beside Pay from's Auto.
        form.choose_to_mode(ToMode::MyWallets, Some(MAIN), PayFromKind::Core);
        assert_eq!(form.pay_from(), PayFrom::Auto);
        assert_eq!(
            form.receive_into(),
            ReceiveInto::from_kind(PayFromKind::Core)
        );
    }
}
