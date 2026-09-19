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

These are the only variables the current executable weekly smoke run references. Set these before running the suite.

| Variable | Words | Purpose |
|---|---|---|
| `BANK_MNEMONIC` | 24 | Bank wallet — pre-funded, password-protected |
| `BANK_PASSWORD` | — | Bank wallet encryption password |
| `TC_WAL_MNEMONIC` | 24 | Wallet management tests (import, rename, lock, remove) |
| `TC_WAL_PASSWORD` | — | Password for wallet-management test wallet |
| `TC_SND_MNEMONIC` | 21 | Send/receive tests |
| `TC_IDN_MNEMONIC` | 21 | Identity registration, top-up, DPNS, DashPay, token, and contract tests |

### Optional / Future-Isolation Variables

These mnemonics are reserved for potential future per-domain wallet isolation. They are **not required** for the current smoke suite — the active suite reuses the **Identity Test** wallet (`TC_IDN_MNEMONIC`) for token, DashPay, contract, developer, network, and asset-lock cases. Provision these only if you plan to run isolated per-domain variants.

| Variable | Words | Potential Purpose |
|---|---|---|
| `TC_TOK_MNEMONIC` | 18 | Token operation tests (isolated wallet) |
| `TC_DPY_MNEMONIC` | 18 | DashPay profile and contact tests (isolated wallet) |
| `TC_DOC_MNEMONIC` | 15 | Contract and document tests (isolated wallet) |
| `TC_DEV_MNEMONIC` | 15 | Developer tools tests (isolated wallet) |
| `TC_NET_MNEMONIC` | 12 | Network/settings tests (isolated wallet) |
| `TC_ALK_MNEMONIC` | 12 | Asset lock tests (isolated wallet) |

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

# --- Required for the current weekly smoke run ---
BANK_MNEMONIC="word1 word2 ... word24"
BANK_PASSWORD="strong-bank-password"
TC_WAL_MNEMONIC="word1 word2 ... word24"
TC_WAL_PASSWORD="waltest1234"
TC_SND_MNEMONIC="word1 word2 ... word21"
TC_IDN_MNEMONIC="word1 word2 ... word21"

# Derived addresses — fill in after first import of each wallet
BANK_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
TC_SND_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
TC_IDN_ADDRESS_0=yXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX

# --- Optional: future per-domain isolation (not used by the active suite) ---
# Uncomment only if you plan to run isolated per-domain test variants.
# TC_TOK_MNEMONIC="word1 word2 ... word18"
# TC_DPY_MNEMONIC="word1 word2 ... word18"
# TC_DOC_MNEMONIC="word1 word2 ... word15"
# TC_DEV_MNEMONIC="word1 word2 ... word15"
# TC_NET_MNEMONIC="word1 word2 ... word12"
# TC_ALK_MNEMONIC="word1 word2 ... word12"
```

## Reusable Test Data

Test cases reference these placeholders for shared, non-secret inputs. They are not stored in `~/.secrets/det-qa-mnemonics.env` — they live here in the test plan because they are either stable system constants or values discovered at runtime.

| Placeholder | Source | Value / How to obtain |
|---|---|---|
| `${DPNS_CONTRACT_ID}` | Stable system contract (Base58) — see `src/backend_task/dashpay/contact_requests.rs` | `GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec` |
| `${TC_TOK_CONTRACT_ID}` | Discovered at runtime from TC-TOK-002-01 results | Run TC-TOK-002-01 (**Tokens → Search Tokens**, keyword `test`), then record the **Contract ID** column from any one result row before running TC-TOK-003-01. If TC-TOK-002-01 returns no results, TC-TOK-003-01 is blocked pending a seeded testnet token contract ID. |

## .env Configuration

Place the following `.env` file at the DET configuration path:

| Platform | Path |
|---|---|
| **macOS** | `~/Library/Application Support/Dash-Evo-Tool/.env` |
| **Linux** | `~/.config/dash-evo-tool/.env` |
| **Windows** | `C:\Users\<User>\AppData\Roaming\Dash-Evo-Tool\config\.env` |

Use the bundled `.env.example` as the source of truth for all `MAINNET_*` and
`TESTNET_*` endpoint and config values. DET copies that file into the app
data directory on first launch, so this guide should only call out
smoke-test-specific constraints and edits:

- `MAINNET_*` values must be present at startup even though this smoke suite
  runs on Testnet.
- The smoke suite uses `TESTNET_*`; ensure
  `TESTNET_show_in_ui=true`.
- If you customize the file for SPV mode, the Testnet Core RPC fields can
  stay as placeholder values because they are not actively used at runtime,
  but they still need to be present for config deserialization.
- `core_zmq_endpoint` is optional and may be omitted.

## Enabling Developer Mode and SPV Backend

SPV backend selection is only visible when Developer Mode is enabled.

### Enable Developer Mode

1. Click **"Settings"** in the left sidebar.
2. Click the **"Advanced Settings"** header to expand it.
3. Check the **"Developer mode"** checkbox.
4. Verify the **"Connection Type"** dropdown appears in **"Connection Settings"** on **Settings (Network Chooser)**.
5. Click **"Wallets"** and verify additional developer UI appears (for example, address tables and refresh controls).

### Select SPV Backend

The backend mode selector is part of **Settings (Network Chooser)**.

1. Open **"Settings"** in the left sidebar.
2. Locate the **"Connection Settings"** section.
3. Open the **"Connection Type"** dropdown (visible only when Developer Mode is enabled).
4. Select **"SPV Client"**.
5. Click the **"Connect"** button to start the SPV client.
6. Wait for the connection status indicator (top bar) to turn green, indicating SPV sync is complete.

> **Note:** The selected backend mode is stored in the `Settings` struct as `core_backend_mode` and persisted to the local database. Developer Mode also reveals additional UI elements (address tables, refresh controls). This is expected.

## Bank Wallet

The bank wallet is a pre-funded, password-protected testnet wallet. It distributes tDASH to test wallets at the start of each session and collects leftover funds during cleanup. It is never used for Platform operations (identity, DPNS, etc.) directly.

### Requirements

- **Minimum balance:** 20 tDASH (recommended: 50+ tDASH for full test suite)
  This covers the 4 tDASH Session Setup outflow plus normal transaction
  fees while leaving comfortably more than 10 tDASH for later Bank balance
  checks.
- **Password:** Must be set during import (value from `${BANK_PASSWORD}`)
- **Mnemonic:** Stored in `~/.secrets/det-qa-mnemonics.env`, never typed in clear text outside DET

### Importing the Bank Wallet

1. Click **"Wallets"** in the left sidebar.
2. Click **"Import Wallet"** in the top-right corner.
3. The mnemonic seed phrase import screen is shown by default.
4. Under **"Select the seed phrase length"**, choose the word count matching `${BANK_MNEMONIC}` (e.g., 24).
5. Enter each word of `${BANK_MNEMONIC}` into the numbered fields (**"1:"**, **"2:"**, ... **"24:"**).
6. In the **"Name:"** field, type `Bank`.
7. In the **"Optional Password:"** field, enter `${BANK_PASSWORD}`.
8. Click **"Save Wallet"**.
9. Wait for the balance to sync (**"Core balance:"** appears under the wallet name).

### Fund Consolidation

If bank wallet funds are spread across multiple UTXOs, consolidate to address index 0:

1. Select the **Bank** wallet on the **"Wallets"** screen.
2. Click **"Receive"** and note the address shown (this is `${BANK_ADDRESS_0}`).
3. Click **"Send"**.
4. In the **"Send to"** field (hint: *"Enter address (X.../y.../evo1.../tevo1...)"*), paste `${BANK_ADDRESS_0}`.
5. Click **"Max"** to set the maximum sendable amount.
6. Click **"Send"** in the form.
7. Verify the form is replaced by a spinner and a **"Sending..."** heading while the transaction is broadcast.
8. Wait for the success banner and verify it shows the sent amount and destination address (**"Sent {amount} DASH to ${BANK_ADDRESS_0}"**).
9. Click **"Back to Wallet"**.
10. Wait for at least one confirmation.

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
