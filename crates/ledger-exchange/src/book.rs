//! Price-time priority limit order book.
//!
//! Adapted from the continuous FIFO matcher in
//! <https://github.com/yiweichi/matching-engine> (`src/orderbook.rs`, MIT,
//! Copyright (c) 2026 Morty).
//!
//! That matcher keeps bids and asks in price-keyed maps, stores each price
//! level as a FIFO queue, walks asks from the lowest price for a buy and bids
//! from the highest price for a sell, fills at the maker's price, and rests any
//! remainder of a limit order. This port uses `VecDeque` levels and
//! [`rust_decimal::Decimal`] prices instead of the original integer arena.
//! Market orders are left out: every accepted order is a limit order.

use crate::error::ExchangeError;
use crate::types::{
    Asset, Fill, FillId, LimitOrder, MatchReport, OrderId, OrderStatus, Price, Quantity, Side,
};
use ledger_core::AgentId;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// One resting order at a price level. The queue front is the oldest.
#[derive(Debug)]
struct RestingOrder {
    id: OrderId,
    agent: AgentId,
    quantity: Quantity,
}

/// FIFO queue of orders at one price.
#[derive(Debug, Default)]
struct Level {
    orders: VecDeque<RestingOrder>,
}

/// Flour/USD limit book.
///
/// A bid matches asks at or below its limit, cheapest ask first. An ask matches
/// bids at or above its limit, highest bid first. At one price, the oldest
/// resting order fills first and keeps that place after a partial fill.
#[derive(Debug)]
pub struct OrderBook {
    bids: BTreeMap<Price, Level>,
    asks: BTreeMap<Price, Level>,
    /// Resting order id to the level that holds it.
    locations: HashMap<OrderId, (Side, Price)>,
    /// Every id ever accepted. Reuse is a duplicate, including after a fill.
    seen: HashSet<OrderId>,
    next_fill_id: u64,
}

impl Default for OrderBook {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderBook {
    /// Empty flour/USD book. Fill ids start at 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locations: HashMap::new(),
            seen: HashSet::new(),
            next_fill_id: 1,
        }
    }

    /// Base asset. Quantity is denominated in this.
    #[must_use]
    pub const fn base() -> Asset {
        Asset::Flour
    }

    /// Quote asset. Prices are denominated in this.
    #[must_use]
    pub const fn quote() -> Asset {
        Asset::Usd
    }

    /// Number of resting orders.
    #[must_use]
    pub fn len(&self) -> usize {
        self.locations.len()
    }

    /// Whether nothing is resting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }

    /// Highest resting bid, if any.
    #[must_use]
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.last_key_value().map(|(price, _)| *price)
    }

    /// Lowest resting ask, if any.
    #[must_use]
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.first_key_value().map(|(price, _)| *price)
    }

    /// Quantity still resting for `id`.
    #[must_use]
    pub fn resting_quantity(&self, id: OrderId) -> Option<Quantity> {
        let (side, price) = self.locations.get(&id)?;
        let level = match side {
            Side::Bid => self.bids.get(price)?,
            Side::Ask => self.asks.get(price)?,
        };
        level
            .orders
            .iter()
            .find(|order| order.id == id)
            .map(|order| order.quantity)
    }

    /// Submits a limit order.
    ///
    /// `sentry_trace` is the caller's `sentry-trace` header. When this submit
    /// is a root span, that header continues the caller's distributed trace.
    /// When the caller already has a tracing span entered, that span is the
    /// parent and the header is not applied. An empty header is ignored.
    ///
    /// # Errors
    ///
    /// Returns [`ExchangeError::DuplicateOrder`] when `order.id` was already
    /// submitted. An inconsistent book is [`ExchangeError::Inconsistent`] and
    /// is logged at error level so the Sentry layer can capture it.
    pub fn submit(
        &mut self,
        order: LimitOrder,
        sentry_trace: Option<&str>,
    ) -> Result<MatchReport, ExchangeError> {
        let header = sentry_trace
            .map(str::trim)
            .filter(|header| !header.is_empty());
        let span = match header {
            Some(header) => tracing::info_span!(
                "exchange.submit_order",
                sentry.name = "exchange.submit_order",
                sentry.op = "exchange.submit",
                sentry.trace = header,
                order.id = %order.id,
                agent.id = order.agent.as_str(),
                side = %order.side,
                price = %order.price,
                quantity = %order.quantity,
                base = "flour",
                quote = "usd",
            ),
            None => tracing::info_span!(
                "exchange.submit_order",
                sentry.name = "exchange.submit_order",
                sentry.op = "exchange.submit",
                order.id = %order.id,
                agent.id = order.agent.as_str(),
                side = %order.side,
                price = %order.price,
                quantity = %order.quantity,
                base = "flour",
                quote = "usd",
            ),
        };
        let _entered = span.enter();
        self.submit_entered(order)
    }

    /// Removes a resting order.
    ///
    /// # Errors
    ///
    /// Returns [`ExchangeError::UnknownOrder`] when `id` is not resting.
    /// A filled order is not resting.
    pub fn cancel(&mut self, id: OrderId) -> Result<Quantity, ExchangeError> {
        let Some((side, price)) = self.locations.remove(&id) else {
            return Err(rejected(id, ExchangeError::UnknownOrder(id)));
        };
        let quantity = self.take_resting(side, price, id)?;
        tracing::info!(order.id = %id, quantity = %quantity, "order cancelled");
        Ok(quantity)
    }

    fn submit_entered(&mut self, order: LimitOrder) -> Result<MatchReport, ExchangeError> {
        if !self.seen.insert(order.id) {
            return Err(rejected(order.id, ExchangeError::DuplicateOrder(order.id)));
        }
        tracing::info!(order.id = %order.id, "order accepted");

        let mut remaining = order.quantity.decimal();
        let fills = {
            let span = tracing::info_span!(
                "exchange.match",
                sentry.name = "exchange.match",
                sentry.op = "exchange.match",
                order.id = %order.id,
                side = %order.side,
            );
            let _entered = span.enter();
            self.match_incoming(&order, &mut remaining)?
        };

        let order_id = order.id;
        let resting_quantity = if remaining > Decimal::ZERO {
            let quantity = Quantity::new(remaining)?;
            self.rest(order, quantity)?;
            Some(quantity)
        } else {
            None
        };

        let status = if fills.is_empty() {
            OrderStatus::Rested
        } else if resting_quantity.is_some() {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Filled
        };

        match status {
            OrderStatus::Rested => {
                tracing::info!(order.id = %order_id, quantity = %remaining, "order rested");
            }
            OrderStatus::PartiallyFilled => {
                tracing::info!(
                    order.id = %order_id,
                    fill.count = fills.len(),
                    quantity = %remaining,
                    "order partially filled"
                );
            }
            OrderStatus::Filled => {
                tracing::info!(order.id = %order_id, fill.count = fills.len(), "order filled");
            }
        }

        Ok(MatchReport {
            fills,
            status,
            resting_quantity,
        })
    }

    /// Incoming buy walks asks from the lowest price. Incoming sell walks bids
    /// from the highest price. Stops when the next price does not cross.
    fn match_incoming(
        &mut self,
        order: &LimitOrder,
        remaining: &mut Decimal,
    ) -> Result<Vec<Fill>, ExchangeError> {
        let mut fills = Vec::new();
        while *remaining > Decimal::ZERO {
            let best = match order.side {
                Side::Bid => self.best_ask(),
                Side::Ask => self.best_bid(),
            };
            let Some(best_price) = best else {
                break;
            };
            let crosses = match order.side {
                Side::Bid => best_price <= order.price,
                Side::Ask => best_price >= order.price,
            };
            if !crosses {
                break;
            }
            self.fill_level(order, best_price, remaining, &mut fills)?;
            self.remove_level_if_empty(opposite(order.side), best_price);
        }
        Ok(fills)
    }

    fn fill_level(
        &mut self,
        order: &LimitOrder,
        price: Price,
        remaining: &mut Decimal,
        fills: &mut Vec<Fill>,
    ) -> Result<(), ExchangeError> {
        let maker_side = opposite(order.side);
        while *remaining > Decimal::ZERO {
            let stepped = {
                let Some(level) = self.level_mut(maker_side, price) else {
                    return Ok(());
                };
                let Some(maker) = level.orders.front_mut() else {
                    return Ok(());
                };
                let maker_qty = maker.quantity.decimal();
                let fill_qty = (*remaining).min(maker_qty);
                if fill_qty <= Decimal::ZERO {
                    return Err(inconsistent("fill quantity was not positive"));
                }
                let maker_left = maker_qty
                    .checked_sub(fill_qty)
                    .ok_or_else(|| inconsistent("maker quantity underflow"))?;
                let taker_left = remaining
                    .checked_sub(fill_qty)
                    .ok_or_else(|| inconsistent("taker quantity underflow"))?;
                *remaining = taker_left;

                if maker_left.is_zero() {
                    let done = level
                        .orders
                        .pop_front()
                        .ok_or_else(|| inconsistent("maker disappeared before removal"))?;
                    (done.id, done.agent, fill_qty, true)
                } else {
                    let id = maker.id;
                    let agent = maker.agent.clone();
                    maker.quantity = Quantity::new(maker_left)?;
                    (id, agent, fill_qty, false)
                }
            };

            let (maker_id, maker_agent, fill_qty, maker_done) = stepped;
            if maker_done {
                self.locations.remove(&maker_id);
            }
            if maker_done {
                // The fill span above names the taker in `order.id`, so the
                // monitor would never see the resting maker order resolve and
                // would report `order-unresolved` for a fully filled order.
                // This span resolves the maker once, when it leaves the book.
                let span = tracing::info_span!(
                    "exchange.fill",
                    sentry.name = "exchange.fill",
                    sentry.op = "exchange.fill",
                    order.id = %maker_id,
                    maker.order.id = %maker_id,
                    taker.order.id = %order.id,
                    role = "maker-filled",
                );
                let _entered = span.enter();
                tracing::info!(order.id = %maker_id, "resting order filled");
            }
            fills.push(self.record_fill(order, maker_id, maker_agent, price, fill_qty)?);
        }
        Ok(())
    }

    fn record_fill(
        &mut self,
        taker: &LimitOrder,
        maker_id: OrderId,
        maker_agent: AgentId,
        price: Price,
        quantity: Decimal,
    ) -> Result<Fill, ExchangeError> {
        let quantity = Quantity::new(quantity)?;
        let quote_amount = price
            .decimal()
            .checked_mul(quantity.decimal())
            .ok_or_else(|| inconsistent("quote amount overflow"))?;
        let id = self.allocate_fill_id()?;
        let fill = Fill {
            id,
            price,
            quantity,
            quote_amount,
            taker_order: taker.id,
            maker_order: maker_id,
            taker: taker.agent.clone(),
            maker: maker_agent,
            taker_side: taker.side,
        };

        let span = tracing::info_span!(
            "exchange.fill",
            sentry.name = "exchange.fill",
            sentry.op = "exchange.fill",
            fill.id = %fill.id,
            order.id = %taker.id,
            maker.order.id = %maker_id,
            taker.order.id = %taker.id,
            price = %price,
            quantity = %quantity,
        );
        let _entered = span.enter();
        tracing::info!(
            fill.id = %fill.id,
            maker.order.id = %maker_id,
            taker.order.id = %taker.id,
            price = %price,
            quantity = %quantity,
            quote.amount = %quote_amount,
            "fill executed"
        );
        Ok(fill)
    }

    fn rest(&mut self, order: LimitOrder, quantity: Quantity) -> Result<(), ExchangeError> {
        let id = order.id;
        let side = order.side;
        let price = order.price;
        if self.locations.contains_key(&id) {
            return Err(inconsistent("rested an order id that is already resting"));
        }
        {
            let level = match side {
                Side::Bid => self.bids.entry(price).or_default(),
                Side::Ask => self.asks.entry(price).or_default(),
            };
            level.orders.push_back(RestingOrder {
                id,
                agent: order.agent,
                quantity,
            });
        }
        self.locations.insert(id, (side, price));
        Ok(())
    }

    fn take_resting(
        &mut self,
        side: Side,
        price: Price,
        id: OrderId,
    ) -> Result<Quantity, ExchangeError> {
        let quantity = {
            let Some(level) = self.level_mut(side, price) else {
                return Err(inconsistent("order index points at a missing level"));
            };
            let Some(position) = level.orders.iter().position(|order| order.id == id) else {
                return Err(inconsistent("order index points at a missing order"));
            };
            let Some(resting) = level.orders.remove(position) else {
                return Err(inconsistent("order missing at index"));
            };
            resting.quantity
        };
        self.remove_level_if_empty(side, price);
        Ok(quantity)
    }

    fn remove_level_if_empty(&mut self, side: Side, price: Price) {
        let empty = match side {
            Side::Bid => self
                .bids
                .get(&price)
                .is_none_or(|level| level.orders.is_empty()),
            Side::Ask => self
                .asks
                .get(&price)
                .is_none_or(|level| level.orders.is_empty()),
        };
        if empty {
            match side {
                Side::Bid => {
                    self.bids.remove(&price);
                }
                Side::Ask => {
                    self.asks.remove(&price);
                }
            }
        }
    }

    fn level_mut(&mut self, side: Side, price: Price) -> Option<&mut Level> {
        match side {
            Side::Bid => self.bids.get_mut(&price),
            Side::Ask => self.asks.get_mut(&price),
        }
    }

    fn allocate_fill_id(&mut self) -> Result<FillId, ExchangeError> {
        let id = self.next_fill_id;
        self.next_fill_id = id
            .checked_add(1)
            .ok_or_else(|| inconsistent("fill id overflow"))?;
        Ok(FillId::from_raw(id))
    }
}

/// Side the incoming order trades against.
fn opposite(side: Side) -> Side {
    match side {
        Side::Bid => Side::Ask,
        Side::Ask => Side::Bid,
    }
}

/// Trader-facing rejection. Warn, so Sentry records a log and not an error event.
fn rejected(order_id: OrderId, err: ExchangeError) -> ExchangeError {
    tracing::warn!(order.id = %order_id, reason = %err, "order rejected");
    err
}

/// Book defect. Error level, so the Sentry tracing layer captures an event.
fn inconsistent(detail: &'static str) -> ExchangeError {
    let err = ExchangeError::Inconsistent { detail };
    tracing::error!(error = %err, "exchange book inconsistent");
    err
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn price_cents(cents: i64) -> Price {
        Price::new(Decimal::new(cents, 2)).expect("positive price")
    }

    fn lots(units: i64) -> Quantity {
        Quantity::new(Decimal::new(units, 0)).expect("positive quantity")
    }

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).expect("agent id")
    }

    fn order(id: u64, name: &str, side: Side, cents: i64, units: i64) -> LimitOrder {
        LimitOrder {
            id: OrderId::new(id),
            agent: agent(name),
            side,
            price: price_cents(cents),
            quantity: lots(units),
        }
    }

    fn submit(book: &mut OrderBook, order: LimitOrder) -> MatchReport {
        book.submit(order, None).expect("order accepted")
    }

    #[test]
    fn non_crossing_orders_rest() {
        let mut book = OrderBook::new();
        let ask = submit(&mut book, order(1, "baker-1", Side::Ask, 500, 4));
        let bid = submit(&mut book, order(2, "baker-2", Side::Bid, 400, 4));

        assert_eq!(ask.status, OrderStatus::Rested);
        assert!(ask.fills.is_empty());
        assert_eq!(bid.status, OrderStatus::Rested);
        assert_eq!(book.best_ask(), Some(price_cents(500)));
        assert_eq!(book.best_bid(), Some(price_cents(400)));
        assert_eq!(book.len(), 2);
    }

    #[test]
    fn fill_uses_the_maker_price() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 500, 4));
        let report = submit(&mut book, order(2, "baker-2", Side::Bid, 650, 4));

        assert_eq!(report.status, OrderStatus::Filled);
        assert_eq!(report.fills.len(), 1);
        let fill = &report.fills[0];
        assert_eq!(fill.price, price_cents(500));
        assert_eq!(fill.quantity, lots(4));
        assert_eq!(fill.quote_amount, Decimal::new(2000, 2));
        assert_eq!(fill.maker_order, OrderId::new(1));
        assert_eq!(fill.taker_order, OrderId::new(2));
        assert_eq!(fill.taker_side, Side::Bid);
        assert!(book.is_empty());
    }

    #[test]
    fn better_price_beats_earlier_time() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 600, 5));
        submit(&mut book, order(2, "baker-2", Side::Ask, 500, 5));
        let report = submit(&mut book, order(3, "baker-3", Side::Bid, 600, 5));

        assert_eq!(report.fills.len(), 1);
        assert_eq!(report.fills[0].maker_order, OrderId::new(2));
        assert_eq!(report.fills[0].price, price_cents(500));
        assert_eq!(book.resting_quantity(OrderId::new(1)), Some(lots(5)));
        assert_eq!(book.resting_quantity(OrderId::new(2)), None);
    }

    #[test]
    fn same_price_fills_the_earlier_order_first() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 500, 3));
        submit(&mut book, order(2, "baker-2", Side::Ask, 500, 3));
        let report = submit(&mut book, order(3, "baker-3", Side::Bid, 500, 4));

        assert_eq!(report.status, OrderStatus::Filled);
        assert_eq!(report.fills.len(), 2);
        assert_eq!(report.fills[0].maker_order, OrderId::new(1));
        assert_eq!(report.fills[0].quantity, lots(3));
        assert_eq!(report.fills[1].maker_order, OrderId::new(2));
        assert_eq!(report.fills[1].quantity, lots(1));
        assert_eq!(book.resting_quantity(OrderId::new(2)), Some(lots(2)));
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn partial_fill_keeps_the_maker_at_the_front() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Bid, 500, 10));
        submit(&mut book, order(2, "baker-2", Side::Bid, 500, 10));
        let first = submit(&mut book, order(3, "baker-3", Side::Ask, 500, 4));
        let second = submit(&mut book, order(4, "baker-4", Side::Ask, 500, 7));

        assert_eq!(first.fills[0].maker_order, OrderId::new(1));
        assert_eq!(first.fills[0].quantity, lots(4));
        assert_eq!(second.fills[0].maker_order, OrderId::new(1));
        assert_eq!(second.fills[0].quantity, lots(6));
        assert_eq!(second.fills[1].maker_order, OrderId::new(2));
        assert_eq!(second.fills[1].quantity, lots(1));
        assert_eq!(book.resting_quantity(OrderId::new(1)), None);
        assert_eq!(book.resting_quantity(OrderId::new(2)), Some(lots(9)));
    }

    #[test]
    fn incoming_bid_sweeps_ask_levels_at_maker_prices() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 500, 10));
        submit(&mut book, order(2, "baker-2", Side::Ask, 600, 10));
        let report = submit(&mut book, order(3, "baker-3", Side::Bid, 600, 25));

        assert_eq!(report.status, OrderStatus::PartiallyFilled);
        assert_eq!(report.resting_quantity, Some(lots(5)));
        assert_eq!(report.fills[0].price, price_cents(500));
        assert_eq!(report.fills[0].quantity, lots(10));
        assert_eq!(report.fills[0].quote_amount, Decimal::new(5000, 2));
        assert_eq!(report.fills[1].price, price_cents(600));
        assert_eq!(report.fills[1].quantity, lots(10));
        assert_eq!(report.fills[1].quote_amount, Decimal::new(6000, 2));
        assert_eq!(book.best_bid(), Some(price_cents(600)));
        assert_eq!(book.best_ask(), None);
        assert_eq!(book.resting_quantity(OrderId::new(2)), None);
        assert_eq!(book.resting_quantity(OrderId::new(3)), Some(lots(5)));
    }

    #[test]
    fn incoming_ask_sweeps_bids_from_the_highest_price() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Bid, 400, 2));
        submit(&mut book, order(2, "baker-2", Side::Bid, 700, 2));
        let report = submit(&mut book, order(3, "baker-3", Side::Ask, 400, 3));

        assert_eq!(report.fills[0].maker_order, OrderId::new(2));
        assert_eq!(report.fills[0].price, price_cents(700));
        assert_eq!(report.fills[1].maker_order, OrderId::new(1));
        assert_eq!(report.fills[1].price, price_cents(400));
        assert_eq!(report.fills[1].quantity, lots(1));
        assert_eq!(book.resting_quantity(OrderId::new(1)), Some(lots(1)));
    }

    #[test]
    fn cancel_removes_liquidity() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 500, 4));
        assert_eq!(book.cancel(OrderId::new(1)).expect("cancelled"), lots(4));
        let report = submit(&mut book, order(2, "baker-2", Side::Bid, 500, 4));

        assert_eq!(report.status, OrderStatus::Rested);
        assert!(report.fills.is_empty());
        assert_eq!(book.best_ask(), None);
    }

    #[test]
    fn duplicate_id_is_rejected_and_leaves_the_book() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 500, 4));
        let err = book
            .submit(order(1, "baker-2", Side::Bid, 500, 1), None)
            .expect_err("duplicate");

        assert_eq!(err, ExchangeError::DuplicateOrder(OrderId::new(1)));
        assert_eq!(book.len(), 1);
        assert_eq!(book.best_ask(), Some(price_cents(500)));
    }

    #[test]
    fn filled_id_cannot_be_reused() {
        let mut book = OrderBook::new();
        submit(&mut book, order(1, "baker-1", Side::Ask, 500, 1));
        submit(&mut book, order(2, "baker-2", Side::Bid, 500, 1));
        let err = book
            .submit(order(1, "baker-1", Side::Ask, 500, 1), None)
            .expect_err("reused id");

        assert_eq!(err, ExchangeError::DuplicateOrder(OrderId::new(1)));
        assert!(book.is_empty());
    }

    #[test]
    fn unknown_cancel_is_rejected() {
        let mut book = OrderBook::new();
        let err = book.cancel(OrderId::new(9)).expect_err("missing");
        assert_eq!(err, ExchangeError::UnknownOrder(OrderId::new(9)));
    }

    #[test]
    fn non_positive_price_and_quantity_are_rejected() {
        assert_eq!(
            Price::new(Decimal::ZERO).expect_err("zero"),
            ExchangeError::NonPositivePrice
        );
        assert_eq!(
            Price::new(Decimal::new(-1, 2)).expect_err("negative"),
            ExchangeError::NonPositivePrice
        );
        assert_eq!(
            Quantity::new(Decimal::ZERO).expect_err("zero"),
            ExchangeError::NonPositiveQuantity
        );
    }

    #[test]
    fn sentry_trace_header_does_not_change_the_match() {
        let mut book = OrderBook::new();
        let header = "00000000000000000000000000000000-0000000000000001-1";
        let report = book
            .submit(order(1, "baker-1", Side::Ask, 500, 2), Some(header))
            .expect("resting ask");
        assert_eq!(report.status, OrderStatus::Rested);
        assert_eq!(book.best_ask(), Some(price_cents(500)));
    }

    #[test]
    fn assets_are_flour_and_usd() {
        assert_eq!(OrderBook::base(), Asset::Flour);
        assert_eq!(OrderBook::quote(), Asset::Usd);
    }
}
