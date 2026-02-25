# Manual Test Scenarios: Mine Blocks Dialog (PR #638)

## Overview

PR #638 adds a "Mine" button to the wallet toolbar that opens a dialog for mining blocks to a selected wallet address. The feature is restricted to developer mode on Regtest/Devnet networks with RPC backend.

---

## Scenario 1: Mine button visibility -- Regtest + Developer Mode + RPC

### Preconditions
- Application connected to a **Regtest** network
- **Developer mode** enabled (Settings)
- Core backend mode set to **RPC** (not SPV)
- At least one wallet loaded

### Steps
1. Navigate to the Wallets screen
2. Select a wallet from the wallet list
3. Observe the toolbar buttons (Send, Receive, etc.)

### Expected Results
- A "Mine" button is visible in the toolbar, alongside Send and Receive buttons

---

## Scenario 2: Mine button visibility -- Devnet + Developer Mode + RPC

### Preconditions
- Application connected to a **Devnet** network
- **Developer mode** enabled
- Core backend mode set to **RPC**
- At least one wallet loaded

### Steps
1. Navigate to the Wallets screen
2. Select a wallet from the wallet list
3. Observe the toolbar buttons

### Expected Results
- The "Mine" button is visible in the toolbar

---

## Scenario 3: Mine button hidden -- Mainnet

### Preconditions
- Application connected to **Mainnet**
- Developer mode enabled
- Core backend mode set to RPC

### Steps
1. Navigate to the Wallets screen
2. Select a wallet
3. Observe the toolbar buttons

### Expected Results
- The "Mine" button is **not** visible

---

## Scenario 4: Mine button hidden -- Testnet

### Preconditions
- Application connected to **Testnet**
- Developer mode enabled
- Core backend mode set to RPC

### Steps
1. Navigate to the Wallets screen
2. Select a wallet
3. Observe the toolbar buttons

### Expected Results
- The "Mine" button is **not** visible

---

## Scenario 5: Mine button hidden -- Developer mode disabled

### Preconditions
- Application connected to **Regtest** network
- **Developer mode disabled**
- Core backend mode set to RPC

### Steps
1. Navigate to the Wallets screen
2. Select a wallet
3. Observe the toolbar buttons

### Expected Results
- The "Mine" button is **not** visible

---

## Scenario 6: Mine button hidden -- SPV backend mode

### Preconditions
- Application connected to **Regtest** network
- Developer mode enabled
- Core backend mode set to **SPV** (not RPC)

### Steps
1. Navigate to the Wallets screen
2. Select a wallet
3. Observe the toolbar buttons

### Expected Results
- The "Mine" button is **not** visible (mining requires RPC `generate_to_address`)

---

## Scenario 7: Open and close Mine dialog

### Preconditions
- Mine button is visible (Regtest + dev mode + RPC)
- A wallet is selected

### Steps
1. Click the "Mine" button in the toolbar
2. Observe the dialog that appears
3. Click "Cancel"

### Expected Results
- Step 2: A modal dialog titled "Mine Blocks" appears with:
  - A label "Mine blocks to a wallet address:"
  - An address dropdown selector showing wallet addresses with balances
  - A "Number of blocks:" text input defaulting to "1"
  - "Cancel" and "Mine" buttons
- Step 3: The dialog closes and no mining occurs

---

## Scenario 8: Close dialog via window close button

### Preconditions
- Mine dialog is open

### Steps
1. Click the X (close) button on the dialog window

### Expected Results
- The dialog closes, state is reset, no mining occurs

---

## Scenario 9: Mine a single block successfully

### Preconditions
- Mine button is visible (Regtest + dev mode + RPC)
- A wallet is selected with at least one core address
- Dash Core is running and accepting RPC commands

### Steps
1. Click "Mine"
2. Leave block count as "1"
3. Observe the selected address in the dropdown
4. Click "Mine" button in the dialog

### Expected Results
- The dialog closes
- A success banner appears: "Mined 1 block(s)"
- The wallet balance updates to reflect the newly mined coinbase reward

---

## Scenario 10: Mine multiple blocks

### Preconditions
- Same as Scenario 9

### Steps
1. Click "Mine"
2. Change block count to "10"
3. Click "Mine" button in the dialog

### Expected Results
- The dialog closes
- A success banner appears: "Mined 10 block(s)"
- The wallet balance increases (note: coinbase outputs require 100 confirmations to be spendable on Regtest)

---

## Scenario 11: Address selector shows multiple addresses

### Preconditions
- Selected wallet has multiple BIP44 external (core receive) addresses
- Mine button is visible

### Steps
1. Click "Mine"
2. Open the address dropdown selector
3. Observe the listed addresses

### Expected Results
- All BIP44 external addresses from the wallet are listed
- Each address shows a truncated address string (first 12 chars) and balance in DASH format (e.g., "yXaBcDeFgHiJ... (0.0000 DASH)")
- The first address is selected by default

---

## Scenario 12: Select a different address for mining

### Preconditions
- Wallet has multiple core addresses

### Steps
1. Open the Mine dialog
2. Open the address dropdown
3. Select a different address than the default
4. Enter block count "1"
5. Click "Mine"

### Expected Results
- Mining reward is sent to the newly selected address
- Success message appears
- Balance for the selected address increases after sufficient confirmations

---

## Scenario 13: Invalid block count -- zero

### Preconditions
- Mine dialog is open

### Steps
1. Clear the block count field
2. Enter "0"
3. Click "Mine"

### Expected Results
- An error message appears in the dialog: "Enter a valid number of blocks (> 0)"
- The dialog remains open; no mining occurs

---

## Scenario 14: Invalid block count -- negative number

### Preconditions
- Mine dialog is open

### Steps
1. Clear the block count field
2. Enter "-5"
3. Click "Mine"

### Expected Results
- An error message appears: "Enter a valid number of blocks (> 0)"
- The dialog remains open

---

## Scenario 15: Invalid block count -- non-numeric input

### Preconditions
- Mine dialog is open

### Steps
1. Clear the block count field
2. Enter "abc"
3. Click "Mine"

### Expected Results
- An error message appears: "Enter a valid number of blocks (> 0)"
- The dialog remains open

---

## Scenario 16: Invalid block count -- empty field

### Preconditions
- Mine dialog is open

### Steps
1. Clear the block count field entirely (empty string)
2. Click "Mine"

### Expected Results
- An error message appears: "Enter a valid number of blocks (> 0)"
- The dialog remains open

---

## Scenario 17: Invalid block count -- decimal number

### Preconditions
- Mine dialog is open

### Steps
1. Enter "1.5" in the block count field
2. Click "Mine"

### Expected Results
- An error message appears: "Enter a valid number of blocks (> 0)"
- The dialog remains open (the field parses as u64, so decimals fail)

---

## Scenario 18: Wallet with no existing core addresses

### Preconditions
- A newly created wallet with no BIP44 external addresses yet
- Mine button is visible

### Steps
1. Select the new wallet
2. Click "Mine"

### Expected Results
- The dialog opens
- A new core receive address is automatically generated for the wallet
- The new address appears in the address dropdown
- Mining can proceed normally

---

## Scenario 19: No wallet selected

### Preconditions
- Mine button is visible
- No wallet is currently selected

### Steps
1. Click "Mine"

### Expected Results
- The dialog opens with an error message: "Select a wallet first"
- No addresses are available in the dropdown

---

## Scenario 20: RPC connection failure during mining

### Preconditions
- Mine dialog is open with valid inputs
- Dash Core RPC becomes unreachable (e.g., Core node stopped)

### Steps
1. Stop Dash Core (or disconnect RPC)
2. Enter block count "1" in the Mine dialog
3. Click "Mine"

### Expected Results
- The dialog closes
- An error banner appears with an RPC error message
- No blocks are mined; wallet balance unchanged

---

## Scenario 21: Wallet balance refreshes after mining

### Preconditions
- Mine button is visible, wallet selected, Dash Core running

### Steps
1. Note the current wallet balance
2. Open the Mine dialog
3. Mine 1 block
4. Observe the wallet balance on the Wallets screen

### Expected Results
- After the success message, the wallet balance is refreshed automatically
- The balance reflects the new coinbase reward (the backend calls `refresh_wallet_info` after mining)

---

## Scenario 22: Very large block count

### Preconditions
- Mine dialog is open

### Steps
1. Enter a very large number (e.g., "999999999999999999")
2. Click "Mine"

### Expected Results
- If the value fits in u64, the RPC call is made (Dash Core may take a long time or return an error depending on its limits)
- If it overflows u64, validation error is shown: "Enter a valid number of blocks (> 0)"

---

## Scenario 23: Toggle developer mode while on Wallets screen

### Preconditions
- Application on Regtest with RPC backend
- Developer mode enabled, Mine button visible

### Steps
1. Observe the Mine button is present
2. Navigate to Settings and disable developer mode
3. Return to the Wallets screen

### Expected Results
- The Mine button is no longer visible in the toolbar

---

## Scenario 24: Switch network from Regtest to Testnet

### Preconditions
- Application on Regtest with dev mode + RPC, Mine button visible

### Steps
1. Switch network to Testnet
2. Navigate to the Wallets screen

### Expected Results
- The Mine button is no longer visible (Testnet is not Regtest/Devnet)
