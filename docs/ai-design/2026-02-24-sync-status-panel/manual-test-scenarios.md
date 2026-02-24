# Manual Test Scenarios: Sync Status Panel (PR #642)

**Feature:** Compact sync status panel on the Wallets screen showing Core and Platform sync status.
**Branch:** `zk-extract/sync-status-panel`
**Date:** 2026-02-24

---

## TS-01: Panel visibility -- no wallet selected

### Preconditions
- Application launched and on the Wallets screen.
- No HD wallet is selected (either no wallets exist or the user has not yet clicked one).

### Steps
1. Navigate to the Wallets screen.
2. Observe the area between the wallet selector and the wallet detail panel.

### Expected Results
- The sync status panel is **not visible**.
- No "Core:" or "Platform:" labels appear.

---

## TS-02: Panel visibility -- HD wallet selected

### Preconditions
- At least one HD wallet is loaded in the application.

### Steps
1. Navigate to the Wallets screen.
2. Select an HD wallet from the left panel.

### Expected Results
- A compact panel appears between the wallet selector and the wallet detail panel.
- The panel contains two lines:
  - **Line 1:** Starts with bold "Core:" label.
  - **Line 2:** Starts with bold "Platform:" label.

---

## TS-03: Core status -- RPC mode, connected

### Preconditions
- Application configured in RPC mode (Dash Core wallet running and reachable).
- RPC connection is healthy.
- An HD wallet is selected.

### Steps
1. Observe the Core line in the sync status panel.

### Expected Results
- Core line displays: **Core: Connected**
- "Connected" text is in dark green.
- No spinner is shown.

---

## TS-04: Core status -- RPC mode, disconnected

### Preconditions
- Application configured in RPC mode.
- Dash Core wallet is stopped or unreachable.
- An HD wallet is selected.

### Steps
1. Observe the Core line in the sync status panel.

### Expected Results
- Core line displays: **Core: Disconnected**
- "Disconnected" text is in the error/red color.
- No spinner is shown.

---

## TS-05: Core status -- SPV mode, idle/stopped

### Preconditions
- Application configured in SPV mode.
- SPV service has not started yet or has been stopped.
- An HD wallet is selected.

### Steps
1. Observe the Core line in the sync status panel.

### Expected Results
- Core line displays: **Core: Disconnected**
- Text uses the secondary/muted color.

---

## TS-06: Core status -- SPV mode, starting

### Preconditions
- Application configured in SPV mode.
- SPV service is in the process of connecting (Starting state).
- An HD wallet is selected.

### Steps
1. Watch the Core line as the SPV service initializes.

### Expected Results
- A blue spinner appears next to the "Core:" label.
- Text reads: **Core: Connecting...**
- Text is in Dash blue color.

---

## TS-07: Core status -- SPV mode, syncing (Headers phase)

### Preconditions
- Application configured in SPV mode.
- SPV sync is actively downloading block headers (earliest sync phase).
- An HD wallet is selected.

### Steps
1. Observe the Core line during initial header sync.

### Expected Results
- A blue spinner is visible.
- Text reads: **Core: Syncing -- Headers NN%** where NN is a whole number 0-100.
- The percentage increases over time as headers are downloaded.

---

## TS-08: Core status -- SPV mode, syncing (Filter Headers phase)

### Preconditions
- Application configured in SPV mode.
- SPV sync has completed headers and is downloading filter headers.
- An HD wallet is selected.

### Steps
1. Observe the Core line during filter header sync.

### Expected Results
- A blue spinner is visible.
- Text reads: **Core: Syncing -- Filter Headers NN%**
- Percentage reflects progress of filter header download.

---

## TS-09: Core status -- SPV mode, syncing (Filters phase)

### Preconditions
- Application configured in SPV mode.
- SPV sync is downloading compact block filters.
- An HD wallet is selected.

### Steps
1. Observe the Core line during filter sync.

### Expected Results
- A blue spinner is visible.
- Text reads: **Core: Syncing -- Filters NN%**

---

## TS-10: Core status -- SPV mode, syncing (Blocks phase)

### Preconditions
- Application configured in SPV mode.
- SPV sync is downloading relevant blocks.
- An HD wallet is selected.

### Steps
1. Observe the Core line during block sync.

### Expected Results
- A blue spinner is visible.
- Text reads: **Core: Syncing -- Blocks NN%**

---

## TS-11: Core status -- SPV mode, fully synced (Running)

### Preconditions
- Application configured in SPV mode.
- SPV sync has completed and the node is running normally.
- An HD wallet is selected.

### Steps
1. Observe the Core line after sync completes.

### Expected Results
- No spinner is shown.
- Text reads: **Core: Synced -- N peers** where N is the number of connected peers.
- Text is in dark green.

---

## TS-12: Core status -- SPV mode, stopping

### Preconditions
- Application configured in SPV mode.
- SPV service is shutting down.
- An HD wallet is selected.

### Steps
1. Trigger an action that stops SPV (e.g., switching networks).
2. Observe the Core line during shutdown.

### Expected Results
- A blue spinner is visible.
- Text reads: **Core: Disconnecting...**
- Text is in Dash blue color.

---

## TS-13: Core status -- SPV mode, error

### Preconditions
- Application configured in SPV mode.
- SPV service has encountered an error.
- An HD wallet is selected.

### Steps
1. Observe the Core line when SPV is in error state.

### Expected Results
- Text reads: **Core: Error**
- Text is in the error/red color.
- No spinner is shown.

---

## TS-14: Platform status -- wallet never synced

### Preconditions
- An HD wallet is selected.
- The wallet has never performed a platform sync (no sync record in database, or timestamp is 0).

### Steps
1. Observe the Platform line in the sync status panel.

### Expected Results
- Platform line displays: **Platform: Addresses: never synced**
- Text uses the secondary/muted color.

---

## TS-15: Platform status -- wallet previously synced

### Preconditions
- An HD wallet is selected.
- The wallet has been synced with the platform at least once (database contains a non-zero sync timestamp and block height).

### Steps
1. Observe the Platform line in the sync status panel.

### Expected Results
- Platform line displays: **Platform: Addresses: N synced (blk H, T ago)**
  - N = number of platform addresses in the wallet.
  - H = the block height at which the last sync occurred.
  - T = relative time since last sync (e.g., "30s ago", "5m ago", "2h ago", "1d ago").
- Text uses the secondary/muted color.

---

## TS-16: Platform status -- active refresh in progress

### Preconditions
- An HD wallet is selected.
- A platform address balance refresh is currently in progress (the `refreshing` flag is true).

### Steps
1. Trigger a platform refresh (e.g., click the refresh button).
2. Observe the Platform line while the refresh is running.

### Expected Results
- A blue spinner appears on the Platform line.
- The address text (count, block height, time ago) is displayed in Dash blue instead of the secondary color.
- Once the refresh completes, the spinner disappears and text returns to secondary color.

---

## TS-17: Time-ago formatting -- seconds

### Preconditions
- A wallet has been synced very recently (less than 60 seconds ago).

### Steps
1. Trigger a platform sync.
2. Immediately observe the Platform line after sync completes.

### Expected Results
- The time-ago portion reads something like "5s ago", "12s ago", etc.
- The number is between 0 and 59 (inclusive).

---

## TS-18: Time-ago formatting -- minutes

### Preconditions
- A wallet was last synced between 1 and 59 minutes ago.

### Steps
1. Observe the Platform line.

### Expected Results
- The time-ago portion reads something like "3m ago", "45m ago".
- Uses integer division (e.g., 90 seconds shows "1m ago", not "1.5m ago").

---

## TS-19: Time-ago formatting -- hours

### Preconditions
- A wallet was last synced between 1 and 23 hours ago.

### Steps
1. Observe the Platform line.

### Expected Results
- The time-ago portion reads something like "2h ago", "18h ago".

---

## TS-20: Time-ago formatting -- days

### Preconditions
- A wallet was last synced 24 hours or more ago.

### Steps
1. Observe the Platform line.

### Expected Results
- The time-ago portion reads something like "1d ago", "7d ago".

---

## TS-21: Wallet switching updates the panel

### Preconditions
- Two or more HD wallets are loaded.
- Wallet A has been synced recently (has platform sync info).
- Wallet B has never been synced (no platform sync info).

### Steps
1. Select Wallet A from the left panel.
2. Observe the sync status panel -- note the platform sync info (address count, block height, time ago).
3. Switch to Wallet B by clicking it in the left panel.
4. Observe the sync status panel.

### Expected Results
- After step 2: Platform line shows address count, block height, and time-ago for Wallet A.
- After step 4: Platform line updates to show "Addresses: never synced" for Wallet B.
- The Core line remains unchanged (it reflects the global core connection, not per-wallet state).

---

## TS-22: Panel respects light and dark mode

### Preconditions
- An HD wallet is selected.
- The application supports theme switching.

### Steps
1. Switch the application to dark mode.
2. Observe the sync status panel colors and contrast.
3. Switch the application to light mode.
4. Observe the sync status panel colors and contrast.

### Expected Results
- In dark mode: panel background uses the dark surface color; "Core:" and "Platform:" labels use the dark-mode primary text color; secondary text is readable against the dark background.
- In light mode: panel background uses the light surface color; labels and secondary text are readable against the light background.
- Status colors (dark green for connected/synced, red for error/disconnected, blue for syncing) remain visually distinct in both modes.

---

## TS-23: SPV sync phase progression

### Preconditions
- Application configured in SPV mode.
- Starting from a fresh state (no previously synced data) so that all four sync phases are traversed.
- An HD wallet is selected.

### Steps
1. Start the application and let SPV sync begin.
2. Continuously monitor the Core line throughout the entire sync process.

### Expected Results
- The Core line progresses through these states in order:
  1. "Connecting..."
  2. "Syncing -- Headers NN%" (percentage climbs from low to 100)
  3. "Syncing -- Filter Headers NN%"
  4. "Syncing -- Filters NN%"
  5. "Syncing -- Blocks NN%"
  6. "Synced -- N peers"
- Each phase percentage starts low and progresses upward.
- A spinner is visible during all syncing phases and disappears when fully synced.

---

## TS-24: SPV sync progress -- target height zero

### Preconditions
- Application configured in SPV mode.
- SPV sync is in early startup where the target height may not yet be known (target = 0).

### Steps
1. Observe the Core line during the very first moments of sync.

### Expected Results
- If a phase has target_height = 0, the progress displays "0%" (not a division-by-zero crash or NaN).
- The panel remains stable and does not flicker or show garbled text.

---

## TS-25: Database refactor -- shared_connection removal

### Preconditions
- This is a code-level verification, not a UI test.

### Steps
1. Verify the application compiles without errors (`cargo build`).
2. Run the full test suite (`cargo test --all-features --workspace`).
3. Search the codebase for any remaining calls to `Database::shared_connection()`.

### Expected Results
- The application compiles successfully.
- All tests pass.
- No remaining references to `shared_connection()` exist in the codebase (the method was removed and `Database.conn` changed from `Arc<Mutex<Connection>>` to `Mutex<Connection>`).

---

## Edge Cases

| # | Scenario | Expected Behavior |
|---|----------|-------------------|
| E1 | System clock is set to the past (before the last sync timestamp) | `format_unix_time_ago` uses `saturating_sub`, so it shows "0s ago" rather than panicking or showing negative time. |
| E2 | SPV sync progress is `None` while status is `Syncing` | Phase text falls back to "starting..." rather than crashing. |
| E3 | Wallet read lock fails (poisoned RwLock) | Address count defaults to 0; platform sync info shows as "never synced". The panel does not crash. |
| E4 | Very large block heights or peer counts | Numbers render as plain integers without overflow; the panel layout adjusts to accommodate wider text. |
| E5 | Rapidly switching between wallets | The `platform_sync_info` cache is refreshed on each selection; no stale data from the previous wallet is shown. |
| E6 | Single-key wallet selected (not HD) | The sync status panel is hidden (it only renders when `selected_wallet` is `Some`, which is for HD wallets only). |
