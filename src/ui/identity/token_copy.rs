//! User-facing copy for the Identity Hub's token surfaces: the Tokens tab, the
//! token detail page and the Home Tokens card. Kept free of egui so the copy
//! is unit-testable, and every string is one complete translation unit.
//!
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` frame T9.

use crate::model::amount::Amount;
use crate::model::datetime;

/// The holder verb, shared with the Send screen and Tokens › My Tokens.
pub use crate::ui::tokens::transfer_tokens_screen::SEND_LABEL;

// Tab header and controls.
pub const HEADING: &str = "Tokens";
pub const SUBTITLE: &str = "Tokens this identity holds or keeps an eye on.";
pub const REFRESH_LABEL: &str = "Refresh";
pub const REFRESHING_TOOLTIP: &str = "Balances are being checked.";
pub const ADD_TOKEN_LABEL: &str = "Add a token";
pub const FIND_BY_NAME_LABEL: &str = "Find a token by name";
pub const ADD_BY_ID_LABEL: &str = "Add by token or contract ID";
pub const RECEIVE_LINK: &str = "How do I receive tokens?";
pub const HIDE_ZERO_LABEL: &str = "Hide tokens with zero balance";
pub const HIDE_ZERO_TOOLTIP: &str =
    "Hides tokens whose balance is 0. Tokens still being checked always stay visible.";
pub const FILTER_HINT: &str = "Filter by name or ID";
pub const SORT_LABEL: &str = "Sort";

// Row actions and menu.
pub const CLAIM_LABEL: &str = "Claim";
pub const MENU_LABEL: &str = "•••";
pub const CHECK_NOW_LABEL: &str = "Check balance now";
pub const DETAILS_LABEL: &str = "Token details";
pub const COPY_ID_LABEL: &str = "Copy token ID";
pub const OPEN_DETAILS_TOOLTIP: &str = "Show this token's details.";
pub const STOP_TRACKING_LABEL: &str = "Stop tracking";
pub const STOP_TRACKING_TITLE: &str = "Stop tracking this token?";
pub const MANAGE_LABEL: &str = "Manage in Tokens screen";
pub const TRY_AGAIN_LABEL: &str = "Try again";
pub const DISMISS_LABEL: &str = "Dismiss";
pub const CANCEL_LABEL: &str = "Cancel";
pub const TRACK_AGAIN_LABEL: &str = "Track tokens again";

// Row badges and status lines.
pub const ISSUED_BY_YOU_BADGE: &str = "Issued by you";
pub const CANT_BE_SENT_BADGE: &str = "Can't be sent";
pub const STATUS_CHECKING: &str = "Checking balance…";
pub const STATUS_COULD_NOT_CHECK: &str = "Couldn't check this balance.";
pub const STATUS_REWARDS: &str = "Rewards may be waiting for you.";
pub const STATUS_ADD_KEY: &str = "Add a key to this identity to send or claim tokens.";

// Reasons a control is disabled; shared with Tokens › My Tokens.
pub const DISABLED_NOT_TRANSFERABLE: &str = "This token cannot be sent to other identities.";
pub const DISABLED_NOT_SYNCED: &str = "Available after sync finishes.";
pub const DISABLED_BALANCE_UNKNOWN: &str = "Available once the balance is checked.";
pub const DISABLED_NOTHING_TO_SEND: &str = "This identity holds none of this token to send.";

// Empty, first-load and failure states.
pub const EMPTY_HEADING: &str = "No tokens yet";
pub const EMPTY_BODY: &str = "Tokens are assets issued on Dash Platform. A token shows here once you add it, so when someone sends you one, add it to see your balance.";
pub const DISMISSED_HEADING: &str = "Nothing tracked for this identity";
pub const FIRST_LOAD: &str =
    "Checking balances for the first time. This usually takes a few seconds.";
pub const FOOTNOTE: &str = "Balances are checked in the background. A token you don't see here can be added with Add a token.";
pub const ERROR_REFRESH_FAILED: &str =
    "Token balances couldn't be updated. Check your internet connection and try again.";
pub const ERROR_REFRESH_SLOW: &str =
    "Updating token balances is taking longer than usual. Try again in a moment.";
pub const ERROR_CONTRACT_MISSING: &str =
    "This token's details aren't available yet. Refresh and try again.";

// Detail page.
pub const BACK_LABEL: &str = "‹ Back to tokens";
pub const ABOUT_HEADING: &str = "About this token";
pub const TOKEN_ID_LABEL: &str = "Token ID";
pub const CONTRACT_LABEL: &str = "Contract";
pub const ISSUED_BY_LABEL: &str = "Issued by";
pub const SMALLEST_UNIT_LABEL: &str = "Smallest unit";
pub const CAN_BE_SENT_LABEL: &str = "Can be sent";
pub const REWARDS_LABEL: &str = "Rewards";
pub const COPY_LABEL: &str = "Copy";
pub const CAN_BE_SENT_YES: &str = "Yes, to any identity.";
pub const CAN_BE_SENT_NO: &str = "No. This token cannot be sent to other identities.";
pub const REWARDS_OVER_TIME: &str = "Paid out to holders over time. Claim them whenever you like.";
pub const REWARDS_NONE: &str = "This token pays no rewards.";
pub const ISSUED_BY_YOU: &str = "Issued by you";
pub const ISSUED_BY_SOMEONE_ELSE: &str = "Issued by someone else";

// Home card and Receive dialog.
pub const TOKENS_CARD_HEADING: &str = "Tokens";
pub const SEE_ALL_TOKENS_LABEL: &str = "See all tokens";
pub const RECEIVE_TOKENS_HINT: &str = "A token you receive shows under Tokens once you add it.";

/// "Last updated {time}."
pub fn last_updated_text(time_ms: u64) -> String {
    format!(
        "Last updated {}.",
        datetime::local_date_time_from_unix_millis(time_ms)
    )
}

/// "Showing balances from {time}." — after a failed refresh.
pub fn stale_since_text(time_ms: u64) -> String {
    format!(
        "Showing balances from {}.",
        datetime::local_date_time_from_unix_millis(time_ms)
    )
}

/// "You can claim {amount} once."
pub fn can_claim_once_text(amount: &Amount) -> String {
    format!("You can claim {amount} once.")
}

/// "Every identity can claim {amount} once." — the detail page's rewards fact.
pub fn once_claim_text(amount: &Amount) -> String {
    format!("Every identity can claim {amount} once.")
}

/// "Claimed on {date}."
pub fn claimed_on_text(date: &str) -> String {
    format!("Claimed on {date}.")
}

/// "{count} tokens are hidden." / "1 token is hidden."
pub fn hidden_count_text(count: usize) -> String {
    if count == 1 {
        "1 token is hidden.".to_string()
    } else {
        format!("{count} tokens are hidden.")
    }
}

/// "Tracking {count} tokens." — the header's count on a long list.
pub fn tracked_count_text(count: usize) -> String {
    format!("Tracking {count} tokens.")
}

/// "You stopped tracking every token for {identity}. …"
pub fn dismissed_body_text(identity: &str) -> String {
    format!(
        "You stopped tracking every token for {identity}. Tokens you track on other identities are unaffected."
    )
}

/// "Stop tracking {token} for {identity}? …"
pub fn stop_tracking_question(token: &str, identity: &str) -> String {
    format!("Stop tracking {token} for {identity}? You can add it again at any time.")
}

/// Tooltip of the tab badge and the Home card's claim line.
pub fn pending_claims_text(count: usize) -> String {
    if count == 1 {
        "1 token has something to claim.".to_string()
    } else {
        format!("{count} tokens have something to claim.")
    }
}

/// "Held by {identity}" — the detail page's caption.
pub fn held_by_text(identity: &str) -> String {
    format!("Held by {identity}")
}

/// Accessible name of a row's ••• button: "More actions for {token}".
pub fn more_actions_label(token: &str) -> String {
    format!("More actions for {token}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every complete sentence.
    const SENTENCES: &[&str] = &[
        SUBTITLE,
        REFRESHING_TOOLTIP,
        HIDE_ZERO_TOOLTIP,
        OPEN_DETAILS_TOOLTIP,
        STATUS_COULD_NOT_CHECK,
        STATUS_REWARDS,
        STATUS_ADD_KEY,
        DISABLED_NOT_TRANSFERABLE,
        DISABLED_NOT_SYNCED,
        DISABLED_BALANCE_UNKNOWN,
        DISABLED_NOTHING_TO_SEND,
        EMPTY_BODY,
        FIRST_LOAD,
        FOOTNOTE,
        ERROR_REFRESH_FAILED,
        ERROR_REFRESH_SLOW,
        ERROR_CONTRACT_MISSING,
        CAN_BE_SENT_YES,
        CAN_BE_SENT_NO,
        REWARDS_OVER_TIME,
        REWARDS_NONE,
        RECEIVE_TOKENS_HINT,
    ];

    /// Every label, heading and caption: a unit of its own, not a sentence.
    const LABELS: &[&str] = &[
        HEADING,
        REFRESH_LABEL,
        ADD_TOKEN_LABEL,
        FIND_BY_NAME_LABEL,
        ADD_BY_ID_LABEL,
        RECEIVE_LINK,
        HIDE_ZERO_LABEL,
        FILTER_HINT,
        SORT_LABEL,
        SEND_LABEL,
        CLAIM_LABEL,
        MENU_LABEL,
        CHECK_NOW_LABEL,
        DETAILS_LABEL,
        COPY_ID_LABEL,
        STOP_TRACKING_LABEL,
        STOP_TRACKING_TITLE,
        MANAGE_LABEL,
        TRY_AGAIN_LABEL,
        DISMISS_LABEL,
        CANCEL_LABEL,
        TRACK_AGAIN_LABEL,
        ISSUED_BY_YOU_BADGE,
        CANT_BE_SENT_BADGE,
        STATUS_CHECKING,
        EMPTY_HEADING,
        DISMISSED_HEADING,
        BACK_LABEL,
        ABOUT_HEADING,
        TOKEN_ID_LABEL,
        CONTRACT_LABEL,
        ISSUED_BY_LABEL,
        SMALLEST_UNIT_LABEL,
        CAN_BE_SENT_LABEL,
        REWARDS_LABEL,
        COPY_LABEL,
        ISSUED_BY_YOU,
        ISSUED_BY_SOMEONE_ELSE,
        TOKENS_CARD_HEADING,
        SEE_ALL_TOKENS_LABEL,
    ];

    fn formatted() -> Vec<String> {
        let amount = Amount::new(100, 2).with_unit_name("DCF");
        vec![
            last_updated_text(0),
            stale_since_text(0),
            can_claim_once_text(&amount),
            once_claim_text(&amount),
            claimed_on_text("2026-10-20"),
            hidden_count_text(1),
            hidden_count_text(3),
            tracked_count_text(8),
            dismissed_body_text("Alice"),
            stop_tracking_question("DCF", "Alice"),
            pending_claims_text(1),
            pending_claims_text(2),
        ]
    }

    #[test]
    fn every_sentence_is_complete_and_every_label_is_not() {
        for sentence in SENTENCES
            .iter()
            .copied()
            .chain(formatted().iter().map(String::as_str))
        {
            assert!(
                sentence.ends_with('.') || sentence.ends_with('?'),
                "'{sentence}' must be a complete sentence"
            );
        }
        for label in LABELS {
            assert!(!label.is_empty());
            assert!(
                !label.ends_with('.'),
                "'{label}' is a label, not a sentence"
            );
        }
        assert!(STATUS_CHECKING.ends_with('…'));
        assert!(!held_by_text("Alice").ends_with('.'), "a caption");
    }

    #[test]
    fn plurals_read_right() {
        assert_eq!(pending_claims_text(1), "1 token has something to claim.");
        assert_eq!(pending_claims_text(2), "2 tokens have something to claim.");
        assert_eq!(hidden_count_text(1), "1 token is hidden.");
        assert_eq!(hidden_count_text(3), "3 tokens are hidden.");
        assert_eq!(tracked_count_text(8), "Tracking 8 tokens.");
        assert_eq!(more_actions_label("DCF"), "More actions for DCF");
    }

    /// Everyday-user copy. "Contract" is allowed only as the name of a
    /// copyable ID (README §5), never "contract owner".
    #[test]
    fn no_hub_copy_uses_token_jargon() {
        for text in SENTENCES
            .iter()
            .chain(LABELS.iter())
            .copied()
            .map(str::to_owned)
            .chain(formatted())
        {
            let lower = text.to_lowercase();
            for banned in [
                "transfer",
                "perpetual",
                "pre-programmed",
                "contract owner",
                "state transition",
                "nonce",
                "distribution",
            ] {
                assert!(!lower.contains(banned), "'{text}' uses '{banned}'");
            }
        }
    }
}
