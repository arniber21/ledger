//! CAT-2 traversal through [`Evaluator`]: CausalIndex, Slice, EvidenceGate.
//!
//! Observation-constructed table tests live in `absence_decision_table.rs`.
//! Cases: never-opened, concurrent, Satisfy, Violate, contributing-tier pin.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, ProducerContract, SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::absence::AbsenceSpecification;
use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::verdict::BindingVerdict;
use sentinel_model::{
    ActivationBarrier, ActivationWatermark, AttributeKey, AttributeValue, ClockDomainId, DataClass,
    Event, EventId, EventKind, FilterEpoch, InconclusiveReason, Integrity, MonitoredApplicationId,
    ObservationTime, OccurrenceTime, ProducerCompletenessSignal, ProducerEvent, SchemaVersion,
    Sensitivity, SentinelClockId, SequenceEpoch, SequenceNumber, SourceActivity, SourceAssurance,
    SourceCapabilities, SourceCapability, SourceCompletenessSignal, SourceEvidence, SourceId,
    SourceIdentity, SourceTier, VerdictAuthority, VerdictOutcome,
};

const BINDING: &str = "example.request.id";
const REQUEST: &str = "req-1";
const AFTER_KIND: &str = "example.approval.granted";
const UNTIL_KIND: &str = "example.action.committed";
const VETO_KIND: &str = "example.approval.revoked";
const APPROVAL: &str = "example-approval-service";
const ACTION: &str = "example-action-gateway";
const REVOCATION: &str = "example-revocation-bus";

fn identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.no-revoke-before-commit".to_owned(),
        specification_version: "v1".to_owned(),
    }
}

fn source(application: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(application).expect("constant"),
        SourceId::new("probe").expect("constant"),
    )
}

fn capable() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

fn selector(kind: &str, attributes: &[(&str, &str)]) -> ParameterValue {
    ParameterValue::Selector(SelectorValue::new(
        EventKind::new(kind).expect("constant"),
        attributes
            .iter()
            .map(|(key, value)| {
                (
                    AttributeKey::new(*key).expect("fixture"),
                    AttributeValue::String((*value).to_owned()),
                )
            })
            .collect(),
    ))
}

fn catalogued_absence() -> AbsenceSpecification {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(AttributeKey::new(BINDING).expect("fixture")),
        ),
        ("after".to_owned(), selector(AFTER_KIND, &[])),
        ("until".to_owned(), selector(UNTIL_KIND, &[])),
        ("veto".to_owned(), selector(VETO_KIND, &[])),
    ]);
    let evidence = BTreeMap::from([
        (
            "after_source".to_owned(),
            EvidenceBinding::new(source(APPROVAL), SourceTier::Contributing, capable()),
        ),
        (
            "until_source".to_owned(),
            EvidenceBinding::new(source(ACTION), SourceTier::Anchor, capable()),
        ),
        (
            "veto_source".to_owned(),
            EvidenceBinding::new(source(REVOCATION), SourceTier::Anchor, capable()),
        ),
    ]);
    let instance = instantiate(
        PatternId::AbsenceAfterUntil,
        identity(),
        ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant")),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds");
    AbsenceSpecification::try_from(
        instance
            .ir()
            .as_absence()
            .expect("CAT-2 lowers to the absence monitor"),
    )
    .expect("catalogued CAT-2 IR converts")
}

fn text(value: &str) -> AttributeValue {
    AttributeValue::String(value.to_owned())
}

fn occurrence(nanos: i128) -> OccurrenceTime {
    OccurrenceTime::new(
        ClockDomainId::new("example-producer").expect("constant"),
        nanos,
        0,
    )
}

fn observation(nanos: i128) -> ObservationTime {
    ObservationTime::new(SentinelClockId::new("sentinel").expect("constant"), nanos)
}

fn event(
    id: &str,
    sequence: u64,
    kind: &str,
    application: &str,
    attributes: &[(&str, AttributeValue)],
    predecessors: BTreeSet<EventId>,
) -> Event {
    let producer = ProducerEvent::new(
        EventId::new(id).expect("constant"),
        SequenceEpoch::new(1),
        SequenceNumber::new(sequence),
        SchemaVersion::new("example-contract-v1").expect("constant"),
        FilterEpoch::new(1),
        EventKind::new(kind).expect("constant"),
        occurrence(i128::from(sequence)),
        attributes
            .iter()
            .map(|(key, value)| (AttributeKey::new(*key).expect("fixture"), value.clone()))
            .collect(),
        capable(),
        Sensitivity::Internal,
        predecessors,
    )
    .expect("a well-formed producer Event");

    let assurance = SourceAssurance::new(
        source(application),
        SourceTier::Anchor,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("well-formed source assurance");

    Event::observe(
        producer,
        assurance,
        observation(i128::from(sequence)),
        DataClass::Tenant,
    )
}

fn after(id: &str, sequence: u64) -> Event {
    event(
        id,
        sequence,
        AFTER_KIND,
        APPROVAL,
        &[(BINDING, text(REQUEST))],
        BTreeSet::new(),
    )
}

fn after_with_preds(id: &str, sequence: u64, predecessors: BTreeSet<EventId>) -> Event {
    event(
        id,
        sequence,
        AFTER_KIND,
        APPROVAL,
        &[(BINDING, text(REQUEST))],
        predecessors,
    )
}

fn until(id: &str, sequence: u64, predecessors: BTreeSet<EventId>) -> Event {
    event(
        id,
        sequence,
        UNTIL_KIND,
        ACTION,
        &[(BINDING, text(REQUEST))],
        predecessors,
    )
}

fn veto(id: &str, sequence: u64, predecessors: BTreeSet<EventId>) -> Event {
    event(
        id,
        sequence,
        VETO_KIND,
        REVOCATION,
        &[(BINDING, text(REQUEST))],
        predecessors,
    )
}

fn source_evidence(application: &str, last: u64, tier: SourceTier) -> SourceEvidence {
    let producer = ProducerCompletenessSignal::new(
        SequenceEpoch::new(1),
        Some(SequenceNumber::new(last)),
        Some(SequenceNumber::new(last)),
        occurrence(0),
        FilterEpoch::new(1),
        SourceActivity::Active {
            since: occurrence(0),
        },
    )
    .expect("valid producer signal");
    let assurance = SourceAssurance::new(
        source(application),
        tier,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("valid assurance");
    SourceEvidence::new(
        source(application),
        tier,
        capable(),
        SourceCompletenessSignal::observe(producer, assurance, observation(40)),
        Vec::new(),
    )
}

fn closed_sources(
    approval_last: u64,
    action_last: u64,
    revocation_last: u64,
    revocation_tier: SourceTier,
) -> BTreeMap<SourceIdentity, SourceEvidence> {
    BTreeMap::from([
        (
            source(APPROVAL),
            source_evidence(APPROVAL, approval_last, SourceTier::Contributing),
        ),
        (
            source(ACTION),
            source_evidence(ACTION, action_last, SourceTier::Anchor),
        ),
        (
            source(REVOCATION),
            source_evidence(REVOCATION, revocation_last, revocation_tier),
        ),
    ])
}

fn sole_verdict(verdicts: Vec<BindingVerdict>) -> BindingVerdict {
    assert_eq!(verdicts.len(), 1, "expected one Binding, got {verdicts:?}");
    verdicts.into_iter().next().expect("len is 1")
}

fn evaluate(
    spec: AbsenceSpecification,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Vec<BindingVerdict> {
    sentinel_core::evaluate(Evaluator::absence(spec), events, sources).expect("evaluation succeeds")
}

fn seal(
    spec: AbsenceSpecification,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Vec<BindingVerdict> {
    let mut evaluator = Evaluator::absence(spec);
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

/// Veto in (after, until], both sources Anchor, coverage closed → Violate.
#[test]
fn a_catalogued_absence_violation_settles_violate() {
    let after_id = EventId::new("after-1").expect("constant");
    let veto_id = EventId::new("veto-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        veto("veto-1", 1, BTreeSet::from([after_id.clone()])),
        until("until-1", 1, BTreeSet::from([veto_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let spec = catalogued_absence();

    let sealed = sole_verdict(seal(spec.clone(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(
        sealed.verdict().authority(),
        VerdictAuthority::Authoritative
    );

    let batched = sole_verdict(evaluate(spec, &events, &sources));
    assert_eq!(batched.verdict().outcome(), VerdictOutcome::Violate);
}

/// after → until with no veto in the interval, coverage closed → Satisfy.
///
/// A post-close veto from the required revocation source keeps G1 open without
/// falsifying the interval.
#[test]
fn a_clean_opened_interval_settles_satisfy() {
    let after_id = EventId::new("after-1").expect("constant");
    let until_id = EventId::new("until-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        until("until-1", 1, BTreeSet::from([after_id])),
        veto("veto-after-close", 1, BTreeSet::from([until_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let spec = catalogued_absence();

    let sealed = sole_verdict(seal(spec.clone(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        sealed.verdict().authority(),
        VerdictAuthority::Authoritative
    );

    let batched = sole_verdict(evaluate(spec, &events, &sources));
    assert_eq!(batched.verdict().outcome(), VerdictOutcome::Satisfy);
}

/// Contributing-tier-only veto in the interval → Inconclusive(Prefix), never Satisfy.
#[test]
fn a_contributing_tier_only_violation_never_reaches_satisfy_through_the_evaluator() {
    let after_id = EventId::new("after-1").expect("constant");
    let veto_id = EventId::new("veto-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        veto("veto-1", 1, BTreeSet::from([after_id.clone()])),
        until("until-1", 1, BTreeSet::from([veto_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Contributing);
    let spec = catalogued_absence();

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

/// until with no after that reaches it, coverage settled → no Binding Verdict.
///
/// after and veto sit causally *after* until, so every required source is
/// observed (G1 open) while the interval never opens — NoObligation, not Satisfy.
#[test]
fn a_never_opened_settled_until_produces_no_verdict() {
    let until_id = EventId::new("until-1").expect("constant");
    let events = vec![
        until("until-1", 1, BTreeSet::new()),
        after_with_preds("after-1", 1, BTreeSet::from([until_id.clone()])),
        veto("veto-1", 1, BTreeSet::from([until_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let spec = catalogued_absence();

    let sealed = seal(spec.clone(), &events, &sources);
    assert!(
        sealed.is_empty(),
        "never-opened settled until is NoObligation, not Satisfy: {sealed:?}"
    );

    let batched = evaluate(spec, &events, &sources);
    assert!(batched.is_empty());
}

/// Concurrent after with until → InconclusiveObservational.
#[test]
fn a_concurrent_after_is_observationally_uncertain() {
    let until_id = EventId::new("until-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        until("until-1", 1, BTreeSet::new()),
        veto("veto-1", 1, BTreeSet::from([until_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let spec = catalogued_absence();

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::ObservationalUncertainty)
    );
}

/// Empty until set produces no Binding.
#[test]
fn an_empty_until_set_produces_no_binding() {
    let events = vec![after("after-1", 1), veto("veto-1", 1, BTreeSet::new())];
    let spec = catalogued_absence();
    let sealed = seal(
        spec.clone(),
        &events,
        &BTreeMap::from([
            (
                source(APPROVAL),
                source_evidence(APPROVAL, 1, SourceTier::Contributing),
            ),
            (
                source(REVOCATION),
                source_evidence(REVOCATION, 1, SourceTier::Anchor),
            ),
        ]),
    );
    assert!(
        sealed.is_empty(),
        "no until Event means no Binding: {sealed:?}"
    );
}

/// Missing SourceEvidence closes the gate → prefix Inconclusive, never Satisfy.
#[test]
fn a_closed_gate_is_prefix_inconclusive_never_satisfy() {
    let after_id = EventId::new("after-1").expect("constant");
    let until_id = EventId::new("until-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        until("until-1", 1, BTreeSet::from([after_id])),
        veto("veto-1", 1, BTreeSet::from([until_id])),
    ];
    let sources = BTreeMap::from([(
        source(ACTION),
        source_evidence(ACTION, 1, SourceTier::Anchor),
    )]);
    let spec = catalogued_absence();

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
}

#[test]
fn a_warming_absence_specification_cannot_emit_an_authoritative_satisfy() {
    let after_id = EventId::new("after-1").expect("constant");
    let until_id = EventId::new("until-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        until("until-1", 1, BTreeSet::from([after_id])),
        veto("veto-after-close", 1, BTreeSet::from([until_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let observed_at = observation(40);
    let watermarks = BTreeMap::from([
        (
            source(APPROVAL),
            ActivationWatermark::new(
                SequenceEpoch::new(1),
                SequenceNumber::new(1),
                observed_at.clone(),
            ),
        ),
        (
            source(ACTION),
            ActivationWatermark::new(
                SequenceEpoch::new(1),
                SequenceNumber::new(1),
                observed_at.clone(),
            ),
        ),
        (
            source(REVOCATION),
            ActivationWatermark::new(SequenceEpoch::new(1), SequenceNumber::new(1), observed_at),
        ),
    ]);
    let barrier =
        ActivationBarrier::new(FilterEpoch::new(2), watermarks).expect("three required sources");
    let spec = catalogued_absence().with_activation_barrier(barrier);

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(sealed.verdict().authority(), VerdictAuthority::Provisional);
}

#[test]
fn a_warming_absence_specification_still_emits_a_provisional_violate() {
    let after_id = EventId::new("after-1").expect("constant");
    let veto_id = EventId::new("veto-1").expect("constant");
    let events = vec![
        after("after-1", 1),
        veto("veto-1", 1, BTreeSet::from([after_id.clone()])),
        until("until-1", 1, BTreeSet::from([veto_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let observed_at = observation(40);
    let watermarks = BTreeMap::from([
        (
            source(APPROVAL),
            ActivationWatermark::new(
                SequenceEpoch::new(1),
                SequenceNumber::new(1),
                observed_at.clone(),
            ),
        ),
        (
            source(ACTION),
            ActivationWatermark::new(
                SequenceEpoch::new(1),
                SequenceNumber::new(1),
                observed_at.clone(),
            ),
        ),
        (
            source(REVOCATION),
            ActivationWatermark::new(SequenceEpoch::new(1), SequenceNumber::new(1), observed_at),
        ),
    ]);
    let barrier =
        ActivationBarrier::new(FilterEpoch::new(2), watermarks).expect("three required sources");
    let spec = catalogued_absence().with_activation_barrier(barrier);

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(sealed.verdict().authority(), VerdictAuthority::Provisional);
}
