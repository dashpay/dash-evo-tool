# Secret Storage Seam — UX Behavior (Tier-2 Keep-Protection)

**Date:** 2026-06-19 (revised 2026-06-23)
**Status:** Current — describes the Tier-2 keep-protection design that shipped in PR #865.
**Scope:** User-facing behavior for wallet secret migration onto the unified storage seam.
Architecture: `docs/ai-design/2026-06-19-secret-storage-seam/`. Authoritative source:
`src/wallet_backend/secret_access.rs`, `src/wallet_backend/single_key.rs`,
`src/context/wallet_lifecycle.rs`.

> **Note on history.** An earlier draft of this document described a "drop-protection"
> interim design: wallets would be downgraded to file-permission-only protection on
> migration, and one-time disclosure notices (Copy A/B/D) would be shown. That design
> was **retired before any code was written** — see `src/context/wallet_lifecycle.rs:20-24`
> for the rationale comment. The current document describes what actually shipped.

---

## What shipped: Tier-2 keep-protection

On first use after the storage-seam migration, a password-protected secret is decrypted
inside a borrowed scope and immediately **re-wrapped** under a **Tier-2 object-password
envelope** (Argon2id key-derivation + XChaCha20-Poly1305 authenticated encryption) sealed
under **the same password** the user already set. Protection is kept; it is never
downgraded.

Consequences:

- `WalletMeta.uses_password` stays `true` for protected HD wallets.
- `ImportedKey.has_passphrase` stays `true` for protected imported keys.
- The wallet continues prompting just-in-time for every secret access.
- The legacy AES-GCM envelope is deleted after re-wrap.
- No at-rest regression — nothing to disclose.

Unprotected secrets (no-password wallets, identity keys, no-passphrase imported keys)
migrate to the raw `SecretBytes` path (keyless vault, obfuscation-only). These also produce
no UX change — they were never user-password-protected.

---

## The situation, stated plainly for the persona

Alex (the Everyday User) set a password on a wallet. After updating to this version and
opening the wallet:

1. Sees the familiar unlock prompt, types the password. *No surprise — same as always.*
2. Wallet opens. The migration re-wraps the secret silently inside the unlock gesture.
   No extra modal, no banner, no notice.
3. Next time, the wallet asks for the password again. *Expected — protection is kept.*

There is no "last time you'll be asked for your password" moment. No one-time disclosure
notice fires. Alex's mental model ("I set a password and the app still asks for it")
remains accurate throughout.

---

## Disclosure surfaces

| Trigger | Notice | Banner type |
|---|---|---|
| Protected HD wallet migrates (lazy, at first unlock) | *none — silent* | — |
| Protected imported key migrates (lazy, via chokepoint) | *none — silent* | — |
| No-password wallet migrates (eager, on load) | *none — no UX change* | — |
| App start | *none* | — |

The migration produces no disclosure because protection is kept. The user set a password;
the password still works; nothing changed from their perspective. Surfacing a security
notice would alarm users about a change they cannot perceive and that does not weaken
their wallet.

Notes:
- **Headless / MCP:** protected wallets do not lazily migrate without a GUI unlock.
  No notices fire headlessly. The legacy reader serves silently.
- **No-password wallets:** eager migration (on load) produces no notice. Nothing
  changes from the user's point of view.

---

## Per-secret encryption (Tier-2)

Protected secrets use per-secret, per-password Argon2id + XChaCha20-Poly1305 envelopes via
`SecretSeam::put_secret_protected` / `get_secret_protected`. The AAD is bound to
`wallet_id ‖ label`, so envelopes are non-transferable between secrets. Two secrets
protected under different passwords cannot decrypt each other — the property tested by
`TS-T2-SK-ISO` in `src/wallet_backend/secret_access.rs`.

The keyless-vault residual (identity keys, no-password secrets) uses
`put_secret` / `get_secret` (raw path). Per-secret encryption for keyless scopes is the
deferred tier.

---

## Item 4 — SEC-201 (passphrase-modal Enter-consume) — cross-reference

**Not fixed here.** See `src/ui/components/passphrase_modal.rs`. With Tier-2,
every secret access re-prompts for protected wallets, which makes this existing papercut
visible more often than before. If SEC-201 is unfixed when this ships, expect a modest
uptick in Enter-key friction reports from users with protected wallets. That is the
migration surfacing an existing bug, not a regression introduced by this work.

---

## i18n compliance

No user-facing copy was added or changed by this migration. Future notices in this area
must follow the project i18n rules:

- Complete sentences, no fragment concatenation.
- Named placeholders only (`{wallet}`, `{key}`), no positional grammar assumptions.
- No logic embedded in text.
- No jargon in persona-facing copy; technical detail belongs in the `with_details` panel.
- Each string a single, extractable translation unit.
