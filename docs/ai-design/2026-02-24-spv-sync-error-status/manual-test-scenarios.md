# Manual Test Scenarios: SPV Sync Error Status

## Context

When SPV sync encounters a fatal error (e.g., masternode sync failure), the app
should transition from "Syncing" to a distinct Error state, with the connectivity
icon turning **magenta** (with "!" glyph and slow pulse) and the tooltip showing
the specific error message.

## Prerequisites

- Dash Evo Tool built with the fix applied
- Access to Testnet (or a network where SPV sync can be triggered)
- SPV backend mode enabled (not RPC mode)

## Scenario 1: Verify error state on sync failure

**Goal:** Confirm the connectivity icon transitions to Error (magenta) when
SPV sync fails, distinct from Disconnected (red).

### Steps

1. Launch Dash Evo Tool and connect to Testnet in SPV mode.
2. Observe the top-left connectivity icon during sync — it should pulse orange
   (Syncing state).
3. If sync completes successfully, the icon should turn green (Running state).
4. If sync fails (e.g., masternode QRInfo failure visible in logs), observe:
   - The connectivity icon turns **magenta** with a slow pulsation and a white
     **"!"** glyph in the center.
   - Hovering over the icon shows tooltip: **"SPV sync error: {detail}"** with
     the specific error message (e.g., "Sync manager Masternode failed: ...").
   - Below that: **"SPV: Error"** detail line.
5. Open the Network Chooser screen and check the SPV status detail — it should
   display the error message.

### Expected Result

- Icon transitions from orange (Syncing) to magenta (Error) on sync failure.
- Error icon is visually distinct from red (Disconnected) — magenta color,
  slow pulse, "!" glyph.
- Tooltip shows "SPV sync error: ..." with the specific error message.
- Error message is visible in the status detail panel.

## Scenario 2: Verify normal sync still works

**Goal:** Confirm the fix doesn't break the happy path.

### Steps

1. Launch Dash Evo Tool and connect to Testnet in SPV mode.
2. Wait for sync to complete (may take several minutes on first sync).
3. Observe the connectivity icon transitions:
   - Orange (Syncing) during sync.
   - Green (Running) after sync completes.
4. Hover over the icon — tooltip should show "SPV synced" with "SPV: Running".

### Expected Result

- Sync completes normally, icon turns green.
- No false error transitions during normal sync.

## Scenario 3: Verify error message content

**Goal:** Confirm the error message stored in `last_error` contains useful
diagnostic information.

### Steps

1. Trigger an SPV sync that fails (e.g., by connecting to a network with
   known chain lock propagation issues).
2. Check application logs for the error:
   - Look for `SPV manager ... reported error: ...` log line.
3. Hover over the connectivity icon and verify the tooltip shows the same
   error message (not a generic "Sync failed" without context).

### Expected Result

- Log contains `SPV manager "Masternode" reported error: Masternode sync failed: ...`.
- Tooltip shows the specific error from the sync manager, including
  the block hash reference.

## Scenario 4: Verify Error state is distinct from Disconnected

**Goal:** Confirm the user can visually distinguish Error from Disconnected.

### Steps

1. With the app in SPV mode, trigger a sync error (Scenario 1).
2. Note the icon appearance: magenta, pulsating, "!" glyph.
3. Switch to a network with no connectivity (e.g., disconnect network).
4. Note the icon appearance: red, static, no glyph.

### Expected Result

- Error state: magenta circle, slow pulse, white "!" glyph.
- Disconnected state: red circle, static (no pulse), no glyph.
- The two states are clearly visually distinguishable.

## Notes

- The actual QRInfo chain lock error is an upstream issue
  (dashpay/rust-dashcore#470). This fix ensures the app **reports** the error
  correctly rather than silently staying stuck in "Syncing".
- A separate upstream issue (dashpay/rust-dashcore#469) tracks the missing
  `try_emit_progress()` call on error paths in dash-spv.
