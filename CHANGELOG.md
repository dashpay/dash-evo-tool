# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Security

- **Dependency advisory GHSA-4w2j-m93h-cj5j cleared**: the `quinn-proto` entry in
  the lock file moves from 0.11.14 to 0.11.15, which fixes a remote
  memory-exhaustion issue in out-of-order stream reassembly. The crate is an
  inert optional entry that no build of this app actually links, so this is
  dependency hygiene rather than a fix for reachable behavior.

- **Dependency advisory GHSA-7gcf-g7xr-8hxj still open** (`serde_with` below
  3.21.0, a panic when serializing empty key-value map entries): it cannot be
  resolved in this repository. `serde_with` 2.x is required by
  `dashcore-rpc-json`, which arrives through pinned revisions of
  `dashpay/platform` and `dashpay/rust-dashcore`; both still declare
  `serde_with = "2.1.0"` at their current development heads as of 2026-07-27.
  Allowing 3.x needs an upstream change in `dashpay/rust-dashcore` first. A TODO
  in `Cargo.toml` marks the re-check.

### Added

- **Keys saved on this device but not on the identity's key lists are now
  listed**: a key can be saved here while appearing on none of the identity's
  key lists — for example when adding it to the network did not finish. The
  identity's key list now shows such keys in their own section, so they can be
  opened and their saved private key removed. Previously nothing could reach
  them, even when a message asked exactly that.

- **Restore keys an upgrade left behind**: an identity that was already in the
  app before the update — a masternode loaded from its ProTxHash, or one that
  held only some of its keys — kept its remaining keys in the previous
  version's data with no way to reach them. Its page now offers to bring them
  across: the node detail page and the Key Info screen list what can be
  restored, named by role, and restore only what you press Restore on. Nothing
  happens at launch or during the update, keys already saved are never replaced
  or removed, and on a password-protected identity the identity password is
  asked for first — cancelling or mistyping it leaves everything as it was. A
  saved key that no longer matches one this identity uses — a key rotated or
  retired since the previous version saved it — is listed with an explanation
  instead of being restored, so a key that could not sign can never make the app
  report a role as held. The same applies to a key held on a separate voting or
  operator identity that this identity does not currently link to: nothing
  outside the old data says that key is still in use, so it is listed with its
  explanation and entering it by hand stays the way to bring it back. A
  masternode loaded from its ProTxHash alone is exactly that case — its owner
  and payout keys come back, its voting key is listed as one that cannot be, and
  checking that key against the chain instead is tracked as issue #942. The
  previous version's data is only ever read, so this is safe to repeat.

- **Legacy key recovery: closed edge cases found during review**: a recovered
  key is now checked against the exact key it's meant to replace, not just
  matching key data found anywhere else on the identity, so a rotated-away or
  mismatched key can no longer be reported as restored. Restoring no longer
  races with other actions on the same identity happening at the same moment
  (an edit, a refresh, a rename, or turning password protection on or off),
  and a restore still waiting on your password can no longer be reset —
  showing the Restore button again as if nothing had started — by an
  unrelated error appearing on screen.

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

- **Masternodes and evonodes are now recorded in the wallet store too**: a node
  loaded from its ProTxHash was only ever written to this app's own records, so
  the wallet store the app shares with the rest of the wallet stack had no entry
  for it at all. Such a node is now recorded there as belonging to no wallet,
  which is what it is — nodes already on this device are added the next time the
  app starts, with nothing to press. Recording it under one of your wallets was
  not an option: removing that wallet would then have deleted the node along
  with it, keys and alias included.

- **Wallet data no longer lives inside the deletable network-cache folder**:
  each network's wallet database used to sit inside the same folder as the
  temporary blockchain sync cache, so clearing or losing that cache folder
  could take real wallet, identity, and key data down with it. Each network's
  wallet database now lives in its own file alongside the app's other
  permanent data, completely separate from the disposable cache.

- **Funding an identity you don't own could silently corrupt wallet data, or
  misdirect a later top-up**: paying Platform credits into an identity that
  belongs to a different wallet on this device could register that identity
  under the paying wallet by mistake. Restarting the app afterward could then
  fail to open a wallet with "Saved wallet data appears damaged and cannot be
  loaded," and — separately — a later top-up of the paying wallet's own
  identity at the same position could be misdirected to the wrong identity
  entirely. Funding another wallet's identity now completes without touching
  the paying wallet's own identity records.

- **One damaged payment record no longer makes every wallet unopenable**: all
  wallets are kept in a single file, and one unreadable payment record in it
  stopped that whole file from opening — every wallet it held, funded ones
  included, reported "Saved wallet data appears damaged and cannot be loaded."
  Such a record is now skipped so the remaining wallets open normally, and the
  records that caused it are no longer written in the first place. Wallets
  already affected open again after updating, with their funds intact and no
  need to restore from a recovery phrase. Amounts are still checked strictly:
  a damaged record that carries a balance continues to stop the file from
  opening rather than show a total that is quietly too low.

- **"Max" now matches what your Core wallet can actually send**: pressing
  "Max" when shielding DASH, funding a Platform address through the Simple
  builder-driven form, sending directly to an identity, or funding an identity
  (creating or topping up, from your wallet balance or a received deposit)
  could suggest an amount larger than the wallet could actually send, so the
  transaction was rejected no matter how you adjusted it. Max and the amount
  check now ask the wallet directly what it can send instead of estimating from
  an on-screen balance, and both reserve room for the fee. The two derive from
  the same wallet answer: if your spendable funds change after that answer,
  Max steps back to "Checking the available amount…" and the amount check
  waits for a fresh answer instead of accepting an outdated ceiling. The
  Advanced manual-input Platform-address flow remains governed by
  the Core inputs the user selects rather than this builder ceiling. Funding
  from a received deposit is also now capped by what actually arrived at that
  deposit address, never by unrelated funds elsewhere in the wallet. While the
  check is running, the amount field shows "Checking the available amount…";
  if it fails, "The available amount could not be checked." appears with a
  "Retry available amount check" button, and you can still switch to a
  different funding method at any point.

- **A key held in the clear is used without asking for a password**: for a key
  an earlier version had saved in two places, one of them password-protected,
  using the key could bring up a password prompt even though a copy needing no
  password was on this device — and dismissing that prompt then refused the
  key outright. The copy that needs no password is now used first, so the
  prompt only appears when it is genuinely required.

- **Show and Sign find a key whose first copy is unreadable**: for a key saved
  in two places where only one copy's stored bytes were still present —
  as after restoring the app's data without its key store — "Show private
  key" and "Sign" could fail on the empty copy while the readable one sat
  unused. Both now reach whichever copy is actually readable.

- **Showing or signing with a key no longer advises saving it**: pressing
  "Show private key" or "Sign" on a key whose place on the identity could not
  be worked out answered with advice about saving the key again — about a key
  the user never entered. Both messages now name a step either situation can
  take: refresh the identity and open the key again.

- **Cancelling a password request is taken as an answer**: for a key an earlier
  version had saved in more than one place, dismissing the password prompt
  brought up the same prompt again for the same key. Cancelling now ends the
  attempt, and the message reflects the cancellation rather than an unrelated
  earlier problem with another copy of the key.

- **Messages about a key that cannot be used now say what to do**: being told a
  key is not saved on this device, or cannot be saved here, left nowhere to go
  next — or worse, named a step that could not work, such as freeing disk space
  when the identity's keys are password-protected, or entering a key the same
  screen would then refuse. Each of these messages now names the step that
  actually resolves its situation.

- **A key that could not be saved no longer looks saved**: when entering a
  private key was refused — including when saving it to this device failed —
  the key's page still showed it as saved until the page was left and reopened,
  offering to reveal it, to sign with it, and to remove it, none of which could
  work. The page now reports a refused key as not saved, which is what it is;
  likewise, a removal that could not be saved no longer shows the key as
  already gone.

- **The identities list sees a key saved by an earlier version**: the Keys
  popup on the identities list showed such a key as not saved on this device —
  even though it is — and opened the key's page in the same wrong state. The
  popup now finds a saved key wherever the version that saved it filed it, as
  the rest of the app already does.

- **A key's wallet is found even when the key is filed twice**: a key that an
  earlier version had saved in two places, wallet-derived in only one of them,
  was treated as belonging to no wallet at all — so the wallet was never offered
  for unlocking and signing with that key could not proceed. The wallet that
  derives a key is now found wherever the key is filed.

- **A key two lists appear to share can be saved again**: when a masternode's
  own record and its voting identity each carried a key with the same number and
  the same public key, entering the private key of either was refused with a
  message saying the key does not belong to this identity — although it plainly
  does, and what the two keys are for is what tells them apart. Such a key is
  now saved where it belongs. When a key really is on two lists at once, the
  message now says so and what to do about it.

- **Entering a key can no longer erase a different one**: keys of a masternode's
  own record and of its voting identity are numbered separately, so two
  different keys can carry the same number. Entering the private key of one of
  them used to take the other's place without a word, and the replaced key's
  private half was gone — with no copy to restore it from if it had been
  imported by hand. Dash Evo Tool now refuses that and explains what happened,
  leaving the saved key untouched. Re-entering a key you already saved still
  replaces itself, as before.

- **A saved voting key can now actually sign**: a voting key held on an
  identity's own record — rather than on a separate voting identity — was saved
  and shown as being on this device, but nothing could use it. Signing looked for
  it in the wrong place, so voting with it failed and the key's page reported it
  missing, on the screen whose job is to answer that question. Dash Evo Tool now
  finds a key by matching it against the key itself, wherever it is filed, so it
  is found whichever version of the app saved it and no key material has to be
  moved to fix this. This also means a key is no longer confused with a different
  key that happens to share its number, which a masternode has whenever its
  voting identity numbers a key the same way as its main identity: removing one
  key could remove the other's private half, and a key could be reported as
  saved on the strength of an unrelated key being present.

- **Removing a key now removes all of it**: "Remove private key" on a key's page
  also erases the copy of that key held in this device's secure storage.
  Previously only the entry naming it was cleared, so the key itself stayed
  behind with nothing pointing at it — it could not be used or brought back, and
  deleting the whole identity afterwards did not clear it either. If the secure
  storage cannot be written to, the removal now stops and says so with the key
  left exactly as it was, so it can simply be tried again.

- **A key is checked before it is used**: showing a saved key or signing with it
  now confirms the key held on this device really is the key on screen. Should
  the two disagree — records an older version left inconsistent — the action
  stops and says so, rather than signing with a key nobody would recognise as
  this identity's.

- **An identity's keys are reachable again**: the keys list under an identity's
  Settings → Advanced now opens each key's own page, so keys can be inspected
  and restored — and, once a key is on this device, signed with or
  password-protected — without changing the interface mode and without starting
  a payment. Previously that list was a read-only table with no way onward, and
  every route to a key's page ran through an action screen — sending,
  withdrawing, a token operation — each of which offers it only when the
  identity already holds a key of the kind that action needs. So an identity
  missing its keys, the one case where this matters most, could not get to them
  at all. The offer to restore keys left behind by an earlier version now also
  appears on the keys list itself, above the keys, rather than only inside a
  key's page. Each key is named by its role and states whether it is saved on
  this device. Keys are named for the identity they belong to: a user identity's
  keys are described in plain language rather than in masternode registration
  terms, which previously appeared on every identity. Leaving a key returns to
  the list with both its keys and the restore offer brought up to date, so a
  restore made from a key's page is reflected immediately instead of being
  offered again.

  A key opened from a masternode's page keeps its name too. A voting key is the
  node's voting key however it is recorded, and its own page now says so instead
  of describing it as another kind of key, which also means the page no longer
  reports such a key as missing while the list it was opened from shows it as
  saved on this device.

  One known limitation, for a voting key stored on the identity itself rather
  than on a separate voting identity: the keys list and the key's page now agree
  on whether such a key is saved here, but saving or removing one by hand can
  affect a voting key of the same number on a linked voting identity, and
  removing it may leave the original in place. So until then, after saving or
  removing a voting key on an identity like that, open the keys list and check
  that each key still reads as you expect, and re-enter any key that should be
  saved but no longer is. This will be closed by the in-progress key-placement
  resolution fix.

- **A key's page now catches up on changes made while it was open**: previously,
  if something else updated your identity while a key's page was open — most
  relevantly, a restore that finished from a different screen — the next key
  edit made on that page could silently overwrite the change. The page now
  picks up such changes as they arrive.

- **Key role names are complete, consistent phrases everywhere**: a key's role
  (owner, voting, payout, and so on) now reads the same complete phrase across
  the keys list, a masternode's page, and the key's own page, instead of a
  partly-assembled label that could vary by screen. The on-chain purpose value
  itself remains available as its own line in Expert view for anyone who wants
  it verbatim.

- **Wallet rename consistency**: renaming a wallet no longer overwrites other
  saved wallet details when metadata cannot be read. Overlapping renames and
  wallet removals also keep displayed aliases and deleted-wallet metadata
  consistent. The rename dialog remains open with the entered name available
  for retry when saving fails, and its controls stay disabled while a save is
  in progress.

- **Shielded availability notice**: now distinguishes when the connected network
  does not support shielded sending from when the current interface mode does
  not unlock it.

- **Identity Home actions simplified**: the action row previously had six
  buttons — several of which opened the same screen (`Send`/`Send to another
  identity`, `Receive`/`Add funds`). It's now one row of four: **Add funds**,
  **Send to wallet**, **Send to another identity**, and **Add contact**.
  **Send to wallet** is disabled with an explanation when the identity has no
  withdrawal key loaded. The **Add funds** screen, and its wording
  throughout, now consistently says "Add funds" instead of "Top Up Identity",
  and its step-by-step deposit messages no longer use technical wording.

- **Fewer native crashes while verifying a deposit**: background worker
  threads now get a larger stack, fixing a crash that could occur during
  deposit verification.

- **Clearer guidance for an already-used deposit**: registering an identity,
  topping one up, or funding a platform address with a deposit that was
  already consumed by another operation now tells you directly to choose a
  different deposit or start a new one, instead of the generic rejection
  message that suggested retrying the same one.

- **Shielded features now detect network support correctly**: the app's
  live check of the connected network's protocol version — used to enable
  shielded sending, receiving, and transfers — no longer gets stuck at its
  startup default. It was silently keeping shielded operations unavailable
  regardless of what the connected network actually supports, and also kept
  the send-fee estimate from picking up the network's current rate. A
  temporary workaround is in place while the underlying issue is fixed
  upstream; the send-fee estimate will keep using its last known rate until
  that lands. The check only accepts a protocol version the connected network
  actually confirms: when the network cannot be reached, shielded operations
  stay unavailable and the app keeps retrying, instead of assuming the version
  the app was built with.

- **Fewer connection failures during unrelated actions**: the app kept asking
  the network for epoch details through a request every server currently
  refuses. Each attempt consumed part of the app's shared request allowance, so
  other actions — adding funds to an identity, for example — could fail with a
  connection error. That request is now paused until the upstream fix is
  released. The send-fee rate it was meant to refresh is the standard rate every
  network charges today, so fees are unchanged, and the Platform Info screen now
  says plainly that the rate shown is fixed rather than read from the network.

### Changed

- **A funding transaction found again on the network is now labelled honestly**:
  when the app rediscovers a saved funding transaction from the chain rather
  than tracking it from the start, it can tell that the network confirmed it but
  not whether it was already spent on an identity. The funding list says exactly
  that instead of claiming it is ready to use or already used. Selecting it still
  works — the network has the final say and refuses one that was already spent.

- **Upstream wallet backend updated (`platform-wallet` / `platform-wallet-storage`)**:
  the `dashpay/platform` dependency is bumped to the PR #3968 tip
  (`d18020f` → `288a6ca`), which lands an embeddable SQLite persistence backend with
  *seedless rehydration*. The wallet manager now restores watch-only wallet state
  (accounts, balances, identities, platform addresses) from the on-disk store
  without the HD seed, re-deriving spend authority just-in-time from the seed only
  when an operation actually signs — so private key material is never left resident
  between operations. DET's shielded operations were updated to match: each now
  resolves the HD seed through the secret-seam chokepoint for the single operation
  and drops it on return. The update also adds persistence and
  rehydration for provider (masternode / evonode) platform-node key pools and for
  DashPay invitations. The later review tip also retries transient startup
  rehydration, selects platform-address transfer and withdrawal inputs from
  hydrated candidates with authoritative on-chain balances, freezes the SPV sync
  watermark when persistence fails, and persists address-reservation timestamps
  plus DashPay address used-state updates. On Unix, DET now tightens app and
  per-network storage directories to owner-only before opening the hardened
  upstream database, so permissive system defaults do not prevent startup. New
  wallet passwords must now be at least eight UTF-8 bytes after trimming
  (measured in bytes, not characters, so a 4-character non-ASCII password like
  `öäüß` — 8 bytes — is accepted); existing wallets with shorter passwords
  that are still in DET's legacy encrypted format remain usable instead of
  failing during lazy migration. Protected (Tier-2) shielded wallets now resolve
  their seed just in time for every operation that spends or binds their Orchard
  keys (initialization, shield from Core, shield from Platform, transfer,
  unshield, and withdraw). Each operation prompts for the passphrase unless the
  user explicitly keeps the wallet unlocked for the session, replacing the
  previous implicit once-per-session reuse.

  **Compatibility note:** wallets already migrated to Tier-2 storage by a
  July 2026 weekly build with a shorter password cannot be opened at this
  upstream tip; do not upgrade those profiles until upstream provides a
  compatibility reader.

  Development builds between `f7ca95f` and `69b7546` stored shielded viewing
  keys in an interim metadata row that this final pin does not migrate; those
  commits were never released, and unlocking the wallet safely re-derives and
  persists the same viewing key in upstream's native table.
  SPV broadcasts are now pure peer-to-peer and report success only after peer
  echo, InstantSend, or confirmation; an ambiguous outcome keeps the input
  reservation until sync or its TTL reconciles it. Adding a DashPay contact
  receiving account also invalidates prior compact-filter coverage, preventing
  an in-flight scan from certifying a wallet account set it did not scan.
  Transitively, the pinned dashpay git dependencies advance with it:
  `rust-dashcore` (`be6e776` → `18c68d4`, which lands the reserve-on-hand-out
  receive-address APIs and SPV acceptance tracking), `grovedb` (`v5.0.0` →
  `v5.0.1`), and the `orchard`
  shielded-crypto fork (`dashified-0.14.0` → `dashified-0.14.1`); no crates.io
  dependencies change.

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
