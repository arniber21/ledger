//! Spec authoring: catalogue validation, Jev match, running monitor, SSE.

use ledger_core::{
    AgentId, BillId, LedgerEvent, LedgerKind, Times, VerdictOutcome, BILL_ATTRIBUTE,
};
use ledger_jev::{BindingVerdict, JevCallFailure};
use ledger_spec::{
    author, clause_question_id, coverage_request, draft_from_interaction, draft_request,
    match_request, split_clauses, AuthoringInput, AuthoringStep, CatalogueCode, ClauseCoverage,
    DraftFailure, HoldReason, RejectReason, DRAFT_MATCH_QUESTION, FIXTURE_POLICY, GEMINI_MODEL,
};
use serde_json::Value;

fn agent() -> AgentId {
    AgentId::new("buyer-1").expect("agent id")
}

fn bill(id: &str) -> BillId {
    BillId::new(id).expect("bill id")
}

fn event(
    id: &str,
    kind: LedgerKind,
    bill_id: &str,
    at: i128,
    predecessors: &[&str],
) -> LedgerEvent {
    LedgerEvent::new(
        id,
        kind,
        bill(bill_id),
        agent(),
        Times::at(at),
        predecessors.iter().copied(),
    )
    .expect("ledger event")
}

fn double_pay() -> Vec<LedgerEvent> {
    vec![
        event("pay", LedgerKind::AgentPayBill, "bill-1", 10, &[]),
        event(
            "settled",
            LedgerKind::ObligationSettled,
            "bill-1",
            20,
            &["pay"],
        ),
        event(
            "pay-again",
            LedgerKind::AgentPayBill,
            "bill-1",
            30,
            &["settled"],
        ),
        event(
            "extra",
            LedgerKind::ExtraPayment,
            "bill-1",
            40,
            &["pay-again"],
        ),
        event("credit", LedgerKind::Credit, "bill-1", 50, &["extra"]),
    ]
}

fn absence_text() -> String {
    serde_json::json!({
        "pattern_id": "absence.after-until",
        "binding_attribute": BILL_ATTRIBUTE,
        "after": "ledger.agent.pay_bill",
        "until": "ledger.exchange.credit",
        "veto": "ledger.exchange.payment"
    })
    .to_string()
}

fn interaction(text: &str) -> String {
    serde_json::json!({
        "object": "interaction",
        "status": "completed",
        "model": GEMINI_MODEL,
        "steps": [
            {
                "type": "thought",
                "summary": [{ "type": "text", "text": "prose that must not be used" }]
            },
            {
                "type": "model_output",
                "content": [{ "type": "text", "text": text }]
            }
        ]
    })
    .to_string()
}

fn jev_choice(label: &str, p_same: f64, p_different: f64) -> String {
    serde_json::json!({
        "model": "jev-test",
        "answers": {
            DRAFT_MATCH_QUESTION: {
                "type": "choice",
                "choice": label,
                "probabilities": { "same": p_same, "different": p_different }
            }
        }
    })
    .to_string()
}

fn run(
    gemini: Result<&str, DraftFailure>,
    jev: Result<&str, JevCallFailure>,
) -> ledger_spec::AuthoringOutcome {
    let agent = agent();
    author(AuthoringInput {
        sentence: FIXTURE_POLICY,
        agent: &agent,
        gemini,
        jev,
        coverage: Err(JevCallFailure::Timeout),
        override_coverage: false,
    })
}

const TWO_CLAUSES: &str = "Pay the bill. Keep a credit.";

fn author_policy(
    sentence: &str,
    jev: Result<&str, JevCallFailure>,
    coverage: Result<&str, JevCallFailure>,
    override_coverage: bool,
) -> ledger_spec::AuthoringOutcome {
    let agent = agent();
    let gemini = interaction(&absence_text());
    author(AuthoringInput {
        sentence,
        agent: &agent,
        gemini: Ok(&gemini),
        jev,
        coverage,
        override_coverage,
    })
}

fn coverage_choice(parts: &[(&str, f64, f64)]) -> String {
    let mut answers = serde_json::Map::new();
    for (index, (label, p_same, p_different)) in parts.iter().enumerate() {
        answers.insert(
            clause_question_id(index),
            serde_json::json!({
                "type": "choice",
                "choice": label,
                "probabilities": { "same": p_same, "different": p_different }
            }),
        );
    }
    serde_json::json!({
        "model": "jev-test",
        "answers": answers
    })
    .to_string()
}

#[test]
fn same_match_starts_a_running_monitor_that_violates_on_double_pay() {
    let gemini = interaction(&absence_text());
    let jev = jev_choice("same", 0.91, 0.09);
    let outcome = run(Ok(&gemini), Ok(&jev));
    let monitor = outcome.into_monitor().expect("accepted monitor");
    assert_eq!(monitor.pattern_id(), "absence.after-until");
    let verdicts = monitor.judge(&agent(), &double_pay()).expect("judge");
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    assert_eq!(verdicts[0].verdict().outcome(), VerdictOutcome::Violate);
}

#[test]
fn accepted_run_streams_sentence_draft_check_and_accepted() {
    let gemini = interaction(&absence_text());
    let jev = jev_choice("same", 0.80, 0.20);
    let outcome = run(Ok(&gemini), Ok(&jev));
    assert!(matches!(
        outcome.steps(),
        [
            AuthoringStep::Sentence { .. },
            AuthoringStep::Draft { .. },
            AuthoringStep::Check {
                verdict: BindingVerdict::Same
            },
            AuthoringStep::Coverage { .. },
            AuthoringStep::Accepted {
                overridden: false,
                ..
            },
        ]
    ));
    let sse = outcome.encode_sse();
    assert!(sse.contains("event: sentence\n"));
    assert!(sse.contains("event: draft\n"));
    assert!(sse.contains("event: check\n"));
    assert!(sse.contains("event: coverage\n"));
    assert!(sse.contains("\"coverage\":\"covered\""));
    assert!(sse.contains("event: accepted\n"));
    assert!(sse.contains("\"override\":false"));
    assert!(sse.contains("\"monitor\":\"running\""));
    assert!(!sse.contains("prose that must not be used"));
    assert!(!sse.contains("fraud"));
}

#[test]
fn unknown_pattern_is_rejected_without_calling_jev() {
    let gemini = interaction(r#"{"pattern_id":"precedence"}"#);
    let outcome = run(Ok(&gemini), Err(JevCallFailure::Timeout));
    assert!(outcome.monitor().is_none());
    assert!(matches!(
        outcome.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::UnknownPattern
        })
    ));
    assert!(!outcome
        .steps()
        .iter()
        .any(|step| matches!(step, AuthoringStep::Check { .. })));
    assert!(outcome
        .encode_sse()
        .contains("\"reason\":\"unknown_pattern\""));
}

#[test]
fn missing_parameter_is_rejected_with_a_catalogue_reason() {
    let gemini = interaction(&format!(
        r#"{{"pattern_id":"absence.after-until","binding_attribute":"{BILL_ATTRIBUTE}"}}"#
    ));
    let outcome = run(Ok(&gemini), Ok(&jev_choice("same", 0.99, 0.01)));
    assert!(outcome.monitor().is_none());
    assert!(matches!(
        outcome.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::CatalogueInvalid {
                code: CatalogueCode::MissingParameter { name }
            }
        }) if name == "after"
    ));
}

#[test]
fn event_kind_the_ledger_does_not_emit_is_rejected() {
    let mut draft: Value = serde_json::from_str(&absence_text()).expect("draft json");
    draft["veto"] = Value::String("ledger.not.emitted".to_owned());
    let gemini = interaction(&draft.to_string());
    let outcome = run(Ok(&gemini), Ok(&jev_choice("same", 0.99, 0.01)));
    assert!(matches!(
        outcome.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::CatalogueInvalid {
                code: CatalogueCode::EventKindNotEmitted { name, event_kind }
            }
        }) if name == "veto" && event_kind == "ledger.not.emitted"
    ));
}

#[test]
fn jev_different_rejects_and_does_not_start_a_monitor() {
    let gemini = interaction(&absence_text());
    let outcome = run(Ok(&gemini), Ok(&jev_choice("different", 0.10, 0.90)));
    assert!(outcome.monitor().is_none());
    assert!(matches!(
        outcome.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::DraftDiffers
        })
    ));
}

#[test]
fn jev_timeout_low_score_and_contradiction_are_inconclusive() {
    let gemini = interaction(&absence_text());
    let low = jev_choice("same", 0.79, 0.21);
    let contradiction = jev_choice("same", 0.90, 0.90);
    let cases = [
        Err(JevCallFailure::Timeout),
        Err(JevCallFailure::Transport),
        Err(JevCallFailure::Http { status: 422 }),
        Ok(low.as_str()),
        Ok(contradiction.as_str()),
    ];
    for jev in cases {
        let outcome = run(Ok(&gemini), jev);
        assert!(outcome.monitor().is_none());
        assert!(matches!(
            outcome.steps().last(),
            Some(AuthoringStep::Rejected {
                reason: RejectReason::MatchInconclusive
            })
        ));
        assert!(matches!(
            outcome
                .steps()
                .iter()
                .find(|step| matches!(step, AuthoringStep::Check { .. })),
            Some(AuthoringStep::Check {
                verdict: BindingVerdict::Unknown
            })
        ));
    }
}

#[test]
fn gemini_failure_and_unreadable_body_do_not_accept() {
    let agent = agent();
    let failed = author(AuthoringInput {
        sentence: FIXTURE_POLICY,
        agent: &agent,
        gemini: Err(DraftFailure::Timeout),
        jev: Ok(&jev_choice("same", 0.99, 0.01)),
        coverage: Err(JevCallFailure::Timeout),
        override_coverage: false,
    });
    assert!(matches!(
        failed.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::DraftUnavailable
        })
    ));

    let unreadable = author(AuthoringInput {
        sentence: FIXTURE_POLICY,
        agent: &agent,
        gemini: Ok("not json"),
        jev: Ok(&jev_choice("same", 0.99, 0.01)),
        coverage: Err(JevCallFailure::Timeout),
        override_coverage: false,
    });
    assert!(matches!(
        unreadable.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::DraftUnreadable
        })
    ));
}

#[test]
fn empty_policy_is_rejected_before_either_model() {
    let agent = agent();
    let outcome = author(AuthoringInput {
        sentence: "  ",
        agent: &agent,
        gemini: Err(DraftFailure::Transport),
        jev: Err(JevCallFailure::Timeout),
        coverage: Err(JevCallFailure::Timeout),
        override_coverage: true,
    });
    assert!(matches!(
        outcome.steps(),
        [
            AuthoringStep::Sentence { .. },
            AuthoringStep::Rejected {
                reason: RejectReason::EmptyPolicy
            }
        ]
    ));
}

#[test]
fn requests_name_the_models_and_do_not_ask_for_prose() {
    let request = draft_request(FIXTURE_POLICY);
    let value: Value = serde_json::from_str(&request).expect("request json");
    assert_eq!(value["model"], GEMINI_MODEL);
    assert_eq!(value["store"], false);
    assert_eq!(value["response_format"]["mime_type"], "application/json");
    let branches = value["response_format"]["schema"]["anyOf"]
        .as_array()
        .expect("anyOf");
    assert!(branches.iter().any(|branch| {
        branch["properties"]["pattern_id"]["enum"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == "absence.after-until"))
            && branch["required"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item == "after"))
    }));
    assert!(request.contains(FIXTURE_POLICY));
    assert!(!request.to_ascii_lowercase().contains("explain"));
    assert!(!request.contains("GEMINI_API_KEY"));

    let draft = draft_from_interaction(&interaction(&absence_text())).expect("draft");
    let jev = match_request(FIXTURE_POLICY, &draft);
    let jev_value: Value = serde_json::from_str(&jev).expect("jev request json");
    assert_eq!(jev_value["model"], "jev-latest");
    assert!(jev_value["questions"]
        .as_object()
        .expect("questions")
        .contains_key(DRAFT_MATCH_QUESTION));
    assert_eq!(
        jev_value["questions"][DRAFT_MATCH_QUESTION]["type"],
        "choice"
    );
    assert!(!jev.to_ascii_lowercase().contains("explain"));
    assert!(!jev.contains("TYPESAFE_API_KEY"));
}

#[test]
fn saved_gemini_and_jev_fixtures_replay_offline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let gemini_request =
        std::fs::read_to_string(root.join("gemini/policy-draft-request.json")).expect("request");
    let gemini = std::fs::read_to_string(root.join("gemini/policy-draft.json")).expect("gemini");
    let jev_request =
        std::fs::read_to_string(root.join("jev/spec-match-request.json")).expect("jev request");
    let jev = std::fs::read_to_string(root.join("jev/spec-match.json")).expect("jev");
    for raw in [&gemini_request, &gemini, &jev_request, &jev] {
        assert!(!raw.contains("GEMINI_API_KEY"));
        assert!(!raw.contains("TYPESAFE_API_KEY"));
        assert!(!raw.contains("Bearer"));
        assert!(!raw.contains("x-goog-api-key"));
    }
    assert_eq!(draft_request(FIXTURE_POLICY), gemini_request);
    let draft = draft_from_interaction(&gemini).expect("fixture draft");
    assert_eq!(draft.pattern_id(), "absence.after-until");
    assert_eq!(match_request(FIXTURE_POLICY, &draft), jev_request);
    let value: Value = serde_json::from_str(&jev).expect("jev json");
    assert_eq!(value["model"], "jev-1.13.0");
    let outcome = run(Ok(&gemini), Ok(&jev));
    assert!(matches!(
        outcome.steps(),
        [
            AuthoringStep::Sentence { .. },
            AuthoringStep::Draft { .. },
            AuthoringStep::Check {
                verdict: BindingVerdict::Same
            },
            AuthoringStep::Coverage { .. },
            AuthoringStep::Accepted {
                overridden: false,
                ..
            },
        ]
    ));
    let monitor = outcome.into_monitor().expect("accepted monitor");
    assert_eq!(monitor.pattern_id(), "absence.after-until");
    let verdicts = monitor.judge(&agent(), &double_pay()).expect("judge");
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    assert_eq!(verdicts[0].verdict().outcome(), VerdictOutcome::Violate);
}

#[test]
fn partial_coverage_shows_the_uncovered_clause_and_does_not_arm() {
    let coverage = coverage_choice(&[("same", 0.91, 0.09), ("different", 0.08, 0.92)]);
    let outcome = author_policy(
        TWO_CLAUSES,
        Ok(&jev_choice("same", 0.95, 0.05)),
        Ok(&coverage),
        false,
    );
    assert!(outcome.monitor().is_none());
    let clauses = split_clauses(TWO_CLAUSES);
    assert_eq!(clauses.len(), 2);
    assert!(matches!(
        outcome.steps(),
        [
            AuthoringStep::Sentence { .. },
            AuthoringStep::Draft { .. },
            AuthoringStep::Check {
                verdict: BindingVerdict::Same
            },
            AuthoringStep::Coverage { .. },
            AuthoringStep::Held {
                reason: HoldReason::ClauseUncovered
            },
        ]
    ));
    let AuthoringStep::Coverage { clauses: marked } = &outcome.steps()[3] else {
        panic!("coverage step");
    };
    assert_eq!(marked[0].text, clauses[0]);
    assert_eq!(marked[0].coverage, ClauseCoverage::Covered);
    assert_eq!(marked[1].text, clauses[1]);
    assert_eq!(marked[1].coverage, ClauseCoverage::Uncovered);
    let sse = outcome.encode_sse();
    assert!(sse.contains("\"coverage\":\"uncovered\""));
    assert!(sse.contains("\"reason\":\"clause_uncovered\""));
    assert!(!sse.contains("event: accepted\n"));
    assert!(!sse.contains("fraud"));
}

#[test]
fn fully_covered_clauses_arm_as_before() {
    let coverage = coverage_choice(&[("same", 0.80, 0.20), ("same", 0.97, 0.03)]);
    let outcome = author_policy(
        TWO_CLAUSES,
        Ok(&jev_choice("same", 0.91, 0.09)),
        Ok(&coverage),
        false,
    );
    let monitor = outcome.into_monitor().expect("fully covered arms");
    assert_eq!(monitor.pattern_id(), "absence.after-until");
    let verdicts = monitor.judge(&agent(), &double_pay()).expect("judge");
    assert_eq!(verdicts[0].verdict().outcome(), VerdictOutcome::Violate);
}

#[test]
fn coverage_failure_marks_clauses_unknown_until_override() {
    let blocked = author_policy(
        TWO_CLAUSES,
        Ok(&jev_choice("same", 0.91, 0.09)),
        Err(JevCallFailure::Timeout),
        false,
    );
    assert!(blocked.monitor().is_none());
    assert!(matches!(
        blocked.steps().last(),
        Some(AuthoringStep::Held {
            reason: HoldReason::ClauseUnknown
        })
    ));
    let AuthoringStep::Coverage { clauses } = &blocked.steps()[3] else {
        panic!("coverage step");
    };
    assert!(clauses
        .iter()
        .all(|clause| clause.coverage == ClauseCoverage::Unknown));
    assert!(blocked
        .encode_sse()
        .contains("\"reason\":\"clause_unknown\""));

    let low = coverage_choice(&[("same", 0.91, 0.09), ("same", 0.79, 0.21)]);
    let under = author_policy(
        TWO_CLAUSES,
        Ok(&jev_choice("same", 0.91, 0.09)),
        Ok(&low),
        false,
    );
    assert!(matches!(
        under.steps().last(),
        Some(AuthoringStep::Held {
            reason: HoldReason::ClauseUnknown
        })
    ));

    let armed = author_policy(
        TWO_CLAUSES,
        Ok(&jev_choice("same", 0.91, 0.09)),
        Err(JevCallFailure::Http { status: 422 }),
        true,
    );
    assert!(matches!(
        armed.steps().last(),
        Some(AuthoringStep::Accepted {
            overridden: true,
            ..
        })
    ));
    assert!(armed.monitor().is_some());
    assert!(armed.encode_sse().contains("\"override\":true"));
}

#[test]
fn override_does_not_arm_a_different_or_unknown_match() {
    let coverage = coverage_choice(&[("same", 0.99, 0.01), ("different", 0.01, 0.99)]);
    let different = author_policy(
        TWO_CLAUSES,
        Ok(&jev_choice("different", 0.10, 0.90)),
        Ok(&coverage),
        true,
    );
    assert!(different.monitor().is_none());
    assert!(matches!(
        different.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::DraftDiffers
        })
    ));

    let unknown = author_policy(
        TWO_CLAUSES,
        Err(JevCallFailure::Timeout),
        Ok(&coverage),
        true,
    );
    assert!(unknown.monitor().is_none());
    assert!(matches!(
        unknown.steps().last(),
        Some(AuthoringStep::Rejected {
            reason: RejectReason::MatchInconclusive
        })
    ));
}

#[test]
fn coverage_request_batches_one_choice_per_clause_and_asks_for_no_prose() {
    let draft = draft_from_interaction(&interaction(&absence_text())).expect("draft");
    let clauses = split_clauses(TWO_CLAUSES);
    let request = coverage_request(TWO_CLAUSES, &clauses, &draft);
    let value: Value = serde_json::from_str(&request).expect("coverage request");
    assert_eq!(value["model"], "jev-latest");
    let questions = value["questions"].as_object().expect("questions");
    assert_eq!(questions.len(), 2);
    assert_eq!(questions[&clause_question_id(0)]["type"], "choice");
    assert_eq!(questions[&clause_question_id(1)]["type"], "choice");
    assert_eq!(value["state"]["clauses"][1], "Keep a credit.");
    assert!(!request.to_ascii_lowercase().contains("explain"));
    assert!(!request.contains("TYPESAFE_API_KEY"));
    assert!(!request.contains("Bearer"));
}
