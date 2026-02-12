# Use Cases: Platform Operations

These use cases cover operations that bridge the Core chain (L1) and Dash Platform (L2), including asset locks, Platform address management, and identity-related wallet operations.

## UC-PO-01: Create an Asset Lock

**Personas**: Priya, Jordan

### User Story
As a user who needs to fund a Platform identity or Platform address, I want to create an asset lock so that Core chain funds become usable on Platform.

### Acceptance Criteria

```
Given I have a wallet with sufficient Core balance,
When I navigate to the Asset Locks section and click "Create Asset Lock,"
Then I am taken to the asset lock creation screen where I can specify an amount.

Given I specify a valid amount (minimum 1000 credits / 0.00000001 DASH),
When I confirm the creation,
Then the app broadcasts the asset lock transaction, waits for an InstantLock confirmation, and the asset lock appears in my list as "Usable: Yes" once the proof is available.
```

### Real-Life Scenario: Priya Creates Asset Lock for New Identity
**Context**: Priya wants to register a new identity to use for DPNS name registration.
**Flow**: Opens wallet, scrolls to Asset Locks section. Clicks "Create Asset Lock." Enters 0.5 DASH. Selects purpose: "Identity Registration." The app derives the correct funding address, creates the asset lock transaction, and broadcasts it. Within seconds, the IS lock is confirmed and the proof becomes available. Priya can now use this asset lock to register an identity.
**Expected Outcome**: Asset lock created and usable in under 30 seconds.
**What Could Go Wrong**:
- Insufficient balance -- error shown before broadcast.
- Core node not connected -- error with guidance to check network settings.
- IS lock not received (network issue) -- asset lock shows as "InstantLock: No" with guidance to wait or retry.

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Priya | Full control over amount. Can choose purpose (Registration, Top-up). Sees the funding address and transaction details. |
| Jordan | Same as Priya. Quick-create option: "Create 0.5 DASH asset lock" in one click. Bulk create option: "Create N asset locks of X DASH each." |

---

## UC-PO-02: Fund a Platform Address from an Asset Lock

**Personas**: Priya, Jordan

### User Story
As a user with an unused asset lock, I want to fund a Platform address with it so that I can hold credits on Platform.

### Acceptance Criteria

```
Given I have an unused asset lock with a valid proof,
When I click "Fund" next to it and select a Platform address,
Then the app creates an AddressFundingFromAssetLock state transition and broadcasts it.

Given the funding succeeds,
When I view my Platform address balance,
Then it reflects the newly added credits (minus Platform fees).
```

---

## UC-PO-03: Fund a Platform Address from Wallet UTXOs (Direct Funding)

**Personas**: Priya, Jordan

### User Story
As a user, I want to fund a Platform address directly from my wallet balance without manually creating an asset lock first so that the process is streamlined.

### Acceptance Criteria

```
Given I have sufficient Core balance,
When I select a Platform address and specify an amount to fund,
Then the app automatically creates an asset lock, waits for the proof, and funds the Platform address in a single end-to-end flow.

Given the fee deduction option is set to "from output,"
When the operation completes,
Then the Platform address receives the specified amount minus fees.
```

### Real-Life Scenario: Jordan Funds a Test Platform Address
**Context**: Jordan needs credits on a Platform address to test a dApp contract deployment.
**Flow**: Opens wallet, goes to the Platform Account section. Clicks "Fund" on an address. Enters 0.2 DASH. Checks "Deduct fees from amount." Confirms. The app handles the asset lock creation and Platform funding automatically. Within a minute, the Platform address shows 0.2 DASH in credits (minus fees).
**Expected Outcome**: Platform address is funded without Jordan needing to manage asset locks manually.
**What Could Go Wrong**: The asset lock proof takes too long (network congestion). The app should show progress: "Creating asset lock... Waiting for proof... Funding Platform address..."

---

## UC-PO-04: Transfer Credits Between Platform Addresses

**Personas**: Priya, Jordan

### User Story
As a user with multiple Platform addresses, I want to transfer credits between them so that I can consolidate or redistribute funds.

### Acceptance Criteria

```
Given I have Platform addresses with balances,
When I initiate a transfer specifying source and destination addresses and amounts,
Then the app creates an AddressFundsTransfer state transition.

Given the transfer succeeds,
When I view both addresses,
Then the source balance is reduced and the destination balance is increased (minus fees).
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Priya | Able to select multiple source addresses to consolidate. Fee estimate shown before confirmation. |
| Jordan | Same as Priya. Batch transfer: redistribute credits from one address to multiple destinations in a single operation. |

---

## UC-PO-05: Withdraw from Platform Address to Core

**Personas**: Priya, Jordan

### User Story
As a user with credits on a Platform address, I want to withdraw them back to the Core chain so that I can spend them as regular Dash.

### Acceptance Criteria

```
Given I have a Platform address with a credit balance,
When I initiate a withdrawal specifying a Core address destination,
Then the app creates an AddressCreditWithdrawal state transition.

Given the withdrawal is processed,
When I check my Core balance after the withdrawal period,
Then the withdrawn amount appears in my Core wallet (minus fees and withdrawal processing time).
```

### Important Note
Withdrawals from Platform to Core are not instant. The user must be informed that there is a processing delay (typically a few minutes to complete on-chain).

---

## UC-PO-06: Search for Unused Asset Locks

**Personas**: Priya, Jordan

### User Story
As a user who may have asset locks created by other tools or lost in app state, I want to scan my wallet for untracked asset locks so that I can recover and use them.

### Acceptance Criteria

```
Given I have a wallet that may contain untracked asset locks,
When I click "Search for Unused" in the Asset Locks section,
Then the app scans Core wallet transactions for asset lock outputs that are not currently tracked.

Given unused asset locks are found,
When the scan completes,
Then the found locks are added to the list with a count and total amount message.
```

---

## UC-PO-07: View Asset Lock Details

**Personas**: Priya, Jordan

### User Story
As a user, I want to inspect the full details of an asset lock so that I can verify its transaction ID, address, amount, InstantLock status, and proof availability.

### Acceptance Criteria

```
Given I have asset locks in my list,
When I click "View" on an asset lock,
Then I am taken to a detail screen showing: full TxID, funding address, amount in duffs and DASH, InstantLock status, proof availability, and raw transaction data.
```

---

## UC-PO-08: Manage Account Categories

**Personas**: Priya

### User Story
As a power user, I want to browse my wallet's addresses organized by account category so that I can understand the purpose and balance of each address type.

### Acceptance Criteria

```
Given I have a wallet with multiple account types,
When I view the Accounts section,
Then I see a dropdown listing all account categories: Main Account, Platform Account, Legacy BIP32, CoinJoin, Identity Registration, Identity System, Identity Top-up, Provider Voting, Provider Owner, Provider Operator, Provider Platform.

Given I select an account category,
When the address table updates,
Then only addresses belonging to that category are shown, with balance, UTXOs, total received, type, index, derivation path, and private key export.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Does NOT see account categories. The "Main Account" is the only relevant concept, and it should be implicit. |
| Priya | Full account category dropdown. Descriptions shown for each category (as currently implemented). |
| Jordan | Same as Priya. Ability to filter by "has balance" or "has activity" to focus on relevant addresses. |
