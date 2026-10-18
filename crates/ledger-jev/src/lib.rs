//! Jev (TypeSafe System One) client: binding judgments and the memo predicate.
//!
//! Deterministic guards on amount, bill id, vendor id, and currency run first
//! and skip Jev when they decide. Otherwise the model returns a typed Choice.
//! This crate maps that Choice onto [`BindingVerdict`] with the fixed 0.80
//! thresholds. The memo Noul maps onto [`PredicateVerdict`]. The same
//! thresholds map a credit or cancellation memo onto [`CloseVerdict`]. This
//! crate does not ask for or keep explanation text. [`BindingVerdict::Same`]
//! merges monitor keys. [`BindingVerdict::Different`] keeps them apart.
//! [`BindingVerdict::Unknown`] is inconclusive and does not merge or retire.

use serde_json::Value;
use thiserror::Error;

mod baseline;
mod client;
mod close;
mod guard;
mod matchers;
mod memo;
mod obligation;
mod request;
mod resolve;

pub use baseline::{baseline_cases, baseline_report, BaselineCase, BaselineReport, CaseComparison};
pub use client::{SystemOne, SystemOneClient, SYSTEM_ONE_URL};
pub use close::{
    close_memo_request, consider_alias, consider_close_memo, map_close_memo_body,
    map_close_memo_exchange, CloseVerdict, MEMO_CLOSES_QUESTION,
};
pub use guard::{guard, GuardDecision};
pub use matchers::{exact_match, fuzzy_match, FUZZY_NAME_THRESHOLD};
pub use memo::{
    map_memo_body, map_memo_exchange, memo_request, PredicateVerdict, BENIGN_MEMO,
    INSTRUCTION_MEMO, MEMO_INSTRUCTS_QUESTION,
};
pub use obligation::ObligationRef;
pub use request::binding_request;
pub use resolve::{resolve, BindingEffect, CallFault, Resolution};

/// Question id for the binding smoke call.
///
/// Ids are for this crate. The model sees [`binding_smoke_request`]'s
/// instructions and criteria, not this string.
pub const SAME_OBLIGATION_QUESTION: &str = "same_obligation";

/// Client-side limit for one System One call.
///
/// A call that exceeds this is [`JevCallFailure::Timeout`], which maps to
/// [`BindingVerdict::Unknown`].
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

// Thresholds written before the live System One call saved at
// `fixtures/jev/same-obligation.json`. These numbers were not changed after
// that response was read. Probabilities are scores, not calibrated confidence.
//
// A two-option Choice splits evenly at 0.5. 0.80 means the winning option
// holds four times the other option's probability. Below that, on timeout or
// error, and on any contradiction, the verdict is Unknown. Unknown does not
// merge monitor keys.

/// Minimum `same` probability that maps to [`BindingVerdict::Same`].
pub const SAME_PROBABILITY_THRESHOLD: f64 = 0.80;

/// Minimum `different` probability that maps to [`BindingVerdict::Different`].
pub const DIFFERENT_PROBABILITY_THRESHOLD: f64 = 0.80;

/// Absolute tolerance for a probability pair against 1, and for a Noul
/// against a complementary `probabilities` object.
pub(crate) const PROBABILITY_SUM_TOLERANCE: f64 = 1e-3;

/// Whether two records are the same obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[must_use]
pub enum BindingVerdict {
    /// The records are one obligation.
    Same,
    /// The records are two obligations.
    Different,
    /// Timeout, error, low score, or contradiction. Not a pass.
    Unknown,
}

/// Why a System One call produced no usable body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum JevCallFailure {
    /// The call exceeded [`CALL_TIMEOUT`].
    #[error("jev call timed out")]
    Timeout,
    /// The call failed before an HTTP status arrived.
    #[error("jev call failed")]
    Transport,
    /// TypeSafe returned a non-success status.
    #[error("jev returned HTTP {status}")]
    Http {
        /// HTTP status code.
        status: u16,
    },
    /// The response body was not UTF-8.
    #[error("jev response was not valid UTF-8")]
    Utf8,
}

/// Body of the one live binding call (`model` `jev-latest`).
#[must_use]
pub fn binding_smoke_request() -> &'static str {
    include_str!("../smoke_request.json")
}

/// Map a raw System One Choice body for one question id.
///
/// Invalid JSON, a missing or non-choice answer, a malformed distribution,
/// a score below [`SAME_PROBABILITY_THRESHOLD`] / [`DIFFERENT_PROBABILITY_THRESHOLD`],
/// and a contradiction all return [`BindingVerdict::Unknown`].
/// Any explanation field on the body is ignored. Thresholds are the same ones
/// [`map_binding_body`] uses.
#[must_use = "an unmapped Choice is dropped"]
pub fn map_choice_body(body: &str, question_id: &str) -> BindingVerdict {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return BindingVerdict::Unknown;
    };
    map_choice_value(&value, question_id)
}

/// Map a raw System One response body to a binding verdict.
///
/// This is [`map_choice_body`] for [`SAME_OBLIGATION_QUESTION`].
#[must_use = "an unmapped Choice is dropped"]
pub fn map_binding_body(body: &str) -> BindingVerdict {
    map_choice_body(body, SAME_OBLIGATION_QUESTION)
}

/// Map a finished Choice call for `question_id`.
///
/// Every [`JevCallFailure`] becomes [`BindingVerdict::Unknown`].
#[must_use = "an unmapped Choice is dropped"]
pub fn map_choice_exchange(
    exchange: Result<&str, JevCallFailure>,
    question_id: &str,
) -> BindingVerdict {
    match exchange {
        Ok(body) => map_choice_body(body, question_id),
        Err(
            JevCallFailure::Timeout
            | JevCallFailure::Transport
            | JevCallFailure::Http { .. }
            | JevCallFailure::Utf8,
        ) => BindingVerdict::Unknown,
    }
}

/// Map a finished binding call. Every [`JevCallFailure`] becomes [`BindingVerdict::Unknown`].
#[must_use = "an unmapped Choice is dropped"]
pub fn map_binding_exchange(exchange: Result<&str, JevCallFailure>) -> BindingVerdict {
    map_choice_exchange(exchange, SAME_OBLIGATION_QUESTION)
}

fn map_choice_value(value: &Value, question_id: &str) -> BindingVerdict {
    let Some(answer) = value
        .get("answers")
        .and_then(|answers| answers.get(question_id))
    else {
        return BindingVerdict::Unknown;
    };
    if answer.get("type").and_then(Value::as_str) != Some("choice") {
        return BindingVerdict::Unknown;
    }
    let Some(choice) = answer.get("choice").and_then(Value::as_str) else {
        return BindingVerdict::Unknown;
    };
    let Some(probabilities) = answer.get("probabilities").and_then(Value::as_object) else {
        return BindingVerdict::Unknown;
    };
    if probabilities.len() != 2 {
        return BindingVerdict::Unknown;
    }
    let Some(p_same) = probabilities.get("same").and_then(Value::as_f64) else {
        return BindingVerdict::Unknown;
    };
    let Some(p_different) = probabilities.get("different").and_then(Value::as_f64) else {
        return BindingVerdict::Unknown;
    };
    decide(choice, p_same, p_different)
}

fn decide(choice: &str, p_same: f64, p_different: f64) -> BindingVerdict {
    if !p_same.is_finite() || !p_different.is_finite() {
        return BindingVerdict::Unknown;
    }
    if !(0.0..=1.0).contains(&p_same) || !(0.0..=1.0).contains(&p_different) {
        return BindingVerdict::Unknown;
    }

    let same_clears = p_same >= SAME_PROBABILITY_THRESHOLD;
    let different_clears = p_different >= DIFFERENT_PROBABILITY_THRESHOLD;
    // Both options clearing is a contradiction even when the two probabilities
    // do not form a distribution.
    if same_clears && different_clears {
        return BindingVerdict::Unknown;
    }

    let sum = p_same + p_different;
    if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
        return BindingVerdict::Unknown;
    }

    match choice {
        "same" if same_clears && p_same > p_different => BindingVerdict::Same,
        "different" if different_clears && p_different > p_same => BindingVerdict::Different,
        "same" | "different" => BindingVerdict::Unknown,
        _ => BindingVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(answer: Value) -> String {
        serde_json::json!({
            "model": "jev-test",
            "answers": { SAME_OBLIGATION_QUESTION: answer }
        })
        .to_string()
    }

    fn choice(label: &str, p_same: f64, p_different: f64) -> Value {
        serde_json::json!({
            "type": "choice",
            "choice": label,
            "probabilities": { "same": p_same, "different": p_different },
            "confidence": 0.5
        })
    }

    #[test]
    fn same_at_threshold_maps_to_same() {
        let raw = body(choice("same", 0.80, 0.20));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Same);
    }

    #[test]
    fn different_at_threshold_maps_to_different() {
        let raw = body(choice("different", 0.20, 0.80));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Different);
    }

    #[test]
    fn probability_below_threshold_maps_to_unknown() {
        let raw = body(choice("same", 0.79, 0.21));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn choice_that_disagrees_with_probabilities_is_unknown() {
        let raw = body(choice("same", 0.10, 0.90));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn both_options_clearing_thresholds_is_unknown() {
        let raw = body(choice("same", 0.90, 0.90));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn distribution_that_does_not_sum_to_one_is_unknown() {
        let raw = body(choice("same", 0.50, 0.20));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn probability_outside_unit_interval_is_unknown() {
        let raw = body(choice("same", 1.20, -0.20));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn explanation_text_is_ignored() {
        let mut answer = choice("different", 0.05, 0.95);
        answer["explanation"] = Value::String("model text that must not be used".to_string());
        assert_eq!(map_binding_body(&body(answer)), BindingVerdict::Different);
    }

    #[test]
    fn invalid_json_is_unknown() {
        assert_eq!(map_binding_body("not json"), BindingVerdict::Unknown);
    }

    #[test]
    fn missing_answer_is_unknown() {
        let raw = serde_json::json!({ "model": "jev-test", "answers": {} }).to_string();
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn non_choice_answer_is_unknown() {
        let raw = body(serde_json::json!({ "type": "noul", "noul": 0.99 }));
        assert_eq!(map_binding_body(&raw), BindingVerdict::Unknown);
    }

    #[test]
    fn call_failures_map_to_unknown() {
        let failures = [
            JevCallFailure::Timeout,
            JevCallFailure::Transport,
            JevCallFailure::Http { status: 422 },
            JevCallFailure::Utf8,
        ];
        for failure in failures {
            assert_eq!(
                map_binding_exchange(Err(failure)),
                BindingVerdict::Unknown,
                "{failure}"
            );
        }
    }

    #[test]
    fn smoke_request_asks_one_choice_and_for_no_explanation() {
        let request: Value = serde_json::from_str(binding_smoke_request()).expect("request JSON");
        assert_eq!(request["model"], "jev-latest");
        let questions = request["questions"].as_object().expect("questions");
        assert_eq!(questions.len(), 1);
        let question = &questions[SAME_OBLIGATION_QUESTION];
        assert_eq!(question["type"], "choice");
        let criteria = question["criteria"].as_object().expect("criteria");
        assert!(criteria.contains_key("same"));
        assert!(criteria.contains_key("different"));
        assert!(request.get("explanation").is_none());
        assert!(question.get("explanation").is_none());
        assert!(!binding_smoke_request()
            .to_ascii_lowercase()
            .contains("explain"));
    }

    #[test]
    fn another_question_id_uses_the_same_thresholds() {
        let raw = serde_json::json!({
            "model": "jev-test",
            "answers": {
                "draft_matches_policy": choice("same", 0.80, 0.20)
            }
        })
        .to_string();
        assert_eq!(
            map_choice_body(&raw, "draft_matches_policy"),
            BindingVerdict::Same
        );
        assert_eq!(
            map_choice_body(&raw, SAME_OBLIGATION_QUESTION),
            BindingVerdict::Unknown
        );
        assert_eq!(
            map_choice_exchange(Err(JevCallFailure::Timeout), "draft_matches_policy"),
            BindingVerdict::Unknown
        );
    }

    #[test]
    fn saved_fixture_replays_through_mapper_offline() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/jev/same-obligation.json");
        let raw = std::fs::read_to_string(&path).expect("saved jev fixture");
        let value: Value = serde_json::from_str(&raw).expect("fixture json");
        assert_eq!(value["model"], "jev-1.13.0");
        assert!(!raw.contains("Bearer"));
        assert!(!raw.contains("TYPESAFE_API_KEY"));
        // Reads the saved response only. No socket is opened.
        assert_eq!(map_binding_body(&raw), BindingVerdict::Same);
        assert_eq!(map_binding_exchange(Ok(&raw)), BindingVerdict::Same);
    }
}
