# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **Automatic Platform node refresh during upgrades**: migrating a pre-1.0
  installation now triggers a best-effort Mainnet or Testnet node refresh.
  Failed attempts retry on later launches until fresh addresses are saved and
  the app reconnects, so upgrading users do not need to find the manual action.

- **Search tags in the "Send to" field**: type `type:core`, `type:platform`,
  `type:shielded`, or `wallet:<name>` to narrow the address suggestions
  instead of scrolling through everything; plain words still search like
  before. Each suggestion now shows a small label for its wallet and its
  type, and the field shows a hint listing the tags you can use when it's
  empty.

- **Masternodes tab**: a new "Masternodes" entry in the left nav (visible when
  Expert mode is on) for loading and managing masternode and evonode (HP
  masternode) identities by ProTxHash. Loaded nodes appear as a card list
  showing type, voter-key readiness, key status, and DPNS-voting status;
  opening a card shows a detail view with inline DPNS contested-name voting,
  Withdraw / Top up / Transfer actions, key management, and — for evonodes
  only — a link to claim token rewards. The load form accepts an optional
  password to encrypt the entered voting/owner/payout keys immediately
  instead of only after a separate step; leaving it blank keeps today's
  behavior, and protection can always be added later from the key screen.
  This replaces loading a masternode or evonode from *Identities → Load
  Existing Identity → Show Advanced Options*, which no longer offers those
  identity types.

- **Wallet/identity indicator on more screens (rollout in progress)**: the
  wallet and identity picker previously shown only at the top of the Identity
  Hub now also appears at the top of the Identities, DashPay, DPNS, and
  Wallets screens. On the Identity Hub and the new Masternodes tab it's fully
  interactive — you can change which wallet or identity you're acting as
  right there. On the other four it's currently a read-only preview of your
  active wallet/identity, with a tooltip on where to change it; making it
  interactive there, and adding it to the remaining screens, is tracked as a
  follow-up.

### Fixed

- **Shielded availability notice**: now distinguishes when the connected network
  does not support shielded sending from when the current interface mode does
  not unlock it.

### Changed

- **Shielded transactions are available on supported networks**: sending,
  receiving, shielding, and unshielding are enabled when the connected network's
  protocol version supports them, including mainnet. These operations were
  previously gated off everywhere pending upstream activation.

- **The first launch after an upgrade asks for each password-protected wallet's
  password**: the app moves your wallets into a new storage format on that first
  launch, and it needs each protected wallet's password to finish the move for
  that wallet. You are asked once per wallet, one at a time. If you don't have a
  password to hand, you can skip that wallet: it stays locked, no coins are lost,
  and its move finishes the next time you unlock it with its password. The
  password is used for the update and is not kept unlocked afterwards.

- **The previous version's database is kept on this device as a read-only
  recovery copy**: it is never written to, and it is no longer erased by "Clear
  Database" or "Remove Wallet". Those actions remove the data *this* version
  uses; the older recovery database remains and may still contain wallet recovery
  data, which both confirmation dialogs now say before you confirm. Because that
  database is read-only, the "Clear Platform Addresses" developer tool is
  unavailable, and says why.

- **Masternode and evonode identities no longer appear in the Identity Hub or
  Identities picker**: they now live exclusively on the new Masternodes tab,
  so you're never offered actions (like registering a username) that don't
  apply to a node's collateral/voting identity.

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

- **Expert mode replaced by three interface levels**: the single Expert-mode
  toggle is now a three-way choice — **Default view**, **Detailed view**, and
  **Developer tools** — set from Network Settings or during first-time setup.
  Detailed view unlocks everything the old Expert mode did (account
  breakdowns, address tables, masternode tools); a few of the most advanced
  actions (state-transition signing overrides, proceeding without a locally
  held key, clearing Platform addresses) now require the new Developer tools
  level. Until you pick a level, the app starts in Detailed view, so nothing
  the old Expert mode showed you is hidden. The choice is remembered and can
  be changed anytime, and it is made in the app — the `DEVELOPER_MODE` entry
  in `.env` no longer has any effect on it. See
  [User Roles](docs/user-roles.md) for details.

- **Welcome screen's experience-level picker is now three cards**: it matches
  the visual style of the Create Wallet / Import Wallet / Just Explore cards
  below it. Each role card adds an icon and a short description, and the one
  you're on gets a highlighted border. Same three levels, same behavior — just
  easier to compare at a glance.

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

- **Shielded notes — no per-note detail view**: the Shielded tab no longer lists
  individual notes (value, block height, spent/unspent status) or a synced-index
  and note-count summary. Your shielded balance total is still accurate; only the
  note-level breakdown is unavailable in this release.

### Removed

- Proof log screen (internal developer tool, not part of the public feature set).
  Proof-log records now go only to the `tracing` log target — both the persisted
  history and the in-app viewer are gone; there is no replacement UI to inspect
  past entries.
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
- The Masternode List Diff inspector (Tools), which showed additions, removals, and
  changes to the masternode list between blocks. No replacement is planned.
- The "Total Received (DASH)" column on the wallet address table. There is no
  upstream source for cumulative historical receipts post-migration, so the column
  cannot be populated.

### Fixed

- **Closing the app now finishes wallet activity and clears in-memory secrets**:
  wallet cleanup still completes if you close the app again while it is already
  closing or if a network change is still connecting.

- **Token balance refresh status**: requesting a refresh while token balances
  are already updating now shows a brief informational note instead of a red
  error banner that must be dismissed.

- **Topping up an identity from more than one funding wallet no longer gets
  stuck loading**: requesting a new deposit address at the same time as
  refreshing your asset-lock transactions could silently drop the refresh,
  leaving those wallets stuck showing "Loading" for the rest of the session.
  Both requests are now sent together.

- **A single damaged transaction record no longer hides an entire wallet**:
  previously, one unreadable entry in a wallet's transaction history could
  make the wallet and its balance disappear from every screen until the
  underlying data was manually repaired. The app now skips the damaged entry
  and keeps the wallet visible, with the rest of its history intact.

- **Scheduled DPNS vote sweeps no longer get stuck after an unexpected error**:
  an internal failure while casting due votes could permanently block future
  vote sweeps for that network until the app was restarted. It now recovers
  on its own.

- **Send confirmations now show what the recipient actually receives**: when
  the network fee is deducted from the amount you entered, the confirmation
  dialog now says so and shows the reduced amount the recipient will get,
  instead of implying they receive the full entered amount.

- **A missed deposit no longer leaves the funding screen waiting forever**:
  if the app missed the notification that your deposit arrived (for example
  because you were on another screen), it now also checks your wallet
  balance directly, so the funding step advances even if the one-time
  notification was missed.

- **Deposit-address screens no longer strand you after an address error**: if
  generating a new receive address failed, the screen used to reset to a bare
  view with no way to retry. It now keeps the retry button available.

- **A network preference that can't be restored no longer defaults you to
  Mainnet**: if the app can't confirm your previous network selection during
  an upgrade, it now asks you to choose a network explicitly instead of
  silently starting the session on Mainnet — important if you were previously
  using Testnet.

- **Submitted Platform actions are no longer reported as rejected when only
  confirmation failed**: if a state transition was broadcast but its result
  could not be confirmed, the app now tells you to check whether it completed
  before trying again instead of showing an unsafe rejection-and-retry message.

- **Shielded actions now say when they are unavailable instead of failing
  obscurely**: if shielding, sending, or withdrawing shielded funds is not
  available on your network yet, the app says so and points you at a regular
  payment, rather than starting the action and failing part-way through.

- **DashPay contact details and request actions are protected from accidental loss or duplicate
  fees**: declining, cancelling, unhiding, or renaming a contact now preserves every unrelated
  encrypted detail. If another client saved details this app cannot read, the app offers a clear,
  confirmed replacement path instead of silently erasing them or leaving the contact permanently
  hidden. Switching Identity Hub tabs also keeps paid request actions disabled until their original
  task finishes.

- **Your settings and scheduled votes now survive an upgrade**: upgrading from an
  earlier version no longer starts the app with a blank configuration. The first
  launch after the upgrade brings across your selected network, start screen,
  theme, onboarding state, Dash-Qt path and the remaining toggles — so a testnet
  user is no longer relaunched on Mainnet — along with your scheduled DPNS votes
  (choice, time and already-cast state) and your identities' top-up history.
  Scheduled votes are imported even on an install whose wallets were already
  moved by a previous launch. If a scheduled vote cannot be read, the app says so
  in a banner with a "Retry now" action instead of dropping it silently; the
  original data is never deleted from the previous version's storage.

- **Expert mode now reveals the Masternodes tab without a restart**: turning on
  Expert mode in Settings immediately shows the "Masternodes" entry in the left
  nav. Previously the Expert-mode flag was stored separately per network, so the
  nav entry could stay hidden (reading a stale value on whichever network context
  the app was showing) until the app was restarted. Expert mode is now a single
  app-wide flag shared across all networks.

- **Clearer error when loading a masternode by an unknown ProTxHash**: entering a
  valid-looking but unregistered ProTxHash in the masternode load form now says no
  masternode or evonode was found for that ProTxHash, instead of the misleading
  generic "Identity not found — check the ID or name" message.

- **Withdrawal key selection**: the Withdraw screen now pre-selects only a key
  whose private key you actually hold (a payout/Transfer key preferred, Owner as
  fallback). Previously it could pick a key that exists on the identity but whose
  private key isn't loaded locally — common on loaded masternode/evonode
  identities where only the Owner key was supplied — which made the withdrawal
  fail at signing with an unhelpful technical error. When no usable key is
  loaded, the screen guides you to add one instead of failing mid-withdrawal.
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
- A burst of unreachable Dash network servers could show a message that
  understated the outage as a problem reaching one server, even once the app's
  own sync was otherwise healthy. This happens when every server the app
  currently knows about becomes briefly unreachable at once — a temporary,
  self-recovering condition. The app now recognizes it and says "All Dash network
  servers are temporarily unreachable. Please wait a minute and retry." instead.
- The onboarding Welcome screen on first launch no longer shows a red
  "Disconnected — check your internet connection" banner before you have done
  anything. On a fresh start there is no wallet yet and no sync has been
  attempted, so that message was misleading; it now stays hidden until you
  finish onboarding, and real connection problems are still reported afterwards.
