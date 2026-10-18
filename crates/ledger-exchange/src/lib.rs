//! Two-asset limit-order book for the simulated exchange.
//!
//! The book lists flour priced in dollars. A limit order matches resting
//! liquidity at the maker's price and any remainder rests. Solana settlement
//! is a later step and is not done here.
//!
//! Spans `exchange.submit_order`, `exchange.match`, and `exchange.fill` follow
//! the caller's active tracing span. Pass a `sentry-trace` header to
//! [`OrderBook::submit`] when that call is the root of a distributed trace.
//! [`install_telemetry`] sends those spans and the order-lifecycle logs to Sentry.

mod book;
mod error;
mod telemetry;
mod types;

pub use book::OrderBook;
pub use error::ExchangeError;
pub use telemetry::{install_telemetry, TelemetryError, TelemetryGuard};
pub use types::{
    Asset, Fill, FillId, LimitOrder, MatchReport, OrderId, OrderStatus, Price, Quantity, Side,
};
