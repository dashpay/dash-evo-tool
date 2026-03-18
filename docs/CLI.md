# det-cli

`det-cli` is the command-line client for Dash Evo Tool's MCP server. Use it to call wallet and core operations from scripts and terminals without opening the GUI.

## Build

```bash
cargo build --features cli
```

The binary is `det-cli` in the Cargo output directory.

## Configuration

`det-cli` reads the app's `.env` file automatically on startup — the same file used by the GUI. No separate configuration is needed.

Config precedence (highest to lowest):

1. CLI flags (`--bearer`, `--addr`)
2. Shell environment variables (`MCP_API_KEY`, `MCP_LISTEN`)
3. App's `.env` file (loaded from the platform data directory)

## Connection modes

### HTTP

Used automatically when `MCP_API_KEY` is available (from `.env` or shell). Connects to the running GUI app's MCP HTTP server.

```bash
det-cli tools
```

The server address defaults to `http://{MCP_LISTEN}/mcp`, or `http://127.0.0.1:9527/mcp` if `MCP_LISTEN` is not set. Override with `--addr`:

```bash
det-cli --addr http://127.0.0.1:9000/mcp tools
```

### Stdio (default when no API key)

When no `MCP_API_KEY` is configured, `det-cli` spawns `dash-evo-tool-mcp` as a child process and communicates via stdin/stdout. No running GUI app required.

`dash-evo-tool-mcp` must be built (`cargo build --features mcp-stdio`) and on `PATH`.

Force stdio mode explicitly with `--standalone`:

```bash
det-cli --standalone tools
```

## Usage

Running `det-cli` with no subcommand lists available tools (same as `det-cli tools`).

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
# No parameters
det-cli call list_wallets

# String parameter
det-cli call generate_receive_address wallet_id=my-wallet

# JSON parameter (boolean)
det-cli call some_tool enabled=true
```

Exit code is non-zero when the tool returns an error.

### `completion <shell>` — generate shell completion

Generates a completion script for the given shell. Supported: `bash`, `zsh`, `fish`, `powershell`.

```bash
det-cli completion bash
det-cli completion zsh
```

## Tool cache

The tool list is cached automatically when you run `det-cli` or `det-cli tools`. The cache includes the server version and is invalidated when the version changes.

`det-cli --help` appends the cached tool list to the standard help output. If the cache is empty or stale, run `det-cli` once to populate it.

## Shell completion setup

### Bash

```bash
# Add completion to your shell session
source <(det-cli completion bash)

# Or add permanently to ~/.bashrc
echo 'source <(det-cli completion bash)' >> ~/.bashrc
```

The bash completion script includes a dynamic completer for tool names in `det-cli call`. It reads from the local cache. Requires `jq` for dynamic tool name completion.

### Zsh

```bash
det-cli completion zsh > "${fpath[1]}/_det-cli"
```

## Examples

```bash
# List wallets loaded in the app (reads MCP_API_KEY from .env)
det-cli call list_wallets

# Generate a receive address (use alias or hex seed hash)
det-cli call generate_receive_address wallet_id=savings

# List wallets in Dash Core
det-cli call list_core_wallets

# Stdio mode (no GUI required)
det-cli --standalone call list_wallets
MCP_NETWORK=testnet det-cli --standalone call list_wallets
```
