# Wallet Redesign: Use Case Scenarios

This directory contains use case scenarios for the Dash Evo Tool wallet screen redesign, organized by functional category. Each use case is tied to specific personas defined in [../personas/](../personas/).

## Use Case Categories

| Category | File | Use Cases | Primary Personas |
|---|---|---|---|
| **Wallet Management** | [01-wallet-management.md](01-wallet-management.md) | 9 use cases | All |
| **Send and Receive** | [02-send-receive.md](02-send-receive.md) | 6 use cases | All |
| **Platform Operations** | [03-platform-operations.md](03-platform-operations.md) | 8 use cases | Priya, Jordan |
| **Security and Keys** | [04-security-and-keys.md](04-security-and-keys.md) | 6 use cases | All |
| **Address Management** | [05-address-management.md](05-address-management.md) | 6 use cases | Priya, Jordan |
| **Network and Settings** | [06-network-and-settings.md](06-network-and-settings.md) | 4 use cases | Priya, Jordan |

**Total: 39 use cases**

## Use Case ID Convention

Each use case has an ID in the format `UC-XX-NN` where:
- `XX` is the category code (WM, SR, PO, SK, AM, NS)
- `NN` is the sequential number within that category

## Coverage Matrix: Persona vs. Feature

| Use Case | Alex (Everyday) | Priya (Power) | Jordan (Developer) |
|---|---|---|---|
| **Wallet Management** | | | |
| UC-WM-01 Create wallet | Guided flow | Full options | Quick-create mode |
| UC-WM-02 Import mnemonic | Word-by-word | Advanced options | Paste-friendly |
| UC-WM-03 Import private key | -- | Yes | Yes |
| UC-WM-04 Switch wallets | -- | Yes | Yes |
| UC-WM-05 Rename wallet | Yes | Yes | Yes |
| UC-WM-06 Remove wallet | -- | Yes (with confirmation) | Yes (quick) |
| UC-WM-07 Lock/unlock | Yes | Yes | -- |
| UC-WM-08 View balance | Simple total | Core/Platform split | + Raw credits |
| UC-WM-09 Refresh | One button | Granular controls | + Auto-refresh |
| **Send and Receive** | | | |
| UC-SR-01 Send to Core address | Simple dialog | + Fee control | + Max keyword |
| UC-SR-02 Send from SK wallet | -- | Yes | Yes |
| UC-SR-03 Receive on Core | QR + one address | Address selector | + Derivation path |
| UC-SR-04 Receive on Platform | -- | Yes | Yes |
| UC-SR-05 Transaction history | Simple list | Full table | + Filter/export |
| UC-SR-06 Batch send | -- | Yes | -- |
| **Platform Operations** | | | |
| UC-PO-01 Create asset lock | -- | Full control | + Quick/bulk create |
| UC-PO-02 Fund from asset lock | -- | Yes | Yes |
| UC-PO-03 Direct Platform fund | -- | Yes | Yes |
| UC-PO-04 Transfer credits | -- | Yes | + Batch |
| UC-PO-05 Withdraw to Core | -- | Yes | Yes |
| UC-PO-06 Search asset locks | -- | Yes | Yes |
| UC-PO-07 View asset lock detail | -- | Yes | Yes |
| UC-PO-08 Account categories | -- | Full dropdown | + Activity filter |
| **Security and Keys** | | | |
| UC-SK-01 Set password | Yes | Yes | Optional |
| UC-SK-02 Export private key | -- | Yes | Yes |
| UC-SK-03 View seed phrase | Yes | Yes | -- |
| UC-SK-04 Auto-lock | Yes | Yes | -- |
| UC-SK-05 Password on send | Opt-in | -- | -- |
| UC-SK-06 Secure removal | -- | Yes | -- |
| **Address Management** | | | |
| UC-AM-01 Address table | -- | Full table | + Copy path |
| UC-AM-02 Add receive address | Via Receive dialog | Via address table | Via address table |
| UC-AM-03 New Platform address | -- | Yes | Yes |
| UC-AM-04 Copy address | Yes | Yes | Yes |
| UC-AM-05 Dual format display | -- | Yes | Yes |
| UC-AM-06 Filter by activity | -- | Yes | Yes |
| **Network and Settings** | | | |
| UC-NS-01 Switch networks | -- | Mainnet/Testnet | + Devnet |
| UC-NS-02 Developer Tools | -- | Partial (power features) | Full |
| UC-NS-03 Devnet config | -- | -- | Yes |
| UC-NS-04 Testnet faucet | -- | -- | Yes |

Legend: "Yes" = feature applies to this persona. "--" = not relevant or should be hidden. Additional notes indicate persona-specific variations.

## Features Identified for Addition (Not Currently Implemented)

The following features were identified during use case analysis as gaps in the current implementation:

| Feature | Use Case | Personas | Priority | Competitor Precedent |
|---|---|---|---|---|
| Transaction history for all users | UC-SR-05 | All | **Must Have** | Every competitor wallet (Level 0 feature) |
| Fee estimate before send confirmation | UC-SR-01 | All | **Must Have** | Exodus, Ledger Live, MetaMask (preview + confirm) |
| Fiat equivalent display | UC-WM-08 | All | **Must Have** | Exodus, Ledger Live, MetaMask, Trust Wallet |
| Balance dashboard as default view | UC-WM-08 | All | **Must Have** | Exodus, Ledger Live (portfolio-first) |
| Send to DPNS username | UC-SR-01 | Alex, Priya | **Must Have** | DashPay mobile (social payments) |
| Address activity filter | UC-AM-06 | Priya, Jordan | Should Have | Electrum (opt-in address display) |
| Batch send (multi-recipient) | UC-SR-06 | Priya | Should Have | Electrum, Dash Core (multi-output) |
| View seed phrase | UC-SK-03 | Alex, Priya | Should Have | Standard in all competitors |
| Auto-lock on inactivity | UC-SK-04 | Alex, Priya | Should Have | Exodus (first-run setup) |
| Quick wallet creation (skip backup) | UC-WM-01 | Jordan | Should Have | MetaMask (dev-friendly onboarding) |
| Address labeling/notes | n/a | Priya | Should Have | Electrum, Dash Electrum (label system) |
| Confirmation for SK wallet removal | UC-WM-06 | Priya | Should Have | Standard UX for destructive operations |
| Contextual help / tooltips | n/a | All | Should Have | Industry best practice (progressive education) |
| Testnet faucet integration | UC-NS-04 | Jordan | Could Have | Common in developer-targeted tools |
| In-app Devnet configuration | UC-NS-03 | Jordan | Could Have | Trust Wallet (custom networks), MetaMask (custom RPC) |
| Bulk asset lock creation | UC-PO-01 | Jordan | Could Have | No competitor precedent (Dash-specific) |
| Password re-entry on send | UC-SK-05 | Alex | Could Have | Dash Core (password on send) |
| Coin control / UTXO selection | n/a | Priya | Could Have | Electrum, Wasabi, Dash Core (advanced) |

## Features Recommended for Removal or Deferral

| Feature | Reason | Recommendation |
|---|---|---|
| "Developer mode" as a global toggle | Misleading label; conflates power user features with developer tools | Replace with progressive disclosure (expand/collapse per section) and a separate "Developer Tools" setting |
| Showing all 12+ account categories by default | Overwhelming for everyday users | Show only "Main Account" and "Platform Account" by default; others behind "Show All Accounts" |
| Derivation paths in the address table for non-technical users | Meaningless to everyday users | Hide unless expanded or in detailed view |
| Refresh mode selector in main wallet toolbar | Clutters the UI for everyday users | Move to a settings menu or a dropdown within the Refresh button for power users |
