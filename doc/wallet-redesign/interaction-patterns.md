# Wallet Screen Interaction Patterns

Detailed specification of states, transitions, error handling, loading states, and user interaction behaviors for the redesigned wallet screen.

---

## 1. Section Expand / Collapse

The content area uses collapsible sections as the primary progressive disclosure mechanism.

### Component: CollapsibleSection

```
States:
  collapsed:  Header visible, content hidden
  expanded:   Header visible, content visible
  loading:    Header visible, spinner in place of content

Transition triggers:
  Click header      -> Toggle collapsed/expanded
  Click expand icon -> Toggle collapsed/expanded

Visual indicators:
  collapsed: ">" arrow icon before section title
  expanded:  "v" arrow icon before section title
  loading:   spinner icon replacing arrow

Persistence:
  Section state is remembered per-session (in-memory).
  When the user returns to the wallet screen within the same session,
  sections that were expanded remain expanded.
  On app restart, all sections revert to their default state
  (as defined in the Information Architecture).

Animation:
  egui does not support smooth CSS-like animations.
  Expand/collapse is instant (single-frame transition).
  No slide or fade effects.
```

### Behavior Rules

1. Clicking anywhere on the section header row toggles the section (not just the arrow icon).
2. Sections can be expanded/collapsed independently. Expanding one does not collapse others.
3. If a section contains async data that has not loaded yet, expanding it shows a centered spinner with text "Loading..." rather than an empty area.

---

## 2. Wallet Selection

### Component: WalletSelector (Top Panel Dropdown)

```
States:
  closed:     Shows current wallet summary (type badge, alias, balance)
  open:       Dropdown list of all wallets + Create/Import actions
  no_wallet:  Shows "Select a Wallet" placeholder text

Transition triggers:
  Click selector          -> Toggle open/closed
  Click wallet in list    -> Select wallet, close dropdown
  Click Create/Import     -> Navigate to respective screen, close dropdown
  Click outside dropdown  -> Close dropdown
  Press Escape            -> Close dropdown

Selected wallet change effects:
  1. Balance header updates immediately
  2. Transaction history reloads (may show spinner briefly)
  3. Accounts section resets to default collapsed state
  4. Asset locks section reloads
  5. All section expand states reset (new wallet = fresh view)
```

### Wallet Types in Selector

```
HD Wallet entry:
  [HD] Wallet Alias              1.2345 DASH
  Badge color: DashColors::DASH_BLUE
  Shows total balance (Core + Platform combined)

SingleKey Wallet entry:
  [SK] Wallet Alias              0.8000 DASH
  Badge color: DashColors::STEEL_BLUE (or similar neutral)
  Shows single address balance

Locked wallet entry:
  [HD] Wallet Alias    [padlock] 1.2345 DASH
  Padlock icon indicates locked state
  Balance is still shown (balance is public data, not key-dependent)
```

---

## 3. Balance Header Interactions

### Expand/Collapse Breakdown

```
Default state:
  Total balance shown as single number
  "Show breakdown" link visible

Expanded state:
  Total balance shown as single number
  Core balance on separate line
  Platform balance on separate line
  "Hide breakdown" link visible

Developer mode additions:
  Platform line shows credits in parentheses
  Unconfirmed balance shown as a third line if non-zero
```

### Balance Update Behavior

```
On refresh start:
  Balance number remains visible (no flash or disappear)
  A subtle spinner or pulsing indicator appears next to the balance
  The Refresh button shows a spinner state

On refresh complete (success):
  Balance updates to new value
  If balance changed, the number briefly highlights:
    Increase: green flash (DashColors::SUCCESS background for 1 second)
    Decrease: red flash (DashColors::ERROR background for 1 second)
    No change: no flash
  Spinner disappears

On refresh complete (error):
  Balance retains previous value
  Error message shown in Zone 3 (below action bar):
    "Failed to refresh: [reason]. [Retry]"
  Message auto-dismisses after 8 seconds
```

---

## 4. Send Flow States

The send flow is a full-screen multi-step process that replaces the current dialog approach.

### State Machine

```
                    +------------+
                    |   Idle     |  (wallet screen, send not active)
                    +-----+------+
                          |
                    Click "Send"
                          |
                    +-----v------+
                    | Enter      |  Step 1: Address, amount, source
                    | Details    |
                    +-----+------+
                          |
                    Click "Continue"
                    (validation passes)
                          |
                    +-----v------+
                    | Confirm    |  Step 2: Summary with fee
                    |            |
                    +-----+------+
                          |
                    Click "Confirm & Send"
                          |
                    +-----v------+
                    | Broadcast- |  Intermediate: Sending...
                    | ing        |  (spinner, buttons disabled)
                    +-----+------+
                         / \
                        /   \
                  success   failure
                      /       \
              +------v-+   +--v--------+
              | Result |   | Error     |
              | (OK)   |   | (Failed)  |
              +--------+   +-----------+
```

### Step 1: Enter Details -- Validation

```
Address field:
  Empty         -> "Enter a Dash address" (placeholder text, no error)
  Valid Core    -> Green checkmark, "Core address" label
  Valid Platform-> Green checkmark, "Platform address" label
  Invalid       -> Red border, "Invalid address format"
  Own address   -> Yellow warning, "This is your own address"

Amount field:
  Empty         -> "Enter amount" (placeholder text)
  Valid         -> No indicator
  Exceeds bal   -> Red border, "Insufficient balance (available: X.XXXX DASH)"
  Below minimum -> Red border, "Minimum amount is 0.00001 DASH"
  Non-numeric   -> Red border, "Enter a valid number"
  "max" keyword -> Fills with maximum spendable amount

Source selector:
  Core Wallet        -> Shows available Core balance
  Platform Addresses -> Shows platform address selector with balances

Continue button:
  Disabled when: address invalid, amount invalid, or amount exceeds balance
  Enabled when: all fields valid
```

### Step 2: Confirm -- Content

```
Summary panel shows:
  - Recipient address (full, not truncated)
  - Amount in DASH
  - Estimated fee in DASH
  - Total deduction (amount + fee, or amount if "subtract fee" checked)
  - Remaining balance after transaction
  - [Dev mode] Fee and amount also shown in duffs

Back button:
  Returns to Step 1 with all fields preserved

Confirm & Send button:
  Triggers wallet unlock popup if wallet is locked
  Then broadcasts the transaction
```

### Broadcasting State

```
During broadcast:
  - Confirm button replaced with spinner and "Sending..."
  - Back button disabled
  - All fields read-only
  - Cancel is not available (transaction is in flight)

Timeout handling:
  If no response within 30 seconds:
  - Show warning: "Transaction is taking longer than expected."
  - Add "Return to Wallet" button (does not cancel the transaction)
```

### Result States

```
Success:
  - Checkmark icon
  - "Transaction Sent" heading
  - Amount sent and recipient address
  - TxID with copy button
  - Status: "Confirmed (InstantSend)" or "Pending (X confirmations)"
  - "Back to Wallet" button

Failure:
  - Error icon
  - "Transaction Failed" heading
  - Error message in plain language:
    - "Insufficient confirmed balance" -> "Your balance has changed. Go back and update the amount."
    - "Network error"                  -> "Could not connect to the network. Check your connection and try again."
    - "Fee estimation failed"          -> "Could not estimate the fee. Try again or set a custom fee."
    - [other]                          -> Show the raw error with "Contact support if this persists."
  - "Try Again" button (returns to Step 1 with fields preserved)
  - "Back to Wallet" button
```

---

## 5. Receive Dialog Interactions

### Tab Switching (Core / Platform)

```
Core tab:
  Shows the next unused BIP44 external address
  QR code for that address
  "Copy Address" and "New Address" buttons

Platform tab:
  Shows the first DIP-17 Platform payment address
  QR code in Bech32m format (tevo1.../evo1...)
  "Copy Address" and "New Address" buttons

Tab switching:
  Instant (no loading). Address data is already in memory.

Level 1 additions:
  Address selector dropdown showing all addresses of the current type
  Selecting an address updates the QR code and displayed address
  Derivation path shown below the address

Platform tab visibility:
  Hidden if the wallet has no Platform addresses (Alex on a fresh wallet)
  Visible once any Platform address exists
```

### QR Code Behavior

```
QR code generation:
  Generated client-side from the address string
  Updates immediately when address changes
  Size: approximately 200x200 pixels (egui-scaled)

QR code click:
  Clicking the QR code copies the address to clipboard
  Shows brief "Copied!" tooltip near the QR code
```

---

## 6. Transaction History Interactions

### Row Interactions

```
Click TxID cell:
  Copies the full TxID to clipboard
  Shows "Copied!" tooltip

Click row (anywhere except TxID):
  [Future] Opens transaction detail view
  [Current] No action (rows are not interactive)

Hover row:
  Subtle background highlight (glass_white or glass_blue)
```

### Status Indicators

```
Confirmed:
  Green checkmark icon
  Tooltip: "Confirmed in block XXXXXX"

Pending:
  Yellow clock icon or spinner
  Text: "X/6 confirmations" (if confirmation count known)
  Tooltip: "Waiting for network confirmation"

InstantSend confirmed:
  Green checkmark with "IS" badge
  Tooltip: "Instantly confirmed via InstantSend"

Failed:
  Red X icon
  Text: "Failed"
  Tooltip: Error reason
```

### Filter (Level 1)

```
Filter dropdown options:
  All (default)
  Sent
  Received
  Internal (change, consolidation)

Filter behavior:
  Filters the visible transaction list in-place
  No reload required (filtering in-memory data)
  Show "No transactions match this filter" if empty
```

---

## 7. Address Table Interactions

### Column Sorting

```
Sortable columns: Address, Balance, UTXOs, Total Received, Type, Index

Click column header:
  First click:  Sort ascending by that column
  Second click: Sort descending
  Third click:  Remove sort (return to default order)

Visual indicator:
  Active sort column header shows arrow:
    Ascending:  "Column ^"
    Descending: "Column v"
```

### Row Actions

```
Copy address:
  Click on the address cell copies it to clipboard
  Show "Copied!" tooltip

View Key button:
  Opens Private Key modal for that address
  If wallet is locked, triggers unlock popup first

Platform address actions:
  [Fund] button -> Opens Fund Platform Address dialog
  [Withdraw] button -> Opens withdrawal flow
  [Transfer] button -> Opens Platform-to-Platform transfer flow
```

### Address Table Columns by Level

```
Level 1 (Priya):
  Address | Balance | UTXOs | Total Received | Type | Actions

Level 2 (Jordan):
  Address | Balance | UTXOs | Total Received | Type | Index | Path | Actions
```

---

## 8. Asset Lock Interactions

### Table Actions

```
View button:
  Navigates to AssetLock detail screen (push onto screen stack)

Fund button:
  Opens Fund Platform Address dialog
  Only enabled when asset lock is usable (has proof, IS lock confirmed)
  Disabled state shows tooltip: "Asset lock is not yet usable"

Create button:
  Navigates to CreateAssetLock screen
  If wallet is locked, triggers unlock popup first

Search for Unused button:
  Triggers a backend scan of wallet transactions
  Shows spinner: "Searching for asset locks..."
  On completion: "Found X unused asset locks" or "No unused asset locks found"
```

### Asset Lock Status Indicators

```
Usable:
  Green checkmark in "Usable" column
  "Fund" button enabled

Not usable (no proof yet):
  Yellow clock icon
  "Fund" button disabled
  Tooltip: "Waiting for proof from Platform"

Not usable (no IS lock):
  Red X in "IS Lock" column
  "Fund" button disabled
  Tooltip: "InstantSend lock not received"
```

---

## 9. Wallet Lock / Unlock Flow

### Triggering Unlock

```
Explicit:
  User clicks "Unlock" in the overflow menu
  -> Unlock popup appears

Implicit (operation requires key):
  User clicks Send, View Key, Create Asset Lock, Refresh, etc.
  -> Unlock popup appears automatically
  -> On successful unlock, the original operation proceeds
  -> On cancel, the operation is aborted (user returned to previous state)
```

### Unlock Popup Behavior

```
Password field:
  Auto-focused when popup appears
  Enter key submits (same as clicking "Unlock")
  Show/hide toggle for password text

Success:
  Popup closes
  Wallet transitions to Open state
  Lock icon in balance header updates to unlocked
  Original operation continues

Failure:
  "Incorrect password. Try again." message shown inline
  Password field is cleared
  Focus returns to password field
  No attempt limit (user can retry indefinitely)

Cancel:
  Popup closes
  Wallet remains locked
  Original operation is aborted
```

### Lock Behavior

```
Explicit lock:
  User clicks "Lock" in overflow menu
  Wallet seed is zeroized in memory immediately
  Lock icon in balance header updates
  Any in-progress operation that requires the key is interrupted
  (Backend tasks already submitted will complete; no new key-dependent ops)

Auto-lock (future feature):
  After N minutes of inactivity (configurable in Settings)
  Same behavior as explicit lock
  Not implemented in initial redesign (marked as future enhancement)
```

---

## 10. Error and Message Patterns

### Message Display

All user-facing messages follow a consistent pattern:

```
Message types:
  Success  -> Green background, checkmark icon
  Info     -> Blue background, info icon
  Warning  -> Yellow/amber background, warning triangle icon
  Error    -> Red background, X icon

Message position:
  Below the action bar (Zone 3), spanning the content area width
  Messages push content down (not overlay)

Auto-dismiss:
  Success messages: 5 seconds
  Info messages: 5 seconds
  Warning messages: 8 seconds
  Error messages: No auto-dismiss (must be manually closed with X button)

Structure:
  [Icon] Message text                              [X close] [Action]
  Example:
  [!] Failed to refresh wallet: connection timeout  [X]      [Retry]
```

### Error Message Language Rules

1. Never show raw Rust error strings to the user.
2. Every error message should explain what happened and suggest a next step.
3. Technical details (error codes, stack traces) are available via a "Show Details" expand within the error message, visible only in Developer mode.

**Error message template:**
```
[What happened]. [What to do next].

Examples:
  "Could not connect to the Dash network. Check your internet connection and try again."
  "Insufficient balance to send 0.5000 DASH. Your available balance is 0.3000 DASH."
  "Asset lock creation failed: transaction was rejected by the network. Ensure you have confirmed UTXOs and try again."
  "Password is incorrect. Try again."
```

---

## 11. Refresh Behavior

### Default Refresh (Level 0)

```
Click "Refresh":
  1. Refresh button shows spinner
  2. Backend task dispatched: RefreshWalletBalance (mode: All)
  3. Balance header shows subtle pulsing indicator
  4. On completion: balance updates, spinner stops
  5. On error: error message shown, spinner stops
```

### Granular Refresh (Level 1)

```
Refresh mode selector (dropdown next to Refresh button):
  All (Auto)               -> Core + Platform (auto-selects full vs terminal)
  Core Only                -> Only Core chain balance and UTXOs
  Platform (Full)          -> Full Platform state sync
  Platform (Terminal)      -> Terminal-only Platform sync (faster)
  Core + Platform (Full)   -> Both Core and Platform full sync
  Core + Platform (Terminal) -> Core + Platform terminal sync

Selecting a mode:
  Saves the selection for the current session
  Next click of "Refresh" uses the selected mode
  Default reverts to "All (Auto)" on app restart

Visual feedback:
  Same spinner behavior as default
  Selected mode shown in the dropdown label
```

### Refresh Disabled State

```
While a refresh is in progress:
  Refresh button is disabled (greyed out, shows spinner)
  Refresh mode selector is disabled
  Tooltip: "Refresh in progress..."
  Another click does nothing

After error:
  Refresh button re-enables immediately
  User can retry
```

---

## 12. Keyboard Shortcuts

```
Global (when wallet screen is focused):
  Ctrl+R    -> Refresh wallet
  Ctrl+S    -> Open Send flow

Within modals:
  Escape    -> Close modal / cancel operation
  Enter     -> Confirm / submit (when a primary action button exists)

Within address/transaction tables:
  Up/Down   -> Navigate rows (future enhancement)
  Ctrl+C    -> Copy selected cell text (future enhancement)
```

Note: egui keyboard handling is limited compared to web frameworks. Shortcuts should be implemented where the framework supports them, but are not critical for the initial redesign.
