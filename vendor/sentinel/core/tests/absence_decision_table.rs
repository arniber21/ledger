//! **CAT-2's decision table, and the silent pass it inherits CAT-4's refusal of.**
//!
//! Issue #141's acceptance criterion for CAT-2: a contributing-tier-only
//! violation inside `(after, until]` never reaches `Satisfy`. The table also
//! refuses a vacuous pass when the interval never opened — that path is
//! NoObligation (`decide` returns `None`), never `Satisfy`.
//!
//! Observation-constructed table tests live here. Traversal through Evaluator
//! lives in `absence_evaluator.rs`.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use sentinel_compile::catalogue::{
    catalogue, instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance,
    ProducerContract, SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::absence::{
    aggregate, decide, AbsenceObservation, AbsenceOutcome, AbsenceSpecification, AnchorWitness,
    CausalPast, ConcurrentFalsifier, ConvictedOutcome, CoverageState, GateState, IntervalPremise,
};
use sentinel_core::evaluator::Evaluator;
use sentinel_core::specification::{
    AttributeKey, PrecedenceSpecification, SpecificationConversionError,
};
use sentinel_core::taint::TaintSpecification;
use sentinel_model::{
    AttributeValue, EventKind, MonitoredApplicationId, SchemaVersion, SourceCapabilities,
    SourceCapability, SourceId, SourceIdentity, SourceTier,
};

fn every_observation() -> Vec<AbsenceObservation> {
    let mut observations = Vec::with_capacity(64);
    for gate in [GateState::Open, GateState::Closed] {
        for interval in [IntervalPremise::Opened, IntervalPremise::NeverOpened] {
            for causal_past in [CausalPast::CarriesFalsifier, CausalPast::Clean] {
                for anchor_witness in [AnchorWitness::Present, AnchorWitness::Absent] {
                    for concurrent in [ConcurrentFalsifier::Present, ConcurrentFalsifier::Absent] {
                        for coverage in [CoverageState::Settled, CoverageState::Open] {
                            observations.push(AbsenceObservation::new(
                                gate,
                                interval,
                                causal_past,
                                anchor_witness,
                                concurrent,
                                coverage,
                            ));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        observations.len(),
        64,
        "six two-valued facts have 64 assignments"
    );
    observations
}

fn clean_opened() -> AbsenceObservation {
    AbsenceObservation::new(
        GateState::Open,
        IntervalPremise::Opened,
        CausalPast::Clean,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    )
}

/// **A violation witnessed only at contributing tier never reaches `Satisfy`.**
#[test]
fn a_contributing_tier_only_violation_never_reaches_satisfy() {
    let contributing_only = AbsenceObservation::new(
        GateState::Open,
        IntervalPremise::Opened,
        CausalPast::CarriesFalsifier,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    );
    assert_eq!(
        decide(contributing_only),
        Some(AbsenceOutcome::InconclusivePrefix),
        "a veto inside (after, until], witnessed only at contributing tier, must \
         downgrade and never license a pass"
    );
    assert_ne!(decide(contributing_only), Some(AbsenceOutcome::Satisfy));

    let anchored = AbsenceObservation::new(
        GateState::Open,
        IntervalPremise::Opened,
        CausalPast::CarriesFalsifier,
        AnchorWitness::Present,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    );
    assert_eq!(decide(anchored), Some(AbsenceOutcome::Violate));

    assert_eq!(decide(clean_opened()), Some(AbsenceOutcome::Satisfy));
}

/// Never-opened + settled is NoObligation, never Satisfy.
#[test]
fn a_never_opened_settled_interval_is_no_obligation_not_satisfy() {
    let never_opened = AbsenceObservation::new(
        GateState::Open,
        IntervalPremise::NeverOpened,
        CausalPast::Clean,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    );
    assert_eq!(
        decide(never_opened),
        None,
        "a Scope that never opened has nothing to discharge"
    );
    assert_ne!(decide(never_opened), Some(AbsenceOutcome::Satisfy));
}

/// Never-opened + coverage still open is prefix Inconclusive — missing evidence
/// is never a pass, and is not silent NoObligation either.
#[test]
fn a_never_opened_open_coverage_is_prefix_inconclusive() {
    let pending = AbsenceObservation::new(
        GateState::Open,
        IntervalPremise::NeverOpened,
        CausalPast::Clean,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Open,
    );
    assert_eq!(decide(pending), Some(AbsenceOutcome::InconclusivePrefix));
}

/// **The `Satisfy` row is exactly the opened, clean, settled observations.**
#[test]
fn the_satisfy_row_is_exactly_the_opened_clean_settled_observations() {
    for observation in every_observation() {
        let expected_satisfy = observation.gate == GateState::Open
            && observation.interval == IntervalPremise::Opened
            && observation.causal_past == CausalPast::Clean
            && observation.concurrent == ConcurrentFalsifier::Absent
            && observation.coverage == CoverageState::Settled;
        assert_eq!(
            decide(observation) == Some(AbsenceOutcome::Satisfy),
            expected_satisfy,
            "{observation:?} disagrees with ADR-0025's licence for Satisfy"
        );
    }
}

/// **No observation carrying a falsifying ancestor reaches `Satisfy`.**
#[test]
fn no_observation_carrying_a_falsifying_ancestor_reaches_satisfy() {
    let mut checked = 0_usize;
    for observation in every_observation() {
        if observation.causal_past != CausalPast::CarriesFalsifier {
            continue;
        }
        assert_ne!(
            decide(observation),
            Some(AbsenceOutcome::Satisfy),
            "{observation:?} reports success on evidence that contains the veto"
        );
        checked += 1;
    }
    assert_eq!(
        checked, 32,
        "half of the input space carries a falsifying ancestor"
    );
}

/// No NeverOpened observation settles Satisfy.
#[test]
fn no_never_opened_observation_reaches_satisfy() {
    for observation in every_observation() {
        if observation.interval != IntervalPremise::NeverOpened {
            continue;
        }
        assert_ne!(
            decide(observation),
            Some(AbsenceOutcome::Satisfy),
            "{observation:?} licensed Satisfy on a Scope that never opened"
        );
    }
}

#[test]
fn a_concurrent_falsifier_is_observationally_uncertain_whatever_its_tier() {
    for anchor_witness in [AnchorWitness::Present, AnchorWitness::Absent] {
        assert_eq!(
            decide(AbsenceObservation::new(
                GateState::Open,
                IntervalPremise::Opened,
                CausalPast::Clean,
                anchor_witness,
                ConcurrentFalsifier::Present,
                CoverageState::Settled,
            )),
            Some(AbsenceOutcome::InconclusiveObservational),
        );
    }
}

#[test]
fn a_closed_evidence_gate_demotes_every_conclusion() {
    for observation in every_observation() {
        if observation.gate == GateState::Open {
            continue;
        }
        assert_eq!(
            decide(observation),
            Some(AbsenceOutcome::InconclusivePrefix),
            "{observation:?} settled something behind a closed gate"
        );
    }
}

#[test]
fn clean_opened_evidence_does_not_pass_until_coverage_closes() {
    let unsettled = AbsenceObservation::new(
        GateState::Open,
        IntervalPremise::Opened,
        CausalPast::Clean,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Open,
    );
    assert_eq!(decide(unsettled), Some(AbsenceOutcome::InconclusivePrefix));
    assert_eq!(decide(clean_opened()), Some(AbsenceOutcome::Satisfy));
}

#[test]
fn widening_a_settled_witness_preserves_dominance() {
    assert_eq!(ConvictedOutcome::Violate.widen(), AbsenceOutcome::Violate);
    assert_eq!(
        ConvictedOutcome::InconclusivePrefix.widen(),
        AbsenceOutcome::InconclusivePrefix
    );
    assert!(ConvictedOutcome::InconclusivePrefix < ConvictedOutcome::Violate);
    assert_eq!(
        AbsenceOutcome::from(ConvictedOutcome::Violate),
        ConvictedOutcome::Violate.widen()
    );
}

#[test]
fn a_binding_that_raised_no_obligation_has_no_conclusion() {
    assert_eq!(aggregate(Vec::new()), None);
    assert_eq!(
        aggregate([AbsenceOutcome::Satisfy]),
        Some(AbsenceOutcome::Satisfy),
    );
}

#[test]
fn a_binding_conclusion_is_the_dominance_maximum() {
    assert_eq!(
        aggregate([AbsenceOutcome::Satisfy, AbsenceOutcome::Violate]),
        Some(AbsenceOutcome::Violate)
    );
    assert_eq!(
        aggregate([
            AbsenceOutcome::Satisfy,
            AbsenceOutcome::InconclusivePrefix,
            AbsenceOutcome::InconclusiveObservational,
        ]),
        Some(AbsenceOutcome::InconclusiveObservational)
    );
}

// ---------------------------------------------------------------------------
// Lowering
// ---------------------------------------------------------------------------

fn identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.no-revoke-before-commit".to_owned(),
        specification_version: "v1".to_owned(),
    }
}

fn contract() -> ProducerContract {
    ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant"))
}

fn source(application: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(application).expect("constant"),
        SourceId::new("probe").expect("constant"),
    )
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

fn capable() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

#[allow(clippy::too_many_lines)] // One arm per catalogued Pattern; a third family grows this.
fn conforming(pattern: PatternId) -> PatternInstance {
    let (arguments, evidence) = match pattern {
        PatternId::PrecedenceApproval => (
            BTreeMap::from([
                (
                    "binding_attribute".to_owned(),
                    ParameterValue::AttributeKey(
                        AttributeKey::new("example.request.id").expect("fixture"),
                    ),
                ),
                (
                    "trigger".to_owned(),
                    selector("example.action.executed", &[]),
                ),
                (
                    "requirement".to_owned(),
                    selector("example.approval.granted", &[]),
                ),
            ]),
            BTreeMap::from([
                (
                    "trigger_source".to_owned(),
                    EvidenceBinding::new(source("example-gateway"), SourceTier::Anchor, capable()),
                ),
                (
                    "requirement_source".to_owned(),
                    EvidenceBinding::new(source("example-authz"), SourceTier::Anchor, capable()),
                ),
            ]),
        ),
        PatternId::ProvenanceTaint => (
            BTreeMap::from([
                (
                    "binding_attribute".to_owned(),
                    ParameterValue::AttributeKey(
                        AttributeKey::new("example.request.id").expect("fixture"),
                    ),
                ),
                ("action".to_owned(), selector("example.email.sent", &[])),
                (
                    "untrusted_content".to_owned(),
                    selector(
                        "example.content.ingested",
                        &[("example.content.trust", "untrusted")],
                    ),
                ),
            ]),
            BTreeMap::from([
                (
                    "action_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-mail-gateway"),
                        SourceTier::Anchor,
                        capable(),
                    ),
                ),
                (
                    "content_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-content-broker"),
                        SourceTier::Anchor,
                        capable(),
                    ),
                ),
            ]),
        ),
        PatternId::AbsenceAfterUntil => (
            BTreeMap::from([
                (
                    "binding_attribute".to_owned(),
                    ParameterValue::AttributeKey(
                        AttributeKey::new("example.request.id").expect("fixture"),
                    ),
                ),
                (
                    "after".to_owned(),
                    selector("example.approval.granted", &[]),
                ),
                (
                    "until".to_owned(),
                    selector("example.action.committed", &[]),
                ),
                ("veto".to_owned(), selector("example.approval.revoked", &[])),
            ]),
            BTreeMap::from([
                (
                    "after_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-approval-service"),
                        SourceTier::Contributing,
                        capable(),
                    ),
                ),
                (
                    "until_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-action-gateway"),
                        SourceTier::Anchor,
                        capable(),
                    ),
                ),
                (
                    "veto_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-revocation-bus"),
                        SourceTier::Anchor,
                        capable(),
                    ),
                ),
            ]),
        ),
        PatternId::AggregationCountWithin => (
            BTreeMap::from([
                (
                    "binding_attribute".to_owned(),
                    ParameterValue::AttributeKey(
                        AttributeKey::new("example.request.id").expect("fixture"),
                    ),
                ),
                (
                    "trigger".to_owned(),
                    selector("example.transfer.attempted", &[]),
                ),
                (
                    "member".to_owned(),
                    selector("example.transfer.completed", &[]),
                ),
                (
                    "lookback".to_owned(),
                    ParameterValue::SequenceLookbackBound(16),
                ),
                ("threshold".to_owned(), ParameterValue::CountThreshold(3)),
            ]),
            BTreeMap::from([
                (
                    "trigger_source".to_owned(),
                    EvidenceBinding::new(source("example-gateway"), SourceTier::Anchor, capable()),
                ),
                (
                    "member_source".to_owned(),
                    EvidenceBinding::new(source("example-ledger"), SourceTier::Anchor, capable()),
                ),
            ]),
        ),

        PatternId::ExistenceBeforeScopeClose => (
            BTreeMap::from([
                (
                    "binding_attribute".to_owned(),
                    ParameterValue::AttributeKey(
                        AttributeKey::new("example.request.id").expect("fixture"),
                    ),
                ),
                ("open".to_owned(), selector("example.saga.opened", &[])),
                (
                    "required".to_owned(),
                    selector("example.saga.compensated", &[]),
                ),
                ("close".to_owned(), selector("example.saga.terminated", &[])),
            ]),
            BTreeMap::from([
                (
                    "open_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-saga-orchestrator"),
                        SourceTier::Contributing,
                        capable(),
                    ),
                ),
                (
                    "required_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-money-system"),
                        SourceTier::Anchor,
                        capable(),
                    ),
                ),
                (
                    "close_source".to_owned(),
                    EvidenceBinding::new(
                        source("example-broker-journal"),
                        SourceTier::Contributing,
                        capable(),
                    ),
                ),
            ]),
        ),
        other => panic!("{other} is catalogued with no conforming bindings in this suite"),
    };
    instantiate(pattern, identity(), contract(), arguments, evidence)
        .expect("a fully bound instantiation succeeds")
}

/// **Every catalogued Pattern lowers into an Evaluator the Core can step.**
#[test]
fn catalogue_declares_no_pattern_the_core_cannot_lower() {
    for declared in catalogue() {
        let built = conforming(declared.id);
        match declared.id {
            PatternId::PrecedenceApproval => {
                let ir = built
                    .ir()
                    .as_precedence()
                    .expect("CAT-1 lowers to the precedence monitor");
                let evaluator = Evaluator::try_from(ir).expect("catalogued CAT-1 IR converts");
                let verdicts = sentinel_core::evaluate(evaluator, &[], &BTreeMap::new())
                    .expect("an evaluable CAT-1 Specification seals over empty evidence");
                assert!(verdicts.is_empty());
            }
            PatternId::ProvenanceTaint => {
                let ir = built
                    .ir()
                    .as_taint()
                    .expect("CAT-4 lowers to the taint monitor");
                let evaluator = Evaluator::try_from(ir).expect("catalogued CAT-4 IR converts");
                let verdicts = sentinel_core::evaluate(evaluator, &[], &BTreeMap::new())
                    .expect("an evaluable CAT-4 Specification seals over empty evidence");
                assert!(verdicts.is_empty());
            }
            PatternId::AbsenceAfterUntil => {
                let ir = built
                    .ir()
                    .as_absence()
                    .expect("CAT-2 lowers to the absence monitor");
                let evaluator = Evaluator::try_from(ir).expect("catalogued CAT-2 IR converts");
                let verdicts = sentinel_core::evaluate(evaluator, &[], &BTreeMap::new())
                    .expect("an evaluable CAT-2 Specification seals over empty evidence");
                assert!(verdicts.is_empty());
            }
            PatternId::AggregationCountWithin => {
                let ir = built
                    .ir()
                    .as_aggregate()
                    .expect("CAT-5 lowers to the aggregate monitor");
                let evaluator = Evaluator::try_from(ir).expect("catalogued CAT-5 IR converts");
                match sentinel_core::evaluate(evaluator, &[], &BTreeMap::new()) {
                    Err(sentinel_core::CoreError::FactsRequired { guards }) => {
                        assert!(!guards.is_empty());
                    }
                    other => {
                        panic!("CAT-5 declares cold guards; expected FactsRequired, got {other:?}")
                    }
                }
            }

            PatternId::ExistenceBeforeScopeClose => {
                let ir = built
                    .ir()
                    .as_existence()
                    .expect("CAT-3 lowers to the existence monitor");
                let evaluator = Evaluator::try_from(ir).expect("catalogued CAT-3 IR converts");
                let verdicts = sentinel_core::evaluate(evaluator, &[], &BTreeMap::new())
                    .expect("an evaluable CAT-3 Specification seals over empty evidence");
                assert!(verdicts.is_empty());
            }
            other => panic!("{other} is catalogued with no lowering into the Core"),
        }
    }
}

#[test]
fn an_absence_monitor_is_not_readable_as_precedence_or_taint() {
    let absence = conforming(PatternId::AbsenceAfterUntil);
    assert!(absence.ir().as_precedence().is_none());
    assert!(absence.ir().as_taint().is_none());
    assert!(absence.ir().as_absence().is_some());

    let taint = conforming(PatternId::ProvenanceTaint);
    assert!(taint.ir().as_absence().is_none());
}

#[test]
fn cat2_lowers_into_the_cores_own_specification() {
    let built = conforming(PatternId::AbsenceAfterUntil);
    let specification = AbsenceSpecification::try_from(
        built
            .ir()
            .as_absence()
            .expect("CAT-2 lowers to the absence monitor"),
    )
    .expect("catalogued CAT-2 IR converts");

    assert_eq!(
        specification.binding_attribute().as_str(),
        "example.request.id"
    );
    assert_eq!(
        specification.after().kind().as_str(),
        "example.approval.granted"
    );
    assert_eq!(
        specification.until().kind().as_str(),
        "example.action.committed"
    );
    assert_eq!(
        specification.veto().kind().as_str(),
        "example.approval.revoked"
    );
    assert_eq!(specification.required_sources().len(), 3);
}

#[test]
fn a_blank_binding_attribute_on_absence_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::AbsenceAfterUntil)
        .ir()
        .as_absence()
        .expect("CAT-2 lowers to the absence monitor")
        .clone();
    ir.binding_attribute = "   ".to_owned();
    assert_eq!(
        AbsenceSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankBindingAttribute)
    );
}

#[test]
fn a_blank_specification_id_on_absence_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::AbsenceAfterUntil)
        .ir()
        .as_absence()
        .expect("CAT-2 lowers to the absence monitor")
        .clone();
    ir.specification_id = "   ".to_owned();
    assert_eq!(
        AbsenceSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankSpecificationId)
    );
}

#[test]
fn a_blank_specification_version_on_absence_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::AbsenceAfterUntil)
        .ir()
        .as_absence()
        .expect("CAT-2 lowers to the absence monitor")
        .clone();
    ir.specification_version = "   ".to_owned();
    assert_eq!(
        AbsenceSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankSpecificationVersion)
    );
}

#[test]
fn a_blank_binding_attribute_on_absence_ir_is_evaluator_conversion_refusal() {
    let mut ir = conforming(PatternId::AbsenceAfterUntil)
        .ir()
        .as_absence()
        .expect("CAT-2 lowers to the absence monitor")
        .clone();
    ir.binding_attribute = "   ".to_owned();
    assert!(matches!(
        Evaluator::try_from(&ir),
        Err(SpecificationConversionError::BlankBindingAttribute)
    ));
}

#[test]
fn absence_specification_new_takes_an_attribute_key() {
    let converted = AbsenceSpecification::try_from(
        conforming(PatternId::AbsenceAfterUntil)
            .ir()
            .as_absence()
            .expect("CAT-2 lowers to the absence monitor"),
    )
    .expect("catalogued CAT-2 IR converts");
    let binding = AttributeKey::new("example.request.id").expect("fixture");
    let constructed = AbsenceSpecification::new(
        converted.identity().clone(),
        binding.clone(),
        converted.after().clone(),
        converted.until().clone(),
        converted.veto().clone(),
        converted.required_sources().clone(),
        converted.schema_version().clone(),
    );
    assert_eq!(constructed.binding_attribute(), &binding);
    assert_eq!(constructed, converted);
}

// Keep the CAT-1 / CAT-4 blank-IR pins that shared this suite historically.
#[test]
fn a_blank_binding_attribute_on_precedence_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::PrecedenceApproval)
        .ir()
        .as_precedence()
        .expect("CAT-1 lowers to the precedence monitor")
        .clone();
    ir.binding_attribute = "   ".to_owned();
    assert_eq!(
        PrecedenceSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankBindingAttribute)
    );
}

#[test]
fn a_blank_binding_attribute_on_taint_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::ProvenanceTaint)
        .ir()
        .as_taint()
        .expect("CAT-4 lowers to the taint monitor")
        .clone();
    ir.binding_attribute = "   ".to_owned();
    assert_eq!(
        TaintSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankBindingAttribute)
    );
}
