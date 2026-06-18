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

# Paginate: first page of 10, then continue from the returned next_cursor
det-cli platform-withdrawals-get status=completed limit=10
det-cli platform-withdrawals-get status=completed limit=10 start_after=<document_id>

# Get full schema and description for a tool
det-cli tool-describe name=core_funds_send

# Send 0.01 DASH (1,000,000 duffs) to an address (network is required)
det-cli core-funds-send wallet-id=savings address=yXyz... amount-duffs=1000000 network=testnet

# Run as stdio MCP server for Claude Desktop or Claude Code
det-cli serve
```

## Masternode / evonode credit withdrawal (headless)

Withdraw a masternode/evonode identity's Platform credits without the GUI:
first load the identity by ProTxHash + keys, then withdraw in either key mode.
The keys are accepted as inline `key=value` arguments (WIF or 64-char hex) — see
the private-key handling note in `MCP.md` and keep the HTTP endpoint loopback-only
for key-bearing calls. Inline `key=value` arguments are visible to other local
users (`ps`, `/proc/<pid>/cmdline`) and are saved to shell history. On a shared or
untrusted host, prefer the deferred env-var/stdin entry path once available, or
clear your shell history afterward. In transfer mode, `to-address` must be a Core
address — Platform (bech32m `dash1…`/`tdash1…`) addresses are rejected.

```bash
# 1. Load an evonode identity (testnet). Provide at least one of the owner or
#    payout key; voting key and alias are optional.
det-cli identity-masternode-load \
  pro-tx-hash=<64-hex protx> \
  node-type=evonode \
  owner-private-key=<WIF> \
  payout-private-key=<WIF> \
  network=testnet
# -> { "identity_id": "...", "available_withdrawal_keys": ["owner","transfer"],
#      "payout_address": "y...", ... }

# 2a. Owner key — destination is forced to the registered payout address.
#     Supplying to-address is rejected.
det-cli identity-masternode-credits-withdraw \
  identity-id=<base58> \
  key-mode=owner \
  amount-credits=100000 \
  network=testnet

# 2b. Payout/transfer key — withdraw to any Core address.
det-cli identity-masternode-credits-withdraw \
  identity-id=<base58> \
  key-mode=transfer \
  to-address=y... \
  amount-credits=100000 \
  network=testnet
```

## Shielded self-verification loop (testnet)

The shielded read/control tools let an agent drive and verify a full shielded
lifecycle headlessly — no GUI. Onboard a pre-funded testnet seed, prepare the
wallet, then move funds and confirm each balance change with `shielded-sync`.

```bash
# 1. Import the funded testnet seed (returns its seed_hash; idempotent)
det-cli core-wallet-import mnemonic="word1 word2 ... word12" network=testnet alias=shielded-test

# 2. Bind shielded keys + warm the proving key (~30s; idempotent)
det-cli shielded-init wallet-id=shielded-test

# 3. Shield some Core DASH into the pool (SPV-gated — can take minutes)
det-cli shielded-shield-from-core wallet-id=shielded-test amount-duffs=2000000 network=testnet

# 4. Sync and read the new shielded balance (expect it to increase)
det-cli shielded-sync wallet-id=shielded-test

# 5. Read the wallet's own shielded address, then transfer to it privately
det-cli shielded-address-get wallet-id=shielded-test
det-cli shielded-transfer wallet-id=shielded-test to-address=tdash1z... amount-credits=50000 network=testnet

# 6. Unshield part back to a Platform address, then withdraw part to Core
det-cli shielded-unshield wallet-id=shielded-test to-address=tdash1... amount-credits=300000 network=testnet
det-cli shielded-withdraw wallet-id=shielded-test to-address=yXyz... amount-credits=300000 network=testnet

# 7. Final sync to confirm the closing balance
det-cli shielded-sync wallet-id=shielded-test

# Fast read at any time (no sync, returns the last synced snapshot)
det-cli shielded-balance-get wallet-id=shielded-test
```

The `mnemonic` for the framework wallet is read from `E2E_WALLET_MNEMONIC`
(shell env or the project-root `.env`) in the backend-e2e harness; for the
standalone `det-cli` loop above, pass it directly to `core-wallet-import`.

