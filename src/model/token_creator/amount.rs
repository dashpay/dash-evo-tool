//! Exact editable token amounts. Invalid text is retained until corrected.

use crate::model::amount::Amount;

/// A token amount as entered, with an explicit representation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenAmountInput {
    pub text: String,
    pub base_units: bool,
}

/// A reason an amount cannot be represented without rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AmountError {
    #[error("Enter a token amount using digits and a decimal point.")]
    Invalid,
    #[error("Enter an amount with at most {decimals} decimal places.")]
    Precision { decimals: u8 },
    #[error("This amount is too large. Reduce the amount or the decimal precision.")]
    Overflow,
    #[error("Choose a decimal precision from 0 to 16.")]
    Decimals,
}

impl TokenAmountInput {
    /// Creates an amount in displayed tokens.
    pub fn tokens(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            base_units: false,
        }
    }

    /// Parses without rounding, including at the u64 boundary.
    pub fn parse(&self, decimals: u8) -> Result<u64, AmountError> {
        parse_amount(&self.text, if self.base_units { 0 } else { decimals })
    }

    /// Changes representation only when the current amount is valid.
    pub fn change_units(&mut self, base_units: bool, decimals: u8) -> Result<(), AmountError> {
        if self.base_units != base_units {
            let value = self.parse(decimals)?;
            self.text = if base_units {
                value.to_string()
            } else {
                format_amount(value, decimals)
            };
            self.base_units = base_units;
        }
        Ok(())
    }

    /// Preserves displayed tokens when precision changes, including in raw-unit mode.
    pub fn change_precision(&mut self, old: u8, new: u8) -> Result<(), AmountError> {
        if self.text.trim().is_empty() {
            return Ok(());
        }
        if self.base_units {
            let shown = format_amount(self.parse(old)?, old);
            match parse_amount(&shown, new) {
                Ok(raw) => self.text = raw.to_string(),
                Err(error) => {
                    // Retain the exact token value for correction at the new precision.
                    self.text = shown;
                    self.base_units = false;
                    return Err(error);
                }
            }
        } else {
            self.parse(new)?;
        }
        Ok(())
    }
}

/// Parses one nonnegative decimal amount with checked integer arithmetic.
pub fn parse_amount(input: &str, decimals: u8) -> Result<u64, AmountError> {
    if decimals > 16 {
        return Err(AmountError::Decimals);
    }
    let text = input.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || text.ends_with('.')
    {
        return Err(AmountError::Invalid);
    }
    if fraction.len() > usize::from(decimals) {
        return Err(AmountError::Precision { decimals });
    }
    let factor = 10u64.pow(u32::from(decimals));
    let whole = whole.parse::<u64>().map_err(|_| AmountError::Overflow)?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| AmountError::Overflow)?
            .checked_mul(10u64.pow(u32::from(decimals) - fraction.len() as u32))
            .ok_or(AmountError::Overflow)?
    };
    whole
        .checked_mul(factor)
        .and_then(|value| value.checked_add(fractional))
        .ok_or(AmountError::Overflow)
}

/// Formats raw units as tokens without floating point.
pub fn format_amount(value: u64, decimals: u8) -> String {
    Amount::new(value, decimals).to_string_opts(false, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_round_trips_u64_boundary_without_floats() {
        for decimals in 0..=16 {
            assert_eq!(
                parse_amount(&format_amount(u64::MAX, decimals), decimals),
                Ok(u64::MAX)
            );
        }
        assert_eq!(
            parse_amount("184467440737095516.16", 2),
            Err(AmountError::Overflow)
        );
    }

    #[test]
    fn precision_preserves_displayed_tokens_and_rejects_rounding() {
        let mut amount = TokenAmountInput::tokens("1000");
        amount.change_units(true, 2).unwrap();
        assert_eq!(amount.text, "100000");
        amount.change_precision(2, 3).unwrap();
        assert_eq!(amount.text, "1000000");
        amount.change_units(false, 3).unwrap();
        assert_eq!(amount.text, "1000");
        let mut fractional = TokenAmountInput::tokens("1.23");
        assert!(fractional.change_precision(2, 1).is_err());
        assert_eq!(fractional.text, "1.23");
    }

    #[test]
    fn malformed_amounts_are_never_coerced_or_rounded() {
        for text in ["", "-1", "+1", "1e2", "1,000", "1 token", "1.2.3", "0.001"] {
            assert!(parse_amount(text, 2).is_err(), "{text}");
        }
        assert_eq!(parse_amount("0", 2), Ok(0));
    }
}
