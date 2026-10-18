//! CAT-3 traversal through [`Evaluator`]: CausalIndex, Slice, EvidenceGate,
//! and O-03 retirement while a Between Scope is still open.
//!
//! Observation-constructed table tests live in `existence_decision_table.rs`.
//! Cases: Satisfy, Violate, concurrent, retirement-while-open, (d2) no demotion
//! of an emitted Satisfy, retire-then-Coverage must not promote, Inactivity.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, ProducerContract, SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::existence::ExistenceSpecification;
use sentinel_core::slice::{
    Binding, RetirementBreach, RetirementCause, RetirementSignal, ScopeHorizon,
};
use sentinel_core::specification::RetirementPolicy;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::{
    AttributeKey, AttributeValue, ClockDomainId, DataClass, Event, EventId, EventKind, FilterEpoch,
    InconclusiveReason, Integrity, MonitoredApplicationId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId,
    SequenceEpoch, SequenceNumber, SourceActivity, SourceAssurance, SourceCapabilities,
    SourceCapability, SourceCompletenessSignal, SourceEvidence, SourceId, SourceIdentity,
    SourceTier, VerdictAuthority, VerdictOutcome,
};

const BINDING: &str = "example.request.id";
const SAGA: &str = "saga-1";
const OPEN_KIND: &str = "example.saga.opened";
const REQUIRED_KIND: &str = "example.saga.compensated";
const CLOSE_KIND: &str = "example.saga.terminated";
const ORCHESTRATOR: &str = "example-saga-orchestrator";
const MONEY: &str = "example-money-system";
const JOURNAL: &str = "example-broker-journal";

fn identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.compensation-before-terminate".to_owned(),
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

fn catalogued_existence() -> ExistenceSpecification {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(AttributeKey::new(BINDING).expect("fixture")),
        ),
        ("open".to_owned(), selector(OPEN_KIND, &[])),
        ("required".to_owned(), selector(REQUIRED_KIND, &[])),
        ("close".to_owned(), selector(CLOSE_KIND, &[])),
    ]);
    let evidence = BTreeMap::from([
        (
            "open_source".to_owned(),
            EvidenceBinding::new(source(ORCHESTRATOR), SourceTier::Contributing, capable()),
        ),
        (
            "required_source".to_owned(),
            EvidenceBinding::new(source(MONEY), SourceTier::Anchor, capable()),
        ),
        (
            "close_source".to_owned(),
            EvidenceBinding::new(source(JOURNAL), SourceTier::Contributing, capable()),
        ),
    ]);
    let instance = instantiate(
        PatternId::ExistenceBeforeScopeClose,
        identity(),
        ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant")),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds");
    ExistenceSpecification::try_from(
        instance
            .ir()
            .as_existence()
            .expect("CAT-3 lowers to the existence monitor"),
    )
    .expect("catalogued CAT-3 IR converts")
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

fn open_evt(id: &str, sequence: u64) -> Event {
    event(
        id,
        sequence,
        OPEN_KIND,
        ORCHESTRATOR,
        &[(BINDING, text(SAGA))],
        BTreeSet::new(),
    )
}

fn required_evt(id: &str, sequence: u64, predecessors: BTreeSet<EventId>) -> Event {
    event(
        id,
        sequence,
        REQUIRED_KIND,
        MONEY,
        &[(BINDING, text(SAGA))],
        predecessors,
    )
}

fn close_evt(id: &str, sequence: u64, predecessors: BTreeSet<EventId>) -> Event {
    event(
        id,
        sequence,
        CLOSE_KIND,
        JOURNAL,
        &[(BINDING, text(SAGA))],
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
    open_last: u64,
    required_last: u64,
    close_last: u64,
    required_tier: SourceTier,
) -> BTreeMap<SourceIdentity, SourceEvidence> {
    BTreeMap::from([
        (
            source(ORCHESTRATOR),
            source_evidence(ORCHESTRATOR, open_last, SourceTier::Contributing),
        ),
        (
            source(MONEY),
            source_evidence(MONEY, required_last, required_tier),
        ),
        (
            source(JOURNAL),
            source_evidence(JOURNAL, close_last, SourceTier::Contributing),
        ),
    ])
}

fn prefix_detail(verdict: &BindingVerdict) -> String {
    let entries: Vec<_> = verdict
        .verdict()
        .uncertainty()
        .into_iter()
        .filter(|entry| entry.reason == InconclusiveReason::PrefixUncertainty)
        .collect();
    assert_eq!(entries.len(), 1, "exactly one PREFIX_UNCERTAINTY entry");
    entries[0].detail.clone()
}

fn settled(step: &Step) -> &[BindingVerdict] {
    match step {
        Step::Settled(verdicts) => verdicts,
        other => panic!("expected Settled, got {other:?}"),
    }
}

fn sole_verdict(verdicts: Vec<BindingVerdict>) -> BindingVerdict {
    assert_eq!(verdicts.len(), 1, "expected one Binding, got {verdicts:?}");
    verdicts.into_iter().next().expect("len is 1")
}

fn seal(
    spec: ExistenceSpecification,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Vec<BindingVerdict> {
    let mut evaluator = Evaluator::existence(spec);
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

/// open → required in (open, close] → close, Coverage settled → Satisfy.
#[test]
fn a_catalogued_existence_with_required_in_interval_settles_satisfy() {
    let open_id = EventId::new("open-1").expect("constant");
    let required_id = EventId::new("req-1").expect("constant");
    let events = vec![
        open_evt("open-1", 1),
        required_evt("req-1", 1, BTreeSet::from([open_id.clone()])),
        close_evt("close-1", 1, BTreeSet::from([required_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let spec = catalogued_existence();
    assert_eq!(
        spec.retirement_policy(),
        RetirementPolicy::Deadline,
        "catalogued CAT-3 declares Deadline"
    );

    let sealed = sole_verdict(seal(spec, &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        sealed.verdict().authority(),
        VerdictAuthority::Authoritative
    );
}

/// open → close with no required in `(open, close]`, Coverage settled, Anchor → Violate.
///
/// A post-close compensation from the money system keeps G1 open (the required
/// source supplied Events) without discharging the interval — silence *inside*
/// the Scope, not an absent producer.
#[test]
fn a_catalogued_existence_missing_required_settles_violate() {
    let open_id = EventId::new("open-1").expect("constant");
    let close_id = EventId::new("close-1").expect("constant");
    let events = vec![
        open_evt("open-1", 1),
        close_evt("close-1", 1, BTreeSet::from([open_id])),
        required_evt("req-after-close", 1, BTreeSet::from([close_id])),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let sealed = sole_verdict(seal(catalogued_existence(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Violate);
}

/// Concurrent open with close → ObservationalUncertainty.
#[test]
fn a_concurrent_open_and_close_is_observationally_uncertain() {
    let events = vec![
        open_evt("open-1", 1),
        close_evt("close-1", 1, BTreeSet::new()),
        required_evt(
            "req-1",
            1,
            BTreeSet::from([EventId::new("close-1").expect("constant")]),
        ),
    ];
    let sources = closed_sources(1, 1, 1, SourceTier::Anchor);
    let sealed = sole_verdict(seal(catalogued_existence(), &events, &sources));
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::ObservationalUncertainty)
    );
}

/// Retirement while Scope still open (no close) → prefix Inconclusive with O-03.
#[test]
fn retiring_an_open_between_scope_emits_prefix_inconclusive() {
    let mut evaluator = Evaluator::existence(catalogued_existence());
    evaluator
        .step(Input::event(open_evt("open-1", 1)))
        .expect("open steps");
    // Withhold the money source so the gate stays closed while Coverage is
    // non-empty — the retired emission licence fixture from O-03.
    for evidence in [
        source_evidence(ORCHESTRATOR, 1, SourceTier::Contributing),
        source_evidence(JOURNAL, 0, SourceTier::Contributing),
    ] {
        evaluator
            .step(Input::source(evidence))
            .expect("partial sources step");
    }

    let step = evaluator
        .step(Input::retire(RetirementSignal::new(
            Binding::new(SAGA),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Deadline,
        )))
        .expect("Deadline retirement is admissible on catalogued CAT-3");
    let sealed = sole_verdict(settled(&step).to_vec());
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    let detail = prefix_detail(&sealed);
    assert!(
        detail.starts_with("O-03 scope retired: deadline at"),
        "got {detail}"
    );
}

/// Clause (d2): retiring after a Coverage-licensed Satisfy has already emitted
/// is refused — the alternative would silently replace Satisfy with Inconclusive.
#[test]
fn retirement_does_not_demote_a_coverage_licensed_satisfy() {
    let open_id = EventId::new("open-1").expect("constant");
    let required_id = EventId::new("req-1").expect("constant");
    let events = vec![
        open_evt("open-1", 1),
        required_evt("req-1", 1, BTreeSet::from([open_id.clone()])),
        close_evt("close-1", 1, BTreeSet::from([required_id])),
    ];
    let mut evaluator = Evaluator::existence(catalogued_existence());
    for event in &events {
        evaluator
            .step(Input::event(event.clone()))
            .expect("events step");
    }
    for evidence in closed_sources(1, 1, 1, SourceTier::Anchor).into_values() {
        evaluator
            .step(Input::source(evidence))
            .expect("sources step");
    }
    assert!(
        evaluator.is_emitted(&Binding::new(SAGA)),
        "Coverage-licensed Satisfy must have emitted before the retire attempt"
    );

    assert_eq!(
        evaluator.step(Input::retire(RetirementSignal::new(
            Binding::new(SAGA),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Deadline,
        ))),
        Err(CoreError::Retirement(
            RetirementBreach::BindingAlreadyEmitted
        ))
    );

    let sealed = sole_verdict(evaluator.seal().expect("seal succeeds").into_verdicts());
    assert_eq!(
        sealed.verdict().outcome(),
        VerdictOutcome::Satisfy,
        "refused retirement must leave the emitted Satisfy intact"
    );
}

/// Pin 2: retire while Coverage is still open, then settle Coverage — must not
/// invent a Satisfy after O-03 retirement.
#[test]
fn retire_then_coverage_must_not_promote_to_satisfy() {
    let open_id = EventId::new("open-1").expect("constant");
    let required_id = EventId::new("req-1").expect("constant");
    let mut evaluator = Evaluator::existence(catalogued_existence());
    for event in [
        open_evt("open-1", 1),
        required_evt("req-1", 1, BTreeSet::from([open_id.clone()])),
        close_evt("close-1", 1, BTreeSet::from([required_id])),
    ] {
        evaluator.step(Input::event(event)).expect("events step");
    }
    // Coverage open: withhold the money source so no Satisfy can emit yet.
    for evidence in [
        source_evidence(ORCHESTRATOR, 1, SourceTier::Contributing),
        source_evidence(JOURNAL, 1, SourceTier::Contributing),
    ] {
        evaluator
            .step(Input::source(evidence))
            .expect("partial sources step");
    }

    let retired = evaluator
        .step(Input::retire(RetirementSignal::new(
            Binding::new(SAGA),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Deadline,
        )))
        .expect("Deadline retirement while Coverage open is admissible");
    let retired_verdict = sole_verdict(settled(&retired).to_vec());
    assert_eq!(
        retired_verdict.verdict().outcome(),
        VerdictOutcome::Inconclusive
    );

    // Complete Coverage after retirement — assess must stay Retired, not Satisfy.
    evaluator
        .step(Input::source(source_evidence(MONEY, 1, SourceTier::Anchor)))
        .expect("completing Coverage after retire must not error");
    let sealed = sole_verdict(evaluator.seal().expect("seal succeeds").into_verdicts());
    assert_eq!(
        sealed.verdict().outcome(),
        VerdictOutcome::Inconclusive,
        "later Coverage must not invent Satisfy after O-03 retirement"
    );
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    let detail = prefix_detail(&sealed);
    assert!(
        detail.starts_with("O-03 scope retired: deadline at"),
        "got {detail}"
    );
}

/// Pin 2 twin: retire while Coverage open, then settle absence → must not Violate.
#[test]
fn retire_then_coverage_must_not_promote_to_violate() {
    let open_id = EventId::new("open-1").expect("constant");
    let mut evaluator = Evaluator::existence(catalogued_existence());
    for event in [
        open_evt("open-1", 1),
        close_evt("close-1", 1, BTreeSet::from([open_id])),
    ] {
        evaluator.step(Input::event(event)).expect("events step");
    }
    for evidence in [
        source_evidence(ORCHESTRATOR, 1, SourceTier::Contributing),
        source_evidence(JOURNAL, 1, SourceTier::Contributing),
    ] {
        evaluator
            .step(Input::source(evidence))
            .expect("partial sources step");
    }

    evaluator
        .step(Input::retire(RetirementSignal::new(
            Binding::new(SAGA),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Deadline,
        )))
        .expect("Deadline retirement while Coverage open is admissible");

    evaluator
        .step(Input::source(source_evidence(MONEY, 1, SourceTier::Anchor)))
        .expect("completing Coverage after retire must not error");
    let sealed = sole_verdict(evaluator.seal().expect("seal succeeds").into_verdicts());
    assert_eq!(
        sealed.verdict().outcome(),
        VerdictOutcome::Inconclusive,
        "later Coverage must not invent Violate after O-03 retirement"
    );
    assert_eq!(
        sealed.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
}

/// Inactivity is grammar-only on the Pattern, but the Core mechanism accepts it
/// on a hand-built fixture that declares the policy.
#[test]
fn inactivity_retirement_is_admissible_on_a_hand_built_fixture() {
    let mut spec = catalogued_existence();
    // Override via with_retirement_policy — catalogue declares Deadline only.
    spec = spec.with_retirement_policy(RetirementPolicy::Inactivity);
    assert_eq!(spec.retirement_policy(), RetirementPolicy::Inactivity);

    let mut evaluator = Evaluator::existence(spec);
    evaluator
        .step(Input::event(open_evt("open-1", 1)))
        .expect("open steps");
    for evidence in [
        source_evidence(ORCHESTRATOR, 1, SourceTier::Contributing),
        source_evidence(JOURNAL, 0, SourceTier::Contributing),
    ] {
        evaluator
            .step(Input::source(evidence))
            .expect("partial sources step");
    }

    let step = evaluator
        .step(Input::retire(RetirementSignal::new(
            Binding::new(SAGA),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Inactivity,
        )))
        .expect("Inactivity is admissible when the Specification declares it");
    let sealed = sole_verdict(settled(&step).to_vec());
    assert_eq!(sealed.verdict().outcome(), VerdictOutcome::Inconclusive);
    let detail = prefix_detail(&sealed);
    assert!(
        detail.starts_with("O-03 scope retired: inactivity at"),
        "got {detail}"
    );
}

/// Catalogued CAT-3 refuses an Inactivity cause (d0) — Pattern declares Deadline only.
#[test]
fn catalogued_cat3_refuses_an_inactivity_cause() {
    let mut evaluator = Evaluator::existence(catalogued_existence());
    evaluator
        .step(Input::event(open_evt("open-1", 1)))
        .expect("open steps");
    assert_eq!(
        evaluator.step(Input::retire(RetirementSignal::new(
            Binding::new(SAGA),
            ScopeHorizon::new(observation(10_000)),
            RetirementCause::Inactivity,
        ))),
        Err(CoreError::Retirement(RetirementBreach::CauseNotDeclared))
    );
}
