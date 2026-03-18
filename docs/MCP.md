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

Communicates via stdin/stdout using the MCP protocol. `AppContext` is initialized lazily on the first tool call, reading the same `.env` and database as the GUI app. Uses the last network selected in the GUI by default. Use the `network` tool to check or change the active network.

Build: `cargo build --features cli`

See [CLI.md](CLI.md) for full `det-cli` documentation.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `MCP_API_KEY` | _(empty — disabled)_ | Enables HTTP server; used as Bearer token secret |
| `MCP_LISTEN` | `127.0.0.1:9527` | HTTP listen address |

Set these in the app's `.env` file (see `.env.example`) or as environment variables before launch.

## Available tools

| Tool | Parameters | Description |
|---|---|---|
| `network` | — | Show active network and available configured networks |
| `list_wallets` | — | List wallets loaded in the app (alias + seed hash) |
| `generate_receive_address` | `wallet_id` | Generate a new receive address for a wallet. Pass the alias or 64-char hex seed hash. |

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
