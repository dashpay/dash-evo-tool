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
det-cli core-wallets-list

# Generate a receive address
det-cli core-address-create wallet-id=savings
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
det-cli --addr http://127.0.0.1:9000/mcp core-wallets-list
```

Force standalone mode with `--standalone` even when an API key is present.

## Usage

Commands use hyphens (`core-wallets-list`, not `core_wallets_list`). Parameters are passed as `key=value` pairs:

```bash
det-cli <command> [key=value ...]
```

Values are parsed as JSON first, falling back to plain strings. Exit code is non-zero on error.

### `serve` — MCP stdio server

```bash
det-cli serve
```

Runs an MCP server over stdin/stdout for Claude Desktop, Claude Code, AI agents, or other MCP clients. See [MCP.md](MCP.md) for client configuration.

### `headless` — HTTP MCP server daemon

```bash
det-cli headless
```

Runs the HTTP MCP server without a GUI. Requires `MCP_API_KEY` to be set. Useful for server environments or automated pipelines. Requires the `headless` feature: `cargo build --features headless`.

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
det-cli core-wallets-list

# Generate a receive address
det-cli core-address-create wallet-id=savings

# Show active network and available networks
det-cli network-info

# Check wallet balance
det-cli core-balances-get wallet-id=savings

# Fetch platform address balances (credits and nonces)
det-cli platform-addresses-list wallet-id=savings

# Query Platform withdrawals currently in queue
det-cli platform-withdrawals-get

# Query recently completed withdrawals
det-cli platform-withdrawals-get status=completed

# Get full schema and description for a tool
det-cli tool-describe name=core_funds_send

# Send 0.01 DASH (1,000,000 duffs) to an address (network is required)
det-cli core-funds-send wallet-id=savings address=yXyz... amount-duffs=1000000 network=testnet

# Import a wallet from a BIP39 mnemonic. Quote the phrase so the shell
# does not split on whitespace. `network` is required.
det-cli core-wallet-import \
  mnemonic="abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" \
  alias="savings" \
  network=testnet

# Same import with a BIP39 passphrase ("25th word") and an at-rest
# encryption password. If the app already has a main password set,
# `encryption-password` MUST match it — mismatched passwords are rejected.
det-cli core-wallet-import \
  mnemonic="..." \
  passphrase="trezor" \
  encryption-password="the-existing-main-password" \
  alias="primary" \
  network=testnet

# Delete a wallet by alias. `confirm-seed-hash` must match the target
# wallet's hex seed_hash exactly — use `core-wallets-list` first to copy
# the correct value. Deletion is permanent and scoped to the specified
# network.
det-cli core-wallet-delete \
  wallet-id="savings" \
  confirm-seed-hash=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  network=testnet

# Delete by passing the hex seed_hash as both the lookup id and the
# confirmation — the confirmation is always required.
det-cli core-wallet-delete \
  wallet-id=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  confirm-seed-hash=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  network=testnet

# Allow deleting a wallet that still holds a non-zero balance. Use only
# when the mnemonic is backed up safely elsewhere; on-chain funds remain
# but are inaccessible without the mnemonic.
det-cli core-wallet-delete \
  wallet-id="savings" \
  confirm-seed-hash=... \
  allow-delete-with-balance=true \
  network=testnet

# Run as stdio MCP server for Claude Desktop or Claude Code
det-cli serve
```
