# Manual Test Scenarios: Multi-Wallet RPC Support

## Overview

When Dash Core (dash-qt) has multiple wallets loaded, wallet-specific RPC calls fail with error code -19 ("Wallet file not specified"). This feature detects the error on first occurrence, calls `listwallets` RPC to enumerate loaded wallets, and either auto-selects a single wallet or presents a SelectionDialog for the user to choose one. The selected wallet name is persisted in `.env` config as `{PREFIX}_core_wallet_name` and used via the `/wallet/<name>` URL path for all subsequent RPC calls.

---

## MWR-001: Single wallet auto-select

### Preconditions
- Application configured for any network (Testnet, Regtest, etc.) with RPC backend
- Dash Core is running with exactly **one** wallet loaded
- No `{PREFIX}_core_wallet_name` is set in `.env`

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Trigger any wallet-related RPC operation (e.g., refresh wallet balance, send payment)

### Expected Result
- The first RPC call triggers error -19, but the application transparently handles it
- No dialog is shown to the user
- The application calls `listwallets`, discovers exactly one wallet, and auto-selects it
- The original operation completes successfully
- The wallet name is persisted to `.env` as `{PREFIX}_core_wallet_name` (e.g., `TESTNET_core_wallet_name=wallet.dat`)
- All subsequent RPC calls use the `/wallet/<name>` URL path without further prompts

### Notes
- The auto-selection should be invisible to the user -- the operation should appear to succeed on the first attempt

---

## MWR-002: Multiple wallets -- user selects one

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is running with **two or more** wallets loaded (e.g., `wallet.dat`, `savings`, `trading`)
- No `{PREFIX}_core_wallet_name` is set in `.env`

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Trigger any wallet-related RPC operation
4. Observe the SelectionDialog that appears
5. Open the ComboBox dropdown and review the listed wallet names
6. Select a wallet (e.g., `savings`)
7. Click the "Confirm" button

### Expected Result
- Step 4: A modal overlay appears with a SelectionDialog titled with wallet selection instructions
- Step 5: The ComboBox lists all wallet names returned by `listwallets` (e.g., `wallet.dat`, `savings`, `trading`)
- Step 7: The dialog closes, and the original RPC operation retries and succeeds using the selected wallet
- The selected wallet name is persisted to `.env` as `{PREFIX}_core_wallet_name=savings`
- All subsequent RPC calls use `/wallet/savings` URL path
- The selection persists across application restarts

### Notes
- Verify the wallet list matches the output of `dash-cli listwallets` on the same Core instance

---

## MWR-003: Multiple wallets -- user cancels selection

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is running with two or more wallets loaded
- No `{PREFIX}_core_wallet_name` is set in `.env`

### Steps
1. Trigger a wallet-related RPC operation
2. Observe the SelectionDialog
3. Click the "Cancel" button

### Expected Result
- The dialog closes
- An info banner appears with instructions to manually configure the wallet name in `.env` (e.g., "Set {PREFIX}_core_wallet_name in your .env configuration file")
- The original RPC operation fails gracefully -- no crash or hang
- No wallet name is persisted to `.env`
- Subsequent RPC operations will continue to trigger the -19 error and re-prompt the dialog

---

## MWR-004: Dash Core not running

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is **not running** or RPC endpoint is unreachable
- No `{PREFIX}_core_wallet_name` is set in `.env`

### Steps
1. Launch the application
2. Attempt a wallet-related RPC operation

### Expected Result
- A normal connection error is displayed (e.g., "Could not connect to Dash Core")
- The SelectionDialog is **not** triggered -- the wallet selection flow only activates on error code -19, not on connection failures
- No wallet name is persisted to `.env`

### Notes
- This verifies that the feature specifically handles error -19 and does not interfere with other RPC error paths

---

## MWR-005: Pre-configured wallet name in .env

### Preconditions
- Application configured for Testnet with RPC backend
- `.env` contains `TESTNET_core_wallet_name=savings`
- Dash Core is running with the `savings` wallet loaded

### Steps
1. Launch the application
2. Navigate to the Wallets screen
3. Trigger a wallet-related RPC operation

### Expected Result
- The RPC call uses `/wallet/savings` URL path from the start
- No error -19 occurs
- No SelectionDialog is shown
- The operation completes successfully on the first attempt

---

## MWR-006: Configured wallet removed from Core

### Preconditions
- Application configured for Testnet with RPC backend
- `.env` contains `TESTNET_core_wallet_name=old_wallet`
- Dash Core is running but `old_wallet` is **no longer loaded** (it was unloaded or removed)
- At least one other wallet is loaded in Core

### Steps
1. Launch the application
2. Trigger a wallet-related RPC operation

### Expected Result
- The RPC call to `/wallet/old_wallet` fails (likely with an error indicating the wallet is not found)
- The application detects the failure and re-triggers the wallet detection flow
- If exactly one wallet remains loaded, it auto-selects (MWR-001 behavior)
- If multiple wallets remain, the SelectionDialog appears (MWR-002 behavior)
- The `.env` is updated with the newly selected wallet name, replacing `old_wallet`

### Notes
- This scenario tests recovery from stale configuration

---

## MWR-007: SelectionDialog UI behavior

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is running with two or more wallets loaded
- No `{PREFIX}_core_wallet_name` is set in `.env`
- A wallet-related RPC operation has been triggered, causing the SelectionDialog to appear

### Steps
1. Observe the dialog overlay -- verify it dims or blocks the background UI
2. Observe the dialog contents: title/instructions, ComboBox, Confirm button, Cancel button
3. Open the ComboBox dropdown and verify all loaded wallet names are listed
4. Select a wallet from the dropdown
5. Press the Escape key on the keyboard
6. Re-trigger the dialog (perform another RPC operation)
7. Click the X (close) button on the dialog (if present)
8. Re-trigger the dialog again
9. Select a wallet and click "Confirm"

### Expected Result
- Step 1: A semi-transparent overlay covers the main UI; only the dialog is interactive
- Step 2: The dialog contains clear instructions, a ComboBox dropdown, and Confirm/Cancel buttons
- Step 3: All wallet names from `listwallets` are shown in the ComboBox
- Step 5: Pressing Escape closes the dialog (equivalent to Cancel) -- same info banner as MWR-003
- Step 7: Clicking X closes the dialog (equivalent to Cancel) -- same info banner as MWR-003
- Step 9: Selecting and confirming works normally (same as MWR-002)

### Notes
- Verify that keyboard navigation works within the dialog (Tab between elements, Enter to confirm)

---

## MWR-008: Network switch -- independent wallet selection per network

### Preconditions
- Application configured with both Testnet and Mainnet RPC backends
- Testnet Core has wallets: `testnet_wallet_1`, `testnet_wallet_2`
- Mainnet Core has wallets: `mainnet_wallet_A`, `mainnet_wallet_B`
- No `{PREFIX}_core_wallet_name` is set in `.env` for either network

### Steps
1. Connect to Testnet
2. Trigger a wallet-related RPC operation
3. In the SelectionDialog, select `testnet_wallet_1` and confirm
4. Switch to Mainnet
5. Trigger a wallet-related RPC operation
6. In the SelectionDialog, select `mainnet_wallet_A` and confirm
7. Switch back to Testnet
8. Trigger a wallet-related RPC operation

### Expected Result
- Step 3: `TESTNET_core_wallet_name=testnet_wallet_1` is persisted to `.env`
- Step 5: A new SelectionDialog appears listing Mainnet wallets (not Testnet wallets)
- Step 6: `MAINNET_core_wallet_name=mainnet_wallet_A` is persisted to `.env`
- Step 7-8: No dialog appears; Testnet continues using `testnet_wallet_1` from the persisted config
- Both `TESTNET_core_wallet_name` and `MAINNET_core_wallet_name` coexist in `.env`

### Notes
- Each network prefix (`MAINNET_`, `TESTNET_`, `DEVNET_`, `LOCAL_`) has its own independent wallet name setting

---

## MWR-009: Wallet name with special characters

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is running with wallets that have special characters in their names:
  - `my wallet.dat` (space and dot)
  - `test-wallet_2` (hyphen and underscore)
  - `wallet (backup)` (parentheses and space)
- No `{PREFIX}_core_wallet_name` is set in `.env`

### Steps
1. Trigger a wallet-related RPC operation
2. Observe the SelectionDialog ComboBox
3. Select `my wallet.dat`
4. Click "Confirm"
5. Verify the RPC call succeeds
6. Restart the application
7. Trigger another wallet-related RPC operation

### Expected Result
- Step 2: All wallet names including those with special characters are displayed correctly in the ComboBox
- Step 4: The wallet name is persisted to `.env` as `{PREFIX}_core_wallet_name=my wallet.dat`
- Step 5: The RPC URL path correctly encodes the wallet name (e.g., `/wallet/my%20wallet.dat` or however the RPC client handles URL encoding)
- Step 7: After restart, the persisted wallet name with special characters is read correctly from `.env` and used without re-prompting

### Notes
- URL encoding of the wallet name in the RPC path is critical -- spaces, dots, and other special characters must be handled correctly
- Verify no corruption occurs when writing/reading special characters to/from the `.env` file

---

## MWR-010: Error -19 only triggers once per session (after successful selection)

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is running with multiple wallets loaded
- No `{PREFIX}_core_wallet_name` is set in `.env`

### Steps
1. Trigger a wallet-related RPC operation (first time)
2. Select a wallet in the SelectionDialog and confirm
3. Trigger multiple different wallet-related RPC operations (refresh balance, list UTXOs, send payment)

### Expected Result
- Step 1-2: The SelectionDialog appears once and the user selects a wallet
- Step 3: All subsequent RPC operations succeed without showing the dialog again
- The `/wallet/<name>` path is consistently applied to all RPC calls for the rest of the session

### Notes
- This verifies that the wallet selection is cached in-memory (not just persisted) so that every RPC call in the session benefits from the selection without re-reading `.env`

---

## MWR-011: Concurrent RPC calls during wallet detection

### Preconditions
- Application configured for any network with RPC backend
- Dash Core is running with multiple wallets loaded
- No `{PREFIX}_core_wallet_name` is set in `.env`
- Multiple background tasks may trigger RPC calls simultaneously (e.g., wallet refresh + balance check)

### Steps
1. Trigger multiple wallet-related operations in quick succession (e.g., refresh wallet info while another operation is pending)

### Expected Result
- Only one SelectionDialog appears (not multiple stacked dialogs)
- The first -19 error triggers the detection; subsequent -19 errors are queued or suppressed until the wallet is selected
- After selection, all pending operations retry successfully

### Notes
- This is an edge case around race conditions -- the implementation should serialize or deduplicate the wallet detection flow
