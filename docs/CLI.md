# det-cli

`det-cli` is the command-line client for Dash Evo Tool's MCP server. Use it to call wallet and core operations from scripts and terminals without opening the GUI.

## Build

```bash
cargo build --features cli
```

The binary is `det-cli` in the Cargo output directory.

## Configuration

`det-cli` automatically reads the app's `.env` file on startup (the same file used by the GUI). This means no separate setup is needed — if you have already configured `MCP_API_KEY` in the app's `.env`, `det-cli` picks it up automatically.

Shell environment variables take precedence over `.env` values, and CLI flags override everything.

**Config precedence (highest to lowest):**

1. CLI flags (`--bearer`, `--addr`)
2. Shell environment variables (`MCP_API_KEY`, `MCP_LISTEN`)
3. App's `.env` file

## Connection modes

### HTTP (when MCP_API_KEY is configured)

Connects to a running Dash Evo Tool instance's MCP HTTP server. HTTP mode is used automatically when `MCP_API_KEY` is set — either via the app's `.env`, a shell env var, or `--bearer`.

The `.env` file is read automatically, so this just works:

```bash
# MCP_API_KEY is already set in ~/.config/Dash-Evo-Tool/.env
det-cli tools
```

To override the API key for a single invocation:

```bash
det-cli --bearer my-secret-key tools
```

The server address defaults to `http://{MCP_LISTEN}/mcp` (from `.env` or env var), falling back to `http://127.0.0.1:9527/mcp`. Override with `--addr`:

```bash
det-cli --addr http://127.0.0.1:9000/mcp tools
```

### Stdio (default when MCP_API_KEY is not set)

When no API key is configured, `det-cli` automatically spawns `dash-evo-tool-mcp` as a child process and connects via stdin/stdout. No running GUI app needed. The `--standalone` flag forces this mode explicitly.

`dash-evo-tool-mcp` must be built (`cargo build --features mcp-stdio`) and on `PATH`.

```bash
# No MCP_API_KEY configured → stdio mode used automatically
det-cli tools

# Force stdio mode explicitly
det-cli --standalone tools
```

## Subcommands

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

### `cache` — cache tool schemas

Downloads the tool list from the server and saves it locally. Required before shell completion works for tool names.

```bash
det-cli cache
```

### `completion <shell>` — generate shell completion

Generates a completion script for the given shell. Supported: `bash`, `zsh`, `fish`, `powershell`.

```bash
det-cli completion bash
det-cli completion zsh
```

## Shell completion setup

### Bash

```bash
# Cache tool names (re-run when tools change)
det-cli cache

# Add completion to your shell session
source <(det-cli completion bash)

# Or add permanently to ~/.bashrc
echo 'source <(det-cli completion bash)' >> ~/.bashrc
```

The bash completion script includes a dynamic completer for tool names in `det-cli call`. It reads from the local cache populated by `det-cli cache`. Requires `jq` for dynamic tool name completion.

### Zsh

```bash
det-cli cache
det-cli completion zsh > "${fpath[1]}/_det-cli"
```

## Examples

```bash
# MCP_API_KEY is set in the app's .env — HTTP mode used automatically
det-cli call list_wallets

# Generate a receive address (use alias or hex seed hash)
det-cli call generate_receive_address wallet_id=savings

# List wallets in Dash Core
det-cli call list_core_wallets

# Override API key for one command
det-cli --bearer my-secret-key call list_wallets

# Stdio mode (no running GUI needed)
det-cli --standalone call list_wallets
MCP_NETWORK=testnet det-cli --standalone call list_wallets
```
