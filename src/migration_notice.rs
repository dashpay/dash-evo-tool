//! The mandatory one-time post-migration notice.
//!
//! This is the SOLE compensating control for two accepted trade-offs
//! introduced by the platform-wallet migration: (a) payments received from
//! DashPay contacts on Testnet/Devnet, or on a secondary account, may not
//! appear in this version (the funds are not lost — the pre-migration data
//! is retained as a backup and remains reachable with the previous app
//! version), and (b) funding an identity directly from a scanned external
//! payment (the former QR-direct-fund path) is no longer supported.
//!
//! Because it is the only compensating control it is **unconditional** and
//! **never downgraded**: shown exactly once to *every* migrated user,
//! regardless of network, account, or whether they actually had any affected
//! contacts. The decision and the wording live here as pure, unit-provable
//! items so the release-blocking guarantee cannot silently regress.

/// Verbatim notice text. MUST NOT be paraphrased, shortened, conditionalised,
/// or made jargon-heavy. Any change to this string is a deliberate,
/// release-gating decision and will fail the migration QA harness.
pub const MIGRATION_NOTICE_TEXT: &str = "This update changes how DashPay \
    contact payment addresses are calculated. Payments you received from \
    DashPay contacts on Testnet or Devnet, or on a secondary account, may \
    not appear in this version. Your funds are not lost — your previous \
    data has been saved as a backup, and you can still access these \
    payments using the previous version of the app. Mainnet payments on \
    your main account are unaffected. Funding an identity directly from a \
    scanned external payment is no longer supported; receive the payment \
    into your wallet first, then fund the identity from your wallet \
    balance.";

/// Pure gating decision for the one-time notice.
///
/// The notice is shown iff the one-time migration has actually finalised on
/// this database (`migration_completed == true`, set by Stage-B's finalise
/// step) and it has not already been shown (`notice_already_shown ==
/// false`). There is intentionally **no** other condition — not network, not
/// account, not contact count — so the sole compensating control reaches
/// every migrated user, and ONLY migrated users.
///
/// `migration_completed` (not merely "not pending") is the discriminator: a
/// brand-new install also has `pending == false` but never migrated, so it
/// must NOT see the notice. The flag is durable and only ever set by the
/// real Stage-B finalise path. This function stays pure: "given these two
/// durable bits, do we show it now?".
pub fn should_show(migration_completed: bool, notice_already_shown: bool) -> bool {
    migration_completed && !notice_already_shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_only_when_migrated_and_not_yet_shown() {
        // (migration_completed, notice_already_shown)
        assert!(
            should_show(true, false),
            "migrated + unseen → show exactly here"
        );
        assert!(
            !should_show(false, false),
            "fresh install (never migrated) → never show"
        );
        assert!(
            !should_show(true, true),
            "migrated + already shown → never again"
        );
        assert!(!should_show(false, true), "not migrated + shown → never");
    }

    #[test]
    fn notice_text_is_verbatim_and_jargon_free() {
        // Pin the exact mandated wording.
        assert_eq!(
            MIGRATION_NOTICE_TEXT,
            "This update changes how DashPay contact payment addresses are \
             calculated. Payments you received from DashPay contacts on \
             Testnet or Devnet, or on a secondary account, may not appear \
             in this version. Your funds are not lost — your previous data \
             has been saved as a backup, and you can still access these \
             payments using the previous version of the app. Mainnet \
             payments on your main account are unaffected. Funding an \
             identity directly from a scanned external payment is no longer \
             supported; receive the payment into your wallet first, then \
             fund the identity from your wallet balance."
        );
        // Jargon guard: none of the forbidden technical terms appear.
        for jargon in [
            "DIP-14",
            "DIP-15",
            "xpub",
            "derivation path",
            "state transition",
            "SDK",
            "RPC",
            "consensus",
            "nonce",
        ] {
            assert!(
                !MIGRATION_NOTICE_TEXT.contains(jargon),
                "notice must stay jargon-free; found {jargon:?}"
            );
        }
    }
}
