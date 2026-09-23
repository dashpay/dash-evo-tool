# WalletContext ownership

Each network's AppContext creates one shared WalletContext before constructing
WalletBackend. The backend and SecretAccess use that same owner. It holds private
HD/imported-key registries and the committed metadata used by UI labels, MCP
resolution and password prompts. Readers receive owned snapshots or runtime wallet
handles; they cannot mutate registry membership or live names through those handles.

WalletContext lives in `src/wallet_backend/wallet_context.rs`. It is a backend-side
live cache: its writers are wallet_backend adapters (`SingleKeyView`,
`WalletMetaView::with_context`, `SecretAccess::with_wallet_context`,
`WalletBackend::hydrate_context_wallets`) and it depends on wallet_backend types.
AppContext holds the `Arc`, exposes it through `AppContext::wallet_context()`, and
drives the HD lifecycle mutators: `register_wallet` calls `register_hd`,
`remove_wallet` calls `remove_wallet`, and `clear_network_database` calls `clear`.
These mutators are `pub(crate)` for that reason.
The module is `pub(crate)`, and `pub` only under `test` or the `bench` feature so
benches can build a standalone instance.

One mutex orders metadata mutations, registration, imports and hydration for this
desktop client, where MCP concurrency is low. Persistence runs under this writer
mutex, then a short write to the separate state RwLock publishes the successful
result. Snapshot and prompt readers (`hd_alias`, `hd_prompt`, `single_alias`, the
registry readers) take only the state lock, so a slow write does not block them.
Bound `WalletMetaView::get/list/try_get` are the exception: they read storage
through `read_metadata`, which takes the writer mutex. Failed persistence does not publish the proposed metadata. These
are two lock roles, not one lock held across both storage and reads.

WalletContext methods own alias selection, separate HD/imported-key namespaces,
membership and publication. Backend storage adapters supply synchronous persistence
callbacks. Hydration and compatibility reads that can upgrade stored records join
the writer ordering; their callbacks use raw storage adapters to avoid recursively
acquiring the writer mutex. Password acquisition and async backend work happen
outside these callbacks.

Lock order: writer mutex, then the state RwLock, then an inner wallet handle lock
(`rename_hd` reads the wallet's xpub while holding the writer). No WalletContext
method holds the state lock while acquiring an inner wallet lock, so callers may
read snapshots (`hd_alias`, `hd_prompt`) while holding an inner wallet guard; keep
it that way when adding mutators. Never call a
WalletContext mutator, or a backend-bound metadata view such as
`WalletBackend::wallet_meta()`, while holding an inner wallet guard. Never
re-enter the writer from a persistence callback. The writer records its holder per
thread, so a same-thread re-entry panics with a clear message instead of
deadlocking silently.

Wallet and SingleKeyWallet retain a crate-private `initial_alias` solely for
construction, legacy hydration and serialization adapters. It is never the live
name; consumers query WalletContext by seed hash or imported-key address. Existing
Arc/RwLock wallet handles still carry balances, derivation and signing state.

This refactor preserves existing synchronous UI mutation scheduling and best-effort
wallet secret cleanup. It does not introduce storage transactions across vault and
sidecar writes, change upstream wallet synchronization, or consolidate unrelated
SPV/signing locks. UI-triggered synchronous writes can still take their existing
storage latency; snapshot and prompt readers do not participate in that writer
gate.

Regression coverage includes failure-before-publication, readers during a paused
writer, hydration ordered with rename, retained-handle isolation, backend aliases,
legacy migration, MCP ambiguity and wallet rename UI behavior.
