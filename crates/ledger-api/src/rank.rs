//! Score one inbox verdict and keep the raw System One exchange.
//!
//! The four scores are stored. Weights are applied later, in the inbox, so a
//! slider does not call Jev. A call failure leaves every dimension unset.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ledger_jev::{
    map_ranking_body, map_ranking_exchange, ranking_request, DimensionScore, JevCallFailure,
    RankScore, RankStep, RankSubject, SystemOne,
};
use ledger_store::{Uuid, VerdictRank};
use serde_json::Value;

use crate::engine::{StreamRank, StreamVerdict};

/// Scores plus the stored request and response.
pub(crate) struct ScoredFrame {
    pub stream: StreamRank,
    pub money: Option<f64>,
    pub evidence: Option<f64>,
    pub ambiguity: Option<f64>,
    pub intent: Option<f64>,
    pub request: Value,
    pub response: Option<Value>,
}

impl ScoredFrame {
    pub(crate) fn row(&self, verdict_id: Uuid, recorded_at: DateTime<Utc>) -> VerdictRank {
        VerdictRank {
            verdict_id,
            recorded_at,
            request: self.request.clone(),
            response: self.response.clone(),
            money: self.money,
            evidence: self.evidence,
            ambiguity: self.ambiguity,
            intent: self.intent,
        }
    }
}

/// Asks Jev, off the async runtime, and maps the body with the fixed 0.80 rule.
pub(crate) async fn score_frame(ranker: Arc<dyn SystemOne>, frame: &StreamVerdict) -> ScoredFrame {
    let request = ranking_request(&subject_from(frame));
    let request_body = request.clone();
    let exchange = match tokio::task::spawn_blocking(move || ranker.evaluate(&request_body)).await {
        Ok(result) => result,
        Err(_) => Err(JevCallFailure::Transport),
    };
    scored_from_exchange(&request, exchange)
}

pub(crate) fn stream_rank_from_row(row: &VerdictRank) -> StreamRank {
    StreamRank {
        money: row.money,
        evidence: row.evidence,
        ambiguity: row.ambiguity,
        intent: row.intent,
        input: row.request.clone(),
        output: row.response.clone(),
    }
}

fn subject_from(frame: &StreamVerdict) -> RankSubject {
    RankSubject {
        outcome: frame.outcome.clone(),
        title: frame.title.clone(),
        spec_id: frame.spec_id.clone(),
        reason_code: frame.reason_code.clone(),
        binding: frame.binding.clone(),
        amount_cents: frame.amount_cents,
        evidence: frame.evidence.clone(),
        timeline: frame
            .timeline
            .iter()
            .map(|step| RankStep {
                source: step.source.clone(),
                kind: step.kind.clone(),
                role: step.role.clone(),
            })
            .collect(),
    }
}

fn scored_from_exchange(request: &str, exchange: Result<String, JevCallFailure>) -> ScoredFrame {
    let score = match &exchange {
        Ok(body) => map_ranking_body(body),
        Err(failure) => map_ranking_exchange(Err(*failure)),
    };
    let response = match exchange {
        Ok(body) => stored_response(&body),
        Err(failure) => Some(failure_value(failure)),
    };
    let request_value = serde_json::from_str(request).unwrap_or_else(|_| serde_json::json!({}));
    frame_from(score, request_value, response)
}

fn frame_from(score: RankScore, request: Value, response: Option<Value>) -> ScoredFrame {
    ScoredFrame {
        stream: StreamRank {
            money: raw(score.money),
            evidence: raw(score.evidence),
            ambiguity: raw(score.ambiguity),
            intent: raw(score.intent),
            input: request.clone(),
            output: response.clone(),
        },
        money: raw(score.money),
        evidence: raw(score.evidence),
        ambiguity: raw(score.ambiguity),
        intent: raw(score.intent),
        request,
        response,
    }
}

fn raw(score: DimensionScore) -> Option<f64> {
    score.raw()
}

fn stored_response(body: &str) -> Option<Value> {
    if body.contains("TYPESAFE_API_KEY")
        || body.contains("GEMINI_API_KEY")
        || body.contains("Bearer")
    {
        return Some(serde_json::json!({ "failure": "withheld" }));
    }
    let Ok(mut value) = serde_json::from_str::<Value>(body) else {
        return Some(serde_json::json!({ "failure": "unreadable" }));
    };
    strip_explanation(&mut value);
    Some(value)
}

fn failure_value(failure: JevCallFailure) -> Value {
    match failure {
        JevCallFailure::Timeout => serde_json::json!({ "failure": "timeout" }),
        JevCallFailure::Transport => serde_json::json!({ "failure": "transport" }),
        JevCallFailure::Http { status } => {
            serde_json::json!({ "failure": "http", "status": status })
        }
        JevCallFailure::Utf8 => serde_json::json!({ "failure": "utf8" }),
    }
}

fn strip_explanation(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| key != "explanation");
            for child in map.values_mut() {
                strip_explanation(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_explanation(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::StreamEntry;

    struct Timeout;

    impl SystemOne for Timeout {
        fn evaluate(&self, request: &str) -> Result<String, JevCallFailure> {
            assert!(!request.contains("TYPESAFE_API_KEY"));
            assert!(!request.to_ascii_lowercase().contains("fraud"));
            Err(JevCallFailure::Timeout)
        }
    }

    fn sample() -> StreamVerdict {
        StreamVerdict {
            id: Uuid::nil(),
            outcome: "Violate".to_owned(),
            title: "needs review".to_owned(),
            evidence: "payment pay → second-payment extra".to_owned(),
            spec_id: "ledger.bill.no-second-payment".to_owned(),
            binding: "bill-1".to_owned(),
            reason_code: Some("second-payment".to_owned()),
            amount_cents: Some(1_250),
            recorded_at: DateTime::from_timestamp(1_700_000_000, 0).expect("time"),
            timeline: vec![StreamEntry {
                event_id: "extra".to_owned(),
                observed_at: "2026-10-18T00:00:00Z".to_owned(),
                source: "exchange".to_owned(),
                kind: "ledger.exchange.payment".to_owned(),
                role: "second-payment".to_owned(),
            }],
            rest: Vec::new(),
            rerank: None,
            rank: None,
        }
    }

    #[tokio::test]
    async fn a_timeout_leaves_every_dimension_unknown() {
        let scored = score_frame(Arc::new(Timeout), &sample()).await;
        assert!(scored.money.is_none());
        assert!(scored.evidence.is_none());
        assert!(scored.ambiguity.is_none());
        assert!(scored.intent.is_none());
        assert_eq!(
            scored.stream.output,
            Some(serde_json::json!({ "failure": "timeout" }))
        );
        let rendered = serde_json::to_string(&scored.stream).expect("json");
        assert!(!rendered.to_lowercase().contains("fraud"));
        assert!(!rendered.contains("TYPESAFE_API_KEY"));
        assert_eq!(scored.request["state"]["verdict"]["title"], "needs review");
    }
}
