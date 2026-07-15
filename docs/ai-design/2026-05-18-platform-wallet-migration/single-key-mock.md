# Single-Key Mock Design

**Purpose:** `SingleKeyBackend` trait boundary, stub behavior, user-facing message, and isolation from the HD wallet migration.

[← back to README](README.md)

---

Cross-references: [open-questions.md #7](open-questions.md) — Decision #7: ship mock now, swap to `SingleKeyPlatformWallet` when upstream ships a non-HD wallet type; [backendtask-contract.md](backendtask-contract.md) — `SendSingleKeyWalletPayment` and `RefreshSingleKeyWalletInfo` dispositions; [removal-inventory.md § RETAIN](removal-inventory.md#retain) — why the legacy table is not dropped; [g2-mock-boundary.md](g2-mock-boundary.md) — `PersistedWalletLoader` follows this same swappable-stub pattern; [phasing.md § Cluster C](phasing.md#cluster-c----srcbackend_taskcoremodrs-8-errors) — P0.5 routes single-key dispatch arms to `TaskError::SingleKeyWalletsUnsupported` as part of the compile floor.

> **Phasing note:** The stub boundary is established at **P0.5**, not P1. Single-key task arms (Cluster C of the P0.5 compile-floor work) return `TaskError::SingleKeyWalletsUnsupported` from the compile floor onward. The `SingleKeyBackend` trait and `SingleKeyStub` type are formalized in P1 but the observable behavior (typed error, user-facing banner) is present from P0.5.

## F. Single-Key Mock Design

### Trait Boundary

A `SingleKeyBackend` trait — DET-internal, object-safe — with the operations the UI needs:

- `refresh(key_id)` — refresh balance/UTXOs
- `send(key_id, recipient, amount)` — send payment
- `list()` — list keys
- `import(wif_key)` — import a key

Two impls anticipated:
- `SingleKeyStub` — now (this spec)
- `SingleKeyPlatformWallet` — when upstream ships a non-HD wallet type

`WalletBackend` holds `Arc<dyn SingleKeyBackend>`. Swapping is a one-line construction change. Trait is justified here (M-DI-HIERARCHY: two impls anticipated, object-safe, avoids a future blast-radius change).

### Stub Behavior

Every operation returns `Err(TaskError::SingleKeyWalletsUnsupported)`.

`TaskError::SingleKeyWalletsUnsupported` is a dedicated fieldless variant — typed, no `String` field; `#[error("…")]` carries the user-facing message. No catch-all string (CLAUDE.md error taxonomy rules).

### User-Facing Message

Shown via `MessageBanner` on single-key screens:

> "Single-key wallets are not supported in this version. Your single-key wallet data is preserved and will work again in a future update. To manage funds now, use an HD (recovery-phrase) wallet."

Requirements met (CLAUDE.md error-message rules):
- Audience: Everyday User persona — no jargon
- Structure: what happened + what to do (self-resolvable)
- Tone: calm, direct, not alarming
- No technical details in the message itself; no "contact support"
- Action available: use an HD wallet

### UI Rendering

Single-key screens support import, list, and sign operations — these are backed
by the upstream `SecretStore` and work in full in this release.

Sending funds (`RefreshSingleKeyWalletInfo`, `SendSingleKeyWalletPayment`) return
`TaskError::SingleKeyWalletsUnsupported`. When those arms fire, the banner is
displayed automatically by the `TaskError` → `MessageBanner` path — no per-screen
handling needed.

### Isolation

- The legacy `single_key_wallet` table is **not migrated and not dropped** — [data-model-and-migration.md](data-model-and-migration.md) drops only HD wallet/UTXO/SPV tables.
- The single-key code path never touches `WalletBackend` / `PlatformWalletManager`.
- When upstream lands a non-HD wallet type: implement `SingleKeyPlatformWallet`, add a migration step, flip construction in `WalletBackend::new()`. Zero blast radius elsewhere.
