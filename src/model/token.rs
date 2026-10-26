use super::validation::{TextLengthError, validate_char_count};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v1::TokenDistributionRulesV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v1::TokenDistributionRulesV1;
use dash_sdk::dpp::data_contract::associated_token::token_once_per_identity_distribution::{
    TokenOncePerIdentityDistribution, accessors::v0::TokenOncePerIdentityDistributionV0Methods,
    v0::TokenOncePerIdentityDistributionV0,
};
use dash_sdk::platform::Identifier;

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

/// Why an identity held by no wallet in this app cannot send token
/// transactions, and what to do about it.
pub const TOKEN_ACTIONS_NEED_WALLET: &str = "This identity is not part of a wallet in this app, \
     so it cannot send token transactions. Add or import the wallet that holds this identity, \
     then try again.";

/// Why token actions wait for the identity's wallet, and what to do about it.
pub const TOKEN_ACTIONS_WAIT_FOR_WALLET: &str = "The wallet that holds this identity is not \
     open yet, so it cannot send token transactions. Unlock the wallet or wait for it to load, \
     then try again.";

/// Whether an identity can send token transactions from this app. Token
/// transactions go through the wallet that holds the identity, so an identity
/// held by no wallet can only view its token data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenActionAvailability {
    Available,
    /// No wallet in this app holds the identity.
    NeedsWallet,
    /// The identity's wallet is known but not loaded (for example, locked).
    WalletNotOpen,
}

impl TokenActionAvailability {
    /// Availability for an identity whose wallet link is `linked_wallet_open`:
    /// `None` when no wallet holds it, otherwise whether that wallet is loaded.
    pub fn from_wallet_link(linked_wallet_open: Option<bool>) -> Self {
        match linked_wallet_open {
            None => Self::NeedsWallet,
            Some(false) => Self::WalletNotOpen,
            Some(true) => Self::Available,
        }
    }

    pub fn is_available(self) -> bool {
        self == Self::Available
    }

    /// The explanation shown next to disabled token actions.
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::Available => None,
            Self::NeedsWallet => Some(TOKEN_ACTIONS_NEED_WALLET),
            Self::WalletNotOpen => Some(TOKEN_ACTIONS_WAIT_FOR_WALLET),
        }
    }
}

/// Who receives newly minted tokens under a token's distribution rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintRecipient {
    /// The identity the minter chose; the rules allow choosing.
    Chosen(Identifier),
    /// The token's configured destination for new tokens.
    ContractDestination(Identifier),
    /// No identity was chosen (or choosing is not allowed) and the token
    /// configures no destination, so Platform refuses the mint.
    Missing,
}

impl MintRecipient {
    /// The recipient of a mint under `rules` when the minter chose `chosen`.
    /// A choice the rules do not allow is ignored, as Platform ignores it.
    pub fn resolve(rules: &TokenDistributionRules, chosen: Option<Identifier>) -> Self {
        let chosen = chosen.filter(|_| rules.minting_allow_choosing_destination());
        match (chosen, rules.new_tokens_destination_identity()) {
            (Some(id), _) => Self::Chosen(id),
            (None, Some(destination)) => Self::ContractDestination(*destination),
            (None, None) => Self::Missing,
        }
    }

    /// The recipient a mint transition carries: only a chosen one, since
    /// Platform applies the token's destination itself.
    pub fn transition_recipient(self) -> Option<Identifier> {
        match self {
            Self::Chosen(id) => Some(id),
            Self::ContractDestination(_) | Self::Missing => None,
        }
    }

    /// The identity that receives the tokens, when there is one.
    pub fn identity(self) -> Option<Identifier> {
        match self {
            Self::Chosen(id) | Self::ContractDestination(id) => Some(id),
            Self::Missing => None,
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn mint_recipient_follows_the_tokens_destination_rules() {
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Setters;
        let chosen = Identifier::new([1; 32]);
        let destination = Identifier::new([2; 32]);
        let rules = |allow_choosing: bool, destination: Option<Identifier>| {
            let mut rules = dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0::default_most_restrictive()
                .distribution_rules;
            rules.set_minting_allow_choosing_destination(allow_choosing);
            rules.set_new_tokens_destination_identity(destination);
            rules
        };

        for (allow, dest, pick, expected) in [
            (true, None, Some(chosen), MintRecipient::Chosen(chosen)),
            (
                true,
                Some(destination),
                Some(chosen),
                MintRecipient::Chosen(chosen),
            ),
            (
                true,
                Some(destination),
                None,
                MintRecipient::ContractDestination(destination),
            ),
            (true, None, None, MintRecipient::Missing),
            (
                false,
                Some(destination),
                Some(chosen),
                MintRecipient::ContractDestination(destination),
            ),
            (false, None, Some(chosen), MintRecipient::Missing),
        ] {
            assert_eq!(
                MintRecipient::resolve(&rules(allow, dest), pick),
                expected,
                "allow={allow} destination={dest:?} chosen={pick:?}"
            );
        }

        assert_eq!(
            MintRecipient::Chosen(chosen).transition_recipient(),
            Some(chosen)
        );
        assert_eq!(
            MintRecipient::ContractDestination(destination).transition_recipient(),
            None,
            "the transition leaves the destination to Platform"
        );
        assert_eq!(
            MintRecipient::ContractDestination(destination).identity(),
            Some(destination)
        );
        assert_eq!(MintRecipient::Missing.identity(), None);
    }

    #[test]
    fn token_actions_need_the_identitys_open_wallet() {
        use TokenActionAvailability::*;
        assert_eq!(TokenActionAvailability::from_wallet_link(None), NeedsWallet);
        assert_eq!(
            TokenActionAvailability::from_wallet_link(Some(false)),
            WalletNotOpen
        );
        assert_eq!(
            TokenActionAvailability::from_wallet_link(Some(true)),
            Available
        );

        assert!(Available.is_available());
        assert_eq!(Available.unavailable_reason(), None);
        for blocked in [NeedsWallet, WalletNotOpen] {
            assert!(!blocked.is_available());
            let reason = blocked.unavailable_reason().expect("a reason");
            assert!(reason.ends_with("then try again."), "{reason}");
        }
        assert_ne!(
            NeedsWallet.unavailable_reason(),
            WalletNotOpen.unavailable_reason()
        );
    }
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
