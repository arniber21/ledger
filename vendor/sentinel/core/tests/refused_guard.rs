//! Terminal `Refused` settles Authoritative `Inconclusive(epistemic)`.
//!
//! Pure fixtures: recorded [`FactResponse`]s, no store, no clock (ADR-0012).
//! The store-backed twins of the closer live in `sentinel-coldpath`.

#![allow(clippy::doc_markdown)]

mod support;

use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::facts::{FactRequest, GuardConclusion, RefusalReason};
use sentinel_core::verdict::REFUSED_GUARD_DETAIL;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::{InconclusiveReason, TierSettlement, VerdictAuthority, VerdictOutcome};

use support::{
    adapted, answer_all, answer_all_as, inputs, required_watermarks,
    ship_requires_prior_charge_with_cold_guards,
};

fn step_all(evaluator: &mut Evaluator, inputs: Vec<Input>) -> Vec<Step> {
    inputs
        .into_iter()
        .map(|input| evaluator.step(input).expect("the fixture steps cleanly"))
        .collect()
}

fn request_after(evaluator: &mut Evaluator, steps: &[Step]) -> FactRequest {
    if let Some(Step::NeedsFacts(request)) = steps
        .iter()
        .find(|step| matches!(step, Step::NeedsFacts(_)))
    {
        return request.clone();
    }
    match evaluator
        .resume(Vec::new())
        .expect("an empty resume is legal")
    {
        Step::NeedsFacts(request) => request,
        other => panic!("expected NeedsFacts, got {other:?}"),
    }
}

fn at_the_request(evidence: &ObservedEvidence) -> (Evaluator, FactRequest) {
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(evidence));
    let request = request_after(&mut evaluator, &steps);
    (evaluator, request)
}

/// The closer: a settling `Refused` is Authoritative Inconclusive(epistemic).
///
/// Mutation 1: drop `Refused` from `settles()`, or skip `epistemic_from_refused`,
/// and this test goes red.
#[test]
fn a_terminal_refused_settles_authoritative_inconclusive_epistemic() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let (mut evaluator, request) = at_the_request(&evidence);

    let step = evaluator
        .resume(answer_all_as(
            &request,
            &GuardConclusion::Refused {
                reason: RefusalReason::EvidenceReaped,
            },
            &required,
        ))
        .expect("a terminal refusal is a settling answer, not a Core bug");
    let verdict = match &step {
        Step::Settled(verdicts) => verdicts[0].verdict().clone(),
        other => panic!("expected Settled, got {other:?}"),
    };

    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::EpistemicUncertainty)
    );
    assert_eq!(verdict.cold_path(), TierSettlement::Settled);
    assert!(
        verdict.uncertainty().iter().any(|entry| entry.reason
            == InconclusiveReason::EpistemicUncertainty
            && entry.detail == REFUSED_GUARD_DETAIL),
        "envelope must carry {REFUSED_GUARD_DETAIL}"
    );
    assert!(
        evaluator.outstanding().is_empty(),
        "a settling refusal leaves the outstanding set"
    );
    assert!(
        evaluator
            .closed_guards()
            .values()
            .all(|conclusion| matches!(conclusion, GuardConclusion::Refused { .. })),
        "closed_guards is derived from settled Refused"
    );
}

/// A settled `Refused` is absorbing: a later `Holds` cannot reopen it.
///
/// Mutation 2: re-insert `Unissued` after `Settled` and this test goes red.
#[test]
fn a_settling_answer_after_a_refusal_does_not_reopen_it() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let (mut evaluator, request) = at_the_request(&evidence);

    evaluator
        .resume(answer_all_as(
            &request,
            &GuardConclusion::Refused {
                reason: RefusalReason::EvidenceNeverRetained,
            },
            &required,
        ))
        .expect("the refusal settles");

    let second = evaluator
        .resume(answer_all(&request, &required))
        .expect("a later Holds is accepted and ignored");
    match second {
        Step::Pending | Step::Settled(_) => {}
        other => panic!("a re-opened query would NeedFacts, got {other:?}"),
    }
    assert!(evaluator.outstanding().is_empty());
    assert!(evaluator.closed_guards().values().all(|conclusion| {
        matches!(
            conclusion,
            GuardConclusion::Refused {
                reason: RefusalReason::EvidenceNeverRetained
            }
        )
    }));
}

/// A settled `Refused` is not outstanding.
///
/// Mutation 3: keep a settled `Refused` in `outstanding()` and this test goes red.
#[test]
fn a_settled_refused_query_leaves_the_outstanding_set() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let (mut evaluator, request) = at_the_request(&evidence);
    assert!(!request.queries().is_empty());

    evaluator
        .resume(answer_all_as(
            &request,
            &GuardConclusion::Refused {
                reason: RefusalReason::GuardUnknown,
            },
            &required,
        ))
        .expect("the refusal settles");

    assert!(evaluator.outstanding().is_empty());
}
