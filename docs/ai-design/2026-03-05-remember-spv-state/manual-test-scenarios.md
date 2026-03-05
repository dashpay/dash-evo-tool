# Manual Test Scenarios: Remember SPV State (#626)

## Prerequisites
- Developer mode enabled
- Backend mode set to SPV

## Scenario 1: SPV reconnects after restart

1. Open Dash Evo Tool
2. Go to Network Chooser, click **Connect** (SPV)
3. Wait for SPV to reach Syncing or Running state
4. Close the app
5. Reopen the app

**Expected:** SPV automatically starts syncing/connecting on launch.

## Scenario 2: SPV stays idle after manual disconnect + restart

1. Open Dash Evo Tool
2. Connect SPV (wait for Syncing/Running)
3. Click **Disconnect**
4. Close the app
5. Reopen the app

**Expected:** SPV is idle (no auto-start).

## Scenario 3: Checkbox reflects connect/disconnect state

1. Open Dash Evo Tool
2. Go to Network Chooser settings, verify "Auto-start SPV on startup" is unchecked
3. Click **Connect** (SPV)
4. Scroll to settings, verify checkbox is now **checked**
5. Click **Disconnect**
6. Verify checkbox is now **unchecked**

## Scenario 4: Backend mode switch clears auto-start

1. Connect SPV (verify checkbox is checked)
2. Switch backend mode from SPV to Dash Core RPC
3. Verify checkbox is unchecked
4. Close and reopen app
5. Switch back to SPV mode

**Expected:** SPV does not auto-start (was cleared when switching to RPC).

## Scenario 5: Developer mode disable clears auto-start

1. Connect SPV
2. Disable developer mode in settings
3. Verify SPV has stopped and auto-start checkbox is no longer visible
4. Close and reopen app
5. Re-enable developer mode
6. Switch to SPV mode

**Expected:** SPV does not auto-start (was cleared when dev mode was disabled).

## Scenario 6: Manual checkbox override still works

1. Without connecting SPV, manually check "Auto-start SPV on startup"
2. Close and reopen app

**Expected:** SPV auto-starts on launch (manual checkbox override respected).
