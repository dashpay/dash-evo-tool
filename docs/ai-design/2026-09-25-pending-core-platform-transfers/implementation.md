# Core → Platform funding history

## Delivered scope

Funding observations enrich rows inside the wallet-level **Dash Core
Transactions** table. Each Core TXID appears once, including a tracked funding
transaction missing from hydrated history. Unconfirmed transactions sort first;
confirmed funding retains its chronological position. **Details** opens the
recorded funding information and any observed competing input spend. The final
Actions column groups Copy, View, Details, and the unavailable cancellation
action using the same compact button style.

Funding records appear only in transaction history, without a separate summary
or navigation button beside the balance. A `RecoveredFromChain` record represents
historical funding with an unknown Platform outcome, not proof of an unfinished
operation. Core confirmation and Platform delivery are distinct.

**Refresh** refreshes wallet balances and then queues a read of tracked locks
and hydrated local history. If a read is already running, one follow-up read is
retained. There is no separate history-refresh button. Reading these records
neither queries Platform consumption nor broadcasts or resumes a transaction.
The details explain that limitation. **Open Core explorer** provides an optional
external check of Core confirmation only; the explorer is not an accounting
or cancellation authority. Async requests remain scoped to wallet/network,
coalesced, and timeout-bounded. Details close when wallet/network changes.

## Cancellation and expiration remain unavailable

The pinned platform revision is `81f1a1c7315e9f0c1a45ffdc206b36c8c48e9a61`.
Its `wallet/asset_lock/sync/recovery.rs` lacks a verified finalized-ancestry
predicate. `abandon_transaction` releases a signed Core payment the caller
chose not to send; it is not an API for cancelling an already-broadcast asset
lock. The backend also lacks a public operation for durable cancellation/rebroadcast
control of a previously broadcast asset lock.

Automatic conflict cleanup does exist for conflicts processed by the live wallet:
`TransactionsSwept` removes tracked locks upstream, and DET already removes the
corresponding history rows in `wallet_backend/event_bridge.rs`. This does not
expose a safe manual reconciliation API for conflicts found only in DET's
persisted historical records after the upstream live records were evicted.

Unconfirmed funding rows expose a disabled **Cancel transfer** action with the
reason in the details. Confirmed funding cannot be reversed by deleting a local
record. Neither age, absence from an explorer, nor a provisional input conflict
is sufficient to release inputs or expire funding. No records or reservations
are deleted by this UI.

Tracked locks also lack durable recipient intent and creation time. Safe
cancellation, verified Platform consumption, and idempotent continuation require
upstream support; they remain WAL-034 gaps rather than implemented actions.

## Regression checks

Synthetic tests cover merged history without duplicate TXIDs, funding-only
records, pending-first ordering, historical recovery distinguished from pending transfers,
restored-history conflicts, stale async results, and light/dark/narrow UI layouts.
The UI regression fails on the card-based version because it has no Unconfirmed
row in the Core table. No live fund movement is used for these tests.

## Advanced wallet actions

The toolbar groups Import key, Refresh, Full resync, and (on Testnet) Get test
DASH in Advanced. Refresh retains its existing balance/transfer-record behavior.
Full resync requests a Core filter scan from genesis for the selected HD wallet
through the pinned manager's `spv_rescan_filters_blocking` API. The blocking
manager call runs off the async executor. The backend rejects requests while
Core is disconnected or already syncing, and an unknown wallet cannot trigger
a scan of another wallet. Wallet records and keys are retained.

The same blocking progress overlay used for initial sync displays scan progress.
It stays open until the selected wallet catches up and the Core sync pipeline
finishes, even if the previous network status still says Synced when the request
starts. Cancellation uses the existing two-step Stop syncing flow. Task errors
and network switches clear the overlay; stale task results cannot clear a newer
scan’s overlay. The request rewinds an
in-memory checkpoint: closing the app before completion requires requesting the
scan again. This is a Core rescan, not a rebuild of Platform/shielded state or a
verified Platform delivery check. Historical conflict cleanup is not guaranteed.
