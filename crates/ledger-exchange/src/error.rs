//! Failures from order validation, matching, and the book index.

use crate::types::OrderId;
use thiserror::Error;

/// Why an order was not applied, or why the book could not continue.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExchangeError {
    /// Price was zero or negative.
    #[error("non-positive price")]
    NonPositivePrice,
    /// Quantity was zero or negative.
    #[error("non-positive quantity")]
    NonPositiveQuantity,
    /// This order id was already submitted.
    #[error("duplicate order {0}")]
    DuplicateOrder(OrderId),
    /// Cancel named an order that is not resting.
    #[error("unknown order {0}")]
    UnknownOrder(OrderId),
    /// The book index and the price levels disagreed.
    ///
    /// This is a defect in the matcher, not a trader rejection.
    #[error("inconsistent order book: {detail}")]
    Inconsistent {
        /// What the matcher observed.
        detail: &'static str,
    },
}
