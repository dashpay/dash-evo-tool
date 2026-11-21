//! User-facing copy for the Identity Hub's Tokens tab. Kept free of egui so
//! the copy is unit-testable, and every string is one complete translation unit.
//!
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` frame T9.

use crate::model::amount::Amount;
use crate::model::datetime;

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

// Row actions and menu.
pub const MENU_LABEL: &str = "•••";
pub const CHECK_NOW_LABEL: &str = "Check balance now";
pub const COPY_ID_LABEL: &str = "Copy token ID";
pub const COPIED_ID: &str = "The token ID was copied to the clipboard.";
pub const STOP_TRACKING_LABEL: &str = "Stop tracking";
pub const STOP_TRACKING_TITLE: &str = "Stop tracking this token?";
pub const TRY_AGAIN_LABEL: &str = "Try again";
pub const DISMISS_LABEL: &str = "Dismiss";
pub const CANCEL_LABEL: &str = "Cancel";
pub const TRACK_AGAIN_LABEL: &str = "Track tokens again";

// Row badges and status lines.
pub const CANT_BE_SENT_BADGE: &str = "Can't be sent";
pub const ISSUED_BY_YOU: &str = "Issued by you";
pub const STATUS_NO_BALANCE_FOUND: &str = "No balance found.";
pub const STATUS_CHECKING: &str = "Checking balance…";
pub const STATUS_COULD_NOT_CHECK: &str = "Couldn't check this balance.";
/// The hub offers no Claim; the status names where to claim.
pub const STATUS_REWARDS: &str = "Rewards may be waiting for you in Tokens › My Tokens.";
pub const STATUS_ADD_KEY: &str = "Add a key to this identity to send or claim tokens.";

// Why a control is disabled.
pub const DISABLED_NOT_SYNCED: &str = "Available after sync finishes.";
pub const DISABLED_NOT_CONNECTED: &str = "Available after you connect to the network.";

// Empty, first-load and failure states.
pub const EMPTY_HEADING: &str = "No tokens yet";
pub const EMPTY_BODY: &str = "Tokens are assets issued on Dash Platform. A token shows here once you add it, so when someone sends you one, add it to see your balance.";
pub const DISMISSED_HEADING: &str = "Nothing tracked for this identity";
pub const FIRST_LOAD: &str =
    "Checking balances for the first time. This usually takes a few seconds.";
pub const FOOTNOTE: &str = "Select Refresh to check balances again. A token you don't see here can be added with Add a token.";

// Receive dialog.
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

/// "You can claim {amount} once in Tokens › My Tokens."
pub fn can_claim_once_text(amount: &Amount) -> String {
    format!("You can claim {amount} once in Tokens › My Tokens.")
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
        STATUS_COULD_NOT_CHECK,
        STATUS_NO_BALANCE_FOUND,
        STATUS_REWARDS,
        STATUS_ADD_KEY,
        DISABLED_NOT_SYNCED,
        DISABLED_NOT_CONNECTED,
        EMPTY_BODY,
        FIRST_LOAD,
        FOOTNOTE,
        RECEIVE_TOKENS_HINT,
        COPIED_ID,
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
        MENU_LABEL,
        CHECK_NOW_LABEL,
        COPY_ID_LABEL,
        STOP_TRACKING_LABEL,
        STOP_TRACKING_TITLE,
        TRY_AGAIN_LABEL,
        DISMISS_LABEL,
        CANCEL_LABEL,
        TRACK_AGAIN_LABEL,
        CANT_BE_SENT_BADGE,
        STATUS_CHECKING,
        EMPTY_HEADING,
        DISMISSED_HEADING,
        ISSUED_BY_YOU,
    ];

    fn formatted() -> Vec<String> {
        let amount = Amount::new(100, 2).with_unit_name("DCF");
        vec![
            last_updated_text(0),
            stale_since_text(0),
            can_claim_once_text(&amount),
            claimed_on_text("2026-10-20"),
            hidden_count_text(1),
            hidden_count_text(3),
            dismissed_body_text("Alice"),
            stop_tracking_question("DCF", "Alice"),
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
    }

    #[test]
    fn plurals_read_right() {
        assert_eq!(hidden_count_text(1), "1 token is hidden.");
        assert_eq!(hidden_count_text(3), "3 tokens are hidden.");
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
