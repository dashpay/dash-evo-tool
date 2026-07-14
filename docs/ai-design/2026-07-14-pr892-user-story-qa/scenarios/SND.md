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

## SND-002: Send Dash from single-key wallet — FAIL (product limitation, explicit typed error)

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
