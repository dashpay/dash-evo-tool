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
| `network_refresh_endpoints` | `network` | `det-cli network-refresh-endpoints` | Fetch fresh DAPI node addresses from DCG discovery service, save to config, and reinit SDK |
| `network_reinit_sdk` | `network` | `det-cli network-reinit-sdk` | Rebuild Core RPC client and Platform SDK with current config (use after changing credentials) |
| `network_switch` | `network` | `det-cli network-switch` | Switch the active network (creates context if needed, may take a few seconds) |
| `core_wallets_list` | `network`? | `det-cli core-wallets-list` | List wallets loaded in the app (alias + seed hash) |
| `core_address_create` | `wallet_id`, `network`? | `det-cli core-address-create` | Generate a new receive address for a wallet |
| `core_balances_get` | `wallet_id`, `network`? | `det-cli core-balances-get` | Show wallet balances (total, confirmed, unconfirmed) in duffs |
| `platform_addresses_list` | `wallet_id`, `network`? | `det-cli platform-addresses-list` | Fetch platform address balances (credits and nonces) |
| `core_funds_send` | `wallet_id`, `address`, `amount_duffs`, `network` | `det-cli core-funds-send` | Send DASH from a wallet to an address (amount in duffs) |
| `platform_withdrawals_get` | `status`?, `network`? | `det-cli platform-withdrawals-get` | Query Platform withdrawal documents (`"queued"` or `"completed"`) |
| `identity_credits_topup` | `wallet_id`, `identity_id`, `amount_duffs`, `network` | `det-cli identity-credits-topup` | Top up an identity with DASH from wallet (via asset lock) |
| `identity_credits_topup_from_platform` | `wallet_id`, `identity_id`, `amount_credits`, `network` | `det-cli identity-credits-topup-from-platform` | Top up an identity from Platform address balances |
| `identity_credits_transfer` | `wallet_id`, `from_identity_id`, `to_identity_id`, `amount_credits`, `network` | `det-cli identity-credits-transfer` | Transfer credits between identities |
| `identity_credits_withdraw` | `wallet_id`, `identity_id`, `to_address`, `amount_credits`, `network` | `det-cli identity-credits-withdraw` | Withdraw identity credits to a Core address |
| `identity_credits_to_address` | `wallet_id`, `identity_id`, `to_address`, `amount_credits`, `network` | `det-cli identity-credits-to-address` | Transfer identity credits to a Platform address |
| `shielded_shield_from_core` | `wallet_id`, `amount_duffs`, `network` | `det-cli shielded-shield-from-core` | Shield DASH from Core wallet into shielded pool (via asset lock, ~30s) |
| `shielded_shield_from_platform` | `wallet_id`, `amount_credits`, `network` | `det-cli shielded-shield-from-platform` | Shield credits from Platform address into shielded pool |
| `shielded_transfer` | `wallet_id`, `to_address`, `amount_credits`, `network` | `det-cli shielded-transfer` | Private shielded-to-shielded transfer |
| `shielded_unshield` | `wallet_id`, `to_address`, `amount_credits`, `network` | `det-cli shielded-unshield` | Unshield credits to a Platform address |
| `shielded_withdraw` | `wallet_id`, `to_address`, `amount_credits`, `network` | `det-cli shielded-withdraw` | Withdraw from shielded pool to a Core address |
| `tool_describe` | `name` | `det-cli tool-describe` | Return the full MCP tool definition for a given tool name |

Parameters marked `?` are optional. The `det-cli` column shows the equivalent CLI command (underscores become hyphens).

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

For **destructive tools** (those that spend funds or modify state — all identity and shielded tools), `network` is **required**. The tool will reject the request if `network` is omitted or does not match the active network. This is a safety measure to prevent accidentally spending funds on the wrong network.

For **read-only tools** (e.g. `core_wallets_list`, `core_balances_get`), `network` is optional. When omitted, the tool operates on whatever network is currently active.

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
