//! SSE helpers for the trace stream.

use std::convert::Infallible;

use axum::response::sse::Event;
use serde_json::Value;
use tokio::sync::mpsc;

/// Sends one JSON SSE message. The error means the browser closed the stream.
pub async fn publish(
    sender: &mpsc::Sender<Result<Event, Infallible>>,
    body: Value,
) -> Result<(), ()> {
    let event = Event::default().json_data(body).map_err(|_| ())?;
    sender.send(Ok(event)).await.map_err(|_| ())
}
