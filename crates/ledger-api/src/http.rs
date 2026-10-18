//! SSE server. Each `data` line is one JSON object the UI already parses.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures::stream::{self, Stream};
use ledger_jev::{ForcedOutcome, SystemOne};
use ledger_store::{Event as StoredEvent, Store, Uuid, VerdictRank};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::engine::{message, verdict_from_stored, ApiError, Engine, StreamVerdict};

#[derive(Clone)]
struct App {
    store: Store,
    tx: broadcast::Sender<StreamVerdict>,
}

/// A running verdict API.
pub struct Server {
    /// Bound address.
    pub addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}

impl Server {
    /// Waits until the process is asked to stop.
    pub async fn wait(mut self) {
        let _ = tokio::signal::ctrl_c().await;
        let _ = self.shutdown.send(true);
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

/// Binds `addr` and polls the events table until shutdown.
///
/// # Errors
///
/// Returns an error when the address cannot be bound or the session cannot start.
pub async fn serve(store: Store, addr: &str) -> Result<Server, ApiError> {
    serve_with_ranker(store, addr, None).await
}

/// Binds `addr` and, when `ranker` is set, scores verdicts and reranks violating traces.
///
/// # Errors
///
/// Returns an error when the address cannot be bound or the session cannot start.
pub async fn serve_with_ranker(
    store: Store,
    addr: &str,
    ranker: Option<Arc<dyn SystemOne>>,
) -> Result<Server, ApiError> {
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    let (tx, _) = broadcast::channel(1024);
    let (shutdown, mut watch_rx) = watch::channel(false);
    let engine_store = store.clone();
    let engine_tx = tx.clone();
    let monitor_stats = crate::metrics::MonitorStats::default();
    let engine_stats = monitor_stats.clone();
    let engine = tokio::spawn(async move {
        let mut engine = match Engine::with_ranker(engine_store, engine_tx, ranker) {
            Ok(engine) => engine,
            Err(error) => {
                tracing::error!(error = %error, "verdict engine failed to start");
                return;
            }
        };
        engine.report_monitors(engine_stats);
        if let Err(error) = engine.score_unranked().await {
            tracing::warn!(error = %error, "could not score stored verdicts");
        }
        loop {
            if *watch_rx.borrow() {
                break;
            }
            if let Err(error) = engine.poll_once().await {
                tracing::warn!(error = %error, "verdict poll failed");
            }
            // Scenes store verdicts on their own. Score those rows after startup
            // so the inbox can reorder from stored scores without another call.
            if let Err(error) = engine.score_unranked().await {
                tracing::warn!(error = %error, "could not score stored verdicts");
            }
            tokio::select! {
                changed = watch_rx.changed() => {
                    if changed.is_err() || *watch_rx.borrow() {
                        break;
                    }
                }
                () = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
        }
    });
    let verdicts = Router::new()
        .route("/health", get(health))
        .route("/v1/verdicts/stream", get(stream))
        .with_state(App {
            store: store.clone(),
            tx,
        });
    let force = Arc::new(Mutex::new(None::<ForcedOutcome>));
    let mut app = verdicts
        .merge(crate::inbox::router(store.clone()))
        .merge(crate::calibration::router(store.clone()))
        .merge(crate::metrics::router(store.clone(), monitor_stats))
        .merge(crate::specs::router())
        .merge(crate::traces::router(crate::traces::AppState {
            store: Some(store.clone()),
        }))
        .merge(crate::quick::router_from_env())
        .merge(crate::author::router(force.clone()));
    if crate::test_api::enabled(&std::env::var("LEDGER_TEST").unwrap_or_default()) {
        app = app.merge(crate::test_api::router_with_force(Some(store), force));
    }
    let mut stop = shutdown.subscribe();
    let join = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = stop.changed().await;
        });
        if let Err(error) = server.await {
            tracing::error!(error = %error, "verdict API stopped");
        }
    });
    // The engine task is aborted with the server task's parent. Keep it alive
    // by detaching it onto the same shutdown watch; abort both on drop.
    let join = tokio::spawn(async move {
        tokio::select! {
            _ = engine => {}
            _ = join => {}
        }
    });
    Ok(Server {
        addr: bound,
        shutdown,
        join: Some(join),
    })
}

async fn health() -> &'static str {
    "ok"
}

async fn stream(
    State(app): State<App>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, axum::http::StatusCode> {
    let mut rx = app.tx.subscribe();
    let past = app.store.list_verdicts(1_000).await.map_err(|error| {
        tracing::warn!(error = %error, "could not replay verdicts");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let ranks = app.store.list_ranks().await.map_err(|error| {
        tracing::warn!(error = %error, "could not replay ranks");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let ranks: HashMap<Uuid, VerdictRank> =
        ranks.into_iter().map(|row| (row.verdict_id, row)).collect();
    let mut initial = Vec::new();
    for verdict in past.into_iter().rev() {
        if let Some(mut frame) = verdict_from_stored(&verdict) {
            if let Some(row) = ranks.get(&verdict.id) {
                frame.rank = Some(crate::rank::stream_rank_from_row(row));
            }
            initial.push(frame);
        }
    }
    while let Ok(frame) = rx.try_recv() {
        if let Some(existing) = initial.iter_mut().find(|existing| existing.id == frame.id) {
            if existing.rank.is_none() && frame.rank.is_some() {
                *existing = frame;
            }
        } else {
            initial.push(frame);
        }
    }
    let mut replayed = initial
        .into_iter()
        .map(|frame| sse_event(&frame))
        .collect::<Vec<_>>();
    replayed.extend(scene_events(&app.store).await);
    let live =
        BroadcastStream::new(rx).filter_map(|item| item.ok().map(|frame| Ok(sse_event(&frame))));
    // A leading comment flushes the response headers and first bytes at once,
    // so an idle stream (no verdicts yet) is visibly open to clients and proxies.
    let hello = stream::once(async { Ok(Event::default().comment("connected")) });
    let replay = stream::iter(replayed.into_iter().map(Ok));
    Ok(Sse::new(hello.chain(replay).chain(live))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

async fn scene_events(store: &Store) -> Vec<Event> {
    let mut events = Vec::new();
    let start = chrono::DateTime::from_timestamp(1_578_322_800, 0).expect("scene epoch");
    let end = start + chrono::Duration::days(40);
    if let Ok(rows) = store
        .events_by_kind_between("scene.counters", start, end, 50)
        .await
    {
        for row in rows {
            if let Some(event) = counters_event(&row) {
                events.push(event);
            }
        }
    }
    if let Ok(rows) = store
        .events_by_kind_between("scene.rates", start, end, 200)
        .await
    {
        for row in rows {
            if let Some(event) = rates_event(&row) {
                events.push(event);
            }
        }
    }
    if let Ok(rows) = store
        .events_by_kind_between("scene.monitors", start, end, 400)
        .await
    {
        for row in rows {
            if let Some(event) = monitors_event(&row) {
                events.push(event);
            }
        }
    }
    events
}

fn counters_event(row: &StoredEvent) -> Option<Event> {
    let ingested = attr_u64(row, "eventsIngested")?;
    let sources = attr_u64(row, "sourceCount").unwrap_or(0);
    let live = attr_u64(row, "liveMonitors").unwrap_or(0);
    json_event(&json!({
        "kind": "counters",
        "counters": {
            "eventsIngested": ingested,
            "sourceCount": sources,
            "liveMonitors": live
        }
    }))
}

fn rates_event(row: &StoredEvent) -> Option<Event> {
    json_event(&json!({
        "kind": "rates",
        "point": {
            "t": row.observed_at.timestamp_millis(),
            "agentLog": attr_u64(row, "agentLog").unwrap_or(0),
            "exchange": attr_u64(row, "exchange").unwrap_or(0),
            "solana": attr_u64(row, "solana").unwrap_or(0)
        }
    }))
}

fn monitors_event(row: &StoredEvent) -> Option<Event> {
    json_event(&json!({
        "kind": "monitors",
        "point": {
            "t": row.observed_at.timestamp_millis(),
            "withGc": attr_u64(row, "withGc")?,
            "withoutGc": attr_u64(row, "withoutGc")?
        }
    }))
}

fn attr_u64(row: &StoredEvent, key: &str) -> Option<u64> {
    row.payload
        .pointer("/attributes")
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .and_then(|text| text.parse().ok())
}

fn json_event(value: &serde_json::Value) -> Option<Event> {
    Event::default().json_data(value).ok()
}

fn sse_event(verdict: &StreamVerdict) -> Event {
    Event::default()
        .id(verdict.id.to_string())
        .json_data(&message(verdict))
        .unwrap_or_else(|_| Event::default().data("{}"))
}
