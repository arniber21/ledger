//! Live trace read API.
//!
//! Rows come from the `events` hypertable (`id`, `observed_at`, `source`,
//! `kind`, `data_class`, `payload`). `payload` is the object written by
//! `ledger-ingest`: `span`, `span_id`, `parent_span_id`, `started_at`,
//! `closed_at`, `attributes`, `causal_links`, `adapted`, and, when present,
//! `sentinel`, `evidence`, and `reason`. There is no trace-id column. The UI
//! groups rows by `payload.parent_span_id`.
//!
//! The feed is span rows (`payload.span`), not every hypertable row. A page
//! of the newest spans also includes the earlier spans on those traces
//! (`agent.decide`, `exchange.match`, `exchange.fill`, `exchange.settle`,
//! `agent.pay_bill`, `agent.settle`, and the same links the other way).

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use ledger_store::{DateTime, Event as StoredEvent, EventCursor, Store, Utc, Uuid};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::fault::toggle;
use crate::stream::publish;

/// How many hypertable rows a snapshot or a poll returns.
const PAGE: i64 = 200;

/// How far before the oldest seed to look for the rest of a trace.
const TRACE_LOOKBACK: chrono::Duration = chrono::Duration::minutes(30);

/// Parent/child hops from `agent.decide` through settle.
const TRACE_HOPS: usize = 8;

/// Shared store handle. `None` until Timescale accepts a connection.
#[derive(Clone)]
pub struct AppState {
    pub store: Option<Store>,
}

/// Routes for the trace screen.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/traces", get(list_traces))
        .route("/api/traces/stream", get(stream_traces))
        .route("/api/faults/{name}", post(toggle_fault))
        .with_state(state)
}

async fn list_traces(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let events = trace_feed(store)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(
        json!({ "events": events.iter().map(event_json).collect::<Vec<_>>() }),
    ))
}

async fn stream_traces(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (sender, receiver) = mpsc::channel(32);
    tokio::spawn(async move {
        publish_loop(state.store, sender).await;
    });
    Sse::new(ReceiverStream::new(receiver))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn toggle_fault(Path(name): Path<String>) -> impl IntoResponse {
    toggle(&name).await
}

async fn publish_loop(mut store: Option<Store>, sender: mpsc::Sender<Result<Event, Infallible>>) {
    let mut cursor: Option<EventCursor> = None;
    let mut announced = false;
    let mut healthy = false;
    loop {
        match read_page(&mut store, cursor).await {
            Ok(events) => {
                healthy = true;
                if !announced {
                    if publish(&sender, hello(&events)).await.is_err() {
                        return;
                    }
                    announced = true;
                } else if !events.is_empty() {
                    for event in &events {
                        if publish(&sender, span_message(event)).await.is_err() {
                            return;
                        }
                    }
                }
                if let Some(last) = events.last() {
                    cursor = Some(EventCursor {
                        observed_at: last.observed_at,
                        id: last.id,
                    });
                }
            }
            Err(()) => {
                if healthy || !announced {
                    if publish(&sender, offline()).await.is_err() {
                        return;
                    }
                    announced = true;
                }
                healthy = false;
                store = None;
                cursor = None;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn read_page(
    store: &mut Option<Store>,
    cursor: Option<EventCursor>,
) -> Result<Vec<StoredEvent>, ()> {
    if store.is_none() {
        *store = Store::connect(&ledger_store::database_url()).await.ok();
    }
    let Some(connected) = store else {
        return Err(());
    };
    let result = match cursor {
        Some(cursor) => {
            connected
                .span_events_after((cursor.observed_at, cursor.id), PAGE)
                .await
        }
        None => trace_feed(connected).await,
    };
    result.map_err(|_| ())
}

/// Newest spans, plus earlier spans that share their `span_id` chain.
async fn trace_feed(store: &Store) -> Result<Vec<StoredEvent>, ledger_store::StoreError> {
    let seeds = store.recent_span_events(PAGE).await?;
    if seeds.is_empty() {
        return Ok(seeds);
    }
    let oldest = seeds[0].observed_at;
    let newest = seeds[seeds.len() - 1].observed_at;
    let start = oldest.checked_sub_signed(TRACE_LOOKBACK).unwrap_or(oldest);
    let end = newest + chrono::Duration::seconds(1);
    let neighborhood = store.span_events_between(start, end, 2_000).await?;
    Ok(assemble_feed(seeds, neighborhood))
}

/// Keeps `seeds` and any neighborhood span on the same parent chain.
fn assemble_feed(seeds: Vec<StoredEvent>, neighborhood: Vec<StoredEvent>) -> Vec<StoredEvent> {
    let mut included: BTreeMap<(DateTime<Utc>, Uuid), StoredEvent> = BTreeMap::new();
    for event in seeds {
        included.insert(event_key(&event), event);
    }
    let mut pool = included.clone();
    for event in neighborhood {
        pool.entry(event_key(&event)).or_insert(event);
    }
    let pool: Vec<StoredEvent> = pool.into_values().collect();

    for _ in 0..TRACE_HOPS {
        let span_ids = id_set(&included, "span_id");
        let parent_ids = id_set(&included, "parent_span_id");
        let mut grew = false;
        for event in &pool {
            let key = event_key(event);
            if included.contains_key(&key) {
                continue;
            }
            let span_id = payload_str(event, "span_id");
            let parent_id = payload_str(event, "parent_span_id");
            let linked = parent_id.is_some_and(|id| span_ids.contains(id))
                || span_id.is_some_and(|id| parent_ids.contains(id));
            if linked {
                included.insert(key, event.clone());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    let mut events: Vec<_> = included.into_values().collect();
    events.sort_by_key(event_key);
    events
}

fn event_key(event: &StoredEvent) -> (DateTime<Utc>, Uuid) {
    (event.observed_at, event.id)
}

fn id_set(events: &BTreeMap<(DateTime<Utc>, Uuid), StoredEvent>, key: &str) -> BTreeSet<String> {
    events
        .values()
        .filter_map(|event| payload_str(event, key).map(str::to_owned))
        .collect()
}

fn payload_str<'a>(event: &'a StoredEvent, key: &str) -> Option<&'a str> {
    event
        .payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn hello(events: &[StoredEvent]) -> Value {
    json!({
        "kind": "hello",
        "events": events.iter().map(event_json).collect::<Vec<_>>(),
    })
}

fn span_message(event: &StoredEvent) -> Value {
    json!({
        "kind": "span",
        "event": event_json(event),
    })
}

fn offline() -> Value {
    json!({ "kind": "offline" })
}

/// One hypertable row, with the column names the table uses.
fn event_json(event: &StoredEvent) -> Value {
    json!({
        "id": event.id.to_string(),
        "observed_at": event.observed_at.to_rfc3339(),
        "source": event.source,
        "kind": event.kind,
        "data_class": event.data_class,
        "payload": event.payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ledger_store::{DateTime, Utc, Uuid};
    use tower::ServiceExt;

    #[test]
    fn event_json_uses_hypertable_columns() {
        let event = StoredEvent {
            id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid"),
            observed_at: DateTime::<Utc>::from_timestamp(1_900_000_000, 0).expect("time"),
            source: "agent.payer".to_owned(),
            kind: "agent.decide".to_owned(),
            data_class: "tenant".to_owned(),
            payload: json!({
                "span": "agent.decide",
                "span_id": "7",
                "parent_span_id": "1",
                "adapted": false,
                "attributes": { "agent.id": "payer" }
            }),
        };
        let body = event_json(&event);
        assert_eq!(body["id"], "11111111-1111-4111-8111-111111111111");
        assert_eq!(body["source"], "agent.payer");
        assert_eq!(body["kind"], "agent.decide");
        assert_eq!(body["data_class"], "tenant");
        assert_eq!(body["payload"]["span"], "agent.decide");
        assert_eq!(body["payload"]["parent_span_id"], "1");
        assert!(body.get("trace_id").is_none());
    }

    #[tokio::test]
    async fn unknown_fault_is_rejected() {
        let response = router(AppState { store: None })
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/faults/not-a-fault")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn traces_without_a_database_are_unavailable() {
        let response = router(AppState { store: None })
            .oneshot(
                Request::builder()
                    .uri("/api/traces")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn assemble_feed_keeps_the_rest_of_a_submit_trace() {
        let at = DateTime::<Utc>::from_timestamp(1_900_000_000, 0).expect("time");
        let decide = linked("agent.decide", "1", None, at);
        let submit = linked(
            "exchange.submit_order",
            "2",
            Some("1"),
            at + chrono::Duration::milliseconds(1),
        );
        let matched = linked(
            "exchange.match",
            "3",
            Some("2"),
            at + chrono::Duration::milliseconds(2),
        );
        let fill = linked(
            "exchange.fill",
            "4",
            Some("3"),
            at + chrono::Duration::milliseconds(3),
        );
        let settled = linked(
            "exchange.settle",
            "5",
            Some("4"),
            at + chrono::Duration::milliseconds(4),
        );
        let agent_settled = linked(
            "agent.settle",
            "6",
            Some("1"),
            at + chrono::Duration::milliseconds(5),
        );
        let pay = linked(
            "agent.pay_bill",
            "7",
            Some("1"),
            at + chrono::Duration::milliseconds(6),
        );
        let unrelated = linked(
            "exchange.tick",
            "99",
            None,
            at + chrono::Duration::milliseconds(7),
        );
        let feed = assemble_feed(
            vec![submit.clone()],
            vec![
                decide,
                submit,
                matched,
                fill,
                settled,
                agent_settled,
                pay,
                unrelated,
            ],
        );
        let names: Vec<_> = feed
            .iter()
            .map(|event| event.payload["span"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            names,
            vec![
                "agent.decide",
                "exchange.submit_order",
                "exchange.match",
                "exchange.fill",
                "exchange.settle",
                "agent.settle",
                "agent.pay_bill",
            ]
        );
    }

    fn linked(span: &str, span_id: &str, parent: Option<&str>, at: DateTime<Utc>) -> StoredEvent {
        let mut payload = json!({
            "span": span,
            "span_id": span_id,
        });
        if let Some(parent) = parent {
            payload["parent_span_id"] = json!(parent);
        }
        StoredEvent {
            id: Uuid::new_v4(),
            observed_at: at,
            source: "exchange".to_owned(),
            kind: span.to_owned(),
            data_class: "tenant".to_owned(),
            payload,
        }
    }
}
