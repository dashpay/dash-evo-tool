use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::theme::{DashColors, Shape};
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{Color32, InnerResponse, Response, Ui, WidgetText};
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
/// Pre-computed from wallet/identity data at builder/setter time. Each field is
/// structured data driving one part of the row: `wallet_name` → wallet pill,
/// `address_string` → address text, `name_label` → `(name)` annotation,
/// `address_kind` → type pill.
#[derive(Debug, Clone)]
struct AddressEntry {
    /// The full address string (populates the text field on selection and is
    /// shown, truncated unless `full_addresses`, as the row's address text).
    address_string: String,
    /// Classification of this entry (drives the type pill).
    address_kind: AddressKind,
    /// Human name distinct from the address, shown as `(name)`: DPNS name or
    /// alias for identities, `"change"` for change addresses. `None` when the
    /// address has no name of its own.
    name_label: Option<String>,
    /// Owning wallet's display name, shown as a pill. `Some` for Core/Platform
    /// entries; `None` for wallet-agnostic kinds (Identity, Shielded).
    wallet_name: Option<String>,
    /// Balance in native units (duffs for Core, credits for Platform/Shielded/Identity).
    balance: u64,
    /// Pre-built ValidatedAddress for immediate use on selection.
    validated: ValidatedAddress,
}

impl AddressEntry {
    /// Alphabetical sort key: the entry's name if it has one, else the address.
    fn sort_key(&self) -> &str {
        self.name_label
            .as_deref()
            .unwrap_or(self.address_string.as_str())
    }

    /// Whether `needle` (already lowercased) substring-matches any searchable
    /// field: the address, the name, the wallet, or the kind's labels.
    fn matches_free_text(&self, needle: &str) -> bool {
        self.address_string.to_lowercase().contains(needle)
            || self
                .name_label
                .as_deref()
                .is_some_and(|s| s.to_lowercase().contains(needle))
            || self
                .wallet_name
                .as_deref()
                .is_some_and(|s| s.to_lowercase().contains(needle))
            || self
                .address_kind
                .short_label()
                .to_lowercase()
                .contains(needle)
            || self
                .address_kind
                .display_name()
                .to_lowercase()
                .contains(needle)
    }
}

/// A recognized search-tag key. Adding a new tag is a single match arm here plus
/// one in [`ParsedQuery::parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKey {
    Type,
    Wallet,
}

/// Split a whitespace-free token into a recognized `key:value` filter tag.
///
/// Returns `None` for free-text tokens: any token without a colon, with a
/// non-alphabetic key, or with an unrecognized key (e.g. `foo:bar`) — those are
/// searched literally. A recognized key with an empty value (e.g. bare `type:`)
/// still returns `Some` with an empty value; the caller treats it as no
/// constraint.
fn parse_tag(token: &str) -> Option<(TagKey, &str)> {
    let (key, value) = token.split_once(':')?;
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let tag = match key.to_ascii_lowercase().as_str() {
        "type" => TagKey::Type,
        "wallet" => TagKey::Wallet,
        _ => return None,
    };
    Some((tag, value))
}

/// A parsed GitHub-style search query: recognized filter tags plus leftover
/// free-text tokens.
///
/// - `type:` values prefix-match `AddressKind::short_label()` (restricted to the
///   instance's enabled kinds); multiple `type:` tokens OR together.
/// - `wallet:` values substring-match the entry's wallet name; multiple tokens
///   OR together.
/// - Free-text tokens AND together, each substring-matching any searchable field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedQuery {
    /// Kinds selected by `type:` tokens (union).
    type_kinds: Vec<AddressKind>,
    /// Whether at least one non-empty `type:` token was present. Distinguishes
    /// "no type constraint" from "type constraint that matched no kind".
    has_type_constraint: bool,
    /// Lowercased values from `wallet:` tokens (union).
    wallet_values: Vec<String>,
    /// Lowercased free-text tokens (intersection).
    free_text: Vec<String>,
}

impl ParsedQuery {
    /// Parse `query` into filter tags and free text. `enabled_kinds` bounds which
    /// kinds a `type:` value can select.
    fn parse(query: &str, enabled_kinds: &[AddressKind]) -> Self {
        let mut parsed = Self::default();
        for token in query.split_whitespace() {
            match parse_tag(token) {
                Some((TagKey::Type, value)) => {
                    if value.is_empty() {
                        continue;
                    }
                    parsed.has_type_constraint = true;
                    let value = value.to_ascii_lowercase();
                    for &kind in enabled_kinds {
                        if kind.short_label().to_ascii_lowercase().starts_with(&value)
                            && !parsed.type_kinds.contains(&kind)
                        {
                            parsed.type_kinds.push(kind);
                        }
                    }
                }
                Some((TagKey::Wallet, value)) => {
                    if !value.is_empty() {
                        parsed.wallet_values.push(value.to_ascii_lowercase());
                    }
                }
                None => parsed.free_text.push(token.to_ascii_lowercase()),
            }
        }
        parsed
    }

    /// The needle used to float prefix-matching addresses to the top: the joined
    /// free text, or `None` when the query is pure tags (no free text).
    fn prefix_needle(&self) -> Option<String> {
        if self.free_text.is_empty() {
            None
        } else {
            Some(self.free_text.join(" "))
        }
    }

    /// Whether `entry` satisfies every tag and free-text constraint.
    fn matches(&self, entry: &AddressEntry) -> bool {
        if self.has_type_constraint && !self.type_kinds.contains(&entry.address_kind) {
            return false;
        }
        if !self.wallet_values.is_empty() {
            match entry.wallet_name.as_deref() {
                Some(name) => {
                    let name = name.to_lowercase();
                    if !self.wallet_values.iter().any(|v| name.contains(v)) {
                        return false;
                    }
                }
                None => return false,
            }
        }
        self.free_text.iter().all(|t| entry.matches_free_text(t))
    }
}

/// One autocomplete row's render descriptor, exposed for UI tests. Each field
/// maps one-to-one to a render decision: `wallet_pill.is_some()` → wallet pill
/// shown, `name.is_some()` → `(name)` shown, `kind` → type pill (always shown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedRow {
    pub address_string: String,
    pub wallet_pill: Option<String>,
    pub name: Option<String>,
    pub kind: AddressKind,
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

/// A wallet paired with its display-only `WalletBackend`-snapshot views:
/// per-address balances (`AppContext::snapshot_address_balances`) and the
/// authoritative per-address derivation paths
/// (`AppContext::snapshot_address_paths`).
///
/// The component takes both explicitly so it never reaches into wallet state
/// for funds (A04 — snapshot is display-only). The paths are what the wallet
/// actually owns and SPV watches, so they, not the wallet's own bookkeeping,
/// decide which addresses autocomplete may offer.
pub type WalletWithSnapshot = (
    Arc<RwLock<Wallet>>,
    std::collections::BTreeMap<Address, u64>,
    std::collections::BTreeMap<Address, DerivationPath>,
);

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
///         .with_wallets(&[(
///             wallet,
///             app_context.snapshot_address_balances(&seed_hash),
///             app_context.snapshot_address_paths(&seed_hash),
///         )])
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

    /// Provide wallet data for **Core and Platform** autocomplete only.
    ///
    /// This extracts BIP-44 (Core) and DIP-17 (Platform) addresses from each
    /// wallet's snapshot `address_paths`. It does NOT extract identities or
    /// shielded addresses — those live outside the `Wallet` struct and must be
    /// added separately:
    ///
    /// - **Identities**: call [`with_identities()`] with `QualifiedIdentity`
    ///   data from `AppContext::load_local_qualified_identities()`.
    /// - **Shielded**: call [`with_shielded_balance()`] with the address
    ///   string from the upstream shielded coordinator's default address.
    ///
    /// Entries are extracted immediately (read lock acquired once per wallet).
    /// Skips gracefully if a wallet lock is poisoned. Each Core/Platform entry
    /// carries its wallet's name for the wallet pill and `wallet:` search tag.
    pub fn with_wallets(mut self, wallets: &[WalletWithSnapshot]) -> Self {
        for (wallet, balances, paths) in wallets {
            self.extract_wallet_entries(wallet, balances, paths);
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
    pub fn set_wallets(&mut self, wallets: &[WalletWithSnapshot]) {
        self.all_entries.retain(|e| {
            e.address_kind != AddressKind::Core && e.address_kind != AddressKind::Platform
        });
        for (wallet, balances, paths) in wallets {
            self.extract_wallet_entries(wallet, balances, paths);
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

    // --- Entry extraction ---

    /// Extract this wallet's Core and Platform autocomplete entries from the
    /// display snapshot's `address_paths` — the addresses the upstream wallet
    /// actually generated and SPV watches.
    ///
    /// Whitelist approach: an address is offered only when its derivation path
    /// matches a kind we recognise (BIP-44 for Core, DIP-17 for Platform).
    /// Unknown or unrecognised paths are excluded — safer to hide an address
    /// than to offer it under the wrong type.
    fn extract_wallet_entries(
        &mut self,
        wallet: &Arc<RwLock<Wallet>>,
        address_balances: &std::collections::BTreeMap<Address, u64>,
        address_paths: &std::collections::BTreeMap<Address, DerivationPath>,
    ) {
        let guard = match wallet.read().ok() {
            Some(g) => g,
            None => return,
        };

        let wallet_name = Some(guard.alias.as_deref().unwrap_or("Wallet").to_string());

        for (address, derivation_path) in address_paths {
            if derivation_path.is_bip44(self.network) {
                let is_change = derivation_path.is_bip44_change(self.network);
                if self.exclude_change && is_change {
                    continue;
                }
                self.all_entries.push(AddressEntry {
                    address_string: address.to_string(),
                    address_kind: AddressKind::Core,
                    name_label: is_change.then(|| "change".to_string()),
                    wallet_name: wallet_name.clone(),
                    balance: address_balances.get(address).copied().unwrap_or(0),
                    validated: ValidatedAddress::Core(address.clone()),
                });
                continue;
            }

            // DIP-17 platform-payment addresses hold credits, not Core UTXOs, so
            // their balance comes from the platform-address cache rather than
            // the UTXO-derived `address_balances`.
            if derivation_path.is_platform_payment(self.network)
                && let Ok(platform_addr) = PlatformAddress::try_from(address.clone())
            {
                let addr_str = platform_addr.to_bech32m_string(self.network);
                let balance = guard
                    .platform_address_info
                    .get(address)
                    .map(|info| info.balance)
                    .unwrap_or(0);
                let bech32m = addr_str.clone();
                self.all_entries.push(AddressEntry {
                    address_string: addr_str,
                    address_kind: AddressKind::Platform,
                    name_label: None,
                    wallet_name: wallet_name.clone(),
                    balance,
                    validated: ValidatedAddress::Platform {
                        address: platform_addr,
                        bech32m,
                    },
                });
            }
        }
    }

    fn extract_identity_entries(&mut self, identities: &[QualifiedIdentity]) {
        for qi in identities {
            let id = qi.identity.id();
            let id_str = id.to_string(Encoding::Base58);
            let dpns_name = qi.dpns_names.first().map(|n| n.name.clone());
            let name_label = dpns_name.clone().or_else(|| qi.alias.clone());
            self.all_entries.push(AddressEntry {
                address_string: id_str,
                address_kind: AddressKind::Identity,
                name_label,
                wallet_name: None,
                balance: qi.identity.balance(),
                validated: ValidatedAddress::Identity {
                    id,
                    dpns_name: dpns_name.clone(),
                },
            });
        }
    }

    fn add_shielded_entry(&mut self, address: String, balance: u64) {
        self.all_entries.push(AddressEntry {
            address_string: address.clone(),
            address_kind: AddressKind::Shielded,
            name_label: None,
            wallet_name: None,
            balance,
            validated: ValidatedAddress::Shielded(address),
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

        let detected_kind = detected
            .to_address_kind()
            .expect("invariant: detected is a known type, Unknown handled above");

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
        // Network prefix validation is centralized in `model/address.rs` so the
        // GUI and the MCP tools share one source of truth.
        if let Err(e) =
            crate::model::address::validate_platform_address_for_network(&canonical, self.network)
        {
            return (Some(e.to_string()), None);
        }
        match PlatformAddress::from_bech32m_string(&canonical) {
            Ok(pa) => (
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
        // Raw hex form (43 bytes = 86 hex chars) is network-agnostic — accept it
        // directly via the shared parser. This preserves the "…or hex" recipient
        // entry the standalone private-send screen advertised.
        if trimmed.len() == crate::model::address::SHIELDED_ADDRESS_RAW_LEN * 2
            && trimmed.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return match crate::model::address::parse_shielded_recipient(trimmed) {
                Some(_) => (None, Some(ValidatedAddress::Shielded(trimmed.to_string()))),
                None => (
                    Some(
                        "This private address is not valid. Please check it and try again."
                            .to_string(),
                    ),
                    None,
                ),
            };
        }

        // Network prefix validation is centralized in `model/address.rs` so the
        // GUI and the MCP tools share one source of truth.
        if let Err(e) =
            crate::model::address::validate_orchard_address_for_network(trimmed, self.network)
        {
            return (Some(e.to_string()), None);
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
            Ok(_) => {
                // Network is already validated above via the expected_prefix check
                // (dash1z for mainnet, tdash1z for non-mainnet). `from_bech32m_string`
                // no longer returns the network — the prefix guard is the sole
                // network discriminator for shielded addresses.
                (None, Some(ValidatedAddress::Shielded(trimmed.to_string())))
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
    ///
    /// The query is parsed as a GitHub-style tag search ([`ParsedQuery`]): free
    /// text still does a substring search across every field, while `type:` and
    /// `wallet:` tags narrow by kind and owning wallet.
    fn filtered_entries(&self) -> (Vec<&AddressEntry>, usize) {
        let parsed = ParsedQuery::parse(self.input_text.trim(), &self.enabled_kinds);

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
                parsed.matches(e)
            })
            .collect();

        // Sort: free-text prefix matches on the address float to the top, then
        // alphabetical by name (falling back to the address).
        let needle = parsed.prefix_needle();
        results.sort_by(|a, b| match &needle {
            None => a.sort_key().cmp(b.sort_key()),
            Some(needle) => {
                let a_prefix = a.address_string.to_lowercase().starts_with(needle);
                let b_prefix = b.address_string.to_lowercase().starts_with(needle);
                b_prefix
                    .cmp(&a_prefix)
                    .then_with(|| a.sort_key().cmp(b.sort_key()))
            }
        });

        let total = results.len();
        results.truncate(10);
        (results, total)
    }

    /// The placeholder legend shown when no caller supplied an explicit hint:
    /// `type:core|platform|... wallet:abc|def|...`, built from live data.
    ///
    /// The types segment lists the enabled kinds (in [`AddressKind::ALL`] order);
    /// the wallets segment lists up to five distinct wallet names, alphabetically,
    /// with a `(+N more)` indicator when more exist. The wallets segment is
    /// omitted entirely when no wallets are loaded.
    fn dynamic_hint(&self) -> String {
        let mut segments = Vec::new();

        let types: Vec<String> = AddressKind::ALL
            .iter()
            .copied()
            .filter(|k| self.enabled_kinds.contains(k))
            .map(|k| k.short_label().to_lowercase())
            .collect();
        if !types.is_empty() {
            segments.push(format!("type:{}", types.join("|")));
        }

        let mut names: Vec<&str> = self
            .all_entries
            .iter()
            .filter_map(|e| e.wallet_name.as_deref())
            .collect();
        names.sort_unstable();
        names.dedup();
        if !names.is_empty() {
            let shown = names.iter().take(5).copied().collect::<Vec<_>>().join("|");
            let mut segment = format!("wallet:{shown}");
            if names.len() > 5 {
                segment.push_str(&format!(" (+{} more)", names.len() - 5));
            }
            segments.push(segment);
        }

        segments.join(" ")
    }

    /// The hint text actually shown in the field: the caller's explicit hint if
    /// set, otherwise the [`dynamic_hint`](Self::dynamic_hint) legend.
    pub fn effective_hint_text(&self) -> String {
        self.hint_text
            .clone()
            .unwrap_or_else(|| self.dynamic_hint())
    }

    /// The render descriptors for the currently filtered rows. Exposed for UI
    /// tests to assert which pills and name annotations appear per row.
    pub fn rendered_rows(&self) -> Vec<RenderedRow> {
        self.filtered_entries()
            .0
            .into_iter()
            .map(|e| RenderedRow {
                address_string: e.address_string.clone(),
                wallet_pill: e.wallet_name.clone(),
                name: e.name_label.clone(),
                kind: e.address_kind,
            })
            .collect()
    }

    // --- Balance formatting ---

    fn format_balance(&self, entry: &AddressEntry) -> String {
        match entry.address_kind {
            AddressKind::Core => Self::format_dash_4dp(Amount::dash_from_duffs(entry.balance)),
            AddressKind::Platform | AddressKind::Shielded | AddressKind::Identity => {
                let dash = Amount::new(entry.balance, DASH_DECIMAL_PLACES).with_unit_name("DASH");
                Self::format_dash_4dp(dash)
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
        // Computed before the `&mut self.input_text` borrow below; falls back to
        // the dynamic tag-search legend when no explicit hint was supplied.
        let hint_text = self.effective_hint_text();
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
                    if !hint_text.is_empty() {
                        text_edit = text_edit
                            .hint_text(egui::RichText::new(&hint_text).color(egui::Color32::GRAY));
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
                // Collect filtered entries into an owned snapshot to release the
                // borrow on self: (balance, address text, entry).
                let (filtered, total_entries) = self.filtered_entries();
                let filtered_len = filtered.len();
                let entries_snapshot: Vec<(String, String, AddressEntry)> = filtered
                    .iter()
                    .map(|e| {
                        let address_display = if self.full_addresses {
                            e.address_string.clone()
                        } else {
                            truncate_address(&e.address_string)
                        };
                        (self.format_balance(e), address_display, (*e).clone())
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
                                        for (i, (balance_str, address_display, entry)) in
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
                                                paint_autocomplete_row(
                                                    ui,
                                                    rect,
                                                    highlighted || response.hovered(),
                                                    entry,
                                                    address_display,
                                                    balance_str,
                                                );
                                            }

                                            let label = row_accessible_label(
                                                entry,
                                                address_display,
                                                balance_str,
                                            );
                                            response.widget_info(|| {
                                                egui::WidgetInfo::labeled(
                                                    egui::WidgetType::Button,
                                                    true,
                                                    label.clone(),
                                                )
                                            });

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

            // Handle autocomplete selection (clear cached_detection).
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

            // Build response.
            // Blur validation producing a result signals changed.
            let blur_validated = lost_focus && validated_address.is_some();
            // One-frame local flag for autocomplete selection.
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

// --- Autocomplete row rendering ---

/// Horizontal padding inside a pill and the gap after a pill or the address text.
const PILL_PAD_X: f32 = 6.0;
const PILL_GAP: f32 = 6.0;

/// Paint one autocomplete row: an optional hover highlight, then, left to right,
/// `[wallet pill] address (name)`, with the type pill and balance right-aligned.
fn paint_autocomplete_row(
    ui: &Ui,
    rect: egui::Rect,
    active: bool,
    entry: &AddressEntry,
    address_display: &str,
    balance_str: &str,
) {
    let dark_mode = ui.visuals().dark_mode;

    if active {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::from(2.0),
            ui.style().visuals.widgets.hovered.bg_fill,
        );
    }

    let text_color = if active {
        ui.style().visuals.widgets.hovered.text_color()
    } else {
        ui.style().visuals.widgets.inactive.text_color()
    };
    let secondary = DashColors::text_secondary(dark_mode);
    let center_y = rect.center().y;
    let mut x = rect.left() + PILL_GAP;

    // Wallet pill (Core/Platform only).
    if let Some(wallet) = &entry.wallet_name {
        x += paint_pill(
            ui,
            x,
            center_y,
            wallet,
            DashColors::text_primary(dark_mode),
            wallet_pill_bg(),
        ) + PILL_GAP;
    }

    // Address text.
    let body_font = egui::TextStyle::Body.resolve(ui.style());
    let addr_galley =
        ui.painter()
            .layout_no_wrap(address_display.to_string(), body_font.clone(), text_color);
    let addr_width = addr_galley.size().x;
    ui.painter().galley(
        egui::pos2(x, center_y - addr_galley.size().y / 2.0),
        addr_galley,
        text_color,
    );
    x += addr_width + PILL_GAP;

    // Name annotation, e.g. "(alice.dash)" or "(change)".
    if let Some(name) = &entry.name_label {
        let name_galley = ui
            .painter()
            .layout_no_wrap(format!("({name})"), body_font, secondary);
        ui.painter().galley(
            egui::pos2(x, center_y - name_galley.size().y / 2.0),
            name_galley,
            secondary,
        );
    }

    // Right cluster: balance rightmost, type pill immediately to its left.
    let small_font = egui::TextStyle::Small.resolve(ui.style());
    let bal_galley =
        ui.painter()
            .layout_no_wrap(balance_str.to_string(), small_font, DashColors::GRAY);
    let bal_width = bal_galley.size().x;
    let bal_x = rect.right() - PILL_GAP - bal_width;
    ui.painter().galley(
        egui::pos2(bal_x, center_y - bal_galley.size().y / 2.0),
        bal_galley,
        DashColors::GRAY,
    );

    let type_text = entry.address_kind.short_label();
    let type_width = measure_pill(ui, type_text);
    let type_left = bal_x - PILL_GAP - type_width;
    paint_pill(
        ui,
        type_left,
        center_y,
        type_text,
        secondary,
        type_pill_bg(),
    );
}

/// Font used for pill text.
fn pill_font(ui: &Ui) -> egui::FontId {
    egui::TextStyle::Small.resolve(ui.style())
}

/// The width a pill would occupy for `text`, without painting it.
fn measure_pill(ui: &Ui, text: &str) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), pill_font(ui), Color32::PLACEHOLDER);
    galley.size().x + PILL_PAD_X * 2.0
}

/// Paint a compact rounded pill with its left edge at `left`, vertically centered
/// on `center_y`. Returns the pill's width.
fn paint_pill(
    ui: &Ui,
    left: f32,
    center_y: f32,
    text: &str,
    text_color: Color32,
    bg: Color32,
) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), pill_font(ui), text_color);
    let text_size = galley.size();
    let width = text_size.x + PILL_PAD_X * 2.0;
    let height = text_size.y + 2.0;
    let rect = egui::Rect::from_min_size(
        egui::pos2(left, center_y - height / 2.0),
        egui::vec2(width, height),
    );
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(Shape::RADIUS_FULL), bg);
    ui.painter().galley(
        egui::pos2(left + PILL_PAD_X, center_y - text_size.y / 2.0),
        galley,
        text_color,
    );
    width
}

/// Translucent accent tint for the wallet pill.
fn wallet_pill_bg() -> Color32 {
    let c = DashColors::DASH_BLUE;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 48)
}

/// Translucent neutral tint for the type pill, distinct from the wallet pill.
fn type_pill_bg() -> Color32 {
    let c = DashColors::GRAY;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 48)
}

/// One-line accessible label mirroring a row's painted content, for screen
/// readers and UI tests (the row is otherwise painted directly and exposes no text).
fn row_accessible_label(entry: &AddressEntry, address_display: &str, balance_str: &str) -> String {
    let mut label = String::new();
    if let Some(wallet) = &entry.wallet_name {
        label.push_str(wallet);
        label.push(' ');
    }
    label.push_str(address_display);
    if let Some(name) = &entry.name_label {
        label.push_str(&format!(" ({name})"));
    }
    label.push_str(&format!(
        " {} {}",
        entry.address_kind.short_label(),
        balance_str
    ));
    label
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

    /// A testnet BIP-44 external (receive) path `m/44'/1'/0'/0/index`.
    fn bip44_receive_path(index: u32) -> DerivationPath {
        use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
        DerivationPath::from(
            [
                ChildNumber::Hardened { index: 44 },
                ChildNumber::Hardened { index: 1 },
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index },
            ]
            .as_slice(),
        )
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
    fn detect_shielded_raw_hex() {
        // 43-byte raw hex form (network-agnostic) routes to shielded validation
        // so the "…or hex" recipient entry keeps working.
        let hex_str = hex::encode(vec![
            0xABu8;
            crate::model::address::SHIELDED_ADDRESS_RAW_LEN
        ]);
        assert_eq!(detect_address_type(&hex_str, true), DetectedType::Shielded);
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

    // --- Blur validation propagation ---

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

    // --- Mixed-case bech32m rejection ---

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

    // --- Search-tag query parser ---

    fn test_entry(
        kind: AddressKind,
        addr: &str,
        name: Option<&str>,
        wallet: Option<&str>,
    ) -> AddressEntry {
        AddressEntry {
            address_string: addr.to_string(),
            address_kind: kind,
            name_label: name.map(String::from),
            wallet_name: wallet.map(String::from),
            balance: 0,
            validated: ValidatedAddress::Shielded(addr.to_string()),
        }
    }

    #[test]
    fn parse_type_tag_selects_kind() {
        let q = ParsedQuery::parse("type:core", &AddressKind::ALL);
        assert!(q.has_type_constraint);
        assert_eq!(q.type_kinds, vec![AddressKind::Core]);
        assert!(q.wallet_values.is_empty());
        assert!(q.free_text.is_empty());
    }

    #[test]
    fn parse_type_prefix_matches() {
        assert_eq!(
            ParsedQuery::parse("type:cor", &AddressKind::ALL).type_kinds,
            vec![AddressKind::Core]
        );
        assert_eq!(
            ParsedQuery::parse("type:plat", &AddressKind::ALL).type_kinds,
            vec![AddressKind::Platform]
        );
    }

    #[test]
    fn parse_type_is_case_insensitive() {
        assert_eq!(
            ParsedQuery::parse("TYPE:Core", &AddressKind::ALL).type_kinds,
            vec![AddressKind::Core]
        );
    }

    #[test]
    fn parse_type_tokens_union() {
        let q = ParsedQuery::parse("type:core type:platform", &AddressKind::ALL);
        assert_eq!(q.type_kinds, vec![AddressKind::Core, AddressKind::Platform]);
        assert!(q.matches(&test_entry(AddressKind::Core, "x", None, None)));
        assert!(q.matches(&test_entry(AddressKind::Platform, "x", None, None)));
        assert!(!q.matches(&test_entry(AddressKind::Shielded, "x", None, None)));
    }

    #[test]
    fn parse_wallet_tag_substring_matches() {
        let q = ParsedQuery::parse("wallet:abc", &AddressKind::ALL);
        assert_eq!(q.wallet_values, vec!["abc".to_string()]);
        assert!(q.matches(&test_entry(AddressKind::Core, "x", None, Some("abcdef"))));
        assert!(!q.matches(&test_entry(AddressKind::Core, "x", None, Some("xyz"))));
        assert!(!q.matches(&test_entry(AddressKind::Core, "x", None, None)));
    }

    #[test]
    fn parse_wallet_is_case_insensitive() {
        let q = ParsedQuery::parse("wallet:ABC", &AddressKind::ALL);
        assert!(q.matches(&test_entry(
            AddressKind::Core,
            "x",
            None,
            Some("MyAbcWallet")
        )));
    }

    #[test]
    fn parse_tags_and_together() {
        let q = ParsedQuery::parse("type:core wallet:abc", &AddressKind::ALL);
        assert!(q.matches(&test_entry(
            AddressKind::Core,
            "x",
            None,
            Some("abc wallet")
        )));
        // Right wallet, wrong type.
        assert!(!q.matches(&test_entry(
            AddressKind::Platform,
            "x",
            None,
            Some("abc wallet")
        )));
        // Right type, wrong wallet.
        assert!(!q.matches(&test_entry(AddressKind::Core, "x", None, Some("zzz"))));
    }

    #[test]
    fn free_text_tokens_and_together() {
        let q = ParsedQuery::parse("foo bar", &AddressKind::ALL);
        assert_eq!(q.free_text, vec!["foo".to_string(), "bar".to_string()]);
        assert!(q.matches(&test_entry(AddressKind::Core, "foobar", None, None)));
        assert!(!q.matches(&test_entry(AddressKind::Core, "foo", None, None)));
    }

    #[test]
    fn unknown_key_is_treated_as_free_text() {
        let q = ParsedQuery::parse("foo:bar", &AddressKind::ALL);
        assert!(!q.has_type_constraint);
        assert!(q.type_kinds.is_empty());
        assert!(q.wallet_values.is_empty());
        assert_eq!(q.free_text, vec!["foo:bar".to_string()]);
    }

    #[test]
    fn empty_tag_value_is_ignored() {
        let type_only = ParsedQuery::parse("type:", &AddressKind::ALL);
        assert!(!type_only.has_type_constraint);
        assert!(type_only.free_text.is_empty());
        assert!(type_only.matches(&test_entry(AddressKind::Shielded, "x", None, None)));

        let wallet_only = ParsedQuery::parse("wallet:", &AddressKind::ALL);
        assert!(wallet_only.wallet_values.is_empty());
        assert!(wallet_only.matches(&test_entry(AddressKind::Core, "x", None, None)));
    }

    #[test]
    fn type_restricted_to_enabled_kinds() {
        let q = ParsedQuery::parse("type:core", &[AddressKind::Platform]);
        assert!(q.has_type_constraint);
        assert!(q.type_kinds.is_empty());
        // A kind not enabled for this instance can never match.
        assert!(!q.matches(&test_entry(AddressKind::Core, "x", None, None)));
    }

    #[test]
    fn type_matching_no_kind_excludes_all() {
        let q = ParsedQuery::parse("type:zzz", &AddressKind::ALL);
        assert!(q.has_type_constraint);
        assert!(q.type_kinds.is_empty());
        assert!(!q.matches(&test_entry(AddressKind::Core, "x", None, None)));
    }

    #[test]
    fn free_text_matches_kind_label() {
        // Preserves the pre-tag behavior: typing "core" filters to Core entries.
        let q = ParsedQuery::parse("core", &AddressKind::ALL);
        assert!(q.matches(&test_entry(AddressKind::Core, "x", None, None)));
        assert!(!q.matches(&test_entry(AddressKind::Shielded, "x", None, None)));
    }

    #[test]
    fn empty_query_matches_everything() {
        let q = ParsedQuery::parse("", &AddressKind::ALL);
        assert!(q.matches(&test_entry(AddressKind::Identity, "x", None, None)));
        assert!(q.prefix_needle().is_none());
    }

    #[test]
    fn prefix_needle_is_joined_free_text() {
        let q = ParsedQuery::parse("type:core abc", &AddressKind::ALL);
        assert_eq!(q.prefix_needle().as_deref(), Some("abc"));
    }

    // --- Entry population ---

    /// An input fed one snapshot-sourced BIP-44 receive address.
    fn core_wallet_input(alias: Option<&str>) -> AddressInput {
        use std::collections::BTreeMap;
        let wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, alias.map(String::from), None)
                .expect("wallet from seed");
        let (_, address) = testnet_core_address();
        let paths = BTreeMap::from([(address, bip44_receive_path(0))]);
        AddressInput::new(Network::Testnet).with_wallets(&[(
            Arc::new(RwLock::new(wallet)),
            BTreeMap::new(),
            paths,
        )])
    }

    fn identity_input(alias: Option<&str>, dpns: Option<&str>) -> AddressInput {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{
            DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
        };
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use std::collections::BTreeMap;

        let identity =
            Identity::create_basic_identity(Identifier::from([9u8; 32]), PlatformVersion::latest())
                .expect("basic identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: alias.map(String::from),
            private_keys: KeyStorage::default(),
            dpns_names: dpns
                .map(|n| {
                    vec![DPNSNameInfo {
                        name: n.to_string(),
                        acquired_at: 0,
                    }]
                })
                .unwrap_or_default(),
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::PendingCreation,
            network: Network::Testnet,
        };
        AddressInput::new(Network::Testnet).with_identities(&[qi])
    }

    #[test]
    fn core_entry_gets_wallet_name_and_no_name_label() {
        let input = core_wallet_input(Some("MyWallet"));
        let core = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Core)
            .expect("core entry present");
        assert_eq!(core.wallet_name.as_deref(), Some("MyWallet"));
        assert_eq!(core.name_label, None, "receive address has no name label");
    }

    #[test]
    fn core_entry_defaults_wallet_name_when_no_alias() {
        let input = core_wallet_input(None);
        let core = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Core)
            .expect("core entry present");
        assert_eq!(core.wallet_name.as_deref(), Some("Wallet"));
    }

    /// A testnet BIP-44 change path `m/44'/1'/0'/1/0` (components[3] == Normal(1)).
    fn bip44_change_path() -> DerivationPath {
        use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
        DerivationPath::from(
            [
                ChildNumber::Hardened { index: 44 },
                ChildNumber::Hardened { index: 1 },
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index: 1 },
                ChildNumber::Normal { index: 0 },
            ]
            .as_slice(),
        )
    }

    #[test]
    fn core_change_entry_gets_change_name_label() {
        use std::collections::BTreeMap;

        let wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, Some("W".to_string()), None)
                .expect("wallet from seed");
        let (_, change_addr) = testnet_core_address();
        let paths = BTreeMap::from([(change_addr, bip44_change_path())]);

        let input = AddressInput::new(Network::Testnet).with_wallets(&[(
            Arc::new(RwLock::new(wallet)),
            BTreeMap::new(),
            paths,
        )]);
        let change = input
            .all_entries
            .iter()
            .find(|e| e.name_label.as_deref() == Some("change"))
            .expect("change entry present");
        assert_eq!(change.address_kind, AddressKind::Core);
        assert_eq!(change.wallet_name.as_deref(), Some("W"));
    }

    #[test]
    fn core_change_entry_excluded_when_requested() {
        use std::collections::BTreeMap;

        let wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, Some("W".to_string()), None)
                .expect("wallet from seed");
        let (_, change_addr) = testnet_core_address();
        let paths = BTreeMap::from([(change_addr, bip44_change_path())]);

        let input = AddressInput::new(Network::Testnet)
            .with_exclude_change(true)
            .with_wallets(&[(Arc::new(RwLock::new(wallet)), BTreeMap::new(), paths)]);
        assert!(
            input
                .all_entries
                .iter()
                .all(|e| e.name_label.as_deref() != Some("change")),
            "change address must be excluded when with_exclude_change(true)"
        );
    }

    /// A BIP-44 path present only in the snapshot — the wallet's legacy
    /// `known_addresses` never saw it — must still surface a Core entry. The
    /// snapshot is the source of truth for what the autocomplete may offer.
    #[test]
    fn core_entries_are_sourced_from_the_snapshot_paths() {
        use std::collections::BTreeMap;

        let mut wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, Some("W".to_string()), None)
                .expect("wallet from seed");
        wallet.known_addresses.clear();
        wallet.watched_addresses.clear();

        let (addr_str, address) = testnet_core_address();
        let paths = BTreeMap::from([(address, bip44_receive_path(0))]);

        let input = AddressInput::new(Network::Testnet).with_wallets(&[(
            Arc::new(RwLock::new(wallet)),
            BTreeMap::new(),
            paths,
        )]);

        let core = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Core)
            .expect("snapshot-sourced core entry present");
        assert_eq!(core.address_string, addr_str);
        assert_eq!(core.wallet_name.as_deref(), Some("W"));
    }

    /// A DIP-17 platform-payment path present only in the snapshot — the
    /// wallet's legacy `watched_addresses` never saw it — must still surface a
    /// Platform entry, rendered in DIP-18 bech32m.
    #[test]
    fn platform_entries_are_sourced_from_the_snapshot_paths() {
        use std::collections::BTreeMap;

        let mut wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, Some("W".to_string()), None)
                .expect("wallet from seed");
        wallet.known_addresses.clear();
        wallet.watched_addresses.clear();

        let (_, address) = testnet_core_address();
        let expected = PlatformAddress::try_from(address.clone())
            .expect("core address converts to a platform address")
            .to_bech32m_string(Network::Testnet);
        let paths = BTreeMap::from([(
            address,
            DerivationPath::platform_payment_path(Network::Testnet, 0, 0, 0),
        )]);

        let input = AddressInput::new(Network::Testnet).with_wallets(&[(
            Arc::new(RwLock::new(wallet)),
            BTreeMap::new(),
            paths,
        )]);

        let platform = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Platform)
            .expect("snapshot-sourced platform entry present");
        assert_eq!(platform.address_string, expected);
        assert_eq!(platform.wallet_name.as_deref(), Some("W"));
    }

    /// FUNDS-SAFETY: the legacy `known_addresses`/`watched_addresses` maps are
    /// no longer an autocomplete source. DET's own bootstrap can derive
    /// addresses upstream does not watch (past the gap limit, or a stale
    /// rehydrate); offering one would invite funds to an address SPV never sees.
    /// With an empty snapshot, a fully-populated legacy map must yield nothing.
    #[test]
    fn legacy_maps_never_source_autocomplete_entries() {
        use crate::model::wallet::{AddressInfo, DerivationPathReference, DerivationPathType};
        use std::collections::BTreeMap;

        // `new_from_seed` seeds `known_addresses` with the first BIP-44 receive
        // address; add a platform-payment entry so both legacy branches are live.
        let mut wallet =
            Wallet::new_from_seed([7u8; 64], Network::Testnet, Some("W".to_string()), None)
                .expect("wallet from seed");
        assert!(
            !wallet.known_addresses.is_empty(),
            "precondition: the legacy map is populated, so this test can prove it is ignored"
        );
        let (_, address) = testnet_core_address();
        let platform_path = DerivationPath::platform_payment_path(Network::Testnet, 0, 0, 0);
        wallet.watched_addresses.insert(
            platform_path.clone(),
            AddressInfo {
                address,
                path_reference: DerivationPathReference::PlatformPayment,
                path_type: DerivationPathType::CLEAR_FUNDS,
            },
        );

        let input = AddressInput::new(Network::Testnet).with_wallets(&[(
            Arc::new(RwLock::new(wallet)),
            BTreeMap::new(),
            BTreeMap::new(),
        )]);

        assert!(
            !input
                .all_entries
                .iter()
                .any(|e| matches!(e.address_kind, AddressKind::Core | AddressKind::Platform)),
            "an empty snapshot must yield no wallet entries, whatever the legacy maps hold"
        );
    }

    #[test]
    fn shielded_entry_has_no_name_or_wallet() {
        let input = AddressInput::new(Network::Testnet)
            .with_shielded_balance("tdash1zexampleaddress".to_string(), 100);
        let entry = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Shielded)
            .expect("shielded entry present");
        assert_eq!(entry.name_label, None);
        assert_eq!(entry.wallet_name, None);
    }

    #[test]
    fn identity_entry_uses_alias_when_no_dpns() {
        let input = identity_input(Some("bob-alias"), None);
        let entry = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Identity)
            .expect("identity entry present");
        assert_eq!(entry.name_label.as_deref(), Some("bob-alias"));
        assert_eq!(entry.wallet_name, None);
    }

    #[test]
    fn identity_entry_prefers_dpns_over_alias() {
        let input = identity_input(Some("bob-alias"), Some("bob.dash"));
        let entry = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Identity)
            .expect("identity entry present");
        assert_eq!(entry.name_label.as_deref(), Some("bob.dash"));
    }

    #[test]
    fn identity_entry_has_no_name_when_neither() {
        let input = identity_input(None, None);
        let entry = input
            .all_entries
            .iter()
            .find(|e| e.address_kind == AddressKind::Identity)
            .expect("identity entry present");
        assert_eq!(entry.name_label, None);
    }

    // --- Dynamic hint legend ---

    #[test]
    fn dynamic_hint_lists_enabled_kinds() {
        let input = AddressInput::new(Network::Testnet);
        assert_eq!(
            input.effective_hint_text(),
            "type:core|platform|shielded|identity"
        );
    }

    #[test]
    fn dynamic_hint_reflects_restricted_kinds() {
        let input = AddressInput::new(Network::Testnet)
            .with_address_kinds(&[AddressKind::Core, AddressKind::Platform]);
        assert_eq!(input.effective_hint_text(), "type:core|platform");
    }

    #[test]
    fn explicit_hint_overrides_dynamic_legend() {
        let input = AddressInput::new(Network::Testnet).with_hint_text("Paste an address");
        assert_eq!(input.effective_hint_text(), "Paste an address");
    }

    #[test]
    fn dynamic_hint_includes_and_trims_wallets() {
        use std::collections::BTreeMap;
        let (_, address) = testnet_core_address();
        let wallets: Vec<WalletWithSnapshot> = (0u8..6)
            .map(|i| {
                let wallet = Wallet::new_from_seed(
                    [i + 1; 64],
                    Network::Testnet,
                    Some(format!("w{i}")),
                    None,
                )
                .expect("wallet from seed");
                let paths = BTreeMap::from([(address.clone(), bip44_receive_path(0))]);
                (Arc::new(RwLock::new(wallet)), BTreeMap::new(), paths)
            })
            .collect();
        let input = AddressInput::new(Network::Testnet).with_wallets(&wallets);
        let hint = input.effective_hint_text();
        assert!(
            hint.contains("wallet:w0|w1|w2|w3|w4"),
            "hint should list the first five wallets: {hint}"
        );
        assert!(
            hint.contains("(+1 more)"),
            "hint should indicate one trimmed wallet: {hint}"
        );
    }
}
