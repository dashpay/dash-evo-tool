# Masternode / Evonode Withdrawals via det-cli — Test-Case Specification

**Date:** 2026-06-18
**Phase:** 1c (Test Case Specification — specs, not code)
**Surface:** det-cli over MCP (headless). Two NEW tools in `src/mcp/tools/identity.rs`.
**Derives from:** `01-requirements-ux.md` (same directory) + locked design decisions.
**Status:** Draft for review. Each case is a contract a future test must encode; a
case is *passing* only when the test fails should the requirement be unmet.

---

## 0. Scope, layering, and ground truth

Two new MCP tools, both pure adapters over existing backend tasks (no backend or
`TaskError` change):

- **Tool A — `masternode_identity_load`** → dispatches `IdentityTask::LoadIdentity(IdentityInputToLoad)`.
- **Tool B — `masternode_credits_withdraw`** → dispatches `IdentityTask::WithdrawFromIdentity(qi, to_address, credits, Some(key_id))`.

### 0.1 Test layers

| Layer | Meaning | Runs in CI? |
|---|---|---|
| **unit** | Pure parsing / validation logic, no `AppContext`, no network, no DB. Covers `node_type` parse, ProTxHash hex+Base58 accept, `key_mode` parse, OWNER-mode-rejects-address pre-flight, amount/address/Platform-address validation, redacting `Debug`. | Yes (always). |
| **tool-level** | Tool `invoke` param validation and error mapping reachable *before* the SPV gate / network dispatch — network-mismatch, missing-key-mode, not-loaded, amount=0, OWNER+to_address reject. Driven through the in-process MCP service against a throwaway `AppContext`/DB (mirror `det-cli` smoke pattern in project `CLAUDE.md`). | Partially — only paths that return before `ensure_spv_synced`. Paths past the SPV gate are e2e. |
| **backend-e2e** | `#[ignore]`, network-dependent, serial. Real load by ProTxHash + real withdraw against testnet. Mirrors `tests/backend-e2e/` (shared `ctx()`, `run_task`, `#[tokio_shared_rt::test(shared, ...)]`). Env-gated by `E2E_MN_*` (see §0.3). | No (manual / nightly). |

### 0.2 Ground-truth references (confirmed against live code)

- `verify_key_input` (`src/backend_task/identity/mod.rs:450`): **64-char → hex**,
  **51/52-char → WIF**, **0 → `None` (not supplied)**, **any other length → error**.
  Single source of truth — tools MUST NOT re-implement.
- ProTxHash parse (`load_identity.rs:83`): `Identifier::from_string(Base58)` then
  fallback `Hex`. Both accepted; confirms OQ-1.
- `available_withdrawal_keys()` (`qualified_identity/mod.rs:745`): for
  Masternode/Evonode returns the **OWNER**-purpose and **TRANSFER**-purpose keys
  bound on the main identity. Owner key → loaded via `owner_private_key`
  (OWNER purpose); payout key → loaded via `payout_private_key` (TRANSFER purpose).
- `masternode_payout_address(network)` (`qualified_identity/mod.rs:691`): derived
  from the first `TRANSFER`/`CRITICAL` key (`ECDSA_HASH160` or `BIP13_SCRIPT_HASH`);
  returns `Option<Address>` — **`None` is reachable**, so FR-B2 "no payout address"
  is a real path.
- `withdraw_from_identity` (`withdraw_from_identity.rs`): passes `to_address` and
  the resolved `signing_key` straight to the SDK `withdraw`. With `to_address=None`
  + an OWNER signing key, **Platform consensus forces the registered payout
  address** — the client-side check is a friendly pre-flight, not the only guard.
- `Secret::Debug` (`src/model/secret.rs:235`) renders `Secret(***)`;
  `ImportWalletParams` (`wallet.rs:397`) hand-writes `Debug` → `<redacted>`. Tool A's
  params struct MUST do the same for its three key fields.
- Sibling `identity_credits_withdraw` (`identity.rs:445`) **intentionally skips**
  `ensure_spv_synced`. Tool B's locked decision is to **add** the gate (NFR-P2 /
  OQ-4 option b) — this divergence from the sibling is a deliberate, test-verified
  choice, not an accident.
- `is_platform_address_string` (`src/model/address.rs:14`) is the Platform-address
  guard. **Pitfall flagged:** `resolve::validate_address` (`resolve.rs:194`) only
  checks the first char against `{X,7,y,8,9}`; it is *not* a substitute for the
  Platform-address guard and would mis-handle `dash1…`/`tdash1…`. Tool B MUST call
  `is_platform_address_string` explicitly (see TC-MN-031 / TC-MN-046).

### 0.3 Proposed e2e env gates (`E2E_MN_*`)

Mirror `E2E_WALLET_MNEMONIC`. All e2e cases skip-with-log if unset (never fail on
absence — they are `#[ignore]` anyway).

| Var | Used by | Notes |
|---|---|---|
| `E2E_MN_PRO_TX_HASH` | load + composition | testnet evonode/masternode ProTxHash (hex). |
| `E2E_MN_OWNER_WIF` | owner-mode cases | owner private key (WIF or 64-hex). |
| `E2E_MN_PAYOUT_WIF` | transfer-mode + payout cases | payout/transfer private key. |
| `E2E_MN_VOTING_WIF` | voting-key case | optional; triggers voter-identity fetch. |
| `E2E_MN_NODE_TYPE` | load | `masternode` or `evonode`; default `evonode`. |

---

## 1. Tool A — `masternode_identity_load`

### Unit layer (no network)

#### TC-MN-001 — node_type "masternode" parses to Masternode
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Parse `node_type="masternode"` through the tool's node-type mapper.
- **Expected:** Maps to `IdentityType::Masternode`. No error.
- **Traces:** FR-A1, table row "node_type", AC-A1.

#### TC-MN-002 — node_type "evonode" parses to Evonode
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Parse `node_type="evonode"`.
- **Expected:** Maps to `IdentityType::Evonode`. No error.
- **Traces:** FR-A1, AC-A1.

#### TC-MN-003 — node_type "user" rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Parse `node_type="user"`.
- **Expected:** `McpToolError::InvalidParam` with message
  `The 'node_type' must be "masternode" or "evonode".`; **never** maps to
  `IdentityType::User`.
- **Traces:** FR-A1, AC-A3, Error-UX row 1.

#### TC-MN-004 — node_type unknown/garbage rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Parse `node_type` = `"MASTERNODE "` (trailing space), `"evo"`, `""`, `"node"`.
- **Expected:** Each → `InvalidParam` (case/whitespace handling must be explicit —
  document whether trimming/lowercasing applies; assert the actual chosen policy).
- **Traces:** FR-A1.

#### TC-MN-005 — ProTxHash accepted as 64-char hex
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Pass a valid 64-hex ProTxHash to the identifier parse path
  (`from_string` Base58→Hex fallback).
- **Expected:** Parses to an `Identifier` (no error). Asserts the hex branch is reached.
- **Traces:** FR-A "pro_tx_hash", OQ-1, AC-A1.

#### TC-MN-006 — ProTxHash accepted as Base58
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Pass the **same** identity ID encoded as Base58.
- **Expected:** Parses to an `Identifier` equal (byte-for-byte) to the one from
  TC-MN-005's hex form for the same underlying bytes.
- **Traces:** FR-A "pro_tx_hash", OQ-1.

#### TC-MN-007 — ProTxHash malformed rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Pass `"not-a-hash"`, `""`, a 63-char hex, a 65-char hex.
- **Expected:** Surfaces as `TaskFailed(IdentifierParsingError { input })` from the
  backend (the tool does not pre-validate length; confirm the parse is delegated and
  the original input string is preserved in the typed variant — never string-parsed).
- **Traces:** Error-UX row "ProTxHash unparseable".

#### TC-MN-008 — at least one of owner/payout required (both absent → reject)
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Build params with `owner_private_key`/`payout_private_key` both empty
  (or omitted); `voting_private_key` may be present or absent.
- **Expected:** `InvalidParam` whose message names **both** keys and explains the two
  withdraw modes:
  `Provide at least one of the owner or payout private key. The owner key withdraws to the registered payout address; the payout key withdraws to any address.`
  Check fires **before** any network/SPV call.
- **Traces:** FR-A4, AC-A4, Error-UX row 2.

#### TC-MN-009 — voting key alone does NOT satisfy the key requirement
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** `voting_private_key` set, `owner`/`payout` both empty.
- **Expected:** Same `InvalidParam` as TC-MN-008 (voting key binds the voter
  identity only; it enables no withdrawal).
- **Traces:** FR-A4 (parenthetical).

#### TC-MN-010 — params Debug redacts every private key
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Construct the load params struct with non-empty owner, voting, payout
  key strings (use obvious sentinels, e.g. `"OWNER_SECRET_VALUE"`). Format with
  `{:?}`.
- **Expected:** Output contains none of the three sentinel substrings; each key
  field renders as `<redacted>` (or `Secret(***)` if wrapped first). `pro_tx_hash`,
  `node_type`, `alias`, `network` may appear in cleartext.
- **Traces:** NFR-S1, NFR-S3, AC-A8. **This is the single most important unit test.**

#### TC-MN-011 — key format delegated to verify_key_input (length policy)
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Document (not re-implement) that the tool feeds raw key strings into
  `Secret` → `IdentityInputToLoad` → `verify_key_input`. Provide a table-of-record:
  64-hex→hex, 51/52→WIF, 0→None, else→error. Assert the tool adds **no** competing
  length check of its own.
- **Expected:** Wrong-length / non-hex / bad-WIF keys are rejected by the backend as
  `KeyInputValidationFailed { key_name, detail }`, role-named, value never echoed.
- **Traces:** FR-A "Key formats", NFR-S3, AC-A5, Error-UX row 3.

### Tool-level (pre-SPV-gate paths, no live network)

#### TC-MN-012 — network mismatch fails before any network call
- **Layer:** tool-level
- **Preconditions:** Active network = testnet (throwaway `AppContext`).
- **Steps:** Invoke with `network=mainnet`, otherwise-valid params.
- **Expected:** `NetworkMismatch { expected: "mainnet", actual: "testnet" }`. No SPV
  start, no DAPI fetch. (`require_network` runs first — confirm via TC-MN-010-style
  no-side-effect assertion if observable.)
- **Traces:** FR-3.3, NFR-N1, AC-A6, Error-UX row "Network missing/mismatch".

#### TC-MN-013 — network param missing/blank → InvalidParam
- **Layer:** tool-level
- **Preconditions:** any active network.
- **Steps:** Omit `network`, or pass an empty/whitespace string.
- **Expected:** `InvalidParam`, not a panic, not `NetworkMismatch`. Note the two
  distinct paths: an **omitted** `network` is a schema-required deserialization
  error (the field has no `#[serde(default)]`); an **empty/blank** string is caught
  by the tool's `require_nonblank_network` guard, which runs before
  `require_network` and returns "The network parameter is required." rather than a
  confusing `NetworkMismatch { expected: "" }`.
- **Traces:** FR-3.3, Error-UX row "Network missing/mismatch".

#### TC-MN-014 — node_type/key-requirement checks run before SPV gate
- **Layer:** tool-level
- **Preconditions:** Active network = testnet; SPV NOT synced.
- **Steps:** Invoke with `node_type=user` (TC-MN-003 condition) and valid network.
- **Expected:** Returns `InvalidParam` **immediately**, without blocking on
  `ensure_spv_synced`. Verifies ordering: cheap validation precedes the SPV wait.
- **Traces:** FR-A1 + NFR-P1 (ordering), AC-A3.

#### TC-MN-015 — annotations & schema (discoverability)
- **Layer:** tool-level
- **Preconditions:** in-process MCP service.
- **Steps:** `tools/list` and `tool-describe name=masternode_identity_load`.
- **Expected:** Tool appears; annotations `read_only=false, destructive=false,
  idempotent=false, open_world=true`; schema is valid JSON, exposes `pro_tx_hash,
  node_type, owner_private_key, voting_private_key, payout_private_key, alias,
  network`; CLI name hyphenates to `masternode-identity-load`.
- **Traces:** NFR-D1, NFR-D2, AC-X1, AC-X3.

### Backend-e2e (`#[ignore]`, network)

#### TC-MN-016 — load happy path: evonode + payout key
- **Layer:** backend-e2e
- **Preconditions:** `E2E_MN_PRO_TX_HASH`, `E2E_MN_PAYOUT_WIF` set; testnet; SPV
  synced; ProTxHash refers to a real testnet evonode with a registered payout addr.
- **Steps:** Dispatch `LoadIdentity` (built as the tool would) with
  `node_type=evonode`, payout key only, `network=testnet`.
- **Expected:** `BackendTaskSuccessResult::LoadedIdentity(qi)`. Assert:
  `payout_key_loaded=true`, `owner_key_loaded=false`, `available_withdrawal_keys`
  **contains** `"transfer"`, `payout_address` is `Some(..)` and is a valid testnet
  Core address, identity resolvable afterward via `resolve::qualified_identity`.
- **Traces:** AC-A1, FR-A output fields, FR-A3.

#### TC-MN-017 — load happy path: masternode + owner key
- **Layer:** backend-e2e
- **Preconditions:** `E2E_MN_PRO_TX_HASH` (a masternode), `E2E_MN_OWNER_WIF`.
- **Steps:** `LoadIdentity` `node_type=masternode`, owner key only.
- **Expected:** `LoadedIdentity(qi)`; `owner_key_loaded=true`;
  `available_withdrawal_keys` contains `"owner"`.
- **Traces:** AC-A2, A-2 (MN vs Evonode parity).

#### TC-MN-018 — load with both owner + payout keys
- **Layer:** backend-e2e
- **Preconditions:** ProTxHash + both WIFs.
- **Steps:** Load with both keys.
- **Expected:** `available_withdrawal_keys` contains **both** `"owner"` and
  `"transfer"`; both `*_key_loaded` booleans true.
- **Traces:** FR-A output, FR-B1 (sets up the both-modes-available case).

#### TC-MN-019 — load with voting key fetches voter identity & binds it
- **Layer:** backend-e2e
- **Preconditions:** ProTxHash + payout WIF + `E2E_MN_VOTING_WIF`.
- **Steps:** Load with payout + voting keys.
- **Expected:** Success; the associated voter identity is bound (load performs the
  extra DAPI fetch). Voting key does **not** add a withdrawal mode —
  `available_withdrawal_keys` is unchanged vs TC-MN-016.
- **Traces:** FR-A "voting_private_key", FR-A4 parenthetical, OQ-5.

#### TC-MN-020 — load: key not present on identity (wrong key) → KeyInputValidationFailed
- **Layer:** backend-e2e
- **Preconditions:** Valid ProTxHash, but a **valid-format** WIF that is NOT a key on
  that identity.
- **Steps:** Load with the mismatched owner (or payout) key.
- **Expected:** `TaskFailed(KeyInputValidationFailed { key_name, detail })` naming the
  role; the key value appears **nowhere** in `Display` or the `data` payload.
- **Traces:** AC-A5, Error-UX row "Key not present", NFR-S3.

#### TC-MN-021 — load: identity not found on network → IdentityNotFound
- **Layer:** backend-e2e
- **Preconditions:** Well-formed but nonexistent ProTxHash (random 64-hex), testnet,
  SPV synced.
- **Steps:** Load.
- **Expected:** `TaskFailed(IdentityNotFound)`; user-facing Display is the
  network-friendly "not found, check ProTxHash and network" wording.
- **Traces:** Error-UX row "Identity not found".

#### TC-MN-022 — load: SPV not synced → SpvSyncFailed (gate present)
- **Layer:** backend-e2e (or tool-level if SPV can be forced to Error without network)
- **Preconditions:** SPV in `Error`/never-synced state (e.g. point at an unreachable
  network or force the status), tool dispatched.
- **Steps:** Invoke load past param validation.
- **Expected:** `ensure_spv_synced` is invoked and, on failure/timeout, returns
  `SpvSyncFailed`. Asserts the load tool **has** the SPV gate (NFR-P1) — contrast
  with the sibling withdraw tool which historically skipped it.
- **Traces:** NFR-P1, Error-UX row "SPV not synced".

#### TC-MN-023 — load re-run is idempotent at the DB layer
- **Layer:** backend-e2e
- **Preconditions:** TC-MN-016 already ran (identity present).
- **Steps:** Load the same identity again with the same keys.
- **Expected:** Success again; local DB row is INSERT-OR-REPLACEd (one row, updated
  keys), no duplicate. Confirms FR-A5's "safe to re-run" documentation claim even
  though the tool annotates `idempotent=false`.
- **Traces:** FR-A5.

---

## 2. Tool B — `masternode_credits_withdraw`

### Unit layer (no network)

#### TC-MN-030 — key_mode "owner"/"transfer" parse; unknown rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Parse `key_mode` = `"owner"`, `"transfer"`, and `"foo"`/`""`/`"OWNER "`.
- **Expected:** `"owner"`/`"transfer"` map to the two internal modes; anything else →
  `InvalidParam` `The 'key_mode' must be "owner" or "transfer".`
- **Traces:** FR-B1, Error-UX row "key_mode unknown".

#### TC-MN-031 — Platform-address detection uses is_platform_address_string
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Feed mainnet `dash1…` and testnet `tdash1…` sample bech32m strings, plus
  a Core `y…`/`X…` address, into the Platform-address guard the tool uses.
- **Expected:** Both `dash1…`/`tdash1…` → detected as Platform (rejected downstream);
  Core addresses → not Platform. Asserts the tool calls `is_platform_address_string`
  (not the weaker first-char `resolve::validate_address`). **Pitfall guard:** a test
  must prove a `dash1…` string is rejected as Platform and not silently passed.
- **Traces:** FR-B3, Error-UX row "TRANSFER + Platform address", §0.2 pitfall.

#### TC-MN-032 — amount_credits == 0 rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** Call `resolve::validate_credits(0)` via the tool path.
- **Expected:** `InvalidParam` `amount_credits must be greater than zero.`
- **Traces:** FR-B "amount_credits", AC-B9, Error-UX row "amount_credits == 0".

#### TC-MN-033 — OWNER mode + supplied to_address rejected (pure pre-flight)
- **Layer:** unit
- **Preconditions:** none — this is a pure param-combination check that must fire
  before any identity resolution or network call.
- **Steps:** `key_mode=owner` with a non-empty `to_address`.
- **Expected:** `InvalidParam`
  `An owner-key withdrawal always goes to the registered payout address. Remove 'to_address', or use key_mode=transfer to choose an address.`
  The rejection does **not** require the identity to be loaded (check ordering: the
  contradiction is surfaced even for an unknown identity, OR document that it runs
  after resolution — pick one and assert it; preferred: reject early on the param
  contradiction).
- **Traces:** FR-B2, AC-B2, Error-UX row "OWNER + to_address".

#### TC-MN-034 — TRANSFER mode + missing to_address rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** `key_mode=transfer` with empty/omitted `to_address`.
- **Expected:** `InvalidParam` requiring a Core address.
- **Traces:** FR-B3, AC-B5.

#### TC-MN-035 — TRANSFER mode + invalid Core address rejected
- **Layer:** unit
- **Preconditions:** none.
- **Steps:** `key_mode=transfer to_address="not-an-address"`.
- **Expected:** `InvalidParam` (format failure via the `NetworkUnchecked` parse path).
  Asserts the address is parsed, not just first-char checked.
- **Traces:** FR-B3, Error-UX row "TRANSFER + invalid address".

### Tool-level (pre-SPV-gate / pre-dispatch paths)

#### TC-MN-040 — withdraw before load → InvalidParam pointing at the load tool
- **Layer:** tool-level
- **Preconditions:** empty local DB; active testnet.
- **Steps:** `masternode_credits_withdraw identity_id=<never-loaded base58>
  key_mode=transfer to_address=<valid core> amount_credits=N network=testnet`.
- **Expected:** `InvalidParam` whose message names `masternode-identity-load`
  (FR-B5 reworded). Resolution fails at `resolve::qualified_identity` before dispatch.
- **Traces:** FR-B5, AC-B8, Error-UX row "Withdraw before load".

#### TC-MN-041 — network mismatch → NetworkMismatch
- **Layer:** tool-level
- **Preconditions:** active testnet.
- **Steps:** Invoke with `network=mainnet`.
- **Expected:** `NetworkMismatch { expected: "mainnet", actual: "testnet" }` before
  dispatch.
- **Traces:** FR-3.3, AC-B9, NFR-N1.

#### TC-MN-042 — amount=0 and OWNER+address rejected before SPV gate
- **Layer:** tool-level
- **Preconditions:** active testnet; SPV not synced.
- **Steps:** (a) `amount_credits=0`; (b) `key_mode=owner to_address=y…`.
- **Expected:** Both return `InvalidParam` immediately, without blocking on the SPV
  gate. Confirms cheap validation precedes the (locked-on) SPV wait.
- **Traces:** FR-B2, AC-B2, AC-B9 + NFR-P2 ordering.

#### TC-MN-043 — annotations & schema
- **Layer:** tool-level
- **Preconditions:** in-process MCP service.
- **Steps:** `tools/list`; `tool-describe name=masternode_credits_withdraw`.
- **Expected:** Present; annotations `read_only=false, destructive=true,
  idempotent=false, open_world=true` (identical to `identity_credits_withdraw`);
  schema exposes `identity_id, key_mode, to_address, amount_credits, network`; CLI
  name `masternode-credits-withdraw`.
- **Traces:** NFR-D2, AC-X1, AC-X3.

#### TC-MN-044 — mode key not loaded → InvalidParam naming the missing key
- **Layer:** tool-level (needs a loaded identity record; can use a DB-seeded
  payout-only `QualifiedIdentity` fixture without live network)
- **Preconditions:** Identity loaded with **only** a payout key (DB fixture or
  TC-MN-016 result).
- **Steps:** `key_mode=owner amount_credits=N network=testnet` (no to_address).
- **Expected:** `InvalidParam`
  `The owner key needed for this withdrawal is not loaded. Re-run masternode-identity-load and include it.`
  Resolution scans `available_withdrawal_keys()` for an OWNER-purpose key, finds
  none, and rejects **before** dispatch (no funds move).
- **Traces:** FR-B1, AC-B7, Error-UX row "key_mode key not loaded".

#### TC-MN-045 — OWNER mode + no payout address → InvalidParam
- **Layer:** tool-level (fixture: MN identity loaded with an owner key but
  `masternode_payout_address()` → `None`)
- **Preconditions:** Loaded identity whose first TRANSFER/CRITICAL key is absent so
  `masternode_payout_address(network)` returns `None`.
- **Steps:** `key_mode=owner amount_credits=N` (no to_address).
- **Expected:** `InvalidParam`
  `This identity has no registered payout address, so an owner-key withdrawal has no destination. Use key_mode=transfer with a Core address.`
  No dispatch.
- **Traces:** FR-B2 (no-payout-address bullet), AC-B3.

#### TC-MN-046 — TRANSFER mode + Platform address → InvalidParam (Core-only)
- **Layer:** tool-level (fixture identity with a transfer key)
- **Preconditions:** Loaded identity with a transfer key; active testnet.
- **Steps:** `key_mode=transfer to_address=tdash1…<valid platform> amount_credits=N`.
- **Expected:** `InvalidParam`
  `Enter a valid Core address — Platform addresses cannot receive withdrawals.`
  via `is_platform_address_string`. No dispatch. **Must not** slip through
  `validate_address`.
- **Traces:** FR-B3, AC-B6, Error-UX row "TRANSFER + Platform address".

#### TC-MN-047 — TRANSFER mode + cross-network Core address → InvalidParam
- **Layer:** tool-level
- **Preconditions:** active testnet; loaded transfer-key identity.
- **Steps:** `key_mode=transfer to_address=<mainnet X…> amount_credits=N
  network=testnet`.
- **Expected:** `InvalidParam` "address does not match the active network"
  (`require_network(ctx.network())` on the parsed `NetworkUnchecked` address). No
  dispatch.
- **Traces:** FR-B3 (network match), NFR-N1.

### Backend-e2e (`#[ignore]`, network)

#### TC-MN-050 — OWNER mode happy path: destination forced to payout, to_address=None
- **Layer:** backend-e2e
- **Preconditions:** Identity loaded with an owner key and a registered payout
  address (TC-MN-017 / TC-MN-018), funded with withdrawable credits; testnet synced.
- **Steps:** Withdraw `key_mode=owner amount_credits=N` with **no** `to_address`.
- **Expected:** Tool resolves the OWNER `KeyID` from `available_withdrawal_keys()`,
  resolves destination from `masternode_payout_address(network)`, dispatches
  `WithdrawFromIdentity(qi, to_address=None, N, Some(owner_key_id))`. Result
  `WithdrewFromIdentity(fee)`. Output `to_address` **equals the resolved payout
  address** (the address actually used, echoed back), `key_mode="owner"`,
  `estimated_fee`/`actual_fee` populated.
- **Traces:** FR-B2, AC-B1, AC-B10, FR-B output "address actually used".
- **Note:** This case verifies the client passes `to_address=None`; Platform
  consensus also forces the payout address server-side (defense in depth). A test
  should assert the **client output** reports the payout address, since the raw ST
  carries `None`.

#### TC-MN-051 — TRANSFER mode happy path: any Core address
- **Layer:** backend-e2e
- **Preconditions:** Identity loaded with a transfer (payout) key, funded; testnet.
- **Steps:** Withdraw `key_mode=transfer to_address=<fresh testnet y… address>
  amount_credits=N`.
- **Expected:** Resolves the TRANSFER `KeyID`, dispatches
  `WithdrawFromIdentity(qi, Some(parsed_core_addr), N, Some(transfer_key_id))`.
  Result `WithdrewFromIdentity(fee)`. Output `to_address` echoes the **caller's**
  address; `key_mode="transfer"`; fees populated.
- **Traces:** FR-B3, AC-B4, AC-B10.

#### TC-MN-052 — OWNER mode + to_address supplied rejected (no network spend)
- **Layer:** backend-e2e (or tool-level — see TC-MN-033/042)
- **Preconditions:** Loaded owner-key identity.
- **Steps:** `key_mode=owner to_address=<some y… addr> amount_credits=N`.
- **Expected:** `InvalidParam` (FR-B2) **before** any ST is broadcast — assert no
  balance change on the identity.
- **Traces:** FR-B2, AC-B2.

#### TC-MN-053 — A→B composition through the local DB
- **Layer:** backend-e2e
- **Preconditions:** Clean DB; `E2E_MN_PRO_TX_HASH` + a withdrawal-capable key;
  testnet synced; identity funded with credits.
- **Steps:** (1) Dispatch the load (Tool A) → persist locally. (2) Without any shared
  in-memory handoff, invoke the withdraw (Tool B) for the returned `identity_id`.
- **Expected:** Step 2 resolves the identity via `resolve::qualified_identity` (no
  "not loaded" error) and completes a withdrawal. Proves the two tools compose
  **only** through `insert_local_qualified_identity` → `get_identity_by_id`, exactly
  like the GUI→withdraw flow.
- **Traces:** §3.4 Composition, AC-A7.

#### TC-MN-054 — withdraw key not loaded (e2e mirror of TC-MN-044)
- **Layer:** backend-e2e
- **Preconditions:** Real identity loaded with **only** a payout key.
- **Steps:** `key_mode=owner` (owner key absent).
- **Expected:** `InvalidParam` naming the missing owner key; no ST broadcast; balance
  unchanged.
- **Traces:** FR-B1, AC-B7.

---

## 3. Cross-cutting

#### TC-MN-060 — both tools discoverable & describable (smoke)
- **Layer:** tool-level
- **Preconditions:** in-process MCP service (no network).
- **Steps:** `det-cli tools`; `det-cli tool-describe name=masternode_identity_load`;
  `det-cli tool-describe name=masternode_credits_withdraw`.
- **Expected:** Both names listed; both `tool-describe` calls return clean,
  client-acceptable JSON schemas (no bare-`true` schemar quirks); registered one line
  each in `tool_router()`; zero tool logic in `src/bin/det_cli/`.
- **Traces:** AC-X1, AC-X2, AC-X3, NFR-D4.

#### TC-MN-061 — TaskFailed data payload never leaks key material
- **Layer:** backend-e2e (uses TC-MN-020's wrong-key path)
- **Preconditions:** load with a valid-format key not on the identity.
- **Steps:** Capture the full `McpError` (Display message **and** the `data`
  payload built from `format!("{task_err:?}")`).
- **Expected:** Neither the message nor the `data` Debug chain contains the key WIF /
  hex. Because keys live in `Secret` (Debug = `Secret(***)`) and the error variant is
  `KeyInputValidationFailed { key_name, detail }` (no key bytes), this holds — assert
  it explicitly given the `data` payload serializes the Debug chain.
- **Traces:** NFR-S1, NFR-S3, AC-A5; cross-checks `error.rs` `TaskFailed` `data`.

---

## 4. Coverage matrix (acceptance criteria → cases)

| AC | Cases |
|---|---|
| AC-A1 | TC-MN-001, 002, 005, 016 |
| AC-A2 | TC-MN-017 |
| AC-A3 | TC-MN-003, 014 |
| AC-A4 | TC-MN-008, 009 |
| AC-A5 | TC-MN-011, 020, 061 |
| AC-A6 | TC-MN-012 |
| AC-A7 | TC-MN-053 |
| AC-A8 | TC-MN-010 |
| AC-B1 | TC-MN-050 |
| AC-B2 | TC-MN-033, 042, 052 |
| AC-B3 | TC-MN-045 |
| AC-B4 | TC-MN-051 |
| AC-B5 | TC-MN-034 |
| AC-B6 | TC-MN-031, 046 |
| AC-B7 | TC-MN-044, 054 |
| AC-B8 | TC-MN-040 |
| AC-B9 | TC-MN-032, 041, 042 |
| AC-B10 | TC-MN-050, 051 |
| AC-X1 | TC-MN-015, 043, 060 |
| AC-X2 | TC-MN-060 |
| AC-X3 | TC-MN-015, 043, 060 |
| NFR-P1 (load SPV gate) | TC-MN-022 |
| NFR-P2 (withdraw SPV gate, OQ-4 b) | TC-MN-042 (ordering); see Gap G-3 |
| OQ-1 (hex+Base58) | TC-MN-005, 006 |
| OQ-5 (voting key) | TC-MN-019 |

---

## 5. Coverage gaps & risks (flagged)

- **G-1 — OWNER-mode forced destination is only *observable* end-to-end.** The client
  passes `to_address=None`; the actual payout-address enforcement is Platform
  consensus. A unit/tool-level test can only assert the tool resolves the payout
  address into its **output echo** and dispatches `None` — it cannot prove consensus
  rejects a forged owner-key→arbitrary-address ST without a live network (and a
  deliberately malformed ST the tool will never build). TC-MN-050 covers the
  happy-path echo; the negative consensus path is **out of unit/tool reach** and is
  accepted as defense-in-depth, not a tested client guarantee.

- **G-2 — `masternode_payout_address() == None` is hard to provoke against a real
  node.** TC-MN-045 needs a fixture MN identity whose loaded keys lack a
  TRANSFER/CRITICAL key. This is a constructed `QualifiedIdentity` (tool-level
  fixture), not a natural testnet node — flag that the fixture must be hand-built and
  kept in sync with `masternode_payout_address`'s key-matching rules.

- **G-3 — Withdraw SPV-gate presence can only be ordering-tested cheaply.** The locked
  decision (OQ-4 option b) adds `ensure_spv_synced` to Tool B, diverging from the
  sibling `identity_credits_withdraw` which skips it. There is **no pure unit test**
  for "the gate exists"; TC-MN-042 only proves cheap validation runs *before* the
  gate. A true gate-present assertion needs an e2e/forced-SPV-error harness
  (analogous to TC-MN-022). Flag: if the implementer instead picks OQ-4 option (a)
  (skip the gate to match the sibling), TC-MN-022's withdraw analogue and this row
  must be revised — the test spec currently assumes option (b).

- **G-4 — `node_type` normalization policy is unspecified.** TC-MN-004 asserts
  rejection of `"MASTERNODE "` etc., but the requirements don't state whether the tool
  trims/lowercases. The test must encode whatever the implementer chooses; until then
  this is an under-specified contract (flag for Phase 2 to pin down, then make the
  test exact).

- **G-5 — Voting-key-only voter-identity-not-found path is untested.** TC-MN-019
  covers the happy voter fetch, but a voting key whose voter identity is absent on
  network returns `IdentityNotFound` from a *second* fetch (load_identity.rs:188).
  No dedicated case — flag as a thin-coverage edge if voting-key support ships in v1
  (OQ-5).

- **G-6 — `det-cli` HTTP-transport key handling (NFR-S4) is documentation-only.** No
  automated test asserts keys aren't logged over HTTP; it's a `docs/MCP.md` note.
  Flag: the redaction unit test (TC-MN-010) is the only programmatic guard, and it
  only covers the in-process `Debug` path, not transport-layer logging.

---

## 6. Notes for the implementing test author

- Reuse the `det-cli` standalone smoke pattern (project `CLAUDE.md` → "Smoke-testing
  changes with det-cli") for tool-level discovery/schema cases (TC-MN-015, 043, 060) —
  these need no funds and no SPV.
- Backend-e2e cases go in a new `tests/backend-e2e/identity_masternode_withdraw.rs`,
  mirroring `identity_withdraw.rs` (shared `ctx()`, `run_task`,
  `#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]`,
  `#[ignore]`). Register the module in `tests/backend-e2e/main.rs`.
- Never assert on error **strings** for control flow in the tests beyond the
  user-facing `Display` text the spec quotes — match on `McpToolError` /
  `TaskError` **variants** (project rule: never parse error strings).
- Every fund-moving e2e assertion must check the result variant **and** at least one
  number (fee or balance delta) — not merely "did not error" (QA test-depth rule).
</content>
</invoke>
