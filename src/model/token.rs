use super::validation::{TextLengthError, validate_char_count};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v1::TokenDistributionRulesV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v1::TokenDistributionRulesV1;
use dash_sdk::dpp::data_contract::associated_token::token_once_per_identity_distribution::TokenOncePerIdentityDistribution;
use dash_sdk::dpp::data_contract::associated_token::token_once_per_identity_distribution::v0::TokenOncePerIdentityDistributionV0;

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

/// The largest amount a once-per-identity distribution may pay: a token
/// balance must fit in `i64`.
pub const MAX_ONCE_PER_IDENTITY_AMOUNT: TokenAmount = i64::MAX as TokenAmount;

/// Why a once-per-identity amount entered by the user is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OncePerIdentityAmountError {
    /// Not a whole number.
    #[error("Enter the amount each identity can claim as a whole number.")]
    NotANumber,
    /// Zero pays nothing.
    #[error("Enter an amount of at least 1 for each identity to claim.")]
    Zero,
    /// More than a token balance can hold.
    #[error("Enter an amount of at most {max} for each identity to claim.")]
    TooLarge { max: TokenAmount },
}

/// Parse the amount (in the token's smallest units) every identity may claim
/// once, as Platform bounds it: at least 1, at most
/// [`MAX_ONCE_PER_IDENTITY_AMOUNT`].
pub fn parse_once_per_identity_amount(
    input: &str,
) -> Result<TokenAmount, OncePerIdentityAmountError> {
    let amount: TokenAmount = input
        .trim()
        .parse()
        .map_err(|_| OncePerIdentityAmountError::NotANumber)?;
    match amount {
        0 => Err(OncePerIdentityAmountError::Zero),
        amount if amount > MAX_ONCE_PER_IDENTITY_AMOUNT => {
            Err(OncePerIdentityAmountError::TooLarge {
                max: MAX_ONCE_PER_IDENTITY_AMOUNT,
            })
        }
        amount => Ok(amount),
    }
}

/// The amount every identity may claim once from `config`, when the token has
/// a once-per-identity distribution (protocol version 14).
pub fn once_per_identity_amount(config: &TokenConfiguration) -> Option<TokenAmount> {
    use dash_sdk::dpp::data_contract::associated_token::token_once_per_identity_distribution::accessors::v0::TokenOncePerIdentityDistributionV0Methods;
    config
        .distribution_rules()
        .once_per_identity_distribution()
        .map(|distribution| distribution.amount())
}

/// The distribution rules for `rules` plus an optional once-per-identity
/// distribution of `once_per_identity_amount`.
///
/// Without one the rules stay version 0, the wire format every token used
/// before protocol version 14 and the only one older networks accept; with one
/// they become version 1.
pub fn distribution_rules_with_once_per_identity(
    rules: TokenDistributionRulesV0,
    once_per_identity_amount: Option<TokenAmount>,
) -> TokenDistributionRules {
    match once_per_identity_amount {
        None => TokenDistributionRules::V0(rules),
        Some(amount) => {
            let mut v1 = TokenDistributionRulesV1::from(rules);
            v1.once_per_identity_distribution = Some(TokenOncePerIdentityDistribution::V0(
                TokenOncePerIdentityDistributionV0 { amount },
            ));
            TokenDistributionRules::V1(v1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_per_identity_amount_is_bounded_like_a_token_balance() {
        assert_eq!(parse_once_per_identity_amount(" 25 "), Ok(25));
        assert_eq!(
            parse_once_per_identity_amount(&MAX_ONCE_PER_IDENTITY_AMOUNT.to_string()),
            Ok(MAX_ONCE_PER_IDENTITY_AMOUNT)
        );
        assert_eq!(
            parse_once_per_identity_amount("0"),
            Err(OncePerIdentityAmountError::Zero)
        );
        assert_eq!(
            parse_once_per_identity_amount(&(MAX_ONCE_PER_IDENTITY_AMOUNT + 1).to_string()),
            Err(OncePerIdentityAmountError::TooLarge {
                max: MAX_ONCE_PER_IDENTITY_AMOUNT
            })
        );
        assert_eq!(
            parse_once_per_identity_amount("1.5"),
            Err(OncePerIdentityAmountError::NotANumber)
        );
    }

    /// The rules of the most restrictive default token, which are version 0.
    fn base_rules() -> TokenDistributionRulesV0 {
        use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
        match TokenConfigurationV0::default_most_restrictive().distribution_rules {
            TokenDistributionRules::V0(v0) => v0,
            TokenDistributionRules::V1(_) => panic!("the default rules are version 0"),
        }
    }

    #[test]
    fn rules_without_a_once_per_identity_distribution_stay_version_0() {
        let rules = distribution_rules_with_once_per_identity(base_rules(), None);
        assert!(matches!(rules, TokenDistributionRules::V0(_)));
        assert_eq!(rules.once_per_identity_distribution(), None);
    }

    #[test]
    fn a_once_per_identity_distribution_makes_version_1_rules() {
        let base = base_rules();
        let rules = distribution_rules_with_once_per_identity(base.clone(), Some(40));
        let TokenDistributionRules::V1(v1) = &rules else {
            panic!("expected version 1 rules");
        };
        assert_eq!(v1.perpetual_distribution, base.perpetual_distribution);
        assert_eq!(
            v1.pre_programmed_distribution,
            base.pre_programmed_distribution
        );
        use dash_sdk::dpp::data_contract::associated_token::token_once_per_identity_distribution::accessors::v0::TokenOncePerIdentityDistributionV0Methods;
        assert_eq!(
            rules.once_per_identity_distribution().map(|d| d.amount()),
            Some(40)
        );
    }

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
