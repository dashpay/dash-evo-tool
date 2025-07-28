use std::fmt::Display;

use bincode::{Decode, Encode};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::{balances::credits::TokenAmount, fee::Credits};
use serde::{Deserialize, Serialize};

pub const DASH_DECIMAL_PLACES: u8 = 11;

#[derive(Serialize, Deserialize, Encode, Decode, Clone, PartialEq, Eq, Default)]
pub struct Amount {
    value: TokenAmount,
    decimal_places: u8,
    unit_name: Option<String>,
}

impl PartialOrd for Amount {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.value.cmp(&other.value))
    }
}

impl PartialEq<TokenAmount> for Amount {
    fn eq(&self, other: &TokenAmount) -> bool {
        self.value == *other
    }
}

impl PartialEq<TokenAmount> for &Amount {
    fn eq(&self, other: &TokenAmount) -> bool {
        self.value == *other
    }
}

impl Display for Amount {
    /// Formats the TokenValue as a user-friendly string with optional unit name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let amount_str = self.to_string_without_unit();

        match &self.unit_name {
            Some(unit) => write!(f, "{} {}", amount_str, unit),
            None => write!(f, "{}", amount_str),
        }
    }
}

impl Amount {
    /// Creates a new Amount.
    pub fn new(value: TokenAmount, decimal_places: u8) -> Self {
        Self {
            value,
            decimal_places,
            unit_name: None,
        }
    }

    /// Creates a new Amount with a unit name.
    pub fn new_with_unit(value: TokenAmount, decimal_places: u8, unit_name: String) -> Self {
        Self {
            value,
            decimal_places,
            unit_name: Some(unit_name),
        }
    }

    /// Creates a new Amount from a string input with specified decimal places.
    pub fn parse_with_decimals(input: &str, decimal_places: u8) -> Result<Self, String> {
        let value = Self::parse_amount_string(input, decimal_places)?;
        Ok(Self::new(value, decimal_places))
    }

    /// Parses a string amount into the internal u64 representation.
    /// Automatically strips any unit suffix from the input string.
    fn parse_amount_string(input: &str, decimal_places: u8) -> Result<u64, String> {
        // Strip any unit suffix by taking only the numeric part
        let numeric_part = input.split_whitespace().next().unwrap_or(input).trim();

        if decimal_places == 0 {
            return numeric_part
                .parse::<u64>()
                .map_err(|_| "Invalid amount: must be a whole number".to_string());
        }

        let parts: Vec<&str> = numeric_part.split('.').collect();
        match parts.len() {
            1 => {
                // No decimal point, parse as whole number
                let whole = parts[0]
                    .parse::<u64>()
                    .map_err(|_| "Invalid amount: must be a number".to_string())?;
                let multiplier = 10u64.pow(decimal_places as u32);
                whole
                    .checked_mul(multiplier)
                    .ok_or_else(|| "Amount too large".to_string())
            }
            2 => {
                // Has decimal point
                let whole = if parts[0].is_empty() {
                    0
                } else {
                    parts[0]
                        .parse::<u64>()
                        .map_err(|_| "Invalid amount: whole part must be a number".to_string())?
                };

                let fraction_str = parts[1];
                if fraction_str.len() > decimal_places as usize {
                    return Err(format!(
                        "Too many decimal places. Maximum allowed: {}",
                        decimal_places
                    ));
                }

                // Pad with zeros if needed
                let padded_fraction =
                    format!("{:0<width$}", fraction_str, width = decimal_places as usize);
                let fraction = padded_fraction
                    .parse::<u64>()
                    .map_err(|_| "Invalid amount: decimal part must be a number".to_string())?;

                let multiplier = 10u64.pow(decimal_places as u32);
                let whole_part = whole
                    .checked_mul(multiplier)
                    .ok_or_else(|| "Amount too large".to_string())?;

                whole_part
                    .checked_add(fraction)
                    .ok_or_else(|| "Amount too large".to_string())
            }
            _ => Err("Invalid amount: too many decimal points".to_string()),
        }
    }

    /// Formats an amount as a user-friendly string.
    pub fn format_amount(amount: u64, decimal_places: u8) -> String {
        if decimal_places == 0 {
            return amount.to_string();
        }

        let divisor = 10u64.pow(decimal_places as u32);
        let whole = amount / divisor;
        let fraction = amount % divisor;

        if fraction == 0 {
            whole.to_string()
        } else {
            // Format with the appropriate number of decimal places, removing trailing zeros
            let fraction_str = format!("{:0width$}", fraction, width = decimal_places as usize);
            let trimmed = fraction_str.trim_end_matches('0');
            format!("{}.{}", whole, trimmed)
        }
    }

    /// Converts the Amount to a f64 representation with the specified decimal places.
    ///
    /// Note this is a non-precise conversion, as f64 cannot represent all decimal values exactly.        
    pub fn to_f64(&self) -> f64 {
        (self.value as f64) / 10u64.pow(self.decimal_places as u32) as f64
    }

    /// Returns the internal TokenAmount representation.
    pub fn as_token_amount(&self) -> TokenAmount {
        self.value
    }

    /// Returns the number of decimal places.
    pub fn decimal_places(&self) -> u8 {
        self.decimal_places
    }

    /// Returns the value as the smallest unit (without decimal conversion).
    pub fn value(&self) -> u64 {
        self.value
    }

    /// Returns the unit name if set.
    pub fn unit_name(&self) -> Option<&str> {
        self.unit_name.as_deref()
    }

    /// Sets the unit name.
    pub fn with_unit_name(mut self, unit_name: String) -> Self {
        self.unit_name = Some(unit_name);
        self
    }

    /// Clears the unit name.
    pub fn without_unit_name(mut self) -> Self {
        self.unit_name = None;
        self
    }

    /// Returns the numeric string representation without the unit name.
    /// This is useful for text input fields where only the number should be shown.
    pub fn to_string_without_unit(&self) -> String {
        if self.decimal_places == 0 {
            self.value.to_string()
        } else {
            let divisor = 10u64.pow(self.decimal_places as u32);
            let whole = self.value / divisor;
            let fraction = self.value % divisor;

            if fraction == 0 {
                whole.to_string()
            } else {
                // Format with the appropriate number of decimal places, removing trailing zeros
                let fraction_str =
                    format!("{:0width$}", fraction, width = self.decimal_places as usize);
                let trimmed = fraction_str.trim_end_matches('0');
                format!("{}.{}", whole, trimmed)
            }
        }
    }

    /// Creates a new Amount with the specified value in TokenAmount.
    pub fn with_value(mut self, value: TokenAmount) -> Self {
        self.value = value;
        self
    }

    /// Updates the decimal places for this amount.
    /// This adjusts the internal value to maintain the same displayed amount.
    pub fn recalculate_decimal_places(mut self, new_decimal_places: u8) -> Self {
        if self.decimal_places != new_decimal_places {
            let current_decimals = self.decimal_places;

            if new_decimal_places > current_decimals {
                // More decimal places - multiply value
                let factor = 10u64.pow((new_decimal_places - current_decimals) as u32);
                self.value = self.value.saturating_mul(factor);
            } else if new_decimal_places < current_decimals {
                // Fewer decimal places - divide value
                let factor = 10u64.pow((current_decimals - new_decimal_places) as u32);
                self.value /= factor;
            }

            self.decimal_places = new_decimal_places;
        }
        self
    }

    /// Checks if the amount is for the same token as the other amount.
    ///
    /// This is determined by comparing the unit names and decimal places.
    pub fn is_same_token(&self, other: &Self) -> bool {
        self.unit_name == other.unit_name && self.decimal_places == other.decimal_places
    }
}

/// Dash-specific amount handling
impl Amount {
    /// Creates a new Dash amount
    pub fn dash(value: TokenAmount) -> Self {
        Self::new_with_unit(value, DASH_DECIMAL_PLACES, "DASH".to_string())
    }

    /// Creates a Dash amount from a duff string.
    pub fn parse_dash(input: &str) -> Result<Self, String> {
        Self::parse_with_decimals(input, DASH_DECIMAL_PLACES)
            .map(|amount| amount.with_unit_name("DASH".to_string()))
    }
}

impl AsRef<Amount> for Amount {
    /// Returns a reference to the Amount.
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Conversion implementations for token types
impl From<&crate::ui::tokens::tokens_screen::IdentityTokenBalance> for Amount {
    /// Converts an IdentityTokenBalance to an Amount.
    ///
    /// The decimal places are automatically determined from the token configuration,
    /// and the token alias is used as the unit name.
    fn from(token_balance: &crate::ui::tokens::tokens_screen::IdentityTokenBalance) -> Self {
        let decimal_places = token_balance.token_config.conventions().decimals();
        Self::new_with_unit(
            token_balance.balance,
            decimal_places,
            token_balance.token_alias.clone(),
        )
    }
}

impl From<crate::ui::tokens::tokens_screen::IdentityTokenBalance> for Amount {
    /// Converts an owned IdentityTokenBalance to an Amount.
    fn from(token_balance: crate::ui::tokens::tokens_screen::IdentityTokenBalance) -> Self {
        Self::from(&token_balance)
    }
}

impl From<&crate::ui::tokens::tokens_screen::IdentityTokenBalanceWithActions> for Amount {
    /// Converts an IdentityTokenBalanceWithActions to an Amount.
    ///
    /// The decimal places are automatically determined from the token configuration,
    /// and the token alias is used as the unit name.
    fn from(
        token_balance: &crate::ui::tokens::tokens_screen::IdentityTokenBalanceWithActions,
    ) -> Self {
        let decimal_places = token_balance.token_config.conventions().decimals();
        Self::new_with_unit(
            token_balance.balance,
            decimal_places,
            token_balance.token_alias.clone(),
        )
    }
}

impl From<crate::ui::tokens::tokens_screen::IdentityTokenBalanceWithActions> for Amount {
    /// Converts an owned IdentityTokenBalanceWithActions to an Amount.
    fn from(
        token_balance: crate::ui::tokens::tokens_screen::IdentityTokenBalanceWithActions,
    ) -> Self {
        Self::from(&token_balance)
    }
}

impl From<Amount> for Credits {
    /// Converts an Amount to Credits.
    ///
    /// This is useful for passing amounts to functions that expect Credits.
    fn from(amount: Amount) -> Self {
        amount.value
    }
}

impl From<&Amount> for Credits {
    /// Converts an Amount to Credits.
    ///
    /// This is useful for passing amounts to functions that expect Credits.
    fn from(amount: &Amount) -> Self {
        amount.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_amount_formatting() {
        // Test 0 decimal places
        assert_eq!(Amount::format_amount(100, 0), "100");

        // Test 2 decimal places
        assert_eq!(Amount::format_amount(12345, 2), "123.45");
        assert_eq!(Amount::format_amount(12300, 2), "123");
        assert_eq!(Amount::format_amount(12340, 2), "123.4");

        // Test 8 decimal places
        assert_eq!(Amount::format_amount(100_000_000, 8), "1");
        assert_eq!(Amount::format_amount(150_000_000, 8), "1.5");
        assert_eq!(Amount::format_amount(123_456_789, 8), "1.23456789");
    }

    #[test]
    fn test_token_amount_parsing() {
        // Test 0 decimal places
        assert_eq!(Amount::parse_amount_string("100", 0).unwrap(), 100);

        // Test 2 decimal places
        assert_eq!(Amount::parse_amount_string("123.45", 2).unwrap(), 12345);
        assert_eq!(Amount::parse_amount_string("123", 2).unwrap(), 12300);
        assert_eq!(Amount::parse_amount_string("123.4", 2).unwrap(), 12340);

        // Test 8 decimal places
        assert_eq!(Amount::parse_amount_string("1", 8).unwrap(), 100000000);
        assert_eq!(Amount::parse_amount_string("1.5", 8).unwrap(), 150000000);
        assert_eq!(
            Amount::parse_amount_string("1.23456789", 8).unwrap(),
            123456789
        );

        assert_eq!(
            Amount::parse_amount_string("1.5 DASH", 8).unwrap(),
            150000000
        );

        // Test parsing amounts with unit suffixes
        assert_eq!(Amount::parse_amount_string("123.45 USD", 2).unwrap(), 12345);
        assert_eq!(
            Amount::parse_amount_string("1.0 BTC", 8).unwrap(),
            100000000
        );
        assert_eq!(Amount::parse_amount_string("50 TOKEN", 0).unwrap(), 50);
    }

    #[test]
    fn test_dash_amounts() {
        // Test Dash parsing
        let dash_amount = Amount::parse_dash("1.5").unwrap();
        assert_eq!(dash_amount.value(), 150_000_000_000);
        assert_eq!(dash_amount.decimal_places(), DASH_DECIMAL_PLACES);
        assert_eq!(dash_amount.unit_name(), Some("DASH"));

        // Test Dash parsing with unit suffix
        let dash_amount_with_unit = Amount::parse_dash("1.5 DASH").unwrap();
        assert_eq!(dash_amount_with_unit.value(), 150_000_000_000);
        assert_eq!(dash_amount_with_unit.decimal_places(), DASH_DECIMAL_PLACES);
        assert_eq!(dash_amount_with_unit.unit_name(), Some("DASH"));
    }

    #[test]
    fn test_amount_display() {
        let amount = Amount::new(12_345, 2);
        assert_eq!(format!("{}", amount), "123.45");

        let dash_amount = Amount::dash(150_000_000_000);
        assert_eq!(format!("{}", dash_amount), "1.5 DASH");

        // Test amount with custom unit name
        let amount_with_unit = Amount::new_with_unit(54321, 2, "USD".to_string());
        assert_eq!(format!("{}", amount_with_unit), "543.21 USD");
    }

    #[test]
    fn test_unit_name_functionality() {
        // Test creating amount with unit name
        let amount = Amount::new_with_unit(12345, 2, "USD".to_string());
        assert_eq!(amount.unit_name(), Some("USD"));
        assert_eq!(amount.value(), 12345);
        assert_eq!(amount.decimal_places(), 2);
        assert_eq!(format!("{}", amount), "123.45 USD");

        // Test adding unit name to existing amount
        let amount = Amount::new(54321, 8).with_unit_name("BTC".to_string());
        assert_eq!(amount.unit_name(), Some("BTC"));

        // Test removing unit name
        let amount = amount.without_unit_name();
        assert_eq!(amount.unit_name(), None);

        // Test Dash amounts include unit name
        let dash_amount = Amount::dash(100_000_000);
        assert_eq!(dash_amount.unit_name(), Some("DASH"));

        // Test parsing with_unit_name
        let parsed = Amount::parse_with_decimals("123.45", 2)
            .unwrap()
            .with_unit_name("TOKEN".to_string());
        assert_eq!(parsed.unit_name(), Some("TOKEN"));
        assert_eq!(parsed.value(), 12345);
    }

    #[test]
    fn test_parsing_errors() {
        // Too many decimal places
        assert!(Amount::parse_amount_string("1.123", 2).is_err());

        // Invalid characters
        assert!(Amount::parse_amount_string("abc", 2).is_err());

        // Multiple decimal points
        assert!(Amount::parse_amount_string("1.2.3", 2).is_err());
    }

    #[test]
    fn test_simplified_parsing_with_units() {
        // Test the new simplified API pattern: parse + with_unit_name
        let token_amount = Amount::parse_with_decimals("123.45 TOKEN", 2).unwrap();
        assert_eq!(token_amount.value(), 12345);
        assert_eq!(token_amount.unit_name(), Some("TOKEN"));
        assert_eq!(format!("{}", token_amount), "123.45 TOKEN");

        // Test that unit in input string is ignored, only the numeric part is parsed
        let btc_amount = Amount::parse_with_decimals("0.5 BTC", 8)
            .unwrap()
            .with_unit_name("BTC".to_string());
        assert_eq!(btc_amount.value(), 50000000);
        assert_eq!(btc_amount.unit_name(), Some("BTC"));

        // Test parsing without unit in string, then adding unit
        let dash_amount = Amount::parse_with_decimals("1.5", 11)
            .unwrap()
            .with_unit_name("DASH".to_string());
        assert_eq!(dash_amount.value(), 150_000_000_000);
        assert_eq!(dash_amount.unit_name(), Some("DASH"));
    }

    #[test]
    fn test_to_string_without_unit() {
        // Test amount without unit
        let amount = Amount::new(12345, 2);
        assert_eq!(amount.to_string_without_unit(), "123.45");
        assert_eq!(format!("{}", amount), "123.45"); // Display should be the same

        // Test amount with unit
        let amount_with_unit = Amount::new_with_unit(12345, 2, "USD".to_string());
        assert_eq!(amount_with_unit.to_string_without_unit(), "123.45"); // Without unit
        assert_eq!(format!("{}", amount_with_unit), "123.45 USD"); // Display includes unit

        // Test Dash amount
        let dash_amount = Amount::dash(150_000_000_000); // 1.5 DASH
        assert_eq!(dash_amount.to_string_without_unit(), "1.5");
        assert_eq!(format!("{}", dash_amount), "1.5 DASH");

        // Test zero amount
        let zero_amount = Amount::new(0, 8);
        assert_eq!(zero_amount.to_string_without_unit(), "0");
    }

    #[test]
    fn test_decimal_places_conversion() {
        // Test converting from 2 decimal places to 8 decimal places
        let amount = Amount::new(12345, 2); // 123.45
        let converted = amount.recalculate_decimal_places(8);
        assert_eq!(converted.value(), 12345000000); // 123.45 with 8 decimals
        assert_eq!(converted.decimal_places(), 8);
        assert_eq!(format!("{}", converted), "123.45");

        // Test converting from 8 decimal places to 2 decimal places
        let amount = Amount::new(12345000000, 8); // 123.45
        let converted = amount.recalculate_decimal_places(2);
        assert_eq!(converted.value(), 12345); // 123.45 with 2 decimals
        assert_eq!(converted.decimal_places(), 2);
        assert_eq!(format!("{}", converted), "123.45");

        // Test no conversion (same decimal places)
        let amount = Amount::new(12345, 2);
        let same = amount.clone().recalculate_decimal_places(2);
        assert_eq!(same.value(), 12345);
        assert_eq!(same.decimal_places(), 2);
    }
}
