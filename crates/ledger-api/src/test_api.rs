//! Test routes mounted only when `LEDGER_TEST=1`.
//!
//! `POST /api/test/reset` truncates events, verdicts, triage, and ranks.
//! `POST /api/test/scenes/{name}` runs one scene with the `ledger-scene`
//! binary. `LEDGER_JEV=replay` unless the process was
//! started otherwise. `POST /api/test/jev` arms Same, Different, Unknown, or
//! timeout for the next scene. Timeout is a call failure. None of these
//! routes invent a Sentry incident or a live Jev body.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use ledger_jev::{map_binding_body, BindingVerdict, ForcedOutcome};
use ledger_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Scenes `ledger-scene` accepts, in run-of-show order.
const SCENES: [&str; 8] = [
    "clean",
    "double-pay",
    "no-approval",
    "hidden-memo",
    "vendor-alias",
    "recurring",
    "gc-soak",
    "spec-live",
];

const PROBE: &str = r#"{"questions":{"same_obligation":{"type":"choice"}}}"#;

const SCENE_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
struct TestApi {
    store: Option<Store>,
    scene_bin: PathBuf,
    force: Arc<Mutex<Option<ForcedOutcome>>>,
}

/// True when the process was started with `LEDGER_TEST=1`.
#[must_use]
pub fn enabled(flag: &str) -> bool {
    flag.trim() == "1"
}

/// Routes for reset, scenes, and a forced Jev outcome.
pub fn router(store: Option<Store>) -> Router {
    router_with_bin(store, scene_binary())
}

fn router_with_bin(store: Option<Store>, scene_bin: PathBuf) -> Router {
    let state = TestApi {
        store,
        scene_bin,
        force: Arc::new(Mutex::new(None)),
    };
    Router::new()
        .route("/api/test", get(status))
        .route("/api/test/reset", post(reset))
        .route("/api/test/scenes/{name}", post(run_scene))
        .route("/api/test/jev", post(force_jev))
        .with_state(state)
}

fn scene_binary() -> PathBuf {
    if let Ok(path) = std::env::var("LEDGER_SCENE_BIN") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let mut path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ledger-scene"));
    path.pop();
    path.push("ledger-scene");
    path
}

fn workspace_root() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    path.canonicalize().unwrap_or(path)
}

fn lock_force(
    force: &Mutex<Option<ForcedOutcome>>,
) -> std::sync::MutexGuard<'_, Option<ForcedOutcome>> {
    force.lock().unwrap_or_else(|error| error.into_inner())
}

async fn status(State(state): State<TestApi>) -> Json<Value> {
    let forced = lock_force(&state.force).map(ForcedOutcome::as_str);
    Json(json!({ "enabled": true, "forced": forced }))
}

async fn reset(State(state): State<TestApi>) -> axum::response::Response {
    let Some(store) = state.store.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "database is not connected" })),
        )
            .into_response();
    };
    if let Err(error) = store.truncate_runtime().await {
        tracing::warn!(error = %error, "test reset failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "reset failed" })),
        )
            .into_response();
    }
    *lock_force(&state.force) = None;
    Json(json!({ "reset": true })).into_response()
}

#[derive(Deserialize)]
struct ForceBody {
    outcome: String,
}

async fn force_jev(
    State(state): State<TestApi>,
    body: Result<Json<ForceBody>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "outcome must be Same, Different, Unknown, or timeout" })),
            )
                .into_response();
        }
    };
    let Some(outcome) = ForcedOutcome::parse(&body.outcome) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "outcome must be Same, Different, Unknown, or timeout" })),
        )
            .into_response();
    };
    *lock_force(&state.force) = Some(outcome);
    let (mapped, failure) = mapped_binding(outcome);
    Json(json!({
        "forced": outcome.as_str(),
        "outcome": mapped,
        "failure": failure,
    }))
    .into_response()
}

fn mapped_binding(outcome: ForcedOutcome) -> (&'static str, Option<&'static str>) {
    match outcome {
        ForcedOutcome::Timeout => ("Unknown", Some("timeout")),
        other => match other.apply(PROBE) {
            Ok(body) => (binding_name(map_binding_body(&body)), None),
            Err(_) => ("Unknown", Some("timeout")),
        },
    }
}

fn binding_name(verdict: BindingVerdict) -> &'static str {
    match verdict {
        BindingVerdict::Same => "Same",
        BindingVerdict::Different => "Different",
        BindingVerdict::Unknown => "Unknown",
    }
}

async fn run_scene(
    State(state): State<TestApi>,
    Path(name): Path<String>,
) -> axum::response::Response {
    if !SCENES.contains(&name.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unknown scene" })),
        )
            .into_response();
    }
    if !state.scene_bin.is_file() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "scene runner is not available" })),
        )
            .into_response();
    }
    let forced = *lock_force(&state.force);
    let mut command = Command::new(&state.scene_bin);
    command
        .arg(&name)
        .current_dir(workspace_root())
        .env("LEDGER_TEST", "1")
        .env(
            "LEDGER_JEV",
            std::env::var("LEDGER_JEV").unwrap_or_else(|_| "replay".to_owned()),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(outcome) = forced {
        command.env("LEDGER_JEV_FORCE", outcome.as_str());
    } else {
        command.env_remove("LEDGER_JEV_FORCE");
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(error = %error, "scene runner did not start");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "scene runner is not available" })),
            )
                .into_response();
        }
    };
    let finished = tokio::time::timeout(SCENE_TIMEOUT, child.wait()).await;
    let status = match finished {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "scene runner stopped");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "scene runner is not available" })),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(json!({ "error": "scene timed out" })),
            )
                .into_response();
        }
    };
    let stdout = read_pipe(child.stdout.as_mut()).await;
    let stderr = read_pipe(child.stderr.as_mut()).await;
    if !status.success() {
        let detail = safe_detail(&stderr, &stdout);
        let code = if detail.contains("unknown scene") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        return (code, Json(json!({ "error": detail }))).into_response();
    }
    Json(json!({
        "name": name,
        "summary": safe_detail(&stdout, ""),
    }))
    .into_response()
}

async fn read_pipe<R>(pipe: Option<&mut R>) -> String
where
    R: AsyncRead + Unpin,
{
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer).await;
    String::from_utf8_lossy(&buffer).into_owned()
}

fn safe_detail(primary: &str, fallback: &str) -> String {
    let line = last_line(primary).or_else(|| last_line(fallback));
    let Some(line) = line else {
        return "scene failed".to_owned();
    };
    let lower = line.to_ascii_lowercase();
    if lower.contains("api_key")
        || lower.contains("bearer")
        || lower.contains("sentry_dsn")
        || lower.contains("typesafe")
    {
        return "scene failed".to_owned();
    }
    if line.chars().count() > 400 {
        line.chars().take(400).collect()
    } else {
        line
    }
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn send(app: Router, request: Request<Body>) -> (StatusCode, String) {
        let response = app.oneshot(request).await.expect("response");
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

    fn app() -> Router {
        router_with_bin(None, PathBuf::from("/nonexistent/ledger-scene"))
    }

    #[test]
    fn test_routes_follow_the_flag() {
        assert!(enabled("1"));
        assert!(!enabled(""));
        assert!(!enabled("0"));
        assert!(!enabled("true"));
    }

    #[tokio::test]
    async fn force_reports_same_different_unknown_and_timeout() {
        let cases = [
            ("Same", "Same", "null"),
            ("Different", "Different", "null"),
            ("Unknown", "Unknown", "null"),
            ("timeout", "Unknown", "\"timeout\""),
        ];
        for (forced, outcome, failure) in cases {
            let (status, text) = send(
                app(),
                Request::post("/api/test/jev")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"outcome":"{forced}"}}"#)))
                    .expect("request"),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{text}");
            assert!(text.contains(&format!("\"forced\":\"{forced}\"")), "{text}");
            assert!(
                text.contains(&format!("\"outcome\":\"{outcome}\"")),
                "{text}"
            );
            assert!(text.contains(&format!("\"failure\":{failure}")), "{text}");
            assert!(!text.contains("fraud"));
            assert!(!text.contains("explanation"));
        }
    }

    #[tokio::test]
    async fn unknown_outcome_and_scene_are_rejected() {
        let (status, text) = send(
            app(),
            Request::post("/api/test/jev")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"outcome":"pass"}"#))
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!text.contains("fraud"));

        let (status, text) = send(
            app(),
            Request::post("/api/test/scenes/not-a-scene")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(text.contains("unknown scene"));
    }

    #[tokio::test]
    async fn reset_without_a_database_is_unavailable() {
        let (status, text) = send(
            app(),
            Request::post("/api/test/reset")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(text.contains("database"));
        assert!(!text.contains("fraud"));
    }

    #[tokio::test]
    async fn missing_scene_runner_is_unavailable() {
        let (status, text) = send(
            app(),
            Request::post("/api/test/scenes/clean")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(text.contains("scene runner"));
    }
}
