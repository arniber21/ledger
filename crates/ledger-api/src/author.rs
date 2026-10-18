//! `POST /api/specs/author` streams one authoring run.
//!
//! Gemini and Jev are replayed from fixtures. A missing fixture rejects the
//! draft or maps to Unknown. This route does not call out, and it does not
//! record a Sentry incident. When `LEDGER_TEST=1`, a forced outcome from
//! `POST /api/test/jev` replaces the Jev fixtures for this run.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use ledger_core::AgentId;
use ledger_jev::ForcedOutcome;
use ledger_spec::author_replay;
use serde::Deserialize;
use serde_json::json;

/// Routes the spec authoring stream.
pub fn router(force: Arc<Mutex<Option<ForcedOutcome>>>) -> Router {
    Router::new()
        .route("/api/specs/author", post(author_stream))
        .with_state(force)
}

#[derive(Deserialize)]
struct AuthorBody {
    sentence: String,
    #[serde(default, rename = "override")]
    override_coverage: bool,
}

async fn author_stream(
    State(force): State<Arc<Mutex<Option<ForcedOutcome>>>>,
    body: Result<Json<AuthorBody>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "sentence is required" })),
            )
                .into_response();
        }
    };
    let Ok(agent) = AgentId::new("payer") else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "authoring could not start" })),
        )
            .into_response();
    };
    let forced = *lock_force(&force);
    let outcome = author_replay(&body.sentence, &agent, body.override_coverage, forced);
    let sse = outcome.encode_sse();
    if sse.to_ascii_lowercase().contains("fraud") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "authoring could not start" })),
        )
            .into_response();
    }
    ([(header::CONTENT_TYPE, "text/event-stream")], sse).into_response()
}

fn lock_force(
    force: &Mutex<Option<ForcedOutcome>>,
) -> std::sync::MutexGuard<'_, Option<ForcedOutcome>> {
    force.lock().unwrap_or_else(|error| error.into_inner())
}
