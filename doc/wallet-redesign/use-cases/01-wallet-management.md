# Use Cases: Wallet Management

## UC-WM-01: Create a New HD Wallet

**Personas**: Alex, Priya, Jordan

### User Story
As a new user, I want to create a fresh HD wallet so that I can start holding and transacting Dash.

### Acceptance Criteria

```
Given I have no wallets loaded,
When I click "Create Wallet,"
Then I am shown a BIP39 mnemonic seed phrase and asked to back it up.

Given I have confirmed my seed phrase backup,
When I optionally set a password and alias,
Then a new HD wallet is created with BIP44 addresses bootstrapped and I am returned to the wallet screen with this wallet selected.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Guided step-by-step flow. Emphasize that the seed phrase must be written down. Offer to set a password. Auto-generate a friendly alias. |
| Priya | Same flow, but also show the master public key and first few addresses for verification. Allow choosing word count (12/15/18/21/24). |
| Jordan | Option to skip seed phrase backup confirmation for throwaway test wallets. "Quick create" mode that generates a wallet in one click with no password. |

### Real-Life Scenario: Alex Creates First Wallet
**Context**: Alex has just installed Dash Evo Tool and sees the "No Wallets Loaded" screen.
**Flow**: Alex clicks "Create Wallet." The app generates a 12-word mnemonic and displays it one word at a time. Alex writes the words on paper. The app asks Alex to re-enter words 3, 7, and 11 for confirmation. Alex sets a password and names the wallet "My Dash." The app shows the wallet screen with a 0.00000000 DASH balance and a Receive button.
**Expected Outcome**: Wallet is created, secured with a password, and ready to receive funds.
**What Could Go Wrong**: Alex does not back up the seed phrase. The app should warn strongly but ultimately allow proceeding (the user owns their risk). Alex forgets the password; the seed phrase is the only recovery path.

---

## UC-WM-02: Import an Existing HD Wallet via Mnemonic

**Personas**: Alex, Priya, Jordan

### User Story
As a user with an existing Dash wallet, I want to import it into Dash Evo Tool by entering my seed phrase so that I can manage it from this app.

### Acceptance Criteria

```
Given I am on the import screen,
When I enter a valid BIP39 mnemonic (12, 15, 18, 21, or 24 words),
Then the app derives the master key, bootstraps addresses, and loads the wallet.

Given the imported wallet has existing balances,
When the wallet is loaded and refreshed,
Then all Core balances, Platform balances, identities, and asset locks are discovered and displayed.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Simple word-by-word input. Auto-suggest from BIP39 word list. Clear error if a word is invalid. |
| Priya | Option to specify identity scan count (how many identity indices to check). Advanced options for non-standard derivation paths. |
| Jordan | Paste-friendly: accept the entire mnemonic as a single space-separated string. |

---

## UC-WM-03: Import a Single Private Key

**Personas**: Priya, Jordan

### User Story
As a technical user, I want to import a single private key (WIF or hex) so that I can manage funds at a specific address.

### Acceptance Criteria

```
Given I am on the import screen and select "Private Key" tab,
When I enter a valid WIF or hex-encoded private key,
Then a SingleKeyWallet is created with the derived P2PKH address.

Given the imported key corresponds to an address with existing UTXOs,
When I click Refresh,
Then the balance and UTXOs are loaded and displayed.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Priya | Uses this for masternode collateral keys or cold storage keys that need occasional access. |
| Jordan | Uses this for scripting: generates a key externally, imports it to test Platform operations. |

---

## UC-WM-04: Switch Between Wallets

**Personas**: Priya, Jordan

### User Story
As a user with multiple wallets, I want to switch between them quickly so that I can manage funds across wallets without friction.

### Acceptance Criteria

```
Given I have 3+ wallets loaded,
When I open the wallet selector dropdown,
Then I see all wallets listed with their alias, type (HD/SK), and balance.

Given I select a different wallet,
When the UI updates,
Then the wallet detail panel, accounts, addresses, and asset locks all update to reflect the newly selected wallet within 1 second.
```

### Real-Life Scenario: Priya Switches to Masternode Wallet
**Context**: Priya has 4 wallets: "Personal," "Business," "Masternode," and "Cold Storage (SK)." She is currently viewing "Personal."
**Flow**: Priya opens the dropdown, scans for "Masternode" by alias, and clicks it. The wallet screen immediately shows the Masternode wallet's balance, provider key paths, and associated identities.
**Expected Outcome**: Switch is instant. Priya can immediately check masternode key status.
**What Could Go Wrong**: Wallet with same alias name (ambiguous). Solution: show truncated seed hash or address alongside alias.

---

## UC-WM-05: Rename a Wallet

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to rename my wallet so that I can identify it by a meaningful label.

### Acceptance Criteria

```
Given I have a wallet selected,
When I click "Rename" and enter a new name (up to 64 characters),
Then the alias is updated in memory and persisted to the database immediately.

Given the wallet has been renamed,
When I view the wallet selector,
Then the new name appears in the dropdown and detail panel.
```

---

## UC-WM-06: Remove a Wallet

**Personas**: Priya, Jordan

### User Story
As a user, I want to remove a wallet from the app so that I can clean up wallets I no longer need.

### Acceptance Criteria

```
Given I have a wallet selected,
When I click "Remove,"
Then a confirmation dialog explains what will be deleted and what will remain (identities stay on-chain but keys are lost unless re-imported).

Given I confirm the removal,
When the operation completes,
Then the wallet is deleted from the database, removed from the in-memory list, and the next available wallet is selected.
```

### Edge Cases
- Removing the only wallet returns to the "No Wallets Loaded" state.
- Removing a wallet with linked identities warns that identity operations will fail until the wallet is re-imported.
- Single key wallets are removed immediately (no confirmation dialog currently -- this should be added).

---

## UC-WM-07: Lock and Unlock a Wallet

**Personas**: Alex, Priya

### User Story
As a user with a password-protected wallet, I want to lock my wallet when I step away and unlock it when I return so that my private keys are not exposed in memory.

### Acceptance Criteria

```
Given my wallet is unlocked (open),
When I click "Lock,"
Then the seed is zeroized in memory, the wallet transitions to Closed state, and operations requiring the private key (send, view key, add address) are disabled.

Given my wallet is locked,
When I click "Unlock" and enter the correct password,
Then the seed is decrypted and the wallet transitions to Open state.

Given my wallet is locked,
When I attempt an operation that requires the private key (e.g., Refresh, Send),
Then the unlock popup appears automatically before proceeding.
```

---

## UC-WM-08: View Wallet Balance Overview

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to see my wallet balance clearly so that I know how much Dash I have.

### Acceptance Criteria

```
Given I have a wallet selected,
When the wallet screen loads,
Then I see my total balance prominently displayed.

Given my wallet has both Core chain funds and Platform credits,
When I view the balance,
Then the total balance combines both, with an expandable breakdown showing Core and Platform separately.
```

### Persona-Specific Display

| Persona | Balance Display |
|---|---|
| Alex | Single number: "1.2345 DASH" with no distinction between Core and Platform. If expanded, show "On-chain: 1.0000 DASH" and "Platform: 0.2345 DASH." |
| Priya | "Core: 1.0000 DASH | Platform: 0.2345 DASH (234,500,000 credits)" always visible. Per-account breakdown in accounts section. |
| Jordan | Same as Priya, plus raw credit values shown everywhere Platform amounts appear. |

---

## UC-WM-09: Refresh Wallet Data

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to refresh my wallet data so that I see up-to-date balances and transaction history.

### Acceptance Criteria

```
Given I have a wallet selected,
When I click "Refresh,"
Then the app queries Core (via RPC or SPV) for updated balances and queries Platform for updated credit balances.

Given a refresh is in progress,
When I view the wallet screen,
Then a spinner indicates the refresh is active.

Given the refresh completes,
When the results arrive,
Then balances, UTXOs, and Platform data are updated in the UI.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Single "Refresh" button. No need to choose what to refresh. |
| Priya | Granular refresh options (Core Only, Platform Full, Platform Terminal, etc.) available as a dropdown or context menu. |
| Jordan | Same as Priya. Additionally, auto-refresh option when switching networks. |
