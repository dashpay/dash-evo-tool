# PR892 User-Story QA — Progress Checklist

Tracks completion of every story in **PR892's own `docs/user-stories.md`**
(`/data/git-worktrees/home-ubuntu-git-dash-evo-tool-2-pr892-build/docs/user-stories.md` @
commit `57195d54`) against the PR892 build. One line per story.

Verdicts: PASS / FAIL / BLOCKED (reason) / N/A (Gap/Superseded/Removed — not implemented, no
testing needed).

**Reconciliation note (2026-07-14, post-initial-sweep):** the first pass of this campaign was
run against `docs/user-stories.md` in the *qa-docs* worktree (based on `v1.0-dev`, 123
stories) — this was a coordinator pointing error, not a stale-doc issue as originally
reported below. PR892's real catalog is a **superset**: 175 stories (155 `[Implemented]`, 17
`[Gap]`, 2 `[Removed]`, 1 `[Superseded by MN-001]`), adding three new categories (UX, IDH,
MN) plus new/retitled/reclassified stories within existing categories. This file has been
reconciled against the real PR892 doc: every story tested in the first pass whose definition
is unchanged keeps its original verdict; stories reclassified to Gap/Removed/Superseded are
marked N/A (with a note where the original FAIL finding is still informative — e.g. a story
now tagged Gap because the feature genuinely isn't implemented, which is exactly what testing
found); genuinely new or redefined stories are unchecked, pending testing. See
`summary-report.md`'s methodology section for full detail, including a second, unrelated
incident (a shared-build-path binary clobber) also noted there.

**Also note**: the source doc has a genuine duplicate ID — `IDN-013` is used for two
different stories ("Password-protect an identity's signing keys (SEC-001)" and "Top up
identity from Platform addresses"). Tracked here as `IDN-013a` and `IDN-013b` respectively to
disambiguate; flagged as a documentation defect worth fixing upstream in `docs/user-stories.md`.

**Totals:** 175 stories total (176 tracked lines here due to the IDN-013 duplicate) — 155
`[Implemented]` (to test), 17 `[Gap]`, 2 `[Removed]`, 1 `[Superseded]` (20 N/A, no testing
needed).

## WAL

- [x] WAL-001: Create a new wallet — PASS
- [x] WAL-002: Import wallet via mnemonic — PASS
- [x] WAL-003: Import single private key — PASS (send-from-SK is a documented product limitation, not a bug)
- [x] WAL-004: Switch between wallets — PASS (per-network isolation noted, not a defect; multi-wallet switching confirmed)
- [x] WAL-005: Rename a wallet — FAIL (Rename button is completely inert on both HD and SK wallets)
- [x] WAL-006: Lock and unlock wallet — FAIL (Lock works; Unlock never opens a password prompt — self-lockout bug)
- [x] WAL-007: Remove a wallet — FAIL (confirmation prompt missing for single-key wallets; present for HD wallets)
- [x] WAL-008: View wallet balances — PASS (Default view does not actually simplify the Wallet screen — UX gap noted)
- [x] WAL-009: View fiat equivalent of balances — N/A (Gap, not implemented)
- [x] WAL-010: Generate receive address — PASS
- [x] WAL-011: View address table — PASS
- [x] WAL-012: View and export private keys — PASS
- [x] WAL-013: View SPV sync status — PASS
- [x] WAL-014: Label addresses — N/A (Gap, not implemented)
- [x] WAL-015: Create throwaway wallet without mnemonic backup — N/A (Gap, not implemented)
- [x] WAL-016: View transaction history — PASS (PR892 cold-boot regression test confirmed fixed)
- [x] WAL-017: Fund Platform address from wallet — FAIL (asset-lock coin selection: "No UTXOs available for selection" despite funded wallet; later shown transient/non-persistent, see ALK.md)
- [x] WAL-018: Fund Platform address from asset lock — BLOCKED (retested post-fix: asset-lock creation now works — a fresh 0.5 DASH lock was created live and confirmed persisted via direct SQLite check — but the "Asset Locks" list still never surfaces it, reproducing ALK-002's confirmed UI/cache bug; that list is the only reachable path to the "Fund a Platform address with this asset lock" dialog, so the story remains genuinely blocked for an independent, now-confirmed reason, not the original WAL-017/env-blocker cause)
- [x] WAL-019: Transfer credits between Platform addresses — PASS (retested post-fix: transferred 0.005 DASH between two of the wallet's own Platform addresses via Advanced Options; all 4 fee-strategy options present; balance math confirmed correct for "Deduct from first input")
- [x] WAL-020: Withdraw from Platform address to Core — PASS (retested post-fix: withdrew 0.005 DASH from a Platform address to a Core address; "Withdrawal initiated successfully!" confirmed)
- [x] WAL-021: Navigate wallet accounts via tabs — PASS
- [x] WAL-022: View system accounts in the Detailed view — PASS (title updated from "developer mode" to "the Detailed view" in the reconciled doc; same underlying test — System tab gated on "not Default view")
- [x] WAL-023: Collapsible transaction history — PASS
- [x] WAL-024: Collapsible balance breakdown — PASS
- [x] WAL-025: Restore a password-protected imported key after an update — BLOCKED (retested post-fix: still no legacy password-protected imported-key fixture exists in this data dir, so the flow itself can't be exercised; but the restore-scan itself now runs cleanly — confirmed via a full healthy session with zero `MigrationFailed`/`WalletBackendUnavailable` warnings in det.log — so the env blocker no longer suppresses it, only the missing fixture blocks the story)
- [x] WAL-026: Unlock a passphrase-protected vault at startup — BLOCKED for live UI (no passphrase-sealed vault fixture; destructive resealing out of scope) — source review confirms the flow (`BootApp`/`UnlockState` in `src/boot.rs`) is implemented as specified
- [x] WAL-027: Balance health check after syncing — FAIL (retested post-fix with a genuine, fully-completed sync and many real balance-changing operations across the session: the header total always correctly reconciled with the account-tab breakdown, and source review confirms no balance-health reconciler or warning-banner mechanism exists anywhere in the codebase — the only match for the story's own language is an internal unit test, `header_total_reconciles_with_core_tab_breakdown_through_real_accessors`, not a user-facing runtime check; same conclusion as the earlier degraded-environment session, now reconfirmed in a fully healthy one)
- [x] WAL-028: Switch the active wallet from the top-nav pill on the Wallets tab — PASS (pill interactivity, in-place switching, cross-surface re-sync, pill/in-tab-picker agreement, and single-wallet inert-pill all confirmed live; single-key-vs-HD precedence sub-check not exercised, no safe fixture)
- [x] WAL-029: View and copy my shielded receive address — PASS (retested post-fix: Shielded tab now renders the address immediately, no longer stuck at "Preparing shielded wallet..."; both clicking the address text and clicking "Copy" verified via `xclip` to copy the full untruncated 83-character `tdash1...` address to the system clipboard, matching the truncated display's prefix/suffix)
- [x] WAL-030: Inspect shielded note details — N/A (Gap, not implemented)
- [x] WAL-031: Single-key wallet balance and UTXOs update automatically — N/A (Gap, not implemented)

## SND

- [x] SND-001: Send Dash to an address — PASS (nav confirmed; full E2E send now completed — but no confirmation dialog appears before broadcast, see SND-005)
- [x] SND-002: Send Dash from single-key wallet — N/A (reclassified to Gap in the reconciled doc; original testing found sending explicitly disabled for single-key wallets with a typed `SingleKeyWalletsUnsupported` error, consistent with — and likely the reason for — this reclassification; see scenarios/SND.md)
- [x] SND-003: Receive Dash with QR code — FAIL (Receive button inert, no QR shown)
- [x] SND-004: Send to a DPNS username — N/A (Gap, not implemented)
- [x] SND-005: See fee estimate before confirming send — FAIL (no fee estimate or confirmation dialog anywhere pre-broadcast; Max silently deducts an undisplayed fee)
- [x] SND-006: Send to multiple recipients — PASS (add/remove recipients, single tx broadcast confirmed on-chain)
- [x] SND-007: Shield DASH from Core wallet — FAIL ("Invalid output address" on submit; root cause disclosed in-app as "Shielded sending is not available on this network yet")
- [x] SND-008: Top up identity from Send screen — BLOCKED (no identity exists yet — IDN not run; Identity-destination UI recognition partially verified)
- [x] SND-009: Shield credits from Platform address — FAIL (retested post-fix: Platform Addresses source now funded and selectable, correctly auto-selects the highest-balance address; but the shielded destination is rejected with "Invalid output address" at submission — same root cause as SND-007 — even though Advanced Options recognizes and tags it "(Shielded)" beforehand)
- [x] SND-010: Withdraw from shielded pool to Core address — BLOCKED (shielded balance always 0; no "Shielded Pool" source option exposed in Send screen)
- [x] SND-011: Transfer identity credits to another identity — BLOCKED (no identity exists yet — IDN not run)
- [x] SND-012: Withdraw identity credits to Core address — BLOCKED (same reasoning as SND-011)
- [x] SND-013: Transfer identity credits to Platform address — BLOCKED (same reasoning as SND-011)
- [x] SND-014: Send maximum from a Core wallet — FAIL (fee-reserve math correct, but the fee-shown-next-to-amount label and the too-low-balance message are both dead code in the render path; source-confirmed, root-causes SND-005)
- [x] SND-015: Unshield credits to a Platform address — FAIL (button exists in source, correctly wired to the unified Send screen preset, but unconditionally hidden behind a hardcoded not-yet-activated `ShieldedOperations` capability gate — never reachable live on any network in this build)
- [x] SND-016: Send privately within the shielded pool — FAIL (same reachability gap as SND-015; spend-lock/verification-in-progress UX for the button is correctly implemented in source but unobservable live for the same reason)

## ALK

- [x] ALK-001: Create an asset lock — PASS (also: differential re-test proves WAL-017's
      coin-selection failure is NOT a global/persistent defect — see scope conclusion in
      `scenarios/ALK.md`; IDN/DPN/DPY/TOK/DOC should NOT be pre-emptively marked BLOCKED)
- [x] ALK-002: View asset lock details — FAIL ("Asset Locks" list never shows a just-created,
      confirmed-usable lock, even after Refresh/renavigation — data is persisted correctly per
      direct SQLite check, this is a UI/cache bug, not a coin-selection issue). Reconfirmed
      post-fix: a fresh 0.5 DASH lock created live in a healthy, fully-synced session still
      never appears, even after Refresh — verdict stands, see scenarios/ALK.md.
- [x] ALK-003: Recover unused asset locks — BLOCKED (same list-population bug as ALK-002 blocks
      reaching any recovery UI). Reconfirmed post-fix — verdict stands, see scenarios/ALK.md.
- [x] ALK-004: Quick-fund workflow — N/A (Gap, not implemented)

## IDN

- [x] IDN-001: Register a new identity — PASS (retested post-env-fix: full E2E wizard using
      "From your wallet" funding — "Identity Registered Successfully!"; the earlier "+Add Key"
      no-op bug in Advanced key-selection mode was NOT re-verified this pass, superseded by
      IDN-007's PASS via the direct "Add a new key" screen)
- [x] IDN-002: Load existing identity by ID — FAIL (ID+key "Load Identity" button silently
      hangs with zero feedback; sibling tabs on the same screen — "From my wallet", "My
      username" — degrade gracefully with clean typed/generic errors)
- [x] IDN-003: Load evonode/masternode identity — N/A (reclassified to `[Superseded by
      MN-001]` in the reconciled doc; original FAIL finding — same silent-hang defect class
      as IDN-002 — carried forward as context for whoever tests MN-001, which now owns this
      capability)
- [x] IDN-004: Top up identity credits — PASS (retested post-env-fix: top-up via Platform
      address, "Identity Topped Up Successfully!")
- [x] IDN-005: Withdraw credits to Core address — PASS (retested post-env-fix: confirmation
      dialog + "Withdrawal Successful!" to a Core address)
- [x] IDN-006: Transfer credits between identities — FAIL (retested post-env-fix with two real
      identities: the "Transfer" button is a confirmed, reproducible click no-op — enabled,
      hoverable with correct tooltip, zero effect on click, 5 repro attempts across both
      destination-type variants; see scenarios/IDN.md)
- [x] IDN-007: Add key to identity — PASS (retested post-env-fix: on-chain `IdentityUpdate`
      state transition confirmed via broadcast+proof-verification log evidence, not just the
      success screen; see scenarios/IDN.md for a secondary key-list-staleness finding tied to
      IDN-009)
- [x] IDN-008: View identity keys and details — FAIL (retested post-env-fix: only an aggregate
      "This identity has N keys" count is reachable; no per-key list with type/purpose/status
      and no individual key detail view — source confirms `KeysScreen`/`KeyInfoScreen` exist but
      have no live navigation trigger for a normal keyed identity, see scenarios/IDN.md)
- [x] IDN-013a: Password-protect an identity's signing keys (SEC-001) — BLOCKED (retested
      post-env-fix: an identity is now reachable, but Key Info screen — which hosts the Key
      Protection section — has no reachable navigation path for a normal keyed User identity in
      this build's default UI; same structural gap as IDN-008, not the prior "no identity"
      reasoning; underlying mechanism previously source-confirmed implemented)
- [x] IDN-009: Refresh identity state — FAIL (retested post-env-fix: button dispatches cleanly
      with no hang — a major improvement — but the displayed key count never updates even after
      3 refreshes + full navigation reload over ~10 min, despite a confirmed on-chain 7th key
      from IDN-007; credit balance does update correctly)
- [x] IDN-010: Search identity by DPNS name — PASS (retested post-env-fix: searching "alice"
      now successfully finds and loads a real Testnet identity, `alice.dash`, 1.1747 DASH —
      previously failed cleanly on the masternode-list/quorum-sync error, now returns real
      results end-to-end)
- [x] IDN-011: Bulk identity creation — N/A (Gap, not implemented)
- [x] IDN-012: Register identity from Platform addresses — PASS (retested post-env-fix: full E2E
      identity registration funded directly from a Platform address, bypassing the broken
      Asset-Locks list entirely — "Identity Registered Successfully!")
- [x] IDN-013b: Top up identity from Platform addresses — PASS (retested post-env-fix: same
      flow/result as IDN-004, "Identity Topped Up Successfully!")
- [x] IDN-014: Fund identity by receiving a deposit to a shown QR/address — FAIL (deposit-address
      step renders zero content — no QR, no address, no amount field, no error; directly
      reachable without a pre-existing identity, re-verified fresh this session)
- [x] IDN-015: Automatic identity discovery after sync — PASS (live det.log from this exact
      running process shows the once-per-session auto-trigger firing and completing on Platform
      readiness; source review confirms rolling 5-index window and alias-preserving refresh)
- [x] IDN-016: Identities and their keys preserved across an app upgrade — BLOCKED for the
      story's literal criteria (no pre-upgrade legacy fixture, unchanged); **separately, a real
      restart-survival test confirmed the flagged asset-lock recurrence risk**: a clean quit +
      relaunch reproduced the exact `ALK.md`/`TEST-VECTOR.md` `WalletBackendNotYetWired` failure
      on a NEW `is_locked` row (WAL-018's 0.5 DASH lock), leaving all 3 identities inaccessible
      via UI (data confirmed intact via direct SQLite check, not lost). Same root-caused defect,
      not a new bug. No DB fix attempted — see scenarios/IDN.md for full detail. **Data dir is
      currently in this broken state; report back before continuing DPN/DPY/TOK/DOC/IDH/MN.**

## DPN

- [x] DPN-001: Register a DPNS username — PASS (retested post-env-fix: registered `detqa892run2`
      for `QA Identity 1`; UX-001 blocking overlay confirmed via log; fee estimate found ~13x
      inaccurate — 0.000056 DASH shown vs ~0.00073 DASH actual — noted, not a criteria failure)
- [x] DPN-002: View owned usernames — PASS (retested post-env-fix: identity picker tiles show
      owned `@username` per identity, e.g. `QA Identity 1` → `@detqa892run2`)
- [x] DPN-003: View active name contests — BLOCKED (retested post-env-fix: still no
      masternode/evonode identity available in this environment — no ProTxHash fixture, real
      registration needs ~1000 tDASH collateral — independent of the asset-lock recurrence; two
      real User identities exist and work fine, but neither is a masternode/evonode identity)
- [x] DPN-004: View past name contests — BLOCKED (same as DPN-003)
- [x] DPN-005: Vote on contested names — BLOCKED (same as DPN-003; acceptance criteria
      itself requires a masternode/evonode identity)
- [x] DPN-006: Schedule votes — BLOCKED (same as DPN-003)
- [x] DPN-007: Batch voting across contests — BLOCKED (same as DPN-003)
- [x] DPN-008: Set an alias for an owned username — BLOCKED (retested post-env-fix: an identity
      with a registered username now exists, but the "My usernames" table has no reachable
      navigation path in this build's Identity Hub — structural gap, same class as
      IDN-008/IDN-013a, not identity-availability)
- [x] DPN-009: Scheduled votes preserved across an app upgrade — BLOCKED (unchanged: no
      pre-upgrade legacy scheduled-votes fixture; additionally no masternode identity exists to
      create a vote to restart-test in the first place — restart not attempted, nothing to test)

## DPY

- [x] DPY-001: View and edit DashPay profile — PASS (retested post-env-fix: profile created for
      `QA Identity 1`, confirmed via on-chain state transition in det.log)
- [x] DPY-002: Search DashPay profiles — PASS (retested post-env-fix: Profile Search finds real
      profiles and correctly reports "no profile" for identities without one, before contact-request)
- [x] DPY-003: Send contact request — PASS (retested post-env-fix, self-tested `QA Identity 1` →
      `QA Identity 2` by identity ID, confirmed via on-chain state transition)
- [x] DPY-004: Accept or reject contact requests — PASS (retested post-env-fix: `QA Identity 2`
      accepted the incoming request; both sides show an established Active contact)
- [x] DPY-005: View contact list and details — PASS (retested post-env-fix: list + detail view
      both work; detail view has no direct click-through from the Contacts tab row itself, noted
      as a gap but not blocking — see scenarios/DPY.md)
- [x] DPY-006: Send payment to contact — FAIL (retested post-env-fix: every payment attempt fails
      with `EncryptionError { detail: "Missing senderKeyIndex" }`; root-caused to a general,
      always-reproducible CBOR-integer-decoding bug in `derive_contact_payment_address`
      (`src/backend_task/dashpay/payments.rs` ~112-126) — a strict `Value::U32` match never
      matches real network-fetched documents, which decode integers as `Value::I128`; confirmed
      NOT an artifact of this pass's cancel-then-accept contact setup — see scenarios/DPY.md)
- [x] DPY-007: View payment history — Partial PASS (retested post-env-fix: screen, empty state,
      and Refresh button all confirmed reachable/correct; populated-list rendering unconfirmed
      because DPY-006 blocks any real payment from completing)
- [x] DPY-008: Generate DashPay QR code — PASS (retested post-env-fix: real QR + `dash:?di=...&dapk=...`
      data URI generated, with correct "can automatically become your contact" security warning)
- [x] DPY-009: Edit contact info — FAIL (retested post-env-fix: nickname/notes editing and
      persistence work correctly, but hiding a contact does NOT move it to a collapsed "Show
      hidden contacts" section on the Identity Hub Contacts tab — it stays listed unconditionally,
      contradicting an explicit acceptance-criteria bullet; hidden flag itself does persist)
- [x] DPY-010: Remove a contact — N/A (Gap, not implemented)
- [x] DPY-011: Auto-accept contact requests — PASS (retested post-env-fix: same QR-generator
      screen's Advanced Options exposes HD Account Index + Validity-Hours fields, with a
      `dapk=` auto-accept proof key embedded in the generated URI)
- [x] DPY-012: Detect payments received from contacts — BLOCKED (retested post-env-fix: cannot be
      live-tested because DPY-006's bug prevents any real DashPay payment from completing;
      plausible but unconfirmed shared root cause via the same address-derivation code path)
- [x] DPY-013: View contacts and avatars offline — Partial PASS (retested post-env-fix: the
      reachable Identity Hub Contacts tab shows contacts instantly, meeting the core criterion; a
      separate legacy Contacts screen was found live-reachable this pass and shows stale/empty
      data plus a real network fetch instead of an offline-first read — new finding, see
      scenarios/DPY.md)
- [x] DPY-014: Cancel a sent contact request — PASS (retested post-env-fix: cancel confirmed via
      on-chain `contactInfo` state transition; cancel-then-accept edge case correctly reconciles
      to an established contact on both sides, exercising the story's "already accepted" bullet
      live for the first time)

## TOK

- [x] TOK-001: View token balances — PASS (retested 2026-07-15: real tracked token
      `lklimek-20260217` listed correctly, per-identity balance table renders for all 3 identities)
- [x] TOK-002: Search and discover tokens — PASS (retested 2026-07-15: live keyword search
      returns real results; add-to-My-Tokens persists across navigation and Refresh)
- [x] TOK-003: Add token by contract or token ID — FAIL (not retested — out of 24-story scope;
      original finding stands: format validation + dispatch both work; well-formed-ID request
      fails but result is silently dropped, zero user feedback)
- [x] TOK-004: Transfer tokens — BLOCKED (retested 2026-07-15: reachable, Transfer correctly
      disabled for a 0 balance; TOK-005's failure blocks ever obtaining a QA-owned balance)
- [x] TOK-005: Create token contract — FAIL (retested 2026-07-15: "Create Token" /
      "Register Token Contract" / "View JSON" all confirmed reproducible click no-ops —
      a11y-verified coordinates, zero log activity, reproduced fresh after a full app relaunch;
      most severe TOK finding this pass, structurally blocks TOK-006–013/015/016/018)
- [x] TOK-006: Mint tokens — BLOCKED (retested 2026-07-15: reachable via a third-party fixture
      token; correct owner-only authorization rejection, not a bug — TOK-005 blocks a real test)
- [x] TOK-007: Burn tokens — BLOCKED (same as TOK-006)
- [x] TOK-008: Freeze and unfreeze token recipients — BLOCKED (same as TOK-006)
- [x] TOK-009: Pause and resume token transfers — BLOCKED (same as TOK-006)
- [x] TOK-010: Destroy frozen funds — BLOCKED (same as TOK-006)
- [x] TOK-011: Claim distributed tokens — FAIL (retested 2026-07-15: Claim form fully functional
      and shows a real live perpetual distribution, but the "Claim" submit button is a confirmed
      click no-op — same defect class as TOK-005)
- [x] TOK-012: Set token pricing and purchase tokens — BLOCKED (retested 2026-07-15: "Update
      Config" form reachable; TOK-005 blocks a real owned-token test)
- [x] TOK-013: Update token configuration — BLOCKED (retested 2026-07-15: "Set Price" reachable,
      correct owner-only authorization rejection)
- [x] TOK-014: Group actions for multi-party governance — PASS (retested 2026-07-15: clean
      empty states for contract/identity selectors, no crash)
- [x] TOK-015: View available token claims — PASS (retested 2026-07-15: "Fetch claims" works
      correctly, returns "No claims found" — contrast with TOK-011's broken button next to it)
- [x] TOK-016: Estimate perpetual token rewards — PARTIAL (retested 2026-07-15: reachable,
      returned an owner-only rejection that appears to contradict TOK-011's finding on the same
      token — flagged for follow-up, not asserted as a confirmed bug)
- [x] TOK-017: Pay for document operations with tokens — BLOCKED (retested 2026-07-15: Create
      Document / Purchase Document both now fully reachable with a real contract, but no
      token-payment UI option found in either flow explored)
- [x] TOK-018: Stop tracking a token balance — FAIL (retested 2026-07-15: "X" button confirmed
      click no-op on both the top-level and per-identity variants — same defect class as
      TOK-005/TOK-011; underlying persistence logic previously confirmed sound via source review)

## DOC

- [x] DOC-001: Register a new data contract — PASS (retested 2026-07-15: full E2E registration
      of "QA Note Contract" for QA Identity 1, owner ID cross-checked on-chain; unlocks DOC-005–009)
- [x] DOC-002: Update an existing data contract — FAIL — **application crash**: `.expect()` on
      `get_contracts()` panics on `WalletBackendNotYetWired`; app relaunched, zero persistent
      state lost
- [x] DOC-003: Import and manage contracts — Partial PASS (retested 2026-07-15: import-by-ID
      PASS with a genuinely new contract; "Remove cached contract" is a confirmed click no-op —
      a11y-verified exact coordinates, 4 attempts, zero dispatch)
- [x] DOC-004: Query and browse documents — FAIL (dispatches a real query that hangs silently
      forever, with a misleading ever-counting "Querying documents..." progress banner)
- [x] DOC-005: Create a document — PASS (retested 2026-07-15: full E2E create on "QA Note
      Contract", `note` document on-chain verified via documents query tool)
- [x] DOC-006: Replace or update a document — PASS (retested 2026-07-15: fetched existing
      document by ID, replaced `message`, same `$id` confirmed on-chain with new content)
- [x] DOC-007: Delete a document — PASS (retested 2026-07-15: deleted a scratch document,
      confirmed absent from a subsequent live query)
- [x] DOC-008: Transfer document ownership — PASS (retested 2026-07-15: QA Identity 1 → QA
      Identity 2 by raw Identity ID; required a purpose-built "QA Transfer Contract" with
      `transferable: 1` — the original "QA Note Contract" correctly rejects transfer per platform
      consensus rules since it never opted in)
- [x] DOC-009: Purchase a document and set document pricing — PASS (retested 2026-07-15: set
      price 100000000 credits on "QA Purchase Contract" doc, QA Identity 2 purchased at that
      price, on-chain `$ownerId` confirms both payment and ownership transfer; required
      `transferable: 1` + `tradeMode: 1` on the fixture contract)

## DEV

- [x] DEV-001: Decode state transitions — PASS
- [x] DEV-002: View proof request log — N/A (reclassified to Gap in the reconciled doc;
      original FAIL finding — no UI implementation, only a failure-only tracing target —
      consistent with this reclassification)
- [x] DEV-003: Inspect ZK proofs — FAIL (Proof deserializer works; GroveSTARK gen/verification deliberately hidden from all UI navigation)
- [x] DEV-004: View document and contract JSON — BLOCKED (Contract deserializer PASS; Document deserializer's contract-loading path blocked by known Testnet masternode-list/quorum-sync issue, see ALK.md)
- [x] DEV-005: View Platform info — FAIL (2/8 sub-tools work — Basic Platform Info, Validator Set Info; rest blocked by known masternode-list-sync issue)
- [x] DEV-006: View masternode list diff — N/A (reclassified to Removed in the reconciled
      doc; original FAIL finding — no UI implementation found — consistent with the removal)
- [x] DEV-007: Check any address balance — BLOCKED (address-format validation PASS; balance fetch blocked by known masternode-list-sync issue)
- [x] DEV-008: Mine blocks on Regtest — BLOCKED (Regtest-only, no regtest node running in this environment)

## NET

- [x] NET-001: Switch networks — PASS
- [x] NET-002: Auto-update from dashmate config — FAIL (no detection/import UI anywhere;
      `.env.example` requires the user to manually run `dashmate config get
      core.rpc.users.dashmate.password ...` and paste it in by hand)
- [x] NET-003: Configure Dash-Qt path — FAIL (`dash_qt_path` exists in the settings model
      with autodetection, but zero UI surface to view/edit/validate it; no `SystemTask`
      variant to update it)
- [x] NET-004: Select theme — PASS
- [x] NET-005: Unlock advanced features by interface mode — PASS (retitled/redefined from
      "Toggle developer mode" in the reconciled doc; original testing — Default view hides
      Masternodes nav + several Advanced Settings sections, Developer view adds a "Developer
      Tools" section, Expert view sits in between — already demonstrates the monotonic
      feature-unlock behavior this story now describes; carried forward as PASS, revisit only
      if a future pass wants to explicitly re-verify the "monotonic" wording)
- [x] NET-006: Select interface mode — PASS (same three labels/descriptions on Welcome
      screen and Settings card, confirmed live via a throwaway instance; choice applies
      immediately and persists across a full quit + cold-boot restart)
- [x] NET-007: Granular refresh controls — PASS (partial; only 2 modes exist —
      "Core + Platform" / "Platform Only" — not the 3 described in the story text; see note)
- [x] NET-008: Select Core backend mode — N/A (reclassified to Removed in the reconciled doc;
      original FAIL finding — explicitly retired in code, "chain sync is SPV-only now" —
      consistent with the removal)
- [x] NET-009: Toggle ZMQ — FAIL (`disable_zmq` field exists in settings model, zero UI
      surface, no `SystemTask` variant to update it)
- [x] NET-010: Onboarding wizard — PASS
- [x] NET-011: Wipe Platform data — BLOCKED (deliberately not run: destructive/irreversible
      against the campaign's shared, evidence-bearing data dir; the agent permission system
      independently halted the attempt and requires explicit human confirmation — see
      `scenarios/NET.md` and `summary-report.md` for details; test LAST alongside NET-019/020)
- [x] NET-012: Configure Devnet through the UI — N/A (Gap, not implemented)
- [x] NET-013: Testnet faucet integration — N/A (Gap, not implemented)
- [x] NET-014: Bulk fund addresses — N/A (Gap, not implemented)
- [x] NET-015: Use Dash Evo Tool without a local Dash Core node — PASS (with a UX note:
      the default-view global banner still says "SPV sync failed", leaking jargon the
      story says the everyday-user UI should avoid)
- [x] NET-016: Refresh Platform (DAPI) node list — PASS (control present on Mainnet/Testnet,
      confirmation dialog appears with correct wording, Cancel aborts cleanly with no side
      effects; note: a fast synthetic click can self-dismiss the dialog same-frame, a
      testing-methodology/robustness note, not a story-blocking defect — see scenarios/NET.md)
- [x] NET-017: View live connection status (indicator and Platform endpoints) — PASS
      (five-state top-panel indicator with hover tooltip confirmed; Connection Status panel
      shows jargon-free SPV/DAPI labels with the raw SPV error revealed only on hover)
- [x] NET-018: Auto-start SPV sync on startup — PASS (toggle persists across full quit +
      cold-boot restart in both directions; sync behavior matched the toggle exactly each
      time — restored to Enabled/baseline before finishing)
- [x] NET-019: Clear all local data for a network — BLOCKED (deliberately not executed:
      irreversible action against the campaign's shared, evidence-bearing data directory;
      requires explicit human authorization and a disposable copy of the data dir, consistent
      with NET-011's precedent; navigation to the control and its confirmation-dialog wording
      confirmed via live UI + source review — see scenarios/NET.md)
- [x] NET-020: Clear cached SPV data to force a resync — PASS (live-executed post-fix: unlike
      NET-011/NET-019, this action doesn't touch wallet/identity/contact data, only the SPV
      chain cache, so it's safe to run while other stories still need the live identity state.
      Confirmation dialog matched acceptance criteria exactly; clicking "Clear Data" produced
      "Cleared SPV data for Testnet. Reconnect to start a new sync."; confirmed on disk —
      block_headers/filters/filter_headers directories under spv/testnet/ were actually removed.
      Button correctly enabled while SPV was in its Error state, per source-confirmed gating
      logic already documented — see scenarios/NET.md)
- [x] NET-021: App settings preserved across an app upgrade — BLOCKED (no pre-upgrade legacy
      settings-storage fixture exists; source review of `legacy_settings.rs` and the
      `v093_upgrade.rs` composite regression test found strong evidence the feature is fully
      implemented and matches this story's acceptance criteria almost verbatim)

## MCP

- [x] MCP-001: Manage wallets via CLI — FAIL (imported wallets are invisible to every
      subsequent `det-cli` command — `core_wallets_list`/`core_address_create`/
      `core_balances_get` all return "Wallet not found" for a wallet imported by a prior
      process, or even by an earlier `already_imported:true` import in the same process;
      root cause confirmed in source: `ListWalletsTool` reads only the in-memory
      `ctx.wallets` map, which is never hydrated from the DB/vault outside the
      SPV-gated path)
- [x] MCP-002: MCP server access for AI agents — PASS (stdio via `det-cli serve` and HTTP
      via `det-cli headless` both verified: protocol lifecycle, bearer auth, session
      handling, network-mismatch guard, dynamic tool discovery all work correctly; carries
      the same wallet-hydration caveat as MCP-001 but that is a wallet-tooling defect, not
      a transport/protocol defect)
- [x] MCP-003: Load a masternode/evonode identity via CLI — BLOCKED (full happy path — no
      masternode/evonode fixture available); CLI plumbing tested clean with a fake
      ProTxHash/WIF: network-required and network-must-match both enforced, no key leakage
      in any output, clean parameter validation, SPV-gated dispatch behaves per docs (no
      crash/hang-without-progress)
- [x] MCP-004: Withdraw masternode/evonode credits via CLI — BLOCKED (no masternode/evonode
      identity loaded — MCP-003 prerequisite BLOCKED); tool schema confirmed to match the
      owner-key/payout-address restriction and fee-reporting acceptance criteria (supporting
      context only, not a live test)

## UX

- [x] UX-001: Blocking progress overlay for unsafe-to-interrupt operations — FAIL (component
      itself is correctly implemented and thoroughly unit-tested, but Send/broadcast — the
      story's own headline example — does not raise it, only DPNS registration does, per an
      explicit single-adopter "Bucket A" rollout scope cut)
- [x] UX-002: Blocking SPV-sync overlay with a "continue in the background" escape — PASS
      (every bullet live-confirmed via screenshots + timestamped logs: jargon-free text, Step N
      of 5, total input suppression, keyboard-only Enter/Tab+Enter dismissal, no re-raise for the
      rest of the episode, auto-lower-on-Error confirmed twice on cold-boot restarts)
- [x] UX-003: Global wallet/identity switcher across all tabs — FAIL (works correctly on the 3
      tabs that adopt it — Wallets, Identity Hub, Masternodes — but 4 of 7 root screens
      — Contracts, Tokens, Tools, Settings — render no switcher at all, not even the baseline
      wallet pill, contradicting "every root screen")
- [x] UX-004: One-time post-migration disclosure notice — N/A (Gap, not implemented)

## IDH

- [x] IDH-001: First-time identity setup — PASS (onboarding empty state matches every criterion;
      dev-mode footer confirmed present at Expert/Developer views, absent at Default, though
      currently non-interactive placeholder text pending a T6 wiring follow-up)
- [x] IDH-002: Identity home at a glance — BLOCKED (no identity reachable, see scenarios/IDN.md;
      source review confirms IdentityHeroCard/OnboardingChecklist/HomeOutcome are wired)
- [x] IDH-003: Multi-identity switching — BLOCKED (multiple identities unreachable; source review
      confirms BreadcrumbPill/IdentityPill/IdentityPickerCard exist; see UX-003 for switcher
      rollout gaps)
- [x] IDH-004: Opt in to DashPay social profile — BLOCKED (no identity reachable; source review
      confirms SocialProfileGateCard and the Home/Settings surfaces are wired)
- [x] IDH-005: Bulk identity creation — N/A (Gap, not implemented)
- [x] IDH-006: Unified activity timeline — N/A (Gap, not implemented)
- [x] IDH-007: Manage contacts from the Identities hub — BLOCKED (no identity/contacts reachable;
      source review confirms Accept/Decline/Cancel/search+Pay/hidden-by-default are wired; see
      DPY-014)
- [x] IDH-008: Name an identity on this device — BLOCKED (no identity reachable; source review
      confirms this is the same set_identity_alias mechanism DPN-008 already found wired, exposed
      via a second Settings-tab UI surface)

## MN

- [x] MN-001: Load a masternode by keys — FAIL (disabled-gate/malformed-hash/unencrypted-note/
      Fill-Random gating all correct, but "Load masternode" with a well-formed fake ProTxHash
      still hangs silently — same defect class as IDN-003, re-confirmed fresh with 20s wait; new
      `det.log` evidence implicates the known wallet-backend blocker as a likely contributing
      cause)
- [x] MN-002: See my masternodes at a glance — PASS on directly-testable scope (empty state
      correctly explains the concept + CTA; Expert-view-only nav gating live-confirmed both ways
      — hides on Default view, restores on Expert view); card-list-with-real-nodes content and
      the literal same-frame de-gating trigger untested (no loaded node; architecturally
      unreachable via mouse-only UI respectively) — noted as untested scope, not failures
- [x] MN-003: Open a masternode and vote — BLOCKED (no loaded masternode reachable — MN-001's
      hang; DPNS-voting UI structurally confirmed via source review only)
- [x] MN-004: Remove a masternode — BLOCKED (same reasoning as MN-003; confirm-before-remove
      dialog structurally confirmed via source review only)
- [x] MN-005: Keep the everyday surface clean — PASS (legacy "Load Existing Identity" screen's
      Identity Type selector now offers User only and its ProTxHash tab is gone entirely — clean
      regression fix vs. IDN-003's prior finding of a Masternode/Evonode toggle there)
- [x] MN-006: Encrypt my node keys at load time — BLOCKED (cannot observe an actual encrypted
      load — MN-001's hang; load-time "Encryption password (optional)" field already confirmed
      present and correctly worded while testing MN-001)
- [x] MN-007: Withdraw a node's credits — BLOCKED (no loaded masternode reachable; Withdraw
      button routing to the shared withdrawal screen confirmed via source review only)
- [x] MN-008: Manage a node's keys — BLOCKED (no loaded masternode reachable; add-key purpose
      selector structurally excludes OWNER/VOTING for every identity type, confirmed via source
      review only)
- [x] MN-009: Claim an evonode's token rewards — BLOCKED (no loaded Evonode reachable;
      Evonode-only "Claim token rewards" gating confirmed via source review only)
- [x] MN-010: Keep the Masternodes tab consistent across a network switch — PASS (unsubmitted
      Evonode + fake ProTxHash + alias in the Load form was fully discarded on a Testnet→Mainnet
      switch, landing on a clean empty List view with zero leftover input; stale per-network
      banners also cleared; app restored to Testnet afterward)
- [x] MN-011: Refresh masternode and voting state — BLOCKED overall (core node-refresh behavior
      needs a loaded node, unreachable), with a positive no-op-safety data point: the Refresh
      control exists and is a confirmed-safe no-op with zero nodes loaded, matching the story's
      own no-op requirement and the source's explicit early-return on an empty node list
- [x] MN-012: Switch wallet/identity from the Masternodes header — PASS on directly-testable
      scope (header renders the 3-segment switcher with the exact `(no masternode yet)`
      placeholder text, corroborated by UX-003's independent prior finding on this same build);
      node-picking / cross-page-identity-isolation behavior untested — no loaded node to pick
