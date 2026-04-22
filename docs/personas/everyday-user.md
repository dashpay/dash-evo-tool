# Persona: Everyday User

## Identity

| Field | Value |
|---|---|
| **Name** | Alex Torres |
| **Age** | 32 |
| **Occupation** | Freelance graphic designer |
| **Technical Proficiency** | Low to moderate; comfortable with mobile apps and basic desktop software, but has no blockchain development experience |
| **Crypto Experience** | 1-2 years; holds Dash and one or two other cryptocurrencies; has used Dash wallets on mobile (Dash Wallet for Android/iOS) |
| **Network** | Mainnet exclusively |

## Description

Alex is a regular Dash holder who primarily uses the currency for occasional payments, receiving freelance payments from clients who prefer crypto, and long-term holding. Alex does not understand or care about derivation paths, BIP standards, or the internal structure of HD wallets. Alex registered a DPNS username to make receiving payments easier and may explore DashPay for social payments in the future.

Alex installed Dash Evo Tool because it was recommended for managing DPNS names and because the mobile wallet does not support all Platform features. Alex expects the wallet experience to feel similar to familiar fintech apps (Exodus, DashPay mobile): show the balance with a fiat equivalent, let them send, let them receive. Alex has used the DashPay mobile wallet and appreciates being able to send to a username instead of an address.

## Primary Goals

1. **See total balance at a glance** -- Know how much Dash is available without needing to understand account types or derivation paths.
2. **Send Dash** -- Pay someone by entering an address and an amount.
3. **Receive Dash** -- Get a receiving address (ideally with a QR code) to share with a payer.
4. **Register and manage a DPNS name** -- Have a human-readable username that maps to a Dash address.
5. **Keep funds secure** -- Password-protect the wallet; trust that the app handles key management correctly.

## Secondary Goals

- Explore DashPay social features (contacts, profiles).
- Hold tokens issued on the Dash Platform.
- Understand transaction history (what was sent, what was received, when).

## Pain Points (Current App)

1. **Overwhelming account structure.** The wallet screen shows "Main Account," "Platform Account," "Identity Registration," "Identity System," "Identity Top-up," "CoinJoin," and many more account categories. Alex does not know what most of these are and feels intimidated. (Competitors like Exodus and MetaMask show only relevant accounts with non-zero balances.)
2. **Confusing address list.** The address table displays derivation paths, UTXO counts, and "View Key" buttons for every address. Alex does not need to see any of this and finds it noisy. (No competitor wallet shows address tables by default -- this is universally an advanced/opt-in feature.)
3. **Unclear balance presentation.** The screen shows "Core balance" and "Platform balance" separately. Alex does not understand the distinction and wonders which number is "the real balance." (Industry standard is a single total balance with breakdown on demand.)
4. **No fiat equivalent.** The balance is shown only in DASH. Alex thinks in USD/EUR and cannot easily judge the value of their holdings. (Every major competitor -- Exodus, Ledger Live, MetaMask, Trust Wallet -- shows fiat equivalents prominently.)
5. **Technical jargon.** Terms like "Asset Lock," "BIP44," "derivation path," "Platform credits," "duffs," and "seed hash" appear in the UI. These are meaningless to Alex. (Industry terminology standards recommend "Recovery phrase" not "mnemonic," "Main Account" not "BIP44," "Platform deposit" not "Asset Lock.")
6. **No transaction history by default.** Transaction history only appears in developer mode. Alex has no way to review past payments. (Transaction history is a Level 0 feature in every competitor wallet.)
7. **Receive flow shows too many addresses.** The receive dialog shows all BIP44 external addresses with balances and a "New Address" button. Alex does not know which address to use or why there are many. (Competitors show exactly one address by default.)
8. **No send-to-username support.** The DashPay mobile wallet allows sending to DPNS usernames, but Dash Evo Tool requires raw addresses even though it supports DPNS on the Contracts screen.

## Success Metrics

| Metric | Target |
|---|---|
| Time to check balance | Under 2 seconds after opening the wallet screen |
| Time to initiate a send | Under 10 seconds (tap Send, enter address and amount, confirm) |
| Time to get a receive address | Under 5 seconds (tap Receive, see QR code) |
| Support requests about "what is an asset lock" | Zero -- the concept should be invisible or explained inline |
| Task completion rate for first send | 95% without external help |

## Frequency of Interaction

- Opens the wallet screen 2-5 times per week.
- Sends or receives Dash 1-3 times per month.
- Checks balance more often than transacting.
- Interacts with Platform features (DPNS, DashPay) occasionally after initial setup.

## What Alex Does NOT Need to See

- Derivation paths or BIP standards.
- Individual address lists with UTXO counts.
- Refresh mode selectors (Core Only, Platform Full, etc.).
- Asset lock creation or management (should be abstracted if needed at all).
- "View Key" buttons for every address.
- Platform credits expressed in raw credit values.

## Quotes (Illustrative)

> "I just want to know my balance and send money. Why does this screen have 12 different account types?"

> "What is an 'asset lock'? Did something go wrong with my funds?"

> "I clicked Receive and got a list of 30 addresses. Which one do I give to my client?"
