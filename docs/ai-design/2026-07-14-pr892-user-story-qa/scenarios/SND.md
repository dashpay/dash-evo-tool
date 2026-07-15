# SND — Send and Receive

Environment: PR892 build, isolated data dir `/data/tmp/det-qa-pr892-data`, network Testnet.

## SND-001: Send Dash to an address — PASS (navigation confirmed; full send flow pending)

Clicking "Send" on the Wallet screen navigates to a dedicated "Send Dash" screen
(breadcrumb: `Wallets > Send Dash`) with: "Send from" (Core Wallet / Identity radio-style
selector, shows live balance), "Send to" (a combined `type:core|platform|identity` address
field), "Amount (DASH)" with a "Max" button, "Advanced Options" toggle, Cancel/Send buttons.
Screen renders correctly and reflects the funded balance. Full end-to-end send (submitting a
real transaction) deferred to a later pass once more of the campaign's funding needs are
known — the screen itself is confirmed functional and correctly wired to the wallet.

Verdict: **PASS** (screen navigation and layout confirmed; a completed on-chain send to
close the loop is still pending — will revisit).

## SND-003: Receive Dash with QR code — **FAIL**

Steps to reproduce:
1. Load a wallet with existing balance (`QA Wallet 1`, Testnet, 3 DASH), Wallet screen,
   Expert view.
2. Click the "Receive" button (next to "Send", top of the wallet detail screen).

Expected (per story acceptance criteria): a QR code encoding the receive address should be
shown so a sender can scan it.

Observed: **nothing happens**. The button gets a keyboard-focus outline (blue border) but:
- No modal or panel opens.
- No screen navigation occurs (breadcrumb stays `Wallets > QA Wallet 1`, unlike "Send" which
  correctly navigates to `Wallets > Send Dash`).
- No new log line appears in `det.log` at the time of the click (compared against "Send",
  which does not log either, but *does* visibly navigate — so the absence of a UI change is
  the actual signal here, not the absence of a log line).

Reproduced 3 times from a clean state (cancelling out of the Send screen each time, then
clicking Receive fresh) — consistent, not a one-off render glitch.

Workaround available: the wallet's live address table ("Addresses (Dash Core)" section,
WAL-011) does expose receive addresses as copyable text, and funding via that address works
correctly (used successfully to receive testnet faucet funds for this campaign) — so the
underlying receive-address mechanism is not broken, only the dedicated QR-code UI entry
point via the "Receive" button.

Screenshot: `SND-003-1-receive-button-inert.png`.

**Verdict: FAIL.** Severity: Medium — feature works around (address table), but the
documented/expected QR-code receive flow (SND-003's whole reason for existing — QR is the
"receive Dash" UX for users copying addresses on a phone) does not work at all in Expert view
on this build. Not tested yet whether Default view exposes it differently — worth a follow-up
check in Default view before final triage.

---

## SND-001 addendum: full end-to-end send completed — confirmation dialog is missing

The original SND-001 write-up deferred a full on-chain send. This pass completed it (see
SND-005/SND-006 below for the transactions) and found a result worth flagging against
SND-001's own acceptance criteria: **"Confirmation dialog before broadcast."**

Observed: in both the simple Send form and the Advanced Options form, clicking
"Send DASH" / "Send" **broadcasts immediately** — there is no confirmation step of any
kind (no "Are you sure?" dialog, no fee/total review screen). The very next frame shows
the "Sent X DASH to Y" success screen. Reproduced on 4 separate sends (0.001 DASH single
recipient, 0.003 DASH to 2 recipients, 0.02 DASH single recipient, plus the SND-006 test
below) — consistent every time, not a timing fluke.

This does not change SND-001's already-recorded PASS (screen navigation/wiring is
correct, and "Enter destination address and amount" works), but the second acceptance
criterion — a confirmation dialog before broadcast — does not hold in this build. See
SND-005 below, which fails for the same underlying reason (no pre-broadcast review step
exists to show a fee estimate in).

## SND-002: Send Dash from single-key wallet — reclassified N/A (Gap) in the corrected catalog

**Reconciliation note**: PR892's real catalog (`docs/user-stories.md` in the PR892-build
worktree, not the doc originally used for this campaign's first pass) tags this story
`[Gap]`, not `[Implemented]`. The FAIL finding below — sending is explicitly and
consistently disabled for single-key wallets, with a dedicated typed error
(`SingleKeyWalletsUnsupported`) — is fully consistent with that reclassification: this is a
genuinely unimplemented feature, not a bug in an implemented one. `progress.md` now tracks
this as N/A; the write-up below is kept as-is since it's still the accurate, evidence-backed
description of current behavior.

## SND-002 (original write-up, kept for evidence): Send Dash from single-key wallet — FAIL (product limitation, explicit typed error)

Steps:
1. Generated a fresh zero-balance receiving address on `QA Wallet 1` (index 61,
   `yiaMw5rBDXSP1PkPeopwUyNhDJq1QonxmG`) and sent it 0.02 DASH from the same wallet (to
   give the eventual single-key wallet a balance to test sending from).
2. Exported the address's WIF via "View Key" → "Copy Key".
3. "Import key (advanced)" → pasted the WIF. Derived address matched exactly
   (`yiaMw5rBDXSP1PkPeopwUyNhDJq1QonxmG`, "This is a Testnet address."). Nickname
   "SND-002 Single Key Test", no passphrase protection.
4. Clicked "Add to wallets" — "SK: SND-002 Single Key Test" appeared in the wallet
   selector immediately.

Observed: same banner as WAL-003 documented — *"Sending from a single-key wallet is not
available in this version. You can still receive funds at this address. To send these
funds, import them into a recovery-phrase wallet."* — Send control present but
inert/disabled. Clicking the top-level "Refresh" button while this wallet was active
surfaced a stronger, explicit **typed error banner**: *"Single-key wallets are not
supported in this version. Your single-key wallet data is preserved and will work again
in a future update. To manage funds now, use an HD (recovery-phrase) wallet."* — "Show
details" revealed the technical error code: `SingleKeyWalletsUnsupported`.

This confirms the limitation is a deliberate, explicitly-typed product decision (not a
crash or silent bug), and answers SND-002's acceptance criterion directly: **"Send flow
works the same as for HD wallets"** does **not** hold — sending is completely disabled
for single-key wallets, whatever the balance.

Cleanup: removed "SK: SND-002 Single Key Test" via "Remove" — deleted instantly with
**no confirmation dialog** (same missing-confirmation bug WAL-007 found for single-key
wallet removal; not re-litigated here). No funds were lost — the underlying address is
also derived by `QA Wallet 1`'s own HD tree, so the 0.02 DASH balance remained spendable
by the HD wallet after the SK entry was removed.

Verdict: **FAIL** against "Send flow works the same as for HD wallets" — this is a
confirmed, clearly-communicated product limitation, consistent with WAL-003's finding.

## SND-005: See fee estimate before confirming send — FAIL

Steps: exercised both the simple Send form and the Advanced Options form (Core Wallet
source, `QA Wallet 1`), looking for any fee estimate or amount+fee breakdown prior to
broadcast.

Observed:
- Neither form shows a fee estimate, total-deduction breakdown, or any confirmation step
  at any point before the transaction is broadcast (see SND-001 addendum above — there is
  no confirmation dialog at all to show a fee estimate in).
- The "Max" button *does* silently account for a fee internally — clicking it with
  `Send to` = a wallet address and available balance `2.99999288 DASH` filled the amount
  field with `2.99998046 DASH`, meaning a `0.00001242 DASH` fee was deducted — but this
  number is never surfaced to the user as a labeled "fee" anywhere in the UI. A user
  would have to manually subtract the two numbers themselves to discover it.
  Screenshot: `SND-005-1-no-fee-breakdown-max-silently-deducts.png`.
- Checked the post-hoc Transaction History table too: its "Fee" column is populated with
  `-` (a dash placeholder) for every transaction, including confirmed ones — so the fee
  isn't surfaced after the fact either, not just before confirming.

None of the acceptance criteria hold: no fee estimate shown in (a nonexistent)
confirmation dialog, no explicit total-deduction display, and no transaction-size/fee
breakdown for either single-key or HD wallets (single-key sending is disabled per
SND-002, so that half of the criterion is moot regardless).

Verdict: **FAIL**.

## SND-006: Send to multiple recipients — PASS

Steps:
1. Send Dash → "Advanced Options" → confirmed "Outputs (Send To)" section supports
   multiple rows: clicking "+ Add Output" appended a second `To:`/`Amount:` row with its
   own "X" remove button (tested add and remove).
2. Filled a real 2-recipient transaction: input `yYCWtyP2mSLzGkZqL9a6G5rpPQQRs1fT5f`
   (2 DASH, via "+ Add Core Address"), outputs `yLRfPRuzq9VzUVLyx44c4ATXWaV1isZdpC`
   (0.001 DASH) and `yZjRFx4KmGB3h36LGbf4xSAzK51cU1hQML` (0.002 DASH) — both destinations
   are `QA Wallet 1`'s own zero-balance addresses, used deliberately as a self-transfer
   so the test costs only the network fee.
3. Clicked "Send".

Observed: success screen read **"Sent 0.003 DASH to 2 recipients"** — a single combined
confirmation, not two separate ones. Confirmed in Transaction History: exactly **one**
new "Sent" row (txid `4574133706f0a3c479ac34aa4ea1d880af546395999f439aaebe3...`),
InstantSend, net wallet-balance change `-0.0000026 DASH` (i.e. only the network fee —
both outputs landed on addresses the same wallet already owns) — proving both outputs
were part of one broadcast transaction, not two.

All acceptance criteria met: add/remove recipients in a list (confirmed), per-recipient
address and amount (confirmed), single transaction broadcast (confirmed).

Verdict: **PASS**. Note: the story text says "As a user with a single-key wallet..." but
the actual UI wires multi-recipient support into the **Advanced Options** panel of the
regular (HD) Send screen, not anything single-key-specific — since single-key sending is
disabled entirely (SND-002), this is presumably just imprecise story wording; the
underlying capability (multiple outputs, one broadcast) works correctly for HD wallets.

## SND-007: Shield DASH from Core wallet — FAIL

Steps:
1. Switched Interface mode to Developer view (Settings → Networks → Interface mode) —
   required per the story's acceptance criteria.
2. Wallet screen → "Shielded" tab → copied the wallet's own shielded address
   (`tdash1zpzmpc25xp0x3g...pp4cvs6cca9x`) via "Copy".
3. Send Dash → pasted the shielded address into the simple "Send to" field.

Observed (step 3): rejected immediately with inline red text **"This address type is
not accepted here."** — the simple combined field only recognizes `type:core|platform|
identity`, not shielded destinations. Screenshot:
`SND-007-2-simple-field-rejects-shielded-address.png`. This happens in **Expert** view
too, not just Developer view — Developer mode is not actually gating this rejection.

4. Switched to Advanced Options — the "To:" field's placeholder explicitly lists
   `tdash1...` as a valid prefix, and pasting the shielded address there **is**
   recognized: it shows a green `(Shielded)` type tag next to the address. Filled a
   Core-Wallet input (`yQYhM8SS8H2JTaNA516qPDxBZLWa1giqWT`, 0.005 DASH) and the shielded
   output (0.005 DASH), then clicked "Send".

Observed (step 4): **fails every time** (reproduced twice) with the banner **"Invalid
output address"** — confirmed in `det.log`:
```
17:13:47.312789Z ERROR dash_evo_tool::ui::components::message_banner: Banner displayed banner="Invalid output address"
17:14:15.778190Z ERROR dash_evo_tool::ui::components::message_banner: Banner displayed banner="Invalid output address"
```
No asset-lock or backend-task activity appears in the log around either attempt — the
rejection happens at client-side validation, before any wallet-backend call. No funds
were lost (`QA Wallet 1`'s "Asset Locks" section stayed empty; balance unaffected beyond
the always-present, already-tested self-transfer fees from other stories).

**Root cause disclosed elsewhere in the UI**: the wallet's own Shielded tab states
outright, directly under the shielded address field: *"Shielded sending is not available
on this network yet. You can still view your shielded balance and receive address."*
Screenshot: `SND-007-1-shielded-sending-not-available-notice.png`. This is the accurate
explanation — but it is never surfaced in the Send screen itself when the shielded
destination is rejected, so a user hitting "Invalid output address" there has no way to
learn why without separately visiting the Shielded tab. Also worth noting: the "Shielded
Notes" section on that same tab says "Note history is managed by the upstream
platform-wallet coordinator and will be surfaced here in a future update," consistent
with shielded transacting being a known, not-yet-wired capability on this network rather
than a one-off bug.

Verdict: **FAIL**. Even though the underlying cause is a disclosed, known limitation
("not available on this network yet"), the story is marked `[Implemented]` in
`docs/user-stories.md` and none of its acceptance criteria are met — no asset lock is
ever created, no shielding occurs, and the error message shown at the point of failure
doesn't explain the real reason to the user.

## SND-008: Top up identity from Send screen — BLOCKED (partially verified)

**Reasoning**: no identity exists yet in this environment (`Identities` screen shows the
empty "Welcome to Identities" state — the IDN category has not run in this QA campaign
chain). Full completion needs (a) a real identity ID to top up, and (b) — per WAL-017 —
the Core-Wallet-source path almost certainly routes through the same asset-lock
transaction builder that's confirmed broken ("No UTXOs available for selection" despite
a funded wallet), so even with a real identity ID this would likely fail the same way
SND-007 and WAL-017 did.

What **was** verified: the Send screen's combined "Send to" field correctly recognizes a
well-formed Base58 identity ID as a valid destination. Typing a 44-character Base58
string into "Send to" (Core Wallet source) produced a green `Identity` type tag, and the
form auto-updated **"Transaction type: Top Up Identity"** with the primary button
relabeling to "Top Up Identity". This confirms the acceptance criteria "Enter an
identity ID (Base58) as destination" and "System uses appropriate backend task" are
correctly wired for the Core-Wallet-source case, at the UI-recognition level.

Deliberately did **not** submit this test transaction — the identity ID used was
fabricated (not a real on-chain identity), and clicking through risks either (a) an
asset-lock-builder failure identical to WAL-017 (uninformative, already documented) or
(b) — worse — actually succeeding at creating an asset lock addressed to a
non-existent identity, which would permanently burn real tDASH with no way to recover it.
Not worth the risk for a fixture-less identity ID.

Also confirmed: with Platform Addresses source disabled (WAL-017: "no Platform addresses
with balance"), the Platform-source half of this story ("direct for Platform") cannot be
exercised at all in this environment either.

Verdict: **BLOCKED** — UI wiring for the Identity destination confirmed correct;
end-to-end completion blocked by (1) no identity fixture exists yet (IDN category not
run), and (2) the Core-Wallet source path is downstream of WAL-017's asset-lock bug.

## SND-009: Shield credits from Platform address — BLOCKED

**Reasoning**: identical root cause to WAL-019/WAL-020 — Send Dash → Advanced Options →
Source Type → "Platform Addresses" is disabled with the inline note "(no Platform
addresses with balance)", a direct consequence of WAL-017's asset-lock coin-selection
bug (Platform balance is permanently 0 in this environment, so it can never be selected
as a source). Cannot even open this flow, let alone reach the "auto-selects the
highest-balance Platform address" behavior the story describes. Compounded by SND-007's
finding that shielded destinations are rejected outright regardless of source.

## SND-010: Withdraw from shielded pool to Core address — BLOCKED

**Reasoning**: two independent blockers. First, Shielded balance is permanently 0 DASH
in this environment (nothing can ever reach the shielded pool — SND-007's "Invalid
output address" bug and WAL-017's asset-lock bug both prevent it, and the app's own
Shielded tab states "Shielded sending is not available on this network yet"). Second,
even setting balance aside, the Send screen's "Source Type" selector (Advanced Options)
only ever offers two options — "Core Wallet" and "Platform Addresses" — no "Shielded
Pool" option is exposed anywhere in this build, in Developer view or otherwise, so there
is no UI path to even attempt this story regardless of balance. Developer mode
(Settings → Networks → Interface mode) was confirmed active during this check.

## SND-011: Transfer identity credits to another identity — BLOCKED

**Reasoning**: no identity exists yet in this environment (see SND-008 — `Identities`
screen empty state confirms the IDN category hasn't run). Partially verified UI
reachability: the Send screen's "Send from" selector shows an "Identity" radio option
alongside "Core Wallet", but clicking it has no effect — the selection stays on "Core
Wallet" — because there are zero loaded identities for it to draw from. This matches the
story's own acceptance criterion, "Select Identity as source from **dropdown of loaded
identities**" — with no identities loaded, there is nothing to select. Cannot test
further until IDN-001 (or another identity-creation story) runs first.

## SND-012: Withdraw identity credits to Core address — BLOCKED

**Reasoning**: identical to SND-011 — requires "Select Identity as source," which is
unreachable with zero loaded identities. Same UI verification applies (Identity radio
present but inert). Cannot test until an identity exists.

## SND-013: Transfer identity credits to Platform address — BLOCKED

**Reasoning**: identical to SND-011/SND-012 — requires "Select Identity as source" plus
a Platform-address (bech32m) destination. The source-selection blocker alone is
sufficient to block this story regardless of the destination side. Cannot test until an
identity exists.

---

*SND category status: SND-001 through SND-013 all now checked in `progress.md`.
Confirmed FAILs: SND-002 (single-key send disabled, typed error), SND-003 (Receive
button inert — from the earlier pass), SND-005 (no fee estimate/confirmation dialog
anywhere pre-broadcast), SND-007 (shielded destinations rejected — "Invalid output
address" — root cause disclosed as "not available on this network yet"). Confirmed
PASS: SND-001 (nav), SND-006 (multi-recipient, single broadcast). BLOCKED (all with
specific, non-speculative reasoning): SND-008/009/010 (Platform/shielded balance can
never be funded — WAL-017 and SND-007 root causes) and SND-011/012/013 (no identity
exists yet — IDN category not run). Final app state left by this pass: network Testnet,
Expert view, `QA Wallet 1` intact at 2.99999288 DASH (Core), no leftover throwaway
wallets.*

## Environment note for SND-014/015/016 (this pass)

This pass hit the same unresolved Testnet wallet-backend blocker documented in
`scenarios/ALK.md` and `scenarios/DEV.md`, and re-encountered by the immediately
preceding agent testing WAL-025–029: on launch, four persistent red banners appeared
("We couldn't finish preparing your wallet.", "SPV sync failed.", "Your wallet is still
starting up.", "Could not load your identities from this device.") and `det.log` showed
`Wallet backend initialization deferred error=Could not access wallet data. Check
available disk space and restart the application.` repeatedly, with the Send screen's
"Show details" on the "still starting up" banner revealing the structured cause
`WalletBackendNotYetWired`. `QA Wallet 1`'s Core balance displayed as `0 DASH` for the
entire session (not the ~2.99999288 DASH left by the prior pass) as a direct symptom —
per instructions, this was not restart-looped or "fixed"; a single non-destructive
top-bar "Refresh" click was tried once and made no difference, consistent with ALK.md's
finding that this failure is currently ~100% reproducible in this data dir. Where the
live UI could not be exercised, PR892's source (`/data/git-worktrees/
home-ubuntu-git-dash-evo-tool-2-pr892-build`) was read directly to determine what the
UI does when reachable — cited inline below with file:line references. Final app state
was left clean (Cancel on the Send form, no broadcast, no wallet changes).

## SND-014: Send maximum from a Core wallet — FAIL

Steps: Wallet screen (`QA Wallet 1`, Testnet) → Send → "Send to" = a Core address
(`yYCWtyP2mSLzGkZqL9a6G5rpPQQRs1fT5f`, recognized as "Wallet address") → clicked "Max".

Observed (live): the Amount field stayed completely empty (placeholder "Enter amount"
still showing) — no value, no fee figure, no message of any kind appeared next to the
field or anywhere else on the screen. `det.log` shows no new line at all around the
click (Max is purely client-side here — nothing dispatched). Screenshot:
`SND-014-1-max-empty-no-message-env-blocked.png`. Because `QA Wallet 1`'s Core balance
displayed as `0 DASH` this session (see environment note above), this result cannot by
itself distinguish "balance genuinely too low" from "balance never loaded" — so the
live click alone does not settle the story's first two bullets. Source review below
does settle them.

**Source-level findings** (`src/ui/wallets/send_screen.rs`,
`src/model/fee_estimation.rs`, `src/ui/components/amount_input.rs`):

- Bullet 1 ("Max sets amount to balance minus fee") — **the underlying math is
  implemented correctly.** `core_max_send_amount_duffs()` / `core_max_send_reserve_duffs()`
  (`model/fee_estimation.rs:1025-1054`) compute `balance − estimated_L1_fee`, scaled by
  UTXO count, and the Core-to-Core branch of `render_amount_input()`
  (`send_screen.rs:2264-2306`) wires this in: on success it sets
  `max = Some(send_amount_duffs)` and builds a hint string
  `"~{fee} reserved for the network fee"`; on failure (balance can't cover the fee) it
  sets `max = None` with hint `"Your balance is too low to cover the network fee."` —
  this is precisely the story's own spec, including the exact "reserve the fee, show a
  calm message" language from `core_max_send_amount_duffs`'s own doc comment.
- Bullet 2 ("fee reserved is shown next to the amount") — **FAILS, structurally, not
  just as an observed gap.** Both hint strings above are threaded only into
  `AmountInput::set_max_exceeded_hint()` (`send_screen.rs:2413`). Reading
  `amount_input.rs:280-312`, that hint is used in exactly one place: inside the
  `Err(...)` branch of `validate_amount()`, appended to an "Amount X exceeds maximum Y"
  message — and *only* when the currently-typed amount is strictly greater than
  `max_amount`. Clicking "Max" sets `amount_str` to *exactly* `max_amount`
  (`amount_input.rs:346-351`), so `amount.value() > max_amount` is false and the error
  branch never fires. The fee-reserved label is therefore dead code from the user's
  perspective on the normal "click Max, see the result" path — it can only ever appear
  if the user manually types a number bigger than what Max would have filled in, wrapped
  inside a validation-error sentence rather than a clean fee label. This matches, and
  root-causes, SND-005's finding that Max "silently deducts a fee" that's "never
  labeled/shown anywhere in the UI."
- Bullet 3 ("too low → no amount + calm message") — **half holds, half fails.** "No
  amount" is correct: when `core_max_send_amount_duffs` returns `None`, `max_amount`
  stays `None` and the Max-button code path in `amount_input.rs:352-355` does not set
  `amount_str` at all, matching the live observation of an empty field. But the "calm
  message explains why" half fails for the identical structural reason as bullet 2: the
  "Your balance is too low..." hint is stored in the same `max_exceeded_hint` field,
  whose only rendering site is gated by `if let Some(max_amount) = self.max_amount` —
  when `max_amount` is `None` (the exact case this message is meant to explain), that
  `if let` never matches, so the message can *never* render, under any input, in this
  case. It is unreachable code from the UI's perspective, not merely untriggered in this
  session.

**Verdict: FAIL.** The reserve-the-fee math (bullet 1) is implemented correctly and
matches the story's intent, but bullets 2 and 3's messaging half are both dead code —
confirmed by reading the exact rendering path, not merely inferred from one session's
balance-unavailable state. Live confirmation of the *positive* path (a genuine non-zero
balance producing a filled, fee-reduced amount) was prevented by this session's
`WalletBackendNotYetWired` environment blocker, but that blocker does not affect this
verdict: the messaging gap exists in the code regardless of whether any balance ever
loads. No transaction was broadcast; the form was cancelled after this test.

## SND-015: Unshield credits to a Platform address — FAIL

**What was checked**: the Shielded tab specifically (not the generic Send screen's
Source Type selector, which SND-009/010 already found has no "Shielded Pool" option) for
a dedicated "Unshield" button, per this story's distinct entry point.

**Live**: navigated to Wallet screen → Shielded tab. The tab never advanced past
`is_initialized == false` for the entire session — it showed only a spinner and
"Preparing shielded wallet..." (or, when the wallet-lock state was checked, "Unlock the
wallet to enable the shielded pool.") — the same stuck state WAL-029 already documented
for this data dir. No action buttons of any kind (Shield / Send (Private) / Unshield)
ever rendered. Screenshot:
`SND-015-016-1-shielded-tab-stuck-preparing-env-blocked.png`. Root cause: the same
`WalletBackendNotYetWired` blocker described in the environment note above — the tab's
`ui()` method returns early whenever `!self.is_initialized`
(`src/ui/wallets/shielded_tab.rs:528-558`), before the action-buttons block is ever
reached, so this session could not distinguish "button exists but is disabled" from
"button doesn't render at all" purely from the live screen.

**Source review settles it** (`src/ui/wallets/shielded_tab.rs`,
`src/context/feature_gate.rs`): the "Unshield" button *does* exist in code
(`shielded_tab.rs:679-694`), fills exactly the role the story describes — it calls
`self.open_send_flow(SendFlow::Unshield)`, which opens
`ScreenType::WalletSendScreen(wallet, SendFlow::Unshield)`, the same unified Send screen
used everywhere else, preset with `SendFlow::Unshield.preset_destination_kinds() ==
[Platform, Core]` and heading "Unshield Credits" (`send_screen.rs:75-118`); the source
is auto-locked to the wallet's shielded pool via `sync_flow_state()`
(`send_screen.rs:1200-1206`, `SourceSelection::Shielded(seed_hash, balance)`). This is
exactly "Select Shielded Pool as source and enter a Platform address as destination,"
correctly wired.

However, the entire action-button row — including "Unshield" — is only rendered when
`FeatureGate::ShieldedOperations.is_available(&self.app_context)` is true
(`shielded_tab.rs:630`); otherwise the tab shows `SHIELDED_OPERATIONS_UNAVAILABLE_LABEL`
("Shielded sending is not available on this network yet...") in its place
(`shielded_tab.rs:711-717`) — the exact text SND-007 already found on this same tab in a
prior, successfully-initialized session. That gate requires
`Capability::ShieldedProtocol`, which is controlled by
`SHIELDED_ACTIVATION_PROTOCOL_VERSION: Option<u32> = None` — a **hardcoded compile-time
constant** in `feature_gate.rs:18`, with an explicit doc comment: "Not shipped anywhere
yet, so no network can offer it" / "unmet on every network." This is not a per-session
or per-network runtime condition — it means the "Unshield" button cannot be shown to any
user, on any network, in this exact build, until upstream ships the shielded state
transitions and this constant is changed. SND-007's independent, earlier-session
observation of the "not available on this network yet" label corroborates this reading
of the code.

**Verdict: FAIL.** Distinguishing per this story's instructions: this is not "button
doesn't exist" (the code is present and correctly implemented for the eventual
capability) and not simply "button exists but blocked by 0 balance" (balance is
irrelevant here — the entire row is gated off before balance is even considered). The
accurate characterization is: the button exists in source and is correctly wired to the
unified Send screen preset, but is unconditionally hidden behind a hardcoded
not-yet-activated protocol-capability gate in this build, so it is never reachable by a
live user on any network today — a deliberate, disclosed limitation, but still a FAIL
against the story's "reachable from the Shielded tab's Unshield button" acceptance
criterion, consistent with the FAIL verdict already recorded for the analogous SND-007
shielded-destination gap. The balance-decrease/Platform-balance-increase bullet is moot
on top of this — shielded balance is documented permanently 0 in this environment
regardless (SND-009/010).

## SND-016: Send privately within the shielded pool — FAIL (with one confirmed correct sub-behavior)

**What was checked**: the Shielded tab specifically for a dedicated "Send (Private)"
button, per this story's distinct entry point, and — since it was reachable via source
even though not via the live UI — whether the spend-lock/verification-in-progress
behavior described in the third bullet is actually implemented.

**Live**: identical situation to SND-015 — the Shielded tab stayed stuck at "Preparing
shielded wallet..." (`is_initialized == false`) for the whole session, so no button,
disabled or otherwise, ever rendered. Same screenshot:
`SND-015-016-1-shielded-tab-stuck-preparing-env-blocked.png`.

**Source review** (`src/ui/wallets/shielded_tab.rs:656-710`): the "Send (Private)"
button exists, calls `self.open_send_flow(SendFlow::ShieldedSend)`, which opens the
unified Send screen preset with heading "Send (Private)"
(`send_screen.rs:89`), destination locked to `[Shielded]`
(`send_screen.rs:115`), and source auto-locked to the shielded pool via
`sync_flow_state()` — matching bullets 1 and 2 exactly, same as SND-015. It is subject
to the identical hardcoded `FeatureGate::ShieldedOperations` gate described in SND-015
above (`SHIELDED_ACTIVATION_PROTOCOL_VERSION = None`), so it is likewise never reachable
by a live user in this build today.

**Bullet 3 ("Spending is paused until the shielded balance is verified, and the button
is disabled with a clear reason while verification is in progress") — confirmed
correctly implemented in source, independent of the reachability gate above.** In
`shielded_tab.rs:630-696`: `spend_locked` is derived from the migration-status-driven
`ShieldedIndicator` (`Verifying` or `Failed` → locked); the "Send (Private)" button is
rendered via `ui.add_enabled(can_spend, send_btn)` where
`can_spend = !syncing && tree_synced && shielded_balance > 0 && !spend_locked`, and its
hover tooltip is the constant `SHIELDED_SPEND_LOCKED_TOOLTIP = "Spending paused until
shielded balance is verified."` whenever `spend_locked` is true; a second, always-visible
row below the buttons additionally shows a lock icon plus
`SHIELDED_SPEND_LOCKED_LABEL = "Spending paused."` in the same state
(`shielded_tab.rs:700-709`) — a dedicated accessibility test,
`tc_a11y_006_locked_spend_state_uses_icon_and_text`, guards that both the icon and the
text are always present together (not colour alone). This is a precise, well-built match
for the story's third bullet — a genuinely good implementation — it just cannot be
observed live today because the button row it lives on is itself gated off (see above).

**Verdict: FAIL** overall, for the same "not reachable by any live user in this build"
reasoning as SND-015 — the button exists and is correctly wired to the unified Send
screen preset (bullets 1–2), and its spend-lock behavior is correctly implemented
in source (bullet 3), but none of it is currently visible or clickable because
`FeatureGate::ShieldedOperations` is hardcoded closed pending an upstream protocol
version that doesn't exist yet. Recorded as FAIL rather than PASS because the story is
tagged `[Implemented]` and none of its criteria are actually observable/usable by a live
user today; recorded as FAIL rather than BLOCKED because the reachability gap is a
deterministic, compile-time condition (not a flaky environment issue) that source review
settles conclusively even though this session's own `WalletBackendNotYetWired` blocker
independently prevented reaching that gate live.

---

*SND-014/015/016 addendum (this pass): all three verdicts are **FAIL**. SND-014: the
Max-button fee math is correct but the fee-reserved label and the low-balance message are
both dead code in the render path (confirmed by source, not just by one session's
zero-balance state) — root-causes SND-005. SND-015/016: the Shielded tab's dedicated
"Unshield" and "Send (Private)" buttons exist and are correctly wired to the unified Send
screen preset (and, for SND-016, the spend-lock/verification-in-progress UX is correctly
implemented), but the entire action-button row is unconditionally hidden behind a
hardcoded `SHIELDED_ACTIVATION_PROTOCOL_VERSION = None` capability gate — consistent
with, and root-causing, SND-007's "not available on this network yet" finding. This
pass's own live testing was additionally constrained by the unresolved Testnet
`WalletBackendNotYetWired` wallet-backend blocker (see `scenarios/ALK.md`/`DEV.md`,
re-encountered by the WAL-025–029 pass immediately prior) — `QA Wallet 1`'s Core balance
showed `0 DASH` all session and the Shielded tab never finished initializing — so live
UI evidence was supplemented with direct source review throughout. No funds were moved;
no transaction was broadcast; the app was left in a clean state (Send form cancelled,
Wallets > QA Wallet 1, Shielded tab).*

---

## SND-009 retest (2026-07-15, post wallet-backend fix): Shield credits from Platform address — FAIL

**Environment**: retested against the same live app instance (PID 2216703) as the WAL-018
through WAL-029 third pass in `WAL.md` — Testnet fully synced, no wallet-backend blocker.
Switched Interface mode to **Developer view** first (Settings > Networks), per this story's
own "Developer mode required" criterion.

Steps:
1. Wallets > `QA Wallet 1` > Send > Advanced Options > Source Type: Platform Addresses.
   Clicked "+ Add Platform Address" — the dropdown listed only the wallet's two
   balance-holding Platform addresses, with the higher-balance one
   (`tdash1kp30ae9x752z7wu20j4m4y945449anlhtqqe9h4l`, 0.0087251398 DASH) listed first,
   consistent with "System auto-selects the highest-balance Platform address" (the simple,
   non-Advanced Send form — not used for this test, since it doesn't support shielded
   destinations, see below — auto-populates exactly this same address as source when
   Platform Addresses is selected there, confirming the auto-selection behavior directly).
2. Selected that address as the sole input, amount 0.003 DASH.
3. Output: pasted the wallet's own shielded address
   (`tdash1zpzmpc25xp0x3gjh650nqhunsmezkqqujawl2g2p6k04uax7nj53fdlpcp77udv8vpp4cvs6cca9x`,
   copied from the Shielded tab per WAL-029). The field correctly tagged it green
   **"(Shielded)"** and accepted an amount, 0.003 DASH.
4. Clicked "Send".

Observed: **fails every time** with the banner **"Invalid output address"** — confirmed in
`det.log`:
```
2026-07-15T07:58:45.559932Z ERROR dash_evo_tool::ui::components::message_banner: Banner displayed banner="Invalid output address"
```
Screenshot: `screenshots/SND-009-1-invalid-output-address-platform-to-shielded.png`. No
funds moved — the Platform source address balance was unaffected by this failed attempt.

This exactly reproduces **SND-007**'s finding (Core → Shielded also rejected with the same
message), now confirmed for the Platform → Shielded direction too: the shielded-destination
rejection is not specific to a Core-Wallet source, it applies uniformly regardless of which
`SourceType` the Send screen uses. Per SND-007's diagnosis, the Shielded tab's own
"Shielded sending is not available on this network yet" notice is the accurate underlying
explanation, but — as before — that explanation is never surfaced at the point of failure in
the Send screen itself.

**Verdict: FAIL.** Bullet 1 ("Select Platform Addresses as source and enter a shielded
address as destination") is reachable and the address is correctly recognized/tagged, but
the transaction cannot actually be submitted — same root cause and same verdict class as
SND-007. Bullet 2 ("auto-selects the highest-balance Platform address") is independently
confirmed working via the simple Send form. Bullet 3 (Developer mode required) confirmed —
this flow was exercised in Developer view as required. The WAL-017 root cause this story was
previously blocked on (no Platform balance) is fully resolved; the story is now blocked by
the same shielded-destination-rejection defect as SND-007, not a funding gap.
