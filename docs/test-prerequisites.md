# Test Prerequisites

Shared preconditions for all smoke test scenarios in [test-cases.md](test-cases.md).

## Environment

| Requirement | Value |
|---|---|
| **Network** | Testnet |
| **Backend mode** | SPV Client (no Dash Core RPC required) |
| **Expert mode** | Enabled (required to select SPV backend) |
| **OS** | macOS, Linux, or Windows |

## Secrets File

Test mnemonics and passwords are stored in `~/det-qa-secrets.env` on the test machine. **This file must never be committed to version control.**

### Required Variables

| Variable | Purpose |
|---|---|
| `BANK_MNEMONIC` | Bank wallet seed phrase (24 words recommended) |
| `BANK_PASSWORD` | Bank wallet encryption password |
| `TC_WAL_MNEMONIC` | Wallet-management tests (import, rename, lock, remove) |
| `TC_WAL_PASSWORD` | Password for the wallet-management test wallet |
| `TC_SND_MNEMONIC` | Send/receive tests |
| `TC_IDN_MNEMONIC` | Identity, DPNS, DashPay, and token tests |

### Derived Addresses

Each mnemonic deterministically produces the same addresses. After first import, record the receive address at index 0 for each wallet:

| Variable | Description |
|---|---|
| `BANK_ADDRESS_0` | Bank wallet — address at index 0 (fund return target) |
| `TC_SND_ADDRESS_0` | Send-test wallet — address at index 0 (pre-fund target) |
| `TC_IDN_ADDRESS_0` | Identity-test wallet — address at index 0 (pre-fund target) |

### Example `~/det-qa-secrets.env`

```env
BANK_MNEMONIC="word1 word2 word3 ... word24"
BANK_PASSWORD="strong-bank-password"
TC_WAL_MNEMONIC="word1 word2 word3 ... word24"
TC_WAL_PASSWORD="waltest1234"
TC_SND_MNEMONIC="word1 word2 word3 ... word24"
TC_IDN_MNEMONIC="word1 word2 word3 ... word24"

# Derived addresses (fill in after first import of each wallet)
BANK_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
TC_SND_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
TC_IDN_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

## .env Configuration

Place the following `.env` file at the DET configuration path:

| Platform | Path |
|---|---|
| **macOS** | `~/Library/Application Support/Dash-Evo-Tool/.env` |
| **Linux** | `~/.config/dash-evo-tool/.env` |
| **Windows** | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config\.env` |

```env
# Testnet SPV mode — only DAPI addresses are required
TESTNET_dapi_addresses=https://34.214.48.68:1443,https://52.12.176.90:1443,https://52.34.144.50:1443,https://44.240.98.102:1443,https://54.201.32.131:1443,https://52.10.229.11:1443,https://52.13.132.146:1443,https://52.40.219.41:1443,https://54.149.33.167:1443,https://35.164.23.245:1443,https://52.33.28.47:1443,https://52.43.13.92:1443,https://52.89.154.48:1443,https://52.24.124.162:1443,https://35.85.21.179:1443,https://54.187.14.232:1443,https://54.68.235.201:1443,https://52.13.250.182:1443
```

No `core_host`, `core_rpc_port`, `core_rpc_user`, `core_rpc_password`, or `core_zmq_endpoint` settings are needed for SPV mode.

## Enabling Expert Mode and SPV Backend

SPV backend selection is only visible when Expert mode is enabled.

1. Click **"Settings"** in the left sidebar.
2. Check the **"Expert mode"** checkbox.
3. Click **"Save"**.
4. The **Core backend mode** selector now appears. Select **"SPV Client"**.
5. Click **"Save"**.
6. Wait for the connection status indicator (top bar) to turn green, indicating SPV sync is complete.

> **Note:** Expert mode also reveals additional developer UI elements (address tables, refresh controls). This is expected.

## Bank Wallet

The bank wallet is a pre-funded, password-protected testnet wallet. It distributes tDASH to test wallets at the start of each session and collects leftover funds during cleanup. It is never used for Platform operations (identity, DPNS, etc.) directly.

### Requirements

- **Minimum balance:** 10 tDASH (recommended: 50+ tDASH for full test suite)
- **Password:** Must be set during import (value from `${BANK_PASSWORD}`)
- **Mnemonic:** Stored in `~/det-qa-secrets.env`, never typed in clear text outside DET

### Importing the Bank Wallet

1. Click **"Wallets"** in the left sidebar.
2. Click **"Import Wallet"** in the top-right corner.
3. Under **"Select what you want to import"**, choose **"Seed Phrase (HD Wallet)"**.
4. Under **"Select the seed phrase length"**, choose the word count matching `${BANK_MNEMONIC}` (e.g., 24).
5. Enter each word of `${BANK_MNEMONIC}` into the numbered fields (**"1:"**, **"2:"**, ... **"24:"**).
6. In the **"Name:"** field, type `Bank`.
7. In the **"Optional Password:"** field, enter `${BANK_PASSWORD}`.
8. Click **"Import Wallet"**.
9. Wait for the balance to sync (**"Core balance:"** appears under the wallet name).

### Fund Consolidation

If bank wallet funds are spread across multiple UTXOs, consolidate to address index 0:

1. Select the **Bank** wallet on the **"Wallets"** screen.
2. Click **"Receive"** and note the address shown (this is `${BANK_ADDRESS_0}`).
3. Click **"Send"**.
4. In the **"To:"** field, paste `${BANK_ADDRESS_0}`.
5. Click **"Max"** to set the maximum sendable amount.
6. A **"Fee Confirmation Required"** dialog appears — review the fee and total.
7. Confirm the transaction.
8. Wait for at least one confirmation.

### Obtaining Test Dash

Fund the bank wallet via the [Dash testnet faucet](https://testnet-faucet.dash.org/) or by requesting tDASH from the Dash team.

## Fresh Start (for Onboarding Tests)

Some tests require a clean DET state with no existing data:

1. Quit DET completely.
2. Remove or rename the data directory:
   - **macOS:** `~/Library/Application Support/Dash-Evo-Tool/`
   - **Linux:** `~/.config/dash-evo-tool/`
   - **Windows:** `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\`
3. Re-create the `.env` file (see [.env Configuration](#env-configuration) above).
4. Re-launch DET.

> **Warning:** This deletes all locally stored wallets, identities, and cached data. Only do this when explicitly required by a test case.
