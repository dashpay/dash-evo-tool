# Persona: Power User

## Identity

| Field | Value |
|---|---|
| **Name** | Priya Nakamura |
| **Age** | 41 |
| **Occupation** | IT systems administrator; part-time masternode operator |
| **Technical Proficiency** | High; comfortable with command-line tools, networking, and server administration; understands blockchain internals at a conceptual level |
| **Crypto Experience** | 5+ years; runs a Dash masternode; actively manages multiple wallets; understands BIP44, UTXOs, and transaction mechanics |
| **Network** | Primarily Mainnet; occasionally Testnet for verification before mainnet operations |

## Description

Priya is a technically fluent Dash community member who runs a masternode and actively participates in the Dash ecosystem. She manages multiple HD wallets and understands the relationship between derivation paths, addresses, and keys. She uses Dash Evo Tool as her primary desktop wallet because it offers Platform features (identities, DPNS, tokens) that command-line tools do not present in a unified interface.

Priya wants full visibility into her wallet's internal structure. She needs to see account breakdowns, individual addresses, balances per address, and UTXO details. She creates asset locks deliberately to fund identities and Platform addresses. She understands the difference between Core chain funds and Platform credits and needs to manage both.

Unlike a developer building on Dash, Priya is an **operator and advanced user** -- she does not write code against the Dash SDK, but she needs the level of detail and control that the current "developer mode" provides.

## Primary Goals

1. **Full wallet visibility** -- See all account types (BIP44, BIP32, CoinJoin, Identity, Provider, Platform) with per-address balances, UTXO counts, and derivation paths.
2. **Manage multiple wallets** -- Switch between wallets easily; rename, lock/unlock, and remove wallets.
3. **Asset lock management** -- Create asset locks, search for unused ones, fund Platform addresses from asset locks.
4. **Identity operations** -- Register identities, top up identity balances, manage identity keys.
5. **Platform address management** -- Fund, transfer credits between, and withdraw from Platform (DIP-17) addresses.
6. **Transaction history** -- Review all past transactions with amounts, dates, TxIDs, and confirmation status.
7. **Masternode key management** -- Access provider voting, owner, operator, and platform node key paths.

## Secondary Goals

- Verify address derivation by inspecting full derivation paths.
- Export private keys (WIF) for specific addresses when needed.
- Monitor CoinJoin balances.
- Test operations on Testnet before executing on Mainnet.
- Create and manage tokens on Platform.

## Pain Points (Current App)

1. **"Developer mode" is a misleading label.** Priya is not a developer but needs all the features currently behind the developer mode toggle. The label makes her feel like she is using an unsupported or unstable mode.
2. **No batch operations.** Transferring Platform credits from multiple addresses requires individual operations. Priya wants multi-select.
3. **Asset lock table is too compact.** Transaction IDs and addresses are truncated; there is no way to sort or filter asset locks.
4. **Refresh is one action.** Priya sometimes wants to refresh only Core balances or only Platform balances, but the default "Refresh" button does both. The dev-mode refresh selector helps, but it is hidden behind the mode toggle.
5. **No address labeling.** Priya cannot annotate individual addresses with notes like "Masternode collateral" or "Cold storage."
6. **Wallet selector shows raw balance.** When managing 4-5 wallets, the dropdown format "HD: WalletName (0.1234 DASH)" is functional but not ideal for quick identification.
7. **No confirmation details before sending.** The send dialog does not show a fee estimate or the exact amount to be deducted before confirmation.
8. **Single key wallets are second-class.** They lack Platform features, transaction history, and account structure. Priya imports private keys for specific purposes and wants at least basic feature parity.

## Success Metrics

| Metric | Target |
|---|---|
| Time to find a specific address across wallets | Under 15 seconds (search or filter) |
| Ability to identify wallet purpose at a glance | Wallet alias + icon/tag visible in selector |
| Asset lock creation to Platform funding | Completable in a single continuous flow |
| Time to check masternode key paths | Under 10 seconds from wallet screen |
| Refresh control granularity | Available without needing a hidden mode toggle |

## Frequency of Interaction

- Opens the wallet screen daily.
- Performs send/receive operations multiple times per week.
- Manages asset locks and Platform addresses weekly.
- Checks masternode-related key paths monthly or when reconfiguring.
- Switches between wallets multiple times per session.

## What Priya Needs That Alex Does Not

- Full address table with derivation paths, balances, UTXOs, and total received.
- Asset lock creation, recovery, and funding workflows.
- Granular refresh controls (Core only, Platform only, combined).
- Transaction history with TxID, block height, and confirmation status.
- "View Key" functionality for exporting private keys.
- Account-level breakdown (Main Account, CoinJoin, Identity Registration, etc.).
- Provider key paths (voting, owner, operator, platform node).
- Network switching to Testnet.

## Quotes (Illustrative)

> "I need to see every address in my wallet, what derivation path it is on, and whether it has been used. This is basic information."

> "Why is transaction history only available in 'developer mode'? I am not a developer, I am a user who wants to see my transactions."

> "I created an asset lock yesterday, but I cannot find it in the list. The search/recover feature should be more prominent."
