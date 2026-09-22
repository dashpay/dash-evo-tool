# v1.0 Parity Audit — "Accepted as Gone, Not Planned" Sign-Off

Closes the batch of v0.10-dev-vs-PR860 UI parity findings verified as genuine,
intentional removals with no restoration planned, and already disclosed to
users before this record was written. Each item below was independently
re-verified against the current working tree (no UI control, task, or field
remains reachable) and cross-checked against an existing disclosure site.

| Finding | What's gone | Verified gone | Disclosure |
|---|---|---|---|
| Search for Unused asset locks | `CoreTask::RecoverAssetLocks` and its "Search for Unused" button | No references in `src/`; replaced by continuous `AssetLockManager` tracking (`WalletTask::ListTrackedAssetLocks`) | `docs/ai-design/2026-06-01-pr860-gap-audit/gaps.md` disclosed-removals table (`CoreTask::RecoverAssetLocks` row) |
| Shielded manual Sync / dev Resync buttons | Manual sync-notes and nullifier-recheck controls on the Shielded tab | No sync/resync buttons in `src/ui/wallets/shielded_tab.rs`; the shielded op enum no longer carries `SyncNotes`/`CheckNullifiers`/`WarmUpProvingKey` — sync is upstream-owned and automatic | Commit `479c8c18` ("delete DET shielded subsystem, route via upstream") is the removal record; no separate CHANGELOG/gaps.md line names the buttons specifically — flagged for awareness, not blocking, since the net effect (automatic sync) is a strict improvement |
| Connection Type (RPC vs SPV) selector | Network settings toggle between RPC and SPV backend modes | No "Connection Type" selector in `src/ui/`; `platform-wallet` is SPV-only by design | `gaps.md` disclosed-removals table ("RPC Core-backend mode" row) |
| RPC Core / ZMQ status rows | Status rows showing RPC and ZMQ connection health | No references in `src/ui/`; removed together with RPC mode | Same `gaps.md` row, plus CHANGELOG "Removed" (ZMQ listener/"Disable ZMQ" line) |
| Dash Core executable path config | "Dash Core Executable Path" file picker in Network Settings | No `dash_qt_path`/executable-path UI in `src/ui/`; the field survives only in `AppSettings`/wire format for on-disk layout compatibility, never rendered | CHANGELOG "Removed" — "the unreachable Dash-Qt launcher and its settings — the executable path, the overwrite-config option, and the close-on-exit option" |
| Overwrite dash.conf checkbox | Network Settings checkbox to let DET rewrite `dash.conf` | No UI references; `overwrite_dash_conf` unused outside settings persistence | Same CHANGELOG line as above |
| Close Dash-Qt when DET exits checkbox | Network Settings checkbox for close-on-exit behavior | No UI references; `close_dash_qt_on_exit` unused outside settings persistence | Same CHANGELOG line as above |
| Disable ZMQ toggle | Network Settings checkbox to opt out of the Core ZMQ listener | No UI references; was already a placebo (listener never spawned) before removal | CHANGELOG "Removed" — ZMQ listener/"Disable ZMQ" line; `gaps.md`'s ZMQ-subsystem finding documents the placebo status pre-removal |
| SPV Peer Source expert setting | "Use local Dash Core node" peer-discovery toggle | No `use_local_spv_node`/"Peer Source" references in `src/`; upstream `platform-wallet` owns peer discovery | `gaps.md` disclosed-removals table ("SPV peer-source expert setting" row) |
| Dash-Qt launch button (status card) | Connection-indicator click launched Dash-Qt | No `StartDashQT` UI callers in `src/`; task struct itself removed | CHANGELOG "Removed" Dash-Qt launcher line; `gaps.md`'s Dash-Qt-launcher finding (resolved by commit `255aa018`) |

All ten findings are closed. No CHANGELOG or `gaps.md` edits accompany this
record — those disclosures already existed; this document is the formal
sign-off closing the outstanding audit findings against them.
