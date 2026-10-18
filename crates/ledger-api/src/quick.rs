//! `POST /api/quick-actions` routes a phrase through Jev.
//!
//! The response is a typed action: Inbox filters, a screen, a verdict id that
//! was already supplied, or a fault name to confirm. A timeout, error, or
//! score under the fixed threshold is Unknown and becomes text search. This
//! handler does not toggle a fault and does not create a verdict.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use ledger_jev::{route_phrase, Candidates, QuickError, QuickReply, SystemOne, SystemOneClient};
use serde::Deserialize;
use serde_json::{json, Value};

/// How Quick actions reaches Jev.
///
/// A live key is only turned into a blocking client inside `spawn_blocking`.
/// Dropping that client on the async runtime panics.
#[derive(Clone)]
enum QuickClient {
    /// `TYPESAFE_API_KEY`. Not printed.
    Live(String),
    /// Test double. No network.
    #[cfg(test)]
    Stub(std::sync::Arc<dyn SystemOne + Send + Sync>),
    /// No key. The route is Unknown.
    Unavailable,
}

impl std::fmt::Debug for QuickClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Live(_) => formatter.write_str("Live"),
            #[cfg(test)]
            Self::Stub(_) => formatter.write_str("Stub"),
            Self::Unavailable => formatter.write_str("Unavailable"),
        }
    }
}

/// Route table for Quick actions.
///
/// `Some` injects a client, which tests use. `None` is an unavailable Jev.
#[cfg(test)]
pub fn router(client: Option<std::sync::Arc<dyn SystemOne + Send + Sync>>) -> Router {
    let client = match client {
        Some(client) => QuickClient::Stub(client),
        None => QuickClient::Unavailable,
    };
    Router::new()
        .route("/api/quick-actions", post(route))
        .with_state(client)
}

/// Route table that reads `TYPESAFE_API_KEY` when a phrase arrives.
///
/// An empty key is unavailable. The key is not written into [`Debug`](std::fmt::Debug).
pub fn router_from_env() -> Router {
    let client = match std::env::var("TYPESAFE_API_KEY") {
        Ok(raw) if !raw.trim().is_empty() => QuickClient::Live(raw.trim().to_owned()),
        _ => QuickClient::Unavailable,
    };
    Router::new()
        .route("/api/quick-actions", post(route))
        .with_state(client)
}

#[derive(Deserialize)]
struct Incoming {
    phrase: String,
    #[serde(default)]
    agents: Vec<String>,
    #[serde(default)]
    specs: Vec<String>,
    #[serde(default)]
    verdicts: Vec<String>,
}

async fn route(
    State(state): State<QuickClient>,
    Json(body): Json<Incoming>,
) -> Result<Json<QuickReply>, (StatusCode, Json<Value>)> {
    let candidates = Candidates {
        agents: body.agents,
        specs: body.specs,
        verdicts: body.verdicts,
    };
    let phrase = body.phrase;
    let state = state.clone();
    let joined = tokio::task::spawn_blocking(move || {
        let live = match &state {
            QuickClient::Live(key) => SystemOneClient::new(key).ok(),
            #[cfg(test)]
            QuickClient::Stub(_) | QuickClient::Unavailable => None,
            #[cfg(not(test))]
            QuickClient::Unavailable => None,
        };
        let client: Option<&dyn SystemOne> = match &state {
            #[cfg(test)]
            QuickClient::Stub(client) => Some(client.as_ref()),
            QuickClient::Live(_) => live.as_ref().map(|client| client as &dyn SystemOne),
            QuickClient::Unavailable => None,
        };
        route_phrase(&phrase, &candidates, client)
    })
    .await;
    match joined {
        Ok(Ok(reply)) => Ok(Json(reply)),
        Ok(Err(QuickError::Empty)) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "phrase is empty" })),
        )),
        Ok(Err(QuickError::TooLong)) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "phrase is too long" })),
        )),
        Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "quick actions stopped" })),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use ledger_jev::JevCallFailure;
    use tower::ServiceExt;

    struct Fixed {
        body: &'static str,
    }

    impl SystemOne for Fixed {
        fn evaluate(&self, request: &str) -> Result<String, JevCallFailure> {
            assert!(!request.contains("TYPESAFE_API_KEY"));
            assert!(!request.contains("Bearer"));
            assert!(!request.to_ascii_lowercase().contains("explain"));
            Ok(self.body.to_owned())
        }
    }

    async fn post(app: Router, phrase: &str) -> (StatusCode, String) {
        let response = app
            .oneshot(
                Request::post("/api/quick-actions")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"phrase":"{phrase}"}}"#)))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let text = String::from_utf8(bytes.to_vec()).expect("utf-8");
        (status, text)
    }

    #[tokio::test]
    async fn empty_phrase_is_rejected() {
        let (status, text) = post(router(None), "  ").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(text.contains("empty"));
        assert!(!text.contains("fraud"));
    }

    #[tokio::test]
    async fn unusable_body_is_unknown_text_search() {
        let client: Arc<dyn SystemOne + Send + Sync> = Arc::new(Fixed {
            body: r#"{"answers":{}}"#,
        });
        let (status, text) = post(router(Some(client)), "open verdict not-on-the-stream").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("\"status\":\"unknown\""));
        assert!(text.contains("\"kind\":\"text_search\""));
        assert!(!text.contains("not-on-the-stream-title"));
        assert!(!text.contains("fraud"));
        assert!(!text.contains("TYPESAFE_API_KEY"));
    }

    #[tokio::test]
    async fn missing_client_does_not_invent_a_route() {
        let (status, text) = post(router(None), "violations over $1,000 by agent-2").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("\"status\":\"unknown\""));
        assert!(text.contains("unavailable"));
        assert!(text.contains("\"kind\":\"text_search\""));
        assert!(!text.contains("fraud"));
    }
}
