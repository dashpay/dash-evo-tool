use std::fmt::Display;

use bincode::{Decode, Encode};
use dash_sdk::dpp::{balances::credits::TokenAmount, fee::Credits};
use serde::{Deserialize, Serialize};

pub const DUFF_DECIMAL_PLACES: u8 = 11;

#[derive(Serialize, Deserialize, Encode, Decode, Clone, PartialEq, Eq)]
pub struct Amount {
    value: TokenAmount,
    decimal_places: u8,
    unit_name: Option<String>,
}

impl Display for Amount {
    /// Formats the TokenValue as a user-friendly string with optional unit name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let amount_str = if self.decimal_places == 0 {
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
        };

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

    /// Creates a new Amount from a string input with specified decimal places and unit name.
    pub fn parse_with_decimals_and_unit(
        input: &str,
        decimal_places: u8,
        unit_name: String,
    ) -> Result<Self, String> {
        let value = Self::parse_amount_string(input, decimal_places)?;
        Ok(Self::new_with_unit(value, decimal_places, unit_name))
    }

    /// Parses a string amount into the internal u64 representation.
    pub fn parse_amount_string(input: &str, decimal_places: u8) -> Result<u64, String> {
        if decimal_places == 0 {
            return input
                .parse::<u64>()
                .map_err(|_| "Invalid amount: must be a whole number".to_string());
        }

        let parts: Vec<&str> = input.split('.').collect();
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
}

/// Dash-specific amount handling
impl Amount {
    /// Creates a new Dash amount
    pub fn dash(credits: Credits) -> Self {
        Self::new_with_unit(credits, DUFF_DECIMAL_PLACES, "DASH".to_string())
    }

    /// Creates a Dash amount from a duff string.
    pub fn parse_dash(input: &str) -> Result<Self, String> {
        Self::parse_with_decimals_and_unit(input, DUFF_DECIMAL_PLACES, "DASH".to_string())
    }

    /// Formats a duff amount as Dash.
    pub fn format_dash(duffs: u64) -> String {
        Self::format_amount(duffs, DUFF_DECIMAL_PLACES)
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

        // Test 8 decimal places (like Bitcoin)
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
    }

    #[test]
    fn test_dash_amounts() {
        // Test Dash formatting (8 decimal places)
        assert_eq!(Amount::format_dash(100_000_000_000_000), "1000");
        assert_eq!(Amount::format_dash(150_000_000_000), "1.5");
        assert_eq!(Amount::format_dash(12_345_678_901), "0.12345678901");

        // Test Dash parsing
        let dash_amount = Amount::parse_dash("1.5").unwrap();
        assert_eq!(dash_amount.value(), 150_000_000);
        assert_eq!(dash_amount.decimal_places(), DUFF_DECIMAL_PLACES);
    }

    #[test]
    fn test_amount_display() {
        let amount = Amount::new(12_345, 2);
        assert_eq!(format!("{}", amount), "123.45");

        let dash_amount = Amount::dash(150_000_000);
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

        // Test parsing with unit name
        let parsed =
            Amount::parse_with_decimals_and_unit("123.45", 2, "TOKEN".to_string()).unwrap();
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
