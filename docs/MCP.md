# MCP Server

Dash Evo Tool exposes wallet and core operations via the [Model Context Protocol](https://modelcontextprotocol.io/). Two modes are available:

## HTTP mode (`mcp` feature)

Embedded in the GUI app. Shares the running app's `AppContext` and follows network switches in real time.

Activation: set `MCP_API_KEY` to a non-empty value before launching the app.

Routes:
- `GET /health` — unauthenticated liveness check, returns `OK`
- `POST /mcp` — MCP protocol endpoint, requires `Authorization: Bearer <key>`

Build: `cargo build --features mcp`

## Stdio mode (via `det-cli serve`)

The `det-cli` binary includes a built-in MCP stdio server. No separate binary needed.

```bash
det-cli serve
```

Communicates via stdin/stdout using the MCP protocol. `AppContext` is initialized lazily on the first tool call, reading the same `.env` and database as the GUI app. Uses the last network selected in the GUI by default. Use the `network_info` tool to check the active network.

Build: `cargo build --features cli`

See [CLI.md](CLI.md) for full `det-cli` documentation.

## Headless mode (`headless` feature)

Runs an HTTP MCP server without a GUI. Useful for server environments or scripts that need the HTTP transport but cannot run the full desktop application.

```bash
det-cli headless
```

`MCP_API_KEY` must be set — headless mode without authentication is not permitted.

Build: `cargo build --features headless`

> **Note:** Headless mode requires building from source with `--features headless`. Pre-built release binaries do not include this feature.

The `headless` feature combines `cli` and `mcp`. The same environment variables (`MCP_API_KEY`, `MCP_LISTEN`) apply.

### Performance considerations

Standalone/headless mode skips the GUI event loop and egui rendering, resulting in lower memory usage and CPU overhead. Prefer headless mode for:

- Server or CI environments where no display is available
- Automated scripts that call tools repeatedly
- Long-running MCP server deployments

The GUI-embedded HTTP mode shares the app's `AppContext` and follows network switches in real time, which is useful when you want to interactively switch networks from the GUI while MCP clients remain connected.

## Protected wallets and shielded operations

Every shielded operation that spends or binds the wallet's Orchard keys
(initialization, shield from Core, shield from Platform, transfer, unshield, and
withdraw) resolves the wallet seed just in time. A password-protected (Tier-2)
wallet therefore requires an interactive passphrase session.

Standalone stdio MCP and headless HTTP MCP use a null secret prompt because
they have no authorized GUI session. These transports return
`SecretPromptUnavailable` when a protected wallet needs to authorize a
shielded operation. There is currently no environment-variable or
CLI-passphrase workaround, so unattended shielded operations from a protected
wallet are not supported. Use an unprotected wallet for that automation.

MCP embedded in a running GUI is not affected in the same way: it can use the
GUI's existing interactive prompt and authorized secret session. This caveat
applies only to standalone and headless transports.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `MCP_API_KEY` | _(empty — disabled)_ | Enables HTTP server; used as Bearer token secret |
| `MCP_LISTEN` | `127.0.0.1:9527` | HTTP listen address |

Set these in the app's `.env` file (see `.env.example`) or as environment variables before launch.

## Available tools

| Tool | Parameters | det-cli command | Description |
|---|---|---|---|
| `network_info` | — | `det-cli network-info` | Show active network and available configured networks |
| `network_reinit_sdk` | `network` | `det-cli network-reinit-sdk` | Rebuild Core RPC client and Platform SDK with current config (use after changing credentials) |
| `network_switch` | `network` | `det-cli network-switch` | Switch the active network (creates context if needed, may take a few seconds) |
| `core_wallets_list` | `network`? | `det-cli core-wallets-list` | List wallets saved for the active network (alias + seed hash) |
| `core_wallet_import` | `mnemonic`, `network`, `alias`? | `det-cli core-wallet-import` | Import a wallet from a BIP-39 recovery phrase (unprotected); returns its seed hash. Idempotent |
| `core_address_create` | `wallet_id`, `network`? | `det-cli core-address-create` | Generate a new receive address for a wallet |
| `core_balances_get` | `wallet_id`, `network`? | `det-cli core-balances-get` | Show wallet balances (total, confirmed, unconfirmed) in duffs |
| `platform_addresses_list` | `wallet_id`, `network`? | `det-cli platform-addresses-list` | Fetch platform address balances (credits and nonces) |
| `core_funds_send` | `wallet_id`, `address`, `amount_duffs`, `network` | `det-cli core-funds-send` | Send DASH from a wallet to an address (amount in duffs) |
| `platform_withdrawals_get` | `status`?, `limit`?, `start_after`?, `network`? | `det-cli platform-withdrawals-get` | Query Platform withdrawal documents (`"queued"` or `"completed"`); returns structured entries with a `next_cursor` for pagination |
| `identity_credits_topup` | `wallet_id`, `identity_id`, `amount_duffs`, `network` | `det-cli identity-credits-topup` | Top up an identity with DASH from wallet (via asset lock) |
| `identity_credits_topup_from_platform` | `wallet_id`, `identity_id`, `amount_credits`, `network` | `det-cli identity-credits-topup-from-platform` | Top up an identity from Platform address balances |
| `identity_credits_transfer` | `wallet_id`, `from_identity_id`, `to_identity_id`, `amount_credits`, `network` | `det-cli identity-credits-transfer` | Transfer credits between identities |
| `identity_credits_withdraw` | `wallet_id`, `identity_id`, `to_address`, `amount_credits`, `network` | `det-cli identity-credits-withdraw` | Withdraw identity credits to a Core address |
| `identity_credits_to_address` | `wallet_id`, `identity_id`, `to_address`, `amount_credits`, `network` | `det-cli identity-credits-to-address` | Transfer identity credits to a Platform address |
| `masternode_identity_load` | `pro_tx_hash`, `node_type`, `owner_private_key`?, `voting_private_key`?, `payout_private_key`?, `alias`?, `network` | `det-cli masternode-identity-load` | Load a masternode/evonode identity by ProTxHash and bind its owner/voting/payout keys. Returns which keys loaded, the available withdrawal modes, and the payout address. Requires at least one of the owner or payout key |
| `masternode_credits_withdraw` | `identity_id`, `key_mode`, `to_address`?, `amount_credits`, `network` | `det-cli masternode-credits-withdraw` | Withdraw a masternode/evonode identity's credits. `key_mode=owner` forces the registered payout address (no `to_address`); `key_mode=transfer` withdraws to any Core address. Platform addresses (bech32m) are rejected for both modes |
| `shielded_shield_from_core` | `wallet_id`, `amount_duffs`, `network` | `det-cli shielded-shield-from-core` | Shield DASH from Core wallet into shielded pool (via asset lock, ~30s) |
| `shielded_shield_from_platform` | `wallet_id`, `amount_credits`, `network` | `det-cli shielded-shield-from-platform` | Shield credits from Platform address into shielded pool |
| `shielded_transfer` | `wallet_id`, `to_address`, `amount_credits`, `network` | `det-cli shielded-transfer` | Private shielded-to-shielded transfer |
| `shielded_unshield` | `wallet_id`, `to_address`, `amount_credits`, `network` | `det-cli shielded-unshield` | Unshield credits to a Platform address |
| `shielded_withdraw` | `wallet_id`, `to_address`, `amount_credits`, `network` | `det-cli shielded-withdraw` | Withdraw from shielded pool to a Core address |
| `shielded_init` | `wallet_id`, `network`? | `det-cli shielded-init` | Bind a wallet's shielded keys and warm the proving key (~30s); idempotent. Run once before shielded ops |
| `shielded_sync` | `wallet_id`, `network`? | `det-cli shielded-sync` | Force a shielded sync and return the post-sync balance (credits + duffs); use to verify a balance change |
| `shielded_balance_get` | `wallet_id`, `network`? | `det-cli shielded-balance-get` | Read the shielded balance from the last synced snapshot (credits + duffs); no sync |
| `shielded_address_get` | `wallet_id`, `network`? | `det-cli shielded-address-get` | Return the wallet's default shielded (Orchard) receive address (bech32m) |
| `tool_describe` | `name` | `det-cli tool-describe` | Return the full MCP tool definition for a given tool name |

Parameters marked `?` are optional. The `det-cli` column shows the equivalent CLI command (underscores become hyphens).

### SPV requirements

Wallet-facing tools that need chain or proof state wait for SPV to fully sync before executing. This includes both core-chain tools (`core_address_create`, `core_balances_get`, `core_funds_send`) and proof-verifying platform tools (`platform_addresses_list`, `identity_credits_topup`, `shielded_shield_from_core`). These Platform operations need SPV because the SDK verifies DAPI proofs against quorum and masternode list data from the synced chain. When another DET instance is already running, SPV falls back to a temporary directory and must sync from scratch.

Tools that make no network calls skip the SPV gate: the metadata tools (`core_wallets_list`, `network_info`, `tool_describe`), the local wallet import (`core_wallet_import`), and the shielded snapshot read `shielded_balance_get` (a pure in-memory read of the last synced balance). Wallet-reading tools hydrate saved wallets from local storage without starting SPV. `shielded_address_get` also skips the SPV gate and wires the wallet backend automatically, but `shielded_init` is still required before it can return an address for a wallet whose shielded keys have not been bound. `shielded_init` and `shielded_sync` still wait for SPV and drive a coordinator sync.

`masternode_credits_withdraw` waits for SPV before dispatching: a withdrawal does proof-verified Platform reads, so it gates like every other proof-verifying tool. (`identity_credits_withdraw` historically skipped this gate; the masternode tool deliberately adds it.)

### Private-key handling

`masternode_identity_load` accepts the masternode owner/voting/payout private keys as JSON string values (WIF or 64-char hex). They are typed as `Secret` in the parameter struct. At deserialization, `Secret::new()` copies the content into a zeroizing, best-effort page-locked buffer, then zeroes and frees the transient serde `String` before returning — so no plain copy of the key persists beyond the deserialization call. The keys are never echoed back: the tool output reports only which keys loaded (booleans), validation errors name the key by role and never its value, and the parameter struct's `Debug` renders each key as `Secret(***)` so it cannot leak into logs or the MCP error `data` payload.

Over the MCP **HTTP** transport these keys traverse the request body. The HTTP endpoint is bearer-authenticated and binds to loopback by default; do not send live mainnet masternode keys over a non-loopback MCP HTTP endpoint. (HTTP transport-layer key handling is not separately enforced in code — keep the endpoint loopback-only for key-bearing calls.)

## CLI interface (det-cli)

`det-cli` is the command-line interface for interacting with MCP tools. It can operate in two modes:

- **Direct tool invocation**: Run tools as CLI commands, e.g. `det-cli core-wallets-list --network testnet`
- **MCP stdio server**: `det-cli serve` starts an MCP server communicating over stdin/stdout

Basic usage:

```bash
# List available commands
det-cli --help

# List wallets
det-cli core-wallets-list

# Check balances for a specific wallet
det-cli core-balances-get --wallet-id "my-wallet"

# Start as MCP stdio server (for Claude Desktop, Claude Code, etc.)
det-cli serve

# Start headless HTTP MCP server (requires MCP_API_KEY)
det-cli headless
```

Tool names use hyphens in CLI commands (e.g. `core_wallets_list` becomes `core-wallets-list`). The CLI dynamically discovers available tools from the MCP server, so new tools are automatically available without CLI changes.

See [CLI.md](CLI.md) for full documentation.

## Network verification

Tools accept a `network` parameter (e.g. `"mainnet"`, `"testnet"`, `"devnet"`, `"local"`). When provided, the request fails immediately if it does not match the server's active network. This prevents accidentally operating on the wrong network.

For **destructive tools** (those that spend funds or modify state — the fund-moving identity tools `identity_credits_topup`, `identity_credits_topup_from_platform`, `identity_credits_transfer`, `identity_credits_withdraw`, `identity_credits_to_address`, `masternode_credits_withdraw`, plus `core_funds_send`, `core_wallet_import`, and the fund-moving shielded tools `shielded_shield_from_core`, `shielded_shield_from_platform`, `shielded_transfer`, `shielded_unshield`, `shielded_withdraw`), `network` is **required**. The tool will reject the request if `network` is omitted or does not match the active network. This is a safety measure to prevent accidentally spending funds on the wrong network. Note: `masternode_identity_load` is non-destructive (it does not move funds) but also requires `network`, because the private keys it binds are chain-scoped.

For **read-only tools** (e.g. `core_wallets_list`, `core_balances_get`) and the shielded control/read tools (`shielded_init`, `shielded_sync`, `shielded_balance_get`, `shielded_address_get`), `network` is optional. When omitted, the tool operates on whatever network is currently active.

The `network_info` and `tool_describe` tools do not perform this check.

Example (HTTP):

```bash
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"core_wallets_list","arguments":{"network":"testnet"}}}'
```

## Quick examples

### Claude Desktop (stdio)

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "dash-evo-tool": {
      "command": "det-cli",
      "args": ["serve"]
    }
  }
}
```

`det-cli` must be on `PATH` (or use the full path to the binary).

### Claude Code (stdio)

Add to `.mcp.json` in your project root, or to `~/.claude.json` for user-level configuration:

```json
{
  "mcpServers": {
    "DET": {
      "type": "stdio",
      "command": "det-cli",
      "args": ["serve"]
    }
  }
}
```

### HTTP with curl

```bash
# Check server is running
curl http://127.0.0.1:9527/health

# List wallets
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"core_wallets_list","arguments":{}}}'

# Generate a receive address
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"core_address_create","arguments":{"wallet_id":"my-wallet"}}}'

# Query queued withdrawals
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"platform_withdrawals_get","arguments":{"status":"queued"}}}'
```

## Authentication

The HTTP MCP server uses bearer token authentication via the `MCP_API_KEY` environment variable.

**Setting the key**: Add `MCP_API_KEY=<your-secret>` to the app's `.env` file or export it as an environment variable before launching. The key must be at least 16 characters long.

**Behavior**:
- **Key not set or empty**: HTTP MCP server is disabled entirely. The log message will indicate "MCP_API_KEY not set".
- **Key too short** (< 16 chars): HTTP MCP server refuses to start. An error is logged indicating the minimum length requirement.
- **Key valid**: HTTP MCP server starts. All requests to `/mcp` must include `Authorization: Bearer <key>`.
- **Stdio mode** (`det-cli serve`): No authentication — security is handled by the process boundary (only the calling process can communicate via stdin/stdout).

**Security considerations**:
- Generate a strong random key (e.g. `openssl rand -hex 32`).
- Do not commit `.env` files containing the key to version control.
- Bearer token comparison uses constant-time equality to prevent timing attacks.
- The HTTP server binds to `127.0.0.1` by default; change `MCP_LISTEN` only if you understand the exposure implications.

## Security

- The HTTP server binds to `127.0.0.1` by default; it is not reachable from the network unless `MCP_LISTEN` is changed.
- Bearer token comparison uses constant-time equality to prevent timing attacks.
- The HTTP server is disabled when `MCP_API_KEY` is empty or unset.
- Stdio mode has no authentication — the caller controls which process connects.
- Headless mode requires `MCP_API_KEY`; it will refuse to start without it.
