# det-cli

`det-cli` is the command-line interface for Dash Evo Tool. Run wallet and platform commands from the terminal — no GUI needed.

## Build

```bash
cargo build --features cli
```

## Quick start

```bash
# List available commands
det-cli

# List wallets
det-cli list-wallets

# Generate a receive address
det-cli generate-receive-address wallet-id=savings

# List wallets in Dash Core
det-cli list-core-wallets
```

Run `det-cli --help` to see all available commands with descriptions.

## Configuration

`det-cli` reads the app's `.env` file automatically — the same file used by the GUI. No separate configuration needed.

Config precedence (highest to lowest):

1. CLI flags (`--addr`)
2. Shell environment variables (`MCP_API_KEY`, `MCP_LISTEN`)
3. App's `.env` file (loaded from the platform data directory)

## Connection modes

### Standalone (default)

When no `MCP_API_KEY` is set, `det-cli` runs its own backend in-process. No running GUI app or server required.

### Connected to Dash Evo Tool GUI

Set `MCP_API_KEY` (in `.env` or shell) to connect to a running Dash Evo Tool instance instead. This shares the app's live state — wallets, network, database.

The GUI address defaults to `http://127.0.0.1:9527/mcp`. Override with `--addr`:

```bash
det-cli --addr http://127.0.0.1:9000/mcp list-wallets
```

Force standalone mode with `--standalone` even when an API key is present.

## Usage

Commands use hyphens (`list-wallets`, not `list_wallets`). Parameters are passed as `key=value` pairs:

```bash
det-cli <command> [key=value ...]
```

Values are parsed as JSON first, falling back to plain strings. Exit code is non-zero on error.

### `serve` — MCP stdio server

```bash
det-cli serve
```

Runs an MCP server over stdin/stdout for Claude Desktop, AI agents, or other MCP clients. See [MCP.md](MCP.md) for Claude Desktop configuration.

### `tools` — refresh command list

```bash
det-cli tools
```

Fetches and displays all available commands. The list is cached automatically.

### `completion <shell>` — shell completion

```bash
det-cli completion bash
det-cli completion zsh
```

## Shell completion

Bash completion is **installed automatically** on first run to `~/.local/share/bash-completion/completions/det-cli`. It works on the next shell session — no manual setup needed. Requires `jq` for dynamic command name completion.

For zsh:

```bash
det-cli completion zsh > "${fpath[1]}/_det-cli"
```

## Examples

```bash
# List wallets (standalone, no server needed)
det-cli list-wallets

# Generate a receive address
det-cli generate-receive-address wallet-id=savings

# List wallets in Dash Core
det-cli list-core-wallets

# Use testnet
MCP_NETWORK=testnet det-cli list-wallets

# Run as stdio MCP server for Claude Desktop
det-cli serve
```
