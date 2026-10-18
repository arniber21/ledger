//! `POST /api/specs/author` runs the authoring pipeline for one policy sentence.
//!
//! The body is `{ "sentence": "...", "override": false }`. The reply is the
//! `text/event-stream` that `ledger-spec` encodes: `sentence`, `draft`,
//! `check`, `coverage`, `accepted`, `held`, `rejected`.
//!
//! Gemini drafts the Pattern (`GEMINI_API_KEY`) and Jev judges the match and
//! each clause (`TYPESAFE_API_KEY`). A missing key, a timeout, or an error is a
//! failed call, which `ledger-spec` turns into a rejection or an Unknown mark.
//! Nothing here fills in a draft or a verdict. Keys are read from the
//! environment and never written to a response or a log.
//!
//! The pipeline runs to the end and the frames are sent together.
//!
//! When `LEDGER_JEV=replay`, Gemini and Jev come from `fixtures/` through
//! `ledger_spec::author_replay` and nothing calls out. A missing fixture is a
//! failed call (rejection or Unknown). When `LEDGER_TEST=1`, a forced outcome
//! from `POST /api/test/jev` replaces the Jev fixtures for the next run.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use ledger_core::AgentId;
use ledger_jev::{ForcedOutcome, JevCallFailure, SystemOne, SystemOneClient};
use ledger_spec::{
    author, author_replay, coverage_request, draft_from_interaction, draft_request, match_request, split_clauses,
    AuthoringInput, DraftFailure,
};
use serde::Deserialize;

const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";
const GEMINI_TIMEOUT: Duration = Duration::from_secs(60);

/// Route for spec authoring.
pub fn router(force: Arc<Mutex<Option<ForcedOutcome>>>) -> Router {
    Router::new()
        .route("/api/specs/author", post(run))
        .with_state(force)
}

fn replay_mode() -> bool {
    std::env::var("LEDGER_JEV").is_ok_and(|mode| mode.trim() == "replay")
}

#[derive(Deserialize)]
struct Incoming {
    sentence: String,
    #[serde(default, rename = "override")]
    override_coverage: bool,
}

async fn run(
    State(force): State<Arc<Mutex<Option<ForcedOutcome>>>>,
    Json(body): Json<Incoming>,
) -> Response {
    let forced = *force.lock().unwrap_or_else(|error| error.into_inner());
    let frames = tokio::task::spawn_blocking(move || {
        if replay_mode() {
            replay_for(&body.sentence, body.override_coverage, forced)
        } else {
            stream_for(&body.sentence, body.override_coverage)
        }
    })
    .await;
    match frames {
        Ok(text) => ([(header::CONTENT_TYPE, "text/event-stream")], text).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn replay_for(sentence: &str, override_coverage: bool, forced: Option<ForcedOutcome>) -> String {
    let agent = AgentId::new("payer").expect("literal agent id");
    author_replay(sentence, &agent, override_coverage, forced).encode_sse()
}

/// Runs every call and returns the encoded frames. Blocking.
fn stream_for(sentence: &str, override_coverage: bool) -> String {
    let gemini = call_gemini(sentence);
    let jev_client = std::env::var("TYPESAFE_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .and_then(|key| SystemOneClient::new(key.trim()).ok());
    let call = |request: &str| -> Result<String, JevCallFailure> {
        match &jev_client {
            Some(client) => client.evaluate(request),
            None => Err(JevCallFailure::Transport),
        }
    };

    let draft = gemini
        .as_ref()
        .ok()
        .and_then(|body| draft_from_interaction(body));
    let (jev, coverage) = match &draft {
        Some(draft) => {
            let jev = call(&match_request(sentence, draft));
            let clauses = split_clauses(sentence);
            let coverage = if clauses.len() > 1 {
                call(&coverage_request(sentence, &clauses, draft))
            } else {
                Err(JevCallFailure::Transport)
            };
            (jev, coverage)
        }
        None => (
            Err(JevCallFailure::Transport),
            Err(JevCallFailure::Transport),
        ),
    };

    let agent = AgentId::new("payer").expect("literal agent id");
    let outcome = author(AuthoringInput {
        sentence,
        agent: &agent,
        gemini: gemini.as_deref().map_err(|failure| *failure),
        jev: jev.as_deref().map_err(|failure| *failure),
        coverage: coverage.as_deref().map_err(|failure| *failure),
        override_coverage,
    });
    outcome.encode_sse()
}

fn call_gemini(sentence: &str) -> Result<String, DraftFailure> {
    let key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    if key.trim().is_empty() {
        return Err(DraftFailure::Transport);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(GEMINI_TIMEOUT)
        .build()
        .map_err(|_| DraftFailure::Transport)?;
    let response = client
        .post(GEMINI_URL)
        .header("x-goog-api-key", key.trim())
        .header("content-type", "application/json")
        .body(draft_request(sentence))
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                DraftFailure::Timeout
            } else {
                DraftFailure::Transport
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(DraftFailure::Http {
            status: status.as_u16(),
        });
    }
    let bytes = response.bytes().map_err(|_| DraftFailure::Transport)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| DraftFailure::Utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_sentence_is_rejected_before_any_call() {
        let text = stream_for("   ", false);
        assert!(text.contains("event: rejected"));
        assert!(!text.contains("event: accepted"));
        assert!(!text.to_ascii_lowercase().contains("fraud"));
    }
}
