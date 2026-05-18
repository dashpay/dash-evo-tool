# DIP-14/15 Contact-Derivation Migration and Hard-Stop

**Purpose:** Full design for the DIP-14/15 derivation migration — per-contact migrate-or-quarantine policy, hard-stop behavior, escalation (user + developer), P0 probe reclassification, revised P4 gate, and secret boundary.

[← back to README](README.md)

---

Resolves [open-questions.md § Decision #6](open-questions.md#decision-6--dip-1415-parity-policy) — RESOLVED: migrate or hard-stop + escalate.

> **SUPERSEDES / WITHDRAWS** the soft fallback ("keep DET derivation for existing contacts, use upstream for new contacts") from earlier versions of this spec. Dual-derivation coexistence is no longer permitted — it would ship two engines indefinitely and silently tolerate divergence. The sole sanctioned outcome is: every contact provably migrated OR quarantined with user and maintainers loudly informed and legacy data preserved.

---

## 6.1 — What "Migrate" Means

For every existing established DashPay contact: prove upstream derivation reproduces the EXACT payment address set DET historically derived and used, then record the upstream contact xpub + address mapping into upstream `IdentityManager`/persister.

**Procedure:**

1. Enumerate established contacts + params (owner identity id, contact identity id, account) + historical highest-used index from `src/database/dashpay.rs`, `src/database/contacts.rs`.
2. DET-derive reference for `index ∈ [0, highest_used]` via `derive_dashpay_incoming_xpub → derive_payment_address` (`src/backend_task/dashpay/hd_derivation.rs:22,35`; `ckd_priv_256` `src/backend_task/dashpay/dip14_derivation.rs:18,186`; P2PKH `hd_derivation.rs:54`) = ground truth.
3. Upstream-derive candidate using same params via `derive_contact_xpub → derive_contact_payment_address(_es)` (`packages/rs-platform-wallet/src/wallet/identity/crypto/dip14.rs`).
4. On success: record upstream mapping via `EstablishedContact`/`ContactRequest` path (`packages/rs-platform-wallet/src/lib.rs`), persisted through `SqliteWalletPersister` (PR #3625).

**Success criterion:** A contact is migrated if and only if for EVERY `index ∈ [0, highest_used_index]`:
- Upstream `Address` is byte-identical to DET `Address` (script + network + encoding)
- Contact `ExtendedPubKey` bytes match

Per-contact: all-or-nothing.

## 6.2 — Possible vs Impossible

**Structural finding from P0 (CONFIRMED — state as fact, not risk):**

DET hardcodes the derivation path `m/9'/5'/15'/{account}'` for all networks (`dip14_derivation.rs:176`). Upstream `AccountType::DashpayReceivingFunds` uses `m/9'/{coin}'/15'/0'/(sender)/(recipient)`, where coin-type varies by network (testnet coin=1') and account is fixed at 0' rather than appearing in the path.

On Mainnet with full-256-bit inputs and account 0, addresses at indices 0–5 are **byte-identical** between DET and upstream — the `CKDpriv256` primitive converges. Divergence is confined to: path coin-type differences, account placement in the path, xpub version bytes, and account-reference encoding. Migration is mechanically tractable via re-derivation on the upstream path; no upstream crypto fix is required.

Expected post-migration residue: non-mainnet contacts and non-account-0 contacts may be quarantined. P0 probe divergence on the full-256-bit class is recorded as a **release-blocking finding** — execution continues, but the quarantine machinery is the sole safety net until an upstream crypto fix or explicit acceptance is recorded.

**Identifier classes:**
- **Low-index** (first 28 bytes zero, `is_index_less_than_2_32` true, `dip14_derivation.rs:205`) — expected migratable but still asserted.
- **Full-256-bit** (high bytes set) — class where path divergence is most visible; focus of P0 probe and runtime audit.

DET's `index_to_child_number` (`src/backend_task/dashpay/dip14_derivation.rs:213-240`) collapses the 256-bit index to 31 bits via `sha256(index)[0..4] & 0x7FFFFFFF` ONLY where a legacy `ChildNumber` is stored; the derived key uses the full 256-bit index in both implementations (`ckd_priv_256` in DET, `ChildNumber::Normal256` upstream). Both agree on identifier-to-index encoding (raw 32 bytes, not hashed) and P2PKH address format.

| Predicate | Definition |
|---|---|
| **Migratable(contact)** | ∀ index ∈ [0, highest_used]: `upstream_addr == det_addr` AND `upstream_contact_xpub == det_contact_xpub` |
| **Impossible(contact)** | ∃ index: `upstream_addr != det_addr` — a historically-transacted address upstream cannot reproduce → funds unreachable via new backend |

Migratability is NEVER inferred from identifier class. It is computed per contact over the real historical index range.

## 6.3 — Hard-Stop Behavior

On ANY impossible contact: quarantine the affected contact(s) AND block DashPay backend cutover. Do NOT abort migration of other data. Do NOT silently proceed. Do NOT auto-mutate or delete user data.

**A04 fail-secure ordering (extends the `*.db.premigration` design in [data-model-and-migration.md](data-model-and-migration.md)):**

1. Back up legacy DB → `*.db.premigration` before any destructive step.
2. Attempt per-contact migration — migratable contacts are recorded into upstream/persister; impossible contacts are collected into the quarantine set and left intact in legacy `dashpay`/`contacts` tables.
3. On any impossible contact: do NOT drop legacy DashPay/contact tables (HD-wallet/UTXO/SPV legacy tables may still drop if cleanly migrated, but ALL DashPay/contact legacy tables are retained while any quarantine entry is non-empty). Refuse DashPay backend cutover — the app starts, wallet/identity/non-DashPay flows proceed normally on the new backend, but DashPay send/receive to quarantined contacts is blocked. Mark the upgrade incomplete via a persistent flag, re-evaluated on each launch until the quarantine is cleared. Preserve `*.db.premigration` while any quarantine flag is set.
4. Surface escalation (see §6.4).

**Rationale:** Wholesale abort would strand wallets and identities on old unsupported code. Quarantine isolates the fund-risk surface, is reversible, and is the least-harm reading of hard-stop — stop the affected fund flows loudly and reversibly, not the whole product.

## 6.4 — Escalation — Two Audiences

### User-Facing

Blocking non-ignorable `MessageBanner` (`src/ui/components/message_banner.rs`) on launch while any contact is quarantined:

> "One or more of your DashPay contacts could not be upgraded to the new wallet engine. Your funds are safe and unchanged. Affected contact: `<Base58 contact id…>`. While this is unresolved, payments to and from this contact are paused; all your other wallets and contacts work normally. You can keep using the previous version of the app to transact with this contact, or wait for an update that resolves this. Your previous data has been preserved."

Requirements met (CLAUDE.md error-message rules):
- Blocking only for the affected DashPay flows, not the whole app.
- Base58 id is a copyable handle (CLAUDE.md rule 6 — Base58 identifiers are allowed in messages).
- Funds-safe stated explicitly.
- Two self-resolvable actions provided.
- Preservation of prior data stated.
- No technical detail in the message; `Debug` repr via `BannerHandle::with_details`.

### Developer-Facing

Structured `tracing` error per impossible contact with fields:
- `owner_identity_id`, `contact_identity_id`, `account_index`
- `identifier_class`, `first_divergent_index`
- `det_address`, `upstream_address`
- `det_contact_xpub_fp`, `upstream_contact_xpub_fp`

(M-LOG-STRUCTURED; NO seeds/keys/raw identifiers beyond public Base58 — A09.)

Machine-readable `dip14-quarantine-report.json` in app data directory (same fields, all quarantined contacts).

**Typed error:** `TaskError::DashPayContactDerivationIrreconcilable { contact: Identifier }` — no string-stuffed variants; message via `#[error("…")]`. No `String` field (CLAUDE.md: never store user-facing strings in error variants).

## 6.5 — P0 Probe and Phasing Interaction

Two independent required gates (not redundant):

**P0 golden-vector probe** (static, pre-impl): synthetic seeds, both identifier classes, both networks. Asserts byte-equality of contact xpub + payment addresses + account-reference. This is the existing P0 lane in [phasing.md](phasing.md).

- If divergence on the full-256-bit class: the P0 probe does NOT silently pass — it becomes a release-blocking finding, forcing either (i) an upstream `dip14.rs` fix to converge (preferred) or (ii) acceptance that the runtime migrate-or-hard-stop is the sole safety net (permitted ONLY because that path provably never allows funds to become silently unreachable).
- P0 divergence escalates the requirement on the runtime path and forbids any "assume equivalent" shortcut. It does NOT permanently block the project.

**Revised P4 gate** (deletion of `src/backend_task/dashpay/dip14_derivation.rs` / `hd_derivation.rs`):

SAFE regardless of P0 outcome; CONDITIONED on runtime migrate-or-hard-stop being implemented and proven. Rationale: the runtime path re-derives every existing contact via both engines using DET logic AT migration time, before deletion. It records upstream mappings only for matches; hard-stops, quarantines, and preserves legacy data for non-matches. After migration, the artifact (recorded mappings + quarantine report + retained legacy tables) is the source of truth, not the DET code — so DET derivation is deletable.

Revised P4 gate: "DET DashPay derivation may be deleted once the one-time migration (§6.1) has executed for all users in the migration path AND the quarantine/hard-stop path (§6.3–6.4) is implemented and QA-covered."

NOT gated on "zero P0 divergence." Gated on "safety net exists and is proven."

**QA lane** (extends P4 correctness gate, release-blocking, runs P3 + P4):

Fixtures with: low-index contact (expect migratable), deliberately-divergent full-256-bit contact (expect quarantine), mixed set. Asserts:
- Migratable → persister byte-identical mapping
- Divergent → quarantined + legacy DashPay/contact tables retained + `*.db.premigration` preserved + blocking banner + structured diagnostic + app starts with non-DashPay flows intact + DashPay to quarantined contacts blocked

## 6.6 — Secret Boundary

No contradiction with `SECRETS.md`. Re-derivation (DET reference + upstream candidate) uses in-memory seed via DET existing encrypted-seed unlock (zeroize-on-drop), as in steady-state today. Only PUBLIC material is persisted: upstream `ContactXpubData`/`EstablishedContact` mapping (public xpub + P2PKH) through the changeset pipeline into `SqliteWalletPersister` (public-only, CI-enforced by `tests/secrets_scan.rs`). The quarantine report contains only public addresses and identity ids — no seeds, keys, or xpriv (A09/ASVS V14.2). Seeds never enter the persister.
