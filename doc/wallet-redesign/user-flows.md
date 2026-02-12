# Wallet Screen User Flows

User flow diagrams for all key wallet operations, organized by persona relevance.

---

## 1. First-Time Experience (Alex, Priya, Jordan)

```
Flow: First Launch with No Wallets
Entry Point: User opens the app and navigates to Wallets

  [App Opens]
       |
       v
  [Left Panel: click "Wallets"]
       |
       v
  [Wallet Screen: No-Wallet Empty State]
  "No Wallets Loaded"
  [Create Wallet]  [Import Wallet]
       |                |
       v                v
  [Create Flow]    [Import Flow]
  (see Flow #2)    (see Flow #3)
```

---

## 2. Create Wallet Flow (Alex, Priya, Jordan)

```
Flow: Create a New HD Wallet
Entry Point: "Create Wallet" button (empty state or wallet selector)

  [Click "Create Wallet"]
       |
       v
  [Step 1: Recovery Phrase]
  - Display 12 words in 4x3 grid
  - Word count selector: 12/15/18/21/24 (Priya, Jordan)
  - Language selector (default: English)
  - "I Have Written These Down >>"
       |
       v
  [Step 2: Verify Backup]
  - Enter words #3, #7, #11
  - Jordan: "Skip Verification" option
       |
       v
  [Step 3: Name and Password]
  - Wallet name (auto-generated default)
  - Password (optional)
  - Password strength meter
  - "Create Wallet"
       |
       v
  [Success Screen]
  - "Wallet Created Successfully"
  - Balance: 0.0000 DASH
  - Next actions: [Fund Wallet] [Create Identity]
       |
       v
  [Wallet Screen with new wallet selected]

Error Paths:
  Step 2 verification fails:
    -> "Word X is incorrect. Try again."
    -> User re-enters; can go back to Step 1 to re-view phrase

  Step 3 passwords don't match:
    -> "Passwords do not match." inline error
    -> User corrects and re-submits
```

---

## 3. Import Wallet Flow (Alex, Priya, Jordan)

```
Flow: Import Wallet via Recovery Phrase
Entry Point: "Import Wallet" button

  [Click "Import Wallet"]
       |
       v
  [Import Screen: Tab Selection]
  [Recovery Phrase]  [Private Key]
       |                  |
       v                  v
  [Mnemonic Tab]     [Private Key Tab]
  (see below)        (see Flow #4)
       |
       v
  [Enter Recovery Phrase]
  - 4-column word grid
  - Word count auto-detected
  - BIP39 auto-suggest on typing
  - Paste support (space-separated string fills grid)
  - Jordan: paste entire phrase at once
       |
       v
  [Optional: Password & Alias]
  - Set wallet name
  - Set password (if original wallet had one)
       |
       v
  [Optional: Identity Scan Count] (Priya, Jordan)
  - "Scan for identities: [10] indices"
  - Default: 10, configurable
       |
       v
  [Import]
       |
       v
  [Loading: "Importing wallet and scanning for balances..."]
       |
       v
  [Wallet Screen with imported wallet selected]
  - Balances discovered and displayed
  - Identities discovered and linked

Error Paths:
  Invalid word entered:
    -> Red border on word field, "Not a valid recovery word"

  Invalid phrase (checksum):
    -> "Invalid recovery phrase. Check for typos."

  Network error during scan:
    -> Wallet imported locally, balance shown as "Unknown - refresh to sync"
```

---

## 4. Import Private Key Flow (Priya, Jordan)

```
Flow: Import a Single Private Key
Entry Point: "Import Wallet" > "Private Key" tab

  [Select "Private Key" tab]
       |
       v
  [Enter Private Key]
  - Input field for WIF or hex key
  - Auto-detect format (WIF vs hex)
  - Show derived address preview
       |
       v
  [Set Alias]
  - Default: "Imported Key"
       |
       v
  [Import]
       |
       v
  [Loading: "Importing key and checking balance..."]
       |
       v
  [Wallet Screen with SingleKey wallet selected]

Error Paths:
  Invalid key format:
    -> "Invalid private key. Enter a WIF (starts with 5, K, or L) or hex-encoded key."

  Key already imported:
    -> "This key is already imported as [wallet alias]."
```

---

## 5. Send Dash Flow (Alex, Priya, Jordan)

```
Flow: Send Dash (Core to Core)
Entry Point: "Send" button on wallet screen

  [Click "Send"]
       |
       |-- If wallet locked --> [Unlock Popup] --> success --> continue
       |                                      --> cancel  --> abort
       v
  [Send Screen: Step 1 - Enter Details]
  - From: [Core Wallet v] (source selector)
  - To: [address input field]
  - Amount: [amount input] DASH
  - [x] Subtract fee from amount
  - Available: X.XXXX DASH
       |
       | Address auto-detection:
       |   "X..." or "y..." -> Core address
       |   "evo1..."/"tevo1..." -> Platform address (changes routing)
       |   Text without prefix -> Try DPNS lookup
       |
       v
  [Validation passes: all fields valid]
       |
       v
  [Click "Continue"]
       |
       v
  [Step 2: Confirm Transaction]
  - Summary: To, Amount, Fee, Total, Remaining Balance
  - Priya/Jordan: fee shown in DASH and duffs
  - [Back] [Confirm & Send]
       |
       v
  [Click "Confirm & Send"]
       |
       v
  [Broadcasting...]
  - Spinner, buttons disabled
       |
      / \
  success  failure
    |        |
    v        v
  [Result: OK]     [Result: Failed]
  - Checkmark      - Error icon
  - TxID [Copy]    - Error message
  - Status         - [Try Again] [Back to Wallet]
  - [Back to Wallet]

Cross-Type Sends:
  Core -> Platform address:
    Step 1 auto-detects Platform address
    Source auto-switches or shows both options
    Flow creates asset lock + funds Platform address internally
    Progress: "Creating asset lock... Waiting for proof... Funding..."

  Platform -> Platform:
    Source selector: Platform Address dropdown
    Normal confirm flow

  Platform -> Core:
    Source selector: Platform Address dropdown
    Confirm shows withdrawal processing time warning
```

---

## 6. Receive Dash Flow (Alex, Priya, Jordan)

```
Flow: Receive Dash
Entry Point: "Receive" button on wallet screen

  [Click "Receive"]
       |
       v
  [Receive Dialog (modal)]
  [Core]  [Platform]    <-- tab bar
       |
       v
  [Core Tab - Default]
  - QR code for next unused address
  - Address text below QR
  - [Copy Address] [New Address]
       |
       |-- Click "Copy Address" -> address to clipboard, "Copied!" tooltip
       |-- Click "New Address" -> new address derived, QR updates
       |-- Click QR code -> address to clipboard
       |
       |-- Priya: address selector dropdown above QR
       |-- Jordan: derivation path shown below address
       |
       v
  [Platform Tab] (if Platform addresses exist)
  - QR code for Platform address (evo1.../tevo1...)
  - Same actions as Core tab
       |
       v
  [Close dialog -> return to wallet screen]
```

---

## 7. Refresh Wallet Flow (Alex, Priya, Jordan)

```
Flow: Refresh Wallet Data
Entry Point: "Refresh" button on action bar

  [Click "Refresh"]
       |
       |-- If wallet locked and refresh requires key:
       |   --> [Unlock Popup] --> success --> continue
       |                      --> cancel  --> abort (no refresh)
       |
       v
  [Refresh starts]
  - Refresh button shows spinner
  - Balance shows subtle pulsing indicator
  - Refresh mode: determined by selector (Level 1+) or default "All (Auto)"
       |
       v
  [Backend: RefreshWalletBalance task dispatched]
       |
      / \
  success  failure
    |        |
    v        v
  [Balance updates]         [Error message shown]
  - Spinner stops            - "Failed to refresh: [reason]"
  - Balance flash if changed - [Retry] action in message
  - Tx history updates       - Spinner stops
  - Account balances update  - Previous balance retained
```

---

## 8. Wallet Lock/Unlock Flow (Alex, Priya)

```
Flow: Lock and Unlock Wallet
Entry Point: Overflow menu "..." > Lock/Unlock

  === LOCK ===
  [Overflow Menu > "Lock"]
       |
       v
  [Wallet locks immediately]
  - Seed zeroized in memory
  - Lock icon updates to locked state
  - Send button shows "(locked)" subtitle
  - View Key buttons disabled
  - No confirmation needed (locking is non-destructive)

  === UNLOCK (explicit) ===
  [Overflow Menu > "Unlock"]
       |
       v
  [Unlock Popup]
  - Password field (auto-focused)
  - [Cancel] [Unlock]
       |
      / \
  correct   incorrect
    |          |
    v          v
  [Wallet unlocks]       [Error: "Incorrect password"]
  - Lock icon updates    - Password field cleared
  - All features enabled - User retries

  === UNLOCK (implicit, triggered by operation) ===
  [User clicks Send / View Key / Create Asset Lock / etc.]
       |
       |-- Wallet is locked?
       |     Yes --> [Unlock Popup] --> success --> operation proceeds
       |                            --> cancel  --> operation aborted
       |     No  --> operation proceeds directly
```

---

## 9. Manage Wallet Flow (Priya, Jordan)

```
Flow: Rename Wallet
Entry Point: Overflow menu > "Rename"

  [Overflow Menu > "Rename"]
       |
       v
  [Rename Dialog]
  - "Current name: [alias]"
  - "New name: [input field]"
  - [Cancel] [Save]
       |
       v
  [Save] -> alias updated in memory and database
       |
       v
  [Wallet selector and header update with new name]


Flow: Remove Wallet
Entry Point: Overflow menu > "Remove"

  [Overflow Menu > "Remove Wallet"]
       |
       v
  [Remove Wallet Confirmation Dialog]
  - Explains what is deleted and what is not
  - Lists linked identities (if any)
  - [Show Recovery Phrase] option
  - [Cancel] [Remove Wallet]
       |
       |-- [Show Recovery Phrase]:
       |     If locked -> [Unlock Popup] -> show phrase
       |     If unlocked -> show phrase directly
       |
       v
  [Click "Remove Wallet"]
       |
       v
  [Wallet removed from database and memory]
       |
       |-- Other wallets exist?
       |     Yes --> next wallet auto-selected
       |     No  --> No-Wallet empty state
```

---

## 10. Switch Wallets Flow (Priya, Jordan)

```
Flow: Switch Between Wallets
Entry Point: Wallet selector in top panel

  [Click wallet selector dropdown]
       |
       v
  [Dropdown opens: list of all wallets]
  [HD] Personal           1.2345 DASH
  [HD] Business           5.0000 DASH  [L]
  [SK] Cold Storage       0.8000 DASH
  ---
  + Create Wallet
  + Import Wallet
       |
       v
  [Click a different wallet]
       |
       v
  [Wallet screen updates]
  - Balance header: new wallet's balance
  - Transaction history: reloads for new wallet
  - Accounts section: resets to collapsed
  - Asset locks: reloads for new wallet
  - All within < 1 second (data is in memory)
```

---

## 11. Create Asset Lock Flow (Priya, Jordan)

```
Flow: Create an Asset Lock
Entry Point: "Create Asset Lock" button in Asset Locks section

  [Click "Create Asset Lock"]
       |
       |-- Wallet locked? --> [Unlock Popup]
       |
       v
  [Asset Lock Creation Screen]
  - Purpose: [Registration v] / [Top-up]
  - Amount: [0.5________] DASH
  - Minimum: 1000 credits (shown as DASH equivalent)
  - If Top-up: Identity selector dropdown
       |
       v
  [Click "Create"]
       |
       v
  [Broadcasting...]
  - "Creating asset lock transaction..."
  - "Waiting for instant lock confirmation..."
  - "Retrieving proof..."
       |
      / \
  success  failure
    |        |
    v        v
  [Return to wallet]        [Error message]
  - Asset lock appears      - Reason + suggested action
    in table as usable      - [Try Again] [Cancel]
```

---

## 12. Fund Platform Address Flow (Priya, Jordan)

```
Flow: Fund a Platform Address from Asset Lock
Entry Point: "Fund" button in asset lock table or Platform address section

  [Click "Fund"]
       |
       v
  [Fund Platform Address Dialog]
  - From: [asset lock selector with amount]
  - To: [Platform address selector]
  - Amount: [pre-filled from asset lock]
  - [x] Deduct fees from amount
  - Fee estimate shown
       |
       v
  [Click "Fund"]
       |
       v
  [Processing...]
  - "Creating funding state transition..."
       |
      / \
  success  failure
    |        |
    v        v
  [Dialog closes]          [Error in dialog]
  - Platform address       - Message + retry
    balance updates
  - Asset lock marked
    as used


Flow: Fund Platform Address Directly (without existing asset lock)
Entry Point: "Fund" button on a Platform address (when no asset locks exist)

  [Click "Fund"]
       |
       v
  [Direct Funding Flow]
  - Amount: [___________] DASH
  - Source: Core wallet balance
  - "This will create an asset lock and fund the address automatically."
       |
       v
  [Click "Fund"]
       |
       v
  [Multi-step Progress]
  - "Step 1/3: Creating asset lock..."
  - "Step 2/3: Waiting for proof..."
  - "Step 3/3: Funding Platform address..."
       |
       v
  [Complete - balance updates]
```

---

## 13. View Private Key Flow (Priya, Jordan)

```
Flow: Export Private Key for an Address
Entry Point: "View Key" button in address table

  [Click "View Key" on address row]
       |
       |-- Wallet locked? --> [Unlock Popup]
       |
       v
  [Private Key Dialog]
  - Security warning (always visible)
  - Address: XoRQE8bHjEm...
  - Private Key: ****************************
  - [Show Key]  [Copy Key]
       |
       |-- [Show Key]: reveals WIF string
       |-- [Copy Key]: copies to clipboard, "Copied!" tooltip
       |
       v
  [Close] -> return to wallet screen
```

---

## 14. Transaction History Drill-Down (Priya, Jordan)

```
Flow: View Full Transaction History
Entry Point: "Show All Transactions" link in Recent Transactions

  [Click "Show All Transactions"]
       |
       v
  [Full Transaction Table]
  - All transactions loaded with pagination (25 per page)
  - Sortable columns: Date, Amount, Status
  - Filter: [All v] | Sent | Received | Internal
  - Level 2: [Export CSV]
       |
       |-- Click TxID -> copies to clipboard
       |-- Click column header -> sort by that column
       |-- Click filter -> filter in-place
       |-- Click [Export CSV] -> download CSV file (Level 2)
       |
       v
  [Navigate pages: << 1 2 3 ... >>]
```

---

## 15. Developer: Request Test Dash (Jordan)

```
Flow: Get Test Dash from Faucet
Entry Point: "Get Test Dash" button in action bar (Testnet, Developer Tools enabled)
Preconditions: On Testnet or Devnet, Developer Tools enabled, wallet selected

  [Click "Get Test Dash"]
       |
       v
  [Faucet Request]
  - Uses wallet's next unused receive address
  - "Requesting test Dash from faucet..."
       |
      / \
  success  failure
    |        |
    v        v
  [Success message]        [Error message]
  "Test Dash requested.    "Faucet request failed:
   Funds should arrive     [rate limit / network error].
   within a few seconds.   Try again later."
   Refresh to see them."
       |
       v
  [Auto-refresh after 5 seconds]
```
