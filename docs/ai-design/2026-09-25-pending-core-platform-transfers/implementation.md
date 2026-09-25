# Core → Platform funding history

## Delivered scope

Funding observations enrich rows inside the wallet-level **Dash Core
Transactions** table. Each Core TXID appears once, including a tracked funding
transaction missing from hydrated history. Unconfirmed transactions sort first;
confirmed funding retains its chronological position. **Details** opens the
recorded funding information and any observed competing input spend.

The balance summary counts transfers that need review. A `RecoveredFromChain`
record represents historical funding with an unknown Platform outcome, not
proof of an unfinished operation, and is excluded from that count. Recovered
records remain in history. Core confirmation and Platform delivery are distinct.

**Reload wallet records** reads tracked locks and hydrated local history. It
neither queries Platform consumption nor broadcasts or resumes a transaction.
The details explain that limitation. **Open Core explorer** provides an optional
external check of Core confirmation only; the explorer is not an accounting
or cancellation authority. Async requests remain scoped to wallet/network,
coalesced, and timeout-bounded. Details close when wallet/network changes.

## Cancellation and expiration remain unavailable

The pinned platform revision is `9f7ed16935bd540e4c2a542688800d2953f9f67a`.
Its `wallet/asset_lock/sync/recovery.rs` lacks a verified finalized-ancestry
predicate. `abandon_transaction` releases a signed Core payment the caller
chose not to send; it is not an API for cancelling an already-broadcast asset
lock. The backend also lacks durable asset-lock cancellation/rebroadcast control.

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
records, pending-first ordering, historical recovery excluded from review counts,
restored-history conflicts, stale async results, and light/dark/narrow UI layouts.
The UI regression fails on the card-based version because it has no Unconfirmed
row in the Core table. No live fund movement is used for these tests.
