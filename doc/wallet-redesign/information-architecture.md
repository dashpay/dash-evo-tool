# Wallet Screen Information Architecture

## Table of Contents

1. [Design Philosophy](#design-philosophy)
2. [Disclosure Model](#disclosure-model)
3. [Screen Hierarchy](#screen-hierarchy)
4. [Wallet Screen Layout Zones](#wallet-screen-layout-zones)
5. [Content Hierarchy per Zone](#content-hierarchy-per-zone)
6. [Navigation and Wallet Selection](#navigation-and-wallet-selection)
7. [Account Category Visibility Rules](#account-category-visibility-rules)
8. [Feature Gating by Mode](#feature-gating-by-mode)

---

## Design Philosophy

Three principles drive every layout decision:

1. **Balance-first**: The most important piece of information in any wallet is how much money the user has. This must be the first thing visible, rendered large and unambiguous.

2. **Progressive disclosure**: Complexity is layered. The default view satisfies Alex (everyday user). One click expands detail for Priya (power user). A settings toggle activates developer tools for Jordan (platform developer). No persona sees information intended for a more advanced persona unless they opt in.

3. **Consistent structure across wallet types**: HD Wallets and SingleKey Wallets share the same visual layout -- a balance header, action buttons, and a content area. The content area adapts to the wallet type, but the user never feels like they are on a "different screen."

---

## Disclosure Model

The UI uses a three-level progressive disclosure model. These are NOT tabs or modes the user toggles on a single screen. Instead:

- **Level 0 (Default)** is what every user sees. It covers Alex's needs entirely.
- **Level 1 (Expanded)** is reached by clicking expand/collapse controls within each section. Priya uses these to drill into address tables, account breakdowns, and asset lock management.
- **Level 2 (Developer Tools)** is activated via a setting in the Settings screen. Jordan enables this once and gets developer-specific features everywhere.

```
Level 0 (Default)
  Visible to: Alex, Priya, Jordan
  Content: Balance header, Send/Receive buttons, transaction history,
           wallet lock status

Level 1 (Expanded Detail)
  Visible to: Priya, Jordan (on demand, per-section)
  Content: Account category breakdown, address tables, asset lock
           management, granular refresh controls, UTXO detail,
           private key export, Platform address management

Level 2 (Developer Tools)
  Visible to: Jordan (after enabling in Settings)
  Content: Raw credit values alongside DASH amounts, refresh mode
           selector, bulk operations, faucet integration (Testnet),
           Devnet configuration, state transition context
```

### How Levels Interact

- Level 1 controls are always available in the UI (expand arrows, "Show Details" links) but sections start collapsed for Level 0 users. Once a user expands a section, the app remembers the preference per-session.
- Level 2 features appear only when the Developer Tools setting is enabled. They are additive: they do not replace Level 0 or Level 1 content; they augment it (e.g., showing "0.5000 DASH (500,000,000 credits)" instead of just "0.5000 DASH").

---

## Screen Hierarchy

The wallet area of the app consists of these screens:

```
Wallets (root screen, always accessible from left panel)
  |
  |-- Wallet Screen (WalletsBalancesScreen)
  |     Main wallet view. Shows balance, actions, content area.
  |     This is the redesign target.
  |
  |-- Send Screen (WalletSendScreen)
  |     Full-screen send flow. Replaces current dialog approach.
  |     Reached via: Send button on wallet screen.
  |     Returns to: Wallet screen on completion or cancel.
  |
  |-- Create Wallet Screen (AddNewWalletScreen)
  |     Guided wallet creation flow.
  |     Reached via: "Create Wallet" button (no-wallet state or wallet menu).
  |
  |-- Import Wallet Screen (ImportMnemonicScreen)
  |     Import via mnemonic or private key.
  |     Reached via: "Import Wallet" button.
  |
  |-- Create Asset Lock Screen (CreateAssetLockScreen)
  |     Asset lock creation flow.
  |     Reached via: "Create Asset Lock" in asset locks section (Level 1).
  |
  |-- Asset Lock Detail Screen
  |     Detail view for a single asset lock.
  |     Reached via: "View" button in asset lock table (Level 1).
```

### Modal Overlays (Not Separate Screens)

These remain as modal overlays on top of the wallet screen:

- **Receive dialog**: Shows address + QR code. Lightweight, no complex flow.
- **Wallet unlock popup**: Password entry for locked wallets. Appears on demand before operations requiring the private key.
- **Rename dialog**: Simple text input for wallet alias.
- **Remove wallet confirmation**: Warning dialog with consequences.
- **Private key view**: Shows WIF key for a selected address with security warning.
- **Fund Platform Address dialog**: Selector for funding a Platform address from an asset lock.

---

## Wallet Screen Layout Zones

The wallet screen is divided into five zones. Each zone has a fixed position in the layout. Content within each zone adapts to the wallet type, disclosure level, and wallet state.

```
+------------------------------------------------------------------+
|                        TOP PANEL (Zone 0)                        |
|  [App navigation: breadcrumb or screen title]                    |
+--------+---------------------------------------------------------+
|        |                                                         |
|  LEFT  |                  ZONE 2: BALANCE HEADER                 |
| PANEL  |  [Total balance] [Core/Platform breakdown] [Lock icon]  |
| (Zone  |                                                         |
|   1)   +---------------------------------------------------------+
|        |                                                         |
|  Nav   |                  ZONE 3: ACTION BAR                     |
|  items |  [Send] [Receive] [Refresh] [More actions...]           |
|        |                                                         |
|        +---------------------------------------------------------+
|        |                                                         |
|        |                  ZONE 4: CONTENT AREA                   |
|        |  [Transaction History]                                  |
|        |  [Accounts Section] (expandable)                        |
|        |  [Asset Locks Section] (expandable, HD only)            |
|        |  [Platform Addresses] (expandable, HD only)             |
|        |                                                         |
+--------+---------------------------------------------------------+
```

### Zone 0: Top Panel

- App-wide top bar with breadcrumb navigation
- **Wallet selector** lives here: a persistent dropdown showing the current wallet name, type badge (HD/SK), and balance
- Network indicator (Mainnet/Testnet/Devnet) as a colored label
- Connection status indicator

### Zone 1: Left Panel

- Navigation sections as defined in the existing left panel
- The "Wallets" section is highlighted when the wallet screen is active
- No wallet-specific content in this zone (wallet selector is in Zone 0)

### Zone 2: Balance Header

The most prominent visual element on the screen.

**Level 0 content (Default)**:
- Total wallet balance in large text: "1.2345 DASH"
- Wallet alias below or above the balance: "My Dash Wallet"
- Lock icon if the wallet is password-protected (locked or unlocked state)
- Wallet type badge: "HD Wallet" or "Imported Key"

**Level 1 content (Expanded, click to reveal)**:
- Balance breakdown: "Core: 1.0000 DASH | Platform: 0.2345 DASH"
- Per-account summary: expandable list of accounts with non-zero balances

**Level 2 content (Developer Tools enabled)**:
- Platform balance also shown in credits: "Platform: 0.2345 DASH (234,500,000 credits)"
- Unconfirmed balance indicator if non-zero

### Zone 3: Action Bar

A horizontal bar of primary action buttons.

**Always visible**:
- **Send** (primary action, prominent styling)
- **Receive** (primary action, prominent styling)
- **Refresh** (secondary action)

**Level 1 additions**:
- **Refresh mode selector** dropdown next to the Refresh button (replaces current dev-only dropdown)

**Level 2 additions**:
- **Get Test Dash** button (visible only on Testnet/Devnet)

**Contextual actions** (dropdown or overflow menu "..."):
- Rename wallet
- Remove wallet
- Lock / Unlock wallet
- Show recovery phrase (HD wallets only)

### Zone 4: Content Area

Scrollable area below the action bar. Contains collapsible sections in vertical order.

**Section order and default visibility:**

| Section | Level 0 (Default) | Level 1 (Expanded) | Level 2 (Developer) |
|---|---|---|---|
| Transaction History | Visible, last 10 | Visible, all + filter | Visible + export CSV |
| Accounts & Addresses | Hidden (collapsed) | Expandable per-account | + filter by activity |
| Asset Locks | Hidden | Visible (HD only) | + bulk create |
| Platform Addresses | Hidden | Visible (HD only) | + raw credit values |

---

## Content Hierarchy per Zone

### Transaction History Section

**Default (Level 0)**:
```
Recent Transactions
+------+----------+-----------+------------+--------+
| Date | Type     | Amount    | Status     | TxID   |
+------+----------+-----------+------------+--------+
| 2/10 | Received | +0.5000   | Confirmed  | abc1.. |
| 2/8  | Sent     | -0.2500   | Confirmed  | def2.. |
| 2/5  | Received | +1.0000   | Pending    | ghi3.. |
+------+----------+-----------+------------+--------+
[Show All Transactions]
```

- Shows last 10 transactions by default
- Type column uses directional arrows or icons: incoming (green arrow down), outgoing (red arrow up), internal (grey circular arrows)
- Status uses visual indicators: checkmark for confirmed, spinner for pending
- TxID is truncated; clicking copies to clipboard
- "Show All Transactions" expands to full history with pagination

**Expanded (Level 1)** adds:
- Fee column
- Block height / confirmation count
- Sortable columns
- Filter by type (Sent / Received / Internal / All)

**Developer (Level 2)** adds:
- Export to CSV
- Filter by date range

### Accounts and Addresses Section

**Default (Level 0)**: Section header is visible but content is collapsed. Shows a one-line summary: "4 accounts, 12 addresses."

**Expanded (Level 1)**: Clicking the section header reveals the account category list. Each account category is a collapsible sub-section:

```
v Main Account                    1.0000 DASH
    [Address Table: 5 addresses with balances]

v Platform Account                0.2345 DASH
    [Address Table: 2 addresses with credit balances]

> CoinJoin                        0.0000 DASH (collapsed)
> Identity Registration           (keys only)
```

Account categories with zero balance and no activity are collapsed and greyed out. "Keys only" categories (Identity Registration, Identity System, etc.) show a "(keys only)" label instead of a balance.

**Developer (Level 2)** adds:
- "Filter: Has Activity" toggle to hide empty accounts
- Full derivation path column in address tables
- Copy derivation path button for scripting

### Asset Locks Section

**Default (Level 0)**: Hidden entirely. Alex does not need to know about asset locks.

**Expanded (Level 1)**: Shows asset locks table:

```
Asset Locks                              [Create] [Search Unused]
+----------------+------------+--------+--------+---------+
| Transaction ID | Amount     | IS Lock| Usable | Actions |
+----------------+------------+--------+--------+---------+
| abc123...      | 0.5000 DASH| Yes    | Yes    | [View] [Fund] |
+----------------+------------+--------+--------+---------+
```

**Developer (Level 2)** adds:
- Bulk create option
- Amount shown in both DASH and credits

### Platform Addresses Section

**Default (Level 0)**: Hidden. Platform addresses are implicitly managed when Alex interacts with identities or DPNS through other screens.

**Expanded (Level 1)**: Shows Platform addresses with credit balances, fund/transfer/withdraw actions.

**Developer (Level 2)** adds:
- Raw credit values
- Batch transfer capability

---

## Navigation and Wallet Selection

### Wallet Selector Design

The wallet selector is a **dropdown in the top panel** (Zone 0). It is always visible regardless of which screen is active, since other screens (Identities, Tokens) also depend on the selected wallet.

**Selector display format**:
```
[HD] My Dash Wallet              1.2345 DASH  v
```

Components:
- **Type badge**: Small pill label "HD" or "SK" (SingleKey)
- **Wallet alias**: Primary text, left-aligned
- **Balance**: Secondary text, right-aligned
- **Lock icon**: Shown after alias if wallet is locked (padlock icon)
- **Dropdown arrow**: Opens the wallet list

**Dropdown list format**:
```
+----------------------------------------------------+
| [HD] My Dash Wallet          1.2345 DASH           |
| [HD] Business Wallet         5.0000 DASH     [L]   |
| [SK] Cold Storage Key        0.8000 DASH           |
|----------------------------------------------------|
| + Create Wallet                                    |
| + Import Wallet                                    |
+----------------------------------------------------+
```

- Each wallet entry shows: type badge, alias, balance, lock indicator
- Bottom section has "Create Wallet" and "Import Wallet" actions
- Currently selected wallet is highlighted

### No-Wallet State

When no wallets are loaded, the wallet screen shows a centered empty state:

```
+--------------------------------------------------+
|                                                  |
|              [Wallet Icon]                       |
|                                                  |
|         No Wallets Loaded                        |
|                                                  |
|    Create a new wallet to start holding and      |
|    transacting Dash, or import an existing       |
|    wallet using your recovery phrase.            |
|                                                  |
|    [Create Wallet]    [Import Wallet]            |
|                                                  |
+--------------------------------------------------+
```

---

## Account Category Visibility Rules

The 13 account categories from `AccountCategory` are mapped to display tiers:

| Category | User-Facing Label | Level 0 | Level 1 | Level 2 | Notes |
|---|---|---|---|---|---|
| BIP44 | Main Account | Summary only | Full table | + derivation paths | Primary spending account |
| BIP32 | Legacy Account | Hidden | Shown if balance > 0 | Always shown | Old-format addresses |
| CoinJoin | Private Send | Hidden | Shown if balance > 0 | Always shown | Mixing funds |
| PlatformPayment | Platform Account | Summary only | Full table | + raw credits | DIP-17 addresses |
| IdentityRegistration | Identity Keys | Hidden | Shown (keys only) | Full table | No balance display |
| IdentitySystem | System Keys | Hidden | Shown (keys only) | Full table | No balance display |
| IdentityTopup | Top-up Keys | Hidden | Shown (keys only) | Full table | No balance display |
| IdentityInvitation | Invitation Keys | Hidden | Shown (keys only) | Full table | No balance display |
| ProviderVoting | Masternode Voting | Hidden | Shown if used | Full table | MN operators |
| ProviderOwner | Masternode Owner | Hidden | Shown if used | Full table | MN operators |
| ProviderOperator | Masternode Operator | Hidden | Shown if used | Full table | MN operators |
| ProviderPlatform | Masternode Platform | Hidden | Shown if used | Full table | MN operators |
| Other | Other | Hidden | Shown if balance > 0 | Always shown | Catch-all |

**Key rules**:
1. At Level 0, Alex sees only the total balance. No account categories are listed.
2. At Level 1, only categories with non-zero balance or active usage are shown expanded. The rest are collapsed or hidden.
3. At Level 2, all categories are available with a filter toggle to show/hide empty ones.
4. "Keys only" categories never show a balance amount. They show "Keys" as their type indicator.

---

## Feature Gating by Mode

This table maps every wallet screen feature to its disclosure level and the setting that controls it:

| Feature | Level 0 | Level 1 | Level 2 | Gating Mechanism |
|---|---|---|---|---|
| Total balance display | Yes | Yes | Yes | Always visible |
| Core/Platform breakdown | No | Yes | Yes | Click expand on balance header |
| Raw credit values | No | No | Yes | Developer Tools setting |
| Send button | Yes | Yes | Yes | Always visible |
| Receive button | Yes | Yes | Yes | Always visible |
| Refresh button | Yes | Yes | Yes | Always visible |
| Refresh mode selector | No | Yes | Yes | Expand arrow next to Refresh |
| Transaction history (recent) | Yes | Yes | Yes | Always visible |
| Transaction history (full) | No | Yes | Yes | "Show All" link |
| Transaction export (CSV) | No | No | Yes | Developer Tools setting |
| Account categories section | Collapsed | Expandable | Expandable + filter | Section expand/collapse |
| Address table | No | Per-account expand | Per-account expand | Nested expand within account |
| Derivation path column | No | No | Yes | Developer Tools setting |
| View Key button | No | Yes | Yes | In address table (Level 1) |
| Asset locks section | Hidden | Visible | Visible + bulk ops | Section visibility |
| Platform addresses section | Hidden | Visible | Visible + batch ops | Section visibility |
| Wallet rename | Overflow menu | Overflow menu | Overflow menu | Same for all |
| Wallet remove | Overflow menu | Overflow menu | Overflow menu | Same for all |
| Lock/Unlock | Overflow menu | Overflow menu | Overflow menu | Same for all |
| Get Test Dash (Faucet) | No | No | Yes (Testnet only) | Developer Tools + network |

---

## SingleKey Wallet Adaptations

SingleKey Wallets use the same layout zones but with simplified content:

- **Zone 2 (Balance Header)**: Shows balance for the single address. No Core/Platform breakdown (SingleKey wallets are Core-only).
- **Zone 3 (Action Bar)**: Send, Receive, Refresh only. No asset lock or Platform actions.
- **Zone 4 (Content Area)**:
  - Transaction History: Same as HD wallet
  - Address Detail: Single address shown inline (not a table). Shows: address, balance, UTXO count.
  - UTXO list: Expandable section showing individual UTXOs with amount and confirmations (Level 1).
  - No Account Categories section.
  - No Asset Locks section.
  - No Platform Addresses section.
