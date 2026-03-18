# MCP Server

Dash Evo Tool exposes wallet and core operations via the [Model Context Protocol](https://modelcontextprotocol.io/). Two transport modes are available, each behind its own Cargo feature flag.

## Transport modes

### HTTP (`mcp-http`)

Embedded in the GUI app. Shares the running app's `AppContext` and follows network switches in real time.

Activation: set `MCP_API_KEY` to a non-empty value before launching the app.

Routes:
- `GET /health` — unauthenticated liveness check, returns `OK`
- `POST /mcp` — MCP protocol endpoint, requires `Authorization: Bearer <key>`

### Stdio (`mcp-stdio`)

Standalone binary `dash-evo-tool-mcp`. No GUI. Communicates via stdin/stdout using the MCP protocol. `AppContext` is initialized lazily on the first tool call, reading the same `.env` and database as the GUI app.

## Building

```bash
# HTTP transport (GUI app)
cargo build --features mcp-http

# Stdio transport (standalone binary)
cargo build --features mcp-stdio

# CLI client (see docs/CLI.md)
cargo build --features cli
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `MCP_API_KEY` | _(empty — disabled)_ | Enables HTTP server; used as Bearer token secret |
| `MCP_LISTEN` | `127.0.0.1:9527` | HTTP listen address |
| `MCP_NETWORK` | `mainnet` | Network for stdio mode: `mainnet`, `testnet`, `devnet`, `regtest` |

Set these in the app's `.env` file (see `.env.example`) or as environment variables before launch.

## Available tools

| Tool | Parameters | Description |
|---|---|---|
| `list_wallets` | — | List wallets loaded in the app (alias + seed hash) |
| `generate_receive_address` | `wallet_id` | Generate a new receive address for a wallet. Pass the alias or 64-char hex seed hash. |
| `list_core_wallets` | — | List wallets loaded in Dash Core |

## Quick examples

### Claude Desktop (stdio)

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "dash-evo-tool": {
      "command": "dash-evo-tool-mcp",
      "env": {
        "MCP_NETWORK": "mainnet"
      }
    }
  }
}
```

`dash-evo-tool-mcp` must be on `PATH` (or use the full path to the binary).

### HTTP with curl

```bash
# Check server is running
curl http://127.0.0.1:9527/health

# List wallets
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_wallets","arguments":{}}}'

# Generate a receive address
curl -s http://127.0.0.1:9527/mcp \
  -H "Authorization: Bearer $MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"generate_receive_address","arguments":{"wallet_id":"my-wallet"}}}'
```

## Security

- The HTTP server binds to `127.0.0.1` by default; it is not reachable from the network unless `MCP_LISTEN` is changed.
- Bearer token comparison uses constant-time equality to prevent timing attacks.
- The HTTP server is disabled when `MCP_API_KEY` is empty or unset.
- Stdio mode has no authentication — the caller controls which process connects.
