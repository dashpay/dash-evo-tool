# Use Cases: Send and Receive

## UC-SR-01: Send Dash to a Core Address

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to send Dash to another person's address so that I can make a payment.

### Acceptance Criteria

```
Given I have a wallet with a positive balance,
When I click "Send" and enter a valid Dash address and an amount,
Then a confirmation screen shows the recipient address, amount, estimated fee, and total deduction.

Given I confirm the send,
When the transaction is broadcast,
Then I see a success message with the TxID, and my balance updates to reflect the deduction.

Given I enter an amount greater than my confirmed balance,
When I try to send,
Then I see an error "Insufficient confirmed balance" before the transaction is attempted.
```

### Real-Life Scenario: Alex Pays a Freelance Client
**Context**: Alex received a request from a client to send 0.5 DASH to `XoRQE8bHjEm...`. Alex's wallet has 1.2 DASH.
**Flow**: Alex opens the wallet screen, clicks "Send." Types the address (or pastes it). Enters "0.5" in the amount field. Leaves "Subtract fee from amount" unchecked. Clicks "Send." A confirmation shows: "Send 0.5000 DASH to XoRQE8b... | Fee: ~0.0001 DASH | Total: 0.5001 DASH." Alex confirms. Transaction broadcasts. Alex sees "Sent 0.5000 DASH - TxID: abc123..."
**Expected Outcome**: Transaction confirmed within seconds (InstantSend).
**What Could Go Wrong**:
- Address is for wrong network (testnet address on mainnet) -- app should reject with clear error.
- Network connection lost during broadcast -- app should retry or show "Transaction not broadcast, try again."
- Clipboard contains extra whitespace around address -- app should trim automatically.

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Simple dialog: address, amount, send. Optional memo. Fee shown but not configurable. |
| Priya | Fee control: option to set a custom fee or choose from Low/Medium/High. Coin control: select specific UTXOs to spend. |
| Jordan | Same as Priya. Amount field should accept "all" or "max" keyword. Show fee in both DASH and duffs. |

---

## UC-SR-02: Send Dash from a Single Key Wallet

**Personas**: Priya, Jordan

### User Story
As a user with a single key wallet, I want to send Dash from it so that I can move funds without importing the key into an HD wallet.

### Acceptance Criteria

```
Given I have a single key wallet selected with UTXOs,
When I click "Send" and enter a valid address and amount,
Then the app constructs a transaction spending from the single key wallet's UTXOs.

Given the single key wallet is locked,
When I attempt to send,
Then the unlock dialog appears before proceeding.
```

---

## UC-SR-03: Receive Dash on a Core Address

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to get a receiving address with a QR code so that someone can send me Dash.

### Acceptance Criteria

```
Given I have a wallet selected,
When I click "Receive,"
Then a dialog shows my next unused BIP44 external address with a QR code.

Given I want to share the address,
When I click "Copy Address,"
Then the address is copied to the clipboard.

Given I need a fresh address,
When I click "New Address,"
Then a new BIP44 external address is generated and displayed.
```

### Persona-Specific Notes

| Persona | Expectation |
|---|---|
| Alex | Show exactly one address with a QR code. "Copy" button prominent. No address selector unless the user explicitly requests a new address. |
| Priya | Address selector showing all existing external addresses with balances. Ability to generate new addresses. Both Core and Platform tabs. |
| Jordan | Same as Priya. Additionally, show the derivation path next to each address for scripting reference. |

### Real-Life Scenario: Alex Shares Address with Client
**Context**: A client wants to pay Alex 2 DASH for a design project.
**Flow**: Alex opens wallet, clicks "Receive." The dialog shows a QR code and the address `XoRQE8bHjEm...`. Alex clicks "Copy Address" and pastes it into the chat with the client. The client scans the QR code on their mobile wallet. Payment arrives in under 2 seconds via InstantSend.
**Expected Outcome**: Funds appear in Alex's balance immediately.
**What Could Go Wrong**: Alex shares an address that already received funds. This is functionally fine but reduces privacy. The app should default to the next unused address.

---

## UC-SR-04: Receive Credits on a Platform Address

**Personas**: Priya, Jordan

### User Story
As a user with Platform addresses, I want to receive credits on a Platform (DIP-17) address so that I can hold funds on Platform independently of identities.

### Acceptance Criteria

```
Given I click "Receive" and select the "Platform" tab,
When the dialog opens,
Then I see my Platform address in Bech32m format (evo1.../tevo1...) with a QR code.

Given I want a new Platform address,
When I click "New Address,"
Then a new DIP-17 Platform payment address is derived and displayed.
```

---

## UC-SR-05: View Transaction History

**Personas**: Alex, Priya, Jordan

### User Story
As a user, I want to see a list of past transactions so that I can track my payment history.

### Acceptance Criteria

```
Given I have a wallet with past transactions,
When I view the wallet screen,
Then I see a transaction list showing: date, type (Sent/Received/Internal), amount, status (Confirmed/Pending), and TxID.

Given I want to verify a specific transaction,
When I click on a transaction's TxID,
Then the TxID is copied to the clipboard (or opens in a block explorer if configured).
```

### Critical Note
Transaction history is currently hidden behind "developer mode." This is incorrect -- transaction history is a fundamental wallet feature that all personas need. It should be visible by default for all users.

### Persona-Specific Display

| Persona | Transaction History Display |
|---|---|
| Alex | Simple list: date, sent/received arrow, amount, confirmation check mark. No TxID unless expanded. |
| Priya | Full table: date, type, amount, fee, status with block height, full TxID with copy button. Sortable columns. |
| Jordan | Same as Priya. Filter by transaction type. Export to CSV. |

---

## UC-SR-06: Send to Multiple Recipients (Batch Send)

**Personas**: Priya

### User Story
As a power user managing payroll or distributions, I want to send Dash to multiple addresses in a single transaction so that I save on fees and time.

### Acceptance Criteria

```
Given I am on the send screen,
When I add multiple recipient rows (address + amount),
Then the app constructs a single transaction with multiple outputs.

Given the total amount exceeds my balance,
When I attempt to send,
Then I see an error showing the shortfall.
```

### Current Status
Not implemented. The current send dialog supports only a single recipient. This is a future enhancement that should be considered in the redesign.
