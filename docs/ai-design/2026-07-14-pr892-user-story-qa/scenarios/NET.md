# NET — Network and Settings

Environment: PR892 build, isolated data dir `/data/tmp/det-qa-pr892-data`, display `:99`.

## NET-001: Switch networks — PASS

Steps:
1. Fresh launch defaults to **Mainnet** (SDK init log: `network=Mainnet`), and shows
   "Disconnected — check your internet connection" initially (expected — SPV not yet started).
2. Navigated to Settings (sidebar, requires scrolling down past Wallets/Tools to reveal
   Settings/Expert-toggle/Dash-logo — sidebar overflows the visible area at default window
   height, see UX note below) — this opens the "Networks" screen.
3. "Connection Settings" card at the top has a `Network:` dropdown, disabled while connected.
4. Clicked "Disconnect" (stops SPV) — dropdown became enabled.
5. Opened dropdown: options are Mainnet / Testnet / Devnet / Local.
6. Selected "Testnet" — SPV immediately started syncing against testnet (`Headers: 80000 /
   1514569`, DAPI "Available (29 unbanned / 29 total endpoints)"), sidebar network indicator
   at the bottom updated to "Testnet".

Verdict: **PASS**.

### UX note (not a defect, worth flagging)
The sidebar navigation (Identities / Masternodes / Contracts / Tokens / Wallets / Tools /
Settings / Expert-toggle / Dash logo) does not fit within the default 800×600 window height
in Expert view — "Settings" is pushed below the fold and only reachable by scrolling the
sidebar itself. Not discovered until the window was manually resized larger and then scrolled.
An easy miss for a new user in the default window size.

### UX note: Dash logo at the bottom of the sidebar is an external link
Clicking the Dash logo at the bottom of the sidebar opens the system's default browser to
`dash.org` (a new top-level browser window), rather than doing anything in-app. Confirmed
intentional (branding link) but worth noting since it's easy to click by accident given its
proximity to "Settings" right above it in the same sidebar column.

## Database Maintenance / Advanced Settings — observed (not yet formally tied to a story)

Settings > Networks > Advanced Settings exposes: Theme selector (NET-004), "Auto-start SPV
on startup" toggle, "Clear Mainnet/Testnet/etc. Database" (destructive, per-network — maps to
NET-011/NET-019 family), "Clear SPV Data" (maps to NET-020). Deferred to the destructive-tests
pass at the end of the campaign per plan.

---

*Remaining NET stories to be completed in a follow-up pass — see `progress.md`.*
