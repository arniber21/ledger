//! Live trace read API.
//!
//! Rows come from the `events` hypertable (`id`, `observed_at`, `source`,
//! `kind`, `data_class`, `payload`). `payload` is the object written by
//! `ledger-ingest`: `span`, `span_id`, `parent_span_id`, `started_at`,
//! `closed_at`, `attributes`, `causal_links`, `adapted`, and, when present,
//! `sentinel`, `evidence`, and `reason`. There is no trace-id column. The UI
//! groups rows by `payload.parent_span_id`.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use ledger_store::{Event as StoredEvent, EventCursor, Store};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::fault::toggle;
use crate::stream::publish;

/// How many hypertable rows a snapshot or a poll returns.
const PAGE: i64 = 200;

/// Rows observed after this are not traces. A bad clock must not fill the feed
/// or move the stream cursor out of reach of real rows.
fn ceiling() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::hours(1)
}

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
    let events = store
        .recent_events_until(ceiling(), PAGE)
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
                .events_after_until((cursor.observed_at, cursor.id), ceiling(), PAGE)
                .await
        }
        None => connected.recent_events_until(ceiling(), PAGE).await,
    };
    result.map_err(|_| ())
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
}
