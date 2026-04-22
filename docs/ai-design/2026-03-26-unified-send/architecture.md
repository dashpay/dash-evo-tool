# Unified Send Screen: Architecture Document

## 1. System Layers Trace

Every Source x Destination combination follows the same architectural path through the system. Below are the 8 new combinations that need wiring, traced through each layer.

### 1.1 Core Wallet -> Shielded (SND-007)

```
UI: validate_and_send() matches (CoreWallet, Shielded)
  -> send_core_to_shielded(seed_hash)
  -> AppAction::BackendTask(BackendTask::ShieldedTask(
       ShieldedTask::ShieldFromAssetLock { seed_hash, amount_duffs }
     ))
Backend: AppContext::run_shielded_task()
  -> creates asset lock from wallet UTXOs
  -> waits for IS-lock/chain-lock proof
  -> shields credits into Orchard pool
  -> sender.send(TaskResult::Success(ShieldedCreditsShielded { seed_hash, amount }))
UI: display_task_result() matches ShieldedCreditsShielded
  -> sets SendStatus::Complete with success message
  -> queues SyncNotes refresh task
```

### 1.2 Core Wallet -> Identity (SND-008, Core path)

```
UI: validate_and_send() matches (CoreWallet, Identity)
  -> send_core_to_identity(seed_hash)
  -> resolves identity from validated_destination (Identifier)
  -> loads QualifiedIdentity from DB or network
  -> constructs IdentityTopUpInfo with FundWithWallet
  -> AppAction::BackendTask(BackendTask::IdentityTask(
       IdentityTask::TopUpIdentity(IdentityTopUpInfo { ... })
     ))
Backend: AppContext::run_identity_task()
  -> creates asset lock, waits for proof, broadcasts top-up transition
  -> sender.send(TaskResult::Success(ToppedUpIdentity(identity, fee_result)))
UI: display_task_result() matches ToppedUpIdentity
  -> sets SendStatus::Complete with identity info and fee
```

### 1.3 Platform -> Shielded (SND-009)

```
UI: validate_and_send() matches (PlatformAddresses(_), Shielded)
  -> send_platform_to_shielded(seed_hash, addresses)
  -> auto-selects highest-balance platform address as from_address
  -> AppAction::BackendTask(BackendTask::ShieldedTask(
       ShieldedTask::ShieldCredits { seed_hash, amount, from_address, nonce_override: None }
     ))
Backend: AppContext::run_shielded_task()
  -> Type 15 state transition
  -> sender.send(TaskResult::Success(ShieldedCreditsShielded { seed_hash, amount }))
UI: display_task_result() matches ShieldedCreditsShielded
  -> sets SendStatus::Complete
  -> queues SyncNotes refresh
```

### 1.4 Platform -> Identity (SND-008, Platform path)

```
UI: validate_and_send() matches (PlatformAddresses(addrs), Identity)
  -> send_platform_to_identity(seed_hash, addresses)
  -> resolves QualifiedIdentity from destination Identifier
  -> allocates platform addresses (reuses allocate_platform_addresses)
  -> AppAction::BackendTask(BackendTask::IdentityTask(
       IdentityTask::TopUpIdentityFromPlatformAddresses { identity, inputs, wallet_seed_hash }
     ))
Backend: AppContext::run_identity_task()
  -> top_up_from_platform_addresses()
  -> sender.send(TaskResult::Success(ToppedUpIdentity(identity, fee_result)))
UI: display_task_result() matches ToppedUpIdentity
```

### 1.5 Shielded -> Core (SND-010)

```
UI: validate_and_send() matches (Shielded(sh, _), Core)
  -> send_shielded_to_core(seed_hash)
  -> parses Core address from validated_destination
  -> AppAction::BackendTask(BackendTask::ShieldedTask(
       ShieldedTask::ShieldedWithdrawal { seed_hash, amount, to_core_address }
     ))
Backend: AppContext::run_shielded_task()
  -> Type 19 state transition
  -> sender.send(TaskResult::Success(ShieldedWithdrawalComplete { seed_hash, amount }))
UI: display_task_result() matches ShieldedWithdrawalComplete
  -> sets SendStatus::Complete with withdrawal note
  -> queues SyncNotes refresh
```

### 1.6 Identity -> Core (SND-012)

```
UI: validate_and_send() matches (Identity(qi), Core)
  -> send_identity_to_core(qualified_identity)
  -> parses Core address, extracts amount in credits
  -> AppAction::BackendTask(BackendTask::IdentityTask(
       IdentityTask::WithdrawFromIdentity(qi, Some(address), credits, None)
     ))
Backend: AppContext::run_identity_task()
  -> withdraw_from_identity()
  -> sender.send(TaskResult::Success(WithdrewFromIdentity(fee_result)))
UI: display_task_result() matches WithdrewFromIdentity
```

### 1.7 Identity -> Platform (SND-013)

```
UI: validate_and_send() matches (Identity(qi), Platform)
  -> send_identity_to_platform(qualified_identity)
  -> extracts PlatformAddress from destination, amount in credits
  -> AppAction::BackendTask(BackendTask::IdentityTask(
       IdentityTask::TransferToAddresses { identity: qi, outputs, key_id: None }
     ))
Backend: AppContext::run_identity_task()
  -> transfer_to_addresses()
  -> sender.send(TaskResult::Success(TransferredCredits(fee_result)))
UI: display_task_result() matches TransferredCredits
```

### 1.8 Identity -> Identity (SND-011)

```
UI: validate_and_send() matches (Identity(qi), Identity)
  -> send_identity_to_identity(qualified_identity)
  -> extracts destination Identifier, amount in credits
  -> AppAction::BackendTask(BackendTask::IdentityTask(
       IdentityTask::Transfer(qi, to_identifier, credits, None)
     ))
Backend: AppContext::run_identity_task()
  -> transfer_to_identity()
  -> sender.send(TaskResult::Success(TransferredCredits(fee_result)))
UI: display_task_result() matches TransferredCredits
```

---

## 2. Code Placement

### 2.1 Files Modified vs New Files

| File | Change Type | Description |
|------|------------|-------------|
| `src/ui/wallets/send_screen.rs` | **Modify** | Add Identity to `SourceSelection`, add 8 new `send_*` methods, extend `validate_and_send()` match, extend `get_transaction_type_description()`, extend `display_task_result()`, add identity selector UI in `render_source_selection()` |
| `src/ui/wallets/send_screen/routing.rs` | **New** | Extract `validate_and_send()` routing logic and all `send_*` handler methods (~700 lines moved + ~300 new) |
| `src/ui/wallets/send_screen/identity_source.rs` | **New** | Identity selector dropdown widget and Identity-related source UI logic |
| `src/mcp/tools/identity.rs` | **New** | 5 identity MCP tools (`identity_credits_topup`, `identity_credits_topup_from_platform`, `identity_credits_transfer`, `identity_credits_withdraw`, `identity_credits_to_address`) |
| `src/mcp/tools/shielded.rs` | **New** | 5 shielded MCP tools (`shielded_shield_from_core`, `shielded_shield_from_platform`, `shielded_transfer`, `shielded_unshield`, `shielded_withdraw`) |
| `src/mcp/tools/wallet.rs` | **Modify** | 3 new platform tools (`platform_credits_transfer`, `platform_credits_withdraw`, `platform_address_fund`) |
| `src/mcp/tools/mod.rs` | **Modify** | Add `pub mod identity; pub mod shielded;` |
| `src/mcp/server.rs` | **Modify** | Register 13 new tools in `tool_router()` |
| `src/mcp/resolve.rs` | **Modify** | Add `resolve::qualified_identity()` helper |
| `docs/MCP.md` | **Modify** | Add 13 new tools to reference table |
| `docs/user-stories.md` | **Modify** | Add SND-007 through SND-013, MCP-003 through MCP-012 |

### 2.2 Send Screen File Decomposition

The current `send_screen.rs` is 2901 lines in a single file. Rather than adding ~500 more lines of routing and identity UI to it, we decompose it into a module directory:

```
src/ui/wallets/send_screen/
  mod.rs              -- WalletSendScreen struct, ui(), ScreenLike impl, render methods
                         (~1800 lines, moved from send_screen.rs)
  routing.rs          -- validate_and_send(), validate_and_send_advanced(),
                         all send_* handler methods (~1000 lines)
  identity_source.rs  -- Identity source selector widget, identity resolution (~200 lines)
```

The existing `src/ui/wallets/send_screen.rs` becomes `src/ui/wallets/send_screen/mod.rs`. The `mod.rs` file re-exports the `WalletSendScreen` type so external references do not change.

### 2.3 Identity Source Integration

The `SourceSelection` enum gains an `Identity` variant:

```rust
pub enum SourceSelection {
    CoreWallet,
    PlatformAddresses(Vec<(PlatformAddress, Address, u64)>),
    Identity(QualifiedIdentity),  // NEW
    Shielded(WalletSeedHash, u64),
}
```

In `render_source_selection()`, the Identity radio button appears between Platform Addresses and Shielded Pool. It is visible when:
- `developer_mode` is true, OR
- the wallet has loaded identities with non-zero credit balance

When selected, a dropdown lists loaded identities (from `AppContext::get_local_qualified_identities()`) showing:
- DPNS name (if registered), otherwise truncated Identity ID
- Credit balance

Selection populates `self.selected_source` with `SourceSelection::Identity(chosen_identity)`.

### 2.4 Progressive Disclosure Logic

Progressive disclosure is implemented at the **screen level** in `render_source_selection()` and `validate_and_send()`, not in individual components. The existing `AddressInput` component remains unchanged -- it always detects all four `AddressKind` variants. The screen-level logic gates what the user can do:

- **Source visibility**: Each radio button has a visibility guard in `render_source_selection()`:
  - Platform: shown when wallet has platform addresses OR `developer_mode`
  - Identity: shown when wallet has loaded identities OR `developer_mode`
  - Shielded: shown only when `developer_mode`

- **Destination rejection**: In `validate_and_send()`, before routing, check the source x destination combination against a validity matrix. Invalid combinations return a user-friendly error string (not a panic). Unsupported combinations (Identity->Shielded, Shielded->Identity) return a specific message suggesting the two-step workaround.

---

## 3. Send Screen Refactoring Plan

### 3.1 Module Extraction Strategy

The 2901-line monolith splits cleanly because the internal methods fall into three groups:

1. **Rendering** (~1800 lines): `ui()`, `render_source_selection()`, `render_amount_section()`, `render_advanced_mode()`, `render_fee_display()`, all the UI layout code. Stays in `mod.rs`.

2. **Routing/Handlers** (~1000 lines): `validate_and_send()`, `validate_and_send_advanced()`, `send_core_to_core()`, `send_core_to_platform()`, `send_platform_to_platform()`, `send_platform_to_core()`, `send_shielded_to_shielded()`, `send_shielded_to_platform()`, plus the 8 new handlers. Moves to `routing.rs` as `impl WalletSendScreen` methods.

3. **Identity source** (~200 lines, all new): Identity dropdown widget, identity list fetching. New file `identity_source.rs`.

The struct definition and `ScreenLike` trait impl stay in `mod.rs`. The routing and identity modules access `WalletSendScreen` fields through `pub(super)` visibility. No public API changes.

### 3.2 New Routing Arms

The `validate_and_send()` match grows from 6 arms to 14 (minus 2 deferred = 12 valid + 2 error):

```rust
match (source.clone(), dest_kind) {
    // Existing 6
    (CoreWallet, Some(Core))               => self.send_core_to_core(),
    (CoreWallet, Some(Platform))           => self.send_core_to_platform(seed_hash),
    (PlatformAddresses(a), Some(Platform)) => self.send_platform_to_platform(seed_hash, a),
    (PlatformAddresses(a), Some(Core))     => self.send_platform_to_core(seed_hash, a),
    (Shielded(sh, _), Some(Shielded))      => self.send_shielded_to_shielded(sh),
    (Shielded(sh, _), Some(Platform))      => self.send_shielded_to_platform(sh),

    // New 8 (backend exists, wiring needed)
    (CoreWallet, Some(Shielded))           => self.send_core_to_shielded(seed_hash),
    (CoreWallet, Some(Identity))           => self.send_core_to_identity(seed_hash),
    (PlatformAddresses(a), Some(Shielded)) => self.send_platform_to_shielded(seed_hash, a),
    (PlatformAddresses(a), Some(Identity)) => self.send_platform_to_identity(seed_hash, a),
    (Shielded(sh, _), Some(Core))          => self.send_shielded_to_core(sh),
    (Identity(qi), Some(Core))             => self.send_identity_to_core(qi),
    (Identity(qi), Some(Platform))         => self.send_identity_to_platform(qi),
    (Identity(qi), Some(Identity))         => self.send_identity_to_identity(qi),

    // Impossible combinations (explicit error)
    (Identity(_), Some(Shielded)) => Err(
        "Sending from an identity to the shielded pool is not yet supported. \
         Transfer to a Platform address first, then shield from there.".into()
    ),
    (Shielded(..), Some(Identity)) => Err(
        "Sending from the shielded pool to an identity is not yet supported. \
         Transfer to a Platform address first, then top up the identity.".into()
    ),

    _ => Err("Invalid source/destination combination".into()),
}
```

### 3.3 Multi-Step Operation Progress

Asset-lock flows (Core->Platform, Core->Shielded, Core->Identity) already handle multi-step logic internally in the backend. The UI pattern is consistent:

1. `set_send_progress_banner(ctx)` with `with_elapsed()` -- already implemented
2. `send_status = SendStatus::WaitingForResult` -- already implemented
3. Backend sends `Progress` results for step updates -- already supported
4. Final result arrives in `display_task_result()` -- extend with new match arms

No new progress pattern needed. The existing `SendStatus::WaitingForResult` + `MessageBanner::set_global()` with `with_elapsed()` covers all cases.

### 3.4 display_task_result() Extensions

Add match arms for results from the 8 new routing paths:

```rust
// Already handled: WalletPayment, TransferredCredits, PlatformAddressFunded,
//                  PlatformAddressWithdrawal, PlatformCreditsTransferred,
//                  ShieldedTransferComplete, ShieldedCreditsUnshielded

// NEW match arms:
ToppedUpIdentity(identity, fee_result) => {
    // Core->Identity or Platform->Identity
    self.send_status = SendStatus::Complete(format!(
        "Identity topped up successfully!\n\nFee: Estimated {} - Actual {}",
        format_credits_as_dash(fee_result.estimated_fee),
        format_credits_as_dash(fee_result.actual_fee)
    ));
}
WithdrewFromIdentity(fee_result) => {
    // Identity->Core
    self.send_status = SendStatus::Complete(
        "Identity withdrawal initiated. \
         Funds will appear on the Core chain after confirmation.".into()
    );
}
ShieldedCreditsShielded { seed_hash, amount } => {
    // Core->Shielded or Platform->Shielded
    self.send_status = SendStatus::Complete(format!(
        "{} shielded successfully!\n\n\
         Balance will update after the next block.",
        format_credits_as_dash(amount)
    ));
    self.pending_refresh_task = Some(
        BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash })
    );
}
ShieldedWithdrawalComplete { seed_hash, amount } => {
    // Shielded->Core
    self.send_status = SendStatus::Complete(format!(
        "Withdrawal of {} from shielded pool initiated.\n\n\
         Funds will appear after confirmation.",
        format_credits_as_dash(amount)
    ));
    self.pending_refresh_task = Some(
        BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash })
    );
}
```

---

## 4. MCP Tools Architecture

### 4.1 File Organization

```
src/mcp/tools/
  mod.rs           -- shared param types (extend)
  meta.rs          -- existing meta tools
  network.rs       -- existing network tools
  wallet.rs        -- existing + 3 new platform tools
  identity.rs      -- NEW: 5 identity tools
  shielded.rs      -- NEW: 5 shielded tools
```

The 13 new tools split by domain:
- **wallet.rs** (3 tools): `platform_credits_transfer`, `platform_credits_withdraw`, `platform_address_fund` -- these operate on wallet-owned platform addresses
- **identity.rs** (5 tools): `identity_credits_topup`, `identity_credits_topup_from_platform`, `identity_credits_transfer`, `identity_credits_withdraw`, `identity_credits_to_address`
- **shielded.rs** (5 tools): `shielded_shield_from_core`, `shielded_shield_from_platform`, `shielded_transfer`, `shielded_unshield`, `shielded_withdraw`

### 4.2 Shared Parameter Types

Each tool defines its own parameter struct for self-documenting JSON schemas. However, common patterns emerge:

- All 13 tools require `wallet_id: String` and `network: Option<String>`
- Identity tools add `identity_id: String`
- Amount is either `amount_duffs: u64` (Core-sourced) or `amount_credits: u64` (Platform/Shielded/Identity-sourced)
- Address destination tools add `to_address: String`

Per the MCP tool pattern, inline param structs are preferred over shared types when field descriptions differ. The existing `WalletIdParams` can be reused by read-only tools but destructive tools typically need additional fields.

### 4.3 Identity Resolution Helper

Add to `src/mcp/resolve.rs`:

```rust
/// Resolve an identity ID string to a QualifiedIdentity.
/// Tries local DB first. Returns error if not found locally.
pub(crate) fn qualified_identity(
    ctx: &AppContext,
    identity_id_str: &str,
) -> Result<QualifiedIdentity, McpToolError> {
    let identifier = Identifier::from_string(identity_id_str, Encoding::Base58)
        .map_err(|_| McpToolError::InvalidParam {
            message: format!("Invalid identity ID: {identity_id_str}"),
        })?;

    ctx.get_identity_by_id(&identifier)
        .map_err(|e| McpToolError::Internal(e.to_string()))?
        .ok_or_else(|| McpToolError::InvalidParam {
            message: format!(
                "Identity not found locally: {identity_id_str}. \
                 Load the identity first using the identity screen or CLI."
            ),
        })
}
```

### 4.4 SPV Sync Requirements

| Tool | Needs `ensure_spv_synced()`? | Reason |
|------|------|--------|
| `platform_credits_transfer` | No | Platform-only operation |
| `platform_credits_withdraw` | No | Platform-only operation |
| `platform_address_fund` | **Yes** | Creates asset lock from wallet UTXOs |
| `identity_credits_topup` | **Yes** | Creates asset lock from wallet UTXOs |
| `identity_credits_topup_from_platform` | No | Platform-only operation |
| `identity_credits_transfer` | No | Platform-only operation |
| `identity_credits_withdraw` | No | Platform-only operation |
| `identity_credits_to_address` | No | Platform-only operation |
| `shielded_shield_from_core` | **Yes** | Creates asset lock from wallet UTXOs |
| `shielded_shield_from_platform` | No | Platform-only operation |
| `shielded_transfer` | No | Platform-only operation |
| `shielded_unshield` | No | Platform-only operation |
| `shielded_withdraw` | No | Platform-only operation |

### 4.5 Network Requirement

All 13 new tools are destructive (they spend funds). Per existing convention in `resolve.rs`, destructive operations must use `resolve::require_network()` rather than `resolve::verify_network()`. This makes the `network` parameter mandatory, preventing accidental cross-network transfers.

---

## 5. Work Decomposition

### Task Dependency Graph

```
T1 (refactor send_screen into module) -----+
                                            |-- T3 (identity source UI)
T2 (wire 5 non-identity send combos) ------+
                                            +-- T4 (identity routing arms)

T5 (MCP platform tools) --- independent
T6 (MCP identity tools) --- independent
T7 (MCP shielded tools) --- independent
T8 (docs + user stories) -- after T2-T7
```

### Task Definitions

#### T1: Refactor Send Screen into Module Directory

**Files affected:**
- `src/ui/wallets/send_screen.rs` -> `src/ui/wallets/send_screen/mod.rs`
- New: `src/ui/wallets/send_screen/routing.rs`

**What to do:**
1. Convert `send_screen.rs` to `send_screen/mod.rs` (rename, create directory)
2. Move `validate_and_send()`, `validate_and_send_advanced()`, and all `send_*` methods to `routing.rs`
3. Move top-level helper functions (`estimate_platform_fee`, `estimate_withdrawal_fee_from_transition`, `estimate_address_funding_fee_from_transition`, `allocate_platform_addresses`, `allocate_platform_addresses_with_fee`) to `routing.rs` as well
4. Make required struct fields `pub(super)` for cross-module access
5. Update `src/ui/wallets/mod.rs` to reference the new module path
6. Verify all tests pass, clippy clean

**Estimated lines:** ~100 new/changed (mostly structural, moving existing code)
**Dependencies:** None
**Agent:** rust-implementer

#### T2: Wire 5 Non-Identity New Send Combinations

**Files affected:**
- `src/ui/wallets/send_screen/routing.rs`
- `src/ui/wallets/send_screen/mod.rs` -- extend `get_transaction_type_description()`, extend `display_task_result()`

**What to do:**
1. Add 5 new `send_*` methods:
   - `send_core_to_shielded(seed_hash)` -- dispatches `ShieldedTask::ShieldFromAssetLock`
   - `send_core_to_identity(seed_hash)` -- resolves identity from destination Identifier, constructs `IdentityTopUpInfo` with `FundWithWallet`, dispatches `IdentityTask::TopUpIdentity`
   - `send_platform_to_shielded(seed_hash, addresses)` -- auto-selects highest-balance address, dispatches `ShieldedTask::ShieldCredits`
   - `send_platform_to_identity(seed_hash, addresses)` -- resolves identity, allocates platform addresses, dispatches `IdentityTask::TopUpIdentityFromPlatformAddresses`
   - `send_shielded_to_core(seed_hash)` -- parses Core address, dispatches `ShieldedTask::ShieldedWithdrawal`
2. Note: Core->Identity and Platform->Identity resolve the identity from the destination field (no Identity source selector needed). They require the wallet to have the identity loaded locally (via `AppContext::get_identity_by_id`). If not found, return a user-friendly error.
3. Extend `validate_and_send()` match with 5 new arms + 2 error arms for impossible combinations
4. Extend `get_transaction_type_description()` with labels for all new combinations
5. Extend `display_task_result()` with `ToppedUpIdentity`, `WithdrewFromIdentity`, `ShieldedCreditsShielded`, `ShieldedWithdrawalComplete` match arms
6. Add progressive disclosure guards for destination validation

**Estimated lines:** ~350 new/changed
**Dependencies:** T1 (soft -- can be done in monolith and moved later, but cleaner after T1)
**Agent:** rust-implementer

#### T3: Add Identity Source UI

**Files affected:**
- New: `src/ui/wallets/send_screen/identity_source.rs`
- `src/ui/wallets/send_screen/mod.rs` -- add `Identity` variant to `SourceSelection`, add identity fields to `WalletSendScreen`, add identity radio button to `render_source_selection()`

**What to do:**
1. Add `Identity(QualifiedIdentity)` variant to `SourceSelection`
2. Add `selected_identity: Option<QualifiedIdentity>` field to `WalletSendScreen`
3. Create identity selector dropdown in `identity_source.rs`: fetches loaded identities from `AppContext::get_local_qualified_identities()`, displays DPNS name + truncated ID + balance, updates `selected_source` on selection
4. Filter identities by the current wallet's seed hash to avoid cross-wallet confusion
5. Add Identity radio button in `render_source_selection()` between Platform and Shielded, with visibility guard (visible when identities exist or developer_mode)
6. Handle amount unit display (credits) when Identity is selected

**Estimated lines:** ~250 new
**Dependencies:** T1 (for module structure)
**Agent:** rust-implementer

#### T4: Wire 3 Identity-Source Send Combinations

**Files affected:**
- `src/ui/wallets/send_screen/routing.rs`
- `src/ui/wallets/send_screen/mod.rs` -- minor display_task_result additions

**What to do:**
1. Add 3 new `send_*` methods:
   - `send_identity_to_core(qi)` -- parses Core address, dispatches `IdentityTask::WithdrawFromIdentity(qi, Some(address), credits, None)`
   - `send_identity_to_platform(qi)` -- extracts Platform address, dispatches `IdentityTask::TransferToAddresses { identity: qi, outputs, key_id: None }`
   - `send_identity_to_identity(qi)` -- extracts destination Identifier, dispatches `IdentityTask::Transfer(qi, to_identifier, credits, None)`
2. Add 3 match arms in `validate_and_send()` for Identity source
3. `WithdrewFromIdentity` and `TransferredCredits` results already have match arms from T2. Verify they display correctly for the Identity source case.

**Estimated lines:** ~200 new
**Dependencies:** T3 (Identity source must exist to select it)
**Agent:** rust-implementer

#### T5: MCP Platform Tools (3 tools)

**Files affected:**
- `src/mcp/tools/wallet.rs` -- add 3 tools
- `src/mcp/server.rs` -- register 3 tools
- `src/mcp/resolve.rs` -- add platform address resolution helper if needed

**What to do:**
1. `platform_credits_transfer`: resolve wallet, read platform address balances from wallet state, parse destination bech32m address, allocate source addresses (highest balance first), dispatch `WalletTask::TransferPlatformCredits`
2. `platform_credits_withdraw`: resolve wallet, read platform address balances, parse Core address to `CoreScript`, allocate source addresses, dispatch `WalletTask::WithdrawFromPlatformAddress`
3. `platform_address_fund`: resolve wallet, parse bech32m destination, dispatch `WalletTask::FundPlatformAddressFromWalletUtxos`. Requires `ensure_spv_synced()`.
4. All 3 use `require_network()`. Tools 1-2 skip SPV sync, tool 3 needs it.
5. Register all 3 in `tool_router()`

**Estimated lines:** ~350 new
**Dependencies:** None
**Agent:** rust-implementer

#### T6: MCP Identity Tools (5 tools)

**Files affected:**
- New: `src/mcp/tools/identity.rs`
- `src/mcp/tools/mod.rs` -- add `pub mod identity;`
- `src/mcp/server.rs` -- register 5 tools
- `src/mcp/resolve.rs` -- add `resolve::qualified_identity()` helper

**What to do:**
1. Add `resolve::qualified_identity()` to `resolve.rs` -- parses Base58 Identifier, looks up in local DB via `AppContext::get_identity_by_id()`
2. `identity_credits_topup`: resolve wallet + identity, determine identity index and top-up index from `QualifiedIdentity`, construct `IdentityTopUpInfo` with `FundWithWallet(amount_duffs, identity_index, top_up_index)`, dispatch `IdentityTask::TopUpIdentity`. Requires `ensure_spv_synced()`.
3. `identity_credits_topup_from_platform`: resolve wallet + identity, read platform address balances, auto-allocate inputs (highest balance first), dispatch `IdentityTask::TopUpIdentityFromPlatformAddresses`
4. `identity_credits_transfer`: resolve wallet + from_identity (via `qualified_identity()`), parse to_identity_id as `Identifier`, dispatch `IdentityTask::Transfer(qi, to_id, credits, None)`
5. `identity_credits_withdraw`: resolve wallet + identity, parse Core address, dispatch `IdentityTask::WithdrawFromIdentity(qi, Some(address), credits, None)`
6. `identity_credits_to_address`: resolve wallet + identity, parse Platform bech32m address, dispatch `IdentityTask::TransferToAddresses { identity, outputs, key_id: None }`
7. Register all 5 in `tool_router()`

**Estimated lines:** ~500 new
**Dependencies:** None (identity resolution added inline in resolve.rs)
**Agent:** rust-implementer

#### T7: MCP Shielded Tools (5 tools)

**Files affected:**
- New: `src/mcp/tools/shielded.rs`
- `src/mcp/tools/mod.rs` -- add `pub mod shielded;`
- `src/mcp/server.rs` -- register 5 tools

**What to do:**
1. `shielded_shield_from_core`: resolve wallet, dispatch `ShieldedTask::ShieldFromAssetLock { seed_hash, amount_duffs }`. Requires `ensure_spv_synced()`. Long-running (~30s).
2. `shielded_shield_from_platform`: resolve wallet, read platform address balances, auto-select highest-balance address, dispatch `ShieldedTask::ShieldCredits { seed_hash, amount, from_address, nonce_override: None }`
3. `shielded_transfer`: resolve wallet, parse shielded bech32m address via `OrchardAddress::from_bech32m_string()` to raw bytes, dispatch `ShieldedTask::ShieldedTransfer { seed_hash, amount, recipient_address_bytes }`
4. `shielded_unshield`: resolve wallet, parse platform bech32m address via `PlatformAddress::from_bech32m_string()`, dispatch `ShieldedTask::UnshieldCredits { seed_hash, amount, to_platform_address }`
5. `shielded_withdraw`: resolve wallet, parse Core address via `Address::from_str()`, dispatch `ShieldedTask::ShieldedWithdrawal { seed_hash, amount, to_core_address }`
6. All use `require_network()`. Only tool 1 needs `ensure_spv_synced()`.
7. Register all 5 in `tool_router()`

**Estimated lines:** ~400 new
**Dependencies:** None
**Agent:** rust-implementer

#### T8: Documentation Updates

**Files affected:**
- `docs/MCP.md`
- `docs/user-stories.md`

**What to do:**
1. Add 13 new tools to the MCP reference table in `docs/MCP.md`
2. Add user stories SND-007 through SND-013 and MCP-003 through MCP-012 to `docs/user-stories.md` (tagged `[Implemented]`)
3. Mark any existing `[Gap]` stories that are now implemented

**Estimated lines:** ~200 new
**Dependencies:** T2-T7 (document after implementation)
**Agent:** docs-writer or rust-implementer

### Task Summary Table

| ID | Title | Lines | Depends On | Agent |
|----|-------|-------|------------|-------|
| T1 | Refactor send screen into module directory | ~100 | None | rust-implementer |
| T2 | Wire 5 non-identity send combinations | ~350 | T1 (soft) | rust-implementer |
| T3 | Add Identity source UI | ~250 | T1 | rust-implementer |
| T4 | Wire 3 identity-source send combinations | ~200 | T3 | rust-implementer |
| T5 | MCP platform tools (3) | ~350 | None | rust-implementer |
| T6 | MCP identity tools (5) | ~500 | None | rust-implementer |
| T7 | MCP shielded tools (5) | ~400 | None | rust-implementer |
| T8 | Documentation updates | ~200 | T2-T7 | docs-writer |

**Parallelization:**
- T1 runs first (short, structural)
- T2 + T3 can run in parallel after T1
- T4 runs after T3
- T5, T6, T7 are fully independent of T1-T4 and of each other -- all three can run in parallel
- T8 runs last

**Maximum parallelism: T2 + T3 + T5 + T6 + T7 (5 tasks in parallel)**

---

## 6. Risk Assessment

### 6.1 Identity as Source: Key Authorization

**Risk:** Identity operations require a signing key with appropriate purpose (TRANSFER). If the loaded `QualifiedIdentity` lacks such a key, the backend will return an error.

**Mitigation:**
- The backend already handles this -- `IdentityTask::Transfer`, `WithdrawFromIdentity`, and `TransferToAddresses` all select the appropriate key automatically and return a `TaskError` if none exists.
- The UI should display the error clearly: "This identity does not have a transfer key. Add a transfer key on the Identity screen before sending credits."
- Pre-validation (P2 scope): before enabling the Send button, check if the selected identity has a usable key. This is an enhancement, not blocking.

### 6.2 Identity Selector: Which Identities to Show

**Risk:** Showing all loaded identities across all wallets could confuse users. An identity loaded from wallet A should not appear when wallet B is selected.

**Mitigation:**
- Filter identities by the currently selected wallet's seed hash. Use `QualifiedIdentity`'s wallet association to match.
- If the identity has no wallet association (manually loaded), show it only in developer mode.

### 6.3 Multi-Step Operation Error Recovery (Asset Lock Flows)

**Risk:** Core->Shielded creates an asset lock. If the shield step fails after the asset lock is confirmed, the user's DASH is locked on-chain but not shielded.

**Mitigation:**
- The backend tasks (`ShieldFromAssetLock`, `TopUpIdentity` with `FundWithWallet`) already handle the full end-to-end flow atomically. If shielding fails, the asset lock remains on-chain but can be recovered via `RecoverAssetLocks`.
- The error message should guide the user: "The asset lock was created but shielding failed. Your funds are safe. Use 'Recover Asset Locks' on the wallet screen to reclaim them, then try again."
- This is existing behavior for Core->Platform as well -- no new risk introduced.

### 6.4 Platform Address Auto-Selection for Shielding

**Risk:** When shielding from Platform (Platform->Shielded), the system auto-selects the highest-balance platform address. The user may want to choose a specific address.

**Mitigation:**
- For v1, auto-select highest balance (matches existing `ShieldCredits` backend pattern which takes a single `from_address`).
- The MCP tool `shielded_shield_from_platform` takes `wallet_id` and amount; it auto-selects the address internally.
- P2 enhancement: add optional address selection in advanced mode.

### 6.5 Concurrent Sends from Same Source

**Risk:** Two concurrent Platform or Identity sends could conflict on nonces.

**Mitigation:**
- The existing `SendStatus::WaitingForResult` pattern disables the Send button while a send is in progress. This prevents concurrent UI sends.
- For MCP tools: the backend handles nonce management. Concurrent MCP calls to the same source will fail at the nonce level with a clear error. This is acceptable for v1.

### 6.6 Testing Strategy

**Unit Tests:**
- `validate_and_send()` routing: verify all 14 viable combinations dispatch the correct `BackendTask` variant. Verify 2 impossible combinations return error strings.
- `get_transaction_type_description()`: verify correct label for all 14 combinations.
- Identity resolution helper (`resolve::qualified_identity()`): test valid ID, invalid ID, not-found ID.

**UI Integration Tests (egui_kittest):**
- Source selection: verify Identity radio appears only when identities exist.
- Progressive disclosure: verify Shielded source hidden when not in developer mode.
- Identity selector dropdown: verify it populates with loaded identities.

**MCP Tool Tests:**
- Each tool: test parameter validation (invalid wallet_id, invalid address format, missing required `network`).
- Happy path: requires backend mocking or test fixtures -- follow existing patterns in MCP tool tests.

**Backend E2E Tests (network-dependent):**
- The 8 new routing paths all use existing backend tasks that already have (or should have) backend E2E coverage.
- No new backend tasks are introduced, so no new backend E2E tests are strictly required.
- Consider adding one end-to-end scenario: Core->Identity->Platform round trip to verify the full cycle.

### 6.7 Security Considerations

- **OWASP A01 (Access Control)**: Identity operations require key authorization. The backend enforces this -- the UI cannot bypass it. No new attack surface.
- **OWASP A03 (Injection)**: MCP tool parameters are parsed through typed `Deserialize` structs with validation. Address strings are validated by `AddressInput`/`ValidatedAddress` or by SDK parsing functions. No shell interpolation.
- **Fund safety**: All destructive MCP tools require `require_network()` (mandatory network parameter) to prevent cross-network mistakes. All are annotated `destructive: true` so MCP clients prompt for confirmation.
- **Input validation**: Identity IDs are validated as Base58 Identifiers. Platform addresses are validated as bech32m. Core addresses are validated as Base58Check. Invalid inputs produce clear error messages without exposing internal state.
- **Concurrency**: `SendStatus::WaitingForResult` prevents double-submit in the UI. MCP tools are stateless adapters; the backend handles nonce-level serialization.
