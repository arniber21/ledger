//! Calibration table. Counts come from stored review events joined to stored
//! Jev scores. The 0.80 thresholds are the constants in `ledger-jev`. This
//! module does not change them.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use ledger_jev::SAME_PROBABILITY_THRESHOLD;
use ledger_store::{CalibrationRow, Store};
use serde_json::{json, Value};

/// Route for the calibration table.
pub fn router(store: Store) -> Router {
    Router::new()
        .route("/api/calibration", get(list_calibration))
        .with_state(store)
}

async fn list_calibration(State(store): State<Store>) -> Result<Json<Value>, StatusCode> {
    let rows = store.calibration().await.map_err(|error| {
        tracing::warn!(error = %error, "calibration read failed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    Ok(Json(calibration_json(&rows)))
}

fn calibration_json(rows: &[CalibrationRow]) -> Value {
    json!({
        "threshold": SAME_PROBABILITY_THRESHOLD,
        "rows": rows.iter().map(row_json).collect::<Vec<_>>(),
    })
}

fn row_json(row: &CalibrationRow) -> Value {
    json!({
        "question": row.question,
        "band": row.band,
        "confirmed": row.confirmed,
        "dismissed": row.dismissed,
        "reviewed": row.reviewed,
    })
}

#[cfg(test)]
mod tests {
    use ledger_jev::DIFFERENT_PROBABILITY_THRESHOLD;

    use super::*;

    #[test]
    fn calibration_json_keeps_counts_and_the_fixed_threshold() {
        assert_eq!(
            serde_json::json!(SAME_PROBABILITY_THRESHOLD),
            serde_json::json!(DIFFERENT_PROBABILITY_THRESHOLD)
        );
        let body = calibration_json(&[CalibrationRow {
            question: "same_obligation".to_owned(),
            band: "unknown".to_owned(),
            confirmed: 1,
            dismissed: 0,
            reviewed: 1,
        }]);
        assert_eq!(
            body["threshold"],
            serde_json::json!(SAME_PROBABILITY_THRESHOLD)
        );
        assert_eq!(body["rows"][0]["band"], "unknown");
        assert_eq!(body["rows"][0]["confirmed"], 1);
        assert_eq!(body["rows"][0]["dismissed"], 0);
        assert_eq!(body["rows"][0]["reviewed"], 1);
        let text = body.to_string();
        assert!(!text.to_ascii_lowercase().contains("fraud"));
        assert!(!text.contains("\"Satisfy\""));
    }

    #[test]
    fn empty_calibration_is_an_empty_table() {
        let body = calibration_json(&[]);
        assert_eq!(body["rows"], json!([]));
        assert_eq!(
            body["threshold"],
            serde_json::json!(SAME_PROBABILITY_THRESHOLD)
        );
    }
}
