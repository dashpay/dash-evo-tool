use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::theme::DashColors;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{InnerResponse, Response, Ui, WidgetText};
use std::ops::Bound;
use std::sync::{Arc, RwLock};

/// Internal detection result including the `Unknown` state for unrecognized input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectedType {
    Core,
    Platform,
    Shielded,
    Identity,
    Unknown,
}

impl DetectedType {
    fn to_address_kind(self) -> Option<AddressKind> {
        match self {
            Self::Core => Some(AddressKind::Core),
            Self::Platform => Some(AddressKind::Platform),
            Self::Shielded => Some(AddressKind::Shielded),
            Self::Identity => Some(AddressKind::Identity),
            Self::Unknown => None,
        }
    }
}

/// A single autocomplete entry rendered in the dropdown.
///
/// Pre-computed from wallet/identity data at builder/setter time.
#[derive(Debug, Clone)]
struct AddressEntry {
    /// The full address string (populates the text field on selection).
    address_string: String,
    /// Classification of this entry.
    address_kind: AddressKind,
    /// Human-readable label (DPNS name, alias, or truncated address).
    display_label: String,
    /// Balance in native units (duffs for Core, credits for Platform/Shielded/Identity).
    balance: u64,
    /// Pre-built ValidatedAddress for immediate use on selection.
    validated: ValidatedAddress,
    /// Whether this is a change address (BIP44 m/44'/5'/0'/1/x).
    /// Only meaningful for Core addresses; always false for other types.
    /// Stored for potential future use in display styling; the "(change)"
    /// suffix is already baked into `display_label` at construction time.
    #[allow(dead_code)]
    is_change: bool,
}

/// Concrete balance range bounds.
///
/// `RangeBounds<u64>` is not object-safe, so we extract start/end bounds
/// at configuration time and store them concretely.
#[derive(Debug, Clone)]
struct BalanceRange {
    start: Bound<u64>,
    end: Bound<u64>,
}

impl BalanceRange {
    fn from_range(range: &impl std::ops::RangeBounds<u64>) -> Self {
        Self {
            start: range.start_bound().cloned(),
            end: range.end_bound().cloned(),
        }
    }

    fn contains(&self, value: u64) -> bool {
        let start_ok = match self.start {
            Bound::Included(s) => value >= s,
            Bound::Excluded(s) => value > s,
            Bound::Unbounded => true,
        };
        let end_ok = match self.end {
            Bound::Included(e) => value <= e,
            Bound::Excluded(e) => value < e,
            Bound::Unbounded => true,
        };
        start_ok && end_ok
    }
}

/// Response from the `AddressInput` component.
#[derive(Clone)]
pub struct AddressInputResponse {
    /// The egui response from the primary text input widget.
    pub response: Response,
    /// Whether the component's value changed this frame.
    changed: bool,
    /// Validation error message, if any.
    error_message: Option<String>,
    /// The validated address, if input is valid.
    validated_address: Option<ValidatedAddress>,
}

impl ComponentResponse for AddressInputResponse {
    type DomainType = ValidatedAddress;

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn is_valid(&self) -> bool {
        self.error_message.is_none()
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.validated_address
    }

    fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
}

/// Unified address input with autocomplete, type detection, and validation.
///
/// Follows the Component design pattern: lazy-initialize as `Option<AddressInput>`
/// in screen structs, configure via builder methods, render with `show()`,
/// bind to domain data with `response.inner.update(&mut self.address)`.
///
/// # Usage
///
/// ```rust,ignore
/// let addr_input = self.address_input.get_or_insert_with(|| {
///     AddressInput::new(network)
///         .with_wallets(&wallets)
///         .with_label("Destination address")
///         .with_hint_text("Enter address or username")
/// });
///
/// let response = addr_input.show(ui);
/// response.inner.update(&mut self.validated_address);
/// ```
pub struct AddressInput {
    // --- Configuration ---
    network: Network,
    enabled_kinds: Vec<AddressKind>,
    show_type_filter: bool,
    dpns_resolution: bool,
    developer_mode: bool,
    selection_only: bool,
    full_addresses: bool,
    label: Option<WidgetText>,
    hint_text: Option<String>,
    desired_width: Option<f32>,
    show_validation_errors: bool,
    balance_range: Option<BalanceRange>,
    exclude_change: bool,

    // --- Autocomplete data (set via builder, read each frame) ---
    all_entries: Vec<AddressEntry>,

    // --- Mutable UI state ---
    input_text: String,
    selected_type_filter: Option<AddressKind>,
    autocomplete_highlight: Option<usize>,
    autocomplete_open: bool,
    has_blurred: bool,
    selected_from_autocomplete: bool,
    cached_detection: Option<(String, DetectedType)>,
    changed: bool,
}

impl AddressInput {
    /// Create a new `AddressInput` for the given network.
    ///
    /// Default: all four address kinds enabled, no wallet data, no autocomplete.
    pub fn new(network: Network) -> Self {
        Self {
            network,
            enabled_kinds: AddressKind::ALL.to_vec(),
            show_type_filter: false,
            dpns_resolution: true,
            developer_mode: false,
            selection_only: false,
            full_addresses: false,
            label: None,
            hint_text: None,
            desired_width: None,
            show_validation_errors: true,
            balance_range: None,
            all_entries: Vec::new(),
            input_text: String::new(),
            selected_type_filter: None,
            autocomplete_highlight: None,
            autocomplete_open: false,
            has_blurred: false,
            selected_from_autocomplete: false,
            cached_detection: None,
            changed: false,
            exclude_change: false,
        }
    }

    /// Restrict which address kinds are accepted and shown.
    pub fn with_address_kinds(mut self, kinds: &[AddressKind]) -> Self {
        self.enabled_kinds = kinds.to_vec();
        self
    }

    /// Provide wallet data for Core and Platform autocomplete.
    ///
    /// Entries are extracted immediately (read lock acquired once per wallet).
    /// Skips gracefully if a wallet lock is poisoned.
    /// When more than one wallet is provided, entries are prefixed with the wallet alias.
    pub fn with_wallets(mut self, wallets: &[Arc<RwLock<Wallet>>]) -> Self {
        let multi = wallets.len() > 1;
        for wallet in wallets {
            self.extract_wallet_entries(wallet, multi);
        }
        self
    }

    /// Provide identity references for Identity-type autocomplete.
    pub fn with_identities(mut self, identities: &[QualifiedIdentity]) -> Self {
        self.extract_identity_entries(identities);
        self
    }

    /// Provide shielded address and balance for Shielded-type autocomplete.
    pub fn with_shielded_balance(mut self, address: String, balance: u64) -> Self {
        self.add_shielded_entry(address, balance);
        self
    }

    /// Show a type filter dropdown to the left of the text input.
    ///
    /// Only displayed when more than one address kind is enabled. Default: false.
    pub fn with_type_filter_dropdown(mut self, show: bool) -> Self {
        self.show_type_filter = show;
        self
    }

    /// Filter autocomplete entries by balance range (in native units).
    ///
    /// All known wallet addresses are included by default (including zero-balance).
    /// Use `with_balance_range(1..)` to show only funded addresses.
    /// Does not affect manual input validation. Default: no filter (all addresses).
    pub fn with_balance_range(mut self, range: impl std::ops::RangeBounds<u64>) -> Self {
        self.balance_range = Some(BalanceRange::from_range(&range));
        self
    }

    /// Exclude change addresses (BIP44 m/44'/5'/0'/1/x) from autocomplete.
    ///
    /// Send inputs should typically exclude change addresses since users
    /// don't share change addresses with others. Default: false (show all).
    pub fn with_exclude_change(mut self, exclude: bool) -> Self {
        self.exclude_change = exclude;
        self
    }

    /// Enable DPNS username resolution for Identity-type addresses. Default: true.
    pub fn with_dpns_resolution(mut self, enabled: bool) -> Self {
        self.dpns_resolution = enabled;
        self
    }

    /// Set the label displayed above the input field.
    pub fn with_label(mut self, label: impl Into<WidgetText>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the hint/placeholder text inside the input field.
    pub fn with_hint_text(mut self, hint: impl Into<String>) -> Self {
        self.hint_text = Some(hint.into());
        self
    }

    /// Set the desired width of the input field.
    pub fn with_desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Enable or disable validation error display. Default: true.
    pub fn with_show_validation_errors(mut self, show: bool) -> Self {
        self.show_validation_errors = show;
        self
    }

    /// Enable developer mode display (exact credits alongside DASH). Default: false.
    pub fn with_developer_mode(mut self, enabled: bool) -> Self {
        self.developer_mode = enabled;
        self
    }

    /// Pre-populate the input field with an address string.
    pub fn with_initial_value(mut self, address: impl Into<String>) -> Self {
        self.input_text = address.into();
        self
    }

    /// Enable selection-only mode. When true, the user must pick from autocomplete;
    /// manual arbitrary addresses are rejected.
    pub fn with_selection_only(mut self, selection_only: bool) -> Self {
        self.selection_only = selection_only;
        self
    }

    /// Show full addresses in dropdown instead of truncated. Default: false.
    pub fn with_full_addresses(mut self, full: bool) -> Self {
        self.full_addresses = full;
        self
    }

    // --- Mutable setters for runtime reconfiguration ---

    /// Update wallet data after initialization (e.g., balance refresh).
    ///
    /// When more than one wallet is provided, entries are prefixed with the wallet alias.
    pub fn set_wallets(&mut self, wallets: &[Arc<RwLock<Wallet>>]) {
        self.all_entries.retain(|e| {
            e.address_kind != AddressKind::Core && e.address_kind != AddressKind::Platform
        });
        let multi = wallets.len() > 1;
        for wallet in wallets {
            self.extract_wallet_entries(wallet, multi);
        }
    }

    /// Update identity data after initialization.
    pub fn set_identities(&mut self, identities: &[QualifiedIdentity]) {
        self.all_entries
            .retain(|e| e.address_kind != AddressKind::Identity);
        self.extract_identity_entries(identities);
    }

    /// Update shielded balance data after initialization.
    pub fn set_shielded_balance(&mut self, address: String, balance: u64) {
        self.all_entries
            .retain(|e| e.address_kind != AddressKind::Shielded);
        self.add_shielded_entry(address, balance);
    }

    /// Update developer mode flag.
    pub fn set_developer_mode(&mut self, enabled: bool) {
        self.developer_mode = enabled;
    }

    // --- Entry extraction ---

    fn extract_wallet_entries(&mut self, wallet: &Arc<RwLock<Wallet>>, multi_wallet: bool) {
        let guard = match wallet.read().ok() {
            Some(g) => g,
            None => return,
        };

        let prefix = if multi_wallet {
            let name = guard.alias.as_deref().unwrap_or("Wallet");
            format!("[{}] ", name)
        } else {
            String::new()
        };

        // Build a set of system addresses to exclude from autocomplete.
        // System addresses (Identity Registration, CoinJoin, Provider keys, etc.)
        // are internal wallet infrastructure — not for user-facing send/receive.
        use crate::ui::wallets::account_summary::AccountCategory;
        let system_addresses: std::collections::HashSet<&Address> = guard
            .watched_addresses
            .values()
            .filter(|info| {
                AccountCategory::from_reference(info.path_reference).is_system_category()
            })
            .map(|info| &info.address)
            .collect();

        // Core addresses from known_addresses (all derived addresses).
        // Balance is looked up from address_balances; addresses without UTXOs
        // get balance 0. Use `with_balance_range(1..)` to show only funded
        // addresses — do NOT filter at the data source.
        // Change addresses (BIP44 m/44'/5'/0'/1/x) are tagged and can be
        // excluded via `with_exclude_change(true)`.
        // System addresses are always excluded.
        for (address, derivation_path) in &guard.known_addresses {
            if system_addresses.contains(address) {
                continue;
            }
            let is_change = derivation_path.is_bip44_change(self.network);
            if self.exclude_change && is_change {
                continue;
            }
            let balance = guard.address_balances.get(address).copied().unwrap_or(0);
            let addr_str = address.to_string();
            let change_suffix = if is_change { " (change)" } else { "" };
            let display = if self.full_addresses {
                format!("{}{}{}", prefix, addr_str, change_suffix)
            } else {
                format!("{}{}{}", prefix, truncate_address(&addr_str), change_suffix)
            };
            self.all_entries.push(AddressEntry {
                address_string: addr_str,
                address_kind: AddressKind::Core,
                display_label: display,
                balance,
                validated: ValidatedAddress::Core(address.clone()),
                is_change,
            });
        }

        // Platform addresses: derive from watched_addresses (all bootstrapped
        // platform payment addresses), with balance from platform_address_info.
        // This ensures fresh wallets with no on-chain activity still show
        // their derived platform addresses.
        use crate::model::wallet::DerivationPathReference;
        let mut seen_platform = std::collections::HashSet::new();
        for addr_info in guard.watched_addresses.values() {
            if addr_info.path_reference != DerivationPathReference::PlatformPayment {
                continue;
            }
            let core_addr = &addr_info.address;
            if let Ok(platform_addr) = PlatformAddress::try_from(core_addr.clone()) {
                let addr_str = platform_addr.to_bech32m_string(self.network);
                if !seen_platform.insert(addr_str.clone()) {
                    continue;
                }
                let balance = guard
                    .platform_address_info
                    .get(core_addr)
                    .map(|info| info.balance)
                    .unwrap_or(0);
                let display = if self.full_addresses {
                    format!("{}{}", prefix, addr_str)
                } else {
                    format!("{}{}", prefix, truncate_address(&addr_str))
                };
                let bech32m = addr_str.clone();
                self.all_entries.push(AddressEntry {
                    address_string: addr_str,
                    address_kind: AddressKind::Platform,
                    display_label: display,
                    balance,
                    validated: ValidatedAddress::Platform {
                        address: platform_addr,
                        bech32m,
                    },
                    is_change: false,
                });
            }
        }
    }

    fn extract_identity_entries(&mut self, identities: &[QualifiedIdentity]) {
        for qi in identities {
            let id = qi.identity.id();
            let id_str = id.to_string(Encoding::Base58);
            let dpns_name = qi.dpns_names.first().map(|n| n.name.clone());
            let display = if let Some(ref name) = dpns_name {
                name.clone()
            } else if let Some(ref alias) = qi.alias {
                alias.clone()
            } else if self.full_addresses {
                id_str.clone()
            } else {
                truncate_address(&id_str)
            };
            self.all_entries.push(AddressEntry {
                address_string: id_str,
                address_kind: AddressKind::Identity,
                display_label: display,
                balance: qi.identity.balance(),
                validated: ValidatedAddress::Identity {
                    id,
                    dpns_name: dpns_name.clone(),
                },
                is_change: false,
            });
        }
    }

    fn add_shielded_entry(&mut self, address: String, balance: u64) {
        let display = if self.full_addresses {
            address.clone()
        } else {
            truncate_address(&address)
        };
        self.all_entries.push(AddressEntry {
            address_string: address.clone(),
            address_kind: AddressKind::Shielded,
            display_label: display,
            balance,
            validated: ValidatedAddress::Shielded(address),
            is_change: false,
        });
    }

    // --- Detection and validation ---

    fn detect_cached(&mut self, input: &str) -> DetectedType {
        if let Some((ref cached_input, cached_type)) = self.cached_detection
            && cached_input == input
        {
            return cached_type;
        }
        let identity_enabled = self.enabled_kinds.contains(&AddressKind::Identity);
        let result = detect_address_type(input, identity_enabled);
        self.cached_detection = Some((input.to_string(), result));
        result
    }

    fn validate_input(&self) -> (Option<String>, Option<ValidatedAddress>) {
        let trimmed = self.input_text.trim();
        if trimmed.is_empty() {
            return (None, None);
        }

        // In selection-only mode, all manual input is rejected. Users must select
        // an address from the autocomplete dropdown.
        if self.selection_only {
            return (
                Some("Please select an address from the list.".to_string()),
                None,
            );
        }

        let identity_enabled = self.enabled_kinds.contains(&AddressKind::Identity);
        let detected = detect_address_type(trimmed, identity_enabled);

        if detected == DetectedType::Unknown {
            return (
                Some(
                    "This does not look like a valid address. Please check for typos.".to_string(),
                ),
                None,
            );
        }

        let detected_kind = detected.to_address_kind().unwrap();

        // Check enabled kinds
        if !self.enabled_kinds.contains(&detected_kind) {
            let msg = match self.enabled_kinds.as_slice() {
                [AddressKind::Core] => "Only wallet addresses are accepted here.",
                [AddressKind::Platform] => "Only platform addresses are accepted here.",
                [AddressKind::Shielded] => "Only private addresses are accepted here.",
                [AddressKind::Identity] => "Only identity IDs are accepted here.",
                _ => "This address type is not accepted here.",
            };
            return (Some(msg.to_string()), None);
        }

        // Type-specific validation
        match detected {
            DetectedType::Core => self.validate_core(trimmed),
            DetectedType::Platform => self.validate_platform(trimmed),
            DetectedType::Shielded => self.validate_shielded(trimmed),
            DetectedType::Identity => self.validate_identity(trimmed),
            DetectedType::Unknown => unreachable!(),
        }
    }

    fn validate_core(&self, trimmed: &str) -> (Option<String>, Option<ValidatedAddress>) {
        match trimmed.parse::<Address<NetworkUnchecked>>() {
            Ok(addr) => match addr.require_network(self.network) {
                Ok(checked) => (None, Some(ValidatedAddress::Core(checked))),
                Err(_) => (
                    Some("This address belongs to a different network. Please check you are using the correct network.".to_string()),
                    None,
                ),
            },
            Err(_) => (
                Some("This does not look like a valid address. Please check for typos.".to_string()),
                None,
            ),
        }
    }

    fn validate_platform(&self, trimmed: &str) -> (Option<String>, Option<ValidatedAddress>) {
        // BIP-350: bech32m must be either all-lowercase or all-uppercase; mixed case is invalid.
        let is_lower = trimmed.chars().all(|c| !c.is_ascii_uppercase());
        let is_upper = trimmed.chars().all(|c| !c.is_ascii_lowercase());
        if !is_lower && !is_upper {
            return (
                Some(
                    "Platform addresses must not mix upper and lower case characters. Please use all lowercase.".to_string(),
                ),
                None,
            );
        }
        let canonical = trimmed.to_lowercase();
        let expected_prefix = match self.network {
            Network::Mainnet => "dash1",
            _ => "tdash1",
        };
        if !canonical.starts_with(expected_prefix)
            || canonical.starts_with(&format!("{}z", expected_prefix))
        {
            return (
                Some("This address belongs to a different network. Please check you are using the correct network.".to_string()),
                None,
            );
        }
        match PlatformAddress::from_bech32m_string(&canonical) {
            Ok((pa, _network)) => (
                None,
                Some(ValidatedAddress::Platform {
                    address: pa,
                    bech32m: canonical,
                }),
            ),
            Err(_) => (
                Some(
                    "This does not look like a valid address. Please check for typos.".to_string(),
                ),
                None,
            ),
        }
    }

    fn validate_shielded(&self, trimmed: &str) -> (Option<String>, Option<ValidatedAddress>) {
        let expected_prefix = match self.network {
            Network::Mainnet => "dash1z",
            _ => "tdash1z",
        };
        if !trimmed.starts_with(expected_prefix) {
            return (
                Some("This address belongs to a different network. Please check you are using the correct network.".to_string()),
                None,
            );
        }
        // Orchard shielded addresses are ~70+ chars; reject anything too short.
        if trimmed.len() < 60 {
            return (
                Some(
                    "This private address looks incomplete. Please paste the full address."
                        .to_string(),
                ),
                None,
            );
        }
        use dash_sdk::dpp::address_funds::OrchardAddress;
        match OrchardAddress::from_bech32m_string(trimmed) {
            Ok((_, network)) => {
                // Shielded addresses only encode mainnet vs non-mainnet in the HRP.
                // Testnet, Devnet, and Local all share "tdash1z" and cannot be
                // distinguished at the address level. Enforce mainnet isolation only.
                let same_mainnet_class =
                    (self.network == Network::Mainnet) == (network == Network::Mainnet);
                if !same_mainnet_class {
                    (
                        Some("This address belongs to a different network. Please check you are using the correct network.".to_string()),
                        None,
                    )
                } else {
                    (None, Some(ValidatedAddress::Shielded(trimmed.to_string())))
                }
            }
            Err(_) => (
                Some(
                    "This private address is not valid. Please check it and try again.".to_string(),
                ),
                None,
            ),
        }
    }

    fn validate_identity(&self, trimmed: &str) -> (Option<String>, Option<ValidatedAddress>) {
        match Identifier::from_string(trimmed, Encoding::Base58) {
            Ok(id) => {
                let dpns = if self.dpns_resolution {
                    self.all_entries
                        .iter()
                        .find(|e| {
                            e.address_kind == AddressKind::Identity
                                && e.validated.as_identity_id() == Some(&id)
                        })
                        .and_then(|e| e.validated.dpns_name().map(|s| s.to_string()))
                } else {
                    None
                };
                (
                    None,
                    Some(ValidatedAddress::Identity {
                        id,
                        dpns_name: dpns,
                    }),
                )
            }
            Err(_) => (
                Some(
                    "This does not look like a valid address. Please check for typos.".to_string(),
                ),
                None,
            ),
        }
    }

    // --- Autocomplete filtering ---

    /// Returns matching entries (truncated to 10) and the total match count
    /// before truncation.
    fn filtered_entries(&self) -> (Vec<&AddressEntry>, usize) {
        let query = self.input_text.trim().to_lowercase();

        let mut results: Vec<&AddressEntry> = self
            .all_entries
            .iter()
            .filter(|e| {
                // Type filter
                if let Some(filter_kind) = self.selected_type_filter
                    && e.address_kind != filter_kind
                {
                    return false;
                }
                // Enabled kinds
                if !self.enabled_kinds.contains(&e.address_kind) {
                    return false;
                }
                // Balance range
                if let Some(ref range) = self.balance_range
                    && !range.contains(e.balance)
                {
                    return false;
                }
                // When query is empty, show all entries (no substring filter)
                if query.is_empty() {
                    return true;
                }
                // Substring match against address, label, and type name.
                // Typing "platform" or "core" filters to that address type.
                e.address_string.to_lowercase().contains(&query)
                    || e.display_label.to_lowercase().contains(&query)
                    || e.address_kind.short_label().to_lowercase().contains(&query)
                    || e.address_kind.display_name().to_lowercase().contains(&query)
            })
            .collect();

        // Sort: exact prefix matches first, then by label
        results.sort_by(|a, b| {
            if query.is_empty() {
                return a.display_label.cmp(&b.display_label);
            }
            let a_prefix = a.address_string.to_lowercase().starts_with(&query);
            let b_prefix = b.address_string.to_lowercase().starts_with(&query);
            b_prefix
                .cmp(&a_prefix)
                .then(a.display_label.cmp(&b.display_label))
        });

        let total = results.len();
        results.truncate(10);
        (results, total)
    }

    // --- Balance formatting ---

    fn format_balance(&self, entry: &AddressEntry) -> String {
        match entry.address_kind {
            AddressKind::Core => Self::format_dash_4dp(Amount::dash_from_duffs(entry.balance)),
            AddressKind::Platform | AddressKind::Shielded | AddressKind::Identity => {
                let dash = Amount::new(entry.balance, DASH_DECIMAL_PLACES).with_unit_name("DASH");
                if self.developer_mode {
                    format!(
                        "{} ({} credits)",
                        Self::format_dash_4dp(dash),
                        entry.balance
                    )
                } else {
                    Self::format_dash_4dp(dash)
                }
            }
        }
    }

    /// Format a DASH amount with exactly 4 decimal places for dropdown display.
    fn format_dash_4dp(amount: Amount) -> String {
        // Get the full-precision string without trimming, then truncate to 4 dp.
        let full = amount.to_string_opts(false, false);
        let formatted = if let Some(dot_pos) = full.find('.') {
            let decimals = &full[dot_pos + 1..];
            if decimals.len() > 4 {
                format!("{}.{}", &full[..dot_pos], &decimals[..4])
            } else {
                // Pad with zeros if fewer than 4 decimals
                format!("{}.{:0<4}", &full[..dot_pos], decimals)
            }
        } else {
            format!("{full}.0000")
        };
        format!("{formatted} DASH")
    }

    // --- show() implementation ---

    fn show_internal(&mut self, ui: &mut Ui) -> InnerResponse<AddressInputResponse> {
        let resp = ui.vertical(|ui| {
            // Label
            if let Some(label) = &self.label {
                ui.label(label.clone());
            }

            // Input row
            let text_response = ui
                .horizontal(|ui| {
                    // Type filter dropdown
                    if self.show_type_filter && self.enabled_kinds.len() > 1 {
                        let current_label = self
                            .selected_type_filter
                            .map(|t| t.display_name())
                            .unwrap_or("All");
                        egui::ComboBox::from_id_salt(ui.id().with("address_type_filter"))
                            .selected_text(current_label)
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(self.selected_type_filter.is_none(), "All")
                                    .clicked()
                                {
                                    self.selected_type_filter = None;
                                }
                                for &kind in &self.enabled_kinds {
                                    let selected = self.selected_type_filter == Some(kind);
                                    if ui.selectable_label(selected, kind.display_name()).clicked()
                                    {
                                        self.selected_type_filter = Some(kind);
                                    }
                                }
                            });
                    }

                    // Text input
                    let mut text_edit = egui::TextEdit::singleline(&mut self.input_text);
                    if let Some(hint) = &self.hint_text {
                        text_edit = text_edit
                            .hint_text(egui::RichText::new(hint).color(egui::Color32::GRAY));
                    }
                    if let Some(width) = self.desired_width {
                        text_edit = text_edit.desired_width(width);
                    } else {
                        text_edit = text_edit.desired_width(f32::INFINITY);
                    }
                    ui.add(text_edit)
                })
                .inner;

            let text_changed = text_response.changed();
            let lost_focus = text_response.lost_focus();
            let has_focus = text_response.has_focus();

            // On text change: reset validation state
            if text_changed {
                self.selected_from_autocomplete = false;
                self.cached_detection = None;
                // Detect paste: a multi-character change in a single frame.
                // Validate immediately so the user doesn't have to blur first.
                self.has_blurred = self.input_text.trim().len() > 3;
            }

            // Detect address type (cached)
            let input_clone = self.input_text.clone();
            let detected = self.detect_cached(&input_clone);

            // On blur: trigger validation
            if lost_focus && !self.input_text.trim().is_empty() {
                self.has_blurred = true;
            }

            // Autocomplete popup
            let mut selected_entry: Option<AddressEntry> = None;
            if has_focus || self.autocomplete_open {
                // Collect filtered entries into an owned snapshot to release the borrow on self
                let (filtered, total_entries) = self.filtered_entries();
                let filtered_len = filtered.len();
                let entries_snapshot: Vec<(String, String, AddressEntry)> = filtered
                    .iter()
                    .map(|e| {
                        let label =
                            format!("{} ({})", e.display_label, e.address_kind.short_label());
                        (label, self.format_balance(e), (*e).clone())
                    })
                    .collect();

                if !entries_snapshot.is_empty() {
                    self.autocomplete_open = true;
                    let popup_id = ui.id().with("address_autocomplete");

                    egui::Area::new(popup_id)
                        .order(egui::Order::Foreground)
                        .fixed_pos(text_response.rect.left_bottom())
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_width(text_response.rect.width());
                                egui::ScrollArea::vertical()
                                    .max_height(200.0)
                                    .show(ui, |ui| {
                                        for (i, (label, balance_str, entry)) in
                                            entries_snapshot.iter().enumerate()
                                        {
                                            let highlighted =
                                                self.autocomplete_highlight == Some(i);

                                            // Single interaction rect spanning the full
                                            // row width. No child widgets — painted
                                            // manually so nothing steals clicks.
                                            let row_height = ui.spacing().interact_size.y;
                                            let row_width = ui.available_width();
                                            let (rect, response) = ui.allocate_exact_size(
                                                egui::vec2(row_width, row_height),
                                                egui::Sense::click(),
                                            );

                                            if ui.is_rect_visible(rect) {
                                                let hovered = response.hovered();
                                                if highlighted || hovered {
                                                    ui.painter().rect_filled(
                                                        rect,
                                                        egui::CornerRadius::from(2.0),
                                                        ui.style().visuals.widgets.hovered.bg_fill,
                                                    );
                                                }

                                                let text_color = if highlighted || hovered {
                                                    ui.style().visuals.widgets.hovered.text_color()
                                                } else {
                                                    ui.style().visuals.widgets.inactive.text_color()
                                                };

                                                let padding = 4.0;
                                                ui.painter().text(
                                                    egui::pos2(
                                                        rect.left() + padding,
                                                        rect.center().y,
                                                    ),
                                                    egui::Align2::LEFT_CENTER,
                                                    label.as_str(),
                                                    egui::TextStyle::Body.resolve(ui.style()),
                                                    text_color,
                                                );

                                                ui.painter().text(
                                                    egui::pos2(
                                                        rect.right() - padding,
                                                        rect.center().y,
                                                    ),
                                                    egui::Align2::RIGHT_CENTER,
                                                    balance_str.as_str(),
                                                    egui::TextStyle::Small.resolve(ui.style()),
                                                    DashColors::GRAY,
                                                );
                                            }

                                            if response.clicked() {
                                                selected_entry = Some(entry.clone());
                                            }
                                        }
                                        if total_entries > 10 {
                                            let remaining = total_entries - 10;
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "...and {} more",
                                                    remaining
                                                ))
                                                .small()
                                                .color(DashColors::GRAY),
                                            );
                                        }
                                    });
                            });
                        });

                    // Keyboard navigation (uses snapshot data, no recomputation)
                    ui.input(|i| {
                        if i.key_pressed(egui::Key::ArrowDown) {
                            self.autocomplete_highlight = Some(
                                self.autocomplete_highlight
                                    .map(|h| (h + 1).min(filtered_len.saturating_sub(1)))
                                    .unwrap_or(0),
                            );
                        }
                        if i.key_pressed(egui::Key::ArrowUp) {
                            self.autocomplete_highlight = self
                                .autocomplete_highlight
                                .map(|h| h.saturating_sub(1))
                                .or(Some(0));
                        }
                        if i.key_pressed(egui::Key::Escape) {
                            self.autocomplete_open = false;
                            self.autocomplete_highlight = None;
                        }
                        if i.key_pressed(egui::Key::Enter)
                            && let Some(idx) = self.autocomplete_highlight
                            && let Some((_, _, entry)) = entries_snapshot.get(idx)
                        {
                            selected_entry = Some(entry.clone());
                        }
                    });
                } else {
                    self.autocomplete_open = false;
                }
            } else {
                self.autocomplete_open = false;
            }

            // Handle autocomplete selection (FIX 7: clear cached_detection)
            let selected_this_frame = selected_entry.is_some();
            if let Some(entry) = selected_entry {
                self.input_text = entry.address_string.clone();
                self.selected_from_autocomplete = true;
                self.cached_detection = None;
                self.autocomplete_open = false;
                self.autocomplete_highlight = None;
                self.has_blurred = true;
            }

            // Validation
            let (error_message, validated_address) = if self.selected_from_autocomplete {
                // Find the matching entry for the selected address
                let validated = self
                    .all_entries
                    .iter()
                    .find(|e| e.address_string == self.input_text)
                    .map(|e| e.validated.clone());
                (None, validated)
            } else if self.has_blurred && !self.input_text.trim().is_empty() {
                self.validate_input()
            } else {
                (None, None)
            };

            // Status/error display below input
            if self.show_validation_errors {
                if let Some(ref error) = error_message {
                    ui.colored_label(DashColors::VALIDATION_WARNING, error);
                } else if self.has_blurred
                    && validated_address.is_some()
                    && let Some(kind) = detected.to_address_kind()
                {
                    ui.colored_label(DashColors::SUCCESS, kind.display_name());
                }
            }

            // Build response
            // FIX 1: blur validation produces a result => signal changed
            let blur_validated = lost_focus && validated_address.is_some();
            // FIX 2: use one-frame local flag for autocomplete selection
            let changed = text_changed || selected_this_frame || self.changed || blur_validated;
            if self.changed {
                self.changed = false;
            }

            AddressInputResponse {
                response: text_response,
                changed,
                error_message,
                validated_address,
            }
        });

        InnerResponse::new(resp.inner, resp.response)
    }
}

impl Component for AddressInput {
    type DomainType = ValidatedAddress;
    type Response = AddressInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        self.show_internal(ui)
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        if self.selected_from_autocomplete {
            return self
                .all_entries
                .iter()
                .find(|e| e.address_string == self.input_text)
                .map(|e| e.validated.clone());
        }
        if self.has_blurred && !self.input_text.trim().is_empty() {
            let (err, val) = self.validate_input();
            if err.is_none() {
                return val;
            }
        }
        None
    }
}

// --- Free functions ---

/// Detect the address type of a raw input string.
///
/// Priority: Shielded > Platform > Core > Identity (Base58 fallback).
/// Identity detection only runs when `identity_enabled` is true.
fn detect_address_type(input: &str, identity_enabled: bool) -> DetectedType {
    match AddressKind::detect(input) {
        Some(AddressKind::Identity) if !identity_enabled => DetectedType::Unknown,
        Some(AddressKind::Core) => DetectedType::Core,
        Some(AddressKind::Platform) => DetectedType::Platform,
        Some(AddressKind::Shielded) => DetectedType::Shielded,
        Some(AddressKind::Identity) => DetectedType::Identity,
        None => DetectedType::Unknown,
    }
}

/// Truncate an address for display in the address input component (8 prefix + 6 suffix).
fn truncate_address(addr: &str) -> String {
    crate::model::address::truncate_address(addr, 8, 6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
    use dash_sdk::dashcore_rpc::dashcore::{PrivateKey, PublicKey};

    /// Generate a valid testnet P2PKH address for testing.
    fn testnet_core_address() -> (String, Address) {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let privkey = PrivateKey::new(sk, Network::Testnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        let addr = Address::p2pkh(&pubkey, Network::Testnet);
        (addr.to_string(), addr)
    }

    /// Generate a valid mainnet P2PKH address for testing.
    fn mainnet_core_address() -> (String, Address) {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let privkey = PrivateKey::new(sk, Network::Mainnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        let addr = Address::p2pkh(&pubkey, Network::Mainnet);
        (addr.to_string(), addr)
    }

    // --- detect_address_type tests ---

    #[test]
    fn detect_shielded_mainnet() {
        let result = detect_address_type("dash1z_some_shielded_addr", true);
        assert_eq!(result, DetectedType::Shielded);
    }

    #[test]
    fn detect_shielded_testnet() {
        let result = detect_address_type("tdash1z_some_shielded_addr", true);
        assert_eq!(result, DetectedType::Shielded);
    }

    #[test]
    fn detect_platform_testnet() {
        // A plausible platform address prefix
        let result = detect_address_type("tdash1qwer1234", false);
        assert_eq!(result, DetectedType::Platform);
    }

    #[test]
    fn detect_platform_mainnet() {
        let result = detect_address_type("dash1qwer1234", false);
        assert_eq!(result, DetectedType::Platform);
    }

    #[test]
    fn detect_core_address() {
        let (addr_str, _) = testnet_core_address();
        let result = detect_address_type(&addr_str, false);
        assert_eq!(result, DetectedType::Core);
    }

    #[test]
    fn detect_unknown_for_garbage() {
        let result = detect_address_type("not-an-address", true);
        assert_eq!(result, DetectedType::Unknown);
    }

    #[test]
    fn detect_empty_is_unknown() {
        let result = detect_address_type("", true);
        assert_eq!(result, DetectedType::Unknown);
    }

    #[test]
    fn detect_whitespace_is_unknown() {
        let result = detect_address_type("   ", true);
        assert_eq!(result, DetectedType::Unknown);
    }

    #[test]
    fn detect_identity_when_enabled() {
        // A 32-byte Base58 identifier that does not parse as a Core address
        let id = Identifier::random();
        let id_str = id.to_string(Encoding::Base58);
        let result = detect_address_type(&id_str, true);
        assert_eq!(result, DetectedType::Identity);
    }

    #[test]
    fn detect_identity_disabled_falls_through_to_unknown() {
        let id = Identifier::random();
        let id_str = id.to_string(Encoding::Base58);
        let result = detect_address_type(&id_str, false);
        // Should be Unknown since identity detection is disabled
        // (unless it happens to parse as a Core address, which is possible for some Base58 values)
        assert!(result == DetectedType::Unknown || result == DetectedType::Core);
    }

    #[test]
    fn shielded_takes_priority_over_platform() {
        // dash1z starts with "dash1" which could match platform, but shielded wins
        let result = detect_address_type("dash1z_test_addr", false);
        assert_eq!(result, DetectedType::Shielded);
    }

    // --- Network validation tests ---

    #[test]
    fn core_address_wrong_network_rejected() {
        let input = AddressInput::new(Network::Testnet);
        let (mainnet_str, _) = mainnet_core_address();
        let (err, val) = input.validate_core(&mainnet_str);
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some(
                "This address belongs to a different network. Please check you are using the correct network."
            )
        );
    }

    #[test]
    fn core_address_correct_network_accepted() {
        let input = AddressInput::new(Network::Testnet);
        let (testnet_str, _) = testnet_core_address();
        let (err, val) = input.validate_core(&testnet_str);
        assert!(err.is_none());
        assert!(val.is_some());
    }

    #[test]
    fn platform_address_wrong_network_rejected() {
        let input = AddressInput::new(Network::Mainnet);
        // tdash1 prefix on mainnet
        let (err, val) = input.validate_platform("tdash1qwer1234");
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some(
                "This address belongs to a different network. Please check you are using the correct network."
            )
        );
    }

    #[test]
    fn shielded_address_wrong_network_rejected() {
        let input = AddressInput::new(Network::Mainnet);
        let (err, val) = input.validate_shielded("tdash1z_test_addr");
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some(
                "This address belongs to a different network. Please check you are using the correct network."
            )
        );
    }

    #[test]
    fn shielded_address_too_short_rejected() {
        let input = AddressInput::new(Network::Testnet);
        let (err, val) = input.validate_shielded("tdash1z");
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some("This private address looks incomplete. Please paste the full address.")
        );
    }

    #[test]
    fn shielded_prefix_only_rejected() {
        let input = AddressInput::new(Network::Mainnet);
        let (err, val) = input.validate_shielded("dash1z");
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some("This private address looks incomplete. Please paste the full address.")
        );
    }

    #[test]
    fn shielded_address_with_invalid_chars_rejected() {
        let input = AddressInput::new(Network::Testnet);
        let long_addr = format!("tdash1z{}", "x".repeat(60));
        let (err, val) = input.validate_shielded(&long_addr);
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some("This private address is not valid. Please check it and try again.")
        );
    }

    // --- Enabled type restriction tests ---

    #[test]
    fn disabled_type_rejected_with_correct_error() {
        let mut input =
            AddressInput::new(Network::Testnet).with_address_kinds(&[AddressKind::Core]);
        // Set a platform-looking address with only Core enabled
        input.input_text = "tdash1qwer1234".to_string();
        input.has_blurred = true;
        let (err, val) = input.validate_input();
        assert!(
            val.is_none(),
            "should reject platform address when only Core is enabled"
        );
        assert_eq!(
            err.as_deref(),
            Some("Only wallet addresses are accepted here.")
        );
    }

    #[test]
    fn disabled_type_empty_input_no_error() {
        let input = AddressInput::new(Network::Testnet).with_address_kinds(&[AddressKind::Core]);
        let (err, val) = input.validate_input();
        assert!(err.is_none(), "empty input should not produce an error");
        assert!(val.is_none());
    }

    // --- Selection-only mode tests ---

    #[test]
    fn selection_only_rejects_manual_input() {
        let mut input = AddressInput::new(Network::Testnet).with_selection_only(true);
        let (addr_str, _) = testnet_core_address();
        input.input_text = addr_str;
        input.has_blurred = true;
        let (err, val) = input.validate_input();
        assert!(
            val.is_none(),
            "selection-only mode should reject manual input"
        );
        assert_eq!(
            err.as_deref(),
            Some("Please select an address from the list.")
        );
    }

    #[test]
    fn selection_only_empty_input_no_error() {
        let input = AddressInput::new(Network::Testnet).with_selection_only(true);
        let (err, val) = input.validate_input();
        assert!(
            err.is_none(),
            "empty input in selection-only mode should not error"
        );
        assert!(val.is_none());
    }

    // --- Identity validation tests ---

    #[test]
    fn validate_identity_valid_identifier() {
        let input =
            AddressInput::new(Network::Testnet).with_address_kinds(&[AddressKind::Identity]);
        let id = Identifier::random();
        let id_str = id.to_string(Encoding::Base58);
        let (err, val) = input.validate_identity(&id_str);
        assert!(err.is_none());
        let val = val.expect("valid identifier should produce ValidatedAddress");
        assert_eq!(val.kind(), AddressKind::Identity);
        assert_eq!(val.as_identity_id(), Some(&id));
    }

    #[test]
    fn validate_identity_invalid_string() {
        let input = AddressInput::new(Network::Testnet);
        let (err, val) = input.validate_identity("not-a-valid-identifier");
        assert!(val.is_none());
        assert_eq!(
            err.as_deref(),
            Some("This does not look like a valid address. Please check for typos.")
        );
    }

    // --- Truncate boundary tests ---

    #[test]
    fn truncate_address_boundary_16_unchanged() {
        assert_eq!(truncate_address("1234567890123456"), "1234567890123456");
    }

    #[test]
    fn truncate_address_boundary_17_truncated() {
        let result = truncate_address("12345678901234567");
        assert_eq!(result, "12345678...234567");
    }

    #[test]
    fn truncate_address_non_ascii_does_not_panic() {
        let addr = "\u{1F355}dash1ztestaddr\u{1F389}longstringpadding";
        let result = truncate_address(addr);
        assert!(result.contains("..."));
    }

    #[test]
    fn truncate_address_multibyte_short_unchanged() {
        let addr = "\u{00E9}\u{00E9}\u{00E9}abc";
        assert_eq!(truncate_address(addr), addr);
    }

    // --- BalanceRange tests ---

    #[test]
    fn balance_range_inclusive() {
        let range = BalanceRange::from_range(&(10..=20));
        assert!(range.contains(10));
        assert!(range.contains(15));
        assert!(range.contains(20));
        assert!(!range.contains(9));
        assert!(!range.contains(21));
    }

    #[test]
    fn balance_range_exclusive() {
        let range = BalanceRange::from_range(&(10..20));
        assert!(range.contains(10));
        assert!(range.contains(19));
        assert!(!range.contains(20));
        assert!(!range.contains(9));
    }

    #[test]
    fn balance_range_unbounded_start() {
        let range = BalanceRange::from_range(&(..=100));
        assert!(range.contains(0));
        assert!(range.contains(100));
        assert!(!range.contains(101));
    }

    #[test]
    fn balance_range_unbounded_end() {
        let range = BalanceRange::from_range(&(50..));
        assert!(!range.contains(49));
        assert!(range.contains(50));
        assert!(range.contains(u64::MAX));
    }

    #[test]
    fn balance_range_fully_unbounded() {
        let range = BalanceRange::from_range(&(..));
        assert!(range.contains(0));
        assert!(range.contains(u64::MAX));
    }

    #[test]
    fn balance_range_zero_only() {
        let range = BalanceRange::from_range(&(0..=0));
        assert!(range.contains(0));
        assert!(!range.contains(1));
    }

    // --- AddressKind display name tests ---

    #[test]
    fn address_kind_display_names() {
        assert_eq!(AddressKind::Core.display_name(), "Wallet address");
        assert_eq!(AddressKind::Platform.display_name(), "Platform address");
        assert_eq!(AddressKind::Shielded.display_name(), "Private address");
        assert_eq!(AddressKind::Identity.display_name(), "Identity");
    }

    // --- truncate_address tests ---

    #[test]
    fn truncate_short_address_unchanged() {
        assert_eq!(truncate_address("short"), "short");
    }

    #[test]
    fn truncate_long_address() {
        let (addr_str, _) = testnet_core_address();
        let truncated = truncate_address(&addr_str);
        assert!(truncated.contains("..."));
        assert!(truncated.len() < addr_str.len());
    }

    // --- ValidatedAddress variant accessor tests ---

    #[test]
    fn validated_core_accessors() {
        let (_, addr) = testnet_core_address();
        let va = ValidatedAddress::Core(addr.clone());
        assert_eq!(va.kind(), AddressKind::Core);
        assert_eq!(va.as_core(), Some(&addr));
        assert!(va.as_platform().is_none());
        assert!(va.as_identity_id().is_none());
        assert!(va.dpns_name().is_none());
    }

    #[test]
    fn validated_identity_accessors() {
        let id = Identifier::random();
        let va = ValidatedAddress::Identity {
            id,
            dpns_name: Some("alice.dash".to_string()),
        };
        assert_eq!(va.kind(), AddressKind::Identity);
        assert_eq!(va.as_identity_id(), Some(&id));
        assert_eq!(va.dpns_name(), Some("alice.dash"));
        assert!(va.as_core().is_none());
    }

    #[test]
    fn validated_shielded_accessors() {
        let va = ValidatedAddress::Shielded("dash1z_test".to_string());
        assert_eq!(va.kind(), AddressKind::Shielded);
        assert_eq!(va.to_address_string(), "dash1z_test");
    }

    // --- FIX 1: Blur validation propagation ---

    #[test]
    fn blur_triggers_validation_for_valid_core_address() {
        let (addr_str, _) = testnet_core_address();
        let mut input = AddressInput::new(Network::Testnet);
        input.input_text = addr_str;
        // Simulate blur: has_blurred is set when focus leaves with non-empty input
        input.has_blurred = true;
        let (err, val) = input.validate_input();
        assert!(err.is_none(), "valid address after blur should not error");
        assert!(
            val.is_some(),
            "valid address after blur should produce a validated address"
        );
    }

    #[test]
    fn current_value_returns_validated_after_blur() {
        let (addr_str, _) = testnet_core_address();
        let mut input = AddressInput::new(Network::Testnet);
        input.input_text = addr_str;
        input.has_blurred = true;
        let val = input.current_value();
        assert!(
            val.is_some(),
            "current_value should return validated address after blur"
        );
        assert_eq!(val.unwrap().kind(), AddressKind::Core);
    }

    // --- FIX 4: Mixed-case bech32m rejection ---

    #[test]
    fn platform_mixed_case_rejected() {
        let input = AddressInput::new(Network::Testnet);
        let (err, val) = input.validate_platform("tDash1qwer1234");
        assert!(val.is_none(), "mixed-case bech32m should be rejected");
        assert_eq!(
            err.as_deref(),
            Some(
                "Platform addresses must not mix upper and lower case characters. Please use all lowercase."
            )
        );
    }

    #[test]
    fn platform_all_lowercase_accepted_for_case_check() {
        let input = AddressInput::new(Network::Testnet);
        // This will fail bech32m parsing, but should NOT fail the case check
        let (err, _) = input.validate_platform("tdash1qwer1234");
        assert_ne!(
            err.as_deref(),
            Some(
                "Platform addresses must not mix upper and lower case characters. Please use all lowercase."
            ),
            "all-lowercase should pass the case check"
        );
    }

    #[test]
    fn platform_all_uppercase_accepted_for_case_check() {
        let input = AddressInput::new(Network::Testnet);
        // All-uppercase is valid per BIP-350 (will fail other checks, but not case)
        let (err, _) = input.validate_platform("TDASH1QWER1234");
        assert_ne!(
            err.as_deref(),
            Some(
                "Platform addresses must not mix upper and lower case characters. Please use all lowercase."
            ),
            "all-uppercase should pass the case check"
        );
    }
}
