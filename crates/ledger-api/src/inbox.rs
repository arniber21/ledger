//! Inbox triage. Status is stored next to verdicts and each change emits
//! a `verdict.review` event.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use ledger_store::{Store, Triage, TriageStatus, TriageWrite};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// Routes for inbox status.
pub fn router(store: Store) -> Router {
    Router::new()
        .route("/api/inbox/triage", get(list_triage).post(set_triage))
        .with_state(store)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TriageBody {
    verdict_id: Uuid,
    status: String,
    #[serde(default)]
    assignee: Option<String>,
}

async fn list_triage(State(store): State<Store>) -> Result<Json<Value>, StatusCode> {
    let rows = store
        .list_triage()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({
        "items": rows.iter().map(triage_json).collect::<Vec<_>>(),
    })))
}

async fn set_triage(State(store): State<Store>, Json(body): Json<TriageBody>) -> impl IntoResponse {
    let Some(status) = TriageStatus::parse(&body.status) else {
        return (StatusCode::BAD_REQUEST, "unknown status").into_response();
    };
    match store
        .record_triage(body.verdict_id, status, body.assignee.as_deref(), "me")
        .await
    {
        Ok(TriageWrite::Stored(change)) => Json(triage_json(&change.triage)).into_response(),
        Ok(TriageWrite::UnknownVerdict) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "triage write failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

fn triage_json(row: &Triage) -> Value {
    json!({
        "verdictId": row.verdict_id.to_string(),
        "status": row.status.as_str(),
        "assignee": row.assignee,
        "updatedAt": row.updated_at.to_rfc3339(),
        "updatedBy": row.updated_by,
    })
}
