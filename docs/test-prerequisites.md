# Test Prerequisites

Shared preconditions for all smoke test scenarios in [test-cases.md](test-cases.md).

## Environment

- **Network:** Testnet
- **Backend mode:** SPV (no Dash Core RPC required)
- **OS:** macOS (arm64 or x86_64) — other platforms as available

## Configuration

### `.env` file

Place the following `.env` file at the DET config path:

- **macOS:** `~/Library/Application Support/Dash-Evo-Tool/.env`
- **Linux:** `~/.config/dash-evo-tool/.env`

```env
# Testnet SPV mode — no Core RPC needed
TESTNET_dapi_addresses=https://34.214.48.68:1443,https://52.12.176.90:1443,https://52.34.144.50:1443,https://44.240.98.102:1443,https://54.201.32.131:1443,https://52.10.229.11:1443,https://52.13.132.146:1443,https://52.40.219.41:1443,https://54.149.33.167:1443,https://35.164.23.245:1443,https://52.33.28.47:1443,https://52.43.13.92:1443,https://52.89.154.48:1443,https://52.24.124.162:1443,https://35.85.21.179:1443,https://54.187.14.232:1443,https://54.68.235.201:1443,https://52.13.250.182:1443
```

No `core_host`, `core_rpc_port`, `core_rpc_user`, `core_rpc_password`, or `core_zmq_endpoint` settings are needed for SPV mode.

### Backend mode

After launching DET, set the Core backend mode to **SPV**:

1. Navigate to Settings → Core Backend Mode
2. Select **SPV**
3. Wait for SPV peers to connect and sync (status indicator turns green)

> **Note:** SPV mode is not the default as of v1.0.0-dev. It must be explicitly selected in settings.

## Bank Wallet

The "bank" wallet is a pre-funded testnet wallet used as the source of funds for all tests. This wallet is never used to create Platform objects directly — it only distributes tDASH.

### Requirements

- **Minimum balance:** 10 tDASH (recommended: 50+ tDASH for full test suite)
- **Fund consolidation:** Before each test session, ensure as many tDASH as possible are consolidated at core address index 0 of the bank wallet. This ensures predictable UTXO behavior.
- **Mnemonic:** Store the bank wallet mnemonic securely. It is imported at the start of each test session.

### Obtaining test Dash

Fund the bank wallet via the [Dash testnet faucet](https://testnet-faucet.dash.org/) or by requesting tDASH from the team.

### Fund consolidation procedure

If bank wallet funds are spread across many addresses/UTXOs:

1. Import the bank wallet in DET
2. Send the full balance (minus fee) to the wallet's own receive address at index 0
3. Wait for confirmation
4. Verify a single large UTXO exists

## Fresh Start (for onboarding tests)

Some tests require a fresh DET state with no existing data. To achieve this:

1. Quit DET completely
2. Remove or rename the data directory:
   - **macOS:** `~/Library/Application Support/Dash-Evo-Tool/`
   - **Linux:** `~/.config/dash-evo-tool/`
3. Re-create the `.env` file (see Configuration above)
4. Re-launch DET

> **Warning:** This deletes all locally stored wallets, identities, and cached data. Only do this when explicitly required by a test case.

## Cleanup Between Sessions

After completing a full test run:

1. Transfer any remaining tDASH from test wallets back to the bank wallet's address at index 0
2. Remove test wallets created during the session
3. Identities and DPNS names created on Platform persist on-chain — they do not need local cleanup but should use unique names (timestamp-based) to avoid collisions with future runs
