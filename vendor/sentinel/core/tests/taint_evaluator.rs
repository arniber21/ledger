//! CAT-4 traversal through [`Evaluator`]: CausalIndex, Slice, EvidenceGate.
//!
//! Observation-constructed table tests live in `taint_decision_table.rs`. This
//! file is the interface: `Evaluator::taint` / `step` / `seal` / `evaluate()`.
//! A contributing-tier-only falsifier must never reach Satisfy.

#![allow(clippy::doc_markdown)]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::slice::{
    Binding, RetirementBreach, RetirementCause, RetirementSignal, ScopeHorizon,
};
use sentinel_core::specification::RetirementPolicy;
use sentinel_core::taint::TaintSpecification;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::{
    ActivationBarrier, ActivationWatermark, Event, EventId, FilterEpoch, InconclusiveReason,
    SequenceEpoch, SequenceNumber, SourceEvidence, SourceIdentity, SourceTier, VerdictAuthority,
    VerdictOutcome,
};

use support::taint::{
    action, catalogued_taint, closed_sources, observation, source, source_evidence,
    trusted_content, untrusted, BROKER, MAIL, REQUEST,
};

fn sole_verdict(verdicts: Vec<BindingVerdict>) -> BindingVerdict {
    assert_eq!(verdicts.len(), 1, "expected one Binding, got {verdicts:?}");
    verdicts.into_iter().next().expect("len is 1")
}

fn evaluate(
    spec: TaintSpecification,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Vec<BindingVerdict> {
    sentinel_core::evaluate(Evaluator::taint(spec), events, sources).expect("evaluation succeeds")
}

fn seal(
    spec: TaintSpecification,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Vec<BindingVerdict> {
    let mut evaluator = Evaluator::taint(spec);
    for event in events {
        evaluator
            .step(Input::event(event.clone()))
            .expect("stepping an Event succeeds");
    }
    for evidence in sources.values() {
        evaluator
            .step(Input::source(evidence.clone()))
            .expect("stepping source evidence succeeds");
    }
    evaluator.seal().expect("seal succeeds").into_verdicts()
}

/// Untrusted Event Before the action, both sources Anchor, coverage closed → Violate.
#[test]
fn a_catalogued_taint_violation_settles_violate() {
    let untrusted_id = EventId::new("content-1").expect("constant");
    let events = vec![
        untrusted("content-1", 1),
        action("mail-1", 1, BTreeSet::from([untrusted_id])),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    let spec = catalogued_taint();

    let sealed = sole_verdict(seal(spec.clone(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(
        sealed.verdict().authority(),
        VerdictAuthority::Authoritative
    );

    let batched = sole_verdict(evaluate(spec, &events, &sources));
    assert_eq!(batched.verdict().outcome(), VerdictOutcome::Violate);
}

/// Action with no untrusted ancestor, coverage closed, Event-local Scope closed → Satisfy.
#[test]
fn a_clean_taint_binding_settles_satisfy() {
    let events = vec![
        trusted_content("content-1", 1),
        action("mail-1", 1, BTreeSet::new()),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    let spec = catalogued_taint();

    let sealed = sole_verdict(seal(spec.clone(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        sealed.verdict().authority(),
        VerdictAuthority::Authoritative
    );

    let batched = sole_verdict(evaluate(spec, &events, &sources));
    assert_eq!(batched.verdict().outcome(), VerdictOutcome::Satisfy);
}

/// Contributing-tier-only falsifier: untrusted in causal past, coverage closed →
/// Inconclusive(PrefixUncertainty), never Satisfy.
#[test]
fn a_contributing_tier_only_violation_never_reaches_satisfy_through_the_evaluator() {
    let untrusted_id = EventId::new("content-1").expect("constant");
    let events = vec![
        untrusted("content-1", 1),
        action("mail-1", 1, BTreeSet::from([untrusted_id])),
    ];
    let sources = closed_sources(1, 1, SourceTier::Contributing);
    let spec = catalogued_taint();

    let sealed = sole_verdict(seal(spec.clone(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    assert_ne!(sealed.verdict().outcome(), VerdictOutcome::Satisfy);

    let batched = sole_verdict(evaluate(spec, &events, &sources));
    assert_eq!(batched.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(batched.verdict().outcome(), VerdictOutcome::Satisfy);
}

/// Empty action set produces no Binding — vacuous Satisfy remains refused.
#[test]
fn an_empty_action_set_produces_no_binding() {
    let events = vec![untrusted("content-1", 1)];
    // Mail-gateway delivered nothing; coverage is incomplete. Seal over a
    // Binding-less prefix must still produce no Verdict, not an error: there
    // is no obligation to settle.
    let spec = catalogued_taint();
    let sealed = seal(
        spec.clone(),
        &events,
        &BTreeMap::from([(
            source(BROKER),
            source_evidence(BROKER, 1, SourceTier::Anchor),
        )]),
    );
    assert!(
        sealed.is_empty(),
        "no action Event means no Binding: {sealed:?}"
    );

    let batched = evaluate(
        spec,
        &events,
        &BTreeMap::from([(
            source(BROKER),
            source_evidence(BROKER, 1, SourceTier::Anchor),
        )]),
    );
    assert!(batched.is_empty());
}

/// Missing SourceEvidence closes the gate → prefix Inconclusive, never Satisfy.
#[test]
fn a_closed_gate_is_prefix_inconclusive_never_satisfy() {
    let untrusted_id = EventId::new("content-1").expect("constant");
    let events = vec![
        untrusted("content-1", 1),
        action("mail-1", 1, BTreeSet::from([untrusted_id])),
    ];
    // Events from both sources, SourceEvidence for mail only.
    let sources = BTreeMap::from([(source(MAIL), source_evidence(MAIL, 1, SourceTier::Anchor))]);
    let spec = catalogued_taint();

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
}

/// Concurrent untrusted Event with the action → InconclusiveObservational.
#[test]
fn a_concurrent_falsifier_is_observationally_uncertain() {
    let events = vec![
        untrusted("content-1", 1),
        action("mail-1", 1, BTreeSet::new()),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    let spec = catalogued_taint();

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::ObservationalUncertainty)
    );
}

/// A held predecessor anywhere closes the gate → prefix Inconclusive, never Satisfy.
#[test]
fn a_held_predecessor_is_prefix_inconclusive_never_satisfy() {
    let missing = EventId::new("never-arrived").expect("constant");
    let events = vec![
        trusted_content("content-1", 1),
        action("mail-1", 1, BTreeSet::new()),
        action("mail-2", 2, BTreeSet::from([missing])),
    ];
    let sources = closed_sources(2, 1, SourceTier::Anchor);
    let spec = catalogued_taint();

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
}
#[test]
fn a_warming_taint_specification_cannot_emit_an_authoritative_satisfy() {
    let events = vec![
        trusted_content("content-1", 1),
        action("mail-1", 1, BTreeSet::new()),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    let observed_at = observation(40);
    let watermarks = BTreeMap::from([
        (
            source(MAIL),
            ActivationWatermark::new(
                SequenceEpoch::new(1),
                SequenceNumber::new(1),
                observed_at.clone(),
            ),
        ),
        (
            source(BROKER),
            ActivationWatermark::new(SequenceEpoch::new(1), SequenceNumber::new(1), observed_at),
        ),
    ]);
    let barrier =
        ActivationBarrier::new(FilterEpoch::new(2), watermarks).expect("two required sources");
    let spec = catalogued_taint().with_activation_barrier(barrier);

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(sealed.verdict().authority(), VerdictAuthority::Provisional);
    assert_ne!(
        sealed.verdict().authority(),
        VerdictAuthority::Authoritative
    );
}

/// A warming CAT-4 may still emit a Provisional Violate.
#[test]
fn a_warming_taint_specification_still_emits_a_provisional_violate() {
    let untrusted_id = EventId::new("content-1").expect("constant");
    let events = vec![
        untrusted("content-1", 1),
        action("mail-1", 1, BTreeSet::from([untrusted_id])),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    let observed_at = observation(40);
    let watermarks = BTreeMap::from([
        (
            source(MAIL),
            ActivationWatermark::new(
                SequenceEpoch::new(1),
                SequenceNumber::new(1),
                observed_at.clone(),
            ),
        ),
        (
            source(BROKER),
            ActivationWatermark::new(SequenceEpoch::new(1), SequenceNumber::new(1), observed_at),
        ),
    ]);
    let barrier =
        ActivationBarrier::new(FilterEpoch::new(2), watermarks).expect("two required sources");
    let spec = catalogued_taint().with_activation_barrier(barrier);

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(sealed.verdict().authority(), VerdictAuthority::Provisional);
}
/// A catalogued CAT-4 Binding cannot be retired, and the catalogue is what says so.
///
/// Clause (d0) reads the Specification's declared retirement policy, which
/// `TryFrom<&TaintIr>` lowered from the Pattern's own `retirement` field. CAT-4
/// declares `NotRequired` -- the action Event whose provenance is constrained
/// *is* the Scope's close -- so the refusal is a consequence of the declaration
/// rather than of a family check written into the evaluator. Change the
/// catalogue and this test moves with it; that is the point.
#[test]
fn retiring_a_cat4_binding_is_rejected() {
    let specification = catalogued_taint();
    assert_eq!(
        specification.retirement_policy(),
        RetirementPolicy::NotRequired,
        "the catalogued CAT-4 Pattern declares no retirement policy"
    );

    let untrusted_id = EventId::new("content-1").expect("constant");
    let mut evaluator = Evaluator::taint(specification);
    for event in [
        untrusted("content-1", 1),
        action("mail-1", 1, BTreeSet::from([untrusted_id])),
    ] {
        evaluator
            .step(Input::event(event))
            .expect("events step cleanly");
    }

    assert_eq!(
        evaluator.step(Input::retire(RetirementSignal::new(
            Binding::new(REQUEST),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Deadline,
        ))),
        Err(CoreError::Retirement(RetirementBreach::NotDeclared))
    );

    // And the refusal left the evaluator usable: the sources still step, and
    // the evaluation still seals into the Verdict it would have reached.
    for evidence in closed_sources(1, 1, SourceTier::Anchor).into_values() {
        evaluator
            .step(Input::source(evidence))
            .expect("source evidence steps after a refused retirement");
    }
    let sealed = evaluator
        .seal()
        .expect("sealing succeeds after the refusal");
    assert_eq!(
        sole_verdict(sealed.into_verdicts()).verdict().outcome(),
        VerdictOutcome::Violate,
        "the refused retirement changed nothing about the conclusion"
    );
}
