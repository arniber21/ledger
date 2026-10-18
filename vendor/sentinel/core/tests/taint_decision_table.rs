//! **CAT-4's decision table, and the silent pass it was rewritten to refuse.**
//!
//! Issue #141's headline defect, found in the adversarial review of #14's plan:
//! folding the anchor-tier condition into a presence-shaped violation row lets a
//! genuine violation witnessed *only* by contributing-tier sources fall through
//! to `Satisfy`. A Specification that reports success because its evidence was
//! too weak to convict is the exact failure this system exists to prevent.
//!
//! The tests are in three layers, and only the third is a general claim:
//!
//! 1. **The named case.** One observation — a falsifying Event in the action's
//!    causal past, no anchor-tier witness, Evidence Coverage closed — with the
//!    two controls that make it non-trivial: the same observation with an anchor
//!    witness settles a `Violate`, and the clean observation still settles a
//!    `Satisfy`. Without those controls "it is not a Satisfy" would be equally
//!    consistent with a table that returns `Inconclusive` for everything.
//! 2. **The whole `Satisfy` row, enumerated.** Which of the thirty-two
//!    observations settle a `Satisfy` is asserted exactly, not bounded, so a
//!    change that widens the row fails here even where it does not touch the
//!    tier question.
//! 3. **The invariant.** No observation carrying a falsifying ancestor reaches
//!    `Satisfy`, over the whole input space rather than over an example.
//!
//! # Reversion evidence (ADR-0031 §3)
//!
//! ADR-0031 is explicit that "reachability is established by execution, never by
//! reading" and that a claim about a defect must be demonstrated under the
//! specific change it names. So the defective table — the one that folds the
//! anchor condition into the violation row's guard and lets the fall-through
//! continue down the table — was written into `core/src/taint.rs` and this suite
//! run against it. The output is recorded in the pull request rather than
//! paraphrased here; the short form is that
//! `a_contributing_tier_only_violation_never_reaches_satisfy`,
//! `the_satisfy_row_is_exactly_the_clean_settled_observations` and
//! `no_observation_carrying_a_falsifying_ancestor_reaches_satisfy` all failed,
//! each naming `Satisfy` where the current table gives `InconclusivePrefix`.
//!
//! The mutation is safe to trust here for the reason ADR-0031 attaches to the
//! technique: it changes *meaning* rather than control flow, so a failure cannot
//! be an artifact of the mutation. It is the defect, written out.
//!
//! # What this file tests
//!
//! The decision table over constructed observations, plus catalogue lowering.
//! Traversal through Evaluator (CausalIndex + Slice + EvidenceGate) lives in
//! `taint_evaluator.rs`.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::collections::BTreeMap;

use sentinel_compile::catalogue::{
    catalogue, instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance,
    ProducerContract, SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::evaluator::Evaluator;
use sentinel_core::specification::{
    AttributeKey, PrecedenceSpecification, SpecificationConversionError,
};
use sentinel_core::taint::{
    aggregate, decide, AnchorWitness, CausalPast, ConcurrentFalsifier, ConvictedOutcome,
    CoverageState, GateState, TaintObservation, TaintOutcome, TaintSpecification,
};
use sentinel_model::{
    AttributeValue, EventKind, MonitoredApplicationId, SchemaVersion, SourceCapabilities,
    SourceCapability, SourceId, SourceIdentity, SourceTier,
};

// ---------------------------------------------------------------------------
// The decision table
// ---------------------------------------------------------------------------

/// Every assignment of the five facts the table reads.
///
/// Enumerated rather than sampled, and deliberately including combinations a
/// real traversal would never produce — a falsifier both in the causal past and
/// Concurrent, say. A table is a total function and the invariant claimed for it
/// is a claim about all of its inputs; restricting the enumeration to the
/// "realistic" ones would be assuming the classification that the table exists
/// to be independent of.
///
/// The nesting is over the five two-variant types rather than over bit patterns,
/// so a sixth fact added to `TaintObservation::new` is a compile error here
/// rather than a loop that quietly keeps covering half the space.
fn every_observation() -> Vec<TaintObservation> {
    let mut observations = Vec::with_capacity(32);
    for gate in [GateState::Open, GateState::Closed] {
        for causal_past in [CausalPast::CarriesFalsifier, CausalPast::Clean] {
            for anchor_witness in [AnchorWitness::Present, AnchorWitness::Absent] {
                for concurrent in [ConcurrentFalsifier::Present, ConcurrentFalsifier::Absent] {
                    for coverage in [CoverageState::Settled, CoverageState::Open] {
                        observations.push(TaintObservation::new(
                            gate,
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
    assert_eq!(
        observations.len(),
        32,
        "five two-valued facts have 32 assignments"
    );
    observations
}

/// The observation a clean, settled, gate-open evaluation produces.
fn clean() -> TaintObservation {
    TaintObservation::new(
        GateState::Open,
        CausalPast::Clean,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    )
}

/// **A violation witnessed only at contributing tier never reaches `Satisfy`.**
///
/// Issue #141's acceptance criterion, as one case with its controls.
///
/// The evidence contains the violating Event. It lies in the action's causal
/// past, so no admissible linearization moves it out; Evidence Coverage is
/// closed, so nothing further is outstanding. The only thing missing is an
/// anchor-tier witness — and ADR-0022 says a contributing-tier source "cannot,
/// alone, anchor a Verdict for a high-assurance property", which is a statement
/// about how much the conclusion can claim and not a licence to discard the
/// observation.
///
/// The defective table discarded it: with the tier condition folded into the
/// violation row's guard, this case fails that row, fails the Concurrent row
/// because the falsifier is `Before` rather than unordered, and lands in the
/// pass row.
#[test]
fn a_contributing_tier_only_violation_never_reaches_satisfy() {
    let contributing_only = TaintObservation::new(
        GateState::Open,
        CausalPast::CarriesFalsifier,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    );
    assert_eq!(
        decide(contributing_only),
        TaintOutcome::InconclusivePrefix,
        "a falsifying Event in the action's causal past, witnessed only at contributing tier, \
         must downgrade the conclusion and never license a pass"
    );
    assert_ne!(decide(contributing_only), TaintOutcome::Satisfy);

    // Control 1: the same observation with an anchor witness settles.
    let anchored = TaintObservation::new(
        GateState::Open,
        CausalPast::CarriesFalsifier,
        AnchorWitness::Present,
        ConcurrentFalsifier::Absent,
        CoverageState::Settled,
    );
    assert_eq!(
        decide(anchored),
        TaintOutcome::Violate,
        "the tier is the only difference from the case above, so this is what it costs"
    );

    // Control 2: the table can still reach Satisfy at all. Without this the
    // assertion above would hold for a table that answers Inconclusive to
    // everything, which would pass the criterion and be useless.
    assert_eq!(decide(clean()), TaintOutcome::Satisfy);
}

/// **The `Satisfy` row is exactly the clean, settled observations.**
///
/// Enumerated rather than bounded. `no_observation_carrying_a_falsifying_ancestor_reaches_satisfy`
/// proves one direction; this proves the row has no other members either, so a
/// change that licensed a pass on an open gate or an unsettled Watermark fails
/// here even though it never touches the tier question.
#[test]
fn the_satisfy_row_is_exactly_the_clean_settled_observations() {
    for observation in every_observation() {
        let expected_satisfy = observation.gate == GateState::Open
            && observation.causal_past == CausalPast::Clean
            && observation.concurrent == ConcurrentFalsifier::Absent
            && observation.coverage == CoverageState::Settled;
        assert_eq!(
            decide(observation) == TaintOutcome::Satisfy,
            expected_satisfy,
            "{observation:?} disagrees with ADR-0025's licence for Satisfy"
        );
    }
}

/// **No observation carrying a falsifying ancestor reaches `Satisfy`.**
///
/// The invariant, over the whole input space rather than over an example. The
/// source-tier facts are quantified over here, which is the substance: whatever
/// a deployment's tiers turn out to be, they cannot move a present violation
/// into the pass row.
#[test]
fn no_observation_carrying_a_falsifying_ancestor_reaches_satisfy() {
    let mut checked = 0_usize;
    for observation in every_observation() {
        if observation.causal_past != CausalPast::CarriesFalsifier {
            continue;
        }
        assert_ne!(
            decide(observation),
            TaintOutcome::Satisfy,
            "{observation:?} reports success on evidence that contains the violating Event"
        );
        checked += 1;
    }
    assert_eq!(
        checked, 16,
        "half of the input space carries a falsifying ancestor; a smaller number means the \
         enumeration stopped covering it"
    );
}

/// A Concurrent falsifier is observational uncertainty, and an anchor witness
/// does not settle it.
///
/// The row that separates this Pattern from a timestamp comparison. Some
/// admissible linearization places the falsifier before the action and another
/// places it after, so no linearization-invariant conclusion exists — ADR-0018's
/// constraint, and `crate::precedence`'s row 2 in the opposite polarity.
#[test]
fn a_concurrent_falsifier_is_observationally_uncertain_whatever_its_tier() {
    for anchor_witness in [AnchorWitness::Present, AnchorWitness::Absent] {
        assert_eq!(
            decide(TaintObservation::new(
                GateState::Open,
                CausalPast::Clean,
                anchor_witness,
                ConcurrentFalsifier::Present,
                CoverageState::Settled,
            )),
            TaintOutcome::InconclusiveObservational,
            "anchor tier does not order two Concurrent Events"
        );
    }
}

/// A closed evidence gate wins outright, including over a settled violation.
///
/// `crate::precedence` gives the reason: the gate runs before obligation
/// analysis and must be able to demote a Violate, so it cannot be a dominated
/// row.
#[test]
fn a_closed_evidence_gate_demotes_every_conclusion() {
    for observation in every_observation() {
        if observation.gate == GateState::Open {
            continue;
        }
        assert_eq!(
            decide(observation),
            TaintOutcome::InconclusivePrefix,
            "{observation:?} settled something behind a closed gate"
        );
    }
}

/// Evidence Coverage that has not closed is not a pass, even on clean evidence.
///
/// ADR-0025's first clause: absence settles only after every source-completeness
/// obligation is closed. The pair is the argument — the two observations differ
/// in `coverage_settled` alone.
#[test]
fn clean_evidence_does_not_pass_until_coverage_closes() {
    let unsettled = TaintObservation::new(
        GateState::Open,
        CausalPast::Clean,
        AnchorWitness::Absent,
        ConcurrentFalsifier::Absent,
        CoverageState::Open,
    );
    assert_eq!(decide(unsettled), TaintOutcome::InconclusivePrefix);
    assert_eq!(decide(clean()), TaintOutcome::Satisfy);
}

/// Widening a settled witness preserves dominance and cannot produce `Satisfy`.
///
/// `ConvictedOutcome` is the invariant expressed as a type, and `widen` is the
/// one place it meets the full lattice. What is asserted is the mapping and its
/// order, not the absence of a variant — the absence is the compiler's job, and
/// a test claiming credit for it would be a tautology.
#[test]
fn widening_a_settled_witness_preserves_dominance() {
    assert_eq!(ConvictedOutcome::Violate.widen(), TaintOutcome::Violate);
    assert_eq!(
        ConvictedOutcome::InconclusivePrefix.widen(),
        TaintOutcome::InconclusivePrefix
    );
    assert!(ConvictedOutcome::InconclusivePrefix < ConvictedOutcome::Violate);
    assert!(ConvictedOutcome::InconclusivePrefix.widen() < ConvictedOutcome::Violate.widen());
    assert_eq!(
        TaintOutcome::from(ConvictedOutcome::Violate),
        ConvictedOutcome::Violate.widen()
    );
}

// ---------------------------------------------------------------------------
// Aggregation over one Binding
// ---------------------------------------------------------------------------

/// **A Binding that raised no obligation has no conclusion, not a pass.**
///
/// The third place a vacuous pass could enter, and the one `Iterator::all` gets
/// wrong: `all` over an empty set is `true`, and `true` renders as `Satisfy`.
/// `crate::precedence::assess` makes the identical call by skipping a Binding
/// with an empty trigger set, and `shared/oracle.ts` makes it too.
#[test]
fn a_binding_that_raised_no_obligation_has_no_conclusion() {
    assert_eq!(aggregate(Vec::new()), None);
    assert_eq!(
        aggregate([TaintOutcome::Satisfy]),
        Some(TaintOutcome::Satisfy),
        "the control: one discharged obligation does conclude"
    );
}

/// The Binding's conclusion is the dominance maximum over its actions.
///
/// The obligation is universally quantified over the Binding's actions, so one
/// settled violation falsifies the conjunction in every linearization and one
/// action's uncertainty makes the conjunction uncertain.
#[test]
fn a_binding_conclusion_is_the_dominance_maximum() {
    assert_eq!(
        aggregate([TaintOutcome::Satisfy, TaintOutcome::Violate]),
        Some(TaintOutcome::Violate)
    );
    assert_eq!(
        aggregate([
            TaintOutcome::Satisfy,
            TaintOutcome::InconclusivePrefix,
            TaintOutcome::InconclusiveObservational,
        ]),
        Some(TaintOutcome::InconclusiveObservational)
    );
    assert_eq!(
        aggregate([TaintOutcome::Satisfy, TaintOutcome::Satisfy]),
        Some(TaintOutcome::Satisfy)
    );
    assert_eq!(
        aggregate([
            TaintOutcome::InconclusiveObservational,
            TaintOutcome::Violate,
        ]),
        Some(TaintOutcome::Violate),
        "a settled violation dominates uncertainty, never the other way round"
    );
}

// ---------------------------------------------------------------------------
// Lowering
// ---------------------------------------------------------------------------

fn identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.action-untainted".to_owned(),
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

/// A conforming instantiation of one catalogued Pattern.
///
/// The wildcard arm panics: `PatternId` is `#[non_exhaustive]`, so an
/// integration test in another crate cannot write an exhaustive `match` over it,
/// and a loud panic naming the Pattern is the closest available equivalent.
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
///
/// Evaluable, not merely nameable: each arm constructs the family Evaluator
/// and runs `evaluate` over empty evidence. Empty evidence yields no Binding,
/// which is the honest residue of a Pattern that raised no obligation — and
/// a no-op `build_verdicts` that could not produce a Verdict would still
/// return `Ok(vec![])` here, so the contributing-tier pin in
/// `taint_evaluator.rs` is what proves the table is wired. CAT-5 declares cold
/// guards, so batch `evaluate` returns `FactsRequired` rather than Verdicts.
/// The wildcard stays: a fifth `PatternId` is a runtime panic, not
/// `From<&MonitorIr>`.
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

/// **A taint prohibition is not readable as a precedence obligation.**
///
/// The two carry the same four identity fields and the same required-source map,
/// and they mean opposite things. `MonitorIr::as_precedence` returning `None`
/// for a taint monitor is what stops a caller lowering CAT-4 into a
/// `PrecedenceSpecification` and evaluating "the action must be preceded by
/// untrusted content" — a Verdict inversion that would settle `Violate` on every
/// clean deployment and `Satisfy` on every tainted one.
#[test]
fn a_taint_monitor_is_not_readable_as_a_precedence_obligation() {
    let taint = conforming(PatternId::ProvenanceTaint);
    assert!(taint.ir().as_precedence().is_none());
    assert!(taint.ir().as_taint().is_some());

    let precedence = conforming(PatternId::PrecedenceApproval);
    assert!(precedence.ir().as_taint().is_none());
    assert!(precedence.ir().as_precedence().is_some());
}

/// CAT-4 lowers into the Core's own vocabulary carrying both selectors intact.
///
/// Structural rather than incidental: the action and the untrusted-content
/// selectors must arrive in the fields that mean what they matched, because
/// swapping them turns the Specification into its own negation.
#[test]
fn cat4_lowers_into_the_cores_own_specification() {
    let built = conforming(PatternId::ProvenanceTaint);
    let specification = TaintSpecification::try_from(
        built
            .ir()
            .as_taint()
            .expect("CAT-4 lowers to the taint monitor"),
    )
    .expect("catalogued CAT-4 IR converts");

    assert_eq!(
        specification.binding_attribute().as_str(),
        "example.request.id"
    );
    assert_eq!(specification.action().kind().as_str(), "example.email.sent");
    assert!(specification.action().required_attributes().is_empty());
    assert_eq!(
        specification.untrusted_content().kind().as_str(),
        "example.content.ingested"
    );
    assert_eq!(
        specification
            .untrusted_content()
            .required_attributes()
            .get(&AttributeKey::new("example.content.trust").expect("fixture")),
        Some(&AttributeValue::String("untrusted".to_owned()))
    );
    assert_eq!(
        specification.required_sources().len(),
        2,
        "both evidence roles bind distinct sources, and the Core needs both"
    );
    assert_eq!(
        specification.schema_version().as_str(),
        "example-contract-v1"
    );
}

/// #168: a poisoned public IR field must not become a Specification.
///
/// These values previously `From`'d and `evaluate` returned `Ok(vec![])` — a
/// silent pass. Conversion now refuses; nothing below calls `evaluate`.
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
fn a_blank_specification_id_on_precedence_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::PrecedenceApproval)
        .ir()
        .as_precedence()
        .expect("CAT-1 lowers to the precedence monitor")
        .clone();
    ir.specification_id = "   ".to_owned();
    assert_eq!(
        PrecedenceSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankSpecificationId)
    );
}

#[test]
fn a_blank_specification_version_on_precedence_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::PrecedenceApproval)
        .ir()
        .as_precedence()
        .expect("CAT-1 lowers to the precedence monitor")
        .clone();
    ir.specification_version = "   ".to_owned();
    assert_eq!(
        PrecedenceSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankSpecificationVersion)
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

#[test]
fn a_blank_specification_id_on_taint_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::ProvenanceTaint)
        .ir()
        .as_taint()
        .expect("CAT-4 lowers to the taint monitor")
        .clone();
    ir.specification_id = "   ".to_owned();
    assert_eq!(
        TaintSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankSpecificationId)
    );
}

#[test]
fn a_blank_specification_version_on_taint_ir_is_conversion_refusal() {
    let mut ir = conforming(PatternId::ProvenanceTaint)
        .ir()
        .as_taint()
        .expect("CAT-4 lowers to the taint monitor")
        .clone();
    ir.specification_version = "   ".to_owned();
    assert_eq!(
        TaintSpecification::try_from(&ir),
        Err(SpecificationConversionError::BlankSpecificationVersion)
    );
}

#[test]
fn a_blank_binding_attribute_on_precedence_ir_is_evaluator_conversion_refusal() {
    let mut ir = conforming(PatternId::PrecedenceApproval)
        .ir()
        .as_precedence()
        .expect("CAT-1 lowers to the precedence monitor")
        .clone();
    ir.binding_attribute = "   ".to_owned();
    assert!(matches!(
        Evaluator::try_from(&ir),
        Err(SpecificationConversionError::BlankBindingAttribute)
    ));
}

#[test]
fn a_blank_binding_attribute_on_taint_ir_is_evaluator_conversion_refusal() {
    let mut ir = conforming(PatternId::ProvenanceTaint)
        .ir()
        .as_taint()
        .expect("CAT-4 lowers to the taint monitor")
        .clone();
    ir.binding_attribute = "   ".to_owned();
    assert!(matches!(
        Evaluator::try_from(&ir),
        Err(SpecificationConversionError::BlankBindingAttribute)
    ));
}

/// Pins that [`TaintSpecification::new`] takes [`AttributeKey`], not a raw
/// String — the issue's constructor hole, executed.
#[test]
fn taint_specification_new_takes_an_attribute_key() {
    let converted = TaintSpecification::try_from(
        conforming(PatternId::ProvenanceTaint)
            .ir()
            .as_taint()
            .expect("CAT-4 lowers to the taint monitor"),
    )
    .expect("catalogued CAT-4 IR converts");
    let binding = AttributeKey::new("example.request.id").expect("fixture");
    let constructed = TaintSpecification::new(
        converted.identity().clone(),
        binding.clone(),
        converted.action().clone(),
        converted.untrusted_content().clone(),
        converted.required_sources().clone(),
        converted.schema_version().clone(),
    );
    assert_eq!(constructed.binding_attribute(), &binding);
    assert_eq!(constructed, converted);
}
