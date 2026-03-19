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

The `headless` feature combines `cli` and `mcp`. The same environment variables (`MCP_API_KEY`, `MCP_LISTEN`) apply.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `MCP_API_KEY` | _(empty — disabled)_ | Enables HTTP server; used as Bearer token secret |
| `MCP_LISTEN` | `127.0.0.1:9527` | HTTP listen address |

Set these in the app's `.env` file (see `.env.example`) or as environment variables before launch.

## Available tools

| Tool | Parameters | Description |
|---|---|---|
| `network_info` | — | Show active network and available configured networks |
| `core_wallets_list` | `network`? | List wallets loaded in the app (alias + seed hash) |
| `core_address_create` | `wallet_id`, `network`? | Generate a new receive address for a wallet. Pass the alias or 64-char hex seed hash. |
| `core_balances_get` | `wallet_id`, `network`? | Show wallet balances (total, confirmed, unconfirmed) in duffs |
| `platform_addresses_list` | `wallet_id`, `network`? | Fetch platform address balances (credits and nonces) for a wallet |
| `core_funds_send` | `wallet_id`, `address`, `amount_duffs`, `network`? | Send DASH from a wallet to an address (amount in duffs) |
| `platform_withdrawals_get` | `status`?, `network`? | Query Platform withdrawal documents. `status` is `"queued"` (default) or `"completed"`. |
| `tool_describe` | `name` | Return the full MCP tool definition (schema, annotations, description) for a given tool name |

Parameters marked `?` are optional.

## Network verification

Most tools accept an optional `network` parameter (e.g. `"mainnet"`, `"testnet"`, `"devnet"`, `"local"`). When provided, the request fails immediately if it does not match the server's active network. This prevents accidentally operating on the wrong network.

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
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"core_wallets_list","arguments":{}}}'

# Generate a receive address
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"core_address_create","arguments":{"wallet_id":"my-wallet"}}}'

# Query queued withdrawals
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"platform_withdrawals_get","arguments":{"status":"queued"}}}'
```

## Security

- The HTTP server binds to `127.0.0.1` by default; it is not reachable from the network unless `MCP_LISTEN` is changed.
- Bearer token comparison uses constant-time equality to prevent timing attacks.
- The HTTP server is disabled when `MCP_API_KEY` is empty or unset.
- Stdio mode has no authentication — the caller controls which process connects.
- Headless mode requires `MCP_API_KEY`; it will refuse to start without it.
