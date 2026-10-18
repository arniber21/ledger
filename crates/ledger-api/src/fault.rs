//! Fault toggles for the trace screen.
//!
//! A click calls the running desk over the same Unix socket as `just fault`.
//! The desk flips the fault and runs another cycle. This process does not
//! invent spans; ingest writes them when the cycle is traced.

use std::path::Path;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use ledger_agents::{toggle_running, AgentError, Fault, CONTROL_SOCKET};
use serde_json::json;

/// Toggles `name` on the running desk.
pub async fn toggle(name: &str) -> impl IntoResponse {
    let fault = match Fault::parse(name) {
        Ok(fault) => fault,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "unknown fault" })),
            )
                .into_response();
        }
    };
    let label = fault.as_str();
    let joined =
        tokio::task::spawn_blocking(move || toggle_running(Path::new(CONTROL_SOCKET), fault)).await;
    match joined {
        Ok(Ok(enabled)) => Json(json!({
            "name": label,
            "enabled": enabled,
        }))
        .into_response(),
        Ok(Err(AgentError::NotRunning)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Fault injection is not connected." })),
        )
            .into_response(),
        Ok(Err(_)) | Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Fault injection is not connected." })),
        )
            .into_response(),
    }
}
