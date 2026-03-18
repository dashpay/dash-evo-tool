# Dash Evo Tool — Smoke Test Cases

Weekly smoke test specification for Dash Evo Tool, covering implemented user stories in SPV mode on testnet.

## Prerequisites

See [test-prerequisites.md](test-prerequisites.md) for environment setup, `.env` configuration, and the "bank" wallet concept.

**Before starting any test session:**

1. Launch DET with the testnet `.env` configured for SPV mode (no Core RPC).
2. Wait for SPV sync to complete (status indicator turns green).
3. Import the bank wallet mnemonic.
4. Verify bank wallet has ≥ 10 tDASH available.
5. Ensure as many tDASH as possible are consolidated at core address index 0 of the bank wallet.

---

## Test Case Format

| Field | Description |
|-------|-------------|
| **Use Case ID** | Reference to `docs/user-stories.md` (e.g., WAL-001) |
| **Test Case ID** | Unique test identifier (e.g., TC-WAL-001-01) |
| **Short Description** | What the test verifies |
| **Pre-Conditions** | State required before test execution |
| **Test Steps** | Numbered steps to execute |
| **Test Data** | Specific inputs or values used |
| **Expected Result** | Observable outcome on success |
| **Post-Condition** | State after test completes; cleanup actions |

---

## Wallet Management (WAL)

### TC-WAL-001-01: Create a new HD wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-001 |
| **Test Case ID** | TC-WAL-001-01 |
| **Short Description** | Create a new wallet with generated mnemonic |
| **Pre-Conditions** | DET is running on testnet, SPV synced, on welcome screen or wallet management |
| **Test Steps** | 1. Click "Create Wallet" 2. Move mouse to generate entropy 3. Select English for mnemonic language 4. Enter wallet name: "Smoke Test Wallet" 5. Skip password (leave empty) 6. Record the displayed mnemonic phrase 7. Confirm creation |
| **Test Data** | Wallet name: "Smoke Test Wallet" |
| **Expected Result** | New wallet appears in wallet selector. Balance shows 0 DASH. Mnemonic is 12 or 24 words. |
| **Post-Condition** | New wallet is active and selected |

### TC-WAL-002-01: Import wallet via mnemonic

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-002 |
| **Test Case ID** | TC-WAL-002-01 |
| **Short Description** | Import the bank wallet using seed phrase |
| **Pre-Conditions** | DET running, SPV synced, bank mnemonic available |
| **Test Steps** | 1. Click "Import Wallet" 2. Enter bank wallet mnemonic 3. Set wallet name: "Bank Wallet" 4. Skip password 5. Confirm import 6. Wait for balance sync |
| **Test Data** | Bank wallet mnemonic (from test prerequisites) |
| **Expected Result** | Wallet imports successfully. After sync, balance shows ≥ 10 tDASH. |
| **Post-Condition** | Bank wallet is active and synced |

### TC-WAL-004-01: Switch between wallets

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-004 |
| **Test Case ID** | TC-WAL-004-01 |
| **Short Description** | Switch between bank wallet and smoke test wallet |
| **Pre-Conditions** | Both "Bank Wallet" and "Smoke Test Wallet" exist |
| **Test Steps** | 1. Open wallet selector dropdown 2. Select "Smoke Test Wallet" 3. Verify balance shows 0 4. Open wallet selector dropdown 5. Select "Bank Wallet" 6. Verify balance shows ≥ 10 tDASH |
| **Test Data** | N/A |
| **Expected Result** | Switching is instant. Balances are correct for each wallet. No app restart needed. |
| **Post-Condition** | Bank Wallet is selected |

### TC-WAL-005-01: Rename a wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-005 |
| **Test Case ID** | TC-WAL-005-01 |
| **Short Description** | Rename the smoke test wallet |
| **Pre-Conditions** | "Smoke Test Wallet" exists |
| **Test Steps** | 1. Select "Smoke Test Wallet" 2. Access wallet settings/rename option 3. Change name to "Renamed Wallet" 4. Confirm 5. Verify name change in wallet selector 6. Restart DET 7. Verify name persisted |
| **Test Data** | New name: "Renamed Wallet" |
| **Expected Result** | Wallet appears as "Renamed Wallet" in selector. Name persists after restart. |
| **Post-Condition** | Wallet is named "Renamed Wallet" |

### TC-WAL-008-01: View wallet balances

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-008 |
| **Test Case ID** | TC-WAL-008-01 |
| **Short Description** | Verify balance display for bank wallet |
| **Pre-Conditions** | Bank wallet imported and synced |
| **Test Steps** | 1. Select bank wallet 2. Navigate to wallet overview/balance screen 3. Verify Core balance is displayed 4. Verify Platform balance is displayed (may be 0) |
| **Test Data** | N/A |
| **Expected Result** | Core balance shows ≥ 10 tDASH. Platform balance shown (0 if no credits). Values are non-negative numbers. |
| **Post-Condition** | N/A |

### TC-WAL-010-01: Generate receive address

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-010 |
| **Test Case ID** | TC-WAL-010-01 |
| **Short Description** | Generate a receive address with QR code |
| **Pre-Conditions** | Bank wallet selected |
| **Test Steps** | 1. Navigate to receive address screen 2. Verify a Dash address is displayed 3. Verify QR code is rendered 4. Copy address to clipboard |
| **Test Data** | N/A |
| **Expected Result** | Address starts with "y" (testnet). QR code is visible. Address is copyable. |
| **Post-Condition** | N/A |

### TC-WAL-013-01: View SPV sync status

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-013 |
| **Test Case ID** | TC-WAL-013-01 |
| **Short Description** | Verify SPV sync status indicator |
| **Pre-Conditions** | DET running in SPV mode on testnet |
| **Test Steps** | 1. Observe connection status indicator 2. During initial sync, verify indicator shows syncing state (orange/magenta) 3. After sync completes, verify indicator shows connected state (green) 4. Verify peer count is displayed |
| **Test Data** | N/A |
| **Expected Result** | Status indicator transitions from syncing to connected. Color-coded status visible. Peer count > 0. |
| **Post-Condition** | SPV fully synced |

### TC-WAL-016-01: View transaction history

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-016 |
| **Test Case ID** | TC-WAL-016-01 |
| **Short Description** | View transaction history for bank wallet |
| **Pre-Conditions** | Bank wallet selected and synced, wallet has prior transactions |
| **Test Steps** | 1. Navigate to transaction history screen 2. Verify transactions are listed 3. Verify each entry shows amount, date, and direction (sent/received) |
| **Test Data** | N/A |
| **Expected Result** | Transaction list is populated. Each entry has amount, timestamp, and direction. |
| **Post-Condition** | N/A |

### TC-WAL-006-01: Lock and unlock wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-006 |
| **Test Case ID** | TC-WAL-006-01 |
| **Short Description** | Lock wallet with password and unlock it |
| **Pre-Conditions** | A wallet exists (use "Renamed Wallet" from TC-WAL-005-01) |
| **Test Steps** | 1. Select the wallet 2. Set a password on the wallet 3. Lock the wallet 4. Attempt a sensitive operation (e.g., send) — verify it is blocked 5. Unlock with the password 6. Verify sensitive operations are now available |
| **Test Data** | Password: "test1234" |
| **Expected Result** | Locked wallet blocks sensitive ops. Correct password unlocks. |
| **Post-Condition** | Wallet is unlocked |

### TC-WAL-007-01: Remove a wallet

| Field | Value |
|-------|-------|
| **Use Case ID** | WAL-007 |
| **Test Case ID** | TC-WAL-007-01 |
| **Short Description** | Remove the smoke test wallet |
| **Pre-Conditions** | "Renamed Wallet" exists, no funds (or funds moved out) |
| **Test Steps** | 1. Select "Renamed Wallet" 2. Access remove/delete option 3. Confirm removal 4. Verify wallet no longer appears in selector |
| **Test Data** | N/A |
| **Expected Result** | Confirmation prompt shown. After confirm, wallet is gone from selector. |
| **Post-Condition** | Only bank wallet remains. Clean up for next test run. |

---

## Send and Receive (SND)

### TC-SND-001-01: Send Dash to an address

| Field | Value |
|-------|-------|
| **Use Case ID** | SND-001 |
| **Test Case ID** | TC-SND-001-01 |
| **Short Description** | Send tDASH from bank wallet to a generated address |
| **Pre-Conditions** | Bank wallet selected with ≥ 10 tDASH. A second wallet exists to receive. |
| **Test Steps** | 1. Create a second wallet ("Receiver Wallet") 2. Generate receive address from Receiver Wallet 3. Switch to Bank Wallet 4. Initiate send 5. Enter receiver address and amount: 0.1 tDASH 6. Review confirmation dialog (verify fee shown) 7. Confirm and broadcast 8. Switch to Receiver Wallet 9. Wait for balance to update |
| **Test Data** | Amount: 0.1 tDASH |
| **Expected Result** | Transaction broadcasts successfully. Receiver wallet balance increases by ~0.1 tDASH after confirmation. Fee was displayed before confirm. |
| **Post-Condition** | Send 0.1 tDASH back to bank wallet to restore funds. Remove Receiver Wallet. |

### TC-SND-003-01: Receive Dash with QR code

| Field | Value |
|-------|-------|
| **Use Case ID** | SND-003 |
| **Test Case ID** | TC-SND-003-01 |
| **Short Description** | Verify QR code generation for receive address |
| **Pre-Conditions** | Bank wallet selected |
| **Test Steps** | 1. Navigate to receive screen 2. Verify QR code is displayed 3. Verify text address is shown alongside QR 4. Click copy button 5. Verify address is in clipboard |
| **Test Data** | N/A |
| **Expected Result** | QR code renders correctly. Text address matches QR content. Copy works. |
| **Post-Condition** | N/A |

### TC-SND-005-01: See fee estimate before confirming

| Field | Value |
|-------|-------|
| **Use Case ID** | SND-005 |
| **Test Case ID** | TC-SND-005-01 |
| **Short Description** | Verify fee estimate is shown in send confirmation |
| **Pre-Conditions** | Bank wallet selected with funds |
| **Test Steps** | 1. Initiate send to any valid testnet address 2. Enter amount: 0.01 tDASH 3. Review confirmation dialog 4. Verify fee amount is displayed 5. Verify total deduction (amount + fee) is shown 6. Cancel the send |
| **Test Data** | Amount: 0.01 tDASH, Destination: any valid testnet address |
| **Expected Result** | Fee estimate and total deduction clearly visible before confirm. |
| **Post-Condition** | Send cancelled, no funds moved |

---

## Asset Locks (ALK)

### TC-ALK-001-01: Create an asset lock

| Field | Value |
|-------|-------|
| **Use Case ID** | ALK-001 |
| **Test Case ID** | TC-ALK-001-01 |
| **Short Description** | Create an asset lock from bank wallet |
| **Pre-Conditions** | Bank wallet selected with ≥ 2 tDASH |
| **Test Steps** | 1. Navigate to asset lock creation 2. Enter amount: 0.5 tDASH 3. Review fee calculation 4. Confirm and create 5. Wait for asset lock confirmation |
| **Test Data** | Amount: 0.5 tDASH |
| **Expected Result** | Asset lock created successfully. Transaction ID shown. Wallet balance decreases by ~0.5 tDASH + fee. |
| **Post-Condition** | Asset lock available for identity registration or top-up |

---

## Identity Operations (IDN)

### TC-IDN-001-01: Register a new identity

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-001 |
| **Test Case ID** | TC-IDN-001-01 |
| **Short Description** | Register a new identity funded by asset lock |
| **Pre-Conditions** | Bank wallet with asset lock from TC-ALK-001-01 |
| **Test Steps** | 1. Navigate to identity registration 2. Select bank wallet as source 3. Follow multi-stage confirmation flow 4. Wait for identity creation on Platform 5. Verify identity ID is displayed |
| **Test Data** | Funding: from asset lock created in TC-ALK-001-01 |
| **Expected Result** | Identity registered successfully. Identity ID shown. Credits balance > 0. |
| **Post-Condition** | New identity available for DPNS, DashPay, and token tests |

### TC-IDN-004-01: Top up identity credits

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-004 |
| **Test Case ID** | TC-IDN-004-01 |
| **Short Description** | Add credits to an existing identity |
| **Pre-Conditions** | Identity from TC-IDN-001-01 exists, bank wallet has funds |
| **Test Steps** | 1. Select the identity 2. Navigate to top-up 3. Enter amount: 0.1 tDASH worth of credits 4. Confirm top-up 5. Wait for confirmation 6. Verify credit balance increased |
| **Test Data** | Top-up amount: 0.1 tDASH |
| **Expected Result** | Credits increase after top-up. Transaction confirmed. |
| **Post-Condition** | Identity has sufficient credits for subsequent tests |

### TC-IDN-008-01: View identity keys and details

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-008 |
| **Test Case ID** | TC-IDN-008-01 |
| **Short Description** | Inspect identity key list and details |
| **Pre-Conditions** | Identity from TC-IDN-001-01 exists |
| **Test Steps** | 1. Select the identity 2. Navigate to keys/details view 3. Verify key list is shown 4. Verify each key shows type, purpose, and status |
| **Test Data** | N/A |
| **Expected Result** | At least one key listed (AUTHENTICATION). Key details (type, purpose, status) are visible. |
| **Post-Condition** | N/A |

### TC-IDN-009-01: Refresh identity state

| Field | Value |
|-------|-------|
| **Use Case ID** | IDN-009 |
| **Test Case ID** | TC-IDN-009-01 |
| **Short Description** | Manually refresh identity data from network |
| **Pre-Conditions** | Identity exists |
| **Test Steps** | 1. Select the identity 2. Click refresh/reload button 3. Verify credit balance and key state update |
| **Test Data** | N/A |
| **Expected Result** | Identity data refreshed without error. Updated balance and keys reflected. |
| **Post-Condition** | N/A |

---

## DPNS (DPN)

### TC-DPN-001-01: Register a DPNS username

| Field | Value |
|-------|-------|
| **Use Case ID** | DPN-001 |
| **Test Case ID** | TC-DPN-001-01 |
| **Short Description** | Register a unique username for the test identity |
| **Pre-Conditions** | Identity from TC-IDN-001-01 with sufficient credits |
| **Test Steps** | 1. Select the identity 2. Navigate to DPNS registration 3. Enter username: "detsmoke-" + current timestamp (for uniqueness) 4. Review cost estimate 5. Confirm registration 6. Wait for Platform confirmation |
| **Test Data** | Username: "detsmoke-{timestamp}" (e.g., "detsmoke-1710770000") |
| **Expected Result** | Username registered successfully. Appears in owned usernames list. Cost deducted from credits. |
| **Post-Condition** | Identity has a DPNS username for DashPay tests |

### TC-DPN-002-01: View owned usernames

| Field | Value |
|-------|-------|
| **Use Case ID** | DPN-002 |
| **Test Case ID** | TC-DPN-002-01 |
| **Short Description** | Verify registered username appears in owned list |
| **Pre-Conditions** | Username registered in TC-DPN-001-01 |
| **Test Steps** | 1. Navigate to owned usernames screen 2. Verify the registered username is listed |
| **Test Data** | N/A |
| **Expected Result** | The username from TC-DPN-001-01 appears in the list. |
| **Post-Condition** | N/A |

### TC-DPN-003-01: View active name contests

| Field | Value |
|-------|-------|
| **Use Case ID** | DPN-003 |
| **Test Case ID** | TC-DPN-003-01 |
| **Short Description** | View the DPNS name contests screen |
| **Pre-Conditions** | DET running on testnet |
| **Test Steps** | 1. Navigate to active DPNS contests screen 2. Verify the screen loads without error 3. If contests exist, verify they show status and vote counts |
| **Test Data** | N/A |
| **Expected Result** | Screen loads. Any listed contests show status and vote information. |
| **Post-Condition** | N/A |

---

## DashPay (DPY)

### TC-DPY-001-01: Create a DashPay profile

| Field | Value |
|-------|-------|
| **Use Case ID** | DPY-001 |
| **Test Case ID** | TC-DPY-001-01 |
| **Short Description** | Create a DashPay profile for the test identity |
| **Pre-Conditions** | Identity with DPNS username and sufficient credits |
| **Test Steps** | 1. Navigate to DashPay profile screen 2. Set display name: "Smoke Test Bot" 3. Set bio: "Automated QA testing" 4. Skip avatar (optional) 5. Save/publish profile 6. Wait for Platform confirmation |
| **Test Data** | Display name: "Smoke Test Bot", Bio: "Automated QA testing" |
| **Expected Result** | Profile created. Display name and bio visible on profile screen. |
| **Post-Condition** | Profile exists for search and contact tests |

### TC-DPY-002-01: Search DashPay profiles

| Field | Value |
|-------|-------|
| **Use Case ID** | DPY-002 |
| **Test Case ID** | TC-DPY-002-01 |
| **Short Description** | Search for a known DashPay profile |
| **Pre-Conditions** | At least one DashPay profile exists on testnet |
| **Test Steps** | 1. Navigate to DashPay search 2. Enter the username registered in TC-DPN-001-01 3. Verify search results appear 4. Verify profile details shown (display name, bio) |
| **Test Data** | Search query: username from TC-DPN-001-01 |
| **Expected Result** | Profile found in search results. Display name and bio match what was set. |
| **Post-Condition** | N/A |

---

## Token Operations (TOK)

### TC-TOK-001-01: View token balances

| Field | Value |
|-------|-------|
| **Use Case ID** | TOK-001 |
| **Test Case ID** | TC-TOK-001-01 |
| **Short Description** | View the "My Tokens" screen |
| **Pre-Conditions** | Identity exists (may have 0 tokens) |
| **Test Steps** | 1. Navigate to "My Tokens" screen 2. Verify screen loads without error 3. If tokens are held, verify balances displayed |
| **Test Data** | N/A |
| **Expected Result** | Token list screen loads. Any held tokens show name and balance. Empty state shown gracefully if no tokens. |
| **Post-Condition** | N/A |

### TC-TOK-002-01: Search and discover tokens

| Field | Value |
|-------|-------|
| **Use Case ID** | TOK-002 |
| **Test Case ID** | TC-TOK-002-01 |
| **Short Description** | Search for tokens by keyword |
| **Pre-Conditions** | DET running on testnet |
| **Test Steps** | 1. Navigate to token search/discovery 2. Enter a search keyword (e.g., "test" or "dash") 3. Verify results appear or "no results" message shows 4. If results found, verify token name and metadata displayed |
| **Test Data** | Search keyword: "test" |
| **Expected Result** | Search executes without error. Results or empty-state message displayed. |
| **Post-Condition** | N/A |

---

## Contracts and Documents (DOC)

### TC-DOC-003-01: Import a contract by ID

| Field | Value |
|-------|-------|
| **Use Case ID** | DOC-003 |
| **Test Case ID** | TC-DOC-003-01 |
| **Short Description** | Import the DPNS contract by ID |
| **Pre-Conditions** | DET running on testnet |
| **Test Steps** | 1. Navigate to contracts screen 2. Click add/import contract 3. Enter DPNS contract ID 4. Confirm import 5. Verify contract appears in list with name and document types |
| **Test Data** | DPNS contract ID (system contract, available on testnet) |
| **Expected Result** | Contract imported successfully. Shows contract name and available document types. |
| **Post-Condition** | Contract available for document browsing |

### TC-DOC-004-01: Query and browse documents

| Field | Value |
|-------|-------|
| **Use Case ID** | DOC-004 |
| **Test Case ID** | TC-DOC-004-01 |
| **Short Description** | Query documents from an imported contract |
| **Pre-Conditions** | DPNS contract imported from TC-DOC-003-01 |
| **Test Steps** | 1. Select the DPNS contract 2. Select a document type (e.g., "domain") 3. Execute query 4. Verify document results are displayed |
| **Test Data** | Contract: DPNS, Document type: "domain" |
| **Expected Result** | Query returns documents. Each document shows its properties. |
| **Post-Condition** | N/A |

---

## Developer and Power Tools (DEV)

### TC-DEV-005-01: View Platform info

| Field | Value |
|-------|-------|
| **Use Case ID** | DEV-005 |
| **Test Case ID** | TC-DEV-005-01 |
| **Short Description** | View Platform network status |
| **Pre-Conditions** | DET running on testnet, connected |
| **Test Steps** | 1. Navigate to Platform info screen 2. Verify epoch info is displayed 3. Verify total credits shown 4. Verify validator list visible |
| **Test Data** | N/A |
| **Expected Result** | Platform info screen loads. Epoch, credits, and validators displayed with current data. |
| **Post-Condition** | N/A |

---

## Network and Settings (NET)

### TC-NET-001-01: Verify testnet network selection

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-001 |
| **Test Case ID** | TC-NET-001-01 |
| **Short Description** | Verify DET is on testnet and network switcher works |
| **Pre-Conditions** | DET running |
| **Test Steps** | 1. Navigate to network/settings screen 2. Verify current network shows "Testnet" 3. Verify other networks are listed (Mainnet, Devnet, Local) 4. Do NOT switch networks (just verify UI) |
| **Test Data** | N/A |
| **Expected Result** | Current network is Testnet. Network switcher lists all available networks. |
| **Post-Condition** | Stay on Testnet |

### TC-NET-004-01: Toggle theme

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-004 |
| **Test Case ID** | TC-NET-004-01 |
| **Short Description** | Switch between light and dark themes |
| **Pre-Conditions** | DET running |
| **Test Steps** | 1. Navigate to settings 2. Select "Dark" theme 3. Verify UI updates to dark colors 4. Select "Light" theme 5. Verify UI updates to light colors |
| **Test Data** | N/A |
| **Expected Result** | Theme changes are applied immediately. UI is readable in both modes. |
| **Post-Condition** | Return to preferred theme |

### TC-NET-005-01: Toggle developer mode

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-005 |
| **Test Case ID** | TC-NET-005-01 |
| **Short Description** | Enable and disable developer mode |
| **Pre-Conditions** | DET running |
| **Test Steps** | 1. Navigate to settings 2. Enable developer mode 3. Verify additional UI elements appear (address tables, refresh controls, debug tools) 4. Disable developer mode 5. Verify advanced elements are hidden |
| **Test Data** | N/A |
| **Expected Result** | Developer mode shows extra controls. Disabling hides them. |
| **Post-Condition** | Developer mode enabled (for subsequent testing) |

### TC-NET-010-01: Onboarding wizard

| Field | Value |
|-------|-------|
| **Use Case ID** | NET-010 |
| **Test Case ID** | TC-NET-010-01 |
| **Short Description** | Verify onboarding wizard on fresh start |
| **Pre-Conditions** | DET launched with no existing wallets (fresh install or data wiped) |
| **Test Steps** | 1. Launch DET fresh (no prior data) 2. Verify welcome screen appears 3. Verify setup options shown (Create Wallet, Import Wallet, Just Explore) 4. Click "Just Explore" 5. Verify app loads in explore mode without a wallet |
| **Test Data** | N/A |
| **Expected Result** | Welcome/onboarding screen shown on first launch. All three options accessible. "Just Explore" works without wallet. |
| **Post-Condition** | App in explore mode |

---

## Recommended Execution Order

Execute tests in this order to build on each other and minimize cleanup:

1. **NET-010-01** — Onboarding (fresh start)
2. **WAL-013-01** — SPV sync status
3. **WAL-001-01** — Create wallet
4. **WAL-002-01** — Import bank wallet
5. **WAL-004-01** — Switch wallets
6. **WAL-005-01** — Rename wallet
7. **WAL-008-01** — View balances
8. **WAL-010-01** — Generate receive address
9. **WAL-016-01** — Transaction history
10. **WAL-006-01** — Lock/unlock
11. **SND-003-01** — Receive with QR
12. **SND-005-01** — Fee estimate
13. **SND-001-01** — Send tDASH
14. **ALK-001-01** — Create asset lock
15. **IDN-001-01** — Register identity
16. **IDN-004-01** — Top up credits
17. **IDN-008-01** — View identity keys
18. **IDN-009-01** — Refresh identity
19. **DPN-001-01** — Register DPNS name
20. **DPN-002-01** — View owned names
21. **DPN-003-01** — View contests
22. **DPY-001-01** — Create profile
23. **DPY-002-01** — Search profiles
24. **TOK-001-01** — View tokens
25. **TOK-002-01** — Search tokens
26. **DOC-003-01** — Import contract
27. **DOC-004-01** — Query documents
28. **DEV-005-01** — Platform info
29. **NET-001-01** — Network selection
30. **NET-004-01** — Theme toggle
31. **NET-005-01** — Developer mode
32. **WAL-007-01** — Remove wallet (cleanup)
