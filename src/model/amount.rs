use std::fmt::{Debug, Display};

use bincode::{Decode, Encode};
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Duffs, TokenAmount};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use serde::{Deserialize, Serialize};

/// 1 dash == 10e11 credits
pub const DASH_DECIMAL_PLACES: u8 = 11;

/// 1 dash = 1e8 duffs, 1 duff = 1e3 credits; see [CREDITS_PER_DUFF]
const DUFF_DECIMAL_PLACES: i32 = DASH_DECIMAL_PLACES as i32 - 3;

#[derive(Serialize, Deserialize, Encode, Decode, Clone, PartialEq, Eq, Default)]
pub struct Amount {
    /// Number of tokens (or [Credits] in case of DASH).
    /// Note these are NOT duffs, see [CREDITS_PER_DUFF].
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

impl Debug for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Amount")
            .field("value", &self.value)
            .field("decimal_places", &self.decimal_places)
            .field("unit_name", &self.unit_name)
            .field("formatted", &self.to_string_without_unit())
            .finish()
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

    /// Creates a new Amount configured for a specific token.
    ///
    /// This extracts the decimal places and token alias from the token configuration
    /// and creates an Amount with the specified value.
    pub fn new_for_token(
        value: TokenAmount,
        token_info: &crate::ui::tokens::tokens_screen::IdentityTokenInfo,
    ) -> Self {
        let decimal_places = token_info.token_config.conventions().decimals();
        Self::new_with_unit(value, decimal_places, token_info.token_alias.clone())
    }

    /// Creates a new Amount from a string input with specified decimal places.
    /// If the input string contains a unit suffix (e.g., "123.45 USD"), the unit name will be preserved.
    pub fn parse_with_decimals(input: &str, decimal_places: u8) -> Result<Self, String> {
        let (value, unit_name) = Self::parse_amount_string_with_unit(input, decimal_places)?;
        match unit_name {
            Some(unit) => Ok(Self::new_with_unit(value, decimal_places, unit)),
            None => Ok(Self::new(value, decimal_places)),
        }
    }

    /// Parses a string amount into the internal u64 representation.
    /// Returns a tuple of (value, optional_unit_name).
    /// Automatically extracts any unit suffix from the input string.
    fn parse_amount_string_with_unit(
        input: &str,
        decimal_places: u8,
    ) -> Result<(u64, Option<String>), String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("Invalid amount: cannot be empty".to_string());
        }

        // Split by whitespace to separate numeric part from potential unit
        let parts: Vec<&str> = input.split_whitespace().collect();
        let numeric_part = parts.first().unwrap_or(&input);
        let unit_name = if parts.len() > 1 {
            Some(parts[1..].join(" ")) // Join remaining parts as unit name (handles multi-word units)
        } else {
            None
        };

        let value = Self::parse_numeric_part(numeric_part, decimal_places)?;
        Ok((value, unit_name))
    }

    /// Parses a string amount into the internal u64 representation.
    /// Automatically strips any unit suffix from the input string.
    /// This method is kept for backward compatibility.
    #[cfg(test)]
    fn parse_amount_string(input: &str, decimal_places: u8) -> Result<u64, String> {
        let (value, _) = Self::parse_amount_string_with_unit(input, decimal_places)?;
        Ok(value)
    }

    /// Parses the numeric part of an amount string.
    fn parse_numeric_part(numeric_part: &str, decimal_places: u8) -> Result<u64, String> {
        if decimal_places == 0 {
            return numeric_part
                .parse::<u64>()
                .map_err(|e| format!("Invalid amount: {}", e));
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
    /// Creates a new Dash amount.
    ///
    /// Note: Due to use of float, this may not be precise.
    pub fn dash(value: f64) -> Self {
        let credits = value * 10f64.powi(DASH_DECIMAL_PLACES as i32);
        Self::new_with_unit(
            checked_round(credits).expect("DASH value overflow"),
            DASH_DECIMAL_PLACES,
            "DASH".to_string(),
        )
    }

    pub fn duffs(value: Duffs) -> Self {
        let dash = value as f64 / 10f64.powi(DUFF_DECIMAL_PLACES);
        Self::dash(dash)
    }

    /// Creates a Dash amount from a duff string.
    pub fn parse_dash(input: &str) -> Result<Self, String> {
        Self::parse_with_decimals(input, DASH_DECIMAL_PLACES)
            .map(|amount| amount.with_unit_name("DASH".to_string()))
    }

    /// Returns the DASH amount as duffs, rounded down to the nearest integer.
    ///
    /// ## Panics
    ///
    /// Panics if the token is not DASH, eg. decimals != DASH_DECIMAL_PLACES or token name is neither `DASH` nor empty.
    pub fn to_duffs(&self) -> Duffs {
        if self.unit_name.as_ref().is_some_and(|name| name != "DASH") {
            panic!("Amount is not in DASH");
        }
        if self.decimal_places != DASH_DECIMAL_PLACES {
            panic!("Amount is not in DASH, decimal places mismatch");
        }

        self.value
            .checked_div(CREDITS_PER_DUFF)
            .expect("Division by zero in DASH to duffs conversion")
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

/// Helper function to convert f64 to u64, with checks for overflow.
/// It rounds the value to the nearest u64, ensuring it is within bounds.
fn checked_round(value: f64) -> Result<u64, String> {
    let rounded = value.round();
    if rounded < u64::MIN as f64 || rounded > u64::MAX as f64 {
        return Err("Overflow: value outside of bounds".to_string());
    }

    Ok(rounded as u64)
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
    fn test_dash_method() {
        // Test creating DASH amounts with different values
        let zero_dash = Amount::dash(0.0);
        assert_eq!(zero_dash.value(), 0);
        assert_eq!(zero_dash.decimal_places(), DASH_DECIMAL_PLACES);
        assert_eq!(zero_dash.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", zero_dash), "0 DASH");

        let one_dash = Amount::dash(1.0);
        assert_eq!(one_dash.value(), 100_000_000_000); // 1 DASH = 10^11 credits
        assert_eq!(one_dash.decimal_places(), DASH_DECIMAL_PLACES);
        assert_eq!(one_dash.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", one_dash), "1 DASH");

        let fractional_dash = Amount::dash(0.12345678901);
        assert_eq!(fractional_dash.value(), 12_345_678_901); // 0.12345678901 DASH
        assert_eq!(format!("{}", fractional_dash), "0.12345678901 DASH");

        // Test edge case: very small amount
        let tiny_dash = Amount::dash(0.00000000001); // 1 credit
        assert_eq!(tiny_dash.value(), 1);
        assert_eq!(format!("{}", tiny_dash), "0.00000000001 DASH");
    }

    #[test]
    fn test_duffs_method() {
        // Test creating DASH amounts from duffs
        // 1 DASH = 100,000,000 duffs = 10^8 duffs
        // 1 duff = 1000 credits (CREDITS_PER_DUFF)
        // So 1 DASH = 10^8 * 10^3 = 10^11 credits

        let zero_duffs = Amount::duffs(0);
        assert_eq!(zero_duffs.value(), 0);
        assert_eq!(zero_duffs.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", zero_duffs), "0 DASH");

        let one_duff = Amount::duffs(1);
        assert_eq!(one_duff.value(), 1000); // 1 duff = 1000 credits
        assert_eq!(one_duff.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", one_duff), "0.00000001 DASH");

        let hundred_million_duffs = Amount::duffs(100_000_000); // 1 DASH
        assert_eq!(hundred_million_duffs.value(), 100_000_000_000);
        assert_eq!(format!("{}", hundred_million_duffs), "1 DASH");

        let one_and_half_dash_in_duffs = Amount::duffs(150_000_000); // 1.5 DASH
        assert_eq!(one_and_half_dash_in_duffs.value(), 150_000_000_000);
        assert_eq!(format!("{}", one_and_half_dash_in_duffs), "1.5 DASH");
    }

    #[test]
    fn test_to_duffs_method() {
        // Test converting DASH amounts back to duffs
        let one_dash = Amount::dash(1.0);
        assert_eq!(one_dash.to_duffs(), 100_000_000); // 1 DASH = 10^8 duffs

        let half_dash = Amount::dash(0.5);
        assert_eq!(half_dash.to_duffs(), 50_000_000); // 0.5 DASH = 5*10^7 duffs

        let one_and_half_dash = Amount::dash(1.5);
        assert_eq!(one_and_half_dash.to_duffs(), 150_000_000); // 1.5 DASH = 1.5*10^8 duffs

        // Test with very small amounts
        let one_credit = Amount::new_with_unit(1, DASH_DECIMAL_PLACES, "DASH".to_string());
        assert_eq!(one_credit.to_duffs(), 0); // 1 credit = 0 duffs (rounded down)

        let thousand_credits = Amount::new_with_unit(1000, DASH_DECIMAL_PLACES, "DASH".to_string());
        assert_eq!(thousand_credits.to_duffs(), 1); // 1000 credits = 1 duff

        // Test with amount without unit name (should work)
        let dash_no_unit = Amount::new(100_000_000_000, DASH_DECIMAL_PLACES);
        assert_eq!(dash_no_unit.to_duffs(), 100_000_000);
    }

    #[test]
    #[should_panic(expected = "Amount is not in DASH")]
    fn test_to_duffs_panics_with_wrong_unit() {
        let btc_amount = Amount::new_with_unit(100_000_000, 8, "BTC".to_string());
        btc_amount.to_duffs(); // Should panic
    }

    #[test]
    #[should_panic(expected = "Amount is not in DASH, decimal places mismatch")]
    fn test_to_duffs_panics_with_wrong_decimals() {
        let wrong_decimals = Amount::new_with_unit(100_000_000, 8, "DASH".to_string());
        wrong_decimals.to_duffs(); // Should panic
    }

    #[test]
    fn test_dash_duffs_roundtrip() {
        // Test that duffs -> DASH -> duffs preserves the value
        let original_duffs = 123_456_789u64;
        let dash_amount = Amount::duffs(original_duffs);
        let converted_back = dash_amount.to_duffs();
        assert_eq!(original_duffs, converted_back);

        // Test edge cases
        let zero_duffs = 0u64;
        let zero_dash = Amount::duffs(zero_duffs);
        assert_eq!(zero_duffs, zero_dash.to_duffs());

        let max_reasonable_duffs = 2_100_000_000_000_000u64; // 21M DASH * 10^8
        let max_dash = Amount::duffs(max_reasonable_duffs);
        assert_eq!(max_reasonable_duffs, max_dash.to_duffs());
        assert_eq!(max_reasonable_duffs * CREDITS_PER_DUFF, max_dash.value());
        assert_eq!(21_000_000.0, max_dash.to_f64());
    }

    #[test]
    fn test_dash_precision() {
        // Test that the dash() method handles precision correctly
        // Note: Due to f64 limitations, very precise decimals might have rounding issues

        // Test values that should be exact in f64
        let exact_values = [0.0, 0.5, 1.0, 1.5, 2.0, 10.0, 100.0];
        for &value in &exact_values {
            let dash_amount = Amount::dash(value);
            let expected_credits = (value * 100_000_000_000.0).round() as u64;
            assert_eq!(dash_amount.value(), expected_credits);
        }

        // Test a value with 11 decimal places (max precision for DASH)
        let precise_dash = Amount::dash(1.23456789012); // This might lose precision due to f64
        // We mainly test that it doesn't panic and creates a valid amount
        assert!(precise_dash.value() > 0);
        assert_eq!(precise_dash.unit_name(), Some("DASH"));
    }

    #[test]
    fn test_amount_display() {
        let amount = Amount::new(12_345, 2);
        assert_eq!(format!("{}", amount), "123.45");

        let dash_amount = Amount::dash(1.5);
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
        let dash_amount = Amount::dash(1.0);
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
        // Test the simplified API pattern: parse_with_decimals now preserves unit names automatically
        let token_amount = Amount::parse_with_decimals("123.45 TOKEN", 2).unwrap();
        assert_eq!(token_amount.value(), 12345);
        assert_eq!(token_amount.unit_name(), Some("TOKEN"));
        assert_eq!(format!("{}", token_amount), "123.45 TOKEN");

        // Test parsing with unit suffix automatically preserves the unit
        let btc_amount = Amount::parse_with_decimals("0.5 BTC", 8).unwrap();
        assert_eq!(btc_amount.value(), 50000000);
        assert_eq!(btc_amount.unit_name(), Some("BTC"));
        assert_eq!(format!("{}", btc_amount), "0.5 BTC");

        // Test parsing without unit in string results in no unit name
        let no_unit_amount = Amount::parse_with_decimals("1.5", 11).unwrap();
        assert_eq!(no_unit_amount.value(), 150_000_000_000);
        assert_eq!(no_unit_amount.unit_name(), None);
        assert_eq!(format!("{}", no_unit_amount), "1.5");

        // Test adding unit name manually when not present in string
        let dash_amount = Amount::parse_with_decimals("1.5", 11)
            .unwrap()
            .with_unit_name("DASH".to_string());
        assert_eq!(dash_amount.value(), 150_000_000_000);
        assert_eq!(dash_amount.unit_name(), Some("DASH"));
        assert_eq!(format!("{}", dash_amount), "1.5 DASH");

        // Test multi-word unit names
        let multi_word_unit = Amount::parse_with_decimals("100 US Dollar", 2).unwrap();
        assert_eq!(multi_word_unit.value(), 10000);
        assert_eq!(multi_word_unit.unit_name(), Some("US Dollar"));
        assert_eq!(format!("{}", multi_word_unit), "100 US Dollar");
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
        let dash_amount = Amount::dash(1.5); // 1.5 DASH
        assert_eq!(dash_amount.to_string_without_unit(), "1.5");
        assert_eq!(format!("{}", dash_amount), "1.5 DASH");
        assert_eq!(dash_amount.to_duffs(), 150_000_000); // 1.5 DASH in duffs

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
