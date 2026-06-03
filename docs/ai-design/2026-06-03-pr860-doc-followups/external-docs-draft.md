# External Docs Draft — PR #860 (platform-wallet migration)

**Target repo:** `dashpay/docs`
**Target path:** `docs/user/network/dash-evo-tool/`
**Published at:** https://docs.dash.org/en/stable/docs/user/network/dash-evo-tool/

Paste the sections below into the relevant pages on the dashpay/docs site after PR #860
merges. The draft is written in the same plain Markdown that the Dash docs site renders.

---

## Tracking

This file must result in a PR or issue filed against `dashpay/docs` after PR #860 merges to
`v1.0-dev`. Until that PR is open, user-facing documentation for the storage migration and
wallet limitations is absent from the public docs site.

TODO: file a `dashpay/docs` issue linking to PR #860 and referencing this draft once the
merge commit is confirmed.

---

## Draft content

### New in this release — wallet storage update

Dash Evo Tool now stores wallet data in an encrypted vault (`secrets.pwsvault`) rather than
the legacy `data.db` database file.

**What changes for you:**

- On first launch after upgrading, DET migrates your existing wallets automatically. A
  brief progress notice appears during migration (usually under one second). No action is
  required.
- Your existing `data.db` file is left on disk but is no longer used. You can keep it as a
  backup or remove it once you have confirmed your wallets loaded correctly.
- Wallet metadata (name, main-wallet flag) moves to a new `det-app.sqlite` file in the same
  folder.

**File locations after migration:**

| Platform | Wallet vault | Metadata sidecar |
|----------|-------------|-----------------|
| macOS | `~/Library/Application Support/Dash-Evo-Tool/secrets/det-secrets.*` | `~/Library/Application Support/Dash-Evo-Tool/det-app.sqlite` |
| Linux | `~/.config/dash-evo-tool/secrets/det-secrets.*` | `~/.config/dash-evo-tool/det-app.sqlite` |
| Windows | `%APPDATA%\Dash-Evo-Tool\config\secrets\det-secrets.*` | `%APPDATA%\Dash-Evo-Tool\config\det-app.sqlite` |

---

### Passphrase prompt — sign-time unlock

DET now asks for your wallet passphrase only when an operation actually needs your private
key (sending funds, registering an identity, signing a message). Previous versions held the
wallet open for the entire session once you unlocked it.

When the prompt appears, you can check **"Keep this wallet unlocked until I close the app"**
to avoid repeated prompts during a busy session. The wallet locks again automatically when
you close DET.

---

### Known limitations — single-key (imported WIF) wallets

Importing a single-key wallet, viewing it, and signing with it work in this release.

**Sending funds and refreshing the balance or UTXO list are not available in this release.**
If you attempt either action, DET will show a notice explaining this.

Your key data is preserved in the encrypted vault and these actions will be available in a
future update. To send funds now, use an HD (recovery-phrase) wallet.

---

### DashPay contacts — legacy address compatibility

This release drops support for DashPay contact-request addresses derived outside mainnet
account 0 under the legacy DIP-14 scheme. This affects:

- Contacts established on **testnet or devnet** using the old address derivation.
- Contacts established on any network using a **non-default account index** (account 1 or
  higher).

If you are affected, the existing contact entry remains visible but payment addresses for
those contacts may not match. Re-establishing the contact from both sides (send a new
contact request and have the other party accept it) restores full functionality.

Mainnet contacts established via the default account (account 0) are not affected.
