//! Two-asset limit-order book for the simulated exchange.
//!
//! The book lists flour priced in dollars. A limit order matches resting
//! liquidity at the maker's price and any remainder rests. Each fill then
//! settles as one SPL transfer of the quote on a local validator.
//!
//! Spans `exchange.submit_order`, `exchange.match`, `exchange.fill`, and
//! `exchange.settle` follow the caller's active tracing span. Pass a
//! `sentry-trace` header to [`OrderBook::submit`] when that call is the root
//! of a distributed trace. [`install_telemetry`] fans those spans out to
//! Sentry and to ledger-ingest. Ingest is not sampled.

mod book;
mod error;
mod settlement;
mod telemetry;
mod types;

pub use book::OrderBook;
pub use error::ExchangeError;
pub use settlement::{
    Settlement, SettlementError, TokenSettler, LOCAL_VALIDATOR_URL, QUOTE_DECIMALS,
};
pub use telemetry::{install_telemetry, TelemetryError, TelemetryGuard};
pub use types::{
    Asset, Fill, FillId, LimitOrder, MatchReport, OrderId, OrderStatus, Price, Quantity, Side,
};
