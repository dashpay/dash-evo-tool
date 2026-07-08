# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **Search tags in the "Send to" field**: type `type:core`, `type:platform`,
  `type:shielded`, or `wallet:<name>` to narrow the address suggestions
  instead of scrolling through everything; plain words still search like
  before. Each suggestion now shows a small label for its wallet and its
  type, and the field shows a hint listing the tags you can use when it's
  empty.
### Changed

- **Wallet balance breakdown is single-sourced**: the per-account tabs and the
  wallet header now derive every balance from one place. The Core header total
  and the Core per-account breakdown are read from the same generation of synced
  wallet data — even while the wallet is busy syncing, the header and the tabs
  are never spliced from different moments — and the Platform tab shows the exact
  same Platform total as the header. Any funds held on addresses outside your
  main and Platform accounts now always appear in a visible tab, so the tabs
  always add up to the header total. This removes an earlier internal cross-check
  (and its occasional "balances didn't fully add up" warning), no longer needed
  now that the figures come from one consistent source.

- **Platform tab shows immediately**: a newly created or imported wallet now
  shows its Platform tab (empty until funded) as soon as it loads, instead of
  waiting for the first Platform-address sync to complete. Your Platform receive
  address is reachable right away, even before or without a network sync.

- **Sign-time wallet unlock**: the passphrase is now requested just-in-time, the
  moment an operation actually needs your secret (sending funds, registering an
  identity, signing). The prompt offers an optional "Keep this wallet unlocked
  until I close the app" checkbox so a busy session asks only once. This replaces
  the old upfront unlock gate that held the wallet open for the whole session —
  the HD seed is no longer kept in memory between operations; it is decrypted on
  demand and wiped as soon as the operation finishes.

- **Wallet storage backend**: HD wallet seeds and single-key private keys are now
  stored in an upstream `platform-wallet-storage` encrypted vault
  (`secrets/det-secrets.pwsvault` in the app data directory) rather than in the legacy
  `data.db` SQLite database. Wallet metadata (alias, main flag,
  Core wallet name) moves to a new `det-app.sqlite` key-value sidecar. The legacy
  `data.db` file is left intact for safety; it is no longer read at runtime.

- **Cold-start migration**: on the first launch after upgrading, DET automatically
  migrates wallet seeds, metadata, and imported single-key data from `data.db` into
  the new storage layout. A progress banner is shown during migration (typically
  under one second on local storage). The migration is idempotent — subsequent
  launches skip it via a completion sentinel in `det-app.sqlite`.

- **Identity funding now goes through one spend engine**: registering or topping up
  an identity always funds from your wallet balance through the upstream
  asset-lock engine, which selects coins and tracks the lock to confirmation. The
  separate "fund directly from a specific transaction output" path was removed so
  there is a single, double-spend-safe funding flow.

- **DashPay profile no longer requires a display name**: you can save your profile
  with the display name left blank; only the length limits on name, bio, and
  avatar URL are still enforced.

- **Shield, Send Privately, and Unshield are now one screen**: each of these
  actions opens the same Send screen, already set up for what you clicked, instead
  of a separate dedicated screen per action. The steps and options are unchanged,
  including pasting a raw shielded address in hex form — there are just fewer
  screens to navigate between.

### Known Limitations

- **Single-key wallets — send and balance refresh not available**: importing a
  single-key wallet (WIF), viewing it, and signing with it all work in this
  release. Sending funds and refreshing the balance or UTXO list are not yet
  supported. Your key data is preserved and these actions will be available in a
  future update. To send funds now, use an HD (recovery-phrase) wallet.

- **DashPay contacts — non-mainnet / non-account-0 legacy addresses**: this
  release drops back-compat for contact-request addresses derived outside mainnet
  account 0 under the old DIP-14 scheme (non-mainnet networks, or secondary
  account indices). If you used DashPay on testnet or devnet with a non-default
  account, existing contact payment addresses for those contacts may not be
  reproduced. Re-establishing the contact from both sides restores full
  functionality.

### Removed

- Proof log screen (internal developer tool, not part of the public feature set).
- QR-code wallet import flow for identity funding and top-up screens.
- The "fund identity directly from a transaction output" option on the identity
  registration and top-up screens (replaced by the single asset-lock funding flow
  described under Changed).
- The unused "Memo (optional)" field on the wallet send dialog and the single-key
  send screen — the note was never attached to the transaction, so it has been
  removed to avoid implying a memo would be saved.
- The "Core Only" wallet refresh option. Core wallet balances and UTXOs now stay
  current automatically, so a manual Core-only refresh had nothing to do; refresh
  now covers Core plus Platform, or Platform only.
- The unused ZMQ Core-event listener subsystem and the non-functional "Disable ZMQ"
  setting. The listener was already gated off and never delivered events, so the
  toggle did nothing; both have been removed (the `zmq`, `zeromq`, and
  `crossbeam-channel` dependencies are no longer needed).
- The unreachable Dash-Qt launcher and its settings — the executable path, the
  overwrite-config option, and the close-on-exit option. There was no way to launch
  Dash-Qt from the app, so the controls had no effect and have been removed.

### Fixed

- `WalletBackend` is now initialised eagerly at `AppState` start, eliminating a
  retry-loop spam on the SDK connection during cold boot.
- Wallet store is rehydrated on cold start, resolving a regression where wallets
  were not visible after the storage migration.
- The Disconnect button on the network settings screen now actually disconnects:
  it stops the wallet backend and updates the connection indicator instead of
  silently doing nothing. A fast double-click can no longer start two disconnects
  at once.
- Shielded balances no longer overstate after upgrading or after using
  "Resync Notes": the spent-note scan cursor is reset so previously spent notes
  are detected again. Previously a migrated or resynced wallet could show notes as
  available that had already been spent, causing later spends to fail.
- The unused-asset-lock picker on the identity registration and top-up screens now
  shows a plain-language status and the funding address for each lock, instead of
  an internal status name, so you can tell which lock is which.
- A failed wallet-funded identity registration now tells you that your funds are
  safe as a funding lock and how to finish: start a new identity and fund it from
  your existing asset lock.
- Platform and identity features stay reachable during initial sync. Previously,
  on a fresh connection the app contacted Platform network nodes before its local
  masternode list had finished syncing; every node it tried was wrongly marked as
  failed and set aside, and once all of them were set aside Platform stopped
  working until restart. The app now waits for the masternode list to be ready
  before contacting those nodes.
- Silent crashes now leave a trace: the app captures stderr output and fatal
  signals to its log file, so an unexpected exit can be diagnosed from the logs
  instead of vanishing without a record.
- Withdrawing from a Platform address to a Core address could fail with an
  internal error even though the address's balance was clearly visible in the
  withdrawal picker. A Platform address discovered by background syncing was
  not always recognized as one the wallet could sign for. The wallet now
  reconciles this automatically, so a visible balance can always be withdrawn.
  No funds were ever at risk — the withdrawal simply failed before anything
  was sent.
- If your wallet's storage was ever unusually slow to finish preparing (e.g.
  after a network switch or on a cold start), the app could wait forever with
  no indication anything was wrong. It now tells you after 30 seconds and
  suggests restarting, instead of leaving the wallet silently invisible.
- The wallet screen's overall balance could show more Dash than its Core or
  Platform account tabs added up to, especially on wallets that have handed
  out many addresses. Funds on addresses the wallet had not yet finished
  indexing into an account tab were counted in the total but missing from the
  per-account breakdown and address list. All known funds now appear in their
  correct account tab.
- Important messages — a saved wallet that couldn't be reopened, a failed
  send or identity operation, a lost connection — could disappear on their
  own after a few seconds, before you had a chance to read them. These now
  stay on screen until you dismiss them yourself. Routine notices (a
  successful action, a validation hint, an in-progress status) still clear
  automatically as before.
- Choosing a funding method on the identity registration or top-up screen and
  then switching wallets no longer silently discards your choice and reverts
  to the default funding method.
- The top-up screen's automatic wallet selection no longer pre-selects a
  wallet whose spendable balance is too small or currently locked to actually
  cover the top-up, which would then be immediately rejected.
- The "My Tokens" tab no longer gets stuck on a loading spinner when you have
  no identities yet; it now shows the expected empty state.
- Two settings changes made in quick succession could occasionally cause one
  of them to be silently lost. Saving settings is now a single atomic step,
  so no change is dropped.
