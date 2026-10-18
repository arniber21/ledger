//! CAT-5 traversal through [`Evaluator`]: Slice, EvidenceGate, cold round trip.
//!
//! Observation-constructed table rows live in `aggregate_decision_table.rs`.
//! This suite drives the whole seam — catalogued instantiation, Slice
//! construction, the declared query, and the Verdict each settled cold answer
//! licenses — and pins issue #173's AMEND 3 and AMEND 4 at the envelope:
//!
//! - a cold `Fails` settles Authoritative `Inconclusive(epistemic)`, mints no
//!   `Violate`, assembles no decisive-violation basis, and puts none of the
//!   witnesses the cold path named into `anchored_event_ids`;
//! - a cold `Refused` settles Authoritative `Inconclusive(epistemic)`;
//! - a cold `Holds` settles `Satisfy` only once Evidence Coverage has closed;
//! - the obligation keys on the trigger, so members alone raise none;
//! - *no* answer the cold path can give mints a `Violate`, which is the whole
//!   family's claim rather than the `Fails` row's.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, ProducerContract, SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::aggregate::AggregateSpecification;
use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::facts::{
    ColdGuardId, ColdWitness, FactRequest, FactResponse, GuardConclusion, RefusalReason,
};
use sentinel_core::verdict::{BindingVerdict, AGGREGATE_FAILS_DETAIL, REFUSED_GUARD_DETAIL};
use sentinel_model::{
    AttributeKey, AttributeValue, ClockDomainId, DataClass, Event, EventId, EventKind, FilterEpoch,
    InconclusiveReason, Integrity, MonitoredApplicationId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId,
    SequenceEpoch, SequenceNumber, SettlementBasisKind, SourceActivity, SourceAssurance,
    SourceCapabilities, SourceCapability, SourceCompletenessSignal, SourceEvidence, SourceId,
    SourceIdentity, SourceTier, TierSettlement, VerdictAuthority, VerdictOutcome,
};

const BINDING: &str = "example.principal.id";
const PRINCIPAL: &str = "principal-1";
const TRIGGER_KIND: &str = "example.assessment.started";
const MEMBER_KIND: &str = "example.transfer.attempted";
const ASSESSOR: &str = "example-assessor";
const LEDGER: &str = "example-ledger";
const LOOKBACK: u64 = 16;
const THRESHOLD: u64 = 3;

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// A complete under-threshold lookback settles Authoritative `Satisfy`.
#[test]
fn a_cold_holds_over_closed_coverage_settles_authoritative_satisfy() {
    let verdict = settle_with(&GuardConclusion::Holds);

    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        verdict.verdict().authority(),
        VerdictAuthority::Authoritative
    );
    assert_eq!(verdict.verdict().cold_path(), TierSettlement::Settled);
    assert_eq!(
        verdict.verdict().settlement_basis(),
        Some(SettlementBasisKind::CompleteEvidence)
    );
    assert_eq!(
        anchored(&verdict),
        BTreeSet::from([EventId::new("trigger-1").expect("constant")]),
        "the trigger is the Event-local Scope close the pass rests on"
    );
}

/// AMEND 3: a cold `Fails` settles Authoritative `Inconclusive`, never `Violate`.
#[test]
fn a_cold_fails_settles_authoritative_inconclusive_not_violate() {
    let witness = ColdWitness::new(
        EventId::new("member-1").expect("constant"),
        source(LEDGER),
        (SequenceEpoch::new(1), SequenceNumber::new(4)),
    );
    let verdict = settle_with(&GuardConclusion::Fails {
        witnesses: BTreeSet::from([witness]),
    });

    assert_eq!(
        verdict.verdict().outcome(),
        VerdictOutcome::Inconclusive,
        "CAT-5 mints no Violate: cold-witness composition is #20's trust boundary"
    );
    assert_eq!(
        verdict.verdict().inconclusive_reason(),
        Some(InconclusiveReason::EpistemicUncertainty)
    );
    assert_eq!(
        verdict.verdict().authority(),
        VerdictAuthority::Authoritative,
        "the count settled, so the finding is not merely provisional"
    );
    assert_eq!(verdict.verdict().cold_path(), TierSettlement::Settled);
    assert_eq!(
        verdict.verdict().settlement_basis(),
        None,
        "no decisive-violation basis is assembled from a cold finding"
    );
    assert!(
        verdict.verdict().decisive_witness_event_ids().is_empty(),
        "a cold witness may not be cited as a decisive Event"
    );
    assert_eq!(
        anchored(&verdict),
        BTreeSet::new(),
        "a non-pass cites no anchor Event, and never the cold path's witnesses"
    );
    assert!(
        carries(&verdict, AGGREGATE_FAILS_DETAIL),
        "the envelope must say why the finding stopped short of a Violate"
    );
}

/// A terminal `Refused` settles Authoritative `Inconclusive(epistemic)`.
#[test]
fn a_cold_refusal_settles_authoritative_inconclusive_epistemic() {
    let verdict = settle_with(&GuardConclusion::Refused {
        reason: RefusalReason::EvidenceReaped,
    });

    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.verdict().inconclusive_reason(),
        Some(InconclusiveReason::EpistemicUncertainty)
    );
    assert_eq!(
        verdict.verdict().authority(),
        VerdictAuthority::Authoritative
    );
    assert!(carries(&verdict, REFUSED_GUARD_DETAIL));
    assert_eq!(anchored(&verdict), BTreeSet::new());
}

/// AMEND 4: a reaped lookback is what `Refused` reports, and it never passes.
///
/// `sentinel-coldpath`'s `CountWithin` refuses an incomplete lookback rather
/// than holding it, so the incompleteness reaches Core as this answer.
#[test]
fn an_incomplete_lookback_never_settles_satisfy() {
    for reason in [
        RefusalReason::EvidenceReaped,
        RefusalReason::EvidenceNeverRetained,
        RefusalReason::CausalEdgesContradictory,
        RefusalReason::GuardUnknown,
    ] {
        let verdict = settle_with(&GuardConclusion::Refused { reason });
        assert_ne!(
            verdict.verdict().outcome(),
            VerdictOutcome::Satisfy,
            "{reason:?} must never discharge the count obligation"
        );
    }
}

/// An unsettled count is prefix-open, and the query is still outstanding.
#[test]
fn an_unanswered_count_is_prefix_open_and_still_asked() {
    let (evaluator, request, emitted) = at_the_request();

    let verdict = sole(emitted);
    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty),
        "a declared but unanswered count is more evidence outstanding, not a refusal"
    );
    assert_eq!(request.queries().len(), 1, "one guard, one Binding");
    assert!(!evaluator.outstanding().is_empty());
}

/// A deferred answer does not settle, so the query is re-issued.
#[test]
fn a_deferred_answer_leaves_the_query_outstanding() {
    let (mut evaluator, request, _) = at_the_request();

    evaluator
        .resume(answer(
            &request,
            &GuardConclusion::Deferred {
                reason: sentinel_core::facts::DeferralReason::ProjectionBehindFloor,
            },
        ))
        .expect("a deferral is a legal answer");

    assert!(
        !evaluator.outstanding().is_empty(),
        "a deferral re-issues rather than settling"
    );
}

/// No answer the cold path can give, and no absence of one, mints a `Violate`.
///
/// `core/src/aggregate.rs`'s header states plainly that **CAT-5 mints no
/// `Violate` today**, and names this test as the pin. It is a different claim
/// from `a_cold_fails_settles_authoritative_inconclusive_not_violate`, which
/// establishes it for the one answer that would be a `Violate` if any were:
/// this asserts it for the whole family, so the mapping in
/// `verdict::build_aggregate` cannot grow a `Violate` arm for some other row
/// while the `Fails` row stays honest.
///
/// `AggregateOutcome` carries no `Violate` variant, so `decide` cannot return
/// one and the only place one could appear is that mapping. Every
/// `GuardConclusion` variant is driven through it, both `Refused` reasons that
/// mean "reaped" and "never asked for", and both ways a count can be absent —
/// no guard bound, and a guard bound but unanswered. Verdicts emitted *before*
/// the answer arrives are checked too, because a prefix emission is a Verdict a
/// reader can act on.
#[test]
fn aggregate_mints_no_violate() {
    let witness = ColdWitness::new(
        EventId::new("member-1").expect("constant"),
        source(LEDGER),
        (SequenceEpoch::new(1), SequenceNumber::new(4)),
    );
    let mut checked = 0_usize;

    // Every settling answer, plus the deferral that re-issues instead.
    for conclusion in [
        GuardConclusion::Holds,
        GuardConclusion::Fails {
            witnesses: BTreeSet::from([witness]),
        },
        GuardConclusion::Fails {
            witnesses: BTreeSet::new(),
        },
        GuardConclusion::Refused {
            reason: RefusalReason::EvidenceReaped,
        },
        GuardConclusion::Refused {
            reason: RefusalReason::GuardUnknown,
        },
        GuardConclusion::Deferred {
            reason: sentinel_core::facts::DeferralReason::ProjectionBehindFloor,
        },
    ] {
        let (mut evaluator, request, emitted) = at_the_request();
        let mut verdicts = emitted;
        // Not `expect`: a `Violate` arm added to `build_aggregate` trips this
        // as `CoreError::Verdict(MissingSettlementBasis)` before any assertion
        // below runs, because `build_aggregate` assembles no basis and
        // `sentinel-model` refuses a `Violate` without one. That second refusal
        // is welcome, but the message a red build shows should name the claim
        // rather than the model error it surfaced as.
        let step = evaluator
            .resume(answer(&request, &conclusion))
            .unwrap_or_else(|error| {
                panic!(
                    "{conclusion:?} made the Verdict unconstructible ({error:?}); CAT-5 mints no \
                     Violate, so no answer may need a settlement basis"
                )
            });
        if let Step::Settled(settled) = step {
            verdicts.extend(settled);
        }
        assert!(
            !verdicts.is_empty(),
            "{conclusion:?} produced no Verdict at all, so this asserts nothing"
        );
        for verdict in &verdicts {
            assert_ne!(
                verdict.verdict().outcome(),
                VerdictOutcome::Violate,
                "{conclusion:?} minted a Violate; #20 owns the cold-witness trust boundary"
            );
            checked += 1;
        }
    }

    // A count that was never asked for: no guard bound, so nothing can answer.
    // Reachable because `with_cold_guard` is the additive builder rather than a
    // constructor argument, so a hand-built Specification starts guard-free.
    let catalogued = catalogued_aggregate();
    let mut evaluator = Evaluator::aggregate(AggregateSpecification::new(
        catalogued.identity().clone(),
        catalogued.binding_attribute().clone(),
        catalogued.trigger().clone(),
        catalogued.member().clone(),
        catalogued.lookback_sequences(),
        catalogued.threshold(),
        catalogued.required_sources().clone(),
        catalogued.schema_version().clone(),
    ));
    let mut unasked = Vec::new();
    for event in fixture_events() {
        if let Step::Settled(settled) = evaluator
            .step(Input::event(event))
            .expect("stepping an Event succeeds")
        {
            unasked.extend(settled);
        }
    }
    for evidence in closed_sources().values() {
        if let Step::Settled(settled) = evaluator
            .step(Input::source(evidence.clone()))
            .expect("stepping source evidence succeeds")
        {
            unasked.extend(settled);
        }
    }
    unasked.extend(evaluator.seal().expect("sealing succeeds").into_verdicts());
    assert!(
        !unasked.is_empty(),
        "an unasked count produced no Verdict, so this asserts nothing"
    );
    for verdict in &unasked {
        assert_ne!(
            verdict.verdict().outcome(),
            VerdictOutcome::Violate,
            "an unasked count minted a Violate"
        );
        assert_ne!(
            verdict.verdict().outcome(),
            VerdictOutcome::Satisfy,
            "an unasked count is missing evidence, and missing evidence is never a pass"
        );
        checked += 1;
    }

    assert!(checked > 0, "no Verdict was examined");
}

/// The obligation keys on the trigger: members alone establish nothing.
#[test]
fn members_without_a_trigger_raise_no_obligation() {
    let mut evaluator = Evaluator::aggregate(catalogued_aggregate());
    for event in [member("member-1", 1), member("member-2", 2)] {
        evaluator
            .step(Input::event(event))
            .expect("stepping an Event succeeds");
    }
    for evidence in closed_sources().values() {
        evaluator
            .step(Input::source(evidence.clone()))
            .expect("stepping source evidence succeeds");
    }

    assert!(
        evaluator.outstanding().is_empty(),
        "no trigger, no obligation, so no cold question is asked"
    );
    assert!(
        evaluator
            .seal()
            .expect("sealing succeeds")
            .into_verdicts()
            .is_empty(),
        "an evidence set full of members and no trigger establishes nothing"
    );
}

/// Two guards answering `Fails` and `Refused` still assemble one Verdict.
///
/// `with_cold_guard` is additive and public, so a Binding can hold two settled
/// answers whose envelope sentences both belong to
/// `InconclusiveReason::EpistemicUncertainty` — the case
/// `CountEvidence::of`'s Fails-outranks-Refused ordering exists to decide.
/// `sentinel_model`'s `validate_uncertainty` admits at most one entry per
/// reason, so writing the two sentences as two entries made this Verdict
/// unconstructible: it came back as `CoreError::Verdict`, which
/// `sentinel-core` reserves for its own bugs rather than for evidence
/// conditions. The two fold into one entry instead, and both sentences survive.
#[test]
fn two_guards_answering_fails_and_refused_still_build_one_verdict() {
    let second = ColdGuardId::new("aggregation.count-within", "cw:second-opinion|t=3|lb=16");
    let (mut evaluator, request, _) =
        at_the_request_for(catalogued_aggregate().with_cold_guard(second.clone()));
    assert_eq!(request.queries().len(), 2, "one query per declared guard");

    let settled_at = BTreeMap::from([
        (
            source(ASSESSOR),
            (SequenceEpoch::new(1), SequenceNumber::new(4)),
        ),
        (
            source(LEDGER),
            (SequenceEpoch::new(1), SequenceNumber::new(4)),
        ),
    ]);
    let responses = request
        .queries()
        .iter()
        .map(|query| {
            let conclusion = if query.guard() == &second {
                GuardConclusion::Refused {
                    reason: RefusalReason::EvidenceReaped,
                }
            } else {
                GuardConclusion::Fails {
                    witnesses: BTreeSet::new(),
                }
            };
            FactResponse::new(query.clone(), conclusion, settled_at.clone())
        })
        .collect();

    let verdict = match evaluator
        .resume(responses)
        .expect("two settling answers are not a Core bug")
    {
        Step::Settled(verdicts) => sole(verdicts),
        other => panic!("expected Settled, got {other:?}"),
    };

    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(
        verdict.verdict().outcome(),
        VerdictOutcome::Satisfy,
        "a refusal alongside a finding is still never a pass"
    );
    let epistemic: Vec<_> = verdict
        .verdict()
        .uncertainty()
        .into_iter()
        .filter(|entry| entry.reason == InconclusiveReason::EpistemicUncertainty)
        .collect();
    assert_eq!(
        epistemic.len(),
        1,
        "at most one entry per reason, or try_from refuses the Verdict"
    );
    let detail = &epistemic[0].detail;
    assert!(
        detail.contains(AGGREGATE_FAILS_DETAIL) && detail.contains(REFUSED_GUARD_DETAIL),
        "folding must not drop either sentence: {detail}"
    );
}

/// One query per Binding per guard, keyed on the Binding the trigger names.
#[test]
fn one_query_is_declared_per_binding_that_raised_an_obligation() {
    let (_, request, _) = at_the_request();
    let query = request.queries().first().expect("one query");
    assert_eq!(query.binding().as_str(), PRINCIPAL);
    assert_eq!(
        query.guard(),
        &sentinel_core::aggregate::count_within_cold_guard_id(
            catalogued_aggregate().member(),
            THRESHOLD,
            LOOKBACK,
        )
    );
}

// ---------------------------------------------------------------------------
// Driving the seam
// ---------------------------------------------------------------------------

/// Steps the fixture to the point the cold question has been asked.
///
/// The prefix-open Verdict the evaluator emits before the count settles comes
/// back with the request, because an emission drains before the query it
/// declared is issued: a hot settlement is never swallowed by its own request.
fn at_the_request() -> (Evaluator, FactRequest, Vec<BindingVerdict>) {
    at_the_request_for(catalogued_aggregate())
}

fn at_the_request_for(
    specification: AggregateSpecification,
) -> (Evaluator, FactRequest, Vec<BindingVerdict>) {
    let mut evaluator = Evaluator::aggregate(specification);
    let mut emitted = Vec::new();
    let mut record = |step: Step| {
        if let Step::Settled(verdicts) = step {
            emitted.extend(verdicts);
        }
    };
    for event in fixture_events() {
        record(
            evaluator
                .step(Input::event(event))
                .expect("stepping an Event succeeds"),
        );
    }
    for evidence in closed_sources().values() {
        record(
            evaluator
                .step(Input::source(evidence.clone()))
                .expect("stepping source evidence succeeds"),
        );
    }
    let request = match evaluator
        .resume(Vec::new())
        .expect("an empty resume is legal")
    {
        Step::NeedsFacts(request) => request,
        other => panic!("expected NeedsFacts, got {other:?}"),
    };
    (evaluator, request, emitted)
}

/// Answers the declared query and returns the Verdict the answer licenses.
fn settle_with(conclusion: &GuardConclusion) -> BindingVerdict {
    let (mut evaluator, request, _) = at_the_request();
    match evaluator
        .resume(answer(&request, conclusion))
        .expect("a settling answer is not a Core bug")
    {
        Step::Settled(verdicts) => sole(verdicts),
        other => panic!("expected Settled, got {other:?}"),
    }
}

fn answer(request: &FactRequest, conclusion: &GuardConclusion) -> Vec<FactResponse> {
    let settled_at = BTreeMap::from([
        (
            source(ASSESSOR),
            (SequenceEpoch::new(1), SequenceNumber::new(4)),
        ),
        (
            source(LEDGER),
            (SequenceEpoch::new(1), SequenceNumber::new(4)),
        ),
    ]);
    request
        .queries()
        .iter()
        .map(|query| FactResponse::new(query.clone(), conclusion.clone(), settled_at.clone()))
        .collect()
}

fn sole(verdicts: Vec<BindingVerdict>) -> BindingVerdict {
    assert_eq!(verdicts.len(), 1, "expected one Binding, got {verdicts:?}");
    verdicts.into_iter().next().expect("len is 1")
}

fn anchored(verdict: &BindingVerdict) -> BTreeSet<EventId> {
    verdict
        .verdict()
        .anchored_event_ids()
        .into_values()
        .flatten()
        .collect()
}

fn carries(verdict: &BindingVerdict, detail: &str) -> bool {
    verdict
        .verdict()
        .uncertainty()
        .iter()
        .any(|entry| entry.detail == detail)
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

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

fn selector(kind: &str) -> ParameterValue {
    ParameterValue::Selector(SelectorValue::new(
        EventKind::new(kind).expect("constant"),
        BTreeMap::new(),
    ))
}

fn catalogued_aggregate() -> AggregateSpecification {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(AttributeKey::new(BINDING).expect("constant")),
        ),
        ("trigger".to_owned(), selector(TRIGGER_KIND)),
        ("member".to_owned(), selector(MEMBER_KIND)),
        (
            "lookback".to_owned(),
            ParameterValue::SequenceLookbackBound(LOOKBACK),
        ),
        (
            "threshold".to_owned(),
            ParameterValue::CountThreshold(THRESHOLD),
        ),
    ]);
    let evidence = BTreeMap::from([
        (
            "trigger_source".to_owned(),
            EvidenceBinding::new(source(ASSESSOR), SourceTier::Anchor, capable()),
        ),
        (
            "member_source".to_owned(),
            EvidenceBinding::new(source(LEDGER), SourceTier::Anchor, capable()),
        ),
    ]);
    let instance = instantiate(
        PatternId::AggregationCountWithin,
        FilterIdentity {
            specification_id: "example.transfer-rate-cap".to_owned(),
            specification_version: "v1".to_owned(),
        },
        ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant")),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds");
    AggregateSpecification::try_from(
        instance
            .ir()
            .as_aggregate()
            .expect("CAT-5 lowers to the aggregate monitor"),
    )
    .expect("catalogued CAT-5 IR converts")
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
        BTreeMap::from([(
            AttributeKey::new(BINDING).expect("constant"),
            AttributeValue::String(PRINCIPAL.to_owned()),
        )]),
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

fn member(id: &str, sequence: u64) -> Event {
    event(id, sequence, MEMBER_KIND, LEDGER, BTreeSet::new())
}

fn fixture_events() -> Vec<Event> {
    let first = member("member-1", 1);
    let second = member("member-2", 2);
    vec![
        first.clone(),
        second.clone(),
        event(
            "trigger-1",
            1,
            TRIGGER_KIND,
            ASSESSOR,
            BTreeSet::from([first.id(), second.id()]),
        ),
    ]
}

fn source_evidence(application: &str, last: u64) -> SourceEvidence {
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
        SourceTier::Anchor,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("valid assurance");
    SourceEvidence::new(
        source(application),
        SourceTier::Anchor,
        capable(),
        SourceCompletenessSignal::observe(producer, assurance, observation(40)),
        Vec::new(),
    )
}

fn closed_sources() -> BTreeMap<SourceIdentity, SourceEvidence> {
    BTreeMap::from([
        (source(ASSESSOR), source_evidence(ASSESSOR, 1)),
        (source(LEDGER), source_evidence(LEDGER, 2)),
    ])
}
