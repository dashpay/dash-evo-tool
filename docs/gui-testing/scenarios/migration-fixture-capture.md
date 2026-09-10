# Scenario: Capture a v0.9.3 migration fixture

**Verifies:** nothing about the current build — this scenario *produces* an
input for the migration matrix. It is complete when a released v0.9.3 binary
has written a data directory holding the
[`wallet-identity-dpns`](../../../tests/migration-fixtures/profiles/wallet-identity-dpns.md)
profile, and that directory has been packed as a fixture archive.

**Tier justification:** v0.9.3 predates det-cli, the MCP server and the SPV
stack, so it has no headless entry point of any kind — the only way to make
that binary write its own on-disk shapes is to drive its real GUI. Nothing in
`kittest` or `backend-e2e` can help: both compile against *current* sources and
would produce current-era files, which is exactly what a migration fixture must
not be.

**Deviation from the A/B contract:** the library's [A/B build comparison
contract](../README.md#ab-build-comparison-contract) does not apply here. This
scenario runs once, against one specific released binary, and the "second
build" role is played later by the migration harness replaying the archive.
Everything else in the README — isolated data directory, testnet default,
fund-movement caps, read the logs — applies unchanged.

## Prerequisites

- Network: **testnet**. A mainnet capture is forbidden outright; see the threat
  model in [`tests/migration-fixtures/README.md`](../../../tests/migration-fixtures/README.md).
- Environment variables (names only — never paste values into a file, a report
  or a command line that lands in shell history):
  - `MIGRATION_FIXTURE_MNEMONIC` — the dedicated, public-by-design fixture
    wallet. **Not** `E2E_WALLET_MNEMONIC`, which belongs to the funded
    backend-E2E framework wallet and must never be published in an artifact.
  - `MIGRATION_FIXTURE_PROTECTED_MNEMONIC` — a second, distinct phrase for the
    password-protected wallet.
- The protected wallet's password is the fixed literal
  `correct horse battery staple`, matching `PROTECTED_PASSWORD` in
  `src/backend_task/migration/v093_upgrade.rs`. It is deliberately not a
  secret.
- **A testnet identity registered ahead of time with the CURRENT build**, from
  `MIGRATION_FIXTURE_MNEMONIC`, holding a resolved, uncontested DPNS name.
  v0.9.3 does ship registration screens, but it has no SPV stack: it sees funds
  only through a local Dash Core testnet node over RPC/ZMQ, and without one it
  cannot build the asset lock a registration needs. Record the identity ID, its
  derivation index and the name before starting.
- `gh` authenticated against `dashpay/dash-evo-tool`, plus `unzip`.
- Confirm the identity and the name still resolve on testnet *today* — see the
  precondition checklist in the profile file. Shared testnet is mutable.

## Setup

```bash
# Scratch tree — every level 0700. The current build's secret store refuses to
# open a data directory with a group/other-writable ancestor, so a fixture
# captured under a loose /tmp cannot be replayed later.
SCRATCH=$(mktemp -d); chmod 700 "$SCRATCH"

# The released v0.9.3 binary. Asset naming changed between eras — this is the
# confirmed name for this tag, do not guess it for another one.
gh release download v0.9.3 --repo dashpay/dash-evo-tool \
  --pattern dash-evo-tool-x86_64-linux.zip --dir "$SCRATCH"
unzip -q "$SCRATCH/dash-evo-tool-x86_64-linux.zip" -d "$SCRATCH/bin"
OLD_BIN=$(find "$SCRATCH/bin" -type f -name 'dash-evo-tool*' | head -1)
chmod +x "$OLD_BIN"; "$OLD_BIN" --version 2>&1 | head -2

# v0.9.3 has no DASH_EVO_DATA_DIR (absent from src/app_dir.rs at that tag).
# It resolves its data directory through ProjectDirs, so XDG_CONFIG_HOME is
# the only isolation lever. Unset DASH_EVO_DATA_DIR so a value inherited from
# the shell cannot mislead anyone reading the transcript later.
FIXTURE_HOME="$SCRATCH/xdg"; mkdir -p "$FIXTURE_HOME"; chmod 700 "$FIXTURE_HOME"
DATADIR="$FIXTURE_HOME/dash-evo-tool"; mkdir -p "$DATADIR"; chmod 700 "$DATADIR"

# Era-correct config. v0.9.3's .env.example uses the older key spelling
# (TESTNET_insight_api_url, TESTNET_show_in_ui, TESTNET_core_zmq_endpoint, …).
# Today's .env.example would be silently wrong here.
git show v0.9.3:.env.example > "$DATADIR/.env"

pgrep -af dash-evo-tool           # never disturb an instance already running
: "${DISPLAY:?Set DISPLAY to the desktop used for GUI testing}"
xdpyinfo >/dev/null

LOG="$SCRATCH/migration-fixture-capture.log"
env -u DASH_EVO_DATA_DIR XDG_CONFIG_HOME="$FIXTURE_HOME" \
  nohup "$OLD_BIN" >"$LOG" 2>&1 &
APP_PID=$!
WID=$(xdotool search --pid "$APP_PID" | head -1)
xdotool windowsize "$WID" 1260 780; xdotool windowactivate "$WID"
```

Confirm before touching anything: `ls "$DATADIR"` shows `.env`, and `data.db`
and `det.log` appear as the app starts. If the data directory stayed empty, the
app resolved a *different* directory — stop, do not continue, and check
`XDG_CONFIG_HOME` before it writes into a real profile.

## Procedure

### 1. Select testnet

1. Open the network chooser and select **testnet**. Record which network the
   app started on rather than assuming; a fresh install does not necessarily
   start where you expect.
2. Confirm the selection persisted in the UI before moving on. Every later step
   writes rows tagged with the active network, and a wallet captured on the
   wrong network makes the whole archive useless.

### 2. Import the unprotected wallet (Wallet U)

3. Go to the wallets area and start the wallet **import** flow ("Follow these
   steps to import your wallet").
4. Select the seed-phrase length matching `MIGRATION_FIXTURE_MNEMONIC`, then
   enter the words. The screen has **one field per word**, not a single
   paste-able box — type or `xdotool type` into each field and tab between
   them. The screen validates continuously and shows "Invalid seed phrase…"
   until every word is present and valid.
5. Under step 2 of that screen, set the **Wallet Name** to a recognisable,
   non-default alias (e.g. `Fixture unprotected`).
6. Leave the **Optional Password** empty. Wallet U must stay unprotected — it
   is what proves an upgrade never hands an unprotected wallet a password the
   user cannot supply.
7. Press **Save Wallet** and confirm the wallet appears in the wallet list.

### 3. Import the password-protected wallet (Wallet P)

8. Repeat the import flow with `MIGRATION_FIXTURE_PROTECTED_MNEMONIC` and a
   distinct alias (e.g. `Fixture protected`).
9. This time fill **Optional Password** with `correct horse battery staple`.
   v0.9.3's import screen has no password-hint field; do not go looking for
   one.
10. Save, then confirm the wallet list shows two distinct wallets. If it shows
    one, the two phrases were the same — the fixture is a single-wallet fixture
    and the profile is not met.

### 4. Derive and record addresses

11. For each wallet, open its address view and make sure at least one receive
    address is derived and displayed.
12. Record the addresses in the capture report (outside the archive). A later
    run asserts the *same* addresses return, which is only checkable against a
    recorded value.

### 5. Adopt the pre-registered identity

13. Open **Load Existing Identity**.
14. Prefer the **By Wallet** tab: select Wallet U, unlock it if asked, set
    **Search type** to *Specific index*, enter the identity's derivation index,
    and press **Search For Identity**. This path links the identity to the
    wallet and derives its keys from the seed, which is what the profile wants.
15. If the wallet search does not find it, fall back to the **By Identity** tab:
    paste the identity ID, set **Identity Type** to *User*, set an alias, and
    load. Record which path was used — it changes what the fixture proves about
    the identity/wallet link.
16. Either path fetches the identity's DPNS names as part of the load. Wait for
    the load to report success and for the name to actually appear beside the
    identity in the identities list. **Do not proceed on the ID alone.**
17. Screenshot the identities list showing the name (`scrot -o …`). At this era
    the name lives inside the opaque `identity.data` bincode blob, so the
    screenshot is the only practical evidence — no SQL query can confirm it.

### 6. Quit cleanly and pack

18. Quit through the app's own window close. Do not `kill` it: a half-written
    WAL sidecar is not a valid fixture.
19. Confirm the process is gone (`pgrep -af dash-evo-tool`) before touching the
    files.
20. Verify the shape of what was captured:

    ```bash
    ls -la "$DATADIR"
    sqlite3 -readonly "$DATADIR/data.db" \
      "PRAGMA user_version; \
       SELECT alias, is_main, uses_password, network FROM wallet; \
       SELECT hex(id), alias, identity_type, network, wallet IS NOT NULL FROM identity;"
    ```

    Expect `user_version` = 11, two wallet rows (one with `uses_password` = 1,
    one with 0), and exactly one identity row. Always `-readonly`: a plain open
    can checkpoint the WAL and mutate the very state being preserved.
21. Pack per [`tests/migration-fixtures/README.md`](../../../tests/migration-fixtures/README.md)
    — include `data.db` and `.env`, exclude `backups/` and `*.log` — then
    upload the archive and fill in the manifest entry's `artifact` fields,
    `captured_at` and `sha256`.
22. Delete the scratch tree only after the archive is uploaded and its checksum
    is recorded. A capture that has to be redone from scratch costs an
    identity registration.

## Safety constraints specific to this scenario

- **This scenario broadcasts nothing.** No send, no registration, no top-up
  happens from the v0.9.3 binary. Any step that appears to offer one is a
  deviation — stop and record it.
- **Never capture with a wallet that holds meaningful funds.** The archive
  publishes the encrypted seed, and the password guarding it is written down in
  this repository. Dust only, dedicated wallet only, and never
  `E2E_WALLET_MNEMONIC`.
- **Never let v0.9.3 see the default data directory.** It has no
  `DASH_EVO_DATA_DIR` override and no confirmation prompt; launching it without
  `XDG_CONFIG_HOME` set would run an old build directly against a real profile
  and its schema-11 expectations.
- **Do not type a recovery phrase on a command line** or leave one visible in a
  screenshot. Capture screenshots before typing or after the field is cleared.
- **One fixture per data directory, always fresh.** Never capture a second
  fixture on top of a directory an earlier run touched.

## Expected outcome / pass criteria

The capture succeeded when all of the following hold:

- `"$DATADIR"` contains `data.db` with `PRAGMA user_version` = **11**, and no
  `det-<network>.sqlite` and no `secrets/` directory — v0.9.3 predates both, and
  their presence means a newer binary opened the directory and already migrated
  it, destroying the fixture.
- Two wallet rows with distinct `seed_hash`, distinct aliases, `uses_password`
  = 1 and 0 respectively, `network` = `testnet` for both.
- Exactly one `identity` row, `identity_type` `User`, `network` `testnet`,
  ideally with a non-null `wallet` link (By Wallet path).
- A screenshot showing the DPNS name rendered next to that identity.
- No Rust panic (`location=…`) in `$LOG` or `$DATADIR/det.log`.
- Every item of the
  [`wallet-identity-dpns`](../../../tests/migration-fixtures/profiles/wallet-identity-dpns.md)
  checklist ticked, and the manifest entry filled in.

Anything less is a `wallet-only` fixture at best. Record which profile was
actually achieved in the manifest entry rather than claiming the intended one.

## Known gotchas

- **`XDG_CONFIG_HOME` is the only isolation lever at this tag.**
  `DASH_EVO_DATA_DIR` does not exist in v0.9.3's `src/app_dir.rs`; exporting it
  isolates nothing and the app quietly uses the real profile directory. Verify
  files actually appear under the scratch directory within seconds of launch.
- **Today's `.env.example` is the wrong file.** v0.9.3 reads the older key
  spelling; copy the one from the tag (`git show v0.9.3:.env.example`). A
  config the old build cannot parse sends it to defaults, which usually shows
  up much later as an empty network list.
- **A zero balance is normal here.** Without a local Dash Core testnet node,
  v0.9.3 shows no funds and an empty `utxos` table — it has no SPV. That does
  not block this scenario: nothing in it spends anything.
- **The window opens at 800×600 and clips controls.** Resize before
  interacting; the import screen's later steps and the Save button sit well
  below the fold.
- **The seed phrase is one field per word.** A single paste into the first
  field yields an invalid phrase, not a filled form.
- **DPNS names can lag the identity load.** The identity appears first and the
  name arrives with the document fetch. Quitting in that window captures an
  identity with no names — a fixture that then "survives" every upgrade while
  proving nothing about DPNS.
- **Shared testnet state expires.** A network reset can remove the identity and
  free its name for someone else. Re-verify with the current build before every
  capture, not once when the fixture was designed.
- **A shared Cargo target directory is irrelevant here, but the downloaded zip
  is not.** Keep the extracted v0.9.3 binary inside the scratch tree and
  `sha256sum` it into the report — "the released v0.9.3 build" is a claim the
  fixture's whole value rests on.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
