//! SSE server. Each `data` line is one JSON object the UI already parses.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures::stream::{self, Stream};
use ledger_jev::SystemOne;
use ledger_store::{Store, Uuid, VerdictRank};
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

/// Binds `addr` and, when `ranker` is set, scores verdicts that have no raw scores yet.
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
    let engine = tokio::spawn(async move {
        let mut engine = match Engine::with_ranker(engine_store, engine_tx, ranker) {
            Ok(engine) => engine,
            Err(error) => {
                tracing::error!(error = %error, "verdict engine failed to start");
                return;
            }
        };
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
    let app = verdicts
        .merge(crate::inbox::router(store.clone()))
        .merge(crate::traces::router(crate::traces::AppState {
            store: Some(store),
        }));
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
    let live =
        BroadcastStream::new(rx).filter_map(|item| item.ok().map(|frame| Ok(sse_event(&frame))));
    let replay = stream::iter(initial.into_iter().map(|frame| Ok(sse_event(&frame))));
    Ok(Sse::new(replay.chain(live)).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

fn sse_event(verdict: &StreamVerdict) -> Event {
    Event::default()
        .id(verdict.id.to_string())
        .json_data(&message(verdict))
        .unwrap_or_else(|_| Event::default().data("{}"))
}
