# Unified Send Routing -- Requirements & UX Analysis

## 1. Executive Summary

**Problem**: Send routing logic (which backend task to invoke for a given source-destination pair) lives entirely in `WalletSendScreen` UI code. The MCP tool `core_funds_send` only handles core-to-core transfers. Any new consumer (CLI, automation, future MCP tools) must reimplement the 4x4 routing matrix.

**Solution**: Extract a pure `resolve_send()` function into `src/model/send_routing.rs` that accepts a validated source, validated destination, and amount, then returns the correct `BackendTask` (or a typed error for unsupported combinations). Both the UI and MCP/CLI share this single routing decision point.

**Key actors**: Everyday User (Alex Torres -- GUI), MCP client (AI agent or script), CLI operator (power user).

## 2. Stakeholder & Actor Analysis

| Actor | Interface | Goal | Key Constraint |
|---|---|---|---|
| Everyday User (Alex) | GUI send screen | Send Dash anywhere by pasting an address | Must not see jargon; needs clear next-step on failure |
| MCP Client | JSON-RPC tool | Programmatic sends across all address types | Structured errors; backward compat with `core_funds_send` |
| CLI Operator | `det_cli` | Scriptable sends from terminal | Prefers a single `wallet_funds_send` command over 4 separate tools |
| Future Integrator | Rust API | Embed send routing in other Rust services | Needs a clean `fn` with no UI or MCP dependencies |

## 3. Routing Matrix (Current State)

Source rows, destination columns. 14 supported, 2 unsupported.

| Source \ Dest | Core | Platform | Shielded | Identity |
|---|---|---|---|---|
| **CoreWallet** | core tx | asset lock | shield | asset lock + top-up |
| **PlatformAddresses** | withdrawal | platform transfer | withdrawal + shield | credit transfer |
| **Shielded** | unshield | unshield + asset lock | shielded tx | unsupported |
| **Identity** | withdrawal | credit transfer | unsupported | credit transfer |

The two unsupported routes (Identity-to-Shielded, Shielded-to-Identity) require a two-hop path. The routing function should return a typed error with a workaround suggestion.

## 4. User Stories

### US-1: Unified send (GUI)
**As** Alex (Everyday User), **I want to** paste any Dash address into the send field and have the app figure out how to deliver the funds, **so that** I do not need to understand Core vs. Platform vs. Shielded distinctions.

**Acceptance criteria**:
- Given a valid destination address of any supported kind, when I press Send, then the correct backend task is dispatched without me selecting a "transfer type."
- Given an unsupported source-destination pair, when I press Send, then I see a calm message explaining what to do instead (not a technical error).

### US-2: Unified send (MCP)
**As** an MCP client, **I want to** call a single `wallet_funds_send` tool with a destination address string, **so that** I do not need to know which of 14 backend tasks to invoke.

**Acceptance criteria**:
- Given `wallet_id`, `address`, `amount_duffs`, and `network`, when the tool is invoked, then routing resolves automatically based on address type and the source type implied by `source` param (defaulting to `"core"`).
- Given an unsupported combination, the tool returns a structured error (not a generic 500) with a user-readable message and error code.

### US-3: Backward compatibility
**As** an existing MCP consumer using `core_funds_send`, **I want** the old tool to keep working, **so that** my scripts do not break.

**Acceptance criteria**:
- `core_funds_send` continues to exist and work for core-to-core sends.
- The new `wallet_funds_send` tool is additive, not a replacement.

### US-4: Amount unit clarity
**As** any caller, **I want** the routing function to accept a single canonical unit and handle conversion internally, **so that** I do not need to know whether the destination expects duffs or credits.

**Acceptance criteria**:
- The routing function accepts amount in **duffs** (the L1 base unit, universally understood).
- When the resolved route requires credits (platform transfers, identity top-ups), the function converts using `CREDITS_PER_DUFF` (currently 1000).
- The MCP tool documents that `amount_duffs` is always in duffs regardless of destination type.

## 5. Function Signature (Conceptual)

```
resolve_send(
    source: SendSource,
    destination: ValidatedAddress,
    amount_duffs: u64,
    context: &SendContext,   // wallet refs, network, fee config
) -> Result<BackendTask, SendRoutingError>
```

Where `SendSource` is a model-layer enum (no UI types):

```
enum SendSource {
    CoreWallet { wallet: Arc<RwLock<Wallet>> },
    PlatformAddresses { seed_hash, addresses: Vec<(PlatformAddress, Address, u64)> },
    Identity { qualified_identity: QualifiedIdentity },
    Shielded { seed_hash, balance_credits: u64 },
}
```

And `SendRoutingError` is a typed enum (not strings) that maps cleanly to both `MessageBanner` (GUI) and `McpToolError` (MCP).

## 6. Error UX

### 6a. Unsupported route errors (user-facing text)

**Identity to Shielded**:
> "Direct transfers from an identity to a private address are not available yet. You can withdraw to a Platform address first, then transfer to a private address from there."

**Shielded to Identity**:
> "Direct transfers from a private address to an identity are not available yet. You can transfer to a Platform address first, then top up the identity from there."

These follow the project conventions: calm tone, what happened + what to do, no jargon ("shielded pool" becomes "private address," "credit balance" becomes "identity").

### 6b. Validation errors

| Condition | Message |
|---|---|
| Amount is zero | "Enter an amount greater than zero." |
| Insufficient balance | "Not enough funds. Your available balance is {amount} DASH." |
| Address not recognized | "This does not look like a valid Dash address. Check for typos and try again." |
| Network mismatch (address vs. app) | "This address belongs to a different network. You are connected to {network}." |

### 6c. MCP error mapping

| SendRoutingError variant | McpToolError | JSON-RPC code |
|---|---|---|
| UnsupportedRoute | InvalidParam | -32602 |
| InsufficientBalance | TaskFailed | -32004 |
| InvalidAmount | InvalidParam | -32602 |
| AddressValidation | InvalidParam | -32602 |

## 7. MCP Tool Design

### New tool: `wallet_funds_send`

```json
{
  "name": "wallet_funds_send",
  "description": "Send DASH from a wallet to any supported address type. Routing is automatic based on address format.",
  "params": {
    "wallet_id": "string -- wallet alias or hex seed hash",
    "address": "string -- destination (Core, Platform, Shielded, or Identity)",
    "amount_duffs": "u64 -- amount in duffs (1 DASH = 100,000,000 duffs)",
    "network": "string -- required: mainnet, testnet, devnet, local",
    "source": "string -- optional: core (default), platform, shielded, identity"
  }
}
```

**Design decisions**:
- `source` defaults to `"core"` for backward compatibility and because it is the most common case (Alex pastes an address, funds come from the Core wallet).
- `core_funds_send` is **retained** as-is. It becomes a thin wrapper or simply stays independent. No breaking changes.
- Tool name follows existing convention: `{domain}_{object}_{action}` -> `wallet_funds_send`.
- The tool reuses `resolve_send()` from the model layer -- no routing logic in the MCP tool itself.

### Output structure

```json
{
  "route": "core_to_platform",
  "txid": "abc123...",
  "amount_duffs": 50000000,
  "destination": "tdash1q..."
}
```

The `route` field tells the caller what actually happened, useful for logging and verification.

## 8. Amount Semantics Decision

**Decision: Accept duffs, convert internally.**

Rationale:
- Duffs are the universal base unit across all Dash interfaces. Alex thinks in DASH (the UI converts); MCP clients think in duffs.
- Credits are a Platform-internal unit. The conversion rate (`CREDITS_PER_DUFF = 1000`) is protocol-defined and already centralized in `model/amount.rs`.
- Requiring callers to pre-convert to credits for platform destinations leaks implementation details and creates a class of bugs (wrong unit passed).
- The routing function performs the conversion when constructing backend tasks that require credits.

Edge case: If the amount in duffs does not convert to a whole number of credits (unlikely since `CREDITS_PER_DUFF` is 1000, and duffs are integers, so every duff maps to exactly 1000 credits), the function should round and document the behavior.

## 9. Prioritization (MoSCoW)

| Priority | Item | Rationale |
|---|---|---|
| **Must** | `resolve_send()` in `src/model/send_routing.rs` | Core deliverable; unblocks MCP and CLI |
| **Must** | `SendRoutingError` typed enum with user-friendly Display | Project error conventions require it |
| **Must** | `wallet_funds_send` MCP tool using `resolve_send()` | Primary consumer beyond GUI |
| **Must** | Retain `core_funds_send` unchanged | Backward compatibility |
| **Should** | Refactor `WalletSendScreen` to use `resolve_send()` | Reduces duplication; validates the API |
| **Should** | Unit tests for all 16 matrix cells | 14 success + 2 unsupported |
| **Could** | CLI `wallet funds send` subcommand | Falls out naturally from MCP tool |
| **Won't (this phase)** | Multi-hop routing for unsupported pairs | Complexity too high; workaround guidance is sufficient |

## 10. Open Questions & Assumptions

1. **Fee handling**: Does `resolve_send()` estimate fees, or does the caller handle that separately? Current UI methods compute fees inline. Recommendation: routing function returns the task; fee estimation stays in `model/fee_estimation.rs` and is called by the UI/MCP layer before confirming. Routing should not block on fee estimation.

2. **Identity resolution**: For `AddressKind::Identity`, the current UI looks up the identity on-chain before constructing the task. Should `resolve_send()` do this, or require pre-resolved identity? Recommendation: require pre-resolved `ValidatedAddress::Identity { id, .. }` -- the routing function should be synchronous and pure. Resolution is an async step the caller performs first.

3. **Source auto-detection**: Could the function auto-select the best source given only a wallet and destination? Tempting but dangerous -- Alex might not realize funds are coming from Platform vs. Core. Explicit source selection is safer. The MCP default of `"core"` handles the 80% case.

4. **Advanced mode (multi-input/multi-output)**: The send screen has an advanced mode with multiple inputs and outputs. This is out of scope for `resolve_send()`, which handles single-source, single-destination routing. Advanced mode remains UI-only for now.
