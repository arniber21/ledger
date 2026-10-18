//! Rerank a violating trace so the counterexample is the minimal set.
//!
//! One Noul per candidate event, batched in one System One request. Mapping
//! uses [`classify_noul`] and the fixed 0.80 thresholds. Selection only: this
//! module does not ask for or keep explanation text.
//!
//! The events Sentinel marked are always in the shown set. A score under either
//! threshold is [`Relevance::Unknown`] for that event, and an unknown event
//! stays shown: unknown is not a decision to leave it out. A timeout, error,
//! contradiction, or malformed answer makes the ranking
//! [`Counterexample::Full`]: the whole trace, in the order given.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::memo::{classify_noul, NoulClass};
use crate::JevCallFailure;

/// One candidate event. Fields are the short record the question sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Ledger event id. It is also the Sentinel event id.
    pub id: String,
    /// Timeline role, such as `payment` or `second-payment`.
    pub role: String,
    /// Sentinel event kind.
    pub kind: String,
    /// `exchange` or `agent.{id}`.
    pub source: String,
}

impl Candidate {
    /// Builds one candidate.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        role: impl Into<String>,
        kind: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            role: role.into(),
            kind: kind.into(),
            source: source.into(),
        }
    }
}

/// The violated spec the questions are about.
#[derive(Debug, Clone, Copy)]
pub struct ViolatedSpec<'a> {
    /// Specification id.
    pub id: &'a str,
    /// Stable reason code. Never model prose.
    pub reason: &'a str,
}

/// Whether one event belongs in the minimal counterexample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Relevance {
    /// `P(yes)` is at or above [`SAME_PROBABILITY_THRESHOLD`](crate::SAME_PROBABILITY_THRESHOLD).
    Relevant,
    /// `P(no)` is at or above [`crate::DIFFERENT_PROBABILITY_THRESHOLD`].
    NotRelevant,
    /// Under threshold, contradiction, or a malformed answer. Not a pass.
    Unknown,
}

/// One mapped event. `noul` is the raw yes-probability when the answer had one.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredEvent {
    /// Candidate id.
    pub id: String,
    /// Mapped relevance. [`Relevance::Unknown`] is not "leave it out".
    pub relevance: Relevance,
    /// Yes-probability from the answer, when it was a finite number.
    pub noul: Option<f64>,
}

/// Which events the timeline shows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Counterexample {
    /// `shown` is the minimal set. `rest` is behind the expand control.
    ///
    /// Both lists keep the candidate order, which is observation order.
    Selected {
        /// Shown by default. Includes every matched id that was a candidate.
        shown: Vec<String>,
        /// Not relevant, and not matched evidence.
        rest: Vec<String>,
    },
    /// Ranking failed. Every candidate, in the order given.
    Full {
        /// The whole trace.
        events: Vec<String>,
    },
}

/// Why the ranking did not select a minimal set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum RankFailure {
    /// The call exceeded the client timeout.
    Timeout,
    /// The call failed before an HTTP status arrived.
    Transport,
    /// TypeSafe returned a non-success status.
    Http,
    /// The response body was not UTF-8.
    Utf8,
    /// An answer disagreed with itself.
    Contradiction,
    /// A score sat between the fixed 0.80 thresholds.
    BelowThreshold,
    /// The body or an answer could not be mapped.
    Unusable,
}

impl RankFailure {
    /// Stable reason code stored with the verdict. Not model prose.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::Http => "http",
            Self::Utf8 => "utf8",
            Self::Contradiction => "contradiction",
            Self::BelowThreshold => "below-threshold",
            Self::Unusable => "unusable",
        }
    }
}

/// Mapped ranking. `failure` is set when the timeline must show the full trace.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranking {
    /// Shown events, or the full trace.
    pub counterexample: Counterexample,
    /// One row per candidate. Empty when the call itself failed.
    pub scores: Vec<ScoredEvent>,
    /// Why the ranking is Unknown. `None` when a minimal set was selected.
    pub failure: Option<RankFailure>,
}

/// The double-pay trace the saved fixture reranks.
///
/// Observation order. `again` and `again:close` are the two events a minimal
/// absence seal marks. `pay` is the earlier payment. The other three are
/// context.
#[must_use]
pub fn double_pay_candidates() -> Vec<Candidate> {
    vec![
        Candidate::new("pay", "payment", "ledger.agent.pay_bill", "agent.payer"),
        Candidate::new("tick-1", "tick", "ledger.exchange.tick", "exchange"),
        Candidate::new(
            "settled",
            "settlement",
            "ledger.exchange.settled",
            "exchange",
        ),
        Candidate::new(
            "order-9",
            "order",
            "ledger.exchange.order_submit",
            "exchange",
        ),
        Candidate::new(
            "again",
            "second-payment",
            "ledger.exchange.payment",
            "exchange",
        ),
        Candidate::new(
            "again:close",
            "credit",
            "ledger.exchange.credit",
            "exchange",
        ),
    ]
}

/// Witness ids for [`double_pay_candidates`].
#[must_use]
pub fn double_pay_matched() -> Vec<String> {
    vec!["again".to_owned(), "again:close".to_owned()]
}

/// Spec the double-pay fixture asks about.
#[must_use]
pub const fn double_pay_spec() -> ViolatedSpec<'static> {
    ViolatedSpec {
        id: "ledger.bill.no-second-payment",
        reason: "second-payment",
    }
}

/// Body of one batched rerank call.
///
/// One Noul per event. The body has no API key and does not ask for an
/// explanation. Object keys are sorted so the saved fixture stays stable.
#[must_use]
pub fn rerank_request(spec: ViolatedSpec<'_>, matched: &[String], events: &[Candidate]) -> String {
    let evidence: Vec<&Candidate> = events
        .iter()
        .filter(|event| matched.iter().any(|id| id == &event.id))
        .collect();
    let mut questions = Map::new();
    for index in 0..events.len() {
        questions.insert(question_id(index), question(index));
    }
    let value = sort_objects(serde_json::json!({
        "state": {
            "spec": {
                "id": spec.id,
                "reason": spec.reason
            },
            "evidence": evidence.iter().map(|event| event_state(event)).collect::<Vec<_>>(),
            "events": events.iter().map(event_state).collect::<Vec<_>>()
        },
        "model": "jev-latest",
        "questions": questions
    }));
    serde_json::to_string(&value).expect("rerank request is finite JSON")
}

/// Map a finished rerank call.
///
/// Every [`JevCallFailure`] yields [`Counterexample::Full`] in candidate order.
#[must_use]
pub fn rank_exchange(
    events: &[Candidate],
    matched: &[String],
    exchange: Result<&str, JevCallFailure>,
) -> Ranking {
    match exchange {
        Ok(body) => rank_body(events, matched, body),
        Err(JevCallFailure::Timeout) => failed(events, RankFailure::Timeout),
        Err(JevCallFailure::Transport) => failed(events, RankFailure::Transport),
        Err(JevCallFailure::Http { .. }) => failed(events, RankFailure::Http),
        Err(JevCallFailure::Utf8) => failed(events, RankFailure::Utf8),
    }
}

/// Map a raw System One body.
///
/// A contradiction or a malformed answer yields [`Counterexample::Full`].
/// A score under the fixed thresholds marks that event
/// [`Relevance::Unknown`] and keeps it in the shown set. Any explanation
/// field is ignored.
#[must_use]
pub fn rank_body(events: &[Candidate], matched: &[String], body: &str) -> Ranking {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return failed(events, RankFailure::Unusable);
    };
    let Some(answers) = value.get("answers").and_then(Value::as_object) else {
        return failed(events, RankFailure::Unusable);
    };
    let mut scores = Vec::with_capacity(events.len());
    let mut failure: Option<RankFailure> = None;
    for (index, event) in events.iter().enumerate() {
        let class = answers
            .get(&question_id(index))
            .map(classify_noul)
            .unwrap_or(NoulClass::Unusable);
        let noul = answers
            .get(&question_id(index))
            .and_then(|answer| answer.get("noul"))
            .and_then(Value::as_f64)
            .filter(|score| score.is_finite());
        failure = worse(failure, failure_of(class));
        scores.push(ScoredEvent {
            id: event.id.clone(),
            relevance: relevance_of(class),
            noul,
        });
    }
    if let Some(failure) = failure {
        return Ranking {
            counterexample: Counterexample::Full {
                events: ids(events),
            },
            scores,
            failure: Some(failure),
        };
    }
    Ranking {
        counterexample: select(events, matched, &scores),
        scores,
        failure: None,
    }
}

fn select(events: &[Candidate], matched: &[String], scores: &[ScoredEvent]) -> Counterexample {
    let mut shown = Vec::new();
    let mut rest = Vec::new();
    for event in events {
        let score = scores.iter().find(|score| score.id == event.id);
        let marked = matched.iter().any(|id| id == &event.id);
        // NotRelevant is the only score that may leave an event out, and only
        // when Sentinel did not mark it. Unknown stays visible.
        let hide = score.is_some_and(|score| score.relevance == Relevance::NotRelevant) && !marked;
        if hide {
            rest.push(event.id.clone());
        } else {
            shown.push(event.id.clone());
        }
    }
    if shown.is_empty() {
        return Counterexample::Full {
            events: ids(events),
        };
    }
    Counterexample::Selected { shown, rest }
}

fn failed(events: &[Candidate], failure: RankFailure) -> Ranking {
    Ranking {
        counterexample: Counterexample::Full {
            events: ids(events),
        },
        scores: Vec::new(),
        failure: Some(failure),
    }
}

fn failure_of(class: NoulClass) -> Option<RankFailure> {
    match class {
        NoulClass::Yes | NoulClass::No | NoulClass::BelowThreshold => None,
        NoulClass::Contradiction => Some(RankFailure::Contradiction),
        NoulClass::Unusable => Some(RankFailure::Unusable),
    }
}

fn worse(current: Option<RankFailure>, next: Option<RankFailure>) -> Option<RankFailure> {
    match (current, next) {
        (None, other) | (other, None) => other,
        (Some(left), Some(right)) => Some(if rank(left) >= rank(right) {
            left
        } else {
            right
        }),
    }
}

fn rank(failure: RankFailure) -> u8 {
    match failure {
        RankFailure::Contradiction => 3,
        RankFailure::BelowThreshold => 2,
        RankFailure::Unusable => 1,
        RankFailure::Timeout | RankFailure::Transport | RankFailure::Http | RankFailure::Utf8 => 0,
    }
}

fn relevance_of(class: NoulClass) -> Relevance {
    match class {
        NoulClass::Yes => Relevance::Relevant,
        NoulClass::No => Relevance::NotRelevant,
        NoulClass::BelowThreshold | NoulClass::Contradiction | NoulClass::Unusable => {
            Relevance::Unknown
        }
    }
}

fn ids(events: &[Candidate]) -> Vec<String> {
    events.iter().map(|event| event.id.clone()).collect()
}

fn question_id(index: usize) -> String {
    format!("event_{index}")
}

fn question(index: usize) -> Value {
    serde_json::json!({
        "type": "noul",
        "instructions": format!(
            "Is `events[{index}]` part of the minimal counterexample for the violated `spec`, given `evidence`?"
        ),
        "criteria": {
            "true": "This event is needed to see the violation. It is one of `evidence`, or it is the earlier payment on the same bill that a second payment repeats. The minimal counterexample is that earlier payment, the second payment, and the credit that bounds them.",
            "false": "This event is not needed to see the violation. A settlement notice, a tick, or an order is not that earlier payment, the second payment, or the bounding credit."
        }
    })
}

fn event_state(event: &Candidate) -> Value {
    serde_json::json!({
        "id": event.id,
        "role": event.role,
        "kind": event.kind,
        "source": event.source
    })
}

fn sort_objects(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                sorted.insert(key, sort_objects(child));
            }
            let mut object = Map::new();
            for (key, child) in sorted {
                object.insert(key, child);
            }
            Value::Object(object)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_objects).collect()),
        other => other,
    }
}

/// Removes explanation fields. The model is not asked for them, and a body
/// that contains one is stored without that text.
pub fn strip_explanation(value: &mut Value) {
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

    fn events() -> Vec<Candidate> {
        double_pay_candidates()
    }

    fn matched() -> Vec<String> {
        double_pay_matched()
    }

    fn noul(probability: f64) -> Value {
        serde_json::json!({ "type": "noul", "noul": probability })
    }

    fn body(scores: &[f64]) -> String {
        let mut answers = Map::new();
        for (index, score) in scores.iter().enumerate() {
            answers.insert(question_id(index), noul(*score));
        }
        serde_json::json!({ "model": "jev-test", "answers": answers }).to_string()
    }

    fn selected(ranking: &Ranking) -> (&[String], &[String]) {
        match &ranking.counterexample {
            Counterexample::Selected { shown, rest } => (shown, rest),
            Counterexample::Full { events } => panic!("expected a selection, got {events:?}"),
        }
    }

    #[test]
    fn request_asks_one_noul_per_event_and_for_no_explanation() {
        let raw = rerank_request(double_pay_spec(), &matched(), &events());
        let request: Value = serde_json::from_str(&raw).expect("request JSON");
        assert_eq!(request["model"], "jev-latest");
        assert_eq!(request["state"]["spec"]["id"], double_pay_spec().id);
        assert_eq!(request["state"]["spec"]["reason"], "second-payment");
        let evidence = request["state"]["evidence"].as_array().expect("evidence");
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0]["id"], "again");
        assert_eq!(evidence[1]["id"], "again:close");
        let questions = request["questions"].as_object().expect("questions");
        assert_eq!(questions.len(), events().len());
        for (index, _) in events().iter().enumerate() {
            let question = &questions[&question_id(index)];
            assert_eq!(question["type"], "noul");
            assert!(question["criteria"]["true"].is_string());
            assert!(question["criteria"]["false"].is_string());
            assert!(question.get("explanation").is_none());
        }
        assert!(request.get("explanation").is_none());
        assert!(!raw.to_ascii_lowercase().contains("explain"));
        assert!(!raw.contains("TYPESAFE_API_KEY"));
        assert!(!raw.contains("Bearer"));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/jev/rerank-double-pay-request.json");
        let saved = std::fs::read_to_string(&path).expect("saved request");
        assert_eq!(raw, saved);
    }

    #[test]
    fn saved_double_pay_fixture_shows_three_key_events_offline() {
        // Live call on 2026-10-18, model jev-1.13.0: 139 ms, 1469 input
        // tokens, 112 output tokens. Thresholds were already 0.80 and were
        // not changed after this response.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev");
        let response =
            std::fs::read_to_string(root.join("rerank-double-pay.json")).expect("fixture");
        let value: Value = serde_json::from_str(&response).expect("fixture json");
        assert_eq!(value["model"], "jev-1.13.0");
        assert!(!response.contains("Bearer"));
        assert!(!response.contains("TYPESAFE_API_KEY"));
        assert!(!response.to_ascii_lowercase().contains("explain"));
        let ranking = rank_exchange(&events(), &matched(), Ok(response.as_str()));
        assert_eq!(ranking.failure, None);
        let (shown, rest) = selected(&ranking);
        assert_eq!(
            shown,
            &[
                "pay".to_owned(),
                "again".to_owned(),
                "again:close".to_owned()
            ]
        );
        assert_eq!(
            rest,
            &[
                "tick-1".to_owned(),
                "settled".to_owned(),
                "order-9".to_owned()
            ]
        );
        assert_eq!(ranking.scores[4].relevance, Relevance::Unknown);
        assert_eq!(ranking.scores[5].relevance, Relevance::Unknown);
    }

    #[test]
    fn decisive_scores_keep_the_three_key_events_and_collapse_the_rest() {
        // pay yes, tick no, settled no, order no, again yes, close yes.
        let ranking = rank_body(
            &events(),
            &matched(),
            &body(&[0.91, 0.05, 0.08, 0.04, 0.96, 0.93]),
        );
        assert_eq!(ranking.failure, None);
        let (shown, rest) = selected(&ranking);
        assert_eq!(
            shown,
            &[
                "pay".to_owned(),
                "again".to_owned(),
                "again:close".to_owned()
            ]
        );
        assert_eq!(
            rest,
            &[
                "tick-1".to_owned(),
                "settled".to_owned(),
                "order-9".to_owned()
            ]
        );
    }

    #[test]
    fn matched_evidence_stays_shown_when_its_score_says_not_relevant() {
        // again and again:close are confidently not relevant, but Sentinel marked them.
        let ranking = rank_body(
            &events(),
            &matched(),
            &body(&[0.05, 0.05, 0.05, 0.05, 0.05, 0.05]),
        );
        let (shown, rest) = selected(&ranking);
        assert_eq!(shown, &["again".to_owned(), "again:close".to_owned()]);
        assert!(rest.iter().any(|id| id == "pay"));
        assert!(!format!("{ranking:?}")
            .to_ascii_lowercase()
            .contains("fraud"));
    }

    #[test]
    fn a_score_under_the_threshold_stays_shown_and_is_unknown() {
        let mut scores = [0.95, 0.05, 0.05, 0.05, 0.95, 0.95];
        scores[2] = 0.50;
        let ranking = rank_body(&events(), &matched(), &body(&scores));
        assert_eq!(ranking.failure, None);
        assert_eq!(ranking.scores[2].relevance, Relevance::Unknown);
        let (shown, rest) = selected(&ranking);
        assert_eq!(
            shown,
            &[
                "pay".to_owned(),
                "settled".to_owned(),
                "again".to_owned(),
                "again:close".to_owned()
            ]
        );
        assert_eq!(rest, &["tick-1".to_owned(), "order-9".to_owned()]);
    }

    #[test]
    fn a_contradiction_shows_the_full_trace() {
        let mut answers = Map::new();
        for index in 0..events().len() {
            answers.insert(question_id(index), noul(0.95));
        }
        answers.insert(
            question_id(0),
            serde_json::json!({
                "type": "noul",
                "noul": 0.95,
                "probabilities": { "true": 0.95, "false": 0.95 }
            }),
        );
        let raw = serde_json::json!({ "model": "jev-test", "answers": answers }).to_string();
        let ranking = rank_body(&events(), &matched(), &raw);
        assert_eq!(ranking.failure, Some(RankFailure::Contradiction));
        assert!(matches!(
            ranking.counterexample,
            Counterexample::Full { .. }
        ));
    }

    #[test]
    fn explanation_text_is_ignored() {
        let mut value: Value =
            serde_json::from_str(&body(&[0.91, 0.05, 0.08, 0.04, 0.96, 0.93])).expect("body");
        value["answers"]["event_0"]["explanation"] =
            Value::String("model text that must not be used".to_owned());
        value["explanation"] = Value::String("also unused".to_owned());
        let raw = value.to_string();
        let ranking = rank_body(&events(), &matched(), &raw);
        assert_eq!(ranking.failure, None);
        let (shown, _) = selected(&ranking);
        assert_eq!(shown[0], "pay");
        strip_explanation(&mut value);
        assert!(!value.to_string().to_ascii_lowercase().contains("explain"));
    }

    #[test]
    fn call_failures_show_the_full_trace_and_are_unknown() {
        let failures = [
            (JevCallFailure::Timeout, RankFailure::Timeout),
            (JevCallFailure::Transport, RankFailure::Transport),
            (JevCallFailure::Http { status: 422 }, RankFailure::Http),
            (JevCallFailure::Utf8, RankFailure::Utf8),
        ];
        let order: Vec<String> = events().iter().map(|event| event.id.clone()).collect();
        for (failure, code) in failures {
            let ranking = rank_exchange(&events(), &matched(), Err(failure));
            assert_eq!(ranking.failure, Some(code), "{failure}");
            assert!(ranking.scores.is_empty());
            assert_eq!(
                ranking.counterexample,
                Counterexample::Full {
                    events: order.clone()
                }
            );
        }
    }

    #[test]
    fn invalid_json_is_the_full_trace() {
        let ranking = rank_body(&events(), &matched(), "not json");
        assert_eq!(ranking.failure, Some(RankFailure::Unusable));
        assert!(matches!(
            ranking.counterexample,
            Counterexample::Full { .. }
        ));
    }
}
