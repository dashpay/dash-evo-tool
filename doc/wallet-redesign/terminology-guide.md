# Wallet Screen Terminology Guide

This guide defines the user-facing language used in the wallet screen. Every label, heading, button, and tooltip must use these terms consistently. Technical terms that appear in the codebase are mapped to their user-facing equivalents.

---

## Core Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| `Wallet` (HD) | **Wallet** | Wallet selector, headings | No need to say "HD Wallet" in user-facing text. The type badge "[HD]" is sufficient. |
| `SingleKeyWallet` | **Imported Key** | Wallet selector, headings | Subtitle "Imported Key" shown below wallet alias. Type badge "[SK]" in selector. |
| `WalletSeed` / Mnemonic | **Recovery phrase** | Creation, backup, import flows | Never use "seed phrase," "mnemonic," or "BIP39 words" in user-facing text. |
| `WalletSeedHash` | (not shown) | Internal use only | Never exposed to users. |
| Private key (WIF) | **Private key** | Key export dialog | Acceptable in the "View Key" context since only technical users access this. |
| BIP44 account | **Main Account** | Account category labels | The primary spending account. |
| BIP32 account | **Legacy Account** | Account category labels | Old-format addresses from pre-BIP44 wallets. |
| CoinJoin account | **Private Send** | Account category labels | Matches Dash Core's terminology for the privacy feature. |
| PlatformPayment | **Platform Account** | Account category labels | DIP-17 payment addresses. |
| IdentityRegistration | **Identity Keys** | Account category labels (Level 1+) | Key-only category, no balance. |
| IdentitySystem | **System Keys** | Account category labels (Level 1+) | Key-only category, no balance. |
| IdentityTopup | **Top-up Keys** | Account category labels (Level 1+) | Key-only category, no balance. |
| IdentityInvitation | **Invitation Keys** | Account category labels (Level 1+) | Key-only category, no balance. |
| ProviderVoting | **Masternode Voting** | Account category labels (Level 1+) | Masternode operator keys. |
| ProviderOwner | **Masternode Owner** | Account category labels (Level 1+) | Masternode operator keys. |
| ProviderOperator | **Masternode Operator** | Account category labels (Level 1+) | Masternode operator keys. |
| ProviderPlatform | **Masternode Platform** | Account category labels (Level 1+) | Masternode operator keys. |

---

## Balance and Amount Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| `total_balance` | **Total Balance** | Balance header | Combined Core + Platform. |
| `confirmed_balance` | **Available** | Balance breakdown (if needed) | Confirmed, spendable funds. |
| `unconfirmed_balance` | **Pending** | Balance breakdown (Level 2) | Unconfirmed incoming. |
| Core balance | **Core** or **On-chain** | Balance breakdown | When showing the split: "Core: 1.0000 DASH". |
| Platform credits | **Platform** | Balance breakdown | When showing the split: "Platform: 0.2345 DASH". |
| Credits (raw) | **credits** | Level 2 parenthetical | "0.2345 DASH (234,500,000 credits)" -- only in Developer mode. |
| Duffs | (not shown) | Never in user-facing text | Use DASH with 8 decimal places instead. Developer mode may show duffs in tooltips. |
| `CREDITS_PER_DUFF` | (not shown) | Internal conversion only | Never exposed. |

---

## Transaction Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| `WalletTransaction` | **Transaction** | Transaction history | No prefix needed. |
| TxID / `txid` | **Transaction ID** or **TxID** | Transaction table, send result | "TxID" is acceptable as shorthand in table headers. |
| InstantSend / IS lock | **Instant confirmation** | Transaction status | "Confirmed instantly" or "Instant confirmation" badge. |
| Confirmation count | **X confirmations** or **X/6** | Transaction status column | "Pending (2/6 confirmations)" for unconfirmed. |
| `net_amount` | **Amount** | Transaction table | Show as "+0.5000" (received) or "-0.2500" (sent). |
| Fee | **Fee** | Transaction detail, send confirmation | "Fee: 0.0001 DASH". |
| Sent | **Sent** | Transaction type | Red upward arrow or minus sign. |
| Received | **Received** | Transaction type | Green downward arrow or plus sign. |
| Internal (change) | **Internal** | Transaction type | Grey circular arrows. For Level 1+ only. |

---

## Address Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| Address (P2PKH) | **Address** | Address table, receive dialog | No prefix needed. Dash addresses are "X..." or "y..." |
| Platform address (DIP-17) | **Platform address** | Platform account, receive dialog | Bech32m format: "evo1..." or "tevo1..." |
| DIP-18 / Bech32m | (not mentioned) | Internal | Users see the address format; they do not need the standard name. |
| Derivation path | **Derivation path** or **Path** | Address table (Level 2 only) | Only shown in Developer mode. |
| Index | **Index** | Address table (Level 2 only) | Only shown in Developer mode. |
| Funds address | **Receiving** | Address type column | External addresses used for receiving funds. |
| Change address | **Change** | Address type column | Internal change addresses. |
| System address | **System** | Address type column | System-purpose addresses. |

---

## Asset Lock Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| Asset lock | **Locked funds** or **Asset lock** | Asset locks section | "Asset lock" is acceptable for Level 1+ users (Priya, Jordan). For Level 0 users, asset locks are hidden entirely. |
| Asset lock proof | **Proof** | Asset lock detail | "Proof available: Yes/No". |
| IS lock (on asset lock) | **Instant lock** | Asset lock table | "Instant lock: Yes/No". |
| Usable | **Ready to use** | Asset lock table | "Ready to use: Yes" is clearer than "Usable: Yes". |
| `CreateAssetLock` | **Create Locked Funds** or **Lock Dash for Platform** | Button label | Descriptive action label. |
| `SearchForUnusedAssetLocks` | **Find Unused Locks** | Button label | Shorter, action-oriented. |
| Purpose: Registration | **For identity registration** | Asset lock creation | Describes why the lock is being created. |
| Purpose: TopUp | **For identity top-up** | Asset lock creation | Describes why the lock is being created. |

---

## Wallet Management Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| Create wallet | **Create Wallet** | Button, screen title | |
| Import wallet | **Import Wallet** | Button, screen title | |
| Alias | **Wallet name** or just **Name** | Rename dialog, selector | "Alias" is technical jargon. |
| Remove wallet | **Remove Wallet** | Button, dialog title | Not "delete" (the wallet still exists on-chain; only local data is removed). |
| Lock wallet | **Lock** | Overflow menu item | |
| Unlock wallet | **Unlock** | Overflow menu item, popup title | |
| Open (wallet state) | **Unlocked** | Lock status indicator | |
| Closed (wallet state) | **Locked** | Lock status indicator | |
| Password | **Password** | Creation, unlock flows | |
| Refresh | **Refresh** | Button label | |
| `RefreshMode::All` | **All (Auto)** | Refresh mode selector | |
| `RefreshMode::CoreOnly` | **Core Only** | Refresh mode selector | |
| `RefreshMode::PlatformFull` | **Platform (Full Sync)** | Refresh mode selector | |
| `RefreshMode::PlatformTerminal` | **Platform (Quick Sync)** | Refresh mode selector | "Terminal" is unclear to non-developers. "Quick Sync" conveys the intent. |
| `RefreshMode::CoreAndPlatformFull` | **Core + Platform (Full)** | Refresh mode selector | |
| `RefreshMode::CoreAndPlatformTerminal` | **Core + Platform (Quick)** | Refresh mode selector | |

---

## Network Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| Mainnet | **Mainnet** | Network indicator | Acceptable as-is; widely understood. |
| Testnet | **Testnet** | Network indicator | Acceptable as-is. |
| Devnet | **Devnet** | Network indicator | Level 2 only. |
| tDASH | **tDASH** | Balance display on Testnet | Prefix "t" indicates test funds. |
| SPV | (not shown) | Internal | Users do not need to know the sync method. Connection status is sufficient. |
| Core RPC | (not shown) | Internal | Exposed only in Devnet configuration (Level 2). |
| gRPC endpoint | **Platform endpoint** | Devnet configuration | Simplified label for developers. |

---

## Platform Operations Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| Fund Platform address | **Fund** | Platform address action button | Short verb for the action. |
| Withdraw from Platform | **Withdraw** | Platform address action button | |
| Transfer credits | **Transfer** | Platform address action button | |
| State transition | (not shown to Level 0/1) | Developer mode only | "State transition" is protocol jargon. |
| Identity | **Identity** | Cross-screen reference | Acceptable as-is; central Dash concept. |
| DPNS | **Username** or **Dash Name** | User-facing label | "DPNS" is an acronym; use "Dash username" or just "username" in user-facing text. |

---

## Developer Mode Terminology

| Internal / Technical Term | User-Facing Term | Context | Notes |
|---|---|---|---|
| Developer mode (current) | **Developer Tools** | Settings label | Renamed to distinguish from the former binary toggle. |
| `developer_mode` setting | **Developer Tools** | Settings screen | "Enable Developer Tools" toggle in Settings. |
| Faucet | **Get Test Dash** | Action button (Testnet) | Friendlier than "Faucet." |
| Bulk create | **Create Multiple** | Asset lock batch action | |
| Export CSV | **Export** | Transaction history action | |

---

## Button Labels

Consistent, action-oriented button labels used across the wallet screen:

| Action | Button Label | Notes |
|---|---|---|
| Send Dash | **Send** | Primary action button. |
| Receive Dash | **Receive** | Primary action button. |
| Refresh balances | **Refresh** | Secondary action button. |
| Create new wallet | **Create Wallet** | Empty state and selector dropdown. |
| Import existing wallet | **Import Wallet** | Empty state and selector dropdown. |
| Copy address to clipboard | **Copy Address** | In receive dialog and address table. |
| Copy TxID to clipboard | **Copy** or click-to-copy | In transaction table. |
| Generate new address | **New Address** | In receive dialog. |
| Add receiving address | **Add Receiving Address** | In address table (Main Account). |
| New Platform address | **New Platform Address** | In address table (Platform Account). |
| View private key | **View Key** | In address table row action. |
| Show/hide password | **Show** / **Hide** | Toggle in password fields. |
| Proceed to next step | **Continue** | Multi-step flows. |
| Go back one step | **Back** | Multi-step flows. |
| Cancel and close | **Cancel** | Dialogs and flows. |
| Confirm destructive action | **Remove Wallet** | Matches the action being confirmed. |
| Confirm send | **Confirm & Send** | Send flow confirmation step. |
| Return to wallet screen | **Back to Wallet** | After send completion. |
| Retry failed operation | **Try Again** or **Retry** | Error states. |
| Show recovery phrase | **Show Recovery Phrase** | Overflow menu (HD wallets). |
| Lock wallet | **Lock** | Overflow menu. |
| Unlock wallet | **Unlock** | Overflow menu, unlock popup. |
| Rename wallet | **Rename** | Overflow menu. |
| Request test funds | **Get Test Dash** | Action bar (Testnet, Level 2). |
| Find unused asset locks | **Find Unused Locks** | Asset locks section. |
| Create asset lock | **Create Asset Lock** | Asset locks section. |
| Fund Platform address | **Fund** | Platform address table action. |
| Withdraw from Platform | **Withdraw** | Platform address table action. |

---

## Tooltip and Help Text Conventions

1. **Tooltips** appear on hover over UI elements. They provide brief clarification (one sentence maximum).
2. **Inline help** appears below input fields to explain expected format or constraints.
3. **Section descriptions** appear below section headers when sections are expanded, providing context for the content.

### Example Tooltips

| Element | Tooltip Text |
|---|---|
| Total Balance | "Combined balance across Core chain and Platform" |
| Core balance | "Funds on the Dash blockchain (layer 1)" |
| Platform balance | "Credits on Dash Platform (layer 2)" |
| Lock icon (locked) | "Wallet is locked. Click to unlock." |
| Lock icon (unlocked) | "Wallet is unlocked. Private keys are accessible." |
| [HD] badge | "Hierarchical Deterministic wallet with multiple addresses" |
| [SK] badge | "Single address wallet imported from a private key" |
| Refresh mode selector | "Choose what to refresh: Core balances, Platform balances, or both" |
| "Ready to use: No" | "This asset lock is not yet confirmed by the network" |
| Derivation path | "BIP44 path used to derive this address from the recovery phrase" |

### Example Inline Help

| Field | Help Text |
|---|---|
| Recovery phrase input | "Enter your 12, 15, 18, 21, or 24 word recovery phrase" |
| Password (creation) | "Optional. Protects your wallet with encryption." |
| Send address input | "Enter a Dash address (X...) or Platform address (evo1...)" |
| Send amount input | "Enter amount in DASH. Type 'max' for maximum spendable amount." |

### Example Section Descriptions

| Section | Description (shown when expanded) |
|---|---|
| Main Account | "Your primary Dash addresses for sending and receiving" |
| Platform Account | "Addresses for holding credits on Dash Platform" |
| Private Send | "Addresses used for CoinJoin privacy mixing" |
| Identity Keys | "Key paths used for identity registration (no balance)" |
| Asset Locks | "Dash locked for use on Platform (identity funding, address funding)" |
