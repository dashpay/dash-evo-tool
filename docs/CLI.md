# det-cli

`det-cli` is the command-line interface for Dash Evo Tool. It provides both a CLI client for calling MCP tools and a built-in MCP stdio server.

## Build

```bash
cargo build --features cli
```

## Configuration

`det-cli` reads the app's `.env` file automatically on startup — the same file used by the GUI. No separate configuration is needed.

Config precedence (highest to lowest):

1. CLI flags (`--bearer`, `--addr`)
2. Shell environment variables (`MCP_API_KEY`, `MCP_LISTEN`)
3. App's `.env` file (loaded from the platform data directory)

## Connection modes

### In-process (default)

When no `MCP_API_KEY` is configured, `det-cli` runs the MCP service in-process. No running GUI app or separate server required.

```bash
det-cli tools
```

### HTTP

Used automatically when `MCP_API_KEY` is available (from `.env` or shell). Connects to the running GUI app's MCP HTTP server.

```bash
det-cli tools
```

The server address defaults to `http://{MCP_LISTEN}/mcp`, or `http://127.0.0.1:9527/mcp` if `MCP_LISTEN` is not set. Override with `--addr`:

```bash
det-cli --addr http://127.0.0.1:9000/mcp tools
```

Force in-process mode with `--standalone` even when an API key is present.

## Usage

Running `det-cli` with no subcommand lists available tools.

### `tools` — list available tools

```bash
det-cli tools
```

Prints each tool name, its description, and its parameters.

### `call <tool> [key=value ...]` — call a tool

```bash
det-cli call <tool-name> [key=value ...]
```

Parameter values are parsed as JSON first, falling back to plain strings.

```bash
det-cli call list_wallets
det-cli call generate_receive_address wallet_id=my-wallet
det-cli call some_tool enabled=true
```

Exit code is non-zero when the tool returns an error.

### `serve` — run as MCP stdio server

```bash
det-cli serve
```

Runs an MCP server over stdin/stdout for use with Claude Desktop, AI agents, or other MCP clients. See [MCP.md](MCP.md) for Claude Desktop configuration.

### `completion <shell>` — generate shell completion

```bash
det-cli completion bash
det-cli completion zsh
```

## Tool cache

The tool list is cached automatically when you run `det-cli` or `det-cli tools`. The cache includes the server version and is invalidated when the version changes.

`det-cli --help` appends the cached tool list to the standard help output.

## Shell completion setup

### Bash

```bash
source <(det-cli completion bash)

# Or add permanently to ~/.bashrc
echo 'source <(det-cli completion bash)' >> ~/.bashrc
```

The bash completion includes dynamic tool name completion for `det-cli call`. Requires `jq`.

### Zsh

```bash
det-cli completion zsh > "${fpath[1]}/_det-cli"
```

## Examples

```bash
# List wallets (in-process, no server needed)
det-cli call list_wallets

# Generate a receive address
det-cli call generate_receive_address wallet_id=savings

# List wallets in Dash Core
det-cli call list_core_wallets

# Use testnet
MCP_NETWORK=testnet det-cli call list_wallets

# Run as stdio MCP server for Claude Desktop
det-cli serve
```
