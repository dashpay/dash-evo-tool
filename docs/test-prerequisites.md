# Test Prerequisites

Shared preconditions for all smoke test scenarios in [test-cases.md](test-cases.md).

## Environment

| Requirement | Value |
|---|---|
| **Network** | Testnet |
| **Backend mode** | SPV Client (no Dash Core RPC required) |
| **Developer mode** | Enabled (required to select SPV backend) |
| **OS** | macOS, Linux, or Windows |

## Secrets File

Test mnemonics and passwords are stored in `~/.secrets/det-qa-mnemonics.env` on the test machine. **This file must never be committed to version control.** Set permissions to `chmod 600`.

### Required Variables

| Variable | Words | Purpose |
|---|---|---|
| `BANK_MNEMONIC` | 24 | Bank wallet — pre-funded, password-protected |
| `BANK_PASSWORD` | — | Bank wallet encryption password |
| `TC_WAL_MNEMONIC` | 24 | Wallet management tests (import, rename, lock, remove) |
| `TC_WAL_PASSWORD` | — | Password for wallet-management test wallet |
| `TC_SND_MNEMONIC` | 21 | Send/receive tests |
| `TC_IDN_MNEMONIC` | 21 | Identity registration and top-up tests |
| `TC_TOK_MNEMONIC` | 18 | Token operation tests |
| `TC_DPY_MNEMONIC` | 18 | DashPay profile and contact tests |
| `TC_DOC_MNEMONIC` | 15 | Contract and document tests |
| `TC_DEV_MNEMONIC` | 15 | Developer tools tests |
| `TC_NET_MNEMONIC` | 12 | Network/settings tests |
| `TC_ALK_MNEMONIC` | 12 | Asset lock tests |

### Derived Addresses

Each mnemonic deterministically produces the same addresses. After first import, record the receive address at index 0 for each wallet:

| Variable | Description |
|---|---|
| `BANK_ADDRESS_0` | Bank wallet — address at index 0 (fund return target) |
| `TC_SND_ADDRESS_0` | Send-test wallet — address at index 0 (pre-fund target) |
| `TC_IDN_ADDRESS_0` | Identity-test wallet — address at index 0 (pre-fund target) |

### Example `~/.secrets/det-qa-mnemonics.env`

```env
# DO NOT COMMIT — test wallet mnemonics
BANK_MNEMONIC="word1 word2 ... word24"
BANK_PASSWORD="strong-bank-password"
TC_WAL_MNEMONIC="word1 word2 ... word24"
TC_WAL_PASSWORD="waltest1234"
TC_SND_MNEMONIC="word1 word2 ... word21"
TC_IDN_MNEMONIC="word1 word2 ... word21"
TC_TOK_MNEMONIC="word1 word2 ... word18"
TC_DPY_MNEMONIC="word1 word2 ... word18"
TC_DOC_MNEMONIC="word1 word2 ... word15"
TC_DEV_MNEMONIC="word1 word2 ... word15"
TC_NET_MNEMONIC="word1 word2 ... word12"
TC_ALK_MNEMONIC="word1 word2 ... word12"

# Derived addresses — fill in after first import of each wallet
BANK_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
TC_SND_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
TC_IDN_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

## .env Configuration

Place the following `.env` file at the DET configuration path:

| Platform | Path |
|---|---|
| **macOS** | `~/Library/Application Support/Dash-Evo-Tool/.env` |
| **Linux** | `~/.config/Dash-Evo-Tool/.env` |
| **Windows** | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\.env` |

```env
# Testnet SPV mode — DAPI addresses plus placeholder Core RPC fields
TESTNET_dapi_addresses=https://34.214.48.68:1443,https://52.12.176.90:1443,https://52.34.144.50:1443,https://44.240.98.102:1443,https://54.201.32.131:1443,https://52.10.229.11:1443,https://52.13.132.146:1443,https://52.40.219.41:1443,https://54.149.33.167:1443,https://35.164.23.245:1443,https://52.33.28.47:1443,https://52.43.13.92:1443,https://52.89.154.48:1443,https://52.24.124.162:1443,https://35.85.21.179:1443,https://54.187.14.232:1443,https://54.68.235.201:1443,https://52.13.250.182:1443

# Core RPC fields are structurally required by NetworkConfig even in SPV mode,
# but are not actively used at runtime. Supply placeholder values.
TESTNET_core_host=127.0.0.1
TESTNET_core_rpc_port=19998
TESTNET_core_rpc_user=user
TESTNET_core_rpc_password=password
TESTNET_insight_api_url=https://insight.testnet.networks.dash.org:3002/insight-api
# core_zmq_endpoint is optional and can be omitted
```

> **Note:** While Core RPC settings (`core_host`, `core_rpc_port`, `core_rpc_user`, `core_rpc_password`, `insight_api_url`) are not actively used in SPV mode, the `NetworkConfig` struct requires them to be present during deserialization. Use placeholder values as shown above. The `core_zmq_endpoint` field is optional (`Option<String>`) and may be omitted.

## Enabling Developer Mode and SPV Backend

SPV backend selection is only visible when Developer Mode is enabled.

### Enable Developer Mode

1. Click **"Settings"** in the left sidebar.
2. Check the **"Developer mode"** checkbox.
3. Click **"Save"**.

### Select SPV Backend

The backend mode selector is in the **Network Chooser** screen, not in Settings.

1. Open the **Network Chooser** screen (displayed at startup or via the network selector).
2. Locate the **"Connection Settings"** section.
3. Open the **"Connection Type"** dropdown (visible only when Developer Mode is enabled).
4. Select **"SPV"**.
5. Confirm the selection and connect.
6. Wait for the connection status indicator (top bar) to turn green, indicating SPV sync is complete.

> **Note:** The selected backend mode is stored in the `Settings` struct as `core_backend_mode` and persisted to the local database. Developer Mode also reveals additional UI elements (address tables, refresh controls). This is expected.

## Bank Wallet

The bank wallet is a pre-funded, password-protected testnet wallet. It distributes tDASH to test wallets at the start of each session and collects leftover funds during cleanup. It is never used for Platform operations (identity, DPNS, etc.) directly.

### Requirements

- **Minimum balance:** 10 tDASH (recommended: 50+ tDASH for full test suite)
- **Password:** Must be set during import (value from `${BANK_PASSWORD}`)
- **Mnemonic:** Stored in `~/.secrets/det-qa-mnemonics.env`, never typed in clear text outside DET

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
   - **Linux:** `~/.config/Dash-Evo-Tool/`
   - **Windows:** `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\`
3. Re-create the `.env` file (see [.env Configuration](#env-configuration) above).
4. Re-launch DET.

> **Warning:** This deletes all locally stored wallets, identities, and cached data. Only do this when explicitly required by a test case.
