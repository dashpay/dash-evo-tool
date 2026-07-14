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

*Remaining SND stories (002, 004–013) to be completed in a follow-up pass — see `progress.md`.*
