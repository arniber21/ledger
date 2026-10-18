//! Instantiating a catalogued Pattern: what is rejected, what is warned about,
//! and what reaches the Filter compiler.
//!
//! The selectors here are synthetic rather than SagaShop-shaped on purpose. This
//! crate's job is the grammar; whether the grammar reproduces the testbed's
//! hand-written Specifications is a question about the testbed, and it is asked
//! in `core/tests/catalogue_grounding.rs` where those fixtures live.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use sentinel_compile::catalogue::{
    catalogue, declaration, instantiate, CorpusExercise, CounterpartMatches, EvidenceBinding,
    InstanceError, InstanceWarning, ParameterKind, ParameterValue, PatternId, PatternInstance,
    RetirementPolicy, SelectorValue, SettledEvidence, TriggerExercise,
};
use sentinel_compile::ir::{RetirementIr, SelectorIr};
use sentinel_compile::{
    activation_watermarks_for, aggregate_by_pattern, compile_filter, ActivationStamp, CompileError,
    FilterIdentity, FilterSource, WidthReport,
};
use sentinel_model::{
    wire::model::v1 as raw_model, ActivationBarrier, AttributeDouble, AttributeKey, AttributeValue,
    ClockDomainId, DataClass, Event, EventFilter, EventId, EventKind, FilterEpoch, Integrity,
    ObservationTime, OccurrenceTime, ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId,
    SequenceEpoch, SequenceNumber, SourceAssurance, SourceCapabilities, SourceCapability,
    SourceTier,
};

use support::{
    absence_arguments, absence_evidence, aggregation_arguments, aggregation_evidence, arguments,
    attr_key, bindings_for, capable, conforming, contract, evidence, identity, selector, source,
    taint_arguments, taint_evidence, ABSENCE_AFTER_KIND, ABSENCE_UNTIL_KIND, ABSENCE_VETO_KIND,
    AGGREGATION_MEMBER_KIND, AGGREGATION_TRIGGER_KIND, BINDING, EXISTENCE_CLOSE_KIND,
    EXISTENCE_OPEN_KIND, EXISTENCE_REQUIRED_KIND, TAINT_ACTION_KIND, TAINT_MARKER_KIND,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
//
// The conforming bindings for every catalogued Pattern live in `support/`,
// shared with `subsumption.rs` (issue #177). What stays here is only what this
// file reads.

/// Reads the precedence IR a CAT-1 instance lowered to.
fn precedence_ir(instance: &PatternInstance) -> &sentinel_compile::ir::PrecedenceIr {
    instance
        .ir()
        .as_precedence()
        .expect("CAT-1 lowers to the precedence monitor")
}

fn instance() -> PatternInstance {
    instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        arguments(),
        evidence(SourceTier::Anchor),
    )
    .expect("a fully bound instantiation succeeds")
}

fn cat4_identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.untrusted-content-must-not-act".to_owned(),
        specification_version: "v1".to_owned(),
    }
}

fn cat4_instance() -> PatternInstance {
    instantiate(
        PatternId::ProvenanceTaint,
        cat4_identity(),
        contract(),
        taint_arguments(),
        taint_evidence(SourceTier::Anchor, SourceTier::Anchor),
    )
    .expect("a fully bound instantiation succeeds")
}

fn publication_stamp(instance: &PatternInstance) -> ActivationStamp {
    let observed_at = ObservationTime::new(
        SentinelClockId::new("sentinel").expect("constant"),
        1_700_000_000_000,
    );
    ActivationStamp {
        observed_at,
        sequence_epochs: instance
            .required_sources()
            .into_iter()
            .map(|identity| (identity, SequenceEpoch::new(1)))
            .collect(),
    }
}

fn assert_one_watermark_per_required_source(instance: &PatternInstance) {
    let stamp = publication_stamp(instance);
    let required = instance.required_sources();
    let watermarks =
        activation_watermarks_for(instance, &stamp).expect("every required source is stamped");
    assert_eq!(watermarks.len(), required.len());
    for (watermark, identity) in watermarks.iter().zip(required.iter()) {
        let source = watermark.source.as_ref().expect("watermark names a source");
        assert_eq!(
            source.monitored_application_id,
            identity.monitored_application().as_str()
        );
        assert_eq!(source.source_id, identity.source().as_str());
        assert_eq!(watermark.observed_at, Some(stamp.observed_at.to_wire()));
        assert_eq!(watermark.complete_through, 0);
        assert_eq!(watermark.sequence_epoch, 1);
    }
}

// ---------------------------------------------------------------------------
// Lowering
// ---------------------------------------------------------------------------

/// Every lowered monitor carries the retirement policy its Pattern declares.
///
/// Read through `MonitorIr::retirement` rather than per family, because that
/// accessor is the one the Verification Core's clause (d0) consults: a Binding
/// is retirable exactly when its Pattern says so, and CAT-4's refusal to retire
/// is therefore a consequence of its own declaration rather than of a family
/// check hardcoded into the evaluator. Lowering the policy for one family only
/// would discard a declared fact and force the Core to re-assert it as a
/// constant.
///
/// Lives here rather than in `catalogue_invariants.rs` because it asserts on the
/// *lowering*, and `conforming` is the fixture that drives every loop in this
/// file — so a Pattern added to the catalogue fails this alongside the rest
/// rather than being quietly omitted from it.
#[test]
fn every_lowered_monitor_carries_its_declared_retirement_policy() {
    for pattern in catalogue() {
        let lowered = conforming(pattern.id);
        let expected = match pattern.retirement {
            RetirementPolicy::NotRequired => RetirementIr::NotRequired,
            RetirementPolicy::Deadline => RetirementIr::Deadline,
            RetirementPolicy::Inactivity => RetirementIr::Inactivity,
            other => panic!("{other:?} is catalogued with no IR dual in this suite"),
        };
        assert_eq!(
            lowered.ir().retirement(),
            expected,
            "{}: declared {} but lowered to {:?}",
            pattern.id,
            pattern.retirement.as_str(),
            lowered.ir().retirement()
        );
    }
}

/// CAT-1 lowers to the one monitor IR variant, carrying every bound value.
#[test]
fn cat1_lowers_to_precedence_ir() {
    let built = instance();
    let ir = precedence_ir(&built);

    assert_eq!(ir.specification_id, "example.action-needs-approval");
    assert_eq!(ir.specification_version, "v1");
    assert_eq!(ir.binding_attribute, "example.request.id");
    assert_eq!(ir.trigger.kind.as_str(), "example.action.executed");
    assert!(ir.trigger.required_attributes.is_empty());
    assert_eq!(ir.requirement.kind.as_str(), "example.approval.granted");
    assert_eq!(ir.requirement.required_attributes.len(), 1);

    // The producer schema contract reaches the IR without ever having been a
    // Pattern parameter.
    assert_eq!(ir.schema_version.as_str(), "example-contract-v1");
}

/// **Every declared parameter is consumed by instantiation.**
///
/// Asserted by perturbation rather than by inspection: bind one parameter
/// differently and the lowered IR must differ. A descriptor nobody reads would
/// let #21 generate a value that silently does nothing, and the parameter schema
/// is the whole product #21 and #35 consume.
#[test]
fn every_declared_parameter_is_consumed_by_instantiation() {
    for declared in catalogue() {
        let base = conforming(declared.id);
        let (arguments, evidence) = bindings_for(declared.id);

        for parameter in declared.parameters {
            let mut perturbed = arguments.clone();
            let replacement = match parameter.kind {
                ParameterKind::AttributeKey => {
                    ParameterValue::AttributeKey(attr_key("example.other.id"))
                }
                ParameterKind::Selector => {
                    selector("example.other.kind", &[("example.other", "x")])
                }
                ParameterKind::SequenceLookbackBound => ParameterValue::SequenceLookbackBound(99),
                ParameterKind::CountThreshold => ParameterValue::CountThreshold(99),
                other => panic!("no perturbation is defined for parameter kind {other:?}"),
            };
            perturbed.insert(parameter.name.to_owned(), replacement);

            let built = instantiate(
                declared.id,
                identity(),
                contract(),
                perturbed,
                evidence.clone(),
            )
            .expect("perturbed instantiation is still well formed");

            assert_ne!(
                built.ir(),
                base.ir(),
                "{}: parameter {} is declared but never read: rebinding it changed nothing",
                declared.id,
                parameter.name
            );
        }
    }
}

/// Every declared evidence role is consumed too — rebinding a role changes the
/// required-source set the Core reads.
#[test]
fn every_declared_evidence_role_is_consumed_by_instantiation() {
    for declared in catalogue() {
        let base = conforming(declared.id);
        let (arguments, evidence) = bindings_for(declared.id);

        for requirement in declared.evidence {
            let mut perturbed = evidence.clone();
            perturbed.insert(
                requirement.role.to_owned(),
                EvidenceBinding::new(source("example-other"), SourceTier::Anchor, capable()),
            );
            let built = instantiate(
                declared.id,
                identity(),
                contract(),
                arguments.clone(),
                perturbed,
            )
            .expect("perturbed instantiation is still well formed");

            assert_ne!(
                built.ir(),
                base.ir(),
                "{}: evidence role {} is declared but never read",
                declared.id,
                requirement.role
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

/// Each declared parameter is required; leaving any one out is a refusal.
#[test]
fn missing_required_parameter_is_rejected() {
    for parameter in declaration(PatternId::PrecedenceApproval).parameters {
        let mut incomplete = arguments();
        incomplete.remove(parameter.name);
        let error = instantiate(
            PatternId::PrecedenceApproval,
            identity(),
            contract(),
            incomplete,
            evidence(SourceTier::Anchor),
        )
        .expect_err("an unbound parameter is a refusal");
        assert_eq!(
            error,
            InstanceError::MissingParameter {
                pattern: PatternId::PrecedenceApproval,
                parameter: parameter.name,
            }
        );
    }
}

/// An argument the Pattern does not declare is a refusal, not a shrug.
///
/// Silently ignoring it would let generation emit a parameter that does nothing
/// and read back as accepted — the failure mode a declared grammar exists to
/// prevent (ADR-0008: well-formedness holds by construction).
#[test]
fn unknown_parameter_is_rejected() {
    let mut extra = arguments();
    extra.insert(
        "deadline_ms".to_owned(),
        ParameterValue::AttributeKey(attr_key("500")),
    );
    let error = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        extra,
        evidence(SourceTier::Anchor),
    )
    .expect_err("an undeclared argument is a refusal");
    assert_eq!(
        error,
        InstanceError::UnknownParameter {
            pattern: PatternId::PrecedenceApproval,
            parameter: "deadline_ms".to_owned(),
        }
    );
}

/// A selector parameter bound to an attribute key is a refusal.
#[test]
fn parameter_kind_mismatch_is_rejected() {
    let mut wrong = arguments();
    wrong.insert(
        "trigger".to_owned(),
        ParameterValue::AttributeKey(attr_key("example.action.executed")),
    );
    let error = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        wrong,
        evidence(SourceTier::Anchor),
    )
    .expect_err("a mistyped parameter is a refusal");
    assert_eq!(
        error,
        InstanceError::ParameterKindMismatch {
            pattern: PatternId::PrecedenceApproval,
            parameter: "trigger",
            expected: ParameterKind::Selector,
            actual: ParameterKind::AttributeKey,
        }
    );
}

/// An unbound evidence role is a refusal: the required-source set would be
/// incomplete, and an incomplete one is `filter.rs`'s undetectable unsoundness.
#[test]
fn unbound_evidence_role_is_rejected() {
    for requirement in declaration(PatternId::PrecedenceApproval).evidence {
        let mut incomplete = evidence(SourceTier::Anchor);
        incomplete.remove(requirement.role);
        let error = instantiate(
            PatternId::PrecedenceApproval,
            identity(),
            contract(),
            arguments(),
            incomplete,
        )
        .expect_err("an unbound evidence role is a refusal");
        assert_eq!(
            error,
            InstanceError::MissingEvidenceBinding {
                pattern: PatternId::PrecedenceApproval,
                role: requirement.role,
            }
        );
    }
}

/// A source bound to a role the Pattern does not declare is a refusal.
#[test]
fn unknown_evidence_role_is_rejected() {
    let mut extra = evidence(SourceTier::Anchor);
    extra.insert(
        "sanitizer_source".to_owned(),
        EvidenceBinding::new(source("example-scrubber"), SourceTier::Anchor, capable()),
    );
    let error = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        arguments(),
        extra,
    )
    .expect_err("an undeclared evidence role is a refusal");
    assert_eq!(
        error,
        InstanceError::UnknownEvidenceRole {
            pattern: PatternId::PrecedenceApproval,
            role: "sanitizer_source".to_owned(),
        }
    );
}

// ---------------------------------------------------------------------------
// Warnings
// ---------------------------------------------------------------------------

/// A deployment that meets every declared requirement warns about nothing.
#[test]
fn a_conforming_deployment_produces_no_warnings() {
    assert_eq!(instance().validate(), Vec::new());
}

/// **A tier shortfall is reported, not swallowed — and not an error.**
///
/// ADR-0022 makes the required tier a Specification property and the placement a
/// deployment fact. Refusing to instantiate would be this crate overruling a
/// deployment it cannot see; silently accepting would be ADR-0017's silent pass
/// one layer earlier. A warning is the only honest third option.
#[test]
fn tier_shortfall_is_reported_not_swallowed() {
    let built = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        arguments(),
        evidence(SourceTier::Contributing),
    )
    .expect("a tier shortfall does not block instantiation");

    assert_eq!(
        built.validate(),
        vec![InstanceWarning::TierShortfall {
            role: "requirement_source",
            source: source("example-authz"),
            required: SourceTier::Anchor,
            placement: SourceTier::Contributing,
        }]
    );

    // And the Specification still demands anchor tier. A Specification whose
    // demands shrank to match a weak deployment could never report that the
    // deployment is weak.
    let ir = precedence_ir(&built);
    assert_eq!(
        ir.required_sources.get(&source("example-authz")),
        Some(&SourceTier::Anchor)
    );
}

/// A source that does not assert a capability the role depends on is warned about.
#[test]
fn capability_shortfall_is_reported_not_swallowed() {
    let mut degraded = evidence(SourceTier::Anchor);
    degraded.insert(
        "requirement_source".to_owned(),
        EvidenceBinding::new(
            source("example-authz"),
            SourceTier::Anchor,
            SourceCapabilities::new([SourceCapability::CausalEdges]),
        ),
    );
    let built = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        arguments(),
        degraded,
    )
    .expect("a capability shortfall does not block instantiation");

    assert_eq!(
        built.validate(),
        vec![InstanceWarning::CapabilityShortfall {
            role: "requirement_source",
            source: source("example-authz"),
            capability: SourceCapability::CompletenessSignals,
        }]
    );
}

/// Two roles binding one source keep the stricter minimum tier.
#[test]
fn two_roles_binding_one_source_keep_the_stricter_minimum() {
    let shared = source("example-monolith");
    let bindings = BTreeMap::from([
        (
            "trigger_source".to_owned(),
            EvidenceBinding::new(shared.clone(), SourceTier::Anchor, capable()),
        ),
        (
            "requirement_source".to_owned(),
            EvidenceBinding::new(shared.clone(), SourceTier::Anchor, capable()),
        ),
    ]);
    let built = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        arguments(),
        bindings,
    )
    .expect("one source may fill both roles");

    let ir = precedence_ir(&built);
    assert_eq!(ir.required_sources.len(), 1);
    assert_eq!(
        ir.required_sources.get(&shared),
        Some(&SourceTier::Anchor),
        "the contributing minimum of trigger_source must not weaken the anchor minimum"
    );
}

// ---------------------------------------------------------------------------
// The Filter seam
// ---------------------------------------------------------------------------

/// A Pattern instance is a `FilterSource`, and the Filter it compiles to is sound.
///
/// This is the seam `compile/src/filter.rs` reserved: "the
/// over-approximation-via-projection seam #14 Patterns plug into". The
/// dependency set is derived from the instance's own selectors, so the trust
/// boundary that file names — a Specification omitting a dependency it reads is
/// unsound and undetectable — cannot be crossed by a catalogued Pattern.
#[test]
fn pattern_instance_is_a_filter_source() {
    let built = instance();
    let compiled = compile_filter(&[&built], FilterEpoch::new(3)).expect("instance compiles");

    let emitted = &compiled.specifications()[0];
    let event_match = emitted.event_match.as_ref().expect("event_match present");
    let kinds: BTreeSet<&str> = event_match.event_kinds.iter().map(String::as_str).collect();
    let keys: BTreeSet<&str> = event_match
        .projected_attribute_keys
        .iter()
        .map(String::as_str)
        .collect();

    // S1: both selector kinds admitted.
    assert!(kinds.contains("example.action.executed"));
    assert!(kinds.contains("example.approval.granted"));
    // S2: the Binding attribute is retained even though no selector requires it.
    assert!(
        keys.contains("example.request.id"),
        "binding attribute dropped; binding_of() would find nothing: {keys:?}"
    );
    assert!(keys.contains("example.approval.outcome"));

    // Required sources reach the Filter.
    let emitted_sources: BTreeSet<String> = emitted
        .required_sources
        .iter()
        .map(|entry| entry.monitored_application_id.clone())
        .collect();
    assert_eq!(
        emitted_sources,
        BTreeSet::from(["example-authz".to_owned(), "example-gateway".to_owned()])
    );
}

/// The width budget an instance reports is the **template's**, not its own.
///
/// This is half of what `compile/src/width.rs` asked #14 for: "FilterWidthBudget
/// is the per-Pattern constant carried per-instance today; #14 will lift it to
/// the template". Every instance of CAT-1 now reports against one declared
/// budget, so the ADR-0008 unit-economics question is asked of the Pattern.
#[test]
fn width_budget_is_read_from_the_template() {
    let built = instance();
    let declared = declaration(PatternId::PrecedenceApproval).width.budget;
    assert_eq!(built.width_budget(), declared);

    let compiled = compile_filter(&[&built], FilterEpoch::new(3)).expect("instance compiles");
    let report = &compiled.widths[0];
    assert_eq!(
        report.declared_width,
        declared.event_kinds + declared.attribute_keys
    );
    // Two kinds, two keys: within CAT-1's declared 2 + 4.
    assert_eq!(report.actual_event_kinds, 2);
    assert_eq!(report.actual_attribute_keys, 2);
    assert!(
        !report.over_budget,
        "CAT-1's own budget should hold for a CAT-1 instance: {report:?}"
    );
}

/// Every catalogued Pattern is instantiable, and its declaration is reachable
/// from the instance.
#[test]
fn every_catalogued_pattern_has_a_lowering() {
    for pattern in catalogue() {
        let built = conforming(pattern.id);
        assert_eq!(built.pattern(), pattern.id);
        assert_eq!(built.declaration().id, pattern.id);
    }
}

/// **Each Pattern lowers to the monitor its own family needs.**
///
/// `PatternInstance::precedence_ir` and `taint_ir` are reachable only from their
/// own arm of an exhaustive `match` on `PatternId`, and each reads the IR through
/// an accessor that returns `None` for the other variant. That pairing is
/// maintained by `lower` and by nothing else, so this is the test those two
/// accessors' doc comments name: it fails if `lower` is edited so that a Pattern
/// produces a monitor of the wrong family.
///
/// The negative half is the point. Asserting only that CAT-4 lowers to
/// `MonitorIr::Taint` would pass for a `MonitorIr` that answered `Some` to both
/// accessors, and a taint monitor read as a precedence obligation is a Verdict
/// inversion rather than a type error at the call site.
/// **Every catalogued Pattern lowers to exactly one monitor family.**
#[allow(clippy::too_many_lines)] // one arm per catalogued Pattern; exhaustiveness is the point
#[test]
fn every_pattern_lowers_to_the_monitor_its_family_needs() {
    for pattern in catalogue() {
        let built = conforming(pattern.id);
        match pattern.id {
            PatternId::PrecedenceApproval => {
                assert!(built.ir().as_precedence().is_some(), "{}", pattern.id);
                assert!(
                    built.ir().as_taint().is_none(),
                    "{}: a precedence obligation must not read as a taint prohibition",
                    pattern.id
                );
                assert!(
                    built.ir().as_absence().is_none(),
                    "{}: a precedence obligation must not read as an absence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_aggregate().is_none(),
                    "{}: a precedence obligation must not read as an aggregation obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_existence().is_none(),
                    "{}: a precedence obligation must not read as an existence obligation",
                    pattern.id
                );
            }
            PatternId::ProvenanceTaint => {
                assert!(built.ir().as_taint().is_some(), "{}", pattern.id);
                assert!(
                    built.ir().as_precedence().is_none(),
                    "{}: a taint prohibition must not read as a precedence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_absence().is_none(),
                    "{}: a taint prohibition must not read as an absence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_aggregate().is_none(),
                    "{}: a taint prohibition must not read as an aggregation obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_existence().is_none(),
                    "{}: a taint prohibition must not read as an existence obligation",
                    pattern.id
                );
            }
            PatternId::AbsenceAfterUntil => {
                assert!(built.ir().as_absence().is_some(), "{}", pattern.id);
                assert!(
                    built.ir().as_precedence().is_none(),
                    "{}: an absence obligation must not read as a precedence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_taint().is_none(),
                    "{}: an absence obligation must not read as a taint prohibition",
                    pattern.id
                );
                assert!(
                    built.ir().as_aggregate().is_none(),
                    "{}: an absence obligation must not read as an aggregation obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_existence().is_none(),
                    "{}: an absence obligation must not read as an existence obligation",
                    pattern.id
                );
            }
            PatternId::AggregationCountWithin => {
                assert!(built.ir().as_aggregate().is_some(), "{}", pattern.id);
                assert!(
                    built.ir().as_precedence().is_none(),
                    "{}: an aggregation obligation must not read as a precedence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_taint().is_none(),
                    "{}: an aggregation obligation must not read as a taint prohibition",
                    pattern.id
                );
                assert!(
                    built.ir().as_absence().is_none(),
                    "{}: an aggregation obligation must not read as an absence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_existence().is_none(),
                    "{}: an aggregation obligation must not read as an existence obligation",
                    pattern.id
                );
            }
            PatternId::ExistenceBeforeScopeClose => {
                assert!(built.ir().as_existence().is_some(), "{}", pattern.id);
                assert!(
                    built.ir().as_precedence().is_none(),
                    "{}: an existence obligation must not read as a precedence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_taint().is_none(),
                    "{}: an existence obligation must not read as a taint prohibition",
                    pattern.id
                );
                assert!(
                    built.ir().as_absence().is_none(),
                    "{}: an existence obligation must not read as an absence obligation",
                    pattern.id
                );
                assert!(
                    built.ir().as_aggregate().is_none(),
                    "{}: an existence obligation must not read as an aggregation obligation",
                    pattern.id
                );
            }
            other => panic!("{other} is catalogued with no expected monitor family here"),
        }
    }
}

/// Aggregation reports nothing for a Pattern with no instances, and sums the
/// ones it has.
///
/// The real three-instance case runs against SagaShop P4/P6/P8 in
/// `core/tests/catalogue_grounding.rs`, where the fixtures live. This covers the
/// two edges that case does not reach.
#[test]
fn aggregate_by_pattern_reports_only_patterns_with_instances() {
    assert!(aggregate_by_pattern(&[]).is_empty());

    let budget = declaration(PatternId::PrecedenceApproval).width.budget;
    let narrow = WidthReport::new(identity(), 2, 2, budget);
    let wide = WidthReport::new(
        FilterIdentity {
            specification_id: "example.other".to_owned(),
            specification_version: "v1".to_owned(),
        },
        4,
        9,
        budget,
    );
    let aggregated = aggregate_by_pattern(&[
        (PatternId::PrecedenceApproval, narrow),
        (PatternId::PrecedenceApproval, wide),
    ]);

    let report = &aggregated[&PatternId::PrecedenceApproval];
    assert_eq!(report.instances, 2);
    assert_eq!(report.total_actual_width, 4 + 13);
    assert_eq!(report.max_actual_width, 13);
    assert_eq!(report.declared_width, Some(6));
    // Telemetry only: the over-budget instance is counted, never truncated and
    // never rejected. `compile/src/width.rs` pins that policy.
    assert_eq!(report.instances_over_budget, 1);
}

// ---------------------------------------------------------------------------
// What the selectors actually matched (issue #144)
// ---------------------------------------------------------------------------
//
// A Specification whose trigger selector matches no Event evaluates cleanly,
// produces zero Verdicts, and reads as a pass. These tests are the census that
// makes it observable at Candidate validation, before a Verdict can exist.
//
// Two of them are the pair that matters, and they are deliberately adjacent:
// `the_corpus_fold_separates_vacuous_evidence_from_an_unexercised_specification`
// asserts that a Specification which measures nothing on *one* evidence set is
// **not** the defect, while one that measures nothing on *every* evidence set
// is. Any signal keyed on "the trigger matched nothing" alone conflates them,
// which would break the correct behaviour of an obligation that never arose.

/// The Event kind the base instance's trigger selects.
const TRIGGER_KIND: &str = "example.action.executed";

fn text(value: &str) -> AttributeValue {
    AttributeValue::String(value.to_owned())
}

/// Builds one Event of a kind carrying the given attribute valuations.
///
/// Assembled from `sentinel_model`'s constructors rather than through the
/// Adapter: `sentinel-compile` has no dev-dependencies, and acquiring one on
/// `sentinel-ingest` would invert the layering — `ingest` sits *downstream* of
/// `compile` in the evidence flow, so a test dependency the other way would let
/// a change in the Adapter break the compiler's own suite.
fn event(sequence: u64, kind: &str, attributes: &[(&str, AttributeValue)]) -> Event {
    let producer = ProducerEvent::new(
        EventId::new(format!("event-{sequence}")).expect("constant"),
        SequenceEpoch::new(1),
        SequenceNumber::new(sequence),
        SchemaVersion::new("example-contract-v1").expect("constant"),
        FilterEpoch::new(1),
        EventKind::new(kind).expect("constant"),
        OccurrenceTime::new(
            ClockDomainId::new("example-producer").expect("constant"),
            i128::from(sequence),
            0,
        ),
        attributes
            .iter()
            .map(|(key, value)| (attr_key(key), value.clone()))
            .collect(),
        SourceCapabilities::new([]),
        Sensitivity::Internal,
        BTreeSet::new(),
    )
    .expect("a well-formed producer Event");

    let assurance = SourceAssurance::new(
        source("example-gateway"),
        SourceTier::Contributing,
        SourceCapabilities::new([]),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("well-formed source assurance");

    Event::observe(
        producer,
        assurance,
        ObservationTime::new(
            SentinelClockId::new("sentinel").expect("constant"),
            i128::from(sequence),
        ),
        DataClass::Tenant,
    )
}

/// An Event that matches the base instance's trigger and names a Binding.
fn triggering_event(sequence: u64, request: &str) -> Event {
    event(sequence, TRIGGER_KIND, &[(BINDING, text(request))])
}

/// An Event that matches the base instance's requirement selector.
fn approving_event(sequence: u64, request: &str) -> Event {
    event(
        sequence,
        "example.approval.granted",
        &[
            (BINDING, text(request)),
            ("example.approval.outcome", text("allow")),
        ],
    )
}

/// Evidence in which the obligation is raised and discharged.
fn exercising_evidence() -> Vec<Event> {
    vec![approving_event(1, "r-1"), triggering_event(2, "r-1")]
}

/// Evidence in which the obligation genuinely never arose.
///
/// The legitimate vacuous case, and the reason a per-evidence-set signal cannot
/// be read as a defect: this Specification is correct, and it measures nothing
/// here because nothing happened that it is about.
fn unexercising_evidence() -> Vec<Event> {
    vec![approving_event(1, "r-2")]
}

/// Builds a CAT-1 instance varying only the Binding attribute and the trigger.
fn instance_with(
    binding_attribute: &str,
    trigger_kind: &str,
    trigger_attributes: &[(&str, &str)],
) -> PatternInstance {
    let mut bound = arguments();
    bound.insert(
        "binding_attribute".to_owned(),
        ParameterValue::AttributeKey(attr_key(binding_attribute)),
    );
    bound.insert(
        "trigger".to_owned(),
        selector(trigger_kind, trigger_attributes),
    );
    instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        bound,
        evidence(SourceTier::Anchor),
    )
    .expect("a well-formed instantiation succeeds whatever the deployment emits")
}

fn nonzero(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("a test constant that is not zero")
}

/// **A trigger naming an Event kind nobody emits is reported.**
///
/// Issue #144's acceptance criterion, at the seam it names. `example.action.executd`
/// is `example.action.executed` with one character missing — the shape of typo
/// #21's generator or #35's form will produce, and the same shape as a Candidate
/// proposed against an Intent Artifact naming a tool the deployment does not
/// call.
///
/// Three assertions, in the order that makes the argument:
///
/// 1. `instantiate` accepts it, because a well-formed `EventKind` naming
///    something nobody emits is indistinguishable from a real kind whose
///    producer has not shipped yet;
/// 2. `validate()` is still `[]` — the honest statement of the gap this issue
///    reported, kept as a live assertion so a later change that closes it there
///    tells someone rather than silently widening the contract;
/// 3. `validate_against` reports it, naming the kind and the denominator.
#[test]
fn a_trigger_naming_an_unemitted_event_kind_is_reported() {
    let mistyped = instance_with(BINDING, "example.action.executd", &[]);
    let events = exercising_evidence();

    assert!(
        mistyped.validate().is_empty(),
        "validate() compares tiers and capabilities, not values: it has nothing to say here"
    );

    let censused = mistyped.exercise(SettledEvidence::asserted(&events));
    assert_eq!(
        censused.trigger,
        TriggerExercise::KindNeverOccurred {
            events_examined: nonzero(2)
        }
    );
    assert_eq!(censused.events_examined, 2);

    let warnings = mistyped.validate_against(SettledEvidence::asserted(&events));
    assert_eq!(
        warnings,
        vec![InstanceWarning::TriggerKindNeverOccurred {
            parameter: "trigger",
            kind: EventKind::new("example.action.executd").expect("constant"),
            events_examined: nonzero(2),
        }],
        "a Specification that raised no obligation was not reported"
    );
}

/// The control that makes the test above non-trivial.
///
/// Same Pattern, same evidence, the real trigger kind: no warning, and the
/// census says an obligation was raised. Without this, "the mistyped trigger is
/// reported" would be equally consistent with every instance being reported.
#[test]
fn a_trigger_that_matches_is_not_reported() {
    let sound = instance_with(BINDING, TRIGGER_KIND, &[]);
    let events = exercising_evidence();

    assert_eq!(
        sound.exercise(SettledEvidence::asserted(&events)).trigger,
        TriggerExercise::RaisedObligations {
            matches: nonzero(1),
            obligations: nonzero(1),
        }
    );
    assert!(sound
        .validate_against(SettledEvidence::asserted(&events))
        .is_empty());
}

/// **A valuation nobody sets is a different mistake from a kind nobody emits.**
///
/// The kind occurs; the required attribute valuation never holds. Remediation is
/// "fix the valuation", not "fix the kind", so the census says so rather than
/// collapsing both into one answer.
#[test]
fn a_trigger_whose_valuation_never_holds_is_reported_differently() {
    let over_specified = instance_with(
        BINDING,
        TRIGGER_KIND,
        &[("example.action.effect", "mutating")],
    );
    let events = exercising_evidence();

    let censused = over_specified.exercise(SettledEvidence::asserted(&events));
    assert_eq!(
        censused.trigger,
        TriggerExercise::ValuationNeverMatched {
            kind_matches: nonzero(1)
        },
        "the Event kind does occur here, so this is not a kind mistake"
    );
    assert_eq!(
        over_specified.validate_against(SettledEvidence::asserted(&events)),
        vec![InstanceWarning::TriggerValuationNeverMatched {
            parameter: "trigger",
            kind: EventKind::new(TRIGGER_KIND).expect("constant"),
            kind_matches: nonzero(1),
        }]
    );
}

/// **Triggers that name no Binding raise no obligation, and that is its own
/// answer.**
///
/// `example.request.i` is `example.request.id` with one character missing. The
/// trigger still matches, so this is not a selector mistake; there is simply no
/// Slice for a Verdict to live in. `core/tests/catalogue_grounding.rs` had to
/// catch this after evaluation because `sentinel-compile` could not see the
/// evidence. It can now.
#[test]
fn a_trigger_matching_events_that_name_no_binding_is_reported_differently() {
    let mis_bound = instance_with("example.request.i", TRIGGER_KIND, &[]);
    let events = exercising_evidence();

    assert_eq!(
        mis_bound
            .exercise(SettledEvidence::asserted(&events))
            .trigger,
        TriggerExercise::MatchedWithoutBinding {
            matches: nonzero(1)
        }
    );
    assert_eq!(
        mis_bound.validate_against(SettledEvidence::asserted(&events)),
        vec![InstanceWarning::TriggerRaisedNoObligation {
            parameter: "trigger",
            binding_attribute: "example.request.i".to_owned(),
            matches: nonzero(1),
        }]
    );
}

/// **An obligation needs one Event that both matches and names a Binding.**
///
/// The disjoint case, constructed: Event 1 matches the trigger and carries no
/// Binding attribute; Event 2 names a Binding and is of another kind. A census
/// carrying two independent totals — one trigger match, one Binding carrier —
/// would report an obligation. The Verification Core emits nothing here:
/// `Evaluator::step_event` admits an Event to a Slice only when `binding_of`
/// returns a Binding, and `precedence::assess` skips a Binding whose trigger set
/// is empty.
///
/// This is the difference between a census that agrees with the evaluator and
/// one that is merely nearly right.
#[test]
fn an_obligation_needs_one_event_that_both_matches_and_binds() {
    let sound = instance_with(BINDING, TRIGGER_KIND, &[]);
    let events = vec![
        event(1, TRIGGER_KIND, &[("example.unrelated", text("x"))]),
        approving_event(2, "r-1"),
    ];

    let censused = sound.exercise(SettledEvidence::asserted(&events));
    assert_eq!(
        censused.binding_carriers, 1,
        "an Event does name a Binding here, which is what makes the case a trap"
    );
    assert_eq!(
        censused.trigger,
        TriggerExercise::MatchedWithoutBinding {
            matches: nonzero(1)
        },
        "the trigger match and the Binding carrier are different Events, so no obligation \
         was raised"
    );
    assert!(!censused.trigger.raised_an_obligation());
}

/// **A Binding attribute holding a non-`String` names no Binding.**
///
/// `PrecedenceSpecification::binding_of` reads only `AttributeValue::String`,
/// and `sentinel_compile::ir::binding_of` is now the one implementation of that
/// so the census cannot drift from it. An Event carrying the Binding attribute
/// as a `Bool` is filed under no Binding at all, which is easy to miss and
/// changes the answer.
#[test]
fn a_binding_attribute_that_is_not_a_string_names_no_binding() {
    let sound = instance_with(BINDING, TRIGGER_KIND, &[]);
    let events = vec![event(
        1,
        TRIGGER_KIND,
        &[(BINDING, AttributeValue::Bool(true))],
    )];

    let censused = sound.exercise(SettledEvidence::asserted(&events));
    assert_eq!(censused.binding_carriers, 0);
    assert_eq!(
        censused.trigger,
        TriggerExercise::MatchedWithoutBinding {
            matches: nonzero(1)
        }
    );
}

/// **A validation that examined no Event says so, and says nothing else.**
///
/// "Nothing matched, and I examined 0 Events" must not render as "nothing
/// matched, and I examined 122 Events." The first is a finding about the
/// validation; only the second is a finding about the Specification.
#[test]
fn evidence_holding_no_event_reports_that_nothing_was_established() {
    let sound = instance_with(BINDING, TRIGGER_KIND, &[]);
    let empty: Vec<Event> = Vec::new();

    let censused = sound.exercise(SettledEvidence::asserted(&empty));
    assert_eq!(censused.trigger, TriggerExercise::NoEvidenceExamined);
    assert!(!censused.trigger.examined_evidence());
    assert_eq!(
        sound.validate_against(SettledEvidence::asserted(&empty)),
        vec![InstanceWarning::NoEvidenceExamined],
        "an empty evidence set must not be reported as a Specification defect"
    );
}

/// `validate_against` is a superset of `validate`, not a replacement for it.
///
/// P7's shape: a requirement source the deployment places below the tier CAT-1
/// demands. The tier shortfall is reported whether or not the trigger was
/// exercised, and the census warning is added to it rather than replacing it.
#[test]
fn validate_against_still_reports_tier_and_capability_shortfalls() {
    let mut bound = arguments();
    bound.insert(
        "trigger".to_owned(),
        selector("example.action.executd", &[]),
    );
    let short = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        contract(),
        bound,
        evidence(SourceTier::Contributing),
    )
    .expect("a tier shortfall is a warning, never a refusal");

    let events = exercising_evidence();
    let warnings = short.validate_against(SettledEvidence::asserted(&events));

    assert_eq!(
        warnings.len(),
        2,
        "expected both findings, got {warnings:?}"
    );
    assert!(matches!(warnings[0], InstanceWarning::TierShortfall { .. }));
    assert!(matches!(
        warnings[1],
        InstanceWarning::TriggerKindNeverOccurred { .. }
    ));
    assert_eq!(
        short.validate(),
        vec![warnings[0].clone()],
        "validate()'s own answer must be unchanged by the addition"
    );
}

/// **Every selector this language can express is satisfiable by some Event.**
///
/// The static argument as an enumeration rather than as a paragraph, which is
/// what decides issue #144's compile-time-versus-evaluation-time question. A
/// `SelectorIr` is an Event kind plus a `BTreeMap` of required attribute
/// equalities keyed by [`AttributeKey`]: a map cannot demand two values for one
/// key, there is no negation, disjunction or range to contradict, and the
/// required values are drawn from the same `AttributeValue` an Event carries.
/// Every variant of it, including the finite-only `Double` and the recursive
/// `Array` and `KeyValueList`, is witnessed here by an Event that matches.
///
/// So there is no unsatisfiable well-formed selector for a compile-time check to
/// find. Blank keys are unrepresentable in the selector type itself (issue
/// #203). **Adding negation or ranges to the selector language creates a real
/// static subset, and whoever adds them owns the check for it.**
#[test]
fn every_selector_this_language_can_express_is_satisfiable_by_some_event() {
    let values = [
        AttributeValue::String("s".to_owned()),
        AttributeValue::Bool(true),
        AttributeValue::Integer(-7),
        AttributeValue::Double(AttributeDouble::try_from(1.5).expect("finite")),
        AttributeValue::Bytes(vec![0, 1, 2]),
        AttributeValue::Array(vec![AttributeValue::Integer(1)]),
        AttributeValue::KeyValueList(BTreeMap::from([(
            attr_key("inner"),
            AttributeValue::Bool(false),
        )])),
    ];

    for (index, value) in values.iter().enumerate() {
        let sequence = u64::try_from(index).expect("a small index") + 1;
        let witness = event(
            sequence,
            "example.any.kind",
            &[("example.key", value.clone())],
        );
        let demanding = SelectorIr::new(
            EventKind::new("example.any.kind").expect("constant"),
            BTreeMap::from([(attr_key("example.key"), value.clone())]),
        );
        assert!(
            demanding.matches(&witness),
            "a selector requiring {value:?} was not satisfied by an Event carrying it"
        );
    }
}

/// Blank attribute names are refused by [`AttributeKey`], not by instantiate.
///
/// Issue #203 closed the vacuous-pass shape that used to reach
/// `InstanceError::BlankSelectorAttributeKey` / `BlankAttributeKey`. Those
/// arms are gone; identity blanks still use [`InstanceError::BlankIdentity`]
/// because Pattern IR identity fields stay public `String`. Exhaustiveness
/// that `Blank*` attribute arms stay gone lives next to `InstanceError` in
/// `catalogue/instance.rs` (the enum is `#[non_exhaustive]` outside the crate).
#[test]
fn blank_attribute_names_are_refused_by_attribute_key_not_instantiate() {
    assert_eq!(AttributeKey::new(""), Err(sentinel_model::IdentifierError));
    assert_eq!(
        AttributeKey::new("   "),
        Err(sentinel_model::IdentifierError)
    );
    assert!(
        instantiate(
            PatternId::PrecedenceApproval,
            FilterIdentity {
                specification_id: "   ".to_owned(),
                specification_version: "v1".to_owned(),
            },
            contract(),
            arguments(),
            evidence(SourceTier::Anchor),
        )
        .is_err_and(|error| matches!(
            error,
            InstanceError::BlankIdentity {
                field: "specification_id",
                ..
            }
        )),
        "BlankIdentity remains for public String identity fields"
    );
}

/// **The corpus fold is what separates the defect from legitimate vacuity.**
///
/// The pair issue #144's own discriminator does not separate on its own. Both
/// Specifications below raise no obligation on `unexercising_evidence()`, and
/// per evidence set they are indistinguishable — which is correct, because on
/// that evidence they *are* the same fact. The difference is corpus-wide:
///
/// - the sound Specification is exercised somewhere, so its vacuous evidence set
///   is a true reading of an obligation that never arose and must keep passing;
/// - the mistyped one is exercised nowhere, so it has caught nothing by
///   construction, and ADR-0009 makes catching things a required Promotion gate.
///
/// Read the `Exercised { traces: 2, exercised_in: 1 }` below as the
/// counter-intuitive assertion it is: a correct Specification is censused as
/// having measured nothing on one of its two evidence sets, and that is not a
/// finding against it.
#[test]
fn the_corpus_fold_separates_vacuous_evidence_from_an_unexercised_specification() {
    let corpus = [exercising_evidence(), unexercising_evidence()];

    let sound = instance_with(BINDING, TRIGGER_KIND, &[]);
    let mistyped = instance_with(BINDING, "example.action.executd", &[]);

    let census = |instance: &PatternInstance| -> Vec<TriggerExercise> {
        corpus
            .iter()
            .map(|events| instance.exercise(SettledEvidence::asserted(events)).trigger)
            .collect()
    };

    let sound_census = census(&sound);
    assert_eq!(
        sound_census[1],
        TriggerExercise::KindNeverOccurred {
            events_examined: nonzero(1)
        },
        "the correct Specification measures nothing on the second evidence set, and that is \
         the vacuous truth it is supposed to report"
    );
    assert_eq!(
        CorpusExercise::of(sound_census),
        CorpusExercise::Exercised {
            traces: nonzero(2),
            exercised_in: nonzero(1),
        }
    );

    let mistyped_census = census(&mistyped);
    assert_eq!(
        CorpusExercise::of(mistyped_census),
        CorpusExercise::NeverExercised { traces: nonzero(2) },
        "a Specification exercised by nothing in its corpus is the defect #144 names"
    );
    assert!(!CorpusExercise::of(census(&mistyped)).was_exercised());
    assert!(CorpusExercise::of(census(&sound)).was_exercised());
}

/// A corpus that examined nothing is not evidence that a Specification works.
///
/// `NothingExamined` is neither a pass nor an accusation, and `was_exercised()`
/// is `false` for it — missing evidence is never a pass, and that includes the
/// evidence about whether the Specification measures anything.
#[test]
fn a_corpus_that_examined_nothing_exercised_nothing() {
    let sound = instance_with(BINDING, TRIGGER_KIND, &[]);
    let empty: Vec<Event> = Vec::new();

    let folded = CorpusExercise::of([
        sound.exercise(SettledEvidence::asserted(&empty)).trigger,
        sound.exercise(SettledEvidence::asserted(&empty)).trigger,
    ]);
    assert_eq!(folded, CorpusExercise::NothingExamined);
    assert!(!folded.was_exercised());
    assert_eq!(
        CorpusExercise::of(Vec::new()),
        CorpusExercise::NothingExamined
    );
}

/// **CAT-2 is censused on its until selector and never on its veto.**
///
/// The absence-shaped form of the trap `compile/src/catalogue/exercise.rs`
/// names. Replaces `no_catalogued_pattern_is_absence_shaped`: the day an
/// absence Pattern ships, the decision about what its census means is forced
/// here rather than defaulted into a precedence-shaped judgement.
///
/// Two evidence sets, each the other's inverse:
///
/// - a **clean** deployment closes an interval (`until`) with an opening
///   delimiter and no veto. It raised an obligation and measured something.
/// - a deployment full of vetoes and no `until` raised **no** obligation,
///   because nothing closed an interval for the property to be about.
///
/// Keying the census on the veto swaps both answers.
#[test]
fn cat2_censuses_its_until_selector_and_never_its_veto() {
    let built = conforming(PatternId::AbsenceAfterUntil);

    let clean = built.exercise(SettledEvidence::asserted(&absence_clean_evidence()));
    assert_eq!(
        clean.trigger,
        TriggerExercise::RaisedObligations {
            matches: nonzero(1),
            obligations: nonzero(1),
        },
        "an until occurred, so an obligation was raised and something was measured"
    );
    assert!(clean.trigger.raised_an_obligation());
    assert_eq!(
        clean.counterpart,
        CounterpartMatches::Interval {
            opening: 1,
            falsifying: 0,
        },
        "the interval opened once and no veto matched, which is a clean close"
    );
    assert_eq!(clean.counterpart.matches(), 0);
    assert_eq!(clean.counterpart.as_str(), "interval");

    let veto_only = built.exercise(SettledEvidence::asserted(&absence_veto_only_evidence()));
    assert_eq!(
        veto_only.trigger,
        TriggerExercise::KindNeverOccurred {
            events_examined: nonzero(2)
        },
        "the until kind never occurred, so nothing was measured — censusing the veto \
         instead would have called this the exercised case"
    );
    assert!(!veto_only.trigger.raised_an_obligation());
    assert_eq!(
        veto_only.counterpart,
        CounterpartMatches::Interval {
            opening: 0,
            falsifying: 2,
        },
        "both vetoes matched, and that is reported rather than judged"
    );
    assert_eq!(veto_only.counterpart.matches(), 2);
}

// ---------------------------------------------------------------------------
// CAT-4 provenance.taint (issue #141)
// ---------------------------------------------------------------------------
//
// The Pattern that takes the catalogue to both flagship families, and the first
// one whose second selector must *never* match where it counts. Every test below
// exists because getting that backwards is plausible rather than absurd: a
// census keyed on the untrusted-content marker would report a clean deployment
// as never exercised, and a Filter that dropped the marker would make every
// Verdict read clean.

/// Reads the absence IR a CAT-2 instance lowered to.
fn absence_ir(instance: &PatternInstance) -> &sentinel_compile::ir::AbsenceIr {
    instance
        .ir()
        .as_absence()
        .expect("CAT-2 lowers to the absence monitor")
}

fn after_event(sequence: u64, request: &str) -> Event {
    event(sequence, ABSENCE_AFTER_KIND, &[(BINDING, text(request))])
}

fn until_event(sequence: u64, request: &str) -> Event {
    event(sequence, ABSENCE_UNTIL_KIND, &[(BINDING, text(request))])
}

fn veto_event(sequence: u64, request: &str) -> Event {
    event(sequence, ABSENCE_VETO_KIND, &[(BINDING, text(request))])
}

/// Evidence from a deployment that opened and closed an interval with no veto.
fn absence_clean_evidence() -> Vec<Event> {
    vec![after_event(1, "r-1"), until_event(2, "r-1")]
}

/// Evidence full of vetoes and no until — the inverse population.
fn absence_veto_only_evidence() -> Vec<Event> {
    vec![veto_event(1, "r-2"), veto_event(2, "r-3")]
}

/// CAT-2 lowers to the absence monitor IR, carrying every bound value.
#[test]
fn cat2_lowers_to_absence_ir() {
    let built = conforming(PatternId::AbsenceAfterUntil);
    let ir = absence_ir(&built);

    assert_eq!(ir.specification_id, identity().specification_id);
    assert_eq!(ir.specification_version, identity().specification_version);
    assert_eq!(ir.binding_attribute, BINDING);
    assert_eq!(ir.after.kind.as_str(), ABSENCE_AFTER_KIND);
    assert_eq!(ir.until.kind.as_str(), ABSENCE_UNTIL_KIND);
    assert_eq!(ir.veto.kind.as_str(), ABSENCE_VETO_KIND);
    assert_eq!(
        ir.required_sources.len(),
        3,
        "three roles bind distinct sources, so all three are required"
    );
}

/// Missing CAT-2 parameters refuse instantiation.
#[test]
fn cat2_refuses_a_missing_until_parameter() {
    let mut arguments = absence_arguments();
    arguments.remove("until");
    let err = instantiate(
        PatternId::AbsenceAfterUntil,
        identity(),
        contract(),
        arguments,
        absence_evidence(
            SourceTier::Contributing,
            SourceTier::Anchor,
            SourceTier::Anchor,
        ),
    )
    .expect_err("until is required");
    assert_eq!(
        err,
        InstanceError::MissingParameter {
            pattern: PatternId::AbsenceAfterUntil,
            parameter: "until",
        }
    );
}

/// The compiled Filter admits all three CAT-2 selectors.
#[test]
fn cat2_filter_admits_after_until_and_veto() {
    let built = conforming(PatternId::AbsenceAfterUntil);
    let compiled = compile_filter(&[&built], FilterEpoch::new(7)).expect("instance compiles");

    let emitted = &compiled.specifications()[0];
    let event_match = emitted.event_match.as_ref().expect("event_match present");
    let kinds: BTreeSet<&str> = event_match.event_kinds.iter().map(String::as_str).collect();
    let keys: BTreeSet<&str> = event_match
        .projected_attribute_keys
        .iter()
        .map(String::as_str)
        .collect();

    assert!(kinds.contains(ABSENCE_AFTER_KIND), "{kinds:?}");
    assert!(kinds.contains(ABSENCE_UNTIL_KIND), "{kinds:?}");
    assert!(
        kinds.contains(ABSENCE_VETO_KIND),
        "the Filter dropped the selector a Violate rests on: {kinds:?}"
    );
    assert!(
        keys.contains(BINDING),
        "binding attribute dropped; binding_of() would find nothing: {keys:?}"
    );
}

/// Shape-blind stamp: CAT-2 gets one activation Watermark per required source.
#[test]
fn activation_stamp_covers_every_cat2_required_source() {
    assert_one_watermark_per_required_source(&conforming(PatternId::AbsenceAfterUntil));
}

// ---------------------------------------------------------------------------
// CAT-5 aggregation.count-within (issue #173)
// ---------------------------------------------------------------------------

/// Reads the aggregate IR a CAT-5 instance lowered to.
fn aggregate_ir(instance: &PatternInstance) -> &sentinel_compile::ir::AggregateIr {
    instance
        .ir()
        .as_aggregate()
        .expect("CAT-5 lowers to the aggregate monitor")
}

fn aggregation_trigger_event(sequence: u64, request: &str) -> Event {
    event(
        sequence,
        AGGREGATION_TRIGGER_KIND,
        &[(BINDING, text(request))],
    )
}

fn aggregation_member_event(sequence: u64, request: &str) -> Event {
    event(
        sequence,
        AGGREGATION_MEMBER_KIND,
        &[(BINDING, text(request))],
    )
}

/// Evidence from a deployment that assessed once with no counted members.
fn aggregation_clean_evidence() -> Vec<Event> {
    vec![aggregation_trigger_event(1, "r-1")]
}

/// Evidence full of members and no trigger — the inverse population.
fn aggregation_member_only_evidence() -> Vec<Event> {
    vec![
        aggregation_member_event(1, "r-2"),
        aggregation_member_event(2, "r-3"),
    ]
}

/// CAT-5 lowers to the aggregate monitor IR, carrying every bound value.
#[test]
fn cat5_lowers_to_aggregate_ir() {
    let built = conforming(PatternId::AggregationCountWithin);
    let ir = aggregate_ir(&built);

    assert_eq!(ir.specification_id, identity().specification_id);
    assert_eq!(ir.specification_version, identity().specification_version);
    assert_eq!(ir.binding_attribute, BINDING);
    assert_eq!(ir.trigger.kind.as_str(), AGGREGATION_TRIGGER_KIND);
    assert_eq!(ir.member.kind.as_str(), AGGREGATION_MEMBER_KIND);
    assert_eq!(ir.lookback_sequences, 16);
    assert_eq!(ir.threshold, 3);
    assert_eq!(
        ir.required_sources.len(),
        2,
        "two roles bind distinct sources, so both are required"
    );
}

/// Missing CAT-5 parameters refuse instantiation.
#[test]
fn cat5_refuses_a_missing_lookback_parameter() {
    let mut arguments = aggregation_arguments();
    arguments.remove("lookback");
    let err = instantiate(
        PatternId::AggregationCountWithin,
        identity(),
        contract(),
        arguments,
        aggregation_evidence(SourceTier::Anchor, SourceTier::Anchor),
    )
    .expect_err("lookback is required");
    assert_eq!(
        err,
        InstanceError::MissingParameter {
            pattern: PatternId::AggregationCountWithin,
            parameter: "lookback",
        }
    );
}

/// A lookback/threshold kind mismatch is refused.
#[test]
fn cat5_refuses_a_wall_clock_shaped_lookback_value() {
    let mut arguments = aggregation_arguments();
    // SequenceLookbackBound is u64 sequence positions — binding a Selector here
    // is the kind of silent Duration→sequence mistake AMEND 2 forbids.
    arguments.insert(
        "lookback".to_owned(),
        selector("example.duration.hours", &[]),
    );
    let err = instantiate(
        PatternId::AggregationCountWithin,
        identity(),
        contract(),
        arguments,
        aggregation_evidence(SourceTier::Anchor, SourceTier::Anchor),
    )
    .expect_err("lookback must be sequence-lookback-bound");
    assert_eq!(
        err,
        InstanceError::ParameterKindMismatch {
            pattern: PatternId::AggregationCountWithin,
            parameter: "lookback",
            expected: ParameterKind::SequenceLookbackBound,
            actual: ParameterKind::Selector,
        }
    );
}

/// The compiled Filter admits both CAT-5 selectors.
#[test]
fn cat5_filter_admits_trigger_and_member() {
    let built = conforming(PatternId::AggregationCountWithin);
    let compiled = compile_filter(&[&built], FilterEpoch::new(7)).expect("instance compiles");

    let emitted = &compiled.specifications()[0];
    let event_match = emitted.event_match.as_ref().expect("event_match present");
    let kinds: BTreeSet<&str> = event_match.event_kinds.iter().map(String::as_str).collect();
    let keys: BTreeSet<&str> = event_match
        .projected_attribute_keys
        .iter()
        .map(String::as_str)
        .collect();

    assert!(kinds.contains(AGGREGATION_TRIGGER_KIND), "{kinds:?}");
    assert!(
        kinds.contains(AGGREGATION_MEMBER_KIND),
        "the Filter dropped the selector the aggregate counts: {kinds:?}"
    );
    assert!(
        keys.contains(BINDING),
        "binding attribute dropped; binding_of() would find nothing: {keys:?}"
    );
}

/// **CAT-5 is censused on its trigger selector and never on its member.**
///
/// The aggregation-shaped form of the trap `compile/src/catalogue/exercise.rs`
/// names. Two evidence sets, each the other's inverse:
///
/// - a **clean** under-threshold assessment (`trigger`) with no members raised
///   an obligation and measured something.
/// - a deployment full of members and no `trigger` raised **no** obligation,
///   because nothing assessed the cap.
///
/// Keying the census on the member swaps both answers.
#[test]
fn cat5_censuses_its_trigger_selector_and_never_its_member() {
    let built = conforming(PatternId::AggregationCountWithin);

    let clean = built.exercise(SettledEvidence::asserted(&aggregation_clean_evidence()));
    assert_eq!(
        clean.trigger,
        TriggerExercise::RaisedObligations {
            matches: nonzero(1),
            obligations: nonzero(1),
        },
        "a trigger occurred, so an obligation was raised and something was measured"
    );
    assert!(clean.trigger.raised_an_obligation());
    assert_eq!(
        clean.counterpart,
        CounterpartMatches::Falsifying(0),
        "no member matched, which is a clean under-threshold assessment"
    );
    assert_eq!(clean.counterpart.matches(), 0);
    assert_eq!(clean.counterpart.as_str(), "falsifying");

    let members_only = built.exercise(SettledEvidence::asserted(
        &aggregation_member_only_evidence(),
    ));
    assert_eq!(
        members_only.trigger,
        TriggerExercise::KindNeverOccurred {
            events_examined: nonzero(2)
        },
        "the trigger kind never occurred, so nothing was measured — censusing the \
         member instead would have called this the exercised case"
    );
    assert!(!members_only.trigger.raised_an_obligation());
    assert_eq!(
        members_only.counterpart,
        CounterpartMatches::Falsifying(2),
        "both members matched, and that is reported rather than judged"
    );
    assert_eq!(members_only.counterpart.matches(), 2);
}

/// Shape-blind stamp: CAT-5 gets one activation Watermark per required source.
#[test]
fn activation_stamp_covers_every_cat5_required_source() {
    assert_one_watermark_per_required_source(&conforming(PatternId::AggregationCountWithin));
}

/// Reads the taint IR a CAT-4 instance lowered to.
fn taint_ir(instance: &PatternInstance) -> &sentinel_compile::ir::TaintIr {
    instance
        .ir()
        .as_taint()
        .expect("CAT-4 lowers to the taint monitor")
}

/// An Event matching the action selector and naming a Binding.
fn action_event(sequence: u64, request: &str) -> Event {
    event(sequence, TAINT_ACTION_KIND, &[(BINDING, text(request))])
}

/// An Event matching the untrusted-content selector and naming a Binding.
fn marker_event(sequence: u64, request: &str) -> Event {
    event(
        sequence,
        TAINT_MARKER_KIND,
        &[
            (BINDING, text(request)),
            ("example.content.trust", text("untrusted")),
        ],
    )
}

/// Evidence from a deployment that did the constrained thing and read nothing
/// untrusted: one action, no marker.
fn taint_clean_evidence() -> Vec<Event> {
    vec![action_event(1, "r-1")]
}

/// Evidence from a deployment that read untrusted content and never acted.
///
/// The inverse population, and the one a census keyed on the wrong selector
/// would read as the exercised case.
fn taint_marked_but_actionless_evidence() -> Vec<Event> {
    vec![marker_event(1, "r-2"), marker_event(2, "r-3")]
}

/// CAT-4 lowers to the taint monitor IR, carrying every bound value.
#[test]
fn cat4_lowers_to_taint_ir() {
    let built = conforming(PatternId::ProvenanceTaint);
    let ir = taint_ir(&built);

    assert_eq!(ir.specification_id, identity().specification_id);
    assert_eq!(ir.specification_version, identity().specification_version);
    assert_eq!(ir.binding_attribute, BINDING);
    assert_eq!(ir.action.kind.as_str(), TAINT_ACTION_KIND);
    assert!(ir.action.required_attributes.is_empty());
    assert_eq!(ir.untrusted_content.kind.as_str(), TAINT_MARKER_KIND);
    assert_eq!(
        ir.untrusted_content
            .required_attributes
            .get(&attr_key("example.content.trust")),
        Some(&AttributeValue::String("untrusted".to_owned()))
    );
    assert_eq!(
        ir.required_sources.len(),
        2,
        "both roles bind distinct sources, so both are required"
    );
}

/// **CAT-4 is censused on its action selector and never on its taint marker.**
///
/// The trap `compile/src/catalogue/exercise.rs` names, in the concrete form
/// CAT-4 puts it in. Two evidence sets, each the other's inverse:
///
/// - a **clean** deployment acts and reads nothing untrusted. It raised an
///   obligation and measured something, and must census as exercised.
/// - a deployment that read untrusted content and never acted raised **no**
///   obligation, because there is no action for the property to be about. It
///   must census as not exercised, and its two marker matches must not be
///   mistaken for the thing that was measured.
///
/// Keying the census on the marker swaps both answers, and the swapped version
/// is not obviously wrong to read — it would report a spotless deployment as the
/// defect #144 exists to catch, every time, forever.
#[test]
fn cat4_censuses_its_action_selector_and_never_its_taint_marker() {
    let built = conforming(PatternId::ProvenanceTaint);

    let clean = built.exercise(SettledEvidence::asserted(&taint_clean_evidence()));
    assert_eq!(
        clean.trigger,
        TriggerExercise::RaisedObligations {
            matches: nonzero(1),
            obligations: nonzero(1),
        },
        "an action occurred, so an obligation was raised and something was measured"
    );
    assert!(clean.trigger.raised_an_obligation());
    assert_eq!(
        clean.counterpart,
        CounterpartMatches::Falsifying(0),
        "no untrusted content was read, which is what a clean deployment looks like"
    );

    let actionless = built.exercise(SettledEvidence::asserted(
        &taint_marked_but_actionless_evidence(),
    ));
    assert_eq!(
        actionless.trigger,
        TriggerExercise::KindNeverOccurred {
            events_examined: nonzero(2)
        },
        "the action kind never occurred, so nothing was measured — censusing the marker \
         instead would have called this the exercised case"
    );
    assert!(!actionless.trigger.raised_an_obligation());
    assert_eq!(
        actionless.counterpart,
        CounterpartMatches::Falsifying(2),
        "both markers matched, and that is reported rather than judged"
    );
}

/// **A corpus of clean Traces exercises CAT-4 rather than condemning it.**
///
/// The consequence of the census above, at the level `CorpusExercise` judges on.
/// If the marker were the obligation-raising selector, a validation corpus drawn
/// from a deployment that has never read untrusted content — the ordinary case,
/// and the one a design partner would hand over first — would fold to
/// `NeverExercised`, which is #144's defect verdict. CAT-4 would then declare
/// itself broken precisely when it is working.
#[test]
fn cat4_is_exercised_by_a_corpus_of_clean_traces() {
    let built = conforming(PatternId::ProvenanceTaint);
    let corpus = [
        taint_clean_evidence(),
        vec![action_event(1, "r-4"), action_event(2, "r-5")],
    ];

    let folded = CorpusExercise::of(
        corpus
            .iter()
            .map(|events| built.exercise(SettledEvidence::asserted(events)).trigger),
    );
    assert_eq!(
        folded,
        CorpusExercise::Exercised {
            traces: nonzero(2),
            exercised_in: nonzero(2),
        }
    );
    assert!(folded.was_exercised());

    let never = CorpusExercise::of(
        [taint_marked_but_actionless_evidence()]
            .iter()
            .map(|events| built.exercise(SettledEvidence::asserted(events)).trigger),
    );
    assert_eq!(
        never,
        CorpusExercise::NeverExercised { traces: nonzero(1) },
        "a corpus in which the constrained action never happens has exercised nothing, \
         however much untrusted content it contains"
    );
}

/// **The compiled Filter admits the untrusted-content selector.**
///
/// The direction in which `compile/src/filter.rs`'s trust boundary inverts an
/// answer rather than losing one. A Filter that dropped the marker's Event kind
/// would leave the monitor unable to see the very Event a Violate rests on, and
/// every Verdict would settle clean — a Specification that cannot fail. The
/// Binding attribute is asserted for the reason `binding_attribute_keys`'s
/// comment gives: the action selector requires no attributes, so nothing else
/// would carry it into the projection.
#[test]
fn cat4_filter_admits_the_untrusted_content_selector() {
    let built = conforming(PatternId::ProvenanceTaint);
    let compiled = compile_filter(&[&built], FilterEpoch::new(7)).expect("instance compiles");

    let emitted = &compiled.specifications()[0];
    let event_match = emitted.event_match.as_ref().expect("event_match present");
    let kinds: BTreeSet<&str> = event_match.event_kinds.iter().map(String::as_str).collect();
    let keys: BTreeSet<&str> = event_match
        .projected_attribute_keys
        .iter()
        .map(String::as_str)
        .collect();

    assert!(
        kinds.contains(TAINT_MARKER_KIND),
        "the Filter dropped the selector a Violate rests on: {kinds:?}"
    );
    assert!(kinds.contains(TAINT_ACTION_KIND), "{kinds:?}");
    assert!(
        keys.contains(BINDING),
        "binding attribute dropped; binding_of() would find nothing: {keys:?}"
    );
    assert!(keys.contains("example.content.trust"), "{keys:?}");
}

/// **A contributing-tier content source is reported, not swallowed.**
///
/// ADR-0022 makes placement a deployment fact, so this is a warning rather than
/// a refusal — the same split CAT-1's `tier_shortfall_is_reported_not_swallowed`
/// asserts. What differs is what the shortfall costs: for CAT-1 it costs an
/// authoritative Violate on an *absence*, and for CAT-4 it costs one on a
/// *presence* the evidence already contains. `core/src/taint.rs` is where that
/// difference is kept out of the Satisfy row.
#[test]
fn cat4_reports_a_contributing_tier_content_source() {
    let built = instantiate(
        PatternId::ProvenanceTaint,
        identity(),
        contract(),
        taint_arguments(),
        taint_evidence(SourceTier::Anchor, SourceTier::Contributing),
    )
    .expect("a below-tier deployment still instantiates");

    assert_eq!(
        built.validate(),
        vec![InstanceWarning::TierShortfall {
            role: "content_source",
            source: source("example-content-broker"),
            required: SourceTier::Anchor,
            placement: SourceTier::Contributing,
        }]
    );

    assert!(
        conforming(PatternId::ProvenanceTaint).validate().is_empty(),
        "the control: a conforming deployment warns about nothing, so the assertion above \
         is about the tier and not about CAT-4 in general"
    );
}

/// Shape-blind stamp: CAT-1 gets one activation Watermark per required source.
#[test]
fn activation_stamp_covers_every_cat1_required_source() {
    assert_one_watermark_per_required_source(&instance());
}

/// Shape-blind stamp: CAT-4 gets one activation Watermark per required source.
///
/// A match on `PrecedenceIr` would silently emit no barrier here.
#[test]
fn activation_stamp_covers_every_cat4_required_source() {
    assert_one_watermark_per_required_source(&conforming(PatternId::ProvenanceTaint));
}

/// Stamped CAT-1 and CAT-4 entries in one envelope read back as barriers.
///
/// `compile_one` itself still emits empty watermarks; the stamp is post-compile
/// so `observed_at` stays a publication instant.
#[test]
fn stamped_cat1_and_cat4_read_back_as_activation_barriers() {
    let cat1 = instance();
    let cat4 = cat4_instance();
    let compiled = compile_filter(&[&cat1, &cat4], FilterEpoch::new(14))
        .expect("both catalogue shapes compile together");
    let cat1_stamp = publication_stamp(&cat1);
    let cat4_stamp = publication_stamp(&cat4);
    let mut specifications = compiled.specifications().to_vec();
    for specification in &mut specifications {
        if specification.specification_id == cat1.filter_identity().specification_id {
            specification.activation_watermarks =
                activation_watermarks_for(&cat1, &cat1_stamp).expect("CAT-1 stamp");
        } else {
            specification.activation_watermarks =
                activation_watermarks_for(&cat4, &cat4_stamp).expect("CAT-4 stamp");
        }
    }
    let filter = EventFilter::try_from(raw_model::EventFilter {
        epoch: Some(14),
        specifications,
        baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }),
    })
    .expect("stamped Warming envelope validates");

    let cat1_identity = cat1.filter_identity();
    let cat4_identity = cat4.filter_identity();
    let cat1_barrier = ActivationBarrier::try_from_event_filter(
        &filter,
        &cat1_identity.specification_id,
        &cat1_identity.specification_version,
    )
    .expect("CAT-1 barrier reads back");
    let cat4_barrier = ActivationBarrier::try_from_event_filter(
        &filter,
        &cat4_identity.specification_id,
        &cat4_identity.specification_version,
    )
    .expect("CAT-4 barrier reads back");

    assert_eq!(cat1_barrier.target_filter_epoch(), FilterEpoch::new(14));
    assert_eq!(cat4_barrier.target_filter_epoch(), FilterEpoch::new(14));
    assert_eq!(
        cat1_barrier.watermarks().len(),
        cat1.required_sources().len()
    );
    assert_eq!(
        cat4_barrier.watermarks().len(),
        cat4.required_sources().len()
    );
    for identity in cat1.required_sources() {
        assert!(
            cat1_barrier.watermarks().contains_key(&identity),
            "CAT-1 barrier dropped {identity:?}"
        );
    }
    for identity in cat4.required_sources() {
        assert!(
            cat4_barrier.watermarks().contains_key(&identity),
            "CAT-4 barrier dropped {identity:?}"
        );
    }
}

/// A missing SequenceEpoch is a typed CompileError, not a silent empty barrier.
#[test]
fn activation_stamp_missing_sequence_epoch_is_a_typed_error() {
    let built = instance();
    let mut stamp = publication_stamp(&built);
    let omitted = built
        .required_sources()
        .into_iter()
        .next()
        .expect("CAT-1 names required sources");
    stamp.sequence_epochs.remove(&omitted);
    assert_eq!(
        activation_watermarks_for(&built, &stamp),
        Err(CompileError::MissingSequenceEpoch(omitted))
    );
}

/// [`SelectorValue::matches`] and [`SelectorIr::matches`] share one helper.
///
/// Candidate adversarial mutation calls the catalogue type. The Core and the
/// census call the IR type. A second copy would silently diverge the day
/// selectors gain negation or ranges (ADR-0027, issue #112).
#[test]
fn selector_value_matches_agrees_with_selector_ir() {
    let kind = EventKind::new("example.approval.granted").expect("constant");
    let mut required = BTreeMap::new();
    required.insert(attr_key("example.approval.outcome"), text("allow"));
    let value = SelectorValue::new(kind.clone(), required.clone());
    let ir = SelectorIr::new(kind, required);

    let matching = approving_event(1, "req-1");
    let wrong_valuation = event(
        2,
        "example.approval.granted",
        &[
            (BINDING, text("req-1")),
            ("example.approval.outcome", text("deny")),
        ],
    );
    let wrong_kind = triggering_event(3, "req-1");

    for sample in [&matching, &wrong_valuation, &wrong_kind] {
        assert_eq!(
            value.matches(sample),
            ir.matches(sample),
            "SelectorValue and SelectorIr disagreed on {sample:?}"
        );
        assert_eq!(
            value.matches_producer(&sample.producer()),
            ir.matches_producer(&sample.producer()),
        );
    }
    assert!(value.matches(&matching));
    assert!(!value.matches(&wrong_valuation));
    assert!(!value.matches(&wrong_kind));
}

// ---------------------------------------------------------------------------
// CAT-3 existence.before-scope-close (issue #210)
// ---------------------------------------------------------------------------

fn existence_ir(instance: &PatternInstance) -> &sentinel_compile::ir::ExistenceIr {
    instance
        .ir()
        .as_existence()
        .expect("CAT-3 lowers to the existence monitor")
}

#[test]
fn cat3_lowers_to_existence_ir() {
    let built = conforming(PatternId::ExistenceBeforeScopeClose);
    let ir = existence_ir(&built);
    assert_eq!(ir.specification_id, identity().specification_id);
    assert_eq!(ir.open.kind.as_str(), EXISTENCE_OPEN_KIND);
    assert_eq!(ir.required.kind.as_str(), EXISTENCE_REQUIRED_KIND);
    assert_eq!(ir.close.kind.as_str(), EXISTENCE_CLOSE_KIND);
    assert_eq!(ir.retirement, sentinel_compile::ir::RetirementIr::Deadline);
}

#[test]
fn cat3_censuses_its_open_selector_and_never_its_required() {
    let built = conforming(PatternId::ExistenceBeforeScopeClose);
    let open_only = vec![event(1, EXISTENCE_OPEN_KIND, &[(BINDING, text("s-1"))])];
    let required_only = vec![event(1, EXISTENCE_REQUIRED_KIND, &[(BINDING, text("s-1"))])];

    let open_ex = built.exercise(SettledEvidence::asserted(&open_only));
    assert_eq!(
        open_ex.trigger,
        TriggerExercise::RaisedObligations {
            matches: nonzero(1),
            obligations: nonzero(1),
        },
        "an open occurred, so an obligation was raised"
    );
    assert!(open_ex.trigger.raised_an_obligation());
    assert_eq!(
        open_ex.counterpart,
        CounterpartMatches::Discharging(0),
        "no required matched yet; discharging count stays zero"
    );

    let req_ex = built.exercise(SettledEvidence::asserted(&required_only));
    assert_eq!(
        req_ex.trigger,
        TriggerExercise::KindNeverOccurred {
            events_examined: nonzero(1)
        },
        "required-only must not census as exercised — that would vacuous-pass a Trace \
         full of compensations with no saga open"
    );
    assert!(!req_ex.trigger.raised_an_obligation());
    assert_eq!(
        req_ex.counterpart,
        CounterpartMatches::Discharging(1),
        "the required match is reported as discharging counterpart, not as the trigger"
    );
}

#[test]
fn cat3_filter_admits_open_required_and_close() {
    let built = conforming(PatternId::ExistenceBeforeScopeClose);
    let compiled = compile_filter(&[&built], FilterEpoch::new(7)).expect("instance compiles");

    let emitted = &compiled.specifications()[0];
    let event_match = emitted.event_match.as_ref().expect("event_match present");
    let kinds: BTreeSet<&str> = event_match.event_kinds.iter().map(String::as_str).collect();

    assert!(
        kinds.contains(EXISTENCE_OPEN_KIND),
        "Filter dropped the open selector: {kinds:?}"
    );
    assert!(
        kinds.contains(EXISTENCE_REQUIRED_KIND),
        "Filter dropped the required selector: {kinds:?}"
    );
    assert!(
        kinds.contains(EXISTENCE_CLOSE_KIND),
        "Filter dropped the close selector: {kinds:?}"
    );
}
