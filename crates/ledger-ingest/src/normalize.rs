//! Span normalisation into stored events, via the ledger adapter when a span
//! is a ledger event.
//!
//! Buying-agent spans (`agent.decide`, `agent.approve`, `agent.pay_bill`,
//! `agent.settle`) and exchange spans (`exchange.submit_order`,
//! `exchange.match`, `exchange.fill`, `exchange.settle`) are all stored.
//! `agent.pay_bill` and the exchange ledger kinds become Sentinel events
//! through [`adapt_trace`]. Other spans are stored as themselves.
//!
//! A causal edge is a span field whose name starts with `link.`. It becomes a
//! Sentinel predecessor only when an earlier event in the same batch has that
//! id. An unresolved link is stored and marked inconclusive. It is not treated
//! as a pass.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Timelike, Utc};
use ledger_core::{
    adapt_trace, AgentId, BillId, DataClass, LedgerEvent, LedgerKind, SourceTier, Times,
};
use ledger_store::{Event as StoredEvent, Uuid};
use sentinel_model::Event as SentinelEvent;
use serde_json::{json, Value};

use crate::capture::CapturedSpan;

const TENANT: &str = "tenant";

/// Normalises `spans` into hypertable rows.
///
/// Rows are ordered by span start time. A span that cannot be adapted is still
/// returned. Missing fields and unresolved causal links set `evidence` to
/// `inconclusive`.
#[must_use]
pub fn normalize(spans: &[CapturedSpan]) -> Vec<StoredEvent> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&index| (spans[index].started_at, spans[index].span_id));

    let ids: Vec<Uuid> = order
        .iter()
        .map(|&index| assigned_id(&spans[index]))
        .collect();
    let mut drafts: Vec<Draft> = Vec::new();
    let mut links_by_position: Vec<Vec<Link>> = Vec::with_capacity(order.len());
    let mut blocked: Vec<Option<String>> = vec![None; order.len()];
    let mut seen = BTreeSet::new();

    for (position, &index) in order.iter().enumerate() {
        let span = &spans[index];
        let id = ids[position];
        let links = causal_links(span, &drafts);
        links_by_position.push(links);
        if !seen.insert(id) {
            blocked[position] = Some("duplicate event id".to_owned());
            continue;
        }
        match ledger_event(span, id, &drafts) {
            Ok(Some(event)) => drafts.push(Draft { position, event }),
            Ok(None) => {}
            Err(reason) => blocked[position] = Some(reason),
        }
    }

    let mut summaries: BTreeMap<String, Summary> = BTreeMap::new();
    let mut batch_error = None;
    if !drafts.is_empty() {
        let events: Vec<LedgerEvent> = drafts.iter().map(|draft| draft.event.clone()).collect();
        let agent = events[0].agent_id().clone();
        match adapt_trace(&agent, &events, None) {
            Ok(adapted) => {
                for event in adapted.events() {
                    summaries.insert(event.id().as_str().to_owned(), summarize(event));
                }
            }
            Err(error) => batch_error = Some(error.to_string()),
        }
    }

    order
        .into_iter()
        .enumerate()
        .map(|(position, index)| {
            row(
                &spans[index],
                ids[position],
                &links_by_position[position],
                drafts.iter().any(|draft| draft.position == position),
                summaries.get(&ids[position].to_string()),
                blocked[position].as_deref(),
                batch_error.as_deref(),
            )
        })
        .collect()
}

struct Draft {
    position: usize,
    event: LedgerEvent,
}

struct Link {
    attribute: String,
    target: String,
    resolved: bool,
}

struct Summary {
    kind: String,
    source: String,
    data_class: &'static str,
    tier: &'static str,
    predecessors: Vec<String>,
}

fn row(
    span: &CapturedSpan,
    id: Uuid,
    links: &[Link],
    attempted: bool,
    summary: Option<&Summary>,
    blocked: Option<&str>,
    batch_error: Option<&str>,
) -> StoredEvent {
    let unresolved = links.iter().any(|link| !link.resolved);
    let mut payload = json!({
        "span": span.name,
        "span_id": span.span_id.to_string(),
        "started_at": span.started_at.to_rfc3339(),
        "closed_at": span.closed_at.to_rfc3339(),
        "attributes": span.fields,
        "causal_links": links.iter().map(link_json).collect::<Vec<_>>(),
        "adapted": summary.is_some(),
    });
    if let Some(parent) = span.parent_span_id {
        payload["parent_span_id"] = json!(parent.to_string());
    }
    if let Some(summary) = summary {
        payload["sentinel"] = json!({
            "event_id": id.to_string(),
            "kind": summary.kind,
            "source": summary.source,
            "tier": summary.tier,
            "data_class": summary.data_class,
            "predecessors": summary.predecessors,
        });
    }

    let failed_adapt = attempted && summary.is_none();
    if blocked.is_some() || failed_adapt || unresolved {
        payload["evidence"] = json!("inconclusive");
        let reason = blocked
            .or(batch_error.filter(|_| failed_adapt))
            .or(unresolved.then_some("unresolved causal link"));
        if let Some(reason) = reason {
            payload["reason"] = json!(reason);
        }
    }

    let kind = summary
        .map(|summary| summary.kind.clone())
        .unwrap_or_else(|| span.name.to_owned());
    let source = summary
        .map(|summary| summary.source.clone())
        .unwrap_or_else(|| source_of(span));
    let data_class = summary.map(|summary| summary.data_class).unwrap_or(TENANT);

    StoredEvent {
        id,
        observed_at: truncate_micros(span.closed_at),
        source,
        kind,
        data_class: data_class.to_owned(),
        payload,
    }
}

fn link_json(link: &Link) -> Value {
    json!({
        "attribute": link.attribute,
        "target": link.target,
        "resolved": link.resolved,
    })
}

fn summarize(event: &SentinelEvent) -> Summary {
    let producer = event.producer();
    Summary {
        kind: producer.kind().as_str().to_owned(),
        source: event.source().source().as_str().to_owned(),
        data_class: data_class_label(event.data_class()),
        tier: tier_label(event.source_tier()),
        predecessors: producer
            .causal_predecessors()
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
    }
}

fn ledger_event(
    span: &CapturedSpan,
    id: Uuid,
    prior: &[Draft],
) -> Result<Option<LedgerEvent>, String> {
    let Some(kind) = ledger_kind(span.name) else {
        return Ok(None);
    };
    let Some(bill) = field(span, "bill.id") else {
        return Err("missing bill id".to_owned());
    };
    let Some(agent) = field(span, "agent.id") else {
        return Err("missing agent id".to_owned());
    };
    let predecessors = causal_links(span, prior)
        .into_iter()
        .filter(|link| link.resolved)
        .map(|link| link.target);
    let occurred = nanos(span.started_at);
    let observed = nanos(span.closed_at);
    let event = LedgerEvent::new(
        id.to_string(),
        kind,
        BillId::new(bill).map_err(|error| error.to_string())?,
        AgentId::new(agent).map_err(|error| error.to_string())?,
        Times::new(occurred, observed),
        predecessors,
    )
    .map_err(|error| error.to_string())?;
    Ok(Some(event))
}

fn ledger_kind(name: &str) -> Option<LedgerKind> {
    match name {
        "agent.pay_bill" | "ledger.agent.pay_bill" => Some(LedgerKind::AgentPayBill),
        "agent.memo" | "ledger.agent.memo" => Some(LedgerKind::BillMemo),
        "agent.decision" | "ledger.agent.decision" => Some(LedgerKind::AgentDecision),
        "ledger.exchange.settled" => Some(LedgerKind::ObligationSettled),
        "ledger.exchange.payment" => Some(LedgerKind::ExtraPayment),
        "ledger.exchange.credit" => Some(LedgerKind::Credit),
        _ => None,
    }
}

fn causal_links(span: &CapturedSpan, prior: &[Draft]) -> Vec<Link> {
    span.fields
        .iter()
        .filter(|(key, _)| key.starts_with("link."))
        .map(|(attribute, target)| Link {
            attribute: attribute.clone(),
            target: target.clone(),
            resolved: prior.iter().any(|draft| draft.event.id() == target),
        })
        .collect()
}

fn source_of(span: &CapturedSpan) -> String {
    if span.name.starts_with("exchange.") || span.name.starts_with("ledger.exchange.") {
        "exchange".to_owned()
    } else if let Some(agent) = field(span, "agent.id") {
        format!("agent.{agent}")
    } else {
        "agent".to_owned()
    }
}

fn field<'a>(span: &'a CapturedSpan, key: &str) -> Option<&'a str> {
    span.fields
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn assigned_id(span: &CapturedSpan) -> Uuid {
    field(span, "event.id")
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or_else(Uuid::new_v4)
}

fn data_class_label(class: DataClass) -> &'static str {
    match class {
        DataClass::Tenant => TENANT,
        DataClass::Corpus => "corpus",
        _ => "unspecified",
    }
}

fn tier_label(tier: SourceTier) -> &'static str {
    match tier {
        SourceTier::Anchor => "anchor",
        SourceTier::Contributing => "contributing",
        _ => "unspecified",
    }
}

fn nanos(time: DateTime<Utc>) -> i128 {
    let seconds = i128::from(time.timestamp());
    let subsec = i128::from(time.timestamp_subsec_nanos());
    seconds.saturating_mul(1_000_000_000).saturating_add(subsec)
}

/// Timescale `timestamptz` stores microseconds. Truncate so a round trip matches.
fn truncate_micros(time: DateTime<Utc>) -> DateTime<Utc> {
    let nanos = time.timestamp_subsec_nanos();
    let truncated = nanos - nanos % 1_000;
    time.with_nanosecond(truncated).unwrap_or(time)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(name: &'static str, id: u64, at: i64, fields: &[(&str, &str)]) -> CapturedSpan {
        let started_at = DateTime::from_timestamp(at, 0).expect("timestamp");
        CapturedSpan::new(
            name,
            id,
            None,
            started_at,
            fields
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn agent_decide_and_exchange_spans_are_stored_unadapted() {
        let spans = [
            span(
                "agent.decide",
                1,
                1_900_000_000,
                &[("agent.id", "payer"), ("decision.kind", "pay")],
            ),
            span(
                "exchange.submit_order",
                2,
                1_900_000_001,
                &[("agent.id", "buyer"), ("order.id", "7")],
            ),
            span("exchange.match", 3, 1_900_000_002, &[("order.id", "7")]),
            span(
                "exchange.fill",
                4,
                1_900_000_003,
                &[("fill.id", "1"), ("order.id", "7")],
            ),
            span("exchange.settle", 5, 1_900_000_004, &[("fill.id", "1")]),
            span(
                "agent.settle",
                6,
                1_900_000_005,
                &[
                    ("agent.id", "buyer"),
                    ("link.order_id", "7"),
                    ("link.fill_id", "1"),
                ],
            ),
        ];
        let rows = normalize(&spans);
        assert_eq!(rows.len(), spans.len());
        assert_eq!(rows[0].kind, "agent.decide");
        assert_eq!(rows[0].source, "agent.payer");
        assert_eq!(rows[0].data_class, "tenant");
        assert_eq!(rows[0].payload["adapted"], json!(false));
        assert!(rows[0].payload.get("evidence").is_none());
        assert_eq!(rows[1].kind, "exchange.submit_order");
        assert_eq!(rows[1].source, "exchange");
        assert_eq!(rows[4].kind, "exchange.settle");
        assert_eq!(rows[5].payload["evidence"], json!("inconclusive"));
        assert_eq!(rows[5].payload["reason"], json!("unresolved causal link"));
    }

    #[test]
    fn pay_bill_is_adapted_and_a_dangling_link_stays_inconclusive() {
        let rows = normalize(&[span(
            "agent.pay_bill",
            1,
            1_900_000_010,
            &[
                ("agent.id", "payer"),
                ("bill.id", "bill-1"),
                ("link.approval_id", "apr-1"),
            ],
        )]);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.kind, "ledger.agent.pay_bill");
        assert_eq!(row.source, "agent.payer");
        assert_eq!(row.data_class, "tenant");
        assert_eq!(row.payload["adapted"], json!(true));
        assert_eq!(row.payload["span"], json!("agent.pay_bill"));
        assert_eq!(row.payload["evidence"], json!("inconclusive"));
        assert_eq!(row.payload["reason"], json!("unresolved causal link"));
        assert_eq!(row.payload["sentinel"]["tier"], json!("contributing"));
        assert_eq!(row.payload["sentinel"]["predecessors"], json!([]));
    }

    #[test]
    fn missing_bill_is_stored_inconclusive() {
        let rows = normalize(&[span(
            "agent.pay_bill",
            1,
            1_900_000_020,
            &[("agent.id", "payer")],
        )]);
        assert_eq!(rows[0].kind, "agent.pay_bill");
        assert_eq!(rows[0].payload["adapted"], json!(false));
        assert_eq!(rows[0].payload["evidence"], json!("inconclusive"));
        assert_eq!(rows[0].payload["reason"], json!("missing bill id"));
    }

    #[test]
    fn resolved_link_becomes_a_sentinel_predecessor() {
        let first = "11111111-1111-4111-8111-111111111111";
        let second = "22222222-2222-4222-8222-222222222222";
        let rows = normalize(&[
            span(
                "agent.pay_bill",
                1,
                1_900_000_030,
                &[
                    ("event.id", first),
                    ("agent.id", "payer"),
                    ("bill.id", "bill-1"),
                ],
            ),
            span(
                "ledger.exchange.payment",
                2,
                1_900_000_031,
                &[
                    ("event.id", second),
                    ("agent.id", "payer"),
                    ("bill.id", "bill-1"),
                    ("link.approval_id", first),
                ],
            ),
        ]);
        assert_eq!(rows[1].kind, "ledger.exchange.payment");
        assert_eq!(rows[1].source, "exchange");
        assert!(rows[1].payload.get("evidence").is_none());
        assert_eq!(rows[1].payload["causal_links"][0]["resolved"], json!(true));
        assert_eq!(rows[1].payload["sentinel"]["predecessors"], json!([first]));
        assert_eq!(rows[1].payload["sentinel"]["tier"], json!("anchor"));
    }
}
