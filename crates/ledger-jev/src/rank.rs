//! Four Score questions for one inbox verdict, combined in code.
//!
//! Money at stake, evidence strength, binding ambiguity, and repeat-versus-one-off
//! are separate Score questions in one System One request. Each raw score is
//! kept only when one level's probability is at least
//! [`crate::SAME_PROBABILITY_THRESHOLD`] (0.80). That cutoff was fixed for the
//! binding call and is not retuned here. A lower probability, a timeout, an
//! error, or a contradiction is [`DimensionScore::Unknown`].
//!
//! Unknown is not a pass and not a violation. A decisive score of 0 is a real
//! score: the threshold is on the level probability, not on the level number.
//! [`priority`] normalizes each raw score by [`RANK_TOP_LEVEL`] and applies
//! caller weights. Any Unknown dimension yields no priority, so the inbox can
//! fall back to severity order. This module does not ask for or keep
//! explanation text.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::{JevCallFailure, SAME_PROBABILITY_THRESHOLD};

/// Question id for money at stake.
///
/// The id is for this crate. The model sees the question instructions.
pub const MONEY_QUESTION: &str = "money_at_stake";

/// Question id for evidence strength.
pub const EVIDENCE_QUESTION: &str = "evidence_strength";

/// Question id for binding ambiguity.
pub const AMBIGUITY_QUESTION: &str = "binding_ambiguity";

/// Question id for a repeated action versus a one-off.
pub const INTENT_QUESTION: &str = "intent_pattern";

/// Highest level number on each ranking scale. Levels are `0..=RANK_TOP_LEVEL`.
pub const RANK_TOP_LEVEL: u32 = 4;

const RANK_LEVELS: usize = 5;

/// How far a reported score may sit from the probability-weighted level sum.
///
/// Fixed before the live ranking call. This is rounding room, not a second
/// decision threshold.
const SCORE_AGREEMENT_TOLERANCE: f64 = 0.02;

/// One evidence step sent with a verdict. Short fields only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankStep {
    /// `exchange` or `agent.{id}`.
    pub source: String,
    /// Event kind.
    pub kind: String,
    /// Role such as `payment` or `second-payment`.
    pub role: String,
}

/// The verdict fields the inbox already shows.
///
/// This is not a new incident. Callers pass a verdict the inbox has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankSubject {
    /// `Satisfy`, `Violate`, or `Inconclusive`.
    pub outcome: String,
    /// Inbox title. A violate is `needs review`.
    pub title: String,
    /// Specification id.
    pub spec_id: String,
    /// Stable reason code, when the verdict has one.
    pub reason_code: Option<String>,
    /// Bill id or order id.
    pub binding: String,
    /// Payment amount in cents, when the evidence carried one.
    pub amount_cents: Option<i64>,
    /// Evidence line the inbox shows.
    pub evidence: String,
    /// Timeline roles, in observation order.
    pub timeline: Vec<RankStep>,
}

/// One stored Score, or Unknown.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub enum DimensionScore {
    /// Raw Score on `0..=RANK_TOP_LEVEL`. Not a verdict.
    Scored(f64),
    /// Timeout, error, a level probability under 0.80, or a contradiction.
    ///
    /// Not a pass and not a violation.
    Unknown,
}

impl DimensionScore {
    /// Returns the raw score when this dimension was kept.
    #[must_use]
    pub const fn raw(self) -> Option<f64> {
        match self {
            Self::Scored(value) => Some(value),
            Self::Unknown => None,
        }
    }
}

/// The four raw scores for one verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RankScore {
    /// Money at stake.
    pub money: DimensionScore,
    /// Strength of the listed evidence.
    pub evidence: DimensionScore,
    /// Ambiguity of the binding.
    pub ambiguity: DimensionScore,
    /// How much the evidence looks repeated rather than one-off.
    pub intent: DimensionScore,
}

impl RankScore {
    /// Every dimension Unknown.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            money: DimensionScore::Unknown,
            evidence: DimensionScore::Unknown,
            ambiguity: DimensionScore::Unknown,
            intent: DimensionScore::Unknown,
        }
    }
}

/// Weights applied to normalized scores. The inbox sliders own these numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankWeights {
    /// Weight for [`RankScore::money`].
    pub money: f64,
    /// Weight for [`RankScore::evidence`].
    pub evidence: f64,
    /// Weight for [`RankScore::ambiguity`].
    pub ambiguity: f64,
    /// Weight for [`RankScore::intent`].
    pub intent: f64,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            money: 0.25,
            evidence: 0.25,
            ambiguity: 0.25,
            intent: 0.25,
        }
    }
}

impl RankWeights {
    fn usable(self) -> bool {
        let weights = [self.money, self.evidence, self.ambiguity, self.intent];
        weights
            .into_iter()
            .all(|weight| weight.is_finite() && weight >= 0.0)
            && weights.into_iter().sum::<f64>() > 0.0
    }
}

/// The no-second-payment verdict the inbox already shows.
///
/// Outcome, title, spec, and reason match `ledger.bill.no-second-payment`.
/// The amount is the 1250 cents the stream test records for that fault.
/// Ids are stable so the saved request does not change between runs.
#[must_use]
pub fn no_second_payment_subject() -> RankSubject {
    RankSubject {
        outcome: "Violate".to_owned(),
        title: "needs review".to_owned(),
        spec_id: "ledger.bill.no-second-payment".to_owned(),
        reason_code: Some("second-payment".to_owned()),
        binding: "bill-1".to_owned(),
        amount_cents: Some(1_250),
        evidence: "payment pay → settlement settled → second-payment extra".to_owned(),
        timeline: vec![
            RankStep {
                source: "agent.payer".to_owned(),
                kind: "ledger.agent.pay_bill".to_owned(),
                role: "payment".to_owned(),
            },
            RankStep {
                source: "exchange".to_owned(),
                kind: "ledger.exchange.settled".to_owned(),
                role: "settlement".to_owned(),
            },
            RankStep {
                source: "exchange".to_owned(),
                kind: "ledger.exchange.payment".to_owned(),
                role: "second-payment".to_owned(),
            },
        ],
    }
}

/// Body of one System One request: four Score questions, no explanation.
///
/// The body has no API key.
#[must_use]
pub fn ranking_request(subject: &RankSubject) -> String {
    let value = sort_objects(serde_json::json!({
        "state": subject_state(subject),
        "model": "jev-latest",
        "questions": {
            MONEY_QUESTION: score_question(
                "How much money is at stake on `verdict`?",
                [
                    "No payment amount is named.",
                    "The named payment is under 100 dollars.",
                    "The named payment is at least 100 dollars and under 1000 dollars.",
                    "The named payment is at least 1000 dollars and under 10000 dollars.",
                    "The named payment is at least 10000 dollars.",
                ],
            ),
            EVIDENCE_QUESTION: score_question(
                "How strong is the evidence on `verdict`?",
                [
                    "No evidence event is listed.",
                    "One evidence event is listed, and nothing else agrees with it.",
                    "Two evidence events are listed, and they describe different actions.",
                    "Two or more evidence events describe the same action.",
                    "The evidence events agree on the actor, the bill, and the action.",
                ],
            ),
            AMBIGUITY_QUESTION: score_question(
                "How ambiguous is the binding on `verdict`?",
                [
                    "One bill id is named, and every evidence event uses that id.",
                    "One bill id is named, and the evidence adds a vendor or invoice that matches it.",
                    "One bill id is named, and the evidence also names a second bill or vendor.",
                    "The evidence names two bill ids, and either one could be the binding.",
                    "No bill id is named.",
                ],
            ),
            INTENT_QUESTION: score_question(
                "How much does `verdict` look like a repeated action rather than a one-off?",
                [
                    "The evidence shows one action and no earlier action on the same bill.",
                    "The evidence shows one action after a different kind of action.",
                    "The evidence shows the same action twice, and also a reversal or a correction.",
                    "The evidence shows the same action twice on the same bill, with no reversal between them.",
                    "The evidence shows the same action three or more times, or the same action after a settlement.",
                ],
            ),
        }
    }));
    serde_json::to_string(&value).expect("ranking request is finite JSON")
}

/// Map a raw System One body. Each factor is independent.
///
/// Invalid JSON, a missing or non-score answer, a probability under
/// [`SAME_PROBABILITY_THRESHOLD`], and a contradiction are
/// [`DimensionScore::Unknown`] for that factor only. Any explanation field
/// is ignored.
#[must_use = "an unmapped Score is dropped"]
pub fn map_ranking_body(body: &str) -> RankScore {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return RankScore::unknown();
    };
    RankScore {
        money: map_factor(&value, MONEY_QUESTION),
        evidence: map_factor(&value, EVIDENCE_QUESTION),
        ambiguity: map_factor(&value, AMBIGUITY_QUESTION),
        intent: map_factor(&value, INTENT_QUESTION),
    }
}

/// Map a finished ranking call.
///
/// Every [`JevCallFailure`] makes every dimension [`DimensionScore::Unknown`].
#[must_use = "an unmapped Score is dropped"]
pub fn map_ranking_exchange(exchange: Result<&str, JevCallFailure>) -> RankScore {
    match exchange {
        Ok(body) => map_ranking_body(body),
        Err(
            JevCallFailure::Timeout
            | JevCallFailure::Transport
            | JevCallFailure::Http { .. }
            | JevCallFailure::Utf8,
        ) => RankScore::unknown(),
    }
}

/// Weighted sum of the four normalized scores.
///
/// Each raw score is divided by [`RANK_TOP_LEVEL`] so the weights mean the
/// same thing on every dimension. Returns `None` when any dimension is
/// Unknown, a weight is negative or non-finite, or the weights sum to zero.
/// `None` is not a priority of zero.
#[must_use]
pub fn priority(score: &RankScore, weights: RankWeights) -> Option<f64> {
    if !weights.usable() {
        return None;
    }
    let money = score.money.raw()?;
    let evidence = score.evidence.raw()?;
    let ambiguity = score.ambiguity.raw()?;
    let intent = score.intent.raw()?;
    let top = f64::from(RANK_TOP_LEVEL);
    Some(
        weights.money * money / top
            + weights.evidence * evidence / top
            + weights.ambiguity * ambiguity / top
            + weights.intent * intent / top,
    )
}

fn score_question(instructions: &str, criteria: [&str; RANK_LEVELS]) -> Value {
    serde_json::json!({
        "type": "score",
        "instructions": instructions,
        "criteria": criteria,
    })
}

fn subject_state(subject: &RankSubject) -> Value {
    let mut verdict = Map::new();
    verdict.insert("outcome".to_owned(), Value::String(subject.outcome.clone()));
    verdict.insert("title".to_owned(), Value::String(subject.title.clone()));
    verdict.insert("spec_id".to_owned(), Value::String(subject.spec_id.clone()));
    if let Some(reason) = &subject.reason_code {
        verdict.insert("reason_code".to_owned(), Value::String(reason.clone()));
    }
    verdict.insert("binding".to_owned(), Value::String(subject.binding.clone()));
    if let Some(cents) = subject.amount_cents {
        verdict.insert("amount".to_owned(), Value::String(amount_text(cents)));
    }
    verdict.insert(
        "evidence".to_owned(),
        Value::String(subject.evidence.clone()),
    );
    let steps = subject
        .timeline
        .iter()
        .take(8)
        .map(|step| {
            serde_json::json!({
                "source": step.source,
                "kind": step.kind,
                "role": step.role,
            })
        })
        .collect();
    verdict.insert("timeline".to_owned(), Value::Array(steps));
    serde_json::json!({ "verdict": Value::Object(verdict) })
}

fn amount_text(cents: i64) -> String {
    let negative = cents < 0;
    let magnitude = match cents.checked_abs() {
        Some(value) => u64::try_from(value).unwrap_or(0),
        None => 1_u64 << 63,
    };
    let dollars = magnitude / 100;
    let remainder = magnitude % 100;
    if negative {
        format!("-{dollars}.{remainder:02}")
    } else {
        format!("{dollars}.{remainder:02}")
    }
}

fn map_factor(value: &Value, question_id: &str) -> DimensionScore {
    let Some(answer) = value
        .get("answers")
        .and_then(|answers| answers.get(question_id))
    else {
        return DimensionScore::Unknown;
    };
    if answer.get("type").and_then(Value::as_str) != Some("score") {
        return DimensionScore::Unknown;
    }
    let Some(score) = answer.get("score").and_then(Value::as_f64) else {
        return DimensionScore::Unknown;
    };
    if !score.is_finite() || !(0.0..=f64::from(RANK_TOP_LEVEL)).contains(&score) {
        return DimensionScore::Unknown;
    }
    let Some(probabilities) = answer.get("probabilities").and_then(Value::as_object) else {
        return DimensionScore::Unknown;
    };
    if probabilities.len() != RANK_LEVELS {
        return DimensionScore::Unknown;
    }
    let mut expected = 0.0;
    let mut sum = 0.0;
    let mut max_probability = 0.0;
    for level in 0..=RANK_TOP_LEVEL {
        let Some(probability) = probabilities
            .get(&level.to_string())
            .and_then(Value::as_f64)
        else {
            return DimensionScore::Unknown;
        };
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return DimensionScore::Unknown;
        }
        sum += probability;
        expected += f64::from(level) * probability;
        if probability > max_probability {
            max_probability = probability;
        }
    }
    if (sum - 1.0).abs() > crate::PROBABILITY_SUM_TOLERANCE {
        return DimensionScore::Unknown;
    }
    if (expected - score).abs() > SCORE_AGREEMENT_TOLERANCE {
        return DimensionScore::Unknown;
    }
    if max_probability < SAME_PROBABILITY_THRESHOLD {
        return DimensionScore::Unknown;
    }
    DimensionScore::Scored(score)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemOne;

    fn expected_peaked(level: u32) -> f64 {
        f64::from(level) * 0.80
            + (0..=RANK_TOP_LEVEL)
                .filter(|index| *index != level)
                .map(|index| f64::from(index) * 0.05)
                .sum::<f64>()
    }

    fn peaked(level: u32) -> Value {
        let mut probabilities = Map::new();
        for index in 0..RANK_LEVELS {
            let number = u32::try_from(index).expect("level");
            let probability = if number == level { 0.80 } else { 0.05 };
            probabilities.insert(number.to_string(), serde_json::json!(probability));
        }
        let score = expected_peaked(level);
        serde_json::json!({
            "type": "score",
            "score": score,
            "confidence": 0.80,
            "probabilities": Value::Object(probabilities),
        })
    }

    fn body(money: Value, evidence: Value, ambiguity: Value, intent: Value) -> String {
        serde_json::json!({
            "model": "jev-test",
            "answers": {
                MONEY_QUESTION: money,
                EVIDENCE_QUESTION: evidence,
                AMBIGUITY_QUESTION: ambiguity,
                INTENT_QUESTION: intent,
            }
        })
        .to_string()
    }

    #[test]
    fn probability_at_threshold_keeps_the_raw_score() {
        let raw = body(peaked(4), peaked(0), peaked(1), peaked(2));
        let score = map_ranking_body(&raw);
        assert!(score
            .money
            .raw()
            .is_some_and(|value| (value - expected_peaked(4)).abs() < 1e-9));
        assert!(score
            .evidence
            .raw()
            .is_some_and(|value| (value - expected_peaked(0)).abs() < 1e-9));
        assert!(score.ambiguity.raw().is_some());
        assert!(score.intent.raw().is_some());
    }

    #[test]
    fn probability_under_threshold_is_unknown_for_that_dimension_only() {
        let mut low = peaked(3);
        low["probabilities"]["3"] = serde_json::json!(0.79);
        low["probabilities"]["0"] = serde_json::json!(0.06);
        low["score"] =
            serde_json::json!(0.0 * 0.06 + 1.0 * 0.05 + 2.0 * 0.05 + 3.0 * 0.79 + 4.0 * 0.05);
        let raw = body(low, peaked(4), peaked(4), peaked(4));
        let score = map_ranking_body(&raw);
        assert_eq!(score.money, DimensionScore::Unknown);
        assert!(score.evidence.raw().is_some());
        assert!(priority(&score, RankWeights::default()).is_none());
    }

    #[test]
    fn a_decisive_zero_is_a_score_not_a_pass() {
        let raw = body(peaked(0), peaked(0), peaked(0), peaked(0));
        let score = map_ranking_body(&raw);
        assert!(score
            .money
            .raw()
            .is_some_and(|value| (value - expected_peaked(0)).abs() < 1e-9));
        let combined = priority(&score, RankWeights::default()).expect("priority");
        assert!(combined > 0.0);
    }

    #[test]
    fn distribution_that_does_not_sum_to_one_is_unknown() {
        let mut answer = peaked(4);
        answer["probabilities"]["4"] = serde_json::json!(0.90);
        answer["score"] = serde_json::json!(3.7);
        let raw = body(answer, peaked(1), peaked(1), peaked(1));
        assert_eq!(map_ranking_body(&raw).money, DimensionScore::Unknown);
    }

    #[test]
    fn score_that_disagrees_with_the_probabilities_is_unknown() {
        let mut answer = peaked(2);
        answer["score"] = serde_json::json!(4.0);
        let raw = body(answer, peaked(1), peaked(1), peaked(1));
        assert_eq!(map_ranking_body(&raw).money, DimensionScore::Unknown);
    }

    #[test]
    fn explanation_text_is_ignored() {
        let mut answer = peaked(1);
        answer["explanation"] = Value::String("model text that must not be used".to_owned());
        let raw = body(answer.clone(), answer.clone(), answer.clone(), answer);
        assert!(map_ranking_body(&raw).money.raw().is_some());
    }

    #[test]
    fn invalid_json_and_a_choice_answer_are_unknown() {
        assert_eq!(map_ranking_body("not json"), RankScore::unknown());
        let choice = serde_json::json!({
            "type": "choice",
            "choice": "same",
            "probabilities": { "same": 0.90, "different": 0.10 }
        });
        let raw = body(choice, peaked(1), peaked(1), peaked(1));
        assert_eq!(map_ranking_body(&raw).money, DimensionScore::Unknown);
    }

    #[test]
    fn call_failures_map_every_dimension_to_unknown() {
        let failures = [
            JevCallFailure::Timeout,
            JevCallFailure::Transport,
            JevCallFailure::Http { status: 422 },
            JevCallFailure::Utf8,
        ];
        for failure in failures {
            assert_eq!(
                map_ranking_exchange(Err(failure)),
                RankScore::unknown(),
                "{failure}"
            );
        }
    }

    #[test]
    fn weights_reorder_two_profiles_and_zero_weights_have_no_priority() {
        let money_high = RankScore {
            money: DimensionScore::Scored(4.0),
            evidence: DimensionScore::Scored(0.0),
            ambiguity: DimensionScore::Scored(0.0),
            intent: DimensionScore::Scored(0.0),
        };
        let evidence_high = RankScore {
            money: DimensionScore::Scored(0.0),
            evidence: DimensionScore::Scored(4.0),
            ambiguity: DimensionScore::Scored(4.0),
            intent: DimensionScore::Scored(4.0),
        };
        let even = RankWeights::default();
        let money_only = RankWeights {
            money: 1.0,
            evidence: 0.0,
            ambiguity: 0.0,
            intent: 0.0,
        };
        let money_even = priority(&money_high, even).expect("money");
        let evidence_even = priority(&evidence_high, even).expect("evidence");
        assert!(evidence_even > money_even);
        let money_weighted = priority(&money_high, money_only).expect("money weight");
        let evidence_weighted = priority(&evidence_high, money_only).expect("evidence weight");
        assert!(money_weighted > evidence_weighted);
        assert!(priority(
            &money_high,
            RankWeights {
                money: 0.0,
                evidence: 0.0,
                ambiguity: 0.0,
                intent: 0.0,
            }
        )
        .is_none());
        let mut unknown = money_high;
        unknown.intent = DimensionScore::Unknown;
        assert!(priority(&unknown, even).is_none());
    }

    #[test]
    fn request_asks_four_scores_and_for_no_explanation() {
        let raw = ranking_request(&no_second_payment_subject());
        let request: Value = serde_json::from_str(&raw).expect("request JSON");
        assert_eq!(request["model"], "jev-latest");
        assert_eq!(request["state"]["verdict"]["title"], "needs review");
        assert_eq!(request["state"]["verdict"]["outcome"], "Violate");
        assert_eq!(request["state"]["verdict"]["amount"], "12.50");
        assert_eq!(
            request["state"]["verdict"]["spec_id"],
            "ledger.bill.no-second-payment"
        );
        let questions = request["questions"].as_object().expect("questions");
        assert_eq!(questions.len(), 4);
        for id in [
            MONEY_QUESTION,
            EVIDENCE_QUESTION,
            AMBIGUITY_QUESTION,
            INTENT_QUESTION,
        ] {
            let question = &questions[id];
            assert_eq!(question["type"], "score");
            assert_eq!(question["criteria"].as_array().expect("criteria").len(), 5);
            assert!(question.get("explanation").is_none());
        }
        assert!(request.get("explanation").is_none());
        let lower = raw.to_ascii_lowercase();
        assert!(!lower.contains("explain"));
        assert!(!lower.contains("fraud"));
        assert!(!raw.contains("TYPESAFE_API_KEY"));
        assert!(!raw.contains("Bearer"));
    }

    #[test]
    fn saved_rank_fixture_replays_offline() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev");
        let request = std::fs::read_to_string(root.join("rank-request.json")).expect("request");
        let response = std::fs::read_to_string(root.join("rank-response.json")).expect("response");
        let mapped = std::fs::read_to_string(root.join("rank-mapped.json")).expect("mapped");
        assert_eq!(ranking_request(&no_second_payment_subject()), request);
        for raw in [&request, &response, &mapped] {
            assert!(!raw.contains("TYPESAFE_API_KEY"));
            assert!(!raw.contains("GEMINI_API_KEY"));
            assert!(!raw.contains("Bearer"));
            assert!(!raw.to_ascii_lowercase().contains("explain"));
            assert!(!raw.to_ascii_lowercase().contains("fraud"));
        }
        let value: Value = serde_json::from_str(&response).expect("response json");
        assert!(value["model"].as_str().unwrap_or("").starts_with("jev"));
        let score = map_ranking_body(&response);
        assert_eq!(score_json(&score), mapped.trim());
        assert_eq!(map_ranking_exchange(Ok(response.as_str())), score);
    }

    fn score_json(score: &RankScore) -> String {
        let value = sort_objects(serde_json::json!({
            "money": score.money.raw(),
            "evidence": score.evidence.raw(),
            "ambiguity": score.ambiguity.raw(),
            "intent": score.intent.raw(),
        }));
        serde_json::to_string(&value).expect("mapped json")
    }

    /// Writes `fixtures/jev/rank-*.json` from one live System One call.
    ///
    /// `just test` does not run this. Replay covers the saved bodies.
    #[test]
    #[ignore]
    fn record_live_rank_call() {
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        let client = crate::SystemOneClient::new(key.clone()).expect("client");
        let request = ranking_request(&no_second_payment_subject());
        assert!(!request.contains(&key));
        let response = client.evaluate(&request).expect("system one call");
        assert!(!response.contains(&key));
        assert!(!response.contains("Bearer"));
        let response = strip_explanation(&response);
        let score = map_ranking_body(&response);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev");
        std::fs::write(root.join("rank-request.json"), &request).expect("request");
        std::fs::write(root.join("rank-response.json"), &response).expect("response");
        std::fs::write(root.join("rank-mapped.json"), score_json(&score)).expect("mapped");
    }

    fn strip_explanation(body: &str) -> String {
        let Ok(mut value) = serde_json::from_str::<Value>(body) else {
            return body.to_owned();
        };
        fn walk(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    map.retain(|key, _| key != "explanation");
                    for child in map.values_mut() {
                        walk(child);
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        walk(item);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        walk(&mut value);
        serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
    }
}
