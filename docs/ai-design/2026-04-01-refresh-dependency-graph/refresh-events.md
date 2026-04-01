# Refresh & Event Dependency Graph

**Date:** 2026-04-01
**Audience:** Developers reviewing the refresh/event architecture

This document maps every event source to its downstream refresh targets, showing the routing path through tasks, result channels, and screen handlers. Use it as a reference when debugging stale UI data, duplicate refreshes, or mode-specific gaps.

---

## 1. Overview

The app has two distinct connection modes with different event models:

- **RPC mode** — periodic polling via `GetBestChainLocks`, wallet data fetched on demand
- **SPV mode** — push-based events from the SPV client; no RPC polling; wallet UTXOs reconciled from SPV storage

Both modes share the same `AppAction` → `BackendTask` → `TaskResult` channel pipeline.

```mermaid
flowchart TD
    subgraph Sources["Event Sources"]
        POLL["Periodic poll\n(trigger_refresh)\nconnection_status.rs:464"]
        SPV_EH["SPV EventHandler\nspv/manager.rs:138"]
        ZMQ["ZMQ listener\ncore_zmq_listener.rs"]
        USER["User action\n(button / navigation)"]
        BOOT["Startup bootstrap\nwallet_lifecycle.rs:309"]
    end

    subgraph Tasks["BackendTask dispatch"]
        T_CL["GetBestChainLocks"]
        T_RW["RefreshWalletInfo"]
        T_PB["FetchPlatformAddressBalances"]
        T_SN["ShieldedTask::SyncNotes"]
        T_LC["LoadContacts"]
        T_FI["FetchIdentity"]
        T_PI["PlatformInfo"]
        T_RECON["reconcile_spv_wallets()\n(internal, no task)"]
    end

    subgraph Results["BackendTaskSuccessResult"]
        R_CL["CoreItem::ChainLocks"]
        R_RW["RefreshedWallet"]
        R_PB["PlatformAddressBalances"]
        R_SN["ShieldedNotesSynced"]
        R_LC["DashPayContactsWithInfo"]
        R_FI["RefreshedIdentity"]
        R_PI["PlatformInfoTaskResult"]
    end

    subgraph UI["UI / State Updates"]
        UI_CS["ConnectionStatus\n(chain height, lock status)"]
        UI_WB["Wallet balance display"]
        UI_PB["Platform balance display"]
        UI_SB["Shielded balance display"]
        UI_DC["DashPay contacts list"]
        UI_ID["Identity details"]
        UI_EP["Epoch / protocol info"]
        UI_SPV["SPV sync progress\n(push, no task)"]
    end

    POLL -->|"RPC mode only"| T_CL
    POLL -->|"always"| T_RECON
    SPV_EH -->|"on_progress / on_sync_event"| UI_SPV
    SPV_EH -->|"reconcile_tx channel\n(debounced 300ms)"| T_RECON
    ZMQ --> T_RW
    USER --> T_RW
    USER --> T_PB
    USER --> T_SN
    USER --> T_LC
    USER --> T_FI
    USER --> T_PI
    BOOT -->|"RPC mode only"| T_RW

    T_CL --> R_CL --> UI_CS
    T_RW --> R_RW --> UI_WB
    T_PB --> R_PB --> UI_PB
    T_SN --> R_SN --> UI_SB
    T_LC --> R_LC --> UI_DC
    T_FI --> R_FI --> UI_ID
    T_PI --> R_PI --> UI_EP
    T_RECON --> UI_WB
```

---

## 2. Event Sources

### 2.1 Periodic Polling — `ConnectionStatus::trigger_refresh()`

**Location:** `src/context/connection_status.rs:464–491`

Called on every UI frame; throttled by elapsed time:

| Condition | Interval |
|---|---|
| Connected | 4 s (`REFRESH_CONNECTED`) |
| Disconnected | 1 s (`REFRESH_DISCONNECTED`) |
| SPV stopping | 200 ms |

On each tick:
- **RPC mode:** dispatches `GetBestChainLocks` → updates `ConnectionStatus` with chain height and lock data
- **SPV mode:** no RPC dispatch; all chain data arrives via push (see §2.2)
- **Always:** calls `refresh_zmq_and_spv()` to recompute `overall_state` from ZMQ and SPV atomics

### 2.2 SPV EventHandler — `spv/manager.rs`

**Location:** `src/spv/manager.rs:138–308`

Three callback types, each writing directly to `ConnectionStatus` atomics:

| Callback | Trigger | Effect |
|---|---|---|
| `on_progress()` | Block sync progress | Updates `sync_progress_state`, sets status `Syncing` / `Running` / `Error` |
| `on_sync_event()` | `SyncComplete`, `BlockProcessed`, `ManagerError`, `InstantLock`, `ChainLock` | Forwards finality events; sends on `reconcile_tx` channel (→ wallet reconciliation) |
| `on_network_event()` | `PeersUpdated` | Writes `connected_peers` count to `ConnectionStatus` |

`SyncComplete` sets status to `Running`; `ManagerError` sets it to `Error`.

### 2.3 SPV Reconciliation — `wallet_lifecycle.rs`

**Location:** `src/context/wallet_lifecycle.rs:679–717`

Triggered by the `reconcile_tx` channel whenever `on_sync_event` fires for:
`BlockProcessed` | `ChainLockReceived` | `InstantLockReceived` | `SyncComplete`

- Debounced 300 ms to coalesce rapid block events
- Calls `reconcile_spv_wallets()` — reads UTXOs/balances directly from SPV storage and updates wallet state in memory (no `BackendTask` roundtrip)

### 2.4 ZMQ Events — `core_zmq_listener.rs`

| Event | Effect |
|---|---|
| `ISLockedTransaction` | Direct wallet transaction update |
| `ChainLockedBlock` | Direct wallet update for chain-locked transactions |

### 2.5 Manual User Actions

| Trigger | Task dispatched |
|---|---|
| Refresh button (wallet screen) | `RefreshWalletInfo` |
| Refresh button (platform screen) | `FetchPlatformAddressBalances` |
| Network switch | `change_context()` on all screens → `refresh_on_arrival()` |
| Screen navigation (arrival) | `refresh_on_arrival()` (screen-dependent) |
| Post-operation (transfer, register, etc.) | Screen sets `pending_platform_balance_refresh = true` → dispatches on next frame |

### 2.6 Startup Bootstrap — `wallet_lifecycle.rs`

**Location:** `src/context/wallet_lifecycle.rs:309–355`

- **RPC mode:** auto-dispatches `RefreshWalletInfo` for each loaded wallet
- **SPV mode:** skipped; wallets rely on reconciliation from the first sync events

---

## 3. Mode-Specific Event Flows

### 3.1 RPC Mode

```mermaid
sequenceDiagram
    participant Frame as UI Frame
    participant CS as ConnectionStatus
    participant BT as BackendTask channel
    participant Core as Dash Core (RPC)
    participant Screen as Active Screen

    Frame->>CS: trigger_refresh() [every 4s]
    CS->>BT: dispatch GetBestChainLocks
    BT->>Core: RPC call
    Core-->>BT: ChainLock data
    BT-->>CS: handle_task_result() → update height/status
    BT-->>Screen: display_task_result(CoreItem::ChainLocks)

    Frame->>CS: trigger_refresh() [startup]
    CS->>BT: dispatch RefreshWalletInfo (each wallet)
    BT->>Core: getbalance / listunspent
    Core-->>BT: UTXO + balance data
    BT-->>Screen: display_task_result(RefreshedWallet)
```

### 3.2 SPV Mode

```mermaid
sequenceDiagram
    participant SPV as SPV Client
    participant EH as EventHandler
    participant CS as ConnectionStatus
    participant Recon as reconcile_tx channel
    participant WL as wallet_lifecycle
    participant Screen as Active Screen

    SPV->>EH: on_progress(height, total)
    EH->>CS: write sync_progress atomics (push)
    Note right of CS: No task dispatched

    SPV->>EH: on_sync_event(BlockProcessed)
    EH->>Recon: send reconcile signal
    Note right of Recon: debounced 300ms
    Recon->>WL: reconcile_spv_wallets()
    WL->>Screen: wallet state updated in Arc<AppContext>
    Note right of Screen: Screen reads on next frame

    SPV->>EH: on_network_event(PeersUpdated(n))
    EH->>CS: connected_peers.store(n)
```

---

## 4. Task Result Routing — `app.rs`

**Location:** `src/app.rs:1254–1378`

Every frame, `AppState::update()` drains `task_result_receiver`:

```mermaid
flowchart TD
    RECV["task_result_receiver.try_recv()"]
    RECV --> CHK{TaskResult variant}

    CHK -->|"Success(result)"| CS_HANDLE["ConnectionStatus::handle_task_result()\n(intercepts ChainLocks)"]
    CS_HANDLE --> SCREEN_SUC["visible_screen.display_task_result(result)"]

    CHK -->|"Error(err)"| SCREEN_ERR["visible_screen.display_task_error(err)"]
    SCREEN_ERR -->|"not handled"| BANNER["MessageBanner::set_global() fallback"]

    CHK -->|"Refresh"| SCREEN_REF["visible_screen.refresh()"]
```

`ConnectionStatus::handle_task_result()` intercepts `CoreItem::ChainLocks` results to update chain height and lock status before the screen sees them. All other result variants pass through unchanged.

---

## 5. Per-Screen Refresh Dependencies

| Screen | `refresh_on_arrival()` triggers | Manual refresh | Post-operation refresh |
|---|---|---|---|
| Wallet (overview) | `RefreshWalletInfo` (RPC) / none (SPV) | `RefreshWalletInfo` | — |
| Wallet (platform tab) | `FetchPlatformAddressBalances` | `FetchPlatformAddressBalances` | `pending_platform_balance_refresh` flag |
| Wallet (shielded tab) | `ShieldedTask::SyncNotes` | `ShieldedTask::SyncNotes` | — |
| Network / status | `GetBestChainLocks`, `PlatformInfo` | — | — |
| Identity detail | `FetchIdentity` | `FetchIdentity` | After register/update |
| DashPay contacts | `LoadContacts` | — | — |
| Contests / DPNS | Usernames fetch | — | After vote / register |

---

## 6. Timing Constants

| Constant | Value | Location |
|---|---|---|
| `REFRESH_CONNECTED` | 4 s | `connection_status.rs` |
| `REFRESH_DISCONNECTED` | 1 s | `connection_status.rs` |
| SPV stopping interval | 200 ms | `connection_status.rs` |
| `SPV_PEER_DEGRADED_TIMEOUT` | 30 s | `connection_status.rs` |
| SPV reconcile debounce | 300 ms | `wallet_lifecycle.rs` |

---

## 7. Known Issues and Gaps

These are observed inconsistencies between SPV and RPC behavior, or missing refresh paths. They are not bugs that block functionality today, but they will cause stale data in specific scenarios.

### GAP-1: SPV mode still calls `ListCoreWallets` on wallet screens

Some wallet screen paths call `refresh_wallet_info()` without checking the current connection mode. In SPV mode this RPC call either fails silently or returns stale/empty data because Core is not managing the wallet. Reconciliation via `reconcile_spv_wallets()` is the correct path in SPV mode.

**Files to check:** `src/backend_task/wallet/`, `src/context/wallet_lifecycle.rs:309`

### GAP-2: No automatic platform balance refresh after SPV sync completes

When `SyncComplete` fires (SPV fully caught up), the wallet UTXO state is reconciled but the platform balance (`FetchPlatformAddressBalances`) is not triggered. A user who opens the platform tab immediately after sync may see stale balances until they manually refresh.

**Workaround:** The `pending_platform_balance_refresh` flag handles the post-transfer case, but not the post-sync case.

### GAP-3: DashPay contacts do not auto-refresh on new incoming contact request

`LoadContacts` is triggered on `refresh_on_arrival()` (when the screen is opened) but there is no push path: if a contact request arrives while the DashPay screen is open, the list does not update. A platform subscription or periodic re-fetch would be needed.

**Location:** `src/ui/screens/` (DashPay contacts screen), `src/backend_task/identity/`

### GAP-4: `refresh_wallet_info()` called from some paths without SPV guard

Related to GAP-1. A defensive check at the dispatch site (or a mode-aware wrapper) would prevent unnecessary RPC calls in SPV mode and make the refresh paths symmetric.
