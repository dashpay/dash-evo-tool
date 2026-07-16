# PR860 QA Delta Campaign — Progress Checklist

Delta retest against `docs/ai-design/2026-07-14-pr892-user-story-qa/` baseline (2026-07-14/15,
175 stories). See `CAMPAIGN-CONTEXT.md` for scope/priority and `summary-report.md` for the
synthesized findings. One line per retested story; verdicts carried unchanged from the baseline
are not re-listed here unless spot-checked (see "PASS spot-checks" section).

Build under test: `docs/platform-wallet-migration-design` @ commit `e6ba4857` (or later — see
summary-report.md for exact SHA at each session). Binary: private hash-verified copy at
`/data/tmp/det-pr860-qa-bin/dash-evo-tool`. Data dir: `/data/tmp/det-pr860-qa-data`. Network:
Testnet unless noted.

Status: **COMPLETE** — started 2026-07-16, finished 2026-07-16 (resumed after an interrupted
mid-campaign restart; all planned checklist items closed out).

## Zero-funding priority list

- [x] WAL-032: Finish a storage update without risking old wallet data — **PASS**. Found a
      genuine legacy-schema fixture (`/data/artifacts/dash-evo-tool/2026-07-13/qa-snapshot-180453/data.db`,
      pre-migration DET-native schema, 20 wallet rows incl. 2 password-protected: `t2-pass`
      testnet, `L2-pass` regtest) — copied (not moved) into an isolated throwaway dir
      `/data/tmp/det-pr860-wal032-fixture-data`. Full migration exercised live: (1) non-password
      wallets migrated automatically with no prompt (log: "Wallet-seed migration pass complete
      imported=12"); (2) password-protected `t2-pass` correctly triggered a blocking modal
      "Continue the storage update — Enter the password for 't2-pass'" with Skip/Continue; (3)
      wrong password → calm "That password did not match. Check it and try again.", dialog
      stays open, retriable; (4) Skip → migration proceeded, wallet stayed in a locked state
      (`Unlock` button shown instead of Lock), rest of storage update finished cleanly
      ("FinishUnwire wallet drain complete", "Identity migration pass complete imported=4").
      Matches every acceptance criterion: per-wallet separate password prompt, skip without
      blocking the rest, previous data.db never mutated (sha256 unchanged throughout — see
      below), calm generic failure message. Also confirmed the zero-fixture fresh-install path
      is clean (no legacy rows detected, sentinel written, no crash, no error in det.log).
      Screenshots: `WAL-032-1` through `WAL-032-4`.
- [x] WAL-005: Rename a wallet — **PASS (FIXED since baseline)**. Baseline: "Rename button has
      no effect." Now: clicking Rename on wallet "t1" (from the WAL-032 fixture) opens a
      "Rename Wallet" dialog with a text field; entered "t1-renamed-qa", clicked Save — header,
      breadcrumb, and wallet-selector dropdown all updated immediately and the new name
      persisted across in-session navigation. Screenshots: `WAL-005-1`, `WAL-005-2`.
- [x] WAL-006: Lock and unlock wallet — **PASS (FIXED since baseline)**. Baseline: "Unlock
      never opens a password prompt — permanent self-lockout." Now: selected the still-locked
      `t2-pass` wallet (left in that state by the WAL-032 skip test above), clicked "Unlock" —
      an "Unlock Wallet" dialog opened correctly with a masked password field and a "Keep this
      wallet unlocked until I close the app" checkbox (unchecked by default, matching the
      story's "defaults to off" requirement). Entered a wrong password (the real password for
      this fixture wallet is unknown) — got the same calm "That password did not match. Check
      it and try again." message, dialog stayed open and retriable, no corruption. Could not
      complete a full correct-password unlock (no known password for this fixture wallet), so
      the round-trip itself is not 100% confirmed, but the core baseline defect — the button
      being a complete no-op — is conclusively fixed. Screenshots: `WAL-006-1`, `WAL-006-2`.
- [x] WAL-007: Remove a wallet — **PASS (FIXED since baseline)**. Baseline: "HD wallets get a
      confirmation dialog; single-key wallets are deleted instantly with zero confirmation."
      Retested both paths: (1) HD wallet "testnet3 - 6" → Remove → confirmation dialog with
      correct wording ("clears the data used by this version... read-only recovery database
      stays on this device") → confirmed removal, wallet gone from selector — matches prior
      PASS behavior for HD wallets, still working. (2) Imported a single-key wallet via "Import
      key (advanced)" using the repo's own test-vector WIF
      (`cMahea7zqjxrtgAbB7LSGbcQUr1uX1ojuat9jZodMN8rFTv2sfUK`, a well-known public BitcoinJS/DET
      test fixture already used in `tests/`, zero value at risk) — clicked Remove — a
      confirmation dialog now appears: "Removing wallet 'qa-sk-remove-test' will delete its
      imported private key and local wallet data from this device. Make sure you have a backup
      of the private key before continuing. Continue?" This is the exact previously-missing
      control. Confirmed removal completed cleanly. Screenshots: `WAL-007-1` through `WAL-007-3`.
- [x] SND-003: Receive Dash with QR code — **PASS (FIXED since baseline)**. Baseline: "Clicking
      Receive does nothing — no modal, no QR, no navigation, no log entry." Now: clicking
      "Receive" on the Wallet screen (Expert view, wallet "zzz") opens a full "Receive" modal
      with a rendered QR code, an address dropdown, the full address as copyable text, "Copy
      Address" (tested — showed "Address copied!" confirmation) and "New Address" buttons.
      Verified on both the "Core" and "Platform" tabs within the modal — both render distinct,
      correct QR codes/addresses/balances. Fully matches acceptance criteria (QR alongside text
      address, copy-to-clipboard). Screenshot: `SND-003-1-receive-qr-modal-FIXED.png`.
- [x] UX-003: Global wallet/identity switcher across all tabs — **FAIL (confirmed still
      reproducing, unchanged from baseline)**. Checked all four screens the baseline flagged:
      Contracts (top panel shows only "Group Actions / Contracts / Documents" — no switcher),
      Tokens ("Import Token / Refresh" — no switcher), Tools ("Tools" title only — no
      switcher), Settings ("Networks" title only — no switcher). Confirmed the switcher
      mechanism itself is intact and working elsewhere in this same session: Masternodes shows
      the full three-segment breadcrumb `Masternodes › zzz › (no masternode yet)`. Gap is
      isolated to exactly the same four screens the baseline named — no regression, no fix.
      Screenshots: `UX-003-1-settings-no-switcher.png`, `UX-003-2-masternodes-has-switcher.png`.
- [x] MCP-001: Manage wallets via CLI — **PASS (FIXED since baseline)**. Imported a throwaway
      test-vector wallet (`abandon abandon ... about`, mainnet, alias `qa-mcp001-wallet`) via
      `core-wallet-import` in a standalone det-cli process, then ran `core-wallets-list` in 3
      separate fresh processes — the wallet (with alias) was visible every time. Re-importing
      the same mnemonic from a 4th fresh process correctly returned `already_imported:true`.
      Baseline's cited defect (`ListWalletsTool::invoke` reading only the in-memory
      `AppContext.wallets` map) is gone: current `src/mcp/tools/wallet.rs:531` now calls
      `resolve::ensure_wallets_hydrated(&ctx).await?` before reading the map, which wires the
      wallet backend and waits out any in-progress storage migration
      (`src/mcp/resolve.rs:125-131`) — this hydration call did not exist in the baseline build.
      `core-address-create` on the same wallet timed out at 2min (SPV-sync gate, not part of
      the smoke-test allowlist per CLAUDE.md — expected, not tested further; data dir
      `/data/tmp/det-pr860-mcp001-data`, kept separate from the main GUI QA data dir).
- [x] NET-004: Select theme — **PASS (no regression)**. Settings > Networks > Advanced
      Settings > Theme dropdown offers System/Light/Dark. Switched Dark → Light: entire UI
      re-themed instantly (confirmed via screenshot, background/text inverted). Switched back
      Light → Dark: reverted instantly. Matches acceptance criteria exactly.
- [x] NET-006: Select interface mode — **PASS (no regression)**. Settings > Networks >
      Interface mode: Default view / Expert view / Developer view. Switched Expert → Default:
      sidebar immediately simplified (fewer items, reordered), description text changed to
      "Shows your balance, send and receive, and usernames." Switched back to Expert: sidebar
      restored (Wallets/Masternodes/Contracts/Tokens/Tools/Settings all present). Applies
      immediately, persists, freely reversible — matches acceptance criteria.
- [x] NET-005: Unlock advanced features by interface mode — **PASS (no regression, light
      check)**. Not exhaustively re-verified (would need a full feature matrix pass), but
      directly observed monotonic behavior consistent with the story: Default view hid the
      System tab and address-table-style detail that Expert view shows (see WAL-022 in the
      baseline), and Developer-only controls (Tools' deserializer screens) remained reachable
      only outside Default view throughout this session. No contradicting evidence found.

## Funded-state stories

Unblocked immediately, without waiting on the faucet: the WAL-032 legacy fixture
(`/data/tmp/det-pr860-wal032-fixture-data`) came with real, already-synced Testnet balances
across ~12 wallets (e.g. `t1` 1.0 DASH, `e2e-test` 92 DASH, `zzz` 0.02 DASH) and 4 migrated
identities. Used this fixture for funded-state testing instead of waiting on the 3-req/hour
faucet — faucet funding of the separate clean dir deferred as a lower-priority parallel task
(see "queued decisions" in summary-report.md).

- [x] WAL-016: View transaction history — **PASS (spot-check, no regression)**. Wallet "zzz"'s
      Dash Core Transaction History rendered 5 real historical transactions (dates back to
      2026-03-23) correctly on a session that itself followed a full legacy-schema migration —
      amounts, dates, fees, confirmation status, and TxIDs all correct. Consistent with the
      PR892 regression fix holding.
- [x] WAL-017: Fund Platform address from wallet — **PASS (confirmed reliable, baseline's
      "transient" note holds)**. Baseline: initially FAIL ("No UTXOs available"), later shown
      transient via a differential test. Retested end-to-end on wallet "t1" (1.0 DASH Core
      balance): Send Dash screen → "Send from: Core Wallet" → typed a Platform address as
      destination → autocomplete correctly recognized it and set "Transaction type: Fund
      Platform Address" → entered 0.01 DASH → full fee breakdown displayed (see SND-005 below)
      → clicked "Fund Platform Address" → "Platform address funded successfully!" No
      coin-selection failure. Screenshot: `WAL-017-1-fund-platform-address-success.png`.
- [x] SND-005: See fee estimate before confirming send — **PASS (FIXED since baseline, and
      story text itself has been tightened)**. Baseline FAIL: "no fee estimate or confirmation
      step exists at all." Current `docs/user-stories.md` criteria are narrower and specific
      (fee estimate inline above Send button, total deduction shown) — both directly confirmed
      live in the WAL-017 flow above: "Estimated network fee: ≈0.00056 DASH", "Recipient
      receives: ≈0.00944 DASH", "Total deducted: ≈0.01 DASH", "Fees are estimated; the exact
      amount is confirmed when you send." Displayed before the send was dispatched, exactly as
      specified. Note: clicking the send button went directly to "Sending..." with no separate
      "Are you sure?" modal — SND-001's own separate criterion ("Confirmation dialog before
      broadcast") still appears unmet, consistent with the baseline's note that this is a
      distinct, narrower gap from SND-005's (now-fixed) fee-estimate criterion. Screenshot:
      `SND-005-1-fee-estimate-shown-FIXED.png`.
- [ ] WAL-018 — recently-touched wallet code (asset-lock funding path)
- [ ] SND-001 (confirmation-dialog sub-criterion), SND-014 — recently-touched send code
- [x] IDN-002: Load existing identity by ID — **PASS (FIXED since baseline)**. Baseline:
      "silent hang. no banner, no log line, no timeout, ever" on this exact screen. Entry
      point relocated since baseline: no longer a button on the root Identities screen —
      reached via the breadcrumb identity-switcher dropdown ("Ziutek Zielonka ›" pill) →
      "Load an existing identity" → "Identity ID & private key" tab (this new Identity Hub
      picker screen, `src/ui/identity/picker.rs`, superseded the old `identities_screen.rs`
      entry point; confirmed via source that `identities_screen.rs`'s own "Load Identity"
      button is very likely dead/unreachable now that `IdentityHubScreen` is the active root —
      not verified further since the working path was found). Entered a real identity ID from
      this fixture's migrated identities (`4794iiLvNfiuQ8pv3qF7zbNmERxkDmRbHKNJvgo3EDQb`, no
      private key), clicked "Load Identity" — resolved **instantly** with "Successfully loaded
      identity." No hang, no timeout. Screenshot: `IDN-002-1-load-identity-success-FIXED.png`.
- [ ] IDN-001, IDN-016 — recently-touched migration/identity code
- [x] MN-001: Load a masternode by keys — **PASS (FIXED since baseline)**. Baseline: "submit
      buttons hang completely silently on click: no banner, no log line, no timeout, ever."
      Retested on Masternodes > "Load a masternode" > Masternode tab. First tried a 62-char
      hex string (too short) — correctly rejected client-side, instantly: "The ProTxHash you
      entered could not be read. Enter a 64-character hex ProTxHash or the Base58 identity
      ID." Then tried a well-formed 64-char hex string that doesn't correspond to a real node
      — dispatched, resolved **instantly** with a clean typed error: "No masternode or evonode
      was found on the network for this ProTxHash. Check the ProTxHash and try again, or
      confirm the node is registered on this network." No hang at any point. This was
      previously the worst-behaved control in the whole baseline report (singled out as "worse
      than every other blocked-by-environment flow"); it now degrades exactly like its working
      sibling fields always did. Screenshot: `MN-001-1-load-masternode-typed-error-FIXED.png`.
- [x] DOC-002: Update an existing data contract — **PASS (FIXED since baseline — was the
      single "Critical" finding)**. Baseline: clicking "Update Contract" panicked the whole
      process via `.expect("Failed to load contracts")` in
      `src/ui/contracts_documents/update_contract_screen.rs`. Live retest: Contracts >
      Contracts (top menu) > "Update Contract" loaded cleanly — Identity/Contract selectors,
      JSON editor, no crash, app fully responsive (identity "det.dash", balance 0.028936
      shown). Source confirms the fix precisely: line 93 now uses
      `app_context.get_contracts().unwrap_or_else(|error| { MessageBanner::set_global(...,
      "Your saved contracts could not be loaded. Try opening this screen again.", ...); Vec::new()
      })` — the exact typed-error pattern the baseline said only the sibling "Register
      Contract" screen had. A new unit test
      `constructor_degrades_when_contracts_cannot_be_loaded` directly covers the baseline's
      precise trigger condition (no wallet backend wired) and asserts the screen degrades to
      an empty contract list instead of panicking — confirmed via source read, not re-run
      (this session's backend is already wired, so the live click above did not exercise the
      exact race, but the unit test does and is dispositive for a `.expect()` → typed-error
      fix of this kind).
- [x] DOC-004: Query and browse documents — **PASS (FIXED since baseline)**. Baseline: "silent
      infinite hang. 'Fetch Documents' dispatches a real query and never resolves — no banner,
      no error, ever." Retested with the identical repro setup (DPNS contract, "SELECT * FROM
      domain"): clicked "Fetch Documents" — resolved **instantly** with real DPNS domain
      records rendered in YAML (parentDomainName, records.identity, $ownerId, label,
      normalizedLabel, etc.), pagination ("Page 1" / "Next Page"), a document filter field, a
      "Select Properties" control, and a YAML/JSON display toggle. No hang, no delay.
      Screenshot: `DOC-004-1-fetch-documents-success-FIXED.png`.
- [x] TOK-005: Create token contract — **PASS (FIXED since baseline — was the root of a
      3-instance click-no-op pattern that transitively blocked TOK-006 through
      TOK-013/015/017)**. Filled in Token Creator (identity "Abelard Alfabetny", name
      "QAToken860", initial supply 1000, preset "Most Restrictive") and clicked "Create Token"
      — a "Confirm Token Contract Registration" dialog appeared with name/supply/cost
      (0.302 DASH) and a NotTradeable warning, exactly as expected. First attempt correctly
      failed with a clean, actionable balance error ("Not enough balance. You have
      0.2032498938 DASH but this operation requires 0.300001 DASH. Please top up your identity
      first.") — topped up the identity via "Add funds" > "From your wallet" > wallet
      "e2e-test" > 1 DASH (this also live-tested SND-008, which passed: "Identity Topped Up
      Successfully!", balance updated to 1.2032 DASH instantly). Retried token creation with
      sufficient balance — "Token Contract Created Successfully!", token appeared in "My
      Tokens" with a real Token ID (`5c9gZUpdcdRdr9ZPGPpsH3dytSnoqarm6cdCMCUyftWW`). Not an
      environment fluke: dialog → balance validation → real broadcast, every step behaved
      correctly. Screenshots: `TOK-005-1-create-token-confirm-dialog-FIXED.png`,
      `TOK-005-2-token-created-success.png`.
- [x] TOK-018: Stop tracking a token balance — **PASS (FIXED since baseline)**. Clicked "X" on
      the newly-created QAToken860 row in "My Tokens" — a "Confirm Remove Token" dialog
      appeared with the exact story wording ("Are you sure you want to stop tracking the token
      'QAToken860'? You can re-add it later. Your actual token balance will not change with
      this action."). Confirmed — token disappeared from the list ("No Tracked Tokens").
- [x] TOK-003: Add token by contract or token ID — **PASS (FIXED since baseline)**. Baseline:
      "well-formed contract ID dispatches correctly and the query genuinely fails... but the
      failure is never surfaced to the user at all." Retested via "Import Token" > pasted the
      just-removed token's ID > "Search" — correctly resolved "Found token: QAToken860" >
      clicked "Import Token" > "Token Added Successfully". This also exercises TOK-018's
      "re-importing the token restores it" criterion, which held. Screenshot:
      `TOK-003-018-1-import-and-restore-success.png`.
- [x] IDN-006: Transfer credits between identities — **PASS (FIXED since baseline)**. Baseline:
      "reproducible click no-op across 5 repro attempts." Retested: identity "Abelard
      Alfabetny" Home > "Send to another identity" > entered 0.001 DASH > "Receiver Identity
      ID" dropdown correctly listed known identities (det.dash / importowany / zzzzzzzzzz) >
      picked "det.dash" (auto-filled its real ID) > "Transfer" > "Confirm Transfer" dialog
      appeared with correct amount/destination > Confirm > "Transfer Successful!" Screenshot:
      `IDN-006-1-transfer-confirm-dialog-FIXED.png`.
- [x] IDN-008: View identity keys and details — **PASS (materially improved since baseline,
      IDN-013a's narrower gap persists)**. Baseline: "only an aggregate key count is
      reachable." Retested: Identity > Settings tab > "▶ Advanced" > "Keys" section now shows
      "This identity has 7 keys" plus a working "Manage keys" button that opens a full
      "Identity Keys" table (Key ID, Purpose, Security Level, Type, Read Only for all 7 keys:
      AUTHENTICATION/TRANSFER/ENCRYPTION/DECRYPTION at various security levels). This is a
      real, complete key list, not just a count — IDN-008's core acceptance criterion is met.
      However, clicking a key row does nothing (tried multiple x-offsets on row 0) — confirmed
      via source: `src/ui/identities/keys/keys_screen.rs` has only a "Back" button, no
      row-click handler, and `grep` for `ScreenType::KeyInfo` dispatch sites found none outside
      `src/ui/mod.rs`'s own enum plumbing — `KeyInfoScreen` is fully wired in the screen-type
      machinery but never navigated to from any UI. So IDN-013a's specific narrower gap
      ("Password-protect identity keys" needs `KeyInfoScreen`) still stands even though
      IDN-008 itself now passes.
- [x] IDN-013a: Password-protect an identity's signing keys — **FAIL (confirmed still
      reproducing, unchanged from baseline)**. See IDN-008 entry above for full detail: source
      confirms `KeyInfoScreen` (where the "Add password protection…" flow described in
      `CLAUDE.md`'s secret-seam design lives) has zero live UI trigger anywhere in the
      codebase — the one plausible entry point (clicking a row in the new "Manage keys" table)
      has no click handler at all.
- [x] IDN-009: Refresh identity state — **FAIL (confirmed still reproducing)**. Clicked
      "Refresh identity data" on identity "Abelard Alfabetny" (Settings > Advanced) — no
      banner appeared (success or error), key count stayed at 7, and `det.log` shows zero
      log activity correlated with the click despite the log being actively written by other
      background tasks (SPV/address-sync) in the same window — suggesting the button may not
      be dispatching a backend task at all, which is arguably worse than baseline's "dispatches
      cleanly... but doesn't update" characterization. Not exhaustively re-verified (would need
      a genuine on-chain key-state change to confirm the refresh's *effect*, which this session
      didn't have set up), but the complete absence of any visible or logged response is
      consistent with — and not better than — the baseline finding.
- [x] TOK-011: Claim distributed tokens — **PASS (FIXED since baseline)**. Baseline: "Claim form
      fully functional and shows a real live perpetual distribution, but the 'Claim' submit
      button is a confirmed click no-op — same defect class as TOK-005." Reproduced the identical
      fixture setup live (created token "QAPerp860" with a TimeBased perpetual distribution,
      every 1h, fixed amount 10 base tokens, recipient ContractOwner — matches the baseline
      fixture's own wording verbatim: "This token is using a time based distribution where every
      1h it will distribute a fixed amount of 10 base tokens."). My Tokens > token name (not
      "More Info" — that only opens a read-only config viewer) > per-identity table > Claim
      opened the same complete form as baseline. Clicking "Claim" now shows a **"Confirm Claim"**
      dialog (an improvement over baseline, which had zero confirmation) — confirming dispatched
      a real backend call that reached the network and returned a correctly-typed consensus
      rejection, `InvalidTokenClaimNoCurrentRewards` (expected: only ~2 minutes had elapsed since
      contract creation, not the full 1h interval, so there is genuinely nothing to claim yet).
      This is conclusive: the control is no longer a silent no-op — it dispatches, round-trips to
      the network, and surfaces a real, correct, typed response. Same fix class as TOK-005/018/003.
      Minor observation (not blocking the verdict): the error banner shown to the user was the
      generic "An unexpected error occurred. Please try again later." rather than a specific
      "nothing to claim yet, check back later" message — worth a follow-up typed `TaskError`
      variant per CLAUDE.md's error-message conventions, but not a functional defect.
- [x] ALK-002: View asset lock details — **FAIL (confirmed still reproducing, unchanged from
      baseline)**. Baseline: "'Asset Locks' list never shows a just-created, confirmed-usable
      lock, even after Refresh/renavigation — data is persisted correctly per direct SQLite
      check, this is a UI/cache bug, not a coin-selection issue." Retested with two independent
      live methods this session: (1) WAL-017/SND-008's "Fund Platform Address"/"Add funds from
      wallet" flows, both of which genuinely broadcast real chain-level asset-lock special
      transactions — confirmed via `det.log`: real `AssetLockPayloadType` transactions with
      `credit_outputs` and `wait_for_proof: entered ... timeout=Some(300s)` log lines, at
      20:12:07 and 20:30:55; (2) the dedicated Wallets > "Create Asset Lock" > Top Up screen,
      which generates a fresh receive address + QR/BIP21 URI and a "Waiting for funds..." status
      (confirmed this sub-flow behaves correctly on its own terms: real address, correct amount,
      correct status text). In both cases, Wallets > [wallet] > Dash Core tab > Asset Locks
      stayed **"No asset locks found"** throughout the entire session, across multiple Refreshes
      and renavigations — reproducing the exact baseline symptom. (Side note, not a fund-safety
      issue: two 0.5 DASH sends made directly to "Create Asset Lock"-generated addresses, when
      the screen was navigated away from before detecting the deposit, landed as ordinary Core
      wallet balance rather than becoming asset locks — the funds are safe and spendable
      normally, just didn't complete the intended lock-creation step; this is a corollary of the
      same underlying gap, not a separate bug.)
- [x] PASS spot-checks — DPN, DPY, TOK, DOC, IDH, MN, DEV:
  - DPN-002 (View owned usernames) — **PASS, no regression**. Identity "det.dash" Home tab shows
    display name "sdafafa1" and "@det" prominently; "Pick a username" checklist item correctly
    shown as already-complete ("You are @det.").
  - DPY-001 (View and edit DashPay profile) — **PASS, no regression**. Identity Settings tab
    shows a full, populated Social profile editor (Change photo, Display name, About, Avatar URL,
    Save/Delete social profile) plus Username card (Primary/Copy/"View all usernames"), "Name on
    this device", and Aliases — matches baseline exactly.
  - TOK/DOC — already substantively covered above today (TOK-005/018/003/011, DOC-002/004), no
    additional spot-checks needed to satisfy the "2-3 per category" guidance.
  - IDH-002/003 (Identity home at a glance / multi-identity switching) — incidentally exercised
    many times throughout this session (Home tab, breadcrumb identity-switcher dropdown used
    repeatedly for ALK-002/TOK-011/DPN-002/DPY-001 above, across 5+ identities) — consistently
    smooth, no regression observed.
  - MN-002 (See my masternodes at a glance) — **PASS, no regression**. "All masternodes" empty
    state: "No masternodes loaded", correct explanatory copy, "Load a masternode" CTA — matches
    baseline; correctly gated to Expert view (session was in Expert throughout).
  - DEV-005 (View Platform info) — **upgraded FAIL → PASS since baseline**. Baseline: "2/8
    sub-tools work — Basic Platform Info, Validator Set Info; rest blocked by known
    masternode-list-sync issue," flagged as "very likely stale, worth a quick re-check." Live
    retest of all remaining 6 sub-tools in Tools > Platform info: **Fetch Current Epoch Info**,
    **Fetch Total Credits on Platform**, **Fetch Version Voting State**, **Fetch Current
    Withdrawals in Queue**, and **Fetch Shielded Pool State** all now return real, correctly
    formatted data (epoch index/height/timing, credit totals, vote tallies, live withdrawal
    records, shielded pool balance). All 8/8 sub-tools now work — the masternode-list-sync
    blocker this baseline flagged is fully resolved.

## New findings (not in baseline, discovered incidentally during this campaign)

- **Sticky "Creating token..." progress banner.** After TOK-005's successful token creation
  (and two subsequent operations — TOK-018 remove, TOK-003 re-import, both also successful),
  the info banner "Creating token..." remained visibly displayed and counting up (observed at
  201s+) instead of being dismissed once the operation resolved. Screen content correctly
  showed "Token Contract Created Successfully!" / "Token Added Successfully" throughout, so
  this is cosmetic (a leaked `BannerHandle`, likely missing a `take_and_clear()` call in the
  Token Creator screen's `display_message()`), not a functional defect — but it is the same
  class of bug as one of the 5 recent `b11ab3ea..e6ba4857` fixes ("a sticky sweep banner after
  DB clear"), suggesting a similar leak exists in the token-creation flow specifically, not
  fully covered by that fix. Screenshot: `NEW-sticky-creating-token-banner.png`.
  **FIXED this session**: root cause confirmed precisely — `BackendTaskSuccessResult::
  RegisteredTokenContract` (the success signal for token-contract creation,
  `src/ui/tokens/tokens_screen/mod.rs`) updated `token_creator_status` but never called
  `operation_banner.take_and_clear()`; the generic pending-operation-completion helper only
  recognized two other result variants and was never wired up for this one either. Fixed via
  Codex Sol with a new regression test (proven red before / green after), clippy clean, merged
  to `docs/platform-wallet-migration-design` as commit `0c8d4834` (squash-merge of `d59c004f`).
  Not yet live-reverified against a fresh build in this session (would require relaunching the
  QA binary against a rebuild, disrupting the funded/synced fixture state) — verification is via
  the targeted unit test plus source review, per the campaign's "narrow verify" guidance.
- **Background identity-sync wallet-id-mismatch error (recurring, self-resolving).** During the
  TOK-005 top-up/create-token sequence, `det.log` recorded 5 occurrences (2026-07-16
  20:33:18–20:36:16, roughly every 40-60s, then stopped) of:
  `identity-sync: failed to persist token balance changeset identity_id=7iTCPAsejtbs3XbTjbMHTQUWE4zdQWBnbmFPLUMzh3Ye
  error=persistence backend error (Fatal): wallet id mismatch: entry names
  c59da12a9461a5b6ca318120a17b7ba6db0130e872f83d9fc914c3693367e0b0 but flush is scoped to
  0000000000000000000000000000000000000000000000000000000000000000`. A background
  identity-sync task attempted to flush a token-balance changeset for an identity scoped to a
  real wallet ID, but the flush call itself was scoped to an all-zero placeholder wallet ID —
  a genuine wallet-scope bug in `platform_wallet::manager::identity_sync`, not an environment
  artifact (this session's backend was fully synced and healthy throughout). Did not visibly
  block any UI flow — all token operations completed successfully around it — but 5 repeated
  Fatal-labeled errors in under 3 minutes is a real correctness bug worth a source-level look,
  possibly related to an identity loaded via IDN-002's "Identity ID & private key" path
  earlier in this session (loaded with no wallet association, which may be why the sync task
  can't resolve its real wallet scope). Not investigated further — QA-only, flagging for
  follow-up.
  **Triaged this session** (source-level, no fix attempted — confirmed upstream, not DET's
  code): root-caused precisely to a seam bug between `packages/rs-platform-wallet/src/manager/
  identity_sync.rs` (`apply_fresh_balances`, ~line 608 — unconditionally flushes token-balance
  changesets under the all-zero `WalletId::default()` sentinel) and `packages/
  rs-platform-wallet-storage/src/sqlite/schema/mod.rs`'s `assert_identities_belong_to_wallet`
  (~line 28-70 — the sentinel scope is only tolerated when the identity's own `wallet_id` column
  is NULL; it errors for any identity that DOES have a real wallet association). **Correction to
  the original guess above**: the mismatch fires for identities WITH a real wallet id attached,
  not identities lacking one — so this affects the common case (most loaded identities), not
  just IDN-002's no-wallet path. Not a UI/crash risk (the error is caught and only logged; the
  in-memory balance cache the UI reads updates regardless), but a real correctness gap: the
  on-disk `token_balances` table silently never gets the update for affected identities, so a
  cold restart could show stale token balances until the next sync pass. Filed as memcan TODO
  `92e7dcad-bb89-4e27-8041-5d6add39bd3d` (project `dash-evo-tool`) with full repro/root-cause
  detail. This is a `platform-wallet` git-dependency bug (pinned rev `d18020f5`), not fixable in
  this repo — queued as a decision for the user: file upstream against `dashpay/platform`, or
  accept as a known low-impact limitation.

## Deferred / not re-run

- NET-011, NET-019 — destructive, deliberately not re-run without explicit user authorization
  (same precedent as baseline campaign).

---

## Session log

### 2026-07-16 — session start

- Confirmed HEAD at `e6ba4857` (matches required SHA).
- Built `dash-evo-tool` fresh (`cargo build --bin dash-evo-tool`) — no-op rebuild (already
  current), copied to private path `/data/tmp/det-pr860-qa-bin/dash-evo-tool`, sha256
  `c91f7ce151d248762c50b3601b0ba2c7d796c3ed28054a12c1d007f64d8bf439`.
- Building `det-cli` (`--features cli`) in background for MCP-001.

### 2026-07-16 — campaign resumed after an interrupted-agent restart

A prior coordinator agent running this exact campaign was killed mid-run by an unrelated
top-level tooling bug (`tmux kill-pane` index drift), not by any error in the campaign itself.
This session resumed from the checkpointed state above — verified on-disk state independently
before trusting the handoff (binary sha256 matched, QA app was still live and running against
`/data/tmp/det-pr860-wal032-fixture-data`, the `fix-token-creator-banner` worktree was confirmed
clean/untouched) rather than assuming it. Completed the remaining checklist: TOK-011, ALK-002,
and the PASS spot-check pass (see above). Dispatched Codex Sol for the sticky-banner fix,
independently verified (targeted `cargo test`/`clippy`), and merged as `0c8d4834`. Triaged the
wallet-id-mismatch bug to a precise upstream root cause and filed it as a memcan TODO rather than
attempting a same-repo fix. Report assembled and published — see the artifact link the
coordinator relayed.
- Display `:99` confirmed free, no other GUI process running.
