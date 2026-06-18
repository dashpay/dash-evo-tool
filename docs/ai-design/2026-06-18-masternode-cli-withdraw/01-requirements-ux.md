# Masternode / Evonode Withdrawals via det-cli — Requirements & UX Spec

**Date:** 2026-06-18
**Phase:** 1a (Requirements) + 1b (UX/DX)
**Surface:** det-cli over MCP (headless). Not the GUI.
**Status:** Draft for review.
**Base:** PR #860 @976ad0d4, branch `feat/masternode-cli-withdraw`.

---

## 1. Executive Summary

**Problem.** A masternode/evonode operator who works headlessly (det-cli / MCP, no
GUI) cannot withdraw their node's accumulated Platform credits today. Two gaps
stand in the way, both already solved in the GUI but absent from the tool layer:

1. **No headless identity load.** Loading a masternode/evonode identity — fetching
   it by ProTxHash and binding its owner/voting/payout private keys — is GUI-only
   (`add_existing_identity_screen.rs` → `IdentityTask::LoadIdentity`). The existing
   `identity_credits_withdraw` MCP tool resolves the identity from the **local
   database** (`resolve::qualified_identity`) and fails with *"Load the identity
   first using the identity screen or CLI"* — but no CLI load path exists. The
   instruction points at a door that isn't built.

2. **The withdraw tool is not masternode-aware.** `identity_credits_withdraw`
   always requires a destination address and always passes `key_id = None`. A
   masternode withdrawal signed with the **OWNER** key must force the destination
   to the node's registered payout address; the tool has no concept of key mode.

**Solution direction.** Two thin MCP tools, mirroring patterns already proven in
the codebase:

- **(A) `identity_masternode_load`** — load a masternode/evonode identity headlessly
  from a ProTxHash plus owner/voting/payout private keys. Mirrors the secret-input
  mechanism of `core_wallet_import` (`src/mcp/tools/wallet.rs`) and dispatches the
  existing `IdentityTask::LoadIdentity`.

- **(B) `identity_masternode_credits_withdraw`** — a masternode-aware credit
  withdrawal supporting **both** key modes, mirroring the GUI's
  `withdraw_screen.rs` rules and dispatching the existing
  `IdentityTask::WithdrawFromIdentity`.

Both are adapters per `docs/MCP_TOOL_DEVELOPMENT.md`: no new business logic, no new
backend tasks. The backend already does everything; we are only opening a headless
door to it.

**Key actors.** Priya (Power User / masternode operator) and Jordan (Platform
Developer) — both on the headless path. The Everyday User (Alex) stays in the GUI
and is explicitly out of scope.

**Two product decisions already LOCKED (not open for re-litigation here):**

- Deliver **both** tools (A load + B withdraw).
- Withdraw supports **both** modes, mirroring the GUI: OWNER key → destination
  forced to the registered payout address; payout/TRANSFER key → any Core address.

---

## 2. Stakeholder & Actor Analysis

### Primary actor — Priya Nakamura (Power User / masternode operator)

| Field | Value |
|---|---|
| Goal here | Withdraw her node's earned Platform credits to Core, scripted, without opening the desktop GUI. |
| Context | Runs a masternode; comfortable with CLI, holds owner/voting/payout keys; understands the payout-address constraint. |
| Pain today | The only withdrawal path is the GUI. Headless automation (cron, ops scripts) is impossible. |
| Success metric | One scripted load + one scripted withdraw, no GUI, clear JSON result with the txid-equivalent confirmation and fees. |

### Primary actor — Jordan Kim (Platform Developer)

| Field | Value |
|---|---|
| Goal here | Spin a test evonode identity into the tool from known testnet keys and exercise the withdraw path during dApp/integration testing. |
| Context | Testnet/Devnet; values speed and directness; reads raw protocol numbers (credits, fees). |
| Pain today | Must hand-drive the GUI to load an evonode identity before any headless work. Breaks automation. |
| Success metric | Load by ProTxHash + keys, then withdraw, entirely from a script; actionable errors with numbers, not raw Rust strings. |

### Secondary actor — AI agent over MCP

Calls the same two tools via the MCP HTTP/stdio transport. Needs accurate tool
**annotations** (`destructive`, `idempotent`, `open_world`) to decide confirmation
prompts, and a clean JSON schema. The masternode-load tool handles private keys, so
its annotations and its `Debug` redaction matter for agent safety.

### Supporting systems

- **Backend task system** — `IdentityTask::LoadIdentity` and
  `IdentityTask::WithdrawFromIdentity` (authoritative enforcement layer).
- **Local DB** — `insert_local_qualified_identity` persists the loaded identity so
  the withdraw tool's `resolve::qualified_identity` can find it (this is the seam
  that makes A→B compose).
- **SPV / DAPI** — load fetches the identity over DAPI; proof verification needs a
  synced SPV chain.
- **`Secret` model type** (`src/model/secret.rs`) — zeroizing, page-locked secret
  carrier already used by `IdentityInputToLoad`.

---

## 3. Functional Requirements

### 3.1 Tool A — `identity_masternode_load`

**Purpose.** Load a masternode or evonode identity headlessly: fetch it by
ProTxHash over DAPI, verify and bind the supplied private keys, and persist it
locally so subsequent tools (withdraw, refresh, etc.) can resolve it.

**Mirrors.** Secret input → `core_wallet_import` (`ImportWalletParams`,
hand-written redacting `Debug`, zeroizing buffers). Dispatch → the existing
`IdentityTask::LoadIdentity(IdentityInputToLoad)`.

#### Inputs

| Param | Type | Required | Notes |
|---|---|---|---|
| `pro_tx_hash` | `String` | yes | The identity handle. ProTxHash is the masternode/evonode identity ID. Accept hex (the canonical MN encoding) and Base58 — `load_identity` already tries Base58 then Hex (`Identifier::from_string`). |
| `node_type` | `String` enum | yes | `"masternode"` or `"evonode"`. Maps to `IdentityType::Masternode` / `IdentityType::Evonode`. **Never** `User`. |
| `owner_private_key` | `String` (WIF or 64-hex) | conditional | At least one key must be present (see rule FR-A4). Bound as the OWNER key. |
| `voting_private_key` | `String` (WIF or 64-hex) | optional | Bound as the voting key on the associated voter identity (load fetches the voter identity via DAPI when supplied). |
| `payout_private_key` | `String` (WIF or 64-hex) | conditional | At least one key must be present (see FR-A4). Bound as the payout/TRANSFER key. |
| `alias` | `String` | optional | Human-readable name; trimmed, empty → none. Falls back to first DPNS name if any (existing load behavior). |
| `network` | `String` | yes | Required. Destructive-adjacent (writes local DB, fetches network). Use `resolve::require_network`. |

Key formats follow `verify_key_input` exactly: **64-char hex** or **51/52-char
WIF**; empty string → "not supplied" (`None`); any other length is an error. We do
not re-implement this — the backend `verify_key_input` is the single source of
truth, and it returns typed `TaskError::KeyInputValidationFailed { key_name, detail }`.

#### Outputs

```json
{
  "identity_id": "<base58>",
  "node_type": "masternode" | "evonode",
  "alias": "<string|null>",
  "owner_key_loaded": true,
  "voting_key_loaded": false,
  "payout_key_loaded": true,
  "available_withdrawal_keys": ["owner", "transfer"],
  "payout_address": "<Core address|null>",
  "dpns_names": ["alice.dash"]
}
```

- `available_withdrawal_keys` is derived from `available_withdrawal_keys()` on the
  resulting `QualifiedIdentity` (OWNER + TRANSFER for MN/Evonode). It tells the
  caller — human or agent — which `key_mode` values the withdraw tool will accept
  for this identity, so the second step is self-describing.
- `payout_address` comes from `masternode_payout_address(network)`. It is the
  destination the caller gets when withdrawing with the OWNER key. Surfacing it
  here means the operator can verify the payout target *before* moving funds.

#### Behavioral rules

- **FR-A1** — `node_type` must be `masternode` or `evonode`. Reject `user` (and any
  other value) with `InvalidParam` — this tool is masternode-specific by design;
  user identities load via a different (future or existing) path.
- **FR-A2** — Construct `IdentityInputToLoad` with `derive_keys_from_wallets =
  false` and `selected_wallet_seed_hash = None`. Headless MN load is key-driven, not
  wallet-derived. (`keys_input` stays empty — that vec is the User-type manual-key
  path.)
- **FR-A3** — On `LoadedIdentity(qi)`, the backend has already called
  `insert_local_qualified_identity`. The identity is now resolvable by
  `resolve::qualified_identity` for the withdraw tool. No extra persistence in the
  tool.
- **FR-A4** — Require **at least one** of `owner_private_key` / `payout_private_key`.
  Loading an MN identity with zero signing keys produces a watch-only record that
  can sign nothing — useless for the locked use case (withdraw). Reject with a
  message that names the two keys and explains they enable the two withdraw modes.
  (`voting_private_key` alone does not enable a withdrawal — it only binds the voter
  identity.)
- **FR-A5** — Re-loading the same identity is effectively idempotent: the backend
  does INSERT-OR-REPLACE keyed by identity ID. Annotate `idempotent(false)`
  conservatively (re-load re-fetches network state and can change bound keys), but
  document that re-running is safe and updates the local record.

### 3.2 Tool B — `identity_masternode_credits_withdraw`

**Purpose.** Withdraw credits from a loaded masternode/evonode identity to Core,
honoring the two key/destination modes.

**Mirrors.** Dispatch → `IdentityTask::WithdrawFromIdentity(qi, Option<Address>,
Credits, Option<KeyID>)`. Destination/key rules → `withdraw_screen.rs`
(`render_address_input`, `show_confirmation_popup`).

#### Inputs

| Param | Type | Required | Notes |
|---|---|---|---|
| `identity_id` | `String` | yes | Base58 identity ID (the loaded MN identity). Resolved via `resolve::qualified_identity`. |
| `key_mode` | `String` enum | yes | `"owner"` or `"transfer"`. Selects which available withdrawal key signs. Explicit, not inferred — see FR-B1. |
| `to_address` | `String` | conditional | Core address. **Required** for `transfer` mode, **forbidden/ignored** for `owner` mode (destination is forced). See FR-B2/B3. |
| `amount_credits` | `u64` | yes | > 0 (`resolve::validate_credits`). |
| `network` | `String` | yes | Required (destructive). `resolve::require_network`. |

#### Outputs

```json
{
  "identity_id": "<base58>",
  "key_mode": "owner" | "transfer",
  "to_address": "<Core address actually used>",
  "amount_credits": 100000,
  "estimated_fee": 1234,
  "actual_fee": 1230
}
```

`to_address` echoes the address **actually used** — for OWNER mode that is the
resolved payout address, not a caller input. The caller always learns where the
funds went.

#### Behavioral rules — the two modes (LOCKED)

- **FR-B1 — Explicit key mode.** The caller chooses `key_mode` explicitly rather
  than the tool guessing from key availability. An MN/evonode identity may have both
  OWNER and TRANSFER keys loaded; the destination semantics differ sharply between
  them (forced vs free), so an ambiguous auto-pick is a foot-gun for a fund-moving
  operation. The tool resolves `key_mode` to a concrete `KeyID` by scanning
  `available_withdrawal_keys()` for a key whose purpose matches
  (`OWNER` ↔ `transfer`→`TRANSFER`). If no matching key is loaded, return
  `InvalidParam` naming which key is missing and that it must be supplied at load
  time.

- **FR-B2 — OWNER mode forces destination.** When `key_mode = "owner"`:
  - Resolve the destination from `masternode_payout_address(network)`.
  - If `to_address` was supplied, **reject** with `InvalidParam`: the OWNER-key
    withdrawal can only go to the registered payout address, so a caller-supplied
    address is a contradiction to surface, not silently ignore. (The GUI hides the
    address field entirely in this mode; headless can't hide a field, so it rejects.)
  - If the identity has no payout address, reject with a clear message — there is
    nowhere for an OWNER-key withdrawal to go.
  - Dispatch with `address = Some(payout_address)`, `key_id = Some(owner_key_id)`.

- **FR-B3 — TRANSFER mode, free destination.** When `key_mode = "transfer"`:
  - `to_address` is **required**; validate format (`resolve::validate_address`) and
    network match, exactly as `identity_credits_withdraw` does (parse
    `NetworkUnchecked` → `require_network(ctx.network())`).
  - Reject Platform (bech32m) addresses — withdrawals settle on Core only. Mirror
    the GUI's `is_platform_address_string` guard with a calm message.
  - Dispatch with `address = Some(parsed_core_address)`,
    `key_id = Some(transfer_key_id)`.

- **FR-B4 — No developer-mode relaxation in the tool.** The GUI relaxes the
  payout-address constraint under developer mode (lets an OWNER-key withdrawal go to
  an arbitrary address). The headless tool does **not** expose that relaxation:
  there is no UI to signal "I know what I'm doing," and a scripted/agent caller
  silently sending an OWNER withdrawal to an arbitrary address is exactly the
  mistake this constraint exists to prevent. OWNER mode always forces the payout
  address. (Open question OQ-3 confirms.)

- **FR-B5 — Identity must be loaded first.** If `resolve::qualified_identity` fails,
  return its existing message — now accurate, because the door (`identity_masternode_load`)
  exists. Consider updating the message to name the new tool.

### 3.3 Network safety (both tools)

- Both are stateful/destructive: `network` is **required**, enforced via
  `resolve::require_network` (not the optional `verify_network`). A mismatch returns
  `NetworkMismatch { expected, actual }`. This is the locked cross-network guard and
  matches every other fund-moving tool.

### 3.4 Composition (A → B), validated

```
identity_masternode_load (pro_tx_hash + keys + network)
        │  fetch by ProTxHash over DAPI, verify keys, bind, persist
        ▼
   LoadedIdentity(qi)  → insert_local_qualified_identity (INSERT-OR-REPLACE)
        │
        ▼
identity_masternode_credits_withdraw (identity_id + key_mode + ... )
        │  resolve::qualified_identity finds the persisted record
        ▼
   WithdrawFromIdentity(qi, dest, credits, key_id) → CreditWithdrawal ST
```

The two tools compose through the **local DB**, exactly as the existing
GUI→withdraw flow does. No shared in-memory state, no ordering coupling beyond
"load before withdraw," which the withdraw tool's error already enforces.

---

## 4. Non-Functional Requirements

### 4.1 Security of private-key input

- **NFR-S1 — Redacting `Debug`.** The load tool's params struct carries three
  private keys. Mirror `ImportWalletParams`: a **hand-written `Debug`** that prints
  each key field as `"<redacted>"`. A derived `Debug` would leak keys into MCP error
  `data` payloads and logs (`McpToolError::TaskFailed` serializes `{task_err:?}`).
  This is the single most important security requirement in this spec.
- **NFR-S2 — Zeroizing buffers.** Wrap raw key strings in `zeroize::Zeroizing` (or
  feed them into `Secret::new`, which page-locks and zeroizes) before handing to
  `IdentityInputToLoad`. `IdentityInputToLoad` already takes `Secret`, so the tool's
  job is to move the input into `Secret` promptly and not retain a plain `String`
  copy.
- **NFR-S3 — Keys never in output or errors.** Output reports only *which* keys
  loaded (booleans), never the key material. Validation errors name the key by role
  ("Owner", "Payout Address") and the failure kind, never echo the value. This is
  already how `TaskError::KeyInputValidationFailed` behaves — preserve it.
- **NFR-S4 — Transport caution.** Over MCP HTTP, keys traverse the request body. The
  HTTP transport is bearer-auth'd and loopback by default; document that callers
  must not send live mainnet MN keys over a non-loopback MCP HTTP endpoint. (Carried
  as a documentation note for `docs/MCP.md`, not enforced in code.)

### 4.2 Network-match enforcement

- **NFR-N1** — `require_network` on both tools (FR-3.3). Keys are network-scoped; a
  WIF parsed against the wrong network fails in `verify_key_input` anyway, but the
  explicit network guard fails *first*, with the clearer cross-network message.

### 4.3 SPV sync prerequisites

- **NFR-P1 — Load needs SPV.** `load_identity` calls `Identity::fetch_by_identifier`
  (and, with a voting key, fetches the voter identity) over DAPI. Per the SPV-gate
  rule in `MCP_TOOL_DEVELOPMENT.md`, DAPI proof verification needs a synced SPV
  chain. The load tool **must** call `resolve::ensure_spv_synced` before dispatch.
- **NFR-P2 — Withdraw and the SPV inconsistency (FLAG).** The existing
  `identity_credits_withdraw` *intentionally skips* `ensure_spv_synced` (comment:
  "no SPV sync needed — this tool only dispatches Platform state transitions").
  This contradicts the documented SPV-gate rule, which says platform-only network
  calls still need SPV for proof verification. We have two options:
  - **(a)** Match the sibling withdraw tool and skip the gate (consistency with the
    one tool a caller will compare against).
  - **(b)** Add the gate (consistency with the documented rule and with load).
  Recommended: **(b) add the gate** — a withdrawal is the most consequential op in
  this spec, and a few seconds of sync-wait is cheap insurance against a proof
  failure mid-withdraw. Carried as **OQ-4** for the implementer to confirm and, if
  (b), to reconcile the existing tool's comment. Either way, **document the choice**.

### 4.4 DX / discoverability (per `MCP_TOOL_DEVELOPMENT.md`)

- **NFR-D1** — Tool names follow `{domain}_{object}_{action}`:
  `identity_masternode_load`, `identity_masternode_credits_withdraw`. CLI auto-hyphenates
  (`identity-masternode-load`).
- **NFR-D2** — Annotations:
  - `identity_masternode_load`: `read_only(false)`, `destructive(false)`,
    `idempotent(false)`, `open_world(true)`.
  - `identity_masternode_credits_withdraw`: `read_only(false)`, `destructive(true)`,
    `idempotent(false)`, `open_world(true)` — identical to `identity_credits_withdraw`.
- **NFR-D3** — Descriptions state the two modes (load: by ProTxHash + keys;
  withdraw: owner→payout-forced vs transfer→any-address) and that `network` is
  required. Keep them concise — agents read these to choose tools.
- **NFR-D4 — Tools, not CLI code.** Both live in `src/mcp/tools/identity.rs`;
  register one line each in `tool_router()`. Zero changes in `src/bin/det_cli/`.

---

## 5. CLI / DX Ergonomics

### Secret passing — mirror `core_wallet_import`

`core_wallet_import` takes the mnemonic as a normal string parameter
(`mnemonic: String`) and relies on (1) redacting `Debug` and (2) zeroizing buffers
internally. We follow the **same mechanism** for MN keys — keys are string params,
protected by redacting `Debug` + zeroizing, not by any special channel. This is the
locked, established pattern; introducing a new secret-input channel here would
diverge from the one tool operators already know.

```bash
# 1. Load an evonode identity headlessly (testnet example)
det-cli identity-masternode-load \
  pro_tx_hash=<64-hex protx> \
  node_type=evonode \
  owner_private_key=<WIF> \
  payout_private_key=<WIF> \
  network=testnet
# → { "identity_id": "...", "available_withdrawal_keys": ["owner","transfer"],
#     "payout_address": "y...", ... }

# 2a. Withdraw with the OWNER key — destination is forced to the payout address
det-cli identity-masternode-credits-withdraw \
  identity_id=<base58> \
  key_mode=owner \
  amount_credits=100000 \
  network=testnet
# (no to_address; supplying one is rejected)

# 2b. Withdraw with the payout/TRANSFER key — any Core address
det-cli identity-masternode-credits-withdraw \
  identity_id=<base58> \
  key_mode=transfer \
  to_address=y... \
  amount_credits=100000 \
  network=testnet
```

### Walkthrough as each persona

- **Priya (operator).** Reads `available_withdrawal_keys` and `payout_address` from
  the load output, confirms the payout target matches her records, then withdraws
  with `key_mode=owner` and no address. She never has to know the payout-address
  rule in advance — the tool enforces it and the load output shows the target. ✔
- **Jordan (developer).** Loads a testnet evonode from known faucet keys, sees the
  two modes enumerated, scripts `key_mode=transfer` to a throwaway address for
  iteration. Errors carry numbers (estimated/actual fee) and name the missing key
  if he forgets to load one. ✔
- **AI agent.** Annotations mark withdraw `destructive`; the agent prompts for
  confirmation. The load tool's redacting `Debug` keeps keys out of any error it
  surfaces back to the model. The `key_mode` enum is explicit, so the agent cannot
  silently pick the wrong mode. ✔

---

## 6. Error UX (typed `McpToolError`, user-friendly per project rules)

All errors flow through `McpToolError`; messages follow the project's error
rules — *what happened + what to do*, no jargon, no raw SDK strings (those go to the
`Debug` chain in `data`). Base58/hex IDs and Core addresses are allowed (they are
copyable handles, not jargon).

| Condition | Variant | Message (Display) |
|---|---|---|
| `node_type` not masternode/evonode | `InvalidParam` | "The 'node_type' must be \"masternode\" or \"evonode\"." |
| No signing key supplied (FR-A4) | `InvalidParam` | "Provide at least one of the owner or payout private key. The owner key withdraws to the registered payout address; the payout key withdraws to any address." |
| Key wrong length / not hex / bad WIF | `TaskFailed(KeyInputValidationFailed)` | (backend, role-named) e.g. "The Owner key is the length of a WIF key but is invalid." |
| Key not present on the identity | `TaskFailed(KeyInputValidationFailed)` | (backend) names the role and that it does not match the on-chain key. |
| ProTxHash unparseable | `TaskFailed(IdentifierParsingError)` | (backend) "could not read the identity ID." |
| Identity not found on network | `TaskFailed(IdentityNotFound)` | (backend) "That identity was not found on this network. Check the ProTxHash and the network." |
| Network missing/mismatch | `NetworkMismatch` / `InvalidParam` | "The 'network' parameter must match the active network: expected {expected}, active {actual}." |
| SPV not synced | `SpvSyncFailed` | "Still syncing with the network — wait a moment and try again." |
| Withdraw before load (FR-B5) | `InvalidParam` | "This identity is not loaded yet. Run identity-masternode-load with the ProTxHash and keys first." |
| `key_mode` unknown | `InvalidParam` | "The 'key_mode' must be \"owner\" or \"transfer\"." |
| `key_mode` key not loaded | `InvalidParam` | "The {owner|payout} key needed for this withdrawal is not loaded. Re-run identity-masternode-load and include it." |
| OWNER mode + `to_address` supplied (FR-B2) | `InvalidParam` | "An owner-key withdrawal always goes to the registered payout address. Remove 'to_address', or use key_mode=transfer to choose an address." |
| OWNER mode + no payout address | `InvalidParam` | "This identity has no registered payout address, so an owner-key withdrawal has no destination. Use key_mode=transfer with a Core address." |
| TRANSFER mode + missing/invalid/Platform address | `InvalidParam` | (mirror existing) "Enter a valid Core address — Platform addresses cannot receive withdrawals." |
| `amount_credits == 0` | `InvalidParam` | "amount_credits must be greater than zero." |

> All Display strings are i18n-ready single sentences with named placeholders.
> Technical chains attach via the `TaskFailed` `data` payload, never the message.

---

## 7. Acceptance Criteria

### Tool A — `identity_masternode_load`

- **AC-A1** Given a valid testnet evonode ProTxHash and a valid payout WIF, when I
  call the tool with `node_type=evonode network=testnet`, then it returns
  `identity_id`, `payout_key_loaded=true`, `available_withdrawal_keys` containing
  `"transfer"`, and the `payout_address`.
- **AC-A2** Given a valid owner key, when loaded, then `available_withdrawal_keys`
  contains `"owner"`.
- **AC-A3** Given `node_type=user`, when called, then `InvalidParam` (this tool is
  masternode-only).
- **AC-A4** Given neither owner nor payout key, when called, then `InvalidParam`
  naming both keys (FR-A4).
- **AC-A5** Given a key of wrong length, when called, then a role-named
  `KeyInputValidationFailed` error; the key value never appears in the message or
  the `data` payload.
- **AC-A6** Given the active network is mainnet and `network=testnet`, then
  `NetworkMismatch` before any network call.
- **AC-A7** After a successful load, when I call
  `identity_masternode_credits_withdraw` for the same `identity_id`, then the
  identity resolves (no "not loaded" error) — proving A→B composition.
- **AC-A8** The params struct's `Debug` output renders every private key as
  `<redacted>` (unit-testable without network).

### Tool B — `identity_masternode_credits_withdraw`

- **AC-B1 (OWNER, happy path)** Given a loaded MN identity with an owner key and a
  registered payout address, when I withdraw `key_mode=owner amount_credits=N` with
  **no** `to_address`, then the withdrawal dispatches to the payout address and the
  output's `to_address` equals that payout address.
- **AC-B2 (OWNER + address rejected)** Same as AC-B1 but with a `to_address`
  supplied → `InvalidParam` (FR-B2).
- **AC-B3 (OWNER, no payout address)** Loaded MN identity lacking a payout address,
  `key_mode=owner` → `InvalidParam` (FR-B2).
- **AC-B4 (TRANSFER, happy path)** Loaded identity with a transfer key,
  `key_mode=transfer to_address=<valid core> amount_credits=N` → dispatch to that
  address; output echoes it.
- **AC-B5 (TRANSFER, missing address)** `key_mode=transfer` with no `to_address`
  → `InvalidParam`.
- **AC-B6 (TRANSFER, Platform address)** `key_mode=transfer` with a bech32m Platform
  address → `InvalidParam` (Core-only).
- **AC-B7 (mode key not loaded)** `key_mode=owner` on an identity loaded with only a
  payout key → `InvalidParam` naming the missing owner key.
- **AC-B8 (not loaded)** Withdraw for an `identity_id` never loaded → `InvalidParam`
  pointing at `identity-masternode-load`.
- **AC-B9 (network)** Network mismatch → `NetworkMismatch`; `amount_credits=0`
  → `InvalidParam`.
- **AC-B10 (output numbers)** On success, output includes `estimated_fee` and
  `actual_fee` from the backend fee result.

### Cross-cutting

- **AC-X1** Both tools appear in `tools/list` (CLI discovery) and
  `tool-describe` returns clean JSON schemas.
- **AC-X2** No tool logic lives in `src/bin/det_cli/`; both tools live in
  `src/mcp/tools/identity.rs` and register one line each in `tool_router()`.
- **AC-X3** Smoke: `det-cli tools` lists both; `det-cli tool-describe
  name=identity_masternode_load` returns its schema (no network needed).

---

## 8. User Stories (for `docs/user-stories.md`)

The catalog already covers GUI MN load (IDN-003) and GUI credit withdrawal
(IDN-005, SND-012), and CLI wallet management (MCP-001). The headless **masternode**
load+withdraw is new. Propose adding to the **Programmatic Access (MCP)** section:

```markdown
### MCP-003: Load a masternode/evonode identity via CLI [Gap]
**Persona:** Priya, Jordan

As a masternode operator, I want to load my masternode or evonode identity
headlessly via det-cli — by ProTxHash plus owner/voting/payout private keys — so
that I can manage it in scripts and automation without opening the GUI.

- Identity is fetched by ProTxHash over the network and persisted locally.
- Private keys are accepted as WIF or hex, never echoed back, and redacted in logs.
- Output reports which keys loaded, the available withdrawal modes, and the
  registered payout address.
- The 'network' parameter is required and must match the active network.

### MCP-004: Withdraw masternode/evonode credits via CLI [Gap]
**Persona:** Priya, Jordan

As a masternode operator, I want to withdraw my node's Platform credits to Core
headlessly via det-cli, in both key modes, so that I can automate payouts.

- With the owner key, the destination is forced to the registered payout address;
  supplying a different address is rejected.
- With the payout/transfer key, I can withdraw to any Core address.
- The withdrawal is queued on Platform and settles after confirmation; the result
  reports the destination used and the estimated and actual fees.
- The 'network' parameter is required and must match the active network.
```

> Tagged `[Gap]` now; flip to `[Implemented]` when Phase 2 lands. Per the locked
> scope, both stories ship together.

---

## 9. Prioritized Backlog (MoSCoW)

| Item | Priority | Rationale |
|---|---|---|
| Tool A: load by ProTxHash + owner/payout keys, redacting `Debug`, persist | **Must** | Without it, withdraw is unreachable headlessly — the locked entry point. |
| Tool B: withdraw with explicit `key_mode`, OWNER→payout-forced, TRANSFER→free | **Must** | The locked core deliverable; both modes mirror the GUI. |
| `require_network` on both; SPV gate on load | **Must** | Cross-network and proof-verification safety for fund-moving ops. |
| `available_withdrawal_keys` + `payout_address` in load output | **Should** | Makes step 2 self-describing; strong DX for both personas and agents. |
| Voting-key binding in load | **Should** | Surfaced in the GUI three-key set; cheap to pass through; needed for full MN management parity, not strictly for withdraw. |
| SPV gate on withdraw (reconcile sibling tool) — OQ-4 | **Should** | Safety vs consistency with existing tool; needs a decision. |
| Developer-mode address relaxation for OWNER withdrawals | **Won't (this phase)** | No headless signal for "I know what I'm doing"; the constraint exists precisely to prevent scripted misfires (FR-B4). |
| Auto-pick `key_mode` from loaded keys | **Won't** | Ambiguous for a fund-moving op; explicit mode is safer (FR-B1). |
| MN reward "claim" as a distinct op | **Won't (N/A)** | There is no separate MN-reward withdrawal path — it is the same `WithdrawFromIdentity`/`CreditWithdrawal` ST. Confirmed in prior investigation. |

---

## 10. Open Questions & Assumptions

### Open product questions

- **OQ-1 — Should `identity_masternode_load` accept ProTxHash in hex only, or also
  Base58?** The backend `load_identity` tries Base58 then Hex. ProTxHash is
  canonically hex (it is a transaction hash). Recommend: **accept both**, document
  hex as the expected form. Low risk; the backend already handles it.
- **OQ-2 — Tool naming: keep `masternode` in the names, or fold into the existing
  `identity_credits_withdraw`?** Recommend **separate, masternode-named tools** so
  the two modes and the payout-forcing are explicit and discoverable, and so the
  existing user-identity withdraw tool stays simple. The alternative — overloading
  `identity_credits_withdraw` with a `key_mode` — risks the foot-gun FR-B1 guards
  against. **Needs confirmation.**
- **OQ-3 — Confirm OWNER-mode never allows a custom address headlessly (FR-B4).**
  The GUI relaxes this in developer mode. We propose **no relaxation** in the tool.
  If operators need OWNER→arbitrary-address headlessly, that is a separate,
  explicitly-flagged feature. **Needs confirmation.**
- **OQ-4 — SPV gate on the withdraw tool (NFR-P2).** Add it (safety, matches the
  documented rule and load) or skip it (match the existing `identity_credits_withdraw`)?
  Recommend **add**, and reconcile the sibling tool's comment. **Needs decision.**
- **OQ-5 — `voting_private_key` in scope for v1?** It is part of the GUI three-key
  set and binds the voter identity (extra DAPI fetch). It is **not** required for
  withdrawal. Recommend: **accept it optionally** for management parity, but it is
  cuttable from a minimal first cut if it complicates the load. **Confirm priority.**

### Assumptions

- **A-1** The locked scope (both tools, both modes) is final; this spec does not
  re-open it.
- **A-2** `IdentityType::Masternode` vs `Evonode` does not change the withdraw
  destination rules — both expose OWNER+TRANSFER via `available_withdrawal_keys()`
  and both have a `masternode_payout_address`. The `node_type` param only sets the
  load type; withdraw behavior is identical across the two.
- **A-3** No new `BackendTask` or `TaskError` variant is needed — both tools are
  pure adapters over existing tasks. (If FR-B5's message is reworded to name the new
  tool, that is a one-line literal change, not a new variant.)
- **A-4** The headless caller is trusted to hold the MN private keys; this spec does
  not add key-custody features beyond the existing zeroizing/redaction guarantees.

---

## Candy Tally (findings surfaced)

| Severity | Count | Items |
|---|---|---|
| High | 1 | NFR-P2 / OQ-4 — withdraw SPV-gate inconsistency vs the documented rule and the sibling tool. |
| Medium | 3 | FR-A4 (key-less MN load is useless), FR-B1 (auto-pick key_mode foot-gun), FR-B2 (OWNER-mode silent address-ignore vs reject). |
| Low | 2 | FR-B5 message points at a tool that didn't exist (now does), OQ-1 ProTxHash encoding ambiguity. |

**Total: 6 findings** (1 High, 3 Medium, 2 Low).
