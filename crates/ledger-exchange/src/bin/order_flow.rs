//! One order flow whose submit, match, and fill spans share the caller's trace.
//!
//! The resting ask is prior book state. The crossing bid runs inside an
//! `agent.decide` span, which is the caller context the exchange continues.

use ledger_core::AgentId;
use ledger_exchange::{install_telemetry, LimitOrder, OrderBook, OrderId, Price, Quantity, Side};
use rust_decimal::Decimal;

fn main() {
    let guard = match install_telemetry() {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("telemetry: {err}");
            std::process::exit(1);
        }
    };

    let result = run();
    drop(guard);

    if let Err(err) = result {
        eprintln!("order flow: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ledger_exchange::ExchangeError> {
    let mut book = OrderBook::new();
    book.submit(limit(1, "baker-1", Side::Ask, 500, 4), None)?;

    let parent = tracing::info_span!(
        "agent.decide",
        sentry.name = "agent.decide",
        sentry.op = "agent.decide",
        agent.id = "baker-2",
        flow = "issue-5-exchange",
    );
    let _entered = parent.enter();
    book.submit(limit(2, "baker-2", Side::Bid, 550, 6), None)?;
    Ok(())
}

fn limit(id: u64, name: &str, side: Side, cents: i64, units: i64) -> LimitOrder {
    LimitOrder {
        id: OrderId::new(id),
        agent: AgentId::new(name).expect("agent id is non-blank"),
        side,
        price: Price::new(Decimal::new(cents, 2)).expect("price is positive"),
        quantity: Quantity::new(Decimal::new(units, 0)).expect("quantity is positive"),
    }
}
