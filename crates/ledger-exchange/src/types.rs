//! Prices, quantities, and the two listed assets.

use crate::error::ExchangeError;
use ledger_core::AgentId;
use rust_decimal::Decimal;
use std::fmt;

/// The two assets this exchange lists.
///
/// Orders buy or sell [`Asset::Flour`]. Prices are dollars per unit of flour.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Asset {
    /// Quote asset. A price is an amount of this asset per one unit of flour.
    Usd,
    /// Base asset. Order quantity is an amount of this asset.
    Flour,
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Usd => "usd",
            Self::Flour => "flour",
        })
    }
}

/// Buy or sell flour.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Side {
    /// Buy flour, paying at most `price` dollars per unit.
    Bid,
    /// Sell flour, receiving at least `price` dollars per unit.
    Ask,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bid => "bid",
            Self::Ask => "ask",
        })
    }
}

/// Identity of one limit order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct OrderId(u64);

impl OrderId {
    /// Wraps a caller-chosen id.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identity of one fill.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct FillId(u64);

impl FillId {
    /// Wraps an id assigned by the book.
    pub(crate) const fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for FillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Dollars per unit of flour. Always positive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Price(Decimal);

impl Price {
    /// Parses a positive price.
    ///
    /// # Errors
    ///
    /// Returns [`ExchangeError::NonPositivePrice`] when `value` is zero or negative.
    pub fn new(value: Decimal) -> Result<Self, ExchangeError> {
        if value <= Decimal::ZERO {
            return Err(ExchangeError::NonPositivePrice);
        }
        Ok(Self(value))
    }

    /// Returns the price as a decimal.
    #[must_use]
    pub const fn decimal(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// An amount of flour. Always positive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Quantity(Decimal);

impl Quantity {
    /// Parses a positive quantity.
    ///
    /// # Errors
    ///
    /// Returns [`ExchangeError::NonPositiveQuantity`] when `value` is zero or negative.
    pub fn new(value: Decimal) -> Result<Self, ExchangeError> {
        if value <= Decimal::ZERO {
            return Err(ExchangeError::NonPositiveQuantity);
        }
        Ok(Self(value))
    }

    /// Returns the quantity as a decimal.
    #[must_use]
    pub const fn decimal(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// A limit order to buy or sell flour.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LimitOrder {
    /// Caller-chosen id. A second submit of the same id is rejected.
    pub id: OrderId,
    /// Simulated agent that owns the order.
    pub agent: AgentId,
    /// Bid or ask.
    pub side: Side,
    /// Limit price in dollars per unit of flour.
    pub price: Price,
    /// Flour to buy or sell.
    pub quantity: Quantity,
}

/// One execution against a resting order, at the maker's price.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fill {
    /// Fill id assigned by the book.
    pub id: FillId,
    /// Maker's price. The taker's limit is not the execution price.
    pub price: Price,
    /// Flour that changed hands.
    pub quantity: Quantity,
    /// Dollars paid, `price * quantity`.
    pub quote_amount: Decimal,
    /// Incoming order.
    pub taker_order: OrderId,
    /// Resting order.
    pub maker_order: OrderId,
    /// Agent who submitted the incoming order.
    pub taker: AgentId,
    /// Agent who rested the maker order.
    pub maker: AgentId,
    /// Side of the incoming order.
    pub taker_side: Side,
}

/// What happened to an accepted limit order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderStatus {
    /// Nothing traded. The whole order rests.
    Rested,
    /// Some quantity traded. The rest rests.
    PartiallyFilled,
    /// The whole order traded. Nothing rests.
    Filled,
}

/// Fills produced by one submit, and any quantity left on the book.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchReport {
    /// Executions, in price-time order.
    pub fills: Vec<Fill>,
    /// How the incoming order finished.
    pub status: OrderStatus,
    /// Quantity still resting. Absent when the order filled completely.
    pub resting_quantity: Option<Quantity>,
}
