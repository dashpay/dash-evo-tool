# Wallet Screen Component Specifications

Component specifications for the redesigned wallet screen, aligned with the egui immediate-mode GUI framework and the existing component design pattern documented in `doc/COMPONENT_DESIGN_PATTERN.md`.

Each component follows these conventions:
- Private fields only
- Builder methods for configuration
- Lazy initialization via `Option<Component>` pattern
- Response struct implementing `ComponentResponse` trait
- Self-contained validation and error handling

---

## 1. WalletSelector

**Purpose**: Global wallet selector displayed in the top panel. Allows switching between wallets and accessing create/import actions.

### Variants

| Variant | When Shown |
|---|---|
| `Selected(WalletSummary)` | A wallet is selected. Shows type badge, alias, balance. |
| `NoWallets` | No wallets loaded. Shows "Select a Wallet" placeholder. |

### Props / Inputs

```rust
struct WalletSelector {
    // Data (from AppContext)
    wallets: Vec<WalletSummary>,
    single_key_wallets: Vec<SingleKeyWalletSummary>,
    selected_wallet_id: Option<WalletSeedHash>,
    selected_sk_wallet_id: Option<String>, // address as key

    // UI state
    dropdown_open: bool,
}

struct WalletSummary {
    id: WalletSeedHash,
    alias: String,
    total_balance: Amount,
    is_locked: bool,
    wallet_type: WalletType, // HD or SingleKey
}
```

### Response

```rust
struct WalletSelectorResponse {
    selected_wallet: Option<WalletSeedHash>,        // HD wallet selected
    selected_sk_wallet: Option<String>,              // SK wallet selected
    create_wallet_requested: bool,
    import_wallet_requested: bool,
}
```

### States

| State | Visual |
|---|---|
| `closed` | Single line: `[HD] My Wallet  1.2345 DASH  v` |
| `open` | Dropdown list with all wallets + actions |
| `no_wallets` | `Select a Wallet  v` (greyed placeholder) |

### Accessibility

- Keyboard: Tab to focus, Enter/Space to open, Arrow keys to navigate list, Enter to select, Escape to close
- The selected wallet name is the accessible label for the control

### Responsive Behavior

- On narrow windows (< 900px): Hide balance from selector, show only alias and type badge
- Dropdown list always shows full info regardless of window width

---

## 2. BalanceHeader

**Purpose**: Displays the wallet's total balance prominently. Supports expandable breakdown.

### Props / Inputs

```rust
struct BalanceHeader {
    // Data
    wallet_alias: String,
    wallet_type: WalletType,
    total_balance: Amount,
    core_balance: Option<Amount>,
    platform_balance: Option<Amount>,
    platform_credits: Option<u64>,       // raw credits (Level 2)
    unconfirmed_balance: Option<Amount>,  // Level 2
    is_locked: bool,
    is_refreshing: bool,
    developer_mode: bool,

    // UI state
    breakdown_expanded: bool,
    balance_flash: Option<(BalanceFlashType, Instant)>,
}

enum BalanceFlashType {
    Increase,
    Decrease,
}
```

### Response

```rust
struct BalanceHeaderResponse {
    overflow_menu_action: Option<OverflowAction>,
    breakdown_toggled: bool,
}

enum OverflowAction {
    Rename,
    Remove,
    Lock,
    Unlock,
    ShowRecoveryPhrase,
}
```

### Visual Layout

```
  Wallet Alias                               [... menu]
  ============

      1.2345 DASH                            [Lock icon]
      Total Balance

  [Show breakdown v]  (or expanded breakdown lines)
```

### Styling

| Element | Style |
|---|---|
| Wallet alias | 18pt, bold, DashColors text color |
| Total balance | 32pt, bold, DashColors text color |
| "Total Balance" label | 12pt, secondary text color |
| Breakdown lines | 14pt, secondary text color |
| Lock icon | 16px, amber when locked, green when unlocked |
| Balance flash (increase) | Background: DashColors::SUCCESS at 20% opacity, fades over 1 second |
| Balance flash (decrease) | Background: DashColors::ERROR at 20% opacity, fades over 1 second |

---

## 3. ActionBar

**Purpose**: Horizontal bar of primary action buttons below the balance header.

### Props / Inputs

```rust
struct ActionBar {
    // State
    is_refreshing: bool,
    is_locked: bool,
    wallet_type: WalletType,
    network: Network,
    developer_mode: bool,
    selected_refresh_mode: RefreshMode,

    // Configuration
    show_refresh_mode_selector: bool, // true at Level 1+
    show_faucet_button: bool,         // true at Level 2 on Testnet/Devnet
}
```

### Response

```rust
struct ActionBarResponse {
    action: Option<ActionBarAction>,
}

enum ActionBarAction {
    Send,
    Receive,
    Refresh(RefreshMode),
    RequestTestDash,
}
```

### Button Layout

```
[  Send  ]    [  Receive  ]    [  Refresh  [Mode v] ]    [Get Test Dash]
  primary       primary           secondary               secondary
```

### Styling

| Button | Style |
|---|---|
| Send | Primary action: DashColors::DASH_BLUE background, white text, prominent |
| Receive | Primary action: Same styling as Send |
| Refresh | Secondary action: Outlined, default text color |
| Refresh (refreshing) | Disabled, spinner icon instead of refresh icon |
| Refresh mode selector | Small dropdown appended to Refresh button |
| Get Test Dash | Secondary action, visible only on Testnet with Developer Tools |

### Behavior When Locked

- Send button shows "(locked)" subtitle in smaller text
- Clicking Send triggers unlock popup, then proceeds to send flow
- Receive and Refresh work without unlock

---

## 4. TransactionHistorySection

**Purpose**: Collapsible section showing recent or all wallet transactions.

### Props / Inputs

```rust
struct TransactionHistorySection {
    // Data
    transactions: Vec<WalletTransaction>,

    // UI state
    expanded: bool,             // section expand/collapse
    show_all: bool,             // false = last 10, true = all with pagination
    current_page: usize,
    page_size: usize,           // default 25
    sort_column: TxSortColumn,
    sort_order: SortOrder,
    filter: TransactionFilter,
    developer_mode: bool,
}

enum TxSortColumn {
    Date,
    Amount,
    Status,
}

enum TransactionFilter {
    All,
    Sent,
    Received,
    Internal,
}
```

### Response

```rust
struct TransactionHistoryResponse {
    copied_txid: Option<String>,
    export_csv_requested: bool,  // Level 2 only
}
```

### Table Columns

| Column | Level 0 | Level 1 | Level 2 |
|---|---|---|---|
| Date | Yes | Yes | Yes |
| Type (icon) | Yes | Yes | Yes |
| Amount | Yes | Yes | Yes |
| Fee | No | Yes | Yes |
| Status | Yes (icon) | Yes (icon + text) | Yes (icon + text) |
| TxID | Yes (truncated) | Yes (truncated, click to copy) | Yes (truncated, click to copy) |

### Empty State

```
No transactions yet.
Send or receive Dash to see your transaction history here.
```

---

## 5. AccountsSection

**Purpose**: Collapsible section containing per-account-category sub-sections, each with an address table.

### Props / Inputs

```rust
struct AccountsSection {
    // Data
    accounts: Vec<AccountSummary>,
    developer_mode: bool,

    // UI state
    section_expanded: bool,
    expanded_accounts: HashSet<AccountCategory>,
    activity_filter: bool, // Level 2: hide empty accounts
}

struct AccountSummary {
    category: AccountCategory,
    label: String,        // user-facing label from terminology guide
    description: String,  // tooltip/inline help text
    balance: Option<Amount>,  // None for key-only categories
    address_count: usize,
    is_key_only: bool,
}
```

### Response

```rust
struct AccountsSectionResponse {
    add_address_requested: Option<AccountCategory>,
    view_key_requested: Option<(AccountCategory, Address)>,
    fund_platform_requested: Option<Address>,
    withdraw_platform_requested: Option<Address>,
}
```

### Visual Layout (Expanded)

```
v Accounts & Addresses                    [Filter: Has Activity v] (L2)
|
| v Main Account                          1.0000 DASH
|   Your primary Dash addresses for sending and receiving
|   [Address Table Component]
|   [+ Add Receiving Address]
|
| v Platform Account                      0.2345 DASH
|   Addresses for holding credits on Dash Platform
|   [Address Table Component]
|   [+ New Platform Address]
|
| > Private Send                          0.0000 DASH  (collapsed, zero balance)
| > Identity Keys                         (keys only)  (collapsed, key-only)
```

### Visibility Rules

Account categories follow the visibility rules from the Information Architecture document. The component filters categories before rendering based on:
1. Current disclosure level (0/1/2)
2. Whether the category has balance or activity
3. Whether the activity filter is enabled (Level 2)

---

## 6. AddressTable

**Purpose**: Sortable table of addresses within a single account category.

### Props / Inputs

```rust
struct AddressTable {
    // Data
    addresses: Vec<AddressData>,
    account_category: AccountCategory,
    developer_mode: bool,

    // UI state
    sort_column: SortColumn,
    sort_order: SortOrder,
}

// Re-uses existing AddressData struct from address_table.rs
```

### Response

```rust
struct AddressTableResponse {
    copied_address: Option<String>,
    view_key_requested: Option<Address>,
    fund_requested: Option<Address>,       // Platform addresses
    withdraw_requested: Option<Address>,   // Platform addresses
}
```

### Columns

| Column | Level 1 | Level 2 | Sortable |
|---|---|---|---|
| Address | Yes (truncated, click to copy) | Yes | Yes |
| Balance | Yes | Yes | Yes |
| UTXOs | Yes | Yes | Yes |
| Total Received | Yes | Yes | Yes |
| Type (Receiving/Change/System) | Yes | Yes | Yes |
| Index | No | Yes | Yes |
| Path (derivation) | No | Yes | No |
| Actions | Yes | Yes | No |

### Actions Column

| Wallet Type | Account | Actions |
|---|---|---|
| HD | Main Account | [View Key] |
| HD | Platform Account | [Fund] [Withdraw] [View Key] |
| HD | Other accounts | [View Key] |

### Empty Account State

```
No addresses with activity in this account.
[+ Add Receiving Address]  (for Main Account)
```

---

## 7. AssetLocksSection

**Purpose**: Collapsible section for viewing and managing asset locks. Visible at Level 1 and above, for HD wallets only.

### Props / Inputs

```rust
struct AssetLocksSection {
    // Data
    asset_locks: Vec<AssetLockEntry>,
    developer_mode: bool,

    // UI state
    section_expanded: bool,
    searching: bool,
}

struct AssetLockEntry {
    txid: String,
    address: String,
    amount_duffs: u64,
    amount_credits: Option<u64>,
    instant_lock: bool,
    usable: bool,
}
```

### Response

```rust
struct AssetLocksSectionResponse {
    create_requested: bool,
    bulk_create_requested: bool,    // Level 2
    search_unused_requested: bool,
    view_requested: Option<String>,  // txid
    fund_requested: Option<String>,  // txid
}
```

### Table Columns

| Column | Level 1 | Level 2 |
|---|---|---|
| Transaction ID | Yes (truncated) | Yes (truncated) |
| Amount (DASH) | Yes | Yes |
| Amount (credits) | No | Yes |
| Instant Lock | Yes (icon) | Yes (icon) |
| Ready to Use | Yes (icon) | Yes (icon) |
| Actions | [View] [Fund] | [View] [Fund] |

### Section Header Actions

| Action | Level 1 | Level 2 |
|---|---|---|
| Create Asset Lock | Yes | Yes |
| Find Unused Locks | Yes | Yes |
| Create Multiple | No | Yes |

---

## 8. CollapsibleSection (Generic)

**Purpose**: Reusable container that wraps any section with expand/collapse behavior. Used by AccountsSection, AssetLocksSection, TransactionHistorySection, and the UTXO list in SingleKey view.

### Props / Inputs

```rust
struct CollapsibleSection {
    title: String,
    summary: Option<String>,  // shown next to title when collapsed (e.g., "4 accounts, 12 addresses")
    expanded: bool,
    header_actions: Vec<HeaderAction>,  // buttons shown in the header row (right-aligned)
}

struct HeaderAction {
    label: String,
    enabled: bool,
}
```

### Response

```rust
struct CollapsibleSectionResponse {
    toggled: bool,
    action_clicked: Option<usize>,  // index of header action clicked
}
```

### Visual Layout

```
Collapsed:
  > Section Title                    summary text    [Action 1] [Action 2]

Expanded:
  v Section Title                    summary text    [Action 1] [Action 2]
    Section description (optional help text)
    [Content rendered by parent]
```

### Styling

| Element | Style |
|---|---|
| Section title | 16pt, semibold |
| Summary text | 14pt, secondary color, italic |
| Arrow icon (">" / "v") | 14pt, same color as title |
| Header actions | Small buttons, secondary styling |
| Expansion area | Indented 16px from left edge |

---

## 9. MessageBanner

**Purpose**: Displays success, info, warning, and error messages below the action bar.

### Props / Inputs

```rust
struct MessageBanner {
    message: Option<MessageContent>,
}

struct MessageContent {
    text: String,
    message_type: MessageType,  // Success, Info, Warning, Error
    action: Option<MessageAction>,
    created_at: Instant,
    auto_dismiss_seconds: Option<u64>,
    detail: Option<String>,  // expandable detail (Level 2)
}

struct MessageAction {
    label: String,  // e.g., "Retry", "Try Again"
}
```

### Response

```rust
struct MessageBannerResponse {
    dismissed: bool,
    action_clicked: bool,
    detail_expanded: bool,
}
```

### Visual Layout

```
+------------------------------------------------------------------+
| [Icon] Message text                            [Action] [X close] |
| v Show details (Level 2, if detail present)                       |
|   Detailed error information...                                   |
+------------------------------------------------------------------+
```

### Styling

| Type | Background | Icon | Border |
|---|---|---|---|
| Success | DashColors::SUCCESS at 15% opacity | Checkmark | DashColors::SUCCESS |
| Info | DashColors::INFO at 15% opacity | Info circle | DashColors::INFO |
| Warning | DashColors::WARNING at 15% opacity | Warning triangle | DashColors::WARNING |
| Error | DashColors::ERROR at 15% opacity | X circle | DashColors::ERROR |

### Auto-Dismiss

- Success and Info: 5 seconds
- Warning: 8 seconds
- Error: No auto-dismiss

---

## 10. ReceiveDialog

**Purpose**: Modal overlay showing a receive address with QR code.

### Props / Inputs

```rust
struct ReceiveDialog {
    // Data
    core_addresses: Vec<AddressInfo>,
    platform_addresses: Vec<AddressInfo>,
    developer_mode: bool,

    // UI state
    active_tab: ReceiveTab,
    selected_address_index: usize,
    show_address_selector: bool,  // Level 1+
}

enum ReceiveTab {
    Core,
    Platform,
}

struct AddressInfo {
    address: String,
    balance: Amount,
    derivation_path: Option<String>,
    is_used: bool,
}
```

### Response

```rust
struct ReceiveDialogResponse {
    closed: bool,
    address_copied: bool,
    new_address_requested: Option<ReceiveTab>,
}
```

### Tab Visibility

- Core tab: Always visible
- Platform tab: Visible only if `platform_addresses` is non-empty

### Level-Dependent Content

| Element | Level 0 | Level 1 | Level 2 |
|---|---|---|---|
| Tab bar | Yes | Yes | Yes |
| QR code | Yes | Yes | Yes |
| Address text | Yes | Yes | Yes |
| Copy Address button | Yes | Yes | Yes |
| New Address button | Yes | Yes | Yes |
| Address selector dropdown | No | Yes | Yes |
| Balance of selected address | No | Yes | Yes |
| Derivation path | No | No | Yes |

---

## 11. OverflowMenu

**Purpose**: "..." button in the balance header that provides secondary wallet actions.

### Props / Inputs

```rust
struct OverflowMenu {
    is_locked: bool,
    wallet_type: WalletType,
    open: bool,
}
```

### Menu Items

| Item | Condition |
|---|---|
| Rename | Always |
| Lock | Wallet is unlocked and has password |
| Unlock | Wallet is locked |
| Show Recovery Phrase | HD wallet only, requires unlock |
| Remove Wallet | Always (shows confirmation dialog) |

### Response

```rust
struct OverflowMenuResponse {
    action: Option<OverflowAction>,
}
```

---

## Component Composition Map

How components are composed on the wallet screen:

```
WalletsBalancesScreen
  |
  |-- WalletSelector (Zone 0, top panel)
  |
  |-- BalanceHeader (Zone 2)
  |     |-- OverflowMenu
  |
  |-- ActionBar (Zone 3)
  |
  |-- MessageBanner (between Zone 3 and Zone 4)
  |
  |-- Content Area (Zone 4, scrollable)
        |
        |-- TransactionHistorySection
        |     |-- CollapsibleSection wrapper
        |     |-- Transaction table (egui_extras::Table)
        |
        |-- AccountsSection
        |     |-- CollapsibleSection wrapper
        |     |-- Per-account CollapsibleSection
        |           |-- AddressTable
        |
        |-- AssetLocksSection (HD only)
        |     |-- CollapsibleSection wrapper
        |     |-- Asset lock table (egui_extras::Table)
        |
        |-- (SingleKey only) UTXOSection
              |-- CollapsibleSection wrapper
              |-- UTXO table with pagination

Modal overlays (rendered on top):
  |-- ReceiveDialog
  |-- WalletUnlockPopup
  |-- RenameDialog
  |-- RemoveWalletDialog
  |-- PrivateKeyDialog
  |-- FundPlatformAddressDialog
```

---

## egui Implementation Notes

### Immediate Mode Considerations

1. **No retained state in widgets**: All component state must be stored in the screen struct. Components are re-rendered every frame from this state.

2. **Lazy initialization**: Use `Option<Component>` with `get_or_insert_with()` as documented in `COMPONENT_DESIGN_PATTERN.md`. Components are created only when first needed.

3. **Response pattern**: Each component's `show()` method returns a response struct. The parent screen inspects the response and updates state accordingly.

4. **Layout**: Use `egui::TopBottomPanel` for the top panel (Zone 0), `egui::SidePanel` for the left panel (Zone 1), and `egui::CentralPanel` for the main content (Zones 2-4). Within the central panel, use `egui::ScrollArea` for the content area (Zone 4).

5. **Tables**: Use `egui_extras::TableBuilder` for address tables, transaction history, and asset lock tables. This is the existing pattern used in the codebase.

6. **Modals**: Use the existing `draw_modal_overlay()` and `modal_frame()` pattern from `dialogs.rs` for all modal overlays.

7. **Theme compliance**: All colors must come from `DashColors`. Support both light and dark modes by using the semantic color methods (`glass_white()`, `glass_blue()`, `glass_border()`) rather than hardcoded Color32 values.

### Data Flow

```
AppContext (wallets, balances, transactions)
    |
    v
WalletsBalancesScreen (holds component state, data references)
    |
    | update() called each frame
    v
Components::show(ui, data) -> Response
    |
    v
Screen inspects Response, dispatches AppAction if needed
    |
    v
AppAction::BackendTask -> tokio::spawn -> TaskResult -> display_task_result()
```

### Performance Considerations

1. **Avoid cloning large data**: Pass references to wallet data where possible. Clone only when needed for async tasks.
2. **Pagination**: Transaction history and UTXO lists use pagination to avoid rendering thousands of rows.
3. **Deferred rendering**: Collapsed sections skip rendering their content entirely (return early if `!expanded`).
4. **Table virtualization**: `egui_extras::TableBuilder` with `body()` handles virtual scrolling for large tables.
