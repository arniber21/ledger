//! Stored hypertable rows back into ledger events.
//!
//! Ingest writes both adapted Sentinel kinds and the original span name.
//! Rows that are not ledger events are skipped. A ledger row that is missing
//! the subject it needs is failed evidence, which the engine reports as
//! `Inconclusive`.

use chrono::{DateTime, Utc};
use ledger_core::{AgentId, BillId, LedgerEvent, LedgerKind, OrderEnd, OrderId, Times};
use ledger_store::Event as StoredEvent;
use serde_json::Value;

/// Reads one stored row.
///
/// `Ok(None)` is a row this monitor does not judge (`agent.decide`, a match,
/// a Solana settle). `Err` is a ledger row whose evidence failed.
pub fn read_event(row: &StoredEvent) -> Result<Option<LedgerEvent>, String> {
    let Some(kind) = kind_of(row) else {
        return Ok(None);
    };
    let agent = AgentId::new(agent_of(row)?).map_err(|error| error.to_string())?;
    let occurred = occurred_ns(row);
    let observed = observed_ns(row.observed_at);
    let times = Times::new(occurred, observed);
    let predecessors = predecessors(row);
    let end = order_end(row);

    if kind == LedgerKind::Tick {
        return LedgerEvent::tick(row.id.to_string(), agent, times, predecessors)
            .map(Some)
            .map_err(|error| error.to_string());
    }
    if kind.carries_order() {
        let order = OrderId::new(required(row, &["order.id", "order_id"], "order id")?)
            .map_err(|error| error.to_string())?;
        return LedgerEvent::order(
            row.id.to_string(),
            kind,
            order,
            end,
            agent,
            times,
            predecessors,
        )
        .map(Some)
        .map_err(|error| error.to_string());
    }
    if kind.carries_bill() {
        let bill = BillId::new(required(row, &["bill.id", "bill_id"], "bill id")?)
            .map_err(|error| error.to_string())?;
        let event = LedgerEvent::new(row.id.to_string(), kind, bill, agent, times, predecessors)
            .map_err(|error| error.to_string())?;
        return Ok(Some(with_taint(event, row)));
    }
    Ok(None)
}

/// Attribute text the engine uses to join an approval to a later payment.
#[must_use]
pub fn attribute<'a>(row: &'a StoredEvent, key: &str) -> Option<&'a str> {
    row.payload
        .pointer("/attributes")
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            row.payload
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
}

fn kind_of(row: &StoredEvent) -> Option<LedgerKind> {
    let sentinel = row
        .payload
        .pointer("/sentinel/kind")
        .and_then(Value::as_str)
        .and_then(LedgerKind::from_stored_name);
    if sentinel.is_some() {
        return sentinel;
    }
    LedgerKind::from_stored_name(&row.kind).or_else(|| {
        row.payload
            .get("span")
            .and_then(Value::as_str)
            .and_then(LedgerKind::from_stored_name)
    })
}

fn agent_of(row: &StoredEvent) -> Result<String, String> {
    if let Some(agent) = attribute(row, "agent.id").or_else(|| attribute(row, "agent_id")) {
        return Ok(agent.to_owned());
    }
    if let Some(agent) = row.source.strip_prefix("agent.") {
        if !agent.trim().is_empty() {
            return Ok(agent.to_owned());
        }
    }
    if row.source == "exchange" || kind_of(row).is_some_and(LedgerKind::is_exchange) {
        return Ok("exchange".to_owned());
    }
    Err("missing agent id".to_owned())
}

fn required(row: &StoredEvent, keys: &[&str], label: &str) -> Result<String, String> {
    keys.iter()
        .find_map(|key| attribute(row, key))
        .map(str::to_owned)
        .ok_or_else(|| format!("missing {label}"))
}

fn order_end(row: &StoredEvent) -> Option<OrderEnd> {
    let span = row
        .payload
        .get("span")
        .and_then(Value::as_str)
        .unwrap_or(row.kind.as_str());
    match span {
        "exchange.fill" => Some(OrderEnd::Fill),
        "exchange.cancel" => Some(OrderEnd::Cancel),
        _ => None,
    }
}

fn with_taint(mut event: LedgerEvent, row: &StoredEvent) -> LedgerEvent {
    let Some(object) = row
        .payload
        .pointer("/attributes")
        .and_then(Value::as_object)
    else {
        return event;
    };
    for (key, value) in object {
        if !key.starts_with("ledger.taint.") {
            continue;
        }
        let Some(text) = value.as_str().filter(|text| !text.trim().is_empty()) else {
            continue;
        };
        event = event.with_attribute(key, text);
    }
    event
}

fn predecessors(row: &StoredEvent) -> Vec<String> {
    if let Some(ids) = string_list(row.payload.pointer("/sentinel/predecessors")) {
        if !ids.is_empty() {
            return ids;
        }
    }
    if let Some(ids) = string_list(row.payload.get("predecessors")) {
        if !ids.is_empty() {
            return ids;
        }
    }
    row.payload
        .get("causal_links")
        .and_then(Value::as_array)
        .map(|links| {
            links
                .iter()
                .filter(|link| link.get("resolved").and_then(Value::as_bool) != Some(false))
                .filter_map(|link| link.get("target").and_then(Value::as_str))
                .filter(|target| !target.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn string_list(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.trim().is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn occurred_ns(row: &StoredEvent) -> i128 {
    row.payload
        .get("occurred_at_ns")
        .and_then(Value::as_i64)
        .map(i128::from)
        .unwrap_or_else(|| observed_ns(row.observed_at))
}

fn observed_ns(time: DateTime<Utc>) -> i128 {
    let seconds = i128::from(time.timestamp());
    let subsec = i128::from(time.timestamp_subsec_nanos());
    seconds.saturating_mul(1_000_000_000).saturating_add(subsec)
}
