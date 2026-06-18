# Development Plan — Masternode/Evonode Withdrawals in det-cli

**Feature branch:** `feat/masternode-cli-withdraw` (off PR #860 base `docs/platform-wallet-migration-design` @976ad0d4).
**Inputs:** `01-requirements-ux.md`, `02-test-spec.md` (TC-MN cases), and the architecture de-risk.
**Phase:** 1d (Development Plan) of the feature workflow. Authored by the architect (Nagatha); transcribed/committed by the lead.

## Scope

Two new MCP tools exposing masternode/evonode credit withdrawal headlessly via det-cli:

- **`identity_masternode_load`** — load a masternode/evonode identity (ProTxHash + owner/voting/payout private keys) into the local store.
- **`identity_masternode_credits_withdraw`** — withdraw the node identity's Platform credits, with explicit owner/payout-key selection and the matching destination rules.

## Locked decisions (do not re-litigate)

- **No backend change.** Both tools dispatch existing tasks: `IdentityTask::LoadIdentity(IdentityInputToLoad)` and `IdentityTask::WithdrawFromIdentity(qi, Option<Address>, Credits, Option<KeyID>)`. No new `BackendTask`, `TaskError`, or `McpToolError` variant.
- **Tool logic in `src/mcp/tools/identity.rs`**; stateless parsing in a new `model/` validator. Register both in `src/mcp/server.rs::tool_router()`.
- **Secrets** mirror `core_wallet_import`: plain `String` params, wrapped in `Secret::new` inside `invoke`; **hand-written redacting `Debug`** on the param struct. Passed as inline `key=value` argv (env-var hardening deferred — see Follow-ups).
- **SPV gate:** both tools call `resolve::ensure_spv_synced` (they do proof-verified Platform reads). Resolves OQ-4 = option (b).
- **`identity_masternode_load`:** `node_type` ∈ {masternode, evonode} (never User); `pro_tx_hash` accepts hex **or** Base58; at least one of owner/payout key **required**; voting key + alias optional; `derive_keys_from_wallets:false`.
- **`identity_masternode_credits_withdraw`:** explicit `key_mode` ∈ {owner, transfer}. **OWNER** → `to_address` forced `None`, any supplied address rejected (`InvalidParam`); Platform routes to the registered payout address. **TRANSFER** → `to_address` required, any valid Core address, Platform addresses rejected. KeyID resolved from `available_withdrawal_keys()` by purpose. Destination is *also* enforced server-side by Platform consensus; the client check is a friendly pre-flight.

## Critical pitfall (must enforce)

`resolve::validate_address` is a **first-character-only** check and is **not** a substitute for `is_platform_address_string` (`src/model/address.rs:14`). Tool B must call `is_platform_address_string` explicitly to reject Platform addresses (verification points TC-MN-031 / TC-MN-046).

## Task breakdown (TDD — tests first per task)

| # | Task | Files | Test cases | Layer |
|---|------|-------|------------|-------|
| 1 | **`model/` validators** — `parse_node_type` (trim + case-insensitive; reject User/garbage — pins G-4), `parse_key_mode`, `require_at_least_one_signing_key`, identity-id decode (hex+Base58); reuse `is_platform_address_string`. | `src/model/masternode_input.rs` (new), `src/model/mod.rs` | TC-MN-001,002,003,004,008,009,030,031 | unit |
| 2 | **Tool A param struct + redacting `Debug`** — `IdentityMasternodeLoadParams`/`Output`; hand-written `Debug` redacts the 3 key fields (mirror `wallet.rs:397`). | `src/mcp/tools/identity.rs` | TC-MN-005,006,007,010,011 | unit |
| 3 | **Tool A `invoke`** — order: `require_network` → `parse_node_type` → key-presence → `ensure_spv_synced` → wrap keys in `Secret::new` → build `IdentityInputToLoad{derive_keys_from_wallets:false, keys_input:vec![]}` → dispatch `LoadIdentity` → map output (loaded keys + `available_withdrawal_keys` + `payout_address`). Register in `tool_router()`. | `src/mcp/tools/identity.rs`, `src/mcp/server.rs` | TC-MN-012,013,014,015 | tool-level |
| 4 | **Tool B param struct + pre-flight units** — `IdentityMasternodeWithdrawParams`/`Output`; pure checks (key_mode, amount=0, OWNER+address contradiction, missing/invalid address, Platform-address via `is_platform_address_string`). | `src/mcp/tools/identity.rs` | TC-MN-030,031,032,033,034,035 | unit |
| 5 | **Tool B `invoke`** — order: `require_network` → `validate_credits` → `parse_key_mode` → **OWNER+to_address contradiction first** → `qualified_identity` (error message names `identity-masternode-load` as the fix) → KeyID from `available_withdrawal_keys()` by purpose → destination rules (owner→payout/`None`; transfer→`is_platform_address_string` + `NetworkUnchecked` + cross-network reject) → `ensure_spv_synced` → dispatch `WithdrawFromIdentity(qi, dest, credits, Some(key_id))`. Register in `tool_router()`. | `src/mcp/tools/identity.rs`, `src/mcp/server.rs` | TC-MN-040,041,042,043,044,045,046,047 | tool-level |
| 6 | **Cross-cutting discoverability + error-redaction** — `tools/list` / `tool-describe` clean schemas, CLI hyphenation, `TaskFailed` `data`-payload non-leak of keys. | existing MCP tool test module | TC-MN-015,043,060,061 | tool-level |
| 7 | **Backend-e2e suite** (`#[ignore]`, `E2E_MN_*`) — mirror `identity_withdraw.rs`; every fund-moving assertion carries a variant + ≥1 number. | `tests/backend-e2e/identity_masternode_withdraw.rs` (new), `tests/backend-e2e/main.rs`, `README.md` (optional) | TC-MN-016..023, 050..054, 061 | backend-e2e |
| 8 | **Docs & user-stories** — `docs/MCP.md` (+ NFR-S4 / G-6 note), `docs/CLI.md`, `docs/user-stories.md` (add MCP-003 headless MN load, MCP-004 headless MN withdraw). | `docs/MCP.md`, `docs/CLI.md`, `docs/user-stories.md` | — | docs |

## Sequencing

1 → 2 → 3 → 4 → 5 → 6 → 7 → 8. Tasks 1–6 are the shippable core; 7 is network-gated (`#[ignore]`); 8 is docs. Each code task writes its tests first (from `02-test-spec.md`), confirms they fail, then implements to green. Run `cargo +nightly fmt` and `cargo clippy --all-features --all-targets -- -D warnings` before each commit.

## Coverage gaps carried from the test spec (G-1..G-6)

- **G-1** OWNER-mode forced destination is only end-to-end observable (client sends `to_address=None`; enforcement is Platform consensus) — covered in Task 7.
- **G-2** `masternode_payout_address() == None` needs a hand-built `QualifiedIdentity` fixture.
- **G-3** Withdraw SPV-gate presence: locked to *add the gate*; test ordering (validation before gate).
- **G-4** `node_type` normalization: trim + lowercase, pinned in Task 1.
- **G-5** voting-key voter-identity-not-found edge: no dedicated case — acceptable for v1.
- **G-6** NFR-S4 HTTP-transport key-logging is documentation-only (Task 8); in-process `Debug` redaction is tested (Task 2/6).

## Out of scope / follow-ups (tracked separately)

- Existing `identity_credits_withdraw` should also gate on `ensure_spv_synced` (memcan todo `10b6c02d`).
- Env-var fallback for private keys (keep secrets out of argv/shell history) — optional hardening.
- `voting_private_key` voter-identity edge cases beyond load.
