# G2 Mock Boundary — Persisted-Wallet Load

**Purpose:** Design of the `PersistedWalletLoader` seam that resolves Decision #2 — the seam is mocked with seed-re-registration now and swappable for real persisted-load when upstream `Wallet::from_persisted` lands.

[← back to README](README.md)

---

Resolves [open-questions.md § Decision #2](open-questions.md#decision-2--g2-seed-re-registration-ux) — RESOLVED: mock it.

"Mock it" means: the SEAM is mocked; the BEHAVIOR behind it is the upstream-prescribed seed-re-registration, swappable for real persisted-load when upstream `Wallet::from_persisted` lands. G2 is downgraded from a hard implementation gate to a deferred one-line swap.

## G2.1 — The Seam

DET-internal object-safe trait `PersistedWalletLoader` (mirrors `SingleKeyBackend` in [single-key-mock.md](single-key-mock.md)):

```rust
fn wallets_to_register(&self, ctx: &StartupContext) -> Result<Vec<WalletRegistration>, TaskError>
```

Returns DET-opaque descriptors (seed handle + network + alias + `is_main`), NOT `platform-wallet` types (M-DONT-LEAK-TYPES, C-NEWTYPE-HIDE).

Two impls:
- **`SeedReregistrationLoader`** — ships now. Reads DET retained encrypted-seed store, yields one `WalletRegistration` per wallet.
- **`UpstreamFromPersisted`** — ships when upstream `Wallet::from_persisted` + `persister.load()` wallets land. Delegates to reconstructed `ClientStartState.wallets`.

`WalletBackend` holds `Arc<dyn PersistedWalletLoader>`.

**Plug point** (see [backend-architecture.md § WalletBackend](backend-architecture.md)): runs BETWEEN `PlatformWalletManager` construction and `start()`. The constructor (`packages/rs-platform-wallet/src/manager/mod.rs`) takes `sdk`/`persister`/`app_handler` and does not auto-start sync — "call start after wallets are registered." For each `WalletRegistration`, the backend calls:
1. `create_wallet_from_seed_bytes(network, seed_bytes, WalletAccountCreationOptions)`
2. `load_persisted()` — rehydrates identity/contact/address deltas (`packages/rs-platform-wallet/src/manager/wallet_lifecycle.rs`)
3. `PlatformWalletManager.start()` — spawns `SpvRuntime` sync

## G2.2 — What the Mock Does at Startup (Decisive)

**Option chosen:** The mock IS the seed-re-registration path — it mocks the SEAM, not the BEHAVIOR.

`SeedReregistrationLoader` re-derives each wallet from DET retained encrypted seed (the seed the user already unlocks, as today). Upstream `load_persisted()` layers identity/DashPay/UTXO/asset-lock deltas. `SpvRuntime` re-confirms on sync.

**Rejected option:** Return empty / force re-add from scratch — rejected (data-loss UX, A04).

`wallets_to_register()` returns one `WalletRegistration{seed_handle, network, alias, is_main}` per wallet in DET seed store. App behavior is identical to today: password prompt on launch, repopulate from persister + first sync. The only change is that the logic lives behind the trait for zero-blast-radius later deletion.

## G2.3 — User-Facing Surface

Transparent. No banner or alert (unlike single-key, which is a capability loss). The seed-unlock prompt already exists today; adding a "re-registering" message is over-messaging (CLAUDE.md rules — messages are what-happened + what-to-do; nothing actionable here). Debug `tracing` only (M-LOG-STRUCTURED). The sole error surface is the existing seed-decrypt failure path — already a typed `TaskError` with a calm banner, unchanged.

## G2.4 — Swap Path

When upstream `Wallet::from_persisted` ships and `persister.load()` populates `ClientStartState.wallets` (the `LOAD_UNIMPLEMENTED = ["ClientStartState::wallets"]` gap in `packages/rs-platform-wallet-storage/src/sqlite/persister.rs` closes):

1. **One-line construction swap:** `Arc::new(SeedReregistrationLoader::new(...))` → `Arc::new(UpstreamFromPersisted::new(...))` in `WalletBackend::new()`.
2. `UpstreamFromPersisted` maps reconstructed wallets into `WalletRegistration`.
3. No migration, no-op, persister DB unchanged, seed store retained (secret boundary).
4. Old impl deleted (M-NO-TOMBSTONES).
5. Zero blast radius — only the loader impl + one construction line. `WalletBackend` API, `BackendTask`, UI, persister DB, and seed store are all untouched.

## G2.5 — Gate Impact (Key Consequence)

**G2 REMOVED as a hard implementation gate.**

G1 (PR #3625 merge + pin bump) remains; G2 is downgraded to a deferred swap-in. `SeedReregistrationLoader` is complete, shippable, and behaviorally correct. The project ships on G1 alone. With Decision #1 pinning to the #3625 head now, even G1 is not a start blocker — it becomes a release-hardening item. See [phasing.md § Combined Gate Posture](phasing.md#combined-gate-posture).

## G2.6 — Phasing and QA

**When built:** P1, alongside `WalletBackend` skeleton + `EventBridge`. The seam must exist before P2. `UpstreamFromPersisted` is NOT built now (no upstream API) — the trait slot is reserved, mirroring the single-key reserved-impl pattern.

**P1 deliverables include:** `PersistedWalletLoader` trait + `SeedReregistrationLoader` impl.

**QA lane (P1, P5 regression):**
- Mock yields exactly N `WalletRegistration` for N seed-store wallets; backend registers all N and `start()` succeeds.
- Swap-boundary compiles with an alternate `StubFromPersisted` impl (proves zero-blast swap path).
- Negative: seed-decrypt failure surfaces existing typed `TaskError`, no panic/data-loss.

**Secret boundary:** `SeedReregistrationLoader` reads the DET encrypted-seed store, feeds `seed_bytes` to upstream `create_wallet_from_seed_bytes` in memory. Seeds NEVER enter the persister (`SECRETS.md`, ASVS V14.2). `WalletRegistration` carries a zeroize-on-drop in-memory seed handle, not persisted material.
