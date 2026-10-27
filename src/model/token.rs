use super::validation::{TextLengthError, validate_char_count};

/// Minimum number of characters in a searchable contract keyword.
pub const MIN_CONTRACT_KEYWORD_CHARS: usize = 3;
/// Maximum number of characters in a searchable contract keyword.
pub const MAX_CONTRACT_KEYWORD_CHARS: usize = 50;

/// Validate a searchable data-contract keyword.
pub fn validate_contract_keyword(keyword: &str) -> Result<(), TextLengthError> {
    validate_char_count(
        keyword,
        MIN_CONTRACT_KEYWORD_CHARS,
        MAX_CONTRACT_KEYWORD_CHARS,
    )
}

/// Whether this application supports editing the requested token setting.
pub(crate) fn token_change_item_is_supported(
    item: &dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
) -> bool {
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem;
    !matches!(
        item,
        TokenConfigurationChangeItem::MinimumPoolNotesForOutgoing(_)
            | TokenConfigurationChangeItem::MinimumPoolNotesForOutgoingControlGroup(_)
            | TokenConfigurationChangeItem::MinimumPoolNotesForOutgoingAdminGroup(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_keyword_accepts_boundary_lengths() {
        assert!(validate_contract_keyword("abc").is_ok());
        assert!(validate_contract_keyword(&"k".repeat(50)).is_ok());
    }

    #[test]
    fn contract_keyword_rejects_outside_character_limits() {
        assert!(validate_contract_keyword("ab").is_err());
        assert!(validate_contract_keyword(&"k".repeat(51)).is_err());
        assert!(validate_contract_keyword(&"é".repeat(50)).is_ok());
    }
}
