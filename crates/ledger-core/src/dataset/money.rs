//! Decimal-string money. Amounts are parsed from two-decimal strings, never floats.

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;
use thiserror::Error;

/// A currency amount parsed from a two-decimal string such as `"500.00"`.
///
/// The dataset currency lives in the world manifest. This type does not carry
/// a currency code; every amount in one loaded company shares that currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money(Decimal);

impl Money {
    /// Zero at two-decimal scale, used when a document has no lines.
    pub const ZERO: Self = Self(Decimal::ZERO);

    /// Adds `rhs`.
    ///
    /// Returns `None` when the sum overflows.
    #[must_use]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// Multiplies this unit amount by `qty`.
    ///
    /// Returns `None` when the product overflows.
    #[must_use]
    pub fn checked_mul_u32(self, qty: u32) -> Option<Self> {
        self.0.checked_mul(Decimal::from(qty)).map(Self)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A string was not a two-decimal money amount.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid decimal-string money: {input}")]
pub struct MoneyParseError {
    input: String,
}

impl FromStr for Money {
    type Err = MoneyParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if !is_two_decimal_string(input) {
            return Err(MoneyParseError {
                input: input.to_owned(),
            });
        }
        Decimal::from_str(input)
            .map(Self)
            .map_err(|_| MoneyParseError {
                input: input.to_owned(),
            })
    }
}

/// `[-]digits.dd` with no exponent, no leading sign other than `-`, and exactly two fractional digits.
fn is_two_decimal_string(input: &str) -> bool {
    let rest = input.strip_prefix('-').unwrap_or(input);
    let Some((whole, frac)) = rest.split_once('.') else {
        return false;
    };
    !whole.is_empty()
        && frac.len() == 2
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && frac.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_decimal_strings() {
        let amount: Money = "500.00".parse().unwrap();
        assert_eq!(amount.to_string(), "500.00");
        assert_eq!("-3.50".parse::<Money>().unwrap().to_string(), "-3.50");
    }

    #[test]
    fn rejects_strings_that_are_not_two_decimal_places() {
        for input in [
            "", "500", "500.0", "500.000", "12.3a", "+500.00", "500.", ".50",
        ] {
            assert!(
                input.parse::<Money>().is_err(),
                "{input} should not parse as money"
            );
        }
    }

    #[test]
    fn checked_arithmetic_matches_line_math() {
        let unit: Money = "6.83".parse().unwrap();
        let line = unit.checked_mul_u32(600).unwrap();
        assert_eq!(line.to_string(), "4098.00");
        let rent: Money = "500.00".parse().unwrap();
        assert_eq!(rent.checked_add(rent).unwrap().to_string(), "1000.00");
    }
}
