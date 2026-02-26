# Manual Test Scenarios: Typed Errors Migration

**Issue:** #660 (Typed Errors Migration)
**Date:** 2026-02-26
**Scope:** Migration from `Result<T, String>` to typed `TaskError` errors in backend tasks.

## Overview

The app is transitioning error handling from generic `TaskResult::Error(String)` to typed `TaskError` with:
- `TaskError` enum with variants including `DashPayError`, `SpvError`, `ConfigError`, etc.
- `From<String>` impl for backwards compatibility (unmigrated domains produce `TaskError::Generic`)
- Display trait for user-friendly messages shown in error banners
- Debug trait for technical details shown in collapsible "details" section
- Connection status detection continues via string matching on error messages

## Preconditions (All Scenarios)

- Dash Evo Tool built from the `refactor/typed-errors-660` branch
- Dash Core node or SPV running and reachable
- At least one wallet loaded (with known mnemonic for unlocking)
- Platform (DAPI) endpoints reachable for the selected network
- Developer mode enabled for enhanced error message inspection
- `RUST_LOG=info` or higher for log visibility (optional, for detailed debugging)

---

## Section 1: Error Banner Display (Happy Path)

### Scenario 1.1: Generic String Error Display

**Goal:** Verify that unmigrated backend domains (producing `TaskError::Generic` via `From<String>`) display user-friendly messages in the error banner.

**Preconditions:**
- A backend task from a domain not yet fully migrated (check git issue for migrated domains)
- Network connectivity intact

**Steps:**
1. Perform an action that triggers a backend task from an unmigrated domain (e.g., an identity operation if Identity domain hasn't been migrated)
2. Introduce an error condition (e.g., invalid input, missing wallet unlock)
3. Observe the error banner at the top of the screen

**Expected Results:**
- Error banner appears with the generic error message (Display output)
- Banner color is red (MessageType::Error)
- Auto-dismiss timer runs (5 seconds default)
- Message is readable and not truncated (text wrapping applied)
- No raw `Debug` formatting visible (e.g., no "TaskError::Generic(...)" prefix)

---

### Scenario 1.2: DashPay Error Display (User-Friendly Message)

**Goal:** Verify that `DashPayError` variants display their user-friendly `user_message()` in the banner, not raw Debug output.

**Preconditions:**
- DashPay domain fully migrated to typed errors
- A wallet with at least one DashPay contact or attempted contact operation
- DAPI available for DashPay queries

**Steps:**
1. Navigate to DashPay > Send Payment
2. Attempt to send credits to a non-existent or invalid recipient identity
3. Observe the error banner

**Expected Results:**
- Error banner displays a user-friendly message like "Recipient identity not found" or similar
- Message is from `DashPayError::user_message()`, not the raw enum variant name
- Banner shows the error in red
- Message does NOT contain Rust struct debug output (no `DashPayError { ... }`)

**Example user-friendly messages:**
- "Recipient not found" instead of "RecipientNotFound"
- "Insufficient balance for payment" instead of "InsufficientBalance"
- "Network unavailable, please retry" instead of "NetworkError"

---

### Scenario 1.3: SPV Error Display

**Goal:** Verify that `SpvError` variants display user-friendly messages when SPV sync or operations fail.

**Preconditions:**
- Backend mode set to SPV (Developer Mode > Backend: SPV)
- SPV sync in progress or configured
- Valid SPV environment (Regtest or testnet recommended)

**Steps:**
1. Enable SPV mode if not already active
2. Trigger SPV sync (Wallets > Refresh > "Platform Only")
3. Intentionally introduce an error (e.g., stop the local node, disconnect network, or wait for a real sync error)
4. Observe error banner

**Expected Results:**
- Error banner appears with user-friendly SPV error message
- Message conveys the issue (e.g., "SPV sync failed", "Unable to connect to peers")
- No raw struct notation or variant names visible
- Banner color is red

---

### Scenario 1.4: Config Error Display

**Goal:** Verify that `ConfigError` variants display user-friendly messages when configuration loading or validation fails.

**Preconditions:**
- `.env` file accessible in the app config directory
- Network configurations available

**Steps:**
1. Corrupt or delete the `.env` file (or a section of it)
2. Restart the application or trigger a network switch
3. Observe any error banners related to config loading

**Expected Results:**
- If a config error occurs, the banner displays a user-friendly message
- Message is NOT the raw Rust struct debug representation
- Example: "Failed to load network configuration" instead of `ConfigError { ... }`
- Application degrades gracefully (uses defaults if applicable)

---

## Section 2: Details Section (Collapsible/Expandable)

### Scenario 2.1: Error Banner Details Expansion

**Goal:** Verify that error banners with technical details show a collapsible "Details" or similar section.

**Preconditions:**
- An error that includes Debug-formatted technical details (e.g., RPC error with full error object)
- UI supports collapsible sections in error banners

**Steps:**
1. Trigger an error with technical details (e.g., RPC communication error)
2. Observe the error banner
3. Look for a "Details" button or expand arrow
4. Click to expand the details section

**Expected Results:**
- Banner shows the user-friendly message (Display)
- A "Details" toggle or expandable section is visible
- Clicking expands to show the Debug-formatted technical information
- Details section is scrollable if content exceeds ~120px height
- Details can be collapsed again

**Example layout:**
```
[Error Banner - Red]
Message: "Failed to connect to RPC: connection timeout"
[▼ Details]
  Details: RpcError { reason: "connection timeout", code: -1, ... }
```

---

### Scenario 2.2: Details Section with Multi-Line Output

**Goal:** Verify that large or multi-line Debug output in the details section is properly formatted and scrollable.

**Preconditions:**
- An error with verbose Debug information (e.g., nested error with multiple fields)
- Details max height is configured (~120px)

**Steps:**
1. Trigger an error with large technical details (e.g., a contract operation error with full trace)
2. Observe the error banner
3. Expand the Details section
4. Verify formatting and scrollability

**Expected Results:**
- Debug output is properly formatted (no line breaks missing)
- If content exceeds max height, a scrollbar appears
- Text is readable in the collapsible section
- Scroll behavior is smooth

---

## Section 3: Connection Status Detection

### Scenario 3.1: RPC Failure Detection from Error Message

**Goal:** Verify that the connection status detector (`connection_status.rs`) correctly identifies RPC failures via string matching and updates the status indicator.

**Preconditions:**
- Backend mode: RPC
- Connection indicator visible in the UI (typically top-right)
- Initial connection state: "Connected to Dash Core Wallet" (green)

**Steps:**
1. Note the connection indicator status (should be green if RPC is online)
2. Disconnect the network or shut down the Dash Core node
3. Trigger a backend task that requires RPC (e.g., refresh wallet)
4. Observe the error banner and connection indicator

**Expected Results:**
- Error banner appears with a user-friendly message
- The exact error message contains the string: "Failed to get best chain lock for mainnet, testnet, devnet, and local"
- Connection indicator changes from green to red (Disconnected state)
- Tooltip on indicator shows "Disconnected from Dash Core Wallet"
- Once RPC is restored, indicator returns to green on the next refresh

---

### Scenario 3.2: DAPI Failure Does Not Affect RPC Status

**Goal:** Verify that DAPI/Platform errors do NOT incorrectly trigger RPC disconnection status.

**Preconditions:**
- Backend mode: RPC
- RPC online and connected (green indicator)
- DAPI endpoints configured

**Steps:**
1. Verify RPC connection is green
2. Disconnect DAPI (stop DAPI nodes, block network traffic, or configure invalid endpoints)
3. Trigger a Platform operation (e.g., refresh Platform address balances)
4. Observe error banner and connection status

**Expected Results:**
- Error banner shows DAPI/Platform error (e.g., "Failed to sync Platform addresses")
- RPC connection indicator remains green (RPC status unchanged)
- String matching logic does NOT match the DAPI error to the RPC "chain lock" message
- When DAPI is restored, Platform operations succeed without affecting RPC indicator

---

### Scenario 3.3: Multiple Error Types in Sequence

**Goal:** Verify that connection status correctly handles mixed error types without confusion.

**Preconditions:**
- Both RPC and DAPI configured
- Initial green connection state

**Steps:**
1. Verify green connection indicator
2. Trigger a DAPI error (disconnect DAPI, attempt Platform sync) — observe banner
3. Then disconnect RPC — trigger another backend task
4. Observe connection status transitions

**Expected Results:**
- After DAPI error: RPC indicator remains green, DAPI shown as unavailable
- After RPC disconnect: Indicator turns red
- String matching is precise: only the exact "chain lock" message triggers RPC status change
- No cross-contamination between DAPI and RPC error detection

---

## Section 4: Backward Compatibility (String-to-TaskError)

### Scenario 4.1: Unmigrated Domain Produces Generic Error

**Goal:** Verify that `From<String>` ensures unmigrated backend domains still work and produce `TaskError::Generic`.

**Preconditions:**
- Identify a backend domain that has NOT yet been migrated (check issue #660 for status)
- Trigger an operation in that domain

**Steps:**
1. Find an unmigrated domain (e.g., if Contested Names not yet migrated, query contested names)
2. Introduce an error condition (bad input, network failure, etc.)
3. Observe the error banner

**Expected Results:**
- Error is displayed correctly in the banner
- No type mismatch errors in build or runtime
- User-friendly message is shown (via Display on TaskError::Generic)
- Backend task completes with error handling (no panic, no crash)

---

### Scenario 4.2: Mixed Migrated and Unmigrated Task Chains

**Goal:** Verify that a task sequence with both migrated and unmigrated domains works correctly.

**Preconditions:**
- A workflow that involves multiple backend domains (e.g., register identity → register DPNS name)
- Some domains migrated, some not

**Steps:**
1. Perform a multi-step operation (e.g., fund wallet → register identity → register DashPay contact)
2. Introduce an error in one step (e.g., invalid identity creation due to bad input)
3. Observe error handling and recovery

**Expected Results:**
- Error is displayed in the banner with a user-friendly message
- Application remains stable
- User can retry or proceed to next step
- No type conversion panics or confusing error messages

---

## Section 5: Error Recovery and State

### Scenario 5.1: Error Does Not Corrupt UI State

**Goal:** Verify that after an error, the UI remains stable and can recover by retrying.

**Preconditions:**
- A successful operation in a domain (e.g., loaded identity, shown balance)
- Error scenario that temporarily fails

**Steps:**
1. Display a screen showing some data (e.g., Wallets screen with balances)
2. Trigger an operation that fails (e.g., bad network, temporarily unavailable service)
3. Observe the error banner
4. Fix the underlying issue (restore network, fix input)
5. Retry the operation

**Expected Results:**
- Error banner displays, UI remains functional
- Previously displayed data is not cleared or corrupted
- Retry succeeds and banner auto-dismisses or user dismisses it
- Data updates correctly after retry

---

### Scenario 5.2: Errors in Modal Dialogs

**Goal:** Verify that errors occurring within modal/detail screens display correctly and don't break modal state.

**Preconditions:**
- A workflow that opens a modal dialog (e.g., Send Payment dialog, Add Contact dialog)
- Error during the operation within the modal

**Steps:**
1. Open a modal dialog (e.g., DashPay > Send Payment)
2. Enter invalid input or trigger an error (e.g., bad recipient, network failure)
3. Submit the form
4. Observe error handling within the modal

**Expected Results:**
- Error banner appears at the top (global level)
- Modal remains open and usable
- User can edit and retry
- Modal can be dismissed (Cancel button works)
- No stale error messages persist after modal is closed and reopened

---

## Section 6: Error Message Clarity

### Scenario 6.1: Verify No Raw Enum Output in User Messages

**Goal:** Confirm that error banners never display raw Rust enum or struct debug output to users.

**Preconditions:**
- Multiple error conditions triggered (from different domains)
- Inspect error banners closely

**Steps:**
1. Trigger 3-5 different error scenarios across domains (DashPay, SPV, Config, etc.)
2. Read each error message carefully
3. Note whether messages are human-readable

**Expected Results:**
- All error messages are human-readable and contextual
- No messages contain:
  - `TaskError::Variant(...)`
  - `DashPayError::VariantName`
  - `{ field: value, ... }` struct debug output
  - Rust type names or generic `<T>` syntax
- Messages are suitable for non-technical end users

---

### Scenario 6.2: Error Messages Include Recovery Hints

**Goal:** Verify that user-friendly error messages suggest recovery actions where applicable.

**Preconditions:**
- Errors that have recoverable states (e.g., wallet locked, network unavailable)

**Steps:**
1. Trigger a wallet-locked error (lock wallet, attempt to refresh)
2. Trigger a network error (disconnect, attempt Platform sync)
3. Read the error messages

**Expected Results:**
- Messages suggest recovery:
  - Wallet error: "Wallet is locked. Please unlock to proceed."
  - Network error: "Network unavailable. Check connection and retry."
  - Or similar guidance appropriate to the error
- Suggestions are in plain language, actionable, and accurate

---

## Section 7: Edge Cases

### Scenario 7.1: Very Long Error Messages

**Goal:** Verify that lengthy error messages are displayed correctly (wrapped, not truncated).

**Preconditions:**
- A domain error with a long message (e.g., detailed validation error, multi-part error)

**Steps:**
1. Trigger an error with a long message (>200 characters)
2. Observe the error banner

**Expected Results:**
- Message is fully displayed (not truncated with "...")
- Text wrapping is applied
- Banner resizes as needed
- Entire message is readable

---

### Scenario 7.2: Rapid Sequential Errors

**Goal:** Verify that multiple errors in quick succession display correctly (not overwritten, properly queued).

**Preconditions:**
- A way to trigger multiple errors quickly (e.g., spam a button, network flakes)
- Message banner supports a queue (up to 5 banners per CLAUDE.md)

**Steps:**
1. Trigger 2-3 errors in rapid succession (within 1 second)
2. Observe the banner area

**Expected Results:**
- All errors are displayed (in a queue if supported)
- OR, the most recent error replaces older ones
- No crashes or lost errors
- UI remains responsive
- Auto-dismiss logic works for all queued messages

---

### Scenario 7.3: Error During Auto-Dismiss

**Goal:** Verify that if a new error arrives while another is auto-dismissing, both are handled correctly.

**Preconditions:**
- An error banner visible and counting down to auto-dismiss
- A way to trigger another error before auto-dismiss completes

**Steps:**
1. Trigger an error — observe the 5-second auto-dismiss countdown
2. After 2-3 seconds, trigger another error
3. Observe the banner behavior

**Expected Results:**
- New error is displayed
- Old error's auto-dismiss is reset or new error replaces it (depending on queue design)
- UI does not flicker or display both simultaneously in a broken way
- Final state is stable and shows the most relevant error

---

### Scenario 7.4: Error with Unicode/Special Characters

**Goal:** Verify that error messages with non-ASCII characters display correctly.

**Preconditions:**
- An error message containing accented characters, emoji, or other Unicode
- Example: a validation error with "café" or "❌" symbol

**Steps:**
1. Trigger an error with Unicode characters (may require crafted test case)
2. Observe the banner

**Expected Results:**
- Characters display correctly (not garbled or replaced with "?")
- Banner layout is not broken by character width
- Text is readable

---

## Section 8: Integration Tests

### Scenario 8.1: Full Wallet Operation with Error Handling

**Goal:** Verify the entire error flow during a realistic user workflow.

**Preconditions:**
- A wallet with some balance
- DAPI and RPC reachable
- Developer mode enabled

**Steps:**
1. Load a wallet
2. Attempt to fund a Platform address (or withdraw, or transfer)
3. Introduce a failure mid-operation (e.g., disconnect network)
4. Observe error banner and recovery
5. Restore network and retry
6. Verify success

**Expected Results:**
- Step 1-2: Operation starts, UI shows progress
- Step 3: Error is caught, banner displays user-friendly message
- Step 4: User can retry
- Step 5: Operation completes successfully on retry
- Step 6: Balance updates correctly, no double-deduction

---

### Scenario 8.2: Network Switching with Errors

**Goal:** Verify that switching networks after an error doesn't leave stale error state.

**Preconditions:**
- Multiple networks configured (Testnet + Mainnet)
- A wallet available on both

**Steps:**
1. Connect to Testnet and trigger an operation
2. Introduce an error (e.g., invalid input)
3. Observe error banner on Testnet
4. Switch to Mainnet
5. Perform a successful operation on Mainnet
6. Observe banners and state

**Expected Results:**
- Step 2-3: Error banner shows on Testnet
- Step 4: Switching networks may clear or replace banner (depending on design)
- Step 5-6: Mainnet operation succeeds with no stale Testnet errors
- Each network's error state is isolated

---

## Checklist for Testers

| Area | Verification |
|------|--------------|
| **Error Display** | Error messages are user-friendly, not raw Rust enum output |
| **Details Section** | Collapsible details show Debug output without breaking layout |
| **RPC Status** | RPC disconnection detected via correct error string match |
| **DAPI Errors** | DAPI errors don't interfere with RPC status detection |
| **Backwards Compat** | Unmigrated domains work via `From<String>` |
| **Recovery Hints** | User-friendly messages suggest recovery actions |
| **UI Stability** | UI remains functional after errors; no state corruption |
| **Auto-dismiss** | Banners auto-dismiss after ~5 seconds (or user dismisses) |
| **Text Wrapping** | Long messages are wrapped, not truncated |
| **Sequential Errors** | Multiple errors handled correctly without loss or flicker |
| **Unicode** | Special characters display correctly |
| **Network Isolation** | Errors don't cross network boundaries (Testnet vs. Mainnet) |

---

## Known Limitations / Areas Out of Scope

1. **Specific error message wording:** Exact phrasing of user-friendly messages is implementation-dependent and not checked in detail here. Focus is on structure (no raw enum output).
2. **Performance of error handling:** Response time to errors is not measured in these scenarios.
3. **Error persistence:** Whether errors are logged to disk or external services is not covered.
4. **Localization:** Error message translation is not tested (assumes English UI).

