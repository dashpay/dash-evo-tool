# Unified Send Screen: Requirements Document

## Executive Summary

### Problem Statement

The Send screen in Dash Evo Tool currently supports 6 of 12 possible Source-to-Destination combinations (and the 4 Identity-related paths are handled on separate screens entirely). Users who want to move funds between layers -- Core wallet to shielded pool, shielded pool to a Core address, or any source to an Identity -- must navigate multiple screens with inconsistent flows. This fragmentation violates the "single place to send funds" mental model that Alex (Everyday User) expects, creates friction for Priya (Power User) who manages funds across all layers, and slows Jordan (Platform Developer) who needs to fund identities and shield credits rapidly during development cycles.

### Key Actors

| Actor | Primary Goal on Send Screen |
|---|---|
| **Alex Torres** (Everyday User) | Send Dash to someone. That's it. Don't make them think about layers. |
| **Priya Nakamura** (Power User) | Full control over source selection, fee strategy, and destination type across all transparent layers. |
| **Jordan Kim** (Platform Developer) | Rapidly fund identities, shield credits, and move test funds between all layers with minimal clicks. |

### Solution Direction

Extend the existing `WalletSendScreen` to handle ALL 12 Source x Destination combinations through one unified interface. The screen already has the correct architecture (source radio buttons, `AddressInput` with type detection, `AmountInput`, routing via `validate_and_send()`). The work is primarily: (1) wire up the 6 missing combinations to existing backend tasks, (2) add Identity as a fourth source type for identity-originated transfers/withdrawals, and (3) apply progressive disclosure so Alex never sees combinations that would confuse them.

---

## 1. Full Source x Destination Matrix

### Sources (4)

| Source | Description | Balance Unit | Who Sees It |
|---|---|---|---|
| **Core Wallet** | UTXO-based L1 DASH | Duffs (displayed as DASH) | All personas |
| **Platform Addresses** | L2 credit addresses (bech32m) | Credits | Priya, Jordan |
| **Identity** | Platform identity credit balance | Credits | Priya, Jordan |
| **Shielded Pool** | ZK shielded credits (Orchard) | Credits | Jordan (developer mode) |

> **Note on Identity as Source**: Currently, identity transfers and withdrawals happen on dedicated Identity screens. This requirements document proposes bringing them into the Send screen as well, so the user has a single "Send" entry point. Identity as a source is distinct from Platform Addresses because identity credits are bound to an identity (with key-based authorization), while Platform address credits are bound to an address (with wallet-key authorization).

### Destinations (4)

| Destination | Address Format | Validation | Who Sees It |
|---|---|---|---|
| **Core** | P2PKH Base58 (`X...`/`y...`) | `AddressKind::Core` | All personas |
| **Platform** | Bech32m (`dash1...`/`tdash1...`) | `AddressKind::Platform` | Priya, Jordan |
| **Shielded** | Bech32m with `z` (`dash1z...`/`tdash1z...`) | `AddressKind::Shielded` | Jordan (developer mode) |
| **Identity** | 32-byte Base58 identifier | `AddressKind::Identity` | Priya, Jordan |

### Complete 4x4 Matrix

| Source / Dest | Core | Platform | Shielded | Identity |
|---|---|---|---|---|
| **Core Wallet** | SEND (L1 tx) | FUND (asset lock) | SHIELD (asset lock) | TOP UP (asset lock) |
| **Platform** | WITHDRAW | TRANSFER | SHIELD | TOP UP |
| **Identity** | WITHDRAW | TRANSFER TO ADDR | N/A (v2) | TRANSFER |
| **Shielded** | WITHDRAW | UNSHIELD | PRIVATE SEND | N/A (v2) |

### Detailed Breakdown per Cell

#### Row 1: Core Wallet as Source

| Dest | Status | Backend Task | Multi-Step? | User-Facing Name | Notes |
|---|---|---|---|---|---|
| Core | IMPLEMENTED | `CoreTask::SendWalletPayment` | No | "Send DASH" | Standard L1 payment |
| Platform | IMPLEMENTED | `WalletTask::FundPlatformAddressFromWalletUtxos` | Yes (asset lock created internally) | "Fund Platform Address" | Asset lock is abstracted away |
| Shielded | BACKEND EXISTS, UI MISSING | `ShieldedTask::ShieldFromAssetLock` | Yes (asset lock + proof wait) | "Shield DASH" | Backend creates asset lock, waits for proof, then shields. ~30s |
| Identity | BACKEND EXISTS, UI ON SEPARATE SCREEN | `IdentityTask::TopUpIdentity` (with `FundWithWallet`) | Yes (asset lock + proof wait + top-up) | "Top Up Identity" | Currently lives on Identity screen. Multi-step: create asset lock, wait for IS-lock/chain-lock proof, broadcast top-up transition |

#### Row 2: Platform Addresses as Source

| Dest | Status | Backend Task | Multi-Step? | User-Facing Name | Notes |
|---|---|---|---|---|---|
| Core | IMPLEMENTED | `WalletTask::WithdrawFromPlatformAddress` | No (single state transition) | "Withdraw to Wallet" | Withdrawal queued on Platform, settled after ~18 blocks |
| Platform | IMPLEMENTED | `WalletTask::TransferPlatformCredits` | No | "Transfer Credits" | Platform-to-platform address transfer |
| Shielded | BACKEND EXISTS, UI MISSING | `ShieldedTask::ShieldCredits` | No | "Shield Credits" | Type 15 state transition |
| Identity | BACKEND EXISTS, UI ON SEPARATE SCREEN | `IdentityTask::TopUpIdentityFromPlatformAddresses` | No | "Top Up Identity" | Currently on Identity screen. Direct platform address to identity credit transfer |

#### Row 3: Identity as Source

| Dest | Status | Backend Task | Multi-Step? | User-Facing Name | Notes |
|---|---|---|---|---|---|
| Core | BACKEND EXISTS, UI ON SEPARATE SCREEN | `IdentityTask::WithdrawFromIdentity` | No | "Withdraw Credits" | Withdrawal queued, settled after ~18 blocks |
| Platform | BACKEND EXISTS, UI ON SEPARATE SCREEN | `IdentityTask::TransferToAddresses` | No | "Transfer to Address" | Identity credits to platform address(es) |
| Shielded | NO BACKEND | N/A | - | - | **Not technically possible in v1.** Would require Identity->Platform->Shielded chain. Defer to v2. |
| Identity | BACKEND EXISTS, UI ON SEPARATE SCREEN | `IdentityTask::Transfer` | No | "Transfer Credits" | Identity-to-identity credit transfer |

#### Row 4: Shielded Pool as Source

| Dest | Status | Backend Task | Multi-Step? | User-Facing Name | Notes |
|---|---|---|---|---|---|
| Core | BACKEND EXISTS, UI MISSING | `ShieldedTask::ShieldedWithdrawal` | No (Type 19 state transition) | "Withdraw from Shield" | L1 withdrawal from shielded pool |
| Platform | IMPLEMENTED | `ShieldedTask::UnshieldCredits` | No (Type 17 state transition) | "Unshield Credits" | Shielded pool to platform address |
| Shielded | IMPLEMENTED | `ShieldedTask::ShieldedTransfer` | No (Type 16 state transition) | "Private Send" | Private ZK transfer |
| Identity | NO BACKEND | N/A | - | - | **Not technically possible in v1.** Would require Shielded->Platform->Identity chain. Defer to v2. |

### Summary of Work

| Category | Count | Cells |
|---|---|---|
| Already implemented in Send screen | 6 | Core->Core, Core->Platform, Platform->Platform, Platform->Core, Shielded->Shielded, Shielded->Platform |
| Backend exists, wire up in Send screen | 6 | Core->Shielded, Core->Identity, Platform->Shielded, Platform->Identity, Shielded->Core, Identity->Core, Identity->Platform, Identity->Identity |
| Not technically possible (defer to v2) | 2 | Identity->Shielded, Shielded->Identity |
| New backend task needed | 0 | All paths have existing backend tasks |

**Correction on counts**: There are actually 8 cells to wire up (the Identity row contributes 3 cells plus Identity as source itself), and 2 cells deferred. Let me recount:

- **Wire up (backend exists, UI missing from Send screen)**: Core->Shielded, Core->Identity, Platform->Shielded, Platform->Identity, Shielded->Core, Identity->Core, Identity->Platform, Identity->Identity = **8 cells**
- **Defer (no backend, not possible in v1)**: Identity->Shielded, Shielded->Identity = **2 cells**
- **Already done**: 6 cells

---

## 2. Stakeholder & Actor Analysis

### Primary Actors

**Alex Torres (Everyday User)**
- **Goal**: Send Dash to someone using an address or (eventually) a username.
- **Pain Points**: Too many choices, technical jargon ("asset lock", "credits", "platform address"), confusion about which balance is "real".
- **Success Metric**: Complete a send in under 10 seconds with zero awareness of layers/asset locks.
- **What they should see**: Core Wallet as the only source. Core address as the only destination type. Everything else hidden.

**Priya Nakamura (Power User)**
- **Goal**: Full control over fund routing between Core, Platform, and Identity layers.
- **Pain Points**: Identity operations scattered across multiple screens. No batch operations. Fee strategy hidden.
- **Success Metric**: Any transparent (non-shielded) send combination in a single screen without switching contexts.
- **What they should see**: Core Wallet, Platform Addresses, and Identity as sources. Core, Platform, and Identity as destinations. Fee strategy options. Advanced mode with multi-input/multi-output.

**Jordan Kim (Platform Developer)**
- **Goal**: Rapidly fund identities, shield credits, move test funds between all layers.
- **Pain Points**: Multi-screen workflows for identity top-up. No send-to-identity from Send screen. Shielded operations not integrated.
- **Success Metric**: Fund an identity from Core wallet in one flow. Shield credits in one flow.
- **What they should see**: Everything. All 4 sources, all 4 destinations (minus the 2 impossible combinations).

### Secondary Actors

- **MCP/CLI clients (AI agents, scripts)**: Need programmatic access to all send combinations via MCP tools.
- **Supporting systems**: SPV sync (must be complete before Core sends), Platform SDK (for state transitions), Orchard proving key (must be warmed for shielded ops).

---

## 3. User Stories & Acceptance Criteria

### New Send Combinations

#### SND-007: Shield DASH from Core wallet [Gap]
**Persona:** Jordan

As a developer, I want to shield DASH directly from my Core wallet so that I can fund my shielded pool without intermediate steps.

**Acceptance Criteria:**
- Given I have Core wallet balance and select "Core Wallet" as source
- When I enter a shielded address (`dash1z...`/`tdash1z...`) as destination and an amount
- Then the system creates an asset lock, waits for proof, and shields the credits
- And a progress banner shows the multi-step process (creating asset lock... waiting for proof... shielding...)
- And on success, my shielded balance updates after the next sync
- And on failure, a user-friendly error explains what went wrong and what to try

#### SND-008: Top up identity from Send screen [Gap]
**Persona:** Priya, Jordan

As a user, I want to top up an identity directly from the Send screen so that I do not have to navigate to the Identity screen to add credits.

**Acceptance Criteria:**
- Given I select "Core Wallet" or "Platform Addresses" as source
- When I enter an identity ID (Base58, 32+ characters) as destination
- Then the system resolves the identity and shows its display name (if DPNS registered)
- And the system uses the appropriate backend task:
  - Core Wallet: `IdentityTask::TopUpIdentity` (via asset lock)
  - Platform Addresses: `IdentityTask::TopUpIdentityFromPlatformAddresses`
- And on success, the identity's credit balance increases
- And the confirmation shows the identity ID and credited amount

#### SND-009: Shield credits from Platform address [Gap]
**Persona:** Jordan

As a developer, I want to shield credits from a Platform address into the shielded pool so that I can make private transactions.

**Acceptance Criteria:**
- Given I select "Platform Addresses" as source
- When I enter my own shielded address as destination
- Then the system executes `ShieldedTask::ShieldCredits` (Type 15 transition)
- And a from-address is selected automatically (highest balance Platform address)
- And on success, credits move from the Platform address to the shielded pool

#### SND-010: Withdraw from shielded pool to Core address [Gap]
**Persona:** Jordan

As a developer, I want to withdraw from the shielded pool directly to a Core L1 address so that I can convert shielded credits back to spendable DASH.

**Acceptance Criteria:**
- Given I have shielded balance and select "Shielded Pool" as source
- When I enter a Core address (`X...`/`y...`) as destination
- Then the system executes `ShieldedTask::ShieldedWithdrawal` (Type 19 transition)
- And the withdrawal is queued on Platform and settles after confirmation

#### SND-011: Transfer identity credits to another identity [Gap — on Send screen]
**Persona:** Priya, Jordan

As a user, I want to transfer credits from one of my identities to another identity using the Send screen so that I have one place for all fund transfers.

**Acceptance Criteria:**
- Given I select an identity as source (from a dropdown of my loaded identities)
- When I enter another identity ID as destination
- Then the system executes `IdentityTask::Transfer`
- And both identity balances update after the transfer

#### SND-012: Withdraw identity credits to Core address [Gap — on Send screen]
**Persona:** Priya, Jordan

As a user, I want to withdraw identity credits to a Core address from the Send screen.

**Acceptance Criteria:**
- Given I select an identity as source
- When I enter a Core address as destination
- Then the system executes `IdentityTask::WithdrawFromIdentity`
- And the withdrawal is queued on Platform

#### SND-013: Transfer identity credits to Platform address [Gap — on Send screen]
**Persona:** Priya, Jordan

As a user, I want to transfer identity credits to a Platform address from the Send screen.

**Acceptance Criteria:**
- Given I select an identity as source
- When I enter a Platform address (bech32m) as destination
- Then the system executes `IdentityTask::TransferToAddresses`
- And the credits arrive at the Platform address

### MCP/CLI User Stories

#### MCP-003: Shield DASH from Core via CLI [Gap]
**Persona:** Jordan

As a developer using the CLI, I want to shield DASH from my Core wallet so that I can automate shielded pool funding in test scripts.

**Acceptance Criteria:**
- Given `det-cli shielded-shield-from-core --wallet-id <id> --amount-duffs <amount>`
- Then the tool dispatches `ShieldedTask::ShieldFromAssetLock` and returns the result
- And the tool blocks until the multi-step operation completes (asset lock + proof + shield)

#### MCP-004: Top up identity via CLI [Gap]
**Persona:** Jordan

As a developer, I want to top up an identity from my wallet via CLI so that I can script identity funding.

**Acceptance Criteria:**
- Given `det-cli identity-topup --wallet-id <id> --identity-id <id> --amount-duffs <amount>`
- Then the tool dispatches `IdentityTask::TopUpIdentity` with `FundWithWallet`
- And returns the new identity balance on success

#### MCP-005: Transfer credits between Platform addresses via CLI [Gap]
**Persona:** Jordan

As a developer, I want to transfer Platform credits via CLI.

**Acceptance Criteria:**
- Given `det-cli platform-credits-transfer --wallet-id <id> --to-address <addr> --amount-credits <amount>`
- Then the tool dispatches `WalletTask::TransferPlatformCredits`

#### MCP-006: Withdraw Platform credits to Core via CLI [Gap]
**Persona:** Jordan

As a developer, I want to withdraw Platform credits to a Core address via CLI.

**Acceptance Criteria:**
- Given `det-cli platform-credits-withdraw --wallet-id <id> --to-address <addr> --amount-credits <amount>`
- Then the tool dispatches `WalletTask::WithdrawFromPlatformAddress`

#### MCP-007: Identity credit transfer via CLI [Gap]
**Persona:** Jordan

As a developer, I want to transfer identity credits to another identity via CLI.

**Acceptance Criteria:**
- Given `det-cli identity-credits-transfer --wallet-id <id> --from-identity <id> --to-identity <id> --amount-credits <amount>`
- Then the tool dispatches `IdentityTask::Transfer`

#### MCP-008: Identity withdrawal via CLI [Gap]
**Persona:** Jordan

As a developer, I want to withdraw identity credits to a Core address via CLI.

**Acceptance Criteria:**
- Given `det-cli identity-credits-withdraw --wallet-id <id> --identity-id <id> --to-address <addr> --amount-credits <amount>`
- Then the tool dispatches `IdentityTask::WithdrawFromIdentity`

#### MCP-009: Shielded transfer via CLI [Gap]
**Persona:** Jordan

As a developer, I want to send a private shielded transfer via CLI.

**Acceptance Criteria:**
- Given `det-cli shielded-transfer --wallet-id <id> --to-address <shielded-addr> --amount-credits <amount>`
- Then the tool dispatches `ShieldedTask::ShieldedTransfer`

#### MCP-010: Shield from Platform address via CLI [Gap]
**Persona:** Jordan

As a developer, I want to shield credits from a Platform address via CLI.

**Acceptance Criteria:**
- Given `det-cli shielded-shield-from-platform --wallet-id <id> --amount-credits <amount>`
- Then the tool dispatches `ShieldedTask::ShieldCredits`

#### MCP-011: Unshield to Platform address via CLI [Gap]
**Persona:** Jordan

As a developer, I want to unshield credits to a Platform address via CLI.

**Acceptance Criteria:**
- Given `det-cli shielded-unshield --wallet-id <id> --to-address <platform-addr> --amount-credits <amount>`
- Then the tool dispatches `ShieldedTask::UnshieldCredits`

#### MCP-012: Shielded withdrawal to Core via CLI [Gap]
**Persona:** Jordan

As a developer, I want to withdraw from shielded pool to Core via CLI.

**Acceptance Criteria:**
- Given `det-cli shielded-withdraw --wallet-id <id> --to-address <core-addr> --amount-credits <amount>`
- Then the tool dispatches `ShieldedTask::ShieldedWithdrawal`

---

## 4. Real-Life Usage Scenarios

### Scenario 1: Alex sends DASH to pay a freelance client (Core -> Core)

Alex opens the wallet, taps Send. The screen shows one source (Core Wallet) pre-selected, a destination field, and an amount field. Alex pastes the client's `X...` address, types `2.5`, sees the fee estimate ("Fee: ~0.00001 DASH"), and taps "Send DASH". A spinner appears for 2 seconds, then a success message. Alex never sees "Platform", "Shielded", or "Identity" anywhere.

**Edge case**: Alex accidentally pastes a `dash1...` Platform address. Since Alex is in Everyday User mode, the address is rejected with: "This address type is not supported in basic mode. Switch to advanced mode in Settings to send to Platform addresses."

### Scenario 2: Priya funds her identity from Platform address balance (Platform -> Identity)

Priya has 50,000 credits across 3 Platform addresses. She opens Send, selects "Platform Addresses" as source. The source breakdown shows her 3 addresses and their balances. She pastes an identity ID into the destination. The system detects it as an Identity destination and shows the identity's DPNS name ("priya.dash") and current credit balance. She enters 30,000 credits. The system auto-allocates across her Platform addresses (highest balance first). She taps "Top Up Identity" and sees success.

**Edge case**: Priya enters an identity ID that doesn't exist on the network. The system shows: "No identity found with this ID. Check the ID and try again."

### Scenario 3: Jordan shields DASH from Core wallet (Core -> Shielded)

Jordan is testing shielded transactions on Testnet. She opens Send, selects "Core Wallet", pastes her own shielded address (`tdash1z...`). The system detects the shielded destination and shows a note: "This will create an asset lock and shield your DASH. This process takes approximately 30 seconds." She enters 1000 duffs, taps "Shield DASH". A progress banner appears:
1. "Creating asset lock..." (3s)
2. "Waiting for lock proof..." (10-20s)
3. "Shielding credits..." (5s)
4. Success: "0.00001000 DASH shielded successfully"

**Edge case**: The asset lock proof times out. Error: "Could not obtain a lock proof in time. Your funds have not been spent. Please try again."

**Failure scenario**: Jordan's wallet has insufficient balance. Error shown before sending: "Insufficient balance. You need at least 0.00001100 DASH (amount + estimated fee)."

### Scenario 4: Jordan moves shielded credits to Core wallet (Shielded -> Core)

Jordan is done testing and wants to move shielded credits back to L1. She selects "Shielded Pool" as source (showing balance: 50,000 credits), enters a Core address (`y...`), enters the amount, and taps "Withdraw from Shield". The withdrawal transition is broadcast and queued.

### Scenario 5: Priya transfers identity credits to another identity (Identity -> Identity)

Priya has two identities -- one for her masternode operations and one for DashPay. She wants to move credits from the masternode identity to the DashPay one. She opens Send, selects her masternode identity from the "Identity" source dropdown, pastes the DashPay identity ID as destination, enters 100,000 credits, and taps "Transfer Credits".

### Scenario 6: MCP/CLI automated test setup (multiple tools)

Jordan's test script does:
```bash
# Create wallet and get address
det-cli core-wallets-list
det-cli core-address-create --wallet-id test-wallet

# After faucet funding, top up identity
det-cli identity-topup --wallet-id test-wallet --identity-id ABC123... --amount-duffs 100000

# Shield some credits for privacy testing
det-cli shielded-shield-from-core --wallet-id test-wallet --amount-duffs 50000

# Transfer shielded credits
det-cli shielded-transfer --wallet-id test-wallet --to-address tdash1z... --amount-credits 25000
```

---

## 5. UX Flow: Send Screen Redesign

### 5.1 Source Selection

**Current**: Radio buttons for Core Wallet, Platform Addresses, Shielded Pool.

**Proposed**: Add Identity as a fourth source type. When Identity is selected, show a dropdown of loaded identities (with DPNS name if available) and their credit balances.

```
Source Selection:
  ( ) Core Wallet           [1.234 DASH]
  ( ) Platform Addresses    [50,000 credits]
  ( ) Identity              [dropdown: "priya.dash (ID: Abc...)" -- 100,000 credits]
  ( ) Shielded Pool         [25,000 credits]
```

**Progressive disclosure rules** (see Section 6) control which sources appear.

### 5.2 Destination Input

**Current**: `AddressInput` component with auto-detection of Core, Platform, Shielded, and Identity address types.

**Proposed**: No change to the component itself. The `AddressInput` already detects all 4 `AddressKind` variants. However:
- After detection, show a **destination summary** below the input:
  - Core: "Wallet address (Xo3f...)" -- no extra info
  - Platform: "Platform address (dash1q...)" -- no extra info
  - Shielded: "Shielded address (dash1z...)" with a privacy note icon
  - Identity: Resolve the identity from the network and show: "Identity: priya.dash (Abc123...)" with current credit balance. If resolution fails: "Identity not found" warning.

- **Invalid combinations**: When the user selects a source and enters an incompatible destination (e.g., Identity->Shielded), show an inline warning: "Sending from Identity to a shielded address is not currently supported. You can first transfer to a Platform address, then shield from there."

### 5.3 Amount Input

**Current**: `AmountInput` component with DASH/credit unit switching.

**Proposed**:
- **Unit auto-selection**: When source is Core Wallet and destination is Core, show amount in DASH. When source involves Platform credits (Platform, Identity, Shielded), show amount in credits with DASH equivalent.
- **Max button**: "Use all available balance" with fee subtracted.
- When source is Core and destination is Platform/Identity/Shielded (i.e., asset lock path), the amount input should be in duffs/DASH (since the source is L1).

### 5.4 Fee Display and Confirmation

**Current**: Fee estimate shown for some paths, confirmation dialog for Core sends.

**Proposed**: Standardize across all paths:

| Path Type | Fee Display | Confirmation |
|---|---|---|
| Core->Core | Mining fee in DASH | Confirmation dialog with total deduction |
| Core->Platform/Identity/Shielded | Mining fee + Platform fee | Progress banner (multi-step, no dialog) |
| Platform->any | Platform fee in credits | Inline fee display, confirm button |
| Identity->any | Platform fee in credits | Inline fee display, confirm button |
| Shielded->any | ZK proof fee in credits | Inline fee display, confirm button |

For all paths: show "Amount + Fee = Total Deduction" before the send button.

### 5.5 Multi-Step Operations: Progress UI

Operations that involve asset locks (Core->Platform, Core->Shielded, Core->Identity) are multi-step:

1. Use `MessageBanner::set_global()` with `.with_elapsed()` for progress
2. Disable the Send button (prevent double-submit)
3. Show step descriptions in the progress banner:
   - "Creating asset lock transaction..."
   - "Waiting for lock proof (this may take 15-30 seconds)..."
   - "Broadcasting Platform transition..."
   - "Success: X DASH sent/shielded/credited"

The existing `SendStatus::WaitingForResult` state handles this. The backend tasks already handle the multi-step logic internally. The UI just needs appropriate progress messaging.

### 5.6 Transaction Type Description

The Send button label should change based on the detected combination:

| Source -> Dest | Button Label |
|---|---|
| Core -> Core | "Send DASH" |
| Core -> Platform | "Fund Platform Address" |
| Core -> Shielded | "Shield DASH" |
| Core -> Identity | "Top Up Identity" |
| Platform -> Core | "Withdraw to Wallet" |
| Platform -> Platform | "Transfer Credits" |
| Platform -> Shielded | "Shield Credits" |
| Platform -> Identity | "Top Up Identity" |
| Identity -> Core | "Withdraw Credits" |
| Identity -> Platform | "Transfer to Address" |
| Identity -> Identity | "Transfer Credits" |
| Shielded -> Core | "Withdraw from Shield" |
| Shielded -> Platform | "Unshield Credits" |
| Shielded -> Shielded | "Private Send" |

This is already partially implemented via `get_transaction_type_description()`. Extend it for the new combinations.

### 5.7 Identity Source: Additional UI Requirements

When "Identity" is selected as source, the UI needs:
1. **Identity selector dropdown**: Shows loaded identities with:
   - DPNS name (if registered)
   - Identity ID (truncated)
   - Credit balance
2. **Key selection** (optional, advanced): Some identity operations accept a specific key ID. Default to `None` (let the backend pick). Show key selector only in developer mode.

Data model change to `SourceSelection`:
```rust
pub enum SourceSelection {
    CoreWallet,
    PlatformAddresses(Vec<(PlatformAddress, Address, u64)>),
    Identity(QualifiedIdentity),  // NEW
    Shielded(WalletSeedHash, u64),
}
```

---

## 6. Progressive Disclosure

### Visibility Matrix

| UI Element | Alex (Everyday) | Priya (Power/Advanced) | Jordan (Developer) |
|---|---|---|---|
| **Source: Core Wallet** | Visible, pre-selected | Visible | Visible |
| **Source: Platform Addresses** | Hidden | Visible (if addresses exist) | Visible |
| **Source: Identity** | Hidden | Visible (if identities loaded) | Visible |
| **Source: Shielded Pool** | Hidden | Hidden | Visible (developer mode) |
| **Dest: Core address** | Visible | Visible | Visible |
| **Dest: Platform address** | Hidden (rejected with message) | Visible | Visible |
| **Dest: Shielded address** | Hidden (rejected with message) | Hidden (rejected with message) | Visible (developer mode) |
| **Dest: Identity ID** | Hidden (rejected with message) | Visible | Visible |
| **Fee strategy selector** | Hidden | Visible (expandable) | Visible |
| **Advanced mode** (multi I/O) | Hidden | Visible (toggle) | Visible |
| **Key ID selector** | Hidden | Hidden | Visible (developer mode) |

### Implementation

The existing `developer_mode` setting and the planned `UserMode` enum (NET-006, currently `[Gap]`) control visibility:

- **Alex sees**: Only `SourceSelection::CoreWallet`. Only `AddressKind::Core` destinations accepted. All other address types show an error message: "This address type requires Advanced mode. You can enable it in Settings."
- **Priya sees**: Core, Platform, Identity sources. Core, Platform, Identity destinations. No shielded.
- **Jordan sees**: Everything. Shielded source and destination require `developer_mode = true`.

Until `UserMode` is implemented (NET-006), the heuristic is:
- Shielded features: behind `developer_mode`
- Platform/Identity features: visible when the wallet has Platform addresses or loaded identities (existing behavior)
- Advanced mode toggle: visible when developer_mode is on or when a user explicitly enables it

### Unsupported Combination Messaging

When a user enters a valid address that creates an unsupported combination for their current mode:

| Situation | Message |
|---|---|
| Alex enters a Platform address | "Platform addresses require Advanced mode. Enable it in Settings to send to Platform addresses." |
| Alex enters a shielded address | "Shielded addresses are a developer feature. Enable Developer mode in Settings." |
| Anyone enters an identity ID with Shielded source | "Sending from the shielded pool to an identity is not yet supported. Transfer to a Platform address first, then top up the identity." |
| Anyone enters a shielded address with Identity source | "Sending from an identity to the shielded pool is not yet supported. Transfer to a Platform address first, then shield from there." |

---

## 7. MCP Tools Reference

### New Tools Needed

| Tool Name | Parameters | Backend Task | Destructive? | Notes |
|---|---|---|---|---|
| `platform_credits_transfer` | `wallet_id`, `to_address` (bech32m), `amount_credits`, `network`? | `WalletTask::TransferPlatformCredits` | Yes (spends credits) | Platform addr -> Platform addr |
| `platform_credits_withdraw` | `wallet_id`, `to_address` (Base58), `amount_credits`, `network`? | `WalletTask::WithdrawFromPlatformAddress` | Yes (spends credits) | Platform addr -> Core addr |
| `platform_address_fund` | `wallet_id`, `to_address` (bech32m), `amount_duffs`, `network`? | `WalletTask::FundPlatformAddressFromWalletUtxos` | Yes (spends DASH) | Core -> Platform addr |
| `identity_credits_topup` | `wallet_id`, `identity_id` (Base58), `amount_duffs`, `network`? | `IdentityTask::TopUpIdentity` (FundWithWallet) | Yes (spends DASH) | Core -> Identity (via asset lock) |
| `identity_credits_topup_from_platform` | `wallet_id`, `identity_id` (Base58), `amount_credits`, `network`? | `IdentityTask::TopUpIdentityFromPlatformAddresses` | Yes (spends credits) | Platform addr -> Identity |
| `identity_credits_transfer` | `wallet_id`, `from_identity_id`, `to_identity_id`, `amount_credits`, `network`? | `IdentityTask::Transfer` | Yes (spends credits) | Identity -> Identity |
| `identity_credits_withdraw` | `wallet_id`, `identity_id`, `to_address` (Base58), `amount_credits`, `network`? | `IdentityTask::WithdrawFromIdentity` | Yes (spends credits) | Identity -> Core |
| `identity_credits_to_address` | `wallet_id`, `identity_id`, `to_address` (bech32m), `amount_credits`, `network`? | `IdentityTask::TransferToAddresses` | Yes (spends credits) | Identity -> Platform addr |
| `shielded_shield_from_core` | `wallet_id`, `amount_duffs`, `network`? | `ShieldedTask::ShieldFromAssetLock` | Yes (spends DASH) | Core -> Shielded |
| `shielded_shield_from_platform` | `wallet_id`, `amount_credits`, `network`? | `ShieldedTask::ShieldCredits` | Yes (spends credits) | Platform addr -> Shielded |
| `shielded_transfer` | `wallet_id`, `to_address` (shielded bech32m), `amount_credits`, `network`? | `ShieldedTask::ShieldedTransfer` | Yes (spends credits) | Shielded -> Shielded |
| `shielded_unshield` | `wallet_id`, `to_address` (bech32m), `amount_credits`, `network`? | `ShieldedTask::UnshieldCredits` | Yes (spends credits) | Shielded -> Platform addr |
| `shielded_withdraw` | `wallet_id`, `to_address` (Base58), `amount_credits`, `network`? | `ShieldedTask::ShieldedWithdrawal` | Yes (spends credits) | Shielded -> Core |

### Existing Tool (already implemented)

| Tool Name | Backend Task | Status |
|---|---|---|
| `core_funds_send` | `CoreTask::SendWalletPayment` | Implemented |

### Tool Annotations

All 13 new tools share these annotations:
- `read_only: false` -- they all mutate state
- `destructive: true` -- they all spend funds
- `idempotent: false` -- repeated calls send more funds
- `open_world: true` -- they all hit the network

### New BackendTask Variants Needed

**None.** All required `BackendTask` variants already exist. The MCP tools are thin adapters that construct the existing task variants and dispatch them.

However, some tools may need minor adjustments in how they resolve parameters:
- `identity_credits_topup` needs to resolve an identity ID to a `QualifiedIdentity` before constructing `IdentityTopUpInfo`. This resolution can use `IdentityTask::LoadIdentity` or direct SDK fetch within the tool (following the pattern in existing identity operations).
- `shielded_shield_from_platform` needs to auto-select the highest-balance Platform address as `from_address`. This selection logic already exists in the Send screen and should be factored into a shared utility.

---

## 8. Data Needs & Processing Rules

### Entities

| Entity | Source | Used By |
|---|---|---|
| Core Wallet balance | SPV sync | Core->* sends, amount validation |
| Platform address list + balances | Platform SDK query | Platform->* sends, source breakdown |
| Loaded identities + credit balances | Platform SDK query | Identity->* sends, source dropdown |
| Shielded pool balance | Local tree + trial decrypt | Shielded->* sends, source display |
| Orchard proving key | Background warm-up | All shielded operations |

### Business Rules

1. **Minimum amounts**: Each backend task enforces minimum amounts (e.g., withdrawal minimum set by Platform). The UI must query and display these before allowing send.
2. **Fee deduction**: For Platform credit operations, fee is deducted from source. The user must have balance >= amount + estimated fee.
3. **Asset lock timeout**: Core->Platform/Identity/Shielded paths create asset locks. If proof is not obtained within ~60 seconds, the operation should fail gracefully without losing funds.
4. **Nonce management**: Platform address operations require correct nonces. The backend handles this, but the UI should not allow concurrent sends from the same source (disable Send button while `SendStatus::WaitingForResult`).
5. **Shielded proving key**: Must be warmed before any shielded operation (~30 seconds cold start). The existing `WarmUpProvingKey` task handles this. The UI should trigger warm-up when the user selects Shielded as source.
6. **Identity key authorization**: Identity operations require a signing key. The backend selects the appropriate key automatically but may fail if no suitable key exists. Error: "This identity does not have a transfer key. Add a transfer key before sending credits."

---

## 9. Prioritized Backlog

### Must Have (P0) -- Core unified send experience

| Item | Rationale |
|---|---|
| Wire Core->Shielded in Send screen | Backend exists (`ShieldFromAssetLock`). Shielded is a headline feature. |
| Wire Core->Identity in Send screen | Backend exists (`TopUpIdentity`). Most requested gap -- users navigate away from Send to top up identities. |
| Wire Platform->Shielded in Send screen | Backend exists (`ShieldCredits`). Completes the shield story. |
| Wire Platform->Identity in Send screen | Backend exists (`TopUpIdentityFromPlatformAddresses`). Second most common identity funding path. |
| Wire Shielded->Core in Send screen | Backend exists (`ShieldedWithdrawal`). Users need a way out of the shielded pool. |
| Add Identity as source type | Requires `SourceSelection::Identity` variant and identity selector dropdown. |
| Wire Identity->Core in Send screen | Backend exists (`WithdrawFromIdentity`). |
| Wire Identity->Platform in Send screen | Backend exists (`TransferToAddresses`). |
| Wire Identity->Identity in Send screen | Backend exists (`Transfer`). |
| Progressive disclosure for sources | Alex should only see Core Wallet. Others gated by mode. |
| MCP tool: `core_funds_send` | Already implemented. |
| MCP tool: `platform_credits_transfer` | High-demand programmatic operation. |
| MCP tool: `platform_credits_withdraw` | Needed for automated testing workflows. |
| MCP tool: `platform_address_fund` | Needed for automated setup workflows. |

### Should Have (P1) -- MCP completeness and polish

| Item | Rationale |
|---|---|
| MCP tool: `identity_credits_topup` | Automates the most common multi-step identity operation. |
| MCP tool: `identity_credits_transfer` | Identity-to-identity is common in test setups. |
| MCP tool: `identity_credits_withdraw` | Completes identity MCP coverage. |
| MCP tool: `identity_credits_to_address` | Identity -> Platform address for MCP. |
| MCP tool: `shielded_shield_from_core` | Long-running (~30s) -- useful as CLI command for scripts. |
| MCP tool: `shielded_shield_from_platform` | Completes shield MCP coverage. |
| MCP tool: `shielded_transfer` | Private transfer via CLI. |
| MCP tool: `shielded_unshield` | Unshield via CLI. |
| MCP tool: `shielded_withdraw` | Withdraw shielded to core via CLI. |
| Dynamic button labels per combination | Already partially implemented. Extend `get_transaction_type_description()`. |
| Multi-step progress messaging | Use `with_elapsed()` banners for asset lock flows. |

### Could Have (P2) -- Enhanced experience

| Item | Rationale |
|---|---|
| Identity resolution on paste | When user pastes an identity ID, resolve DPNS name and show it. |
| Proving key warm-up on source select | Pre-warm Orchard key when Shielded source is selected. |
| Invalid combination inline warnings | Show helpful "transfer to Platform first" messages. |
| Fee preview for all combinations | Standardize fee display across all 12 paths. |
| Advanced mode for Identity source | Multi-output identity transfers (to multiple addresses/identities). |

### Won't Have (v1 scope exclusion)

| Item | Rationale |
|---|---|
| Identity->Shielded | No direct protocol path exists. Requires two-step (Identity->Platform->Shielded). Not worth automating in v1 -- too niche. |
| Shielded->Identity | Same constraint. Requires Shielded->Platform->Identity chain. |
| Send-to-DPNS-username (SND-004) | Separate feature with its own UX implications. Out of scope for this task. |
| Batch sends (multiple destinations from one source) | Already partially implemented in advanced mode for Core/Platform. Not extending to Identity/Shielded in v1. |
| Cross-wallet sends | Sending from one wallet to fund another wallet's identity. Out of scope. |

---

## 10. Open Questions & Assumptions

### Open Questions

1. **Identity source: which identities to show?** Only identities loaded for the current wallet, or all loaded identities across all wallets? Recommendation: current wallet only, to avoid confusion.

2. **Platform->Shielded: which Platform address to use as source?** Auto-select highest balance? Let user choose? Recommendation: auto-select highest balance (matching existing `ShieldCredits` pattern which takes a single `from_address`).

3. **Core->Identity: identity resolution latency.** Resolving an identity from a pasted ID requires a network call. Should this block the UI or happen asynchronously? Recommendation: async resolution with a loading indicator next to the destination field.

4. **Should Identity operations remain on the Identity screen too?** If we add Identity->* to the Send screen, should the existing Identity screen buttons (Transfer, Withdraw, Top Up) remain as shortcuts, or redirect to the Send screen? Recommendation: keep both -- the Identity screen provides context-specific shortcuts; the Send screen provides the unified "send from anywhere" experience.

5. **Shielded operations: developer mode or separate toggle?** Currently behind developer_mode. Should there be a more granular "enable shielded features" toggle? Recommendation: keep behind developer_mode for v1. Revisit when shielded features are production-ready.

### Assumptions

1. **No new `BackendTask` variants are needed.** All 14 viable Source->Destination paths map to existing backend tasks.
2. **`AddressInput` already handles all 4 `AddressKind` variants.** No changes needed to the address input component.
3. **Identity resolution (ID -> DPNS name) is fast enough** (~1-2s) for inline display.
4. **The `WalletSendScreen` can hold an optional `QualifiedIdentity`** for the Identity source without significantly complicating its state.
5. **MCP tools can resolve `QualifiedIdentity` from an identity ID string** by loading the identity from the network (using existing `IdentityTask::LoadIdentity` or direct SDK fetch).
6. **Asset lock operations for Core->Shielded use the same wallet-managed flow** as Core->Platform (the backend task `ShieldFromAssetLock` already handles this end-to-end).

### Success Metrics

| Metric | Target |
|---|---|
| All 14 viable Source->Dest combinations accessible from one screen | 14/14 |
| Alex can send DASH without seeing Platform/Identity/Shielded UI | 100% |
| Time to top up identity from Send screen (Priya) | Under 30 seconds (includes asset lock wait) |
| MCP tool coverage for send operations | 14 tools (1 existing + 13 new) |
| Zero new `BackendTask` variants required | 0 new variants |
