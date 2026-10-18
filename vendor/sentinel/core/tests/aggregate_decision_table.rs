//! CAT-5's decision table, and the vacuous / over-strong passes it refuses.
//!
//! Issue #173 / O-07, AMEND 3 and AMEND 4:
//!
//! - a cold `Fails` (count at or over threshold) is an Authoritative
//!   `Inconclusive`, never a `Violate`;
//! - an incomplete or unholdable lookback never reaches `Satisfy`;
//! - a complete under-threshold lookback — including an empty one — reaches
//!   `Satisfy` only once Evidence Coverage has closed;
//! - the obligation keys on the trigger selector only, so an evidence set full
//!   of members and no trigger establishes nothing.
//!
//! Observation-constructed table rows live here. Evaluator traversal lives in
//! `aggregate_evaluator.rs`.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance, ProducerContract,
    SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::aggregate::{
    aggregate, count_within_cold_guard_id, decide, AggregateObservation, AggregateOutcome,
    AggregateSpecification, CountEvidence, CoverageState, GateState, SettledCount,
    COUNT_WITHIN_GUARD_NAME,
};
use sentinel_core::evaluator::Evaluator;
use sentinel_core::facts::{ColdGuardId, GuardConclusion, RefusalReason};
use sentinel_core::specification::EventSelector;
use sentinel_core::CoreError;
use sentinel_model::{
    AttributeKey, AttributeValue, EventKind, MonitoredApplicationId, SchemaVersion,
    SourceCapabilities, SourceCapability, SourceId, SourceIdentity, SourceTier,
};

const BINDING: &str = "example.principal.id";
const TRIGGER_KIND: &str = "example.assessment.started";
const MEMBER_KIND: &str = "example.transfer.attempted";
const LOOKBACK: u64 = 16;
const THRESHOLD: u64 = 3;

// ---------------------------------------------------------------------------
// The table itself
// ---------------------------------------------------------------------------

fn every_observation() -> Vec<AggregateObservation> {
    let mut observations = Vec::new();
    for gate in [GateState::Open, GateState::Closed] {
        for count in [
            CountEvidence::Unasked,
            CountEvidence::Outstanding,
            CountEvidence::UnderThreshold,
            CountEvidence::AtOrOverThreshold,
            CountEvidence::Unholdable,
        ] {
            for coverage in [CoverageState::Settled, CoverageState::Open] {
                observations.push(AggregateObservation::new(gate, count, coverage));
            }
        }
    }
    assert_eq!(observations.len(), 20, "two by five by two assignments");
    observations
}

fn clean_holds() -> AggregateObservation {
    AggregateObservation::new(
        GateState::Open,
        CountEvidence::UnderThreshold,
        CoverageState::Settled,
    )
}

/// AMEND 3: a settled cold `Fails` concludes over-threshold, never a pass.
#[test]
fn cold_fails_is_a_finding_that_never_reaches_satisfy() {
    let over = AggregateObservation::new(
        GateState::Open,
        CountEvidence::AtOrOverThreshold,
        CoverageState::Settled,
    );
    assert_eq!(decide(over), AggregateOutcome::OverThreshold);
}

/// AMEND 4: a permanently unholdable lookback is epistemic, never a pass.
#[test]
fn an_unholdable_lookback_never_satisfies() {
    for coverage in [CoverageState::Settled, CoverageState::Open] {
        let unholdable =
            AggregateObservation::new(GateState::Open, CountEvidence::Unholdable, coverage);
        assert_eq!(
            decide(unholdable),
            AggregateOutcome::InconclusiveEpistemic,
            "a Refused lookback must never discharge"
        );
    }
}

/// A complete under-threshold lookback — empty included — passes once coverage closes.
#[test]
fn a_complete_under_threshold_lookback_satisfies_when_coverage_closed() {
    assert_eq!(decide(clean_holds()), AggregateOutcome::Satisfy);
}

/// The same lookback with Evidence Coverage still open stays prefix-open.
#[test]
fn under_threshold_with_open_coverage_is_prefix_not_satisfy() {
    let open_coverage = AggregateObservation::new(
        GateState::Open,
        CountEvidence::UnderThreshold,
        CoverageState::Open,
    );
    assert_eq!(decide(open_coverage), AggregateOutcome::InconclusivePrefix);
}

/// An unasked count is epistemic; an outstanding one is prefix-open.
#[test]
fn an_absent_count_is_never_a_pass() {
    assert_eq!(
        decide(AggregateObservation::new(
            GateState::Open,
            CountEvidence::Unasked,
            CoverageState::Settled,
        )),
        AggregateOutcome::InconclusiveEpistemic
    );
    assert_eq!(
        decide(AggregateObservation::new(
            GateState::Open,
            CountEvidence::Outstanding,
            CoverageState::Settled,
        )),
        AggregateOutcome::InconclusivePrefix
    );
}

/// A closed evidence gate demotes even a settled finding.
#[test]
fn a_closed_gate_wins_outright() {
    for count in [
        CountEvidence::Unasked,
        CountEvidence::Outstanding,
        CountEvidence::UnderThreshold,
        CountEvidence::AtOrOverThreshold,
        CountEvidence::Unholdable,
    ] {
        for coverage in [CoverageState::Settled, CoverageState::Open] {
            assert_eq!(
                decide(AggregateObservation::new(
                    GateState::Closed,
                    count,
                    coverage
                )),
                AggregateOutcome::InconclusivePrefix,
                "{count:?} / {coverage:?}"
            );
        }
    }
}

/// The pass row is exactly one of the twenty assignments.
#[test]
fn the_satisfy_row_is_exactly_open_gate_under_threshold_settled_coverage() {
    let satisfy: Vec<_> = every_observation()
        .into_iter()
        .filter(|observation| decide(*observation) == AggregateOutcome::Satisfy)
        .collect();
    assert_eq!(satisfy, vec![clean_holds()]);
}

/// No absent, unholdable or over-threshold count reaches the pass row.
#[test]
fn no_finding_or_absent_count_reaches_satisfy() {
    for observation in every_observation() {
        if matches!(
            observation.count,
            CountEvidence::AtOrOverThreshold
                | CountEvidence::Unholdable
                | CountEvidence::Unasked
                | CountEvidence::Outstanding
        ) {
            assert_ne!(
                decide(observation),
                AggregateOutcome::Satisfy,
                "{observation:?}"
            );
        }
    }
}

/// A settled cold answer cannot be widened into a pass.
#[test]
fn settled_count_never_widens_to_satisfy() {
    for settled in [
        SettledCount::OverThreshold,
        SettledCount::InconclusiveEpistemic,
    ] {
        assert_ne!(settled.widen(), AggregateOutcome::Satisfy);
        assert_ne!(AggregateOutcome::from(settled), AggregateOutcome::Satisfy);
    }
}

/// A Binding that raised no obligation has no conclusion, vacuous or otherwise.
#[test]
fn a_binding_that_raised_no_obligation_has_no_conclusion() {
    assert_eq!(aggregate(std::iter::empty::<AggregateOutcome>()), None);
    assert_eq!(
        aggregate([AggregateOutcome::Satisfy]),
        Some(AggregateOutcome::Satisfy)
    );
}

/// One trigger's finding is not weakened by another trigger's pass.
#[test]
fn over_threshold_dominates_every_other_outcome() {
    assert_eq!(
        aggregate([
            AggregateOutcome::Satisfy,
            AggregateOutcome::InconclusivePrefix,
            AggregateOutcome::InconclusiveEpistemic,
            AggregateOutcome::OverThreshold,
        ]),
        Some(AggregateOutcome::OverThreshold)
    );
}

// ---------------------------------------------------------------------------
// The reduction from cold answers to the table's one fact
// ---------------------------------------------------------------------------

/// `CountEvidence::of` reduces guard declarations and settled answers to one fact.
#[test]
fn count_evidence_reduces_declarations_and_answers() {
    let guard = ColdGuardId::new(COUNT_WITHIN_GUARD_NAME, "cw:example|t=3|lb=16");
    let declared = BTreeSet::from([guard.clone()]);

    assert_eq!(
        CountEvidence::of(&BTreeSet::new(), false, false, &BTreeMap::new()),
        CountEvidence::Unasked,
        "no guard bound is an unasked count, not a held one"
    );
    assert_eq!(
        CountEvidence::of(&declared, false, false, &BTreeMap::new()),
        CountEvidence::Unasked,
        "a declared guard nobody asked is still an unasked count"
    );
    assert_eq!(
        CountEvidence::of(&declared, true, false, &BTreeMap::new()),
        CountEvidence::Outstanding
    );
    assert_eq!(
        CountEvidence::of(&declared, true, true, &BTreeMap::new()),
        CountEvidence::UnderThreshold,
        "a fully settled query set with no refusal or failure is a clean Holds"
    );

    let fails = BTreeMap::from([(
        guard.clone(),
        GuardConclusion::Fails {
            witnesses: BTreeSet::new(),
        },
    )]);
    assert_eq!(
        CountEvidence::of(&declared, true, true, &fails),
        CountEvidence::AtOrOverThreshold
    );

    let refused = BTreeMap::from([(
        guard,
        GuardConclusion::Refused {
            reason: RefusalReason::EvidenceNeverRetained,
        },
    )]);
    assert_eq!(
        CountEvidence::of(&declared, true, true, &refused),
        CountEvidence::Unholdable
    );
}

/// Only an outstanding count can still change; the other four are terminal.
#[test]
fn only_an_outstanding_count_could_still_settle() {
    assert!(CountEvidence::Outstanding.could_still_settle());
    for terminal in [
        CountEvidence::Unasked,
        CountEvidence::UnderThreshold,
        CountEvidence::AtOrOverThreshold,
        CountEvidence::Unholdable,
    ] {
        assert!(!terminal.could_still_settle(), "{terminal:?}");
    }
}

// ---------------------------------------------------------------------------
// Catalogued lowering
// ---------------------------------------------------------------------------

/// The derived guard id fingerprints member, threshold and lookback.
///
/// Pinned against `sentinel-coldpath`'s own derivation by
/// `core_derives_the_same_cold_guard_id`, which lives in `coldpath/src/operator.rs`
/// because `coldpath` depends on `core` and so is the only crate that can see
/// both sides. This row pins the format itself, so a change here is visible
/// without reading the other crate.
#[test]
fn the_derived_guard_id_fingerprints_member_threshold_and_lookback() {
    let member = EventSelector::new(
        EventKind::new(MEMBER_KIND).expect("constant"),
        BTreeMap::from([(
            AttributeKey::new("example.transfer.currency").expect("constant"),
            AttributeValue::String("USD".to_owned()),
        )]),
    );
    let id = count_within_cold_guard_id(&member, THRESHOLD, LOOKBACK);
    assert_eq!(id.id(), COUNT_WITHIN_GUARD_NAME);
    assert_eq!(
        id.version(),
        format!("cw:{MEMBER_KIND},example.transfer.currency=String(\"USD\")|t=3|lb=16")
    );

    let wider = count_within_cold_guard_id(&member, THRESHOLD + 1, LOOKBACK);
    assert_ne!(id, wider, "the threshold is part of the question asked");
    let longer = count_within_cold_guard_id(&member, THRESHOLD, LOOKBACK + 1);
    assert_ne!(id, longer, "the lookback is part of the question asked");
}

/// Catalogued CAT-5 lowers with the derived guard already bound.
#[test]
fn catalogued_cat5_lowers_with_a_bound_cold_guard() {
    let specification = catalogued_aggregate();
    let expected = count_within_cold_guard_id(specification.member(), THRESHOLD, LOOKBACK);
    assert_eq!(
        specification.cold_guards(),
        &BTreeSet::from([expected]),
        "a catalogued instance must ask the cold path, not assume a clean count"
    );
    assert_eq!(specification.threshold(), THRESHOLD);
    assert_eq!(specification.lookback_sequences(), LOOKBACK);
    assert_eq!(specification.trigger().kind().as_str(), TRIGGER_KIND);
    assert_eq!(specification.member().kind().as_str(), MEMBER_KIND);
    assert!(
        specification.activation_barrier().is_none(),
        "AggregateIr carries no barrier field"
    );
}

/// A guard-free Specification is an unasked count, not a held one.
#[test]
fn a_specification_built_without_a_guard_declares_none() {
    let specification = AggregateSpecification::new(
        catalogued_aggregate().identity().clone(),
        AttributeKey::new(BINDING).expect("constant"),
        EventSelector::new(
            EventKind::new(TRIGGER_KIND).expect("constant"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new(MEMBER_KIND).expect("constant"),
            BTreeMap::new(),
        ),
        LOOKBACK,
        THRESHOLD,
        BTreeSet::new(),
        SchemaVersion::new("example-contract-v1").expect("constant"),
    );
    assert!(specification.cold_guards().is_empty());
    assert_eq!(
        CountEvidence::of(specification.cold_guards(), false, true, &BTreeMap::new()),
        CountEvidence::Unasked
    );
}

/// The batch driver refuses CAT-5 outright: it has no fact supplier.
#[test]
fn the_batch_driver_refuses_a_catalogued_aggregate() {
    let error = sentinel_core::evaluate(
        Evaluator::aggregate(catalogued_aggregate()),
        &[],
        &BTreeMap::new(),
    )
    .expect_err("a declared cold guard must not be silently dropped");
    let CoreError::FactsRequired { guards } = error else {
        panic!("expected FactsRequired, got {error:?}");
    };
    assert_eq!(guards.len(), 1);
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

fn instance() -> PatternInstance {
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
            EvidenceBinding::new(source("example-assessor"), SourceTier::Anchor, capable()),
        ),
        (
            "member_source".to_owned(),
            EvidenceBinding::new(source("example-ledger"), SourceTier::Anchor, capable()),
        ),
    ]);
    instantiate(
        PatternId::AggregationCountWithin,
        FilterIdentity {
            specification_id: "example.transfer-rate-cap".to_owned(),
            specification_version: "v1".to_owned(),
        },
        ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant")),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds")
}

fn catalogued_aggregate() -> AggregateSpecification {
    AggregateSpecification::try_from(
        instance()
            .ir()
            .as_aggregate()
            .expect("CAT-5 lowers to the aggregate monitor"),
    )
    .expect("catalogued CAT-5 IR converts")
}
