# Crypto Wallet UX Patterns and Competitor Analysis

Research document for the Dash Evo Tool wallet screen redesign.

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Dash-Specific Wallets](#1-dash-specific-wallets)
3. [Popular Desktop Wallets](#2-popular-desktop-wallets)
4. [Browser/Extension Wallets](#3-browserextension-wallets)
5. [Industry UX Best Practices](#4-industry-ux-best-practices)
6. [Cross-Wallet Pattern Analysis](#5-cross-wallet-pattern-analysis)
7. [Current Dash Evo Tool State](#6-current-dash-evo-tool-wallet-state)
8. [Recommendations for Dash Evo Tool](#7-recommendations-for-dash-evo-tool)
9. [Sources](#sources)

---

## Executive Summary

This document analyzes UX patterns across 10+ cryptocurrency wallets (desktop, mobile, and browser) to inform the Dash Evo Tool wallet screen redesign. The key findings are:

- **Progressive disclosure** is the dominant pattern: simple views for casual users, expandable detail for power users.
- **Unified balance view** with breakdown on demand is the industry standard. No wallet shows raw address tables by default.
- **Wallet selection** is a global control (top-of-screen selector), not per-screen navigation.
- **Send/receive flows** are modal or full-screen overlays, not separate screens for simple transactions.
- **Terminology matters**: wallets that use plain language ("Main Account" vs "m/44'/5'/0'") see higher adoption and fewer support tickets.
- **Status feedback** (pending/confirmed/failed) is critical and a top complaint area when poorly implemented.
- **Locked/special-purpose funds** should be visible but clearly separated from spendable balance.

---

## 1. Dash-Specific Wallets

### 1.1 Dash Core Wallet (Qt Desktop)

**Overview**: Full-node wallet that downloads the complete blockchain. Designed for power users and masternode operators.

**Wallet Management**:
- Single wallet instance per application (no multi-wallet selector in the GUI)
- Wallet encryption with password lock
- Manual backup via menu (File > Backup Wallet)

**Balance Display**:
- Shows: Available, Pending, Immature (coinbase), and Total
- CoinJoin balance shown separately when CoinJoin is active
- Balance depends on blockchain sync status; UI distinguishes synced vs syncing states

**Key Features**:
- CoinJoin mixing with visual progress indicator
- InstantSend toggle on transactions
- Coin control (advanced, hidden behind menu toggle "Settings > Options > Wallet > Enable coin control")
- Address book for contacts
- Debug/RPC console for developers
- Masternode commands and governance voting

**UX Strengths**:
- Familiar Bitcoin Core-derived interface (comfortable for Bitcoin users)
- Comprehensive coin control for advanced users
- Clear separation of CoinJoin funds

**UX Weaknesses**:
- Dated Qt interface, not visually modern
- No progressive disclosure; all features visible at once or buried in menus
- No multi-wallet management from the UI
- Long initial sync requirement with no useful wallet features until complete
- Address management is a flat list (no account grouping)

### 1.2 Dash Electrum

**Overview**: Lightweight SPV wallet using external servers for blockchain indexing. Based on Electrum (Bitcoin).

**Wallet Management**:
- Supports multiple wallet files (File > Open Wallet)
- 12-word mnemonic (128-bit seed) for recovery
- Can import/export private keys

**Balance Display**:
- Balance shown in main toolbar: Confirmed and Unconfirmed
- Status bar shows network connection info

**Interface Structure**:
- Tab-based: History, Send, Receive, Addresses, Coins, Contacts, Console
- Addresses tab shows all derived addresses with balance, label, and type (receiving/change)
- Coins tab shows individual UTXOs (opt-in via View > Show Coins)

**Key Features**:
- Coin control via Coins tab (select specific UTXOs for spending, freeze coins)
- Raw transaction creation and signing from the GUI
- CoinJoin support
- InstantSend support
- Masternode features
- Label/tagging system for addresses and transactions

**UX Strengths**:
- Near-instant startup (SPV, no full sync)
- Tab-based progressive disclosure (basic users stay on History/Send/Receive)
- Explicit coin control without hiding it too deeply
- Address labeling for organization

**UX Weaknesses**:
- Dated interface (Electrum look and feel from ~2012)
- Technical terminology throughout ("UTXO", "derivation path", "nLockTime")
- No visual differentiation of address types beyond columns
- Coin freezing concept not explained to users

### 1.3 DashPay Mobile Wallet (Android)

**Overview**: Mobile wallet with Dash Platform integration. First wallet to support DPNS usernames and social payments.

**Wallet Management**:
- Single wallet per app instance
- 12-word or 24-word passphrase recovery
- Customizable shortcut bar (4 quick actions)

**Key Features**:
- **Username registration**: Register a DPNS username directly
- **Contact-based payments**: Send to usernames instead of addresses; contacts exchange encrypted xpubs
- **Payment history per contact**: Track payments between specific contacts
- **Privacy**: Contact xpubs encrypted so only the two parties can derive addresses
- CoinJoin support
- Merchant map
- Staking support

**UX Strengths**:
- Social payment paradigm (send to "alice" not "XdK7f...") is industry-leading
- Contact-based relationship model hides address complexity entirely
- Clean mobile-first design
- Real-world spending integration (merchant map, DashDirect)

**UX Weaknesses**:
- Android-only (iOS in development)
- Username registration UX could better educate about costs and irreversibility
- No desktop equivalent of the social payment features

**Relevance to Dash Evo Tool**: DashPay's social payment model and contact system demonstrate the target UX for Platform features. Dash Evo Tool should consider how to surface DPNS/DashPay contacts alongside traditional address management for users who have registered identities.

---

## 2. Popular Desktop Wallets

### 2.1 Exodus

**Overview**: Multi-asset desktop wallet known for its polished visual design. Supports 260+ cryptocurrencies.

**Wallet Management**:
- Global wallet with portfolio view as the home screen
- Up to 3 portfolios per wallet (for organizing by strategy)
- First-run protection flow: password and auto-lock setup

**Balance Display**:
- **Portfolio dashboard**: Aggregated total value, visual pie chart breakdown by asset, percentage allocation
- **Per-asset view**: Balance, fiat equivalent, small performance graph, send/receive/swap actions
- **Multi-chain aggregation**: Assets on multiple networks show combined balance, with network breakdown on drill-down
- Timeline views: 24h, 1W, 1M, 3M, 1Y, All

**Interface Structure**:
- Left sidebar: Portfolio, Wallet (asset list), Exchange, Staking, Apps
- Central area: Context-dependent (portfolio charts, asset details, transaction flow)
- Per-asset page: Balance, price chart, transaction history, send/receive buttons

**Key Features**:
- Built-in exchange (swap between assets)
- Staking for supported assets
- Personalized themes and visual customization
- Export options for tax reporting
- Hardware wallet (Trezor) integration

**UX Strengths**:
- **Best-in-class visual design**: Consistently praised as the most polished crypto wallet UI
- Instant portfolio overview with actionable breakdown
- Progressive disclosure: simple portfolio view, drill into asset details, advanced settings behind menus
- Fiat currency equivalents shown everywhere
- First-run flow guides users through security setup without overwhelming

**UX Weaknesses**:
- Closed source (security concern for power users)
- No coin control / UTXO management
- Limited advanced features (no custom derivation paths, no raw transactions)
- Exchange rates can be unfavorable compared to dedicated exchanges
- No multi-device sync (desktop instances are independent)

**Relevance to Dash Evo Tool**: Exodus demonstrates that a portfolio-first view with drill-down into asset/account details is the preferred UX pattern. The visual hierarchy (total balance > asset breakdown > individual transactions) is a proven information architecture.

### 2.2 Ledger Live (Desktop)

**Overview**: Companion app for Ledger hardware wallets. Multi-asset portfolio manager.

**Wallet Management**:
- Each blockchain gets one or more "accounts" (e.g., Bitcoin Account 1, Bitcoin Account 2)
- Portfolio dashboard aggregates all accounts into a single net-worth view
- Adding accounts is a guided flow: select asset > connect device > derive accounts > name account

**Balance Display**:
- **Portfolio level**: Total value, performance chart, recent activity across all accounts
- **Account level**: Balance, transaction history, receive address, operations history
- Aggregation across accounts with ability to drill down

**Interface Structure**:
- Left sidebar: Portfolio, Accounts (list), Send, Receive, Manager, Discover
- Top bar: Settings, notifications
- Central area: Content for selected section

**Design Principles** (from Ledger developer guidelines):
- **Security**: Cryptographic signing only on hardware device; Live is a "coordinator and visualizer"
- **Privacy**: Minimal network calls, default-off telemetry
- **Usability**: Intuitive onboarding, clear transaction details, helpful confirmations

**Transaction Flow**:
1. Prepare transaction in desktop app (recipient, amount, fee selection)
2. Review on hardware device screen
3. Confirm on hardware device
4. App shows pending > confirmed status

**UX Strengths**:
- Clean separation of concerns (prepare in app, sign on device)
- Multi-account management with clear naming
- Portfolio aggregation with drill-down
- Standardized UI components across blockchain integrations
- Good onboarding for new users (guided account creation)

**UX Weaknesses**:
- Heavy reliance on hardware device (cannot do anything without it connected)
- Account creation can be confusing (multiple derivation paths for same asset)
- Sync can be slow for chains with many transactions
- Limited advanced features (no coin control in standard mode)

**Relevance to Dash Evo Tool**: Ledger Live's account model (named accounts with portfolio aggregation) is relevant for Dash Evo Tool's multi-account HD wallet structure. The guided account addition flow could inform how Dash Evo Tool handles adding new wallets.

### 2.3 Wasabi Wallet (Desktop, Bitcoin-only)

**Overview**: Privacy-focused Bitcoin desktop wallet built with Avalonia (cross-platform .NET). Emphasizes CoinJoin privacy.

**Wallet Management**:
- Multiple wallets supported (File > Open Wallet)
- Each wallet is a separate file with its own seed
- Wallet list with labels and balance preview

**Balance Display**:
- Total balance with privacy score indicator
- Balance breakdown: Private, Semi-private, Non-private (based on CoinJoin anonymity set)
- Anonymity score ranges from 2-300 (default target: 50)

**Interface Structure (v2.0)**:
- Sidebar: Wallet list, settings
- Central: Balance, transaction list, CoinJoin progress
- Privacy presets: Privacy, Speed, Cost (quick selection of CoinJoin strategy)

**Key Features**:
- **Automated CoinJoin**: Runs in background by default; progress bar shows mixing status
- **Privacy score per UTXO**: Visual indicator of anonymity level
- **Coin control**: Full UTXO management with privacy labels
- Tor routing by default
- Client-side block filtering (BIP 158) for privacy

**UX Strengths**:
- **Privacy made accessible**: Automated CoinJoin with simple presets (Privacy/Speed/Cost)
- Privacy score is a simple, understandable metric for a complex concept
- Modern UI (v2.0 complete rewrite)
- Power user features (coin control, UTXO management) without cluttering basic view

**UX Weaknesses**:
- Bitcoin-only (no multi-asset)
- CoinJoin can be confusing for new users despite automation
- Privacy terminology still requires learning curve
- Tor can slow initial sync

**Relevance to Dash Evo Tool**: Wasabi's approach to CoinJoin UX is directly relevant since Dash also has CoinJoin. The privacy score concept and the preset-based configuration (Privacy/Speed/Cost) could be adapted for Dash's CoinJoin features. The automated background approach is preferable to manual mixing.

### 2.4 Electrum (Bitcoin)

**Overview**: The original lightweight Bitcoin wallet. Established in 2011, tab-based interface.

**Wallet Management**:
- Multiple wallet files (File > Open, File > New)
- Watch-only wallets, multi-sig wallets, hardware wallet integration
- Import from seed, keys, or addresses

**Balance Display**:
- Toolbar shows: Confirmed, Unconfirmed, Total
- Per-address balance in Addresses tab
- Per-UTXO balance in Coins tab

**Coin Control Pattern**:
- Addresses tab: View > Show Addresses (opt-in)
- Coins tab: View > Show Coins (opt-in)
- Right-click on coin > "Add to coin control" to restrict spending
- "Coin control active" indicator in status bar
- Freeze coins to exclude from automatic selection

**UX Strengths**:
- Established, trusted interface
- Powerful coin control and UTXO management
- Tab-based organization keeps features discoverable
- Label system for addresses and transactions

**UX Weaknesses**:
- Very dated visual design
- No portfolio or fiat value display
- Technical terminology without explanation
- No visual hierarchy (flat tab structure)

**Relevance to Dash Evo Tool**: Electrum's tab-based approach with opt-in advanced tabs (Coins, Addresses) is a proven pattern for progressive disclosure in desktop wallets. The coin control workflow (select > mark > spend) is the established standard.

---

## 3. Browser/Extension Wallets

### 3.1 MetaMask

**Overview**: Most popular browser extension wallet for EVM chains. Used by 30M+ monthly active users.

**Account Management (2025 redesign)**:
- Account selector at top of screen (always visible, shows avatar + name)
- Click to see all accounts with balances
- "Recently used accounts" section for quick switching
- Search functionality for users with many accounts
- Support for multiple Secret Recovery Phrases (SRPs) within one instance
- Profile Sync across devices (keeps names and settings)

**Balance Display (post-redesign)**:
- **Unified multi-chain view**: Single view showing all assets across networks (Ethereum, Polygon, Solana, Bitcoin, etc.)
- Aggregated balance with per-chain breakdown on demand
- Token list with balances and fiat equivalents
- NFTs in separate tab

**Interface Structure**:
- Top bar: Account selector (avatar + name), network indicator
- Main area: Assets tab, NFTs tab, Activity tab
- Bottom actions: Send, Receive, Swap, Bridge, Buy

**Key UX Improvements (2025)**:
- Account avatar placed next to account name (clear visual identity)
- Network selection separated from global menu
- Multi-chain asset view eliminates need to switch networks to see balances
- Human-readable signing messages (shows what will happen, not raw contract data)
- Smart accounts support

**UX Strengths**:
- **Unified multi-chain view** eliminates confusion about which network user is on
- Clear account identity (avatar + name always visible)
- Activity tab shows transaction status with clear pending/confirmed/failed states
- Extension popup format is compact and task-focused

**UX Weaknesses**:
- Account switching still problematic with many accounts
- Gas fee display can confuse beginners
- Extension popup is small for complex operations
- Onboarding (seed phrase backup) is still intimidating

**Relevance to Dash Evo Tool**: MetaMask's evolution toward unified multi-chain views and prominent account identity mirrors the challenge Dash Evo Tool faces with multiple account categories. The account selector pattern (top-of-screen, always visible, with avatar and name) is directly applicable.

### 3.2 Trust Wallet

**Overview**: Multi-chain self-custody wallet (mobile + browser extension). Supports millions of assets across 100+ blockchains.

**Account Management**:
- Multiple accounts under a single recovery phrase (up to 15 wallets)
- Instant switching between accounts
- Organization by portfolio (personal vs business)
- Settings > Manage Wallets for wallet management

**Balance Display**:
- Total portfolio value in chosen fiat currency
- Per-asset balance with price change indicators
- Auto-detection of supported tokens
- Custom token addition (RPC URL, contract address)

**Interface Structure**:
- Tab bar: Wallet, Discover, Send, Swap, Settings
- Wallet tab: Portfolio value, token list with balances
- Tiered interface: simple view by default, advanced features progressively revealed

**UX Strengths**:
- Broad asset support with auto-detection
- Simple tiered interface design
- Easy wallet-to-wallet switching
- Custom network/token addition without leaving the app

**UX Weaknesses**:
- Extremely poor customer support (rated 1.2/5 on review platforms)
- Reports of missing funds and unauthorized transfers
- High swap fees
- Token display can be overwhelming with auto-detected assets

**Relevance to Dash Evo Tool**: Trust Wallet's tiered interface pattern (simple by default, progressable) and multi-wallet management (up to 15, instant switching) directly inform how Dash Evo Tool should handle its HD wallet + SingleKeyWallet collection.

---

## 4. Industry UX Best Practices

### 4.1 Progressive Disclosure (Top Priority Pattern)

Progressive disclosure is the most-cited UX principle in crypto wallet design literature:

**Implementation layers**:
1. **Level 0 (Default)**: Total balance, recent transactions, send/receive buttons
2. **Level 1 (One click)**: Per-account/asset breakdown, transaction details
3. **Level 2 (Opt-in)**: Address management, coin control, derivation paths
4. **Level 3 (Developer/Expert)**: Raw transactions, RPC console, UTXO details

**Do's**:
- Let users complete simple actions (view balance, send) before surfacing advanced features
- Use contextual tooltips and inline help instead of documentation links
- Show fiat equivalents alongside crypto amounts everywhere
- Break complex operations into guided sequential steps

**Don'ts**:
- Show all features at once on the main screen
- Use technical jargon without explanation
- Require advanced knowledge for basic operations
- Force users through educational content before they can act

### 4.2 Balance Display Best Practices

From cross-wallet analysis, the standard balance hierarchy is:

```
Total Balance (fiat equivalent)
  |-- Available / Spendable
  |-- Pending (incoming, unconfirmed)
  |-- Locked (CoinJoin, asset locks, time-locked)
  |-- Staked / Platform Credits
```

**Key patterns**:
- Show fiat equivalent of total balance prominently (largest text on screen)
- Break down into Available vs non-Available with clear labels
- Pending transactions shown with status indicator (spinner, progress bar)
- Locked funds visible but clearly marked as non-spendable
- Use color coding: green for available, yellow/amber for pending, grey for locked

### 4.3 Transaction Status Communication

Transaction status is the #1 complaint area across all wallets:

**Required states**:
- **Preparing**: User is building transaction (pre-broadcast)
- **Broadcasting**: Transaction sent to network
- **Pending**: Awaiting confirmations (show confirmation count)
- **Confirmed**: Transaction complete
- **Failed**: Transaction rejected (with clear reason and suggested action)

**Best practices**:
- Show transaction progress on home screen (not buried in transaction list)
- Provide confirmation count for pending transactions
- Explain failures in plain language with actionable suggestions
- Never show a transaction as "sent" if it might fail

### 4.4 Terminology Conventions

Industry-standard user-facing terminology (avoid jargon):

| Technical Term | User-Facing Term |
|---|---|
| UTXO | (hidden from users; show as "coin" or "output" only in advanced mode) |
| Derivation path | Account type / Account |
| Mnemonic / Seed phrase | Recovery phrase / Recovery words |
| Private key | (hidden; refer to "wallet backup" or "recovery phrase") |
| BIP44/32 | Main Account / Legacy Account |
| InstantSend | Instant payment / Instant confirmation |
| CoinJoin | Privacy mixing / Private send |
| Asset Lock | Locked funds / Platform deposit |
| Credits | Platform balance / Platform credits |
| nLockTime | Time lock / Lock until [date] |

### 4.5 Eight Design Principles for Blockchain UX

Synthesized from industry research:

1. **Prioritize clarity and education**: Use accessible language. "Recovery phrase" not "mnemonic". Inline tooltips. Progressive disclosure.
2. **Design for trust and security**: Emphasize irreversible actions with warnings and multi-step confirmations. Show transaction progress. Use visual trust signals (padlock, checkmarks).
3. **Simplify wallet connection flows**: One-click where possible. Preview transaction summaries (amounts, fees, recipients) before confirmation.
4. **Make transparency a feature**: Link to blockchain explorers. Show transaction hashes, confirmations, timestamps.
5. **Leverage smart contracts, hide complexity**: Human-friendly labels. Never display raw contract data or cryptographic material by default.
6. **Design for multiple user personas**: Beginner/expert mode toggle. Simple and advanced views. Guided tours for newcomers.
7. **Plan for errors, gas failures, and edge cases**: Explain failures in plain language. Suggest fixes. Show network health indicators.
8. **Preview before confirming**: Pre-transaction summary showing what will be sent, received, fees, and fiat equivalents.

### 4.6 Security UX (Visual Cues and Risk Communication)

- **Color-coded risk indicators**: Green (safe/confirmed), Yellow (caution/pending), Red (high-risk/failed)
- **Pre-transaction summaries**: Break down what the transaction does before signing
- **Proactive security nudges**: Remind about backups, warn about large amounts, flag unusual addresses
- **Scam warnings**: Flag unknown addresses, suspicious contracts, high-value transfers to new recipients

---

## 5. Cross-Wallet Pattern Analysis

### 5.1 Wallet Selection and Management

| Wallet | Selection Pattern | Multi-Wallet | Location |
|---|---|---|---|
| Dash Core | None (single wallet) | No | N/A |
| Dash Electrum | File > Open Wallet | Yes (file-based) | Menu |
| Exodus | Portfolio selector | Up to 3 portfolios | Top area |
| Ledger Live | Account list in sidebar | Yes (per-asset accounts) | Left sidebar |
| MetaMask | Top-of-screen selector | Yes (multiple SRPs) | Top bar, always visible |
| Trust Wallet | Settings > Manage Wallets | Up to 15 wallets | Settings |
| Wasabi | Sidebar wallet list | Yes (file-based) | Left sidebar |

**Consensus pattern**: Global selector (sidebar or top bar), always visible, showing current wallet/account name.

### 5.2 Balance Display Patterns

| Wallet | Primary Balance | Breakdown | Fiat Equivalent |
|---|---|---|---|
| Dash Core | Available + Pending | Available / Pending / Immature | No |
| Exodus | Portfolio total (fiat) | Per-asset with charts | Yes (primary) |
| Ledger Live | Portfolio total (fiat) | Per-account | Yes (primary) |
| MetaMask | Total (fiat) | Per-token, per-network | Yes |
| Trust Wallet | Portfolio total (fiat) | Per-token | Yes |
| Wasabi | Total (BTC) | Private / Semi-private / Non-private | No |

**Consensus pattern**: Fiat-denominated total balance as the largest element, with crypto breakdown on drill-down.

### 5.3 Send/Receive Flow Patterns

| Wallet | Send Flow | Receive Flow | Confirmation |
|---|---|---|---|
| Dash Core | Send tab (always visible) | Receive tab | Password dialog |
| Exodus | Per-asset Send button | Per-asset Receive button | Preview + confirm |
| Ledger Live | Global Send in sidebar | Global Receive in sidebar | Hardware device confirm |
| MetaMask | Bottom action bar Send | Bottom action bar Receive | Preview + confirm |
| Wasabi | Send tab | Receive tab | Password + preview |

**Consensus pattern**: Send/receive accessible from both global navigation AND per-account/asset context. Multi-step flow: Enter details > Preview summary > Confirm.

### 5.4 How Wallets Handle Locked/Special-Purpose Funds

| Wallet | Locked Fund Types | Display Pattern |
|---|---|---|
| Dash Core | CoinJoin denominated funds | Separate "CoinJoin Balance" field |
| Wasabi | CoinJoin (by privacy score) | Color-coded privacy levels |
| Ledger Live | Staked assets | Separate "Staking" section per asset |
| MetaMask | Staked ETH | Shown in activity with "Staking" label |

**Consensus pattern**: Locked funds shown as a separate line item or section, clearly labeled with reason for lock, not mixed into "available" balance.

### 5.5 Developer/Advanced Mode Patterns

| Wallet | Mode Toggle | Advanced Features Revealed |
|---|---|---|
| Dash Core | Settings > Options | Coin control, debug console, RPC |
| Electrum | View menu toggles | Coins tab, Addresses tab, Console tab |
| Exodus | None (limited features) | N/A |
| Wasabi | Always available via sidebar | Coin control, UTXO labels |
| Trust Wallet | Settings | Custom networks, manual token add |

**Consensus pattern**: Advanced features toggled via settings or view menu. When enabled, appear as additional tabs/sections, not replacing the default view.

---

## 6. Current Dash Evo Tool Wallet State

Based on analysis of the current codebase (`/home/ubuntu/git/dash-evo-tool/src/ui/wallets/`):

### 6.1 Current Structure

The wallet screen (`WalletsBalancesScreen`) currently:

- Uses a **combo box** for wallet selection (`selected_wallet` / `selected_single_key_wallet`)
- Shows **account categories** via `AccountCategory` enum with 13 types:
  - User-visible: Main Account (BIP44), Legacy BIP32, CoinJoin, Platform Payment
  - Key-only (no balance): Identity Registration/System/Top-up/Invitation, Provider Voting/Owner/Operator/Platform
- Displays address tables with sorting (column-based sort by address, balance, path)
- Has **developer-only features**: RefreshMode dropdown with 6 modes (All, Core Only, Platform Full, etc.)
- Supports **dialogs** for: Send, Receive, Fund Platform Address, Private Key view
- Handles **wallet unlock** (password popup) before operations
- Shows **asset locks** in a separate sub-view

### 6.2 Current Pain Points (Inferred from Code)

1. **Information overload**: 13 account categories shown simultaneously, many holding zero balance and serving as key-only
2. **Flat address table**: All addresses shown in a table regardless of relevance
3. **Developer features mixed with user features**: RefreshMode dropdown and raw derivation path display visible to all users
4. **No balance summary/dashboard**: Jumps directly to address-level detail
5. **Single key wallets** handled as a completely separate code path (`single_key_view.rs`)
6. **No fiat equivalent** display for balances
7. **No transaction history** on the main wallet screen (addresses only)
8. **Dialog-based send/receive** rather than guided flow with preview

---

## 7. Recommendations for Dash Evo Tool

Based on the competitor analysis and UX best practices, here are prioritized recommendations:

### 7.1 High Priority (Core UX Improvements)

**R1: Implement a Balance Dashboard as the Default View**
- Show total wallet balance prominently (large text) with fiat equivalent
- Break down into: Available, Locked (asset locks), Platform Credits, Pending
- Use Exodus-style visual hierarchy: total > breakdown > per-account
- Hide zero-balance key-only accounts from this view entirely

**R2: Simplify Account Categories for Regular Users**
- Default view shows only: Main Account, Platform Account, and any with non-zero balance
- Developer/power-user mode reveals all 13 categories
- Rename categories to user-friendly labels (already partially done: "Main Account" for BIP44)
- Add inline tooltips explaining each category on hover

**R3: Global Wallet Selector (Top Bar or Sidebar)**
- Replace the combo box with a persistent wallet selector (always visible)
- Show wallet name/alias + total balance in the selector
- Support quick switching between HD wallets and SingleKeyWallets
- Show visual indicator for locked/encrypted wallets

**R4: Redesign Send/Receive as Guided Multi-Step Flows**
- Replace dialog-based approach with full-screen or panel-based flow
- Steps: Enter details > Select funding source > Preview summary (with fee breakdown and fiat equivalent) > Confirm
- Show clear success/failure status after broadcast
- For receive: Generate address > Show QR code > Copy button > Link to block explorer

### 7.2 Medium Priority (Progressive Disclosure)

**R5: Implement User Mode Toggle**
- Three-level progressive disclosure: Level 0 (default), Level 1 (expanded detail), Level 2 (Developer Tools, opt-in via settings)
- Level 0: Balance dashboard, simplified send/receive, recent transactions
- Level 1: Address tables, derivation paths, refresh mode selector (expand/collapse per section)
- Level 2: Coin control, raw transaction data, bulk operations, faucet

**R6: Add Transaction History to Wallet Screen**
- Show recent transactions below the balance dashboard
- Include: date, amount (with fiat), direction (sent/received), status (pending/confirmed), counterparty (address or DPNS username if known)
- Link each transaction to a block explorer

**R7: Unify HD Wallet and SingleKeyWallet Views**
- Both wallet types should appear in the same wallet selector
- Visually distinguish them (icon or label: "HD Wallet" vs "Imported Key")
- SingleKeyWallet should have the same balance dashboard pattern, just simpler

**R8: Visual Treatment of Locked/Special Funds**
- Asset locks: Show as a separate "Locked" section with clear explanation
- CoinJoin funds: If applicable, show separately with mixing status
- Platform Credits: Show in Platform section with "Credits" terminology
- Use color coding: Available (default), Locked (grey/amber), Pending (pulsing/animated)

### 7.3 Lower Priority (Polish and Advanced Features)

**R9: Add Fiat Equivalents Everywhere**
- Display fiat value alongside every crypto amount
- User-selectable base currency (USD, EUR, etc.)
- Requires a price feed integration

**R10: Coin Control (Developer Tools)**
- Allow selecting specific UTXOs for spending (like Electrum's Coins tab)
- Show per-UTXO: amount, confirmations, address, derivation path
- Support freezing/unfreezing UTXOs
- Only visible with Developer Tools enabled (Level 2)

**R11: Contextual Help System**
- Tooltips on all balance fields explaining what they mean
- First-run walkthrough for new users
- "What is this?" links for Dash-specific concepts (CoinJoin, Platform, Asset Locks)

**R12: Network Status and Sync Indicators**
- Show SPV sync status clearly
- Indicate network (Mainnet/Testnet/Devnet) prominently
- Show connection quality indicator

---

## Sources

### Dash-Specific
- [Dash Core Wallet Documentation](https://docs.dash.org/en/stable/docs/user/wallets/dashcore/index.html)
- [Dash Electrum Documentation](https://docs.dash.org/en/stable/docs/user/wallets/electrum/index.html)
- [DashPay Explanation](https://dashplatform.readme.io/docs/explanation-dashpay)
- [DashPay on Google Play](https://play.google.com/store/apps/details?id=hashengineering.darkcoin.wallet)
- [Dash Roadmap](https://www.dash.org/roadmap/)

### Desktop Wallets
- [Exodus Wallet Review 2026](https://cryptomaniaks.com/reviews/wallets/exodus)
- [Exodus Multi-Portfolio Support](https://support.exodus.com/article/1527-how-do-i-create-multiple-portfolios-on-the-same-device)
- [Exodus Portfolio Total Value](https://support.exodus.com/article/32-what-is-the-total-value-of-my-exodus-wallet)
- [Ledger Live UI/UX Guidelines](https://developers.ledger.com/docs/blockchain/ui-ux-guidelines)
- [Ledger Live Desktop UI Flow](https://developers.ledger.com/docs/blockchain/ui-ux-guidelines/lld-ui-guidelines)
- [Wasabi Wallet 2.0 Announcement](https://blog.wasabiwallet.io/wasabi-wallet-2/)
- [Wasabi Wallet Review 2026](https://cryptoadventure.com/wasabi-wallet-review-2026-privacy-focused-bitcoin-desktop-wallet-with-coin-control/)
- [Electrum Coin Control Guide](https://bitcoinelectrum.com/how-to-spend-specific-utxos-in-electrum/)

### Browser/Extension Wallets
- [MetaMask UX Update 2025](https://metamask.io/news/metamask-extensions-updated-ux-elevates-network-dapp-and-account-selection)
- [MetaMask Roadmap 2025](https://metamask.io/news/metamask-roadmap-2025)
- [MetaMask Product Updates 2025](https://metamask.io/news/metamask-product-updates-2025)
- [MetaMask UX Case Study (Expedite Studio)](https://expeditestudio.com/case-studies/metamask/)
- [MetaMask Account Switching](https://support.metamask.io/configure/accounts/switching-accounts-in-metamask)
- [Trust Wallet Multiple Accounts](https://trustwallet.com/blog/announcements/multiple-accounts-now-live-in-trust-wallet-extension)

### UX Best Practices and Research
- [2025 Guide to Crypto Wallet UX (Cryptowisser)](https://www.cryptowisser.com/guides/crypto-wallet-ux-guide-2025/)
- [5 UX Design Principles for Crypto Wallets (SpaceKayak)](https://www.spacekayak.xyz/blogs/5-ux-design-principles-for-the-best-crypto-wallet-experience)
- [8 Best UX Practices for Blockchain Design (ProCreator)](https://procreator.design/blog/designing-for-blockchain-best-ux-practices/)
- [Crypto Wallet UX Design (Alien Design)](https://www.thealien.design/insights/crypto-wallet-ux-design)
- [The U in Crypto Stands for Usable - CHI 2021 Empirical Study](https://dl.acm.org/doi/fullHtml/10.1145/3411764.3445407)
- [Three Challenges in Crypto Wallet UX (Inspire X)](https://medium.com/@inspirexnewsletter/three-challenges-in-crypto-wallet-ux-design-and-the-role-of-ux-in-web-3-0-80aad1784ec6)
- [Bitcoin Design - Coin Selection](https://bitcoin.design/guide/how-it-works/coin-selection/)
- [HD Wallets and Derivation Paths (MyEtherWallet)](https://help.myetherwallet.com/en/articles/5867305-hd-wallets-and-derivation-paths)
- [Understanding Derivation Paths (Ledger)](https://www.ledger.com/blog/understanding-crypto-addresses-and-derivation-paths)

### User Feedback and Reviews
- [Digital Wallet Fatigue 2025 (Decta)](https://www.decta.com/company/media/digital-wallet-and-financial-app-user-experience-2025)
- [Trust Wallet Trustpilot Reviews](https://www.trustpilot.com/review/trustwallet.com)
- [MetaMask UX Overhaul (Blockworks)](https://blockworks.co/news/metamask-ux-overhaul)
