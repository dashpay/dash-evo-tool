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
