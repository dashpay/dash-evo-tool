# Manual Test Scenarios: SPV Peer Status Rework

**Feature:** Replace SPV auto-stop with Connecting state and degraded-state warning
**Branch:** `fix/spv-peer-timeout`
**Date:** 2025-02-25

## Overview

These scenarios verify the reworked SPV peer connection lifecycle:

- SPV **no longer auto-stops** on peer timeout -- it keeps running and retrying peer discovery.
- New **four-state** connection indicator: Red (Disconnected), Orange/fast-pulse (Connecting), Orange/slow-pulse (Syncing), Green (Synced).
- After ~30 seconds with zero peers, a **degraded warning** appears in the tooltip ("Having trouble finding peers...") but SPV stays running.

---

## Global Preconditions

1. Dash Evo Tool is installed and launches successfully.
2. The application is configured for **SPV mode** unless stated otherwise.
3. At least one network (Testnet or Mainnet) is configured with valid DAPI endpoints.
4. Tester has access to network controls (firewall rules, VPN toggle) to simulate peer availability.

---

## Scenario 1: Fresh SPV Start on Good Network

**Goal:** Verify the Connecting -> Syncing -> Synced progression.

### Steps

1. Launch the application with normal network connectivity.
2. Observe the connection indicator immediately after SPV starts.
3. Hover over the indicator to read the tooltip.
4. Wait for peers to connect (usually within a few seconds).
5. Observe the indicator transition during sync phases.
6. Wait for sync to complete.

### Expected Results

- **Step 2:** Orange circle with **fast pulse** (`time * 2.5`). Tooltip header: "Connecting...".
- **Step 4:** Once peers connect, indicator remains orange but pulse slows (`time * 1.2`). Tooltip header changes to "Syncing". SPV label shows phase progress (e.g., "SPV: Headers: 12345 / 27000 (45%)").
- **Step 6:** Indicator turns **green** with normal pulse. Tooltip: "Ready\nSPV: Synced\nDAPI: Available (N unbanned / M total endpoints)".

---

## Scenario 2: Fresh SPV Start on Bad Network (No Peers)

**Goal:** Verify SPV stays running and shows degraded warning instead of stopping.

### Steps

1. Block outbound P2P peer connections (firewall on port 9999/19999).
2. Launch the application. Ensure DAPI endpoints are reachable.
3. Observe the indicator -- should show orange (Connecting).
4. Wait approximately 30 seconds.
5. Hover over the indicator to read the tooltip.
6. Continue waiting 1-2 minutes.

### Expected Results

- **Step 3:** Orange indicator with fast pulse. Tooltip: "Connecting...\nSPV: Starting\nDAPI: Available (...)".
- **Step 5:** After ~30s, tooltip adds: "\nHaving trouble finding peers. Check your connection." A **warning banner** also appears with the same text.
- **Step 6:** SPV **remains running** (does NOT stop). Indicator stays orange. SPV continues trying DNS lookups and peer discovery in the background.
- **Critical:** Verify NO "SPV disconnected" error banner. Verify `stop_spv()` is NOT called (check logs -- no "stopping SPV" message).
- **Recovery:** If you restore connectivity and peers connect, the warning banner **automatically clears** and the indicator transitions to Syncing/Synced.

---

## Scenario 3: Peers Disconnect Mid-Sync

**Goal:** Verify state transitions back to Connecting (not Disconnected) when peers drop.

### Steps

1. Start SPV and wait for it to begin syncing with peers (orange, slow pulse).
2. Block all peer connections via firewall.
3. Observe the indicator over the next 5-10 seconds.

### Expected Results

- **Step 2:** Peer count drops to 0. The `spv_no_peers_since` timer starts.
- **Step 3:** Indicator changes from slow-pulse orange (Syncing) to **fast-pulse orange (Connecting)**. SPV status remains `Syncing` but with 0 peers, `refresh_state()` maps this to `Connecting`.
- SPV does NOT stop. No error banner.
- After 30s of no peers, tooltip adds "Having trouble finding peers...".

---

## Scenario 4: Peers Reconnect After Disconnect

**Goal:** Verify seamless recovery when peers become available again.

### Steps

1. Complete Scenario 3 (SPV is in Connecting state, no peers).
2. Restore network connectivity (remove firewall rule).
3. Observe the indicator.

### Expected Results

- **Step 2-3:** Peers reconnect via SPV's internal discovery. `spv_no_peers_since` is cleared (`peers > 0` resets to `None`).
- Indicator transitions from fast-pulse orange (Connecting) to slow-pulse orange (Syncing) as peers connect and sync resumes.
- Eventually reaches green (Synced) once sync completes.
- No manual restart needed -- SPV recovered on its own.

---

## Scenario 5: Network Switch

**Goal:** Verify connection state resets cleanly on network switch.

### Steps

1. Confirm SPV is synced on current network (green indicator).
2. Switch to a different network via the network chooser.
3. Observe the indicator immediately after switch.
4. Wait for SPV to start on the new network.

### Expected Results

- **Step 2:** `ConnectionStatus::reset()` clears all state: `spv_status` -> Idle, `spv_connected_peers` -> 0, `spv_no_peers_since` -> None, `overall_state` -> Disconnected.
- **Step 3:** Indicator turns **red** momentarily.
- **Step 4:** Transitions to orange/Connecting, then Syncing, then green/Synced.

---

## Scenario 6: All DAPI Endpoints Banned

**Goal:** Verify that losing DAPI forces Disconnected even with SPV peers.

### Steps

1. Confirm SPV is synced (green indicator).
2. Cause all DAPI endpoints to become banned.
3. Observe the indicator after the next refresh cycle (within 1-4 seconds).

### Expected Results

- `refresh_state()` returns `Disconnected` because `dapi_available()` is false.
- Indicator turns **red**, regardless of SPV peer status.
- Tooltip: "Disconnected\nSPV: Synced\nDAPI: All M endpoints banned".

---

## Scenario 7: Connection Indicator Visual States

**Goal:** Verify all four visual states render correctly.

### Expected Results

| State | Color | Pulse | Background glow |
|---|---|---|---|
| Disconnected | Red (`error_color`) | None (`scale = 1.0`) | Same radius as main circle |
| Connecting | Orange (`warning_color`) | Fast pulse (`1.0 + 0.2 * sin(t*2.5)`) | Pulsating with 0.3 opacity |
| Syncing | Orange (`warning_color`) | Slow pulse (`1.0 + 0.15 * sin(t*1.2)`) | Pulsating with 0.3 opacity |
| Synced | Green (`success_color`) | Normal pulse (`1.0 + 0.2 * sin(t*2.0)`) | Pulsating with 0.3 opacity |

- Connecting and Syncing use the same orange color but differ in pulse rate.
- Only Disconnected does NOT call `repaint_animation`.

---

## Scenario 8: Tooltip Text for Each State

**Goal:** Verify tooltip accuracy across all SPV states.

| SPV Status | Peers | Overall State | Tooltip line 1 | Tooltip line 2 | Extra |
|---|---|---|---|---|---|
| Idle | 0 | Disconnected | "Disconnected" | "SPV: Idle" | -- |
| Starting | 0 | Connecting | "Connecting..." | "SPV: Starting" | After 30s: "Having trouble finding peers..." |
| Syncing | >0 | Syncing | "Syncing" | "SPV: Headers: X / Y (Z%)" | Phase progress shown |
| Syncing | 0 | Connecting | "Connecting..." | "SPV: <phase>" | After 30s: degraded warning |
| Running | >0 | Synced | "Ready" | "SPV: Synced" | -- |
| Running | 0 | Connecting | "Connecting..." | "SPV: Synced" | After 30s: degraded warning |
| Stopping | 0 | Connecting | "Connecting..." | "SPV: Stopping" | -- |
| Stopped | 0 | Disconnected | "Disconnected" | "SPV: Stopped" | -- |

---

## Scenario 9: Running State with Peers Dropping to Zero

**Goal:** Verify Running (Synced) transitions correctly when peers vanish.

### Steps

1. Confirm green indicator (SPV Running, peers connected).
2. Block peer connections.
3. Observe the indicator over 30+ seconds.

### Expected Results

- Once peer count drops to 0, `refresh_state()` maps active SPV with zero peers to `Connecting` (fast orange pulse).
- **Wait** -- `Running` status means sync finished. The SPV library may transition to a different status internally if peers drop. Observe actual SpvStatus transitions.
- If SPV stays `Running` with 0 peers: indicator should remain **orange** (`Connecting`), and the `spv_no_peers_since` timer continues without calling `stop_spv()`.
- After 30s with 0 peers, degraded warning banner and tooltip appear.

---

## Scenario 10: Long-Running Stability

**Goal:** Verify no resource leaks from peer tracking.

### Steps

1. Launch and sync SPV fully.
2. Note memory usage (RSS).
3. Run for 1+ hour with occasional peer churn (toggle connectivity 2-3 times).
4. Check memory every 15 minutes.

### Expected Results

- Memory stable (no unbounded growth).
- `spv_no_peers_since` is `Option<Instant>` (fixed size), `spv_connected_peers` is `AtomicU16`.
- After peer churn, returns to Synced without stale state.
- No Mutex poisoning or deadlock warnings in logs.

---

## Scenario 11: RPC Mode Unaffected

**Goal:** Verify SPV peer logic doesn't interfere with RPC mode.

### Steps

1. Launch in RPC mode (Dash Core wallet connected).
2. Verify indicator follows RPC/ZMQ status.
3. Stop Dash Core.
4. Observe indicator.

### Expected Results

- No SPV timeout logic is involved.
- Green when RPC online + ZMQ connected + DAPI available; red otherwise.
- Tooltip shows RPC-specific content.
- No "Having trouble finding peers..." or SPV-related messages.

---

## Scenario 12: Connecting State vs Syncing Pulse Differentiation

**Goal:** Verify the visual difference between Connecting and Syncing is noticeable.

### Steps

1. Start SPV with peers blocked -- observe Connecting state pulse.
2. Unblock peers -- observe transition to Syncing state pulse.
3. Compare the two pulse rates side by side (or record video).

### Expected Results

- **Connecting** pulse is noticeably faster (2.5 Hz base) with slightly larger amplitude.
- **Syncing** pulse is calmer (1.2 Hz base) with smaller amplitude.
- Both are orange -- the pulse rate is the primary visual differentiator.
- Transition between states should be smooth (no flicker or jump).
