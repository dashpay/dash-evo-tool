# Unfinished Core → Platform transfers

## Delivered scope

This change implements the visibility/status-only option of the transfer recovery
plan. The wallet shows unfinished Platform-address funding beside its balance,
with a link to wallet-level Transaction History in every user role. Pending Core
transactions sort ahead of dated history. An unresolved funding operation appears
once, with its Core transaction ID, funding amount, known fee, and confirmation
date (or an explicit unknown value).

Status checking reads the upstream tracked locks and DET's event-sourced history,
which is hydrated from persistence at wallet load. A confirmed competing input
spend is reported even when evicted from upstream's live transaction list. This
is an observation, not a finality verdict or permission to release funds.
Requests run through BackendTask, coalesce, retain previous data on failure, and
reject responses for an obsolete wallet/network/request. Refreshing status never
resumes or broadcasts a transaction.

## Backend limits

The pinned platform revision is `9f7ed16935bd540e4c2a542688800d2953f9f67a`.
Its `wallet/asset_lock/sync/recovery.rs` explicitly lacks a verified finalized
ancestry predicate. `abandon_transaction` applies to a signed Core transaction
chosen not to be sent, not an already-broadcast funding lock. There is no durable
asset-lock cancellation/rebroadcast-control API. These limitations prevent safe
cancellation and reconciliation in a DET-only change.

Tracked locks persist the funding transaction but not the requested Platform
recipients, original transfer amount, fee strategy, or creation time. The UI
therefore labels the funding amount accurately, does not invent recipient intent
or unavailable totals, and treats confirmed-but-unconsumed/recovered locks as
delivery unverified. A consumed lock is excluded from the unfinished group.
No new ledger, reservations, or secret storage is introduced.

## Follow-up requirements

- Upstream read-only evidence assessment with verified chain membership/finality,
  historical record lookup, atomic reconciliation, and durable retry suppression.
- Durable operation intent recorded before dispatch on both funding paths,
  including recipients, fee strategy, creation time, and an unambiguous TXID link.
- Cancellation confirmation, crash recovery, and per-input release only after
  those backend contracts are available. No timeout-based refunds.
- Verified Platform consumption status and a reviewed, idempotent continuation
  of the same funding operation. Existing advanced funding tools are unchanged;
  this status view does not add a new retry/finish action.
- Exact backend-attributed unavailable amounts and fee/delivery reconciliation.

## Acceptance checks

Use synthetic wallets and transactions only. Verify pending-first ordering,
conflict observations from restored history, missing-history uncertainty,
consumed/identity/shielded exclusions, late response rejection, and refresh
coalescing. In the UI, verify discovery from the balance, one funding entry,
unknown values, offline guidance, both themes, narrow layouts, and keyboard
navigation. No live fund movement is required for this scope.
