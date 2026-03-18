# det-cli

`det-cli` is the command-line client for Dash Evo Tool's MCP server. Use it to call wallet and core operations from scripts and terminals without opening the GUI.

## Build

```bash
cargo build --features cli
```

The binary is `det-cli` in the Cargo output directory.

## Connection modes

### HTTP (default)

Connects to a running Dash Evo Tool instance's MCP HTTP server.

Requires either `--bearer <token>` or the `DET_CLI_BEARER` environment variable.

```bash
export DET_CLI_BEARER=my-secret-key
det-cli tools
```

The default server address is `http://127.0.0.1:9527/mcp`. Override with `--addr`:

```bash
det-cli --addr http://127.0.0.1:9000/mcp tools
```

### Standalone (`--standalone`)

Spawns `dash-evo-tool-mcp` as a child process and connects via stdin/stdout. Does not require a running GUI app. Does not require `--bearer`.

`dash-evo-tool-mcp` must be built (`cargo build --features mcp-stdio`) and on `PATH`.

```bash
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
export DET_CLI_BEARER=my-secret-key

# List wallets loaded in the app
det-cli call list_wallets

# Generate a receive address (use alias or hex seed hash)
det-cli call generate_receive_address wallet_id=savings

# List wallets in Dash Core
det-cli call list_core_wallets

# Same operations without a running GUI (standalone mode)
det-cli --standalone call list_wallets
MCP_NETWORK=testnet det-cli --standalone call list_wallets
```
