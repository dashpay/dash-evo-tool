# MCP — CLI / MCP Server

Environment: PR892 build (`57195d54`), binary `det-cli` built fresh from
`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build` with
`cargo build --bin det-cli --features cli` (and, for the HTTP transport check, `--features
headless`), landing at `/data/target/debug/det-cli` (shared cargo target dir, confirmed via
`cargo metadata`). No GUI/display involved — this category is a pure CLI/JSON-RPC surface.

**Isolation note**: this category does not use `/data/tmp/det-qa-pr892-data` (the main GUI
campaign data dir) at all. Two dedicated throwaway data dirs were used instead, both created
fresh for this pass and never touched by anything else:
- `/data/tmp/det-qa-mcp-cli-data` — stdio/standalone CLI testing (`det-cli <cmd>`, `det-cli
  serve`).
- `/data/tmp/det-qa-mcp-http-data` — headless HTTP transport testing (`det-cli headless`,
  listening on `127.0.0.1:19527`, a non-default port chosen to avoid any collision risk with
  the main GUI instance even though that instance has `MCP_API_KEY` unset/HTTP-disabled).

The main GUI campaign process (PID 989399, `/data/tmp/det-qa-pr892-data`) was left completely
untouched throughout — confirmed running before, during, and after this pass; its `.env` has
`MCP_API_KEY=` (empty), so it never exposed an HTTP MCP endpoint to begin with. (Aside: `/tmp`
and `/data/tmp` resolve to the same `ext4` device/inode on this host — `stat` confirms identical
device/inode numbers for paths under each — so "under `/tmp`" and "under `/data/tmp`" are the
same physical location here; this was checked specifically to rule out any accidental overlap
with the main campaign dir, and none was found.)

All destructive/fund-moving tool parameters in this pass used well-known public BIP-39 test
vectors (e.g. `abandon abandon ... about`, `legal winner thank year wave sausage worth useful
legal winner thank yellow`) — never the campaign's funded QA wallet mnemonic — since MCP-001/002
only require exercising wallet **management** mechanics (import/list/derive), not real funds.

## MCP-001: Manage wallets via CLI — **FAIL**

Acceptance criteria (from `docs/user-stories.md`): list wallets, check balances, generate
addresses, and send funds from the command line; CLI discovers tools dynamically via MCP
protocol; shell completion for tool names and parameters.

### What works

- **Dynamic tool discovery**: `det-cli tools` (no cache, no context) lists all 30 commands
  (26 MCP tools + 4 CLI-only meta-commands: `tools`, `tool-describe`, `serve`, `completion`)
  with full descriptions and per-parameter help, matching `docs/MCP.md`'s tool table exactly.
- **`network-info`**: `{"active":"mainnet","available":["mainnet","testnet","devnet","local"]}`
  — instant (146ms), no context/SPV needed, confirms the network-exempt fast path.
- **`tool-describe`**: returns full JSON Schema (input + output + annotations) for any tool,
  e.g. `core_wallets_list`, `core_funds_send`, `masternode_identity_load` (confirmed the
  `Secret` param type collapses private-key fields to `{"type":"string"}` with no leakage of
  constraints that would hint at key material). Unknown tool name correctly errors
  `Tool 'nonexistent_tool' not found` (code -32602).
- **`core-wallet-import`**: imports a BIP-39 mnemonic, returns a `seed_hash`, and is genuinely
  idempotent — re-running the same import returns `already_imported:true` with the same hash
  instead of erroring or duplicating.
- **Parameter validation**: missing required fields (`address` for `core-funds-send`, `network`
  for `core-wallet-import`) are rejected with clear `missing field '<name>'` errors (code
  -32602), not a panic or an opaque failure.
- **Full dispatch chain**: `core-wallets-list` on a brand-new data dir correctly drives
  MCP service → tool → `AppContext` → SQLite, creating `det-app.sqlite`, the secrets vault, and
  `data.db` on first call.

### What's broken: imported wallets are invisible to every subsequent CLI command

This is the headline finding, and it breaks the core "manage wallets via CLI" promise for the
CLI's own documented usage pattern (`docs/CLI.md`'s every example is a **separate** `det-cli
<command>` process invocation):

```
$ det-cli core-wallet-import mnemonic="abandon ... about" network=mainnet alias=mcp-qa-test-wallet
{"seed_hash":"62a772f8...","alias":"mcp-qa-test-wallet","already_imported":false}

$ det-cli core-wallets-list
{"wallets":[]}                     # <-- the wallet just imported is not there

$ det-cli core-address-create wallet-id=mcp-qa-test-wallet network=mainnet
Error: Wallet not found: "mcp-qa-test-wallet" (no wallets loaded)   (code -32001)

$ det-cli core-balances-get wallet-id=mcp-qa-test-wallet
Error: Wallet not found: "mcp-qa-test-wallet" (no wallets loaded)   (code -32001)
```

Reproduced identically for a second, never-before-seen mnemonic (ruling out a
stale-cache/one-off explanation), and reproduced with `det-cli serve` (stdio JSON-RPC, single
long-running process) when the import call targets a wallet that was already persisted from an
**earlier** process — i.e. the failure is not "each CLI invocation is a new process" alone, it's
that a previously-imported wallet's `already_imported:true` fast path also never registers the
wallet in the current process's live map. Direct SQLite inspection of the data dir confirms the
`wallets` and `meta_wallet` tables have **zero rows** after two successful, idempotency-verified
imports — the import writes the encrypted seed to the secrets vault (`det-secrets.pwsvault`
grows from empty to 615 bytes after the first import) but the app-level wallet registration
sidecar is never durably written where a fresh process's hydration step would find it.

**Root cause** (confirmed by reading the source, `src/mcp/tools/wallet.rs` and
`src/context/wallet_lifecycle/registration.rs`): `core_wallets_list` (`wallet.rs:520-539`) reads
only the in-memory `ctx.wallets` `RwLock<BTreeMap>` — it never calls
`ctx.ensure_wallet_backend(...)` or any hydration step. That in-memory map starts empty on every
fresh `AppContext` (standalone mode is a brand-new `AppContext` per process, or per lazy-init in
a `serve` session) and is only rebuilt from persisted state by
`WalletBackend::hydrate_context_wallets` (`wallet_backend/mod.rs:431-469`), which is reachable
**only** via `ctx.ensure_wallet_backend(...)`, which in turn is invoked **only** from
`resolve::ensure_spv_synced` (`mcp/resolve.rs`). `docs/MCP.md`'s own SPV-requirements section
explicitly lists `core_wallets_list` and `core_wallet_import` among the tools that "make no
network calls" and therefore skip that gate — which is correct for avoiding an SPV wait, but as
an unintended side effect it also skips the (network-free, local-only) hydration call, so a
freshly-imported wallet is registered into the vault/DB but never into the map the list/lookup
tools read from.

**Precise boundary, confirmed empirically**: a *fresh* (never-before-imported) mnemonic imported
and immediately listed **within the same `det-cli serve` process** does work — `register_wallet`
inserts into `ctx.wallets` directly on its non-duplicate success path:
```
core_wallet_import  -> {"seed_hash":"89c4a8ef...","already_imported":false}
core_wallets_list   -> {"wallets":[{"seed_hash":"89c4a8ef...","alias":"mcp-qa-test-wallet-2"}]}
```
But the same wallet, queried from **any subsequent process** (a new `det-cli core-wallets-list`
invocation, or a new `det-cli serve` session), is gone again — confirming the gap is specifically
"no hydration on cold/lazy AppContext init," not a bug in the in-memory registration path itself.

**Consequence for the acceptance criteria**: "generate addresses" and "check balances" from the
CLI are unreachable for any wallet that wasn't imported in the exact same long-lived process just
before the call — which is not how the CLI is documented or intended to be used (every example
in `docs/CLI.md` is a standalone command). "Send funds" (`core-funds-send`) would hit the
identical `Wallet not found` failure for the same reason (not separately re-verified with a live
send, to avoid spending real funds on a throwaway mnemonic with no balance — the wallet-lookup
failure is the blocking step regardless of what follows it).

### Positive control: the underlying SPV/address-derivation mechanism itself works

To confirm this isn't a deeper regression in address generation, a fresh mnemonic was imported
and `core-address-create` called for it **in the same `det-cli serve` session**, with the
`network` left as the data dir's default (Mainnet — chosen specifically because it is *not* the
network affected by this campaign's known Testnet wallet-backend blocker, see
`CAMPAIGN-CONTEXT.md`). Unlike the cross-process case, the call did **not** fail immediately with
`Wallet not found` — it correctly resolved the wallet and proceeded into a real SPV sync from
scratch, visible in `det.log`: Mainnet header sync 0% → 100% of ~2.5M headers within about a
minute, then filter-header/filter/block sync continuing steadily (headers 100%, filter headers
~28%, filters ~10%, block-relevance scan in progress at last check). The call did not complete
within the 280s bound given to this check — a full from-scratch Mainnet SPV sync legitimately
takes several minutes, and this is a throwaway data dir with no cached chain state — so no
address was actually returned in this pass, but the absence of an immediate lookup error and the
presence of genuine, progressing SPV work is enough to confirm the underlying SPV/backend wiring
is healthy on Mainnet in the standalone CLI path. The bug above is specifically the
list/lookup-vs-registration hydration gap, not a general standalone-mode outage.

### Shell completion

Not independently exercised interactively (this pass has no interactive shell to Tab through),
but `docs/CLI.md` documents auto-install to
`~/.local/share/bash-completion/completions/det-cli` on first run, and `det-cli completion bash`
is listed as a discovered command by `det-cli tools` — the completion script generator itself
was not run given the CLI-user-facing wallet-visibility bug already established a clear FAIL for
this story; revisit if the hydration bug above gets fixed.

**Verdict: FAIL.** Tool discovery, schema introspection, wallet *import*, and parameter
validation all work correctly, but the CLI cannot durably "manage" a wallet across the
process-per-command usage pattern its own docs describe: an imported wallet is invisible to
`core-wallets-list`, `core-address-create`, and `core-balances-get` (and by the same code path,
`core-funds-send`) in any invocation after the one that imported it, unless that later call
happens to land in the exact same long-lived process before any prior wallet was ever previously
imported to disk. This is a reproducible, source-confirmed bug in
`src/mcp/tools/wallet.rs`'s `ListWalletsTool`/lookup path missing a local (non-network)
hydration call — not a missing feature and not a network/environment issue.

## MCP-002: MCP server access for AI agents — **PASS (with the same caveat as MCP-001)**

Acceptance criteria: HTTP and stdio transports available; bearer token auth for HTTP mode;
network verification guard; tools expose wallet/identity/platform operations.

### Stdio transport (`det-cli serve`)

Drove a full JSON-RPC session over a paced stdin FIFO (`initialize` → `notifications/initialized`
→ `tools/call`), matching the MCP lifecycle `docs/MCP.md` describes for Claude Desktop/Code:

```
-> {"jsonrpc":"2.0","id":1,"method":"initialize", ...}
<- {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18", ...,
     "serverInfo":{"name":"dash-evo-tool","version":"1.0.0-dev"}, ...}}
-> {"jsonrpc":"2.0","method":"notifications/initialized"}
-> {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"core_wallet_import", ...}}
<- {"jsonrpc":"2.0","id":2,"result":{...,"already_imported":true}}
-> {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"core_wallets_list", ...}}
<- {"jsonrpc":"2.0","id":3,"result":{"structuredContent":{"wallets":[]}, ...}}
```
Protocol framing, request/response correlation by `id`, and error propagation
(`InvalidParam`/`TaskFailed`/`Internal` all surface as proper JSON-RPC error objects with codes)
all work correctly. The empty `wallets:[]` here is the same MCP-001 hydration bug, not a
transport defect — confirmed above that a wallet imported fresh **within** the same session lists
correctly.

### HTTP transport (`det-cli headless`, `mcp`+`cli` features)

Built with `--features headless` and launched against the isolated
`/data/tmp/det-qa-mcp-http-data` dir on a non-default port (`127.0.0.1:19527`) with a freshly
generated 48-hex-char `MCP_API_KEY`:

- `GET /health` (unauthenticated) → `200 OK` / body `OK`.
- `POST /mcp` with **no** `Authorization` header → `401 {"error":"unauthorized"}`.
- `POST /mcp` with a **wrong** bearer token → `401 {"error":"unauthorized"}`.
- `POST /mcp` with the correct token but missing the dual `Accept: application/json,
  text/event-stream` header → `406 Not Acceptable` (per the streamable-HTTP MCP spec; not a
  bug, just a stricter Accept-header requirement than a plain JSON POST).
- `POST /mcp` with correct token + headers, `initialize` → `200`, SSE-framed JSON-RPC response,
  `mcp-session-id` header returned.
- Follow-up `tools/call` for `network_info` and `core_wallets_list` using that session ID →
  both succeed (`200`, correct JSON-RPC results).
- `tools/list` over HTTP → 27 unique tool names (26 MCP tools + `tool_describe`), consistent
  with the stdio tool count.
- **Network verification guard**: `core_wallets_list` called with `"network":"testnet"` against
  a Mainnet-active context → `{"error":{"code":-32002,"message":"Network mismatch: expected
  testnet, got mainnet"}}`, `200` (JSON-RPC-level error, correct HTTP status) — confirmed working
  identically to the stdio path.

**Verdict: PASS.** Both transports (stdio via `det-cli serve`, HTTP via `det-cli headless`) are
present, functional, and correctly gated (bearer auth on HTTP, network-mismatch guard on both).
The one caveat carried over from MCP-001: an AI agent connecting fresh (no prior wallets
imported in that same session) will see an empty wallet list even if wallets exist in the data
dir, for the same hydration-gap reason — worth the product team's attention since it directly
undercuts "assist users with wallet queries," but it is a wallet-tooling defect, not a
transport/protocol/auth defect, so it does not sink the MCP-002 story on its own.

## Cross-cutting notes

- **Not the known Testnet blocker.** Both stories were tested primarily on Mainnet (the fresh
  throwaway data dirs default to Mainnet with no network configured), specifically to keep this
  pass independent of the campaign's known Testnet wallet-backend/masternode-list-sync issue
  (`CAMPAIGN-CONTEXT.md`). The one live SPV exercise done here (Mainnet header sync from scratch)
  completed normally, further reinforcing that the known blocker is Testnet-specific and not
  reproduced here.
- **`masternode_identity_load` / shielded tools**: not exercised end-to-end (would need real
  ProTxHash/keys or funded shielded-capable wallets, out of scope for MCP-001/002's "manage
  wallets" / "server access" acceptance criteria) — their schemas were confirmed valid via
  `tool-describe`, and their SPV-gating/private-key-handling behavior is already documented in
  `docs/MCP.md` and was not independently re-verified.
- No PR892 application source was modified. The wallet-hydration bug above was diagnosed by
  reading source (`src/mcp/tools/wallet.rs`,
  `src/context/wallet_lifecycle/registration.rs`) and cross-checking with direct SQLite
  inspection of the throwaway data dir — no fixes attempted, per campaign instructions.

## MCP-003: Load a masternode/evonode identity via CLI — BLOCKED (full happy path); plumbing tested clean

Acceptance criteria: identity fetched by ProTxHash over the network and persisted locally;
private keys accepted as WIF or hex, never echoed back, redacted in logs; output reports
which keys loaded, available withdrawal modes, and the registered payout address; `network`
required and must match the active network.

No real masternode/evonode fixture exists in this environment (confirmed absent in
`scenarios/IDN.md`/`scenarios/DEV.md`), so the full happy path (real ProTxHash → real fetch →
real key binding) cannot be exercised. Tested the CLI's plumbing and error-handling quality
instead, using a throwaway data dir (`/data/tmp/det-qa-mcp003-data`, deleted after the pass)
and a syntactically-plausible but fake ProTxHash (`f4bda60b...fb061`, 64 hex chars) and a
well-formed fake testnet WIF (`cN9spWsv...dwavaw`, generated locally, never a real key).

### Schema (`det-cli tool-describe name=masternode_identity_load`)

Confirms the tool exists with `pro_tx_hash`, `node_type`, `network` required; `owner_private_key`
/`voting_private_key`/`payout_private_key` typed as `Secret` (collapses to a bare
`{"type":"string"}` in the schema — no leaked length/format constraints that would hint at key
material, same pattern MCP-001/002 found for other `Secret` fields). Output schema reports
`owner_key_loaded`/`voting_key_loaded`/`payout_key_loaded` (booleans, never the key value),
`available_withdrawal_keys`, `payout_address`, and `dpns_names` — matches the acceptance
criteria's reporting requirements exactly.

### Live behavior

1. **Network mismatch rejected cleanly**: this fresh data dir defaults to Mainnet (no network
   ever configured). Calling with `network=testnet` was rejected immediately:
   `Network mismatch: expected testnet, got mainnet (code -32002)` — no dispatch attempted,
   no hang. Confirms the network-matching acceptance criterion.
2. **Missing `network` rejected cleanly**: omitting `network` entirely failed instantly with
   `failed to deserialize parameters: missing field 'network' (code -32602)` — confirms
   `network` is a required parameter, enforced before any dispatch.
3. **No keys at all rejected cleanly**: omitting all three private-key parameters failed
   instantly with a well-worded, role-based error: *"Provide at least one of the owner or
   payout private key. The owner key withdraws to the registered payout address; the payout
   key withdraws to any address."* (code -32602) — no key values involved, clean validation.
4. **Valid-network dispatch with fake ProTxHash/WIF**: called with `network=mainnet`
   (matching the data dir's active network) — the tool did **not** fail fast; it proceeded
   into a real Mainnet SPV sync from scratch (headers/filter-headers/filters/masternode-list
   progressing normally in the log, ~25% headers synced within 20s), matching MCP-001's
   established positive-control precedent exactly (`core-address-create` on a fresh Mainnet
   dir behaves identically). This confirms `masternode_identity_load` is SPV-gated per
   `docs/MCP.md`'s "SPV requirements" section (it is not listed among the SPV-gate-skipping
   tools) — the "hang" is expected chain-sync wait, not a bug. The run was capped at 20-60s
   (well short of a full from-scratch Mainnet sync) so no final "identity not found" result
   was observed, but the dispatch itself was clean: no crash, no panic, no stall without
   progress.
5. **No key leakage anywhere**: `grep`-checked all captured stdout/stderr from every run
   above for the fake WIF string (`cN9spWsv...dwavaw`) — zero matches in any run, including
   the ~20s of real SPV-sync logging in step 4. No `det.log` file was even created in the
   throwaway data dir (standalone CLI logs to stderr only, already checked). Confirms the
   redaction acceptance criterion held throughout every observed code path.
6. **Minor observation (not a story-blocking defect)**: a syntactically-garbage
   `owner-private-key` value (not a valid WIF or hex) was *not* rejected before the SPV wait —
   it took the same long-running dispatch path as the well-formed fake key, rather than
   failing fast on an obviously malformed key. Key-format validation appears to happen inside
   the task (after the SPV gate), not as an early parameter check. This is a UX-efficiency
   observation (a user with a typo'd key waits through a full sync before finding out), not a
   redaction or network-matching violation, so it does not change the verdict.

**Verdict: BLOCKED** for the full happy-path (no real masternode/evonode fixture available in
this environment — matches the established reasoning for IDN-003/DEV-006/MN-001-adjacent
stories). The testable plumbing — network-required, network-must-match, key-redaction, clean
parameter validation, and clean SPV-gated dispatch with no crash/hang-without-progress/key-leak
— all passed. No FAIL-triggering defect (key leakage or network-matching violation) was found.

## MCP-004: Withdraw masternode/evonode credits via CLI — BLOCKED (no loaded identity)

Acceptance criteria: owner-key mode forces the destination to the registered payout address
(rejecting a different address); payout/transfer-key mode allows withdrawal to any Core
address; withdrawal queues on Platform and settles after confirmation, reporting destination
and estimated/actual fees; `network` required and must match the active network.

**Verdict: BLOCKED.** Reasoning: no masternode/evonode identity loaded (MCP-003 prerequisite
BLOCKED — no fixture available). This tool operates on an already-loaded identity
(`identity_id` of a prior `masternode_identity_load` call), which cannot exist in this
environment.

### Supporting context (schema only, not a live test)

`det-cli tools` confirms `masternode-credits-withdraw` exists. Its full schema
(`det-cli tool-describe name=masternode_credits_withdraw`) directly mirrors every acceptance-
criteria bullet:

- `key_mode`: `"owner"` (destination forced to the payout address) or `"transfer"` (withdraw
  to any Core address) — matches bullets 1 and 2 verbatim.
- `to_address`: *"Required for 'transfer' mode; forbidden for 'owner' mode (the destination is
  the registered payout address)."* — confirms the owner-key/payout-address restriction is
  enforced at the parameter level, not just descriptively.
- `network`: required (*"required for destructive operations"*).
- Output schema: `to_address` (*"the Core address the funds were actually sent to"*),
  `estimated_fee`, `actual_fee` — matches bullet 3's "reports the destination used and the
  estimated and actual fees" exactly.
- Tool annotations: `destructiveHint: true` — correctly flagged as a fund-moving operation.

This confirms the tool is implemented with the correct shape and restrictions at the schema
level, but this is supporting context only — no live call was made (no identity to operate
on), so this does not upgrade the verdict beyond BLOCKED.
