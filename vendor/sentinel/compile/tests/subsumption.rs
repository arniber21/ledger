//! Same-family Specification subsumption over catalogue instantiations (issue
//! #177): which pairs are proven, which are left undecided, and which are not
//! comparable at all.
//!
//! Every test here checks the analysis against its own restatement of two Core
//! decision tables, so none of them can notice the tables drifting.
//! `core/tests/subsumption_cross_check.rs` is the suite that executes the
//! proven pairs through the Verification Core; read the two together.
//!
//! Two tests are the anti-regression pins the design is arranged around, and
//! both would pass under the most plausible wrong implementation ("the
//! Candidate is more specific everywhere") if they were not here:
//! `cat1_a_candidate_that_narrows_the_requirement_is_undecided` and
//! `cat4_a_candidate_that_widens_the_untrusted_content_selector_is_undecided`.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    catalogue, instantiate, EvidenceBinding, ParameterKind, ParameterValue, PatternId,
    PatternInstance, ProducerContract,
};
use sentinel_compile::{
    subsumption, CandidateUnderReview, DeployedSpecification, EvidenceInclusion, FilterIdentity,
    InclusionDirection, NotComparable, SelectorDivergence, Subsumption, Undecided,
};
use sentinel_model::{AttributeValue, EventKind, SchemaVersion, SourceTier};

use support::{
    absence_arguments, absence_evidence, aggregation_arguments, aggregation_evidence, attr_key,
    capable, conforming, contract, existence_arguments, existence_evidence, identity, selector,
    source, taint_arguments, ABSENCE_AFTER_KIND, BINDING, EXISTENCE_CLOSE_KIND, TAINT_ACTION_KIND,
    TAINT_MARKER_KIND,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const TRIGGER_KIND: &str = "example.action.executed";
const REQUIREMENT_KIND: &str = "example.approval.granted";
const GATEWAY: &str = "example-gateway";
const AUTHZ: &str = "example-authz";
const MAIL: &str = "example-mail-gateway";
const BROKER: &str = "example-content-broker";

fn ask(candidate: &PatternInstance, deployed: &PatternInstance) -> Subsumption {
    subsumption(
        CandidateUnderReview::new(candidate),
        DeployedSpecification::new(deployed),
    )
}

fn named(specification_id: &str) -> FilterIdentity {
    FilterIdentity {
        specification_id: specification_id.to_owned(),
        specification_version: "v1".to_owned(),
    }
}

fn text(value: &str) -> AttributeValue {
    AttributeValue::String(value.to_owned())
}

/// A CAT-1 instance varying both selectors and where each evidence role is bound.
fn cat1_with(
    specification_id: &str,
    trigger: &[(&str, &str)],
    requirement: &[(&str, &str)],
    trigger_source: &str,
    requirement_source: &str,
) -> PatternInstance {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        ("trigger".to_owned(), selector(TRIGGER_KIND, trigger)),
        (
            "requirement".to_owned(),
            selector(REQUIREMENT_KIND, requirement),
        ),
    ]);
    let evidence = BTreeMap::from([
        (
            "trigger_source".to_owned(),
            EvidenceBinding::new(source(trigger_source), SourceTier::Contributing, capable()),
        ),
        (
            "requirement_source".to_owned(),
            EvidenceBinding::new(source(requirement_source), SourceTier::Anchor, capable()),
        ),
    ]);
    instantiate(
        PatternId::PrecedenceApproval,
        named(specification_id),
        contract(),
        arguments,
        evidence,
    )
    .expect("a fully bound CAT-1 instantiation succeeds")
}

/// A CAT-1 instance with the canonical role bindings.
fn cat1(
    specification_id: &str,
    trigger: &[(&str, &str)],
    requirement: &[(&str, &str)],
) -> PatternInstance {
    cat1_with(specification_id, trigger, requirement, GATEWAY, AUTHZ)
}

/// The canonical deployed CAT-1: any action, approvals with `outcome = allow`.
fn deployed_cat1() -> PatternInstance {
    cat1(
        "example.deployed",
        &[],
        &[("example.approval.outcome", "allow")],
    )
}

/// A CAT-4 instance varying both selectors and where each evidence role is bound.
fn cat4_with(
    specification_id: &str,
    action: &[(&str, &str)],
    untrusted_content: &[(&str, &str)],
    action_source: &str,
    content_source: &str,
) -> PatternInstance {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        ("action".to_owned(), selector(TAINT_ACTION_KIND, action)),
        (
            "untrusted_content".to_owned(),
            selector(TAINT_MARKER_KIND, untrusted_content),
        ),
    ]);
    let evidence = BTreeMap::from([
        (
            "action_source".to_owned(),
            EvidenceBinding::new(source(action_source), SourceTier::Anchor, capable()),
        ),
        (
            "content_source".to_owned(),
            EvidenceBinding::new(source(content_source), SourceTier::Anchor, capable()),
        ),
    ]);
    instantiate(
        PatternId::ProvenanceTaint,
        named(specification_id),
        contract(),
        arguments,
        evidence,
    )
    .expect("a fully bound CAT-4 instantiation succeeds")
}

fn cat4(
    specification_id: &str,
    action: &[(&str, &str)],
    untrusted_content: &[(&str, &str)],
) -> PatternInstance {
    cat4_with(specification_id, action, untrusted_content, MAIL, BROKER)
}

/// The canonical deployed CAT-4: any action, markers with `trust = untrusted`.
fn deployed_cat4() -> PatternInstance {
    cat4(
        "example.deployed-taint",
        &[],
        &[("example.content.trust", "untrusted")],
    )
}

/// Instantiates one Pattern from given arguments and evidence under a named identity.
fn instance_of(
    pattern: PatternId,
    specification_id: &str,
    arguments: BTreeMap<String, ParameterValue>,
    evidence: BTreeMap<String, EvidenceBinding>,
) -> PatternInstance {
    instantiate(
        pattern,
        named(specification_id),
        contract(),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds")
}

/// Unwraps a proof or fails naming what came back instead.
fn proof(answer: Subsumption) -> sentinel_compile::SubsumptionProof {
    match answer {
        Subsumption::Subsumed(proof) => proof,
        other => panic!("expected a proof, got {other}"),
    }
}

/// Unwraps an undecided answer or fails naming what came back instead.
fn undecided(answer: Subsumption) -> Undecided {
    match answer {
        Subsumption::Undecided(undecided) => undecided,
        other => panic!("expected Undecided, got {other}"),
    }
}

/// Unwraps a not-comparable answer or fails naming what came back instead.
fn not_comparable(answer: Subsumption) -> NotComparable {
    match answer {
        Subsumption::NotComparable(reason) => reason,
        other => panic!("expected NotComparable, got {other}"),
    }
}

// ---------------------------------------------------------------------------
// Reflexivity, identity, and the forcing function
// ---------------------------------------------------------------------------

/// Every catalogued Pattern, compared with itself, gets exactly the answer its
/// family argument licenses — by name.
///
/// The enumerable twin of the compile error in `subsumption`: [`PatternId`] is
/// `#[non_exhaustive]` outside `sentinel-compile`, so this loop needs a
/// wildcard, and the wildcard panics. A sixth Pattern fails here as well as
/// failing to compile there.
#[test]
fn every_catalogued_pattern_subsumes_itself_or_says_why_not() {
    for declared in catalogue() {
        let instance = conforming(declared.id);
        let answer = ask(&instance, &instance);
        match declared.id {
            PatternId::PrecedenceApproval | PatternId::ProvenanceTaint => {
                assert!(
                    answer.is_proven(),
                    "{} compared with itself: {answer}",
                    declared.id
                );
            }
            PatternId::AbsenceAfterUntil
            | PatternId::AggregationCountWithin
            | PatternId::ExistenceBeforeScopeClose => assert_eq!(
                answer,
                Subsumption::Undecided(Undecided::PolarityNotDerived {
                    pattern: declared.id
                }),
                "{} derives no inclusion argument in this slice",
                declared.id
            ),
            other => panic!("{other} is catalogued with no expected subsumption answer"),
        }
    }
}

/// Specification identity is not semantics: the plainest subsumed pair.
#[test]
fn an_identical_specification_under_a_different_identity_is_still_subsumed() {
    let deployed = deployed_cat1();
    let candidate = cat1(
        "example.candidate-copy",
        &[],
        &[("example.approval.outcome", "allow")],
    );
    assert_ne!(candidate.identity(), deployed.identity());

    let proof = proof(ask(&candidate, &deployed));
    assert_eq!(proof.pattern, PatternId::PrecedenceApproval);
    assert_eq!(
        &proof.deployed,
        deployed.identity(),
        "the proof names the deployed Specification, not the Candidate"
    );
}

/// The two newtypes exist because transposing the arguments inverts the question.
///
/// On a strictly narrowing CAT-1 pair the two orders must give different
/// answers; if they agreed, nothing would defend the newtypes' reason to exist.
#[test]
fn transposing_the_candidate_and_the_deployed_specification_changes_the_answer() {
    let narrow = cat1(
        "example.narrow",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
    );
    let wide = deployed_cat1();

    assert!(ask(&narrow, &wide).is_proven());
    assert!(
        !ask(&wide, &narrow).is_proven(),
        "the wider trigger raises obligations the narrower Specification never raises"
    );
}

/// A proof names every selector role the Pattern declares, in declared order.
///
/// Guards a future three-selector argument that silently checks two.
#[test]
fn a_proof_names_every_selector_role_the_pattern_declares() {
    for pattern in [PatternId::PrecedenceApproval, PatternId::ProvenanceTaint] {
        let instance = conforming(pattern);
        let proof = proof(ask(&instance, &instance));
        let declared: Vec<&str> = instance
            .declaration()
            .parameters
            .iter()
            .filter(|parameter| parameter.kind == ParameterKind::Selector)
            .map(|parameter| parameter.name)
            .collect();
        let proven: Vec<&str> = proof
            .inclusions
            .iter()
            .map(|inclusion| inclusion.parameter)
            .collect();
        assert_eq!(proven, declared, "{pattern}");
    }
}

/// `is_proven` is the only boolean, and it is true for a proof alone.
#[test]
fn is_proven_is_true_only_for_a_proof() {
    let deployed = deployed_cat1();
    let narrowed_requirement = cat1(
        "example.c",
        &[],
        &[
            ("example.approval.outcome", "allow"),
            ("example.approval.scope", "prod"),
        ],
    );
    let taint = conforming(PatternId::ProvenanceTaint);

    assert!(matches!(
        ask(&deployed, &deployed),
        Subsumption::Subsumed(_)
    ));
    assert!(ask(&deployed, &deployed).is_proven());
    assert!(matches!(
        ask(&narrowed_requirement, &deployed),
        Subsumption::Undecided(_)
    ));
    assert!(!ask(&narrowed_requirement, &deployed).is_proven());
    assert!(matches!(
        ask(&taint, &deployed),
        Subsumption::NotComparable(_)
    ));
    assert!(!ask(&taint, &deployed).is_proven());
}

// ---------------------------------------------------------------------------
// CAT-1 polarity: trigger covariant, requirement contravariant
// ---------------------------------------------------------------------------

#[test]
fn cat1_a_candidate_that_narrows_the_trigger_is_subsumed() {
    let candidate = cat1(
        "example.c",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
    );
    let proof = proof(ask(&candidate, &deployed_cat1()));
    assert_eq!(
        proof.inclusions[0].direction,
        InclusionDirection::CandidateNarrower
    );
    assert_eq!(
        proof.inclusions[1].direction,
        InclusionDirection::DeployedNarrower
    );
    assert_eq!(proof.evidence, EvidenceInclusion::Equal);
}

/// **The canonical must-stay-undecided case.** Do not "fix" it into a
/// refutation.
///
/// Structurally, the Candidate raises obligations the deployed Specification
/// does not. That is not a proof the Candidate adds a Violate: in a deployment
/// that only ever emits `region = "eu"`, the two raise exactly the same
/// obligations and the Candidate *is* subsumed. Which deployment this is, is a
/// question no analysis may answer from a declared Topology, which ADR-0004
/// makes just another Specification, never an input; and the
/// empty set is contained in everything.
#[test]
fn cat1_a_candidate_that_widens_the_trigger_is_undecided() {
    let deployed = cat1(
        "example.deployed",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
    );
    let candidate = cat1("example.c", &[], &[("example.approval.outcome", "allow")]);
    assert_eq!(
        undecided(ask(&candidate, &deployed)),
        Undecided::SelectorNotIncluded {
            parameter: "trigger",
            direction: InclusionDirection::CandidateNarrower,
            divergence: SelectorDivergence::RequirementNotCarried {
                key: attr_key("example.region"),
                demanded: text("eu"),
            },
        }
    );
}

/// The contravariant direction, proven: a wider discharging selector violates
/// *less*, never more.
#[test]
fn cat1_a_candidate_that_widens_the_requirement_is_subsumed() {
    let candidate = cat1("example.c", &[], &[]);
    assert!(ask(&candidate, &deployed_cat1()).is_proven());
}

/// **Anti-regression for the contravariance bug.**
///
/// A "Candidate more specific everywhere" implementation returns `Subsumed`
/// here, and is wrong: narrowing the discharging selector means fewer Events
/// discharge the Candidate's obligation, so it violates where the deployed
/// Specification is satisfied. `core/tests/subsumption_cross_check.rs` holds a
/// recorded Trace on which exactly that happens.
#[test]
fn cat1_a_candidate_that_narrows_the_requirement_is_undecided() {
    let candidate = cat1(
        "example.c",
        &[],
        &[
            ("example.approval.outcome", "allow"),
            ("example.approval.scope", "prod"),
        ],
    );
    assert_eq!(
        undecided(ask(&candidate, &deployed_cat1())),
        Undecided::SelectorNotIncluded {
            parameter: "requirement",
            direction: InclusionDirection::DeployedNarrower,
            divergence: SelectorDivergence::RequirementNotCarried {
                key: attr_key("example.approval.scope"),
                demanded: text("prod"),
            },
        }
    );
}

/// Copy a Specification and tighten everything: the realistic authoring
/// mistake, and exactly what "more specific everywhere" would call subsumed.
#[test]
fn cat1_a_candidate_narrower_on_both_roles_is_undecided() {
    let candidate = cat1(
        "example.c",
        &[("example.region", "eu")],
        &[
            ("example.approval.outcome", "allow"),
            ("example.approval.scope", "prod"),
        ],
    );
    let answer = undecided(ask(&candidate, &deployed_cat1()));
    assert!(
        matches!(
            answer,
            Undecided::SelectorNotIncluded {
                parameter: "requirement",
                direction: InclusionDirection::DeployedNarrower,
                ..
            }
        ),
        "{answer}"
    );
}

#[test]
fn cat1_a_candidate_narrower_on_the_trigger_and_wider_on_the_requirement_is_subsumed() {
    let candidate = cat1("example.c", &[("example.region", "eu")], &[]);
    assert!(ask(&candidate, &deployed_cat1()).is_proven());
}

/// Two selectors that can never match one Event are undecided, not refused.
///
/// `{region: "eu"}` against `{region: "us"}` shares no Event, and that is still
/// no evidence the Candidate adds a Violate.
#[test]
fn cat1_selectors_conflicting_on_one_attribute_value_are_undecided_not_refused() {
    let deployed = cat1(
        "example.deployed",
        &[("example.region", "us")],
        &[("example.approval.outcome", "allow")],
    );
    let candidate = cat1(
        "example.c",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
    );
    assert_eq!(
        undecided(ask(&candidate, &deployed)),
        Undecided::SelectorNotIncluded {
            parameter: "trigger",
            direction: InclusionDirection::CandidateNarrower,
            divergence: SelectorDivergence::RequirementsConflict {
                key: attr_key("example.region"),
                subject: text("eu"),
                target: text("us"),
            },
        }
    );
}

#[test]
fn cat1_trigger_kinds_that_differ_are_undecided() {
    let mut arguments = support::arguments();
    arguments.insert(
        "trigger".to_owned(),
        selector("example.action.proposed", &[]),
    );
    let candidate = instance_of(
        PatternId::PrecedenceApproval,
        "example.c",
        arguments,
        support::evidence(SourceTier::Anchor),
    );
    assert_eq!(
        undecided(ask(&candidate, &conforming(PatternId::PrecedenceApproval))),
        Undecided::SelectorNotIncluded {
            parameter: "trigger",
            direction: InclusionDirection::CandidateNarrower,
            divergence: SelectorDivergence::KindsDiffer {
                subject: EventKind::new("example.action.proposed").expect("constant"),
                target: EventKind::new(TRIGGER_KIND).expect("constant"),
            },
        }
    );
}

// ---------------------------------------------------------------------------
// CAT-4 polarity: both selectors covariant
// ---------------------------------------------------------------------------

#[test]
fn cat4_a_candidate_narrower_on_both_selectors_is_subsumed() {
    let candidate = cat4(
        "example.c",
        &[("example.recipient.domain", "external")],
        &[
            ("example.content.trust", "untrusted"),
            ("example.content.origin", "web"),
        ],
    );
    let proof = proof(ask(&candidate, &deployed_cat4()));
    assert!(proof
        .inclusions
        .iter()
        .all(|inclusion| inclusion.direction == InclusionDirection::CandidateNarrower));
    assert_eq!(proof.evidence, EvidenceInclusion::Equal);
}

/// **Anti-regression for copying CAT-1's contravariance onto CAT-4.**
///
/// Under CAT-1's argument a wider second selector reads as subsumed. Here the
/// second selector falsifies rather than discharges: a Candidate that treats
/// more content as untrusted violates on content the deployed Specification
/// does not treat as untrusted at all.
#[test]
fn cat4_a_candidate_that_widens_the_untrusted_content_selector_is_undecided() {
    let candidate = cat4("example.c", &[], &[]);
    assert_eq!(
        undecided(ask(&candidate, &deployed_cat4())),
        Undecided::SelectorNotIncluded {
            parameter: "untrusted_content",
            direction: InclusionDirection::CandidateNarrower,
            divergence: SelectorDivergence::RequirementNotCarried {
                key: attr_key("example.content.trust"),
                demanded: text("untrusted"),
            },
        }
    );
}

#[test]
fn cat4_a_candidate_that_widens_the_action_selector_is_undecided() {
    let deployed = cat4(
        "example.deployed-taint",
        &[("example.recipient.domain", "external")],
        &[("example.content.trust", "untrusted")],
    );
    let candidate = deployed_cat4();
    assert!(matches!(
        undecided(ask(&candidate, &deployed)),
        Undecided::SelectorNotIncluded {
            parameter: "action",
            direction: InclusionDirection::CandidateNarrower,
            ..
        }
    ));
}

// ---------------------------------------------------------------------------
// Evidence conditions: identities only, and equal
// ---------------------------------------------------------------------------

/// Same selectors, one role bound to a different source: CAT-1 cannot decide.
///
/// Not monotone either way. A deployed Specification requiring a source the
/// Candidate does not has an evidence gate that can stay closed — `core/src/
/// evidence.rs`'s `EvidenceGate::evaluate` ranges over the required set — while
/// the Candidate settles Violate. One requiring fewer can lose the anchor-tier
/// source whose watermark-complete silence settles CAT-1's Violate
/// (`has_silent_anchor` in `core/src/precedence.rs`, ADR-0033).
#[test]
fn cat1_evidence_bound_to_a_different_source_is_undecided() {
    let candidate = cat1_with(
        "example.c",
        &[],
        &[("example.approval.outcome", "allow")],
        GATEWAY,
        "example-authz-replica",
    );
    assert_eq!(
        undecided(ask(&candidate, &deployed_cat1())),
        Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            candidate_only: BTreeSet::from([source("example-authz-replica")]),
            deployed_only: BTreeSet::from([source(AUTHZ)]),
        }
    );
}

/// The minimum tier the IR carries per source is unread on every decision
/// path, so a pair differing only there is still proven.
#[test]
fn required_sources_differing_only_in_minimum_tier_does_not_block_a_proof() {
    // The roles swap sources, so the identity sets are equal and each source's
    // declared minimum tier is the other role's.
    let candidate = cat1_with(
        "example.c",
        &[],
        &[("example.approval.outcome", "allow")],
        AUTHZ,
        GATEWAY,
    );
    let deployed = deployed_cat1();
    assert_ne!(
        candidate.ir().required_sources(),
        deployed.ir().required_sources(),
        "the fixture must actually differ in minimum tier"
    );
    assert_eq!(
        proof(ask(&candidate, &deployed)).evidence,
        EvidenceInclusion::Equal
    );
}

/// **Not proven, although CAT-4's evidence gate is monotone in the required
/// set.** The planned `⊆` condition was refuted by execution: the deployed
/// Specification drops the content source, its monitor reaches the Violate, and
/// Verdict assembly refuses the untrusted-content witness from that source. See
/// `a_deployed_specification_requiring_fewer_sources_cannot_emit_the_violate_its_monitor_reaches`
/// in `core/tests/subsumption_cross_check.rs`.
#[test]
fn cat4_a_deployed_specification_requiring_a_subset_of_sources_is_undecided() {
    let deployed = cat4_with(
        "example.deployed-taint",
        &[],
        &[("example.content.trust", "untrusted")],
        MAIL,
        MAIL,
    );
    assert_eq!(deployed.ir().required_sources().len(), 1);
    assert_eq!(
        undecided(ask(&deployed_cat4(), &deployed)),
        Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            candidate_only: BTreeSet::from([source(BROKER)]),
            deployed_only: BTreeSet::new(),
        }
    );
}

/// The converse: a deployed Specification requiring a source the Candidate
/// does not can stay gated where the Candidate violates.
#[test]
fn cat4_a_deployed_specification_requiring_a_superset_of_sources_is_undecided() {
    let candidate = cat4_with(
        "example.c",
        &[],
        &[("example.content.trust", "untrusted")],
        MAIL,
        MAIL,
    );
    assert_eq!(
        undecided(ask(&candidate, &deployed_cat4())),
        Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            candidate_only: BTreeSet::new(),
            deployed_only: BTreeSet::from([source(BROKER)]),
        }
    );
}

/// CAT-1 refuses the same shape for its own reason: fewer required sources can
/// lose the anchor-tier source whose silence settles its Violate.
#[test]
fn cat1_a_deployed_specification_requiring_a_subset_of_sources_is_undecided() {
    let deployed = cat1_with(
        "example.deployed",
        &[],
        &[("example.approval.outcome", "allow")],
        AUTHZ,
        AUTHZ,
    );
    assert_eq!(
        undecided(ask(&deployed_cat1(), &deployed)),
        Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            candidate_only: BTreeSet::from([source(GATEWAY)]),
            deployed_only: BTreeSet::new(),
        }
    );
}

/// Role bindings collapse into one required-source map keyed by source, and
/// the analysis compares that map, not the role bindings.
#[test]
fn two_roles_binding_one_source_compare_on_the_collapsed_map() {
    // Same two anchor-tier sources, roles swapped: different role bindings,
    // identical collapsed maps.
    let candidate = cat4_with(
        "example.c",
        &[],
        &[("example.content.trust", "untrusted")],
        BROKER,
        MAIL,
    );
    let deployed = deployed_cat4();
    assert_ne!(
        candidate.evidence_binding("action_source"),
        deployed.evidence_binding("action_source")
    );
    assert_eq!(
        candidate.ir().required_sources(),
        deployed.ir().required_sources()
    );
    assert!(ask(&candidate, &deployed).is_proven());
}

// ---------------------------------------------------------------------------
// Families with no derived argument: no numeric or interval comparison crept in
// ---------------------------------------------------------------------------

#[test]
fn cat5_pairs_are_undecided_because_no_lookback_or_threshold_argument_is_derived() {
    let evidence = || aggregation_evidence(SourceTier::Anchor, SourceTier::Anchor);
    let base = instance_of(
        PatternId::AggregationCountWithin,
        "example.deployed-count",
        aggregation_arguments(),
        evidence(),
    );

    let mut shorter = aggregation_arguments();
    shorter.insert(
        "lookback".to_owned(),
        ParameterValue::SequenceLookbackBound(8),
    );
    let mut stricter = aggregation_arguments();
    stricter.insert("threshold".to_owned(), ParameterValue::CountThreshold(2));

    for (varies, arguments) in [("lookback", shorter), ("threshold", stricter)] {
        let candidate = instance_of(
            PatternId::AggregationCountWithin,
            "example.c",
            arguments,
            evidence(),
        );
        for (left, right) in [(&candidate, &base), (&base, &candidate)] {
            assert_eq!(
                ask(left, right),
                Subsumption::Undecided(Undecided::PolarityNotDerived {
                    pattern: PatternId::AggregationCountWithin
                }),
                "a pair differing only in {varies} must not be ordered"
            );
        }
    }
}

#[test]
fn cat2_pairs_are_undecided_because_no_interval_argument_is_derived() {
    let evidence = || {
        absence_evidence(
            SourceTier::Contributing,
            SourceTier::Anchor,
            SourceTier::Anchor,
        )
    };
    let mut narrower_after = absence_arguments();
    narrower_after.insert(
        "after".to_owned(),
        selector(ABSENCE_AFTER_KIND, &[("example.approval.outcome", "allow")]),
    );
    let candidate = instance_of(
        PatternId::AbsenceAfterUntil,
        "example.c",
        narrower_after,
        evidence(),
    );
    let deployed = instance_of(
        PatternId::AbsenceAfterUntil,
        "example.deployed-absence",
        absence_arguments(),
        evidence(),
    );
    assert_eq!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::PolarityNotDerived {
            pattern: PatternId::AbsenceAfterUntil
        })
    );
}

#[test]
fn cat3_pairs_are_undecided_because_no_scope_argument_is_derived() {
    let evidence = || {
        existence_evidence(
            SourceTier::Contributing,
            SourceTier::Anchor,
            SourceTier::Contributing,
        )
    };
    let mut narrower_close = existence_arguments();
    narrower_close.insert(
        "close".to_owned(),
        selector(EXISTENCE_CLOSE_KIND, &[("example.saga.outcome", "aborted")]),
    );
    let candidate = instance_of(
        PatternId::ExistenceBeforeScopeClose,
        "example.c",
        narrower_close,
        evidence(),
    );
    let deployed = instance_of(
        PatternId::ExistenceBeforeScopeClose,
        "example.deployed-existence",
        existence_arguments(),
        evidence(),
    );
    assert_eq!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::PolarityNotDerived {
            pattern: PatternId::ExistenceBeforeScopeClose
        })
    );
}

// ---------------------------------------------------------------------------
// Well-posedness
// ---------------------------------------------------------------------------

/// A Binding of one is not a Binding of the other: the two Violate-sets hold
/// different kinds of element, so no inclusion between them is meaningful.
#[test]
fn a_different_binding_attribute_is_not_comparable() {
    let mut arguments = support::arguments();
    arguments.insert(
        "binding_attribute".to_owned(),
        ParameterValue::AttributeKey(attr_key("example.tenant.id")),
    );
    let candidate = instance_of(
        PatternId::PrecedenceApproval,
        "example.c",
        arguments,
        support::evidence(SourceTier::Anchor),
    );
    assert_eq!(
        not_comparable(ask(&candidate, &conforming(PatternId::PrecedenceApproval))),
        NotComparable::DifferentBindingAttribute {
            candidate: "example.tenant.id".to_owned(),
            deployed: BINDING.to_owned(),
        }
    );
}

#[test]
fn a_different_producer_contract_is_not_comparable() {
    let v2 = SchemaVersion::new("example-contract-v2").expect("constant");
    let candidate = instantiate(
        PatternId::PrecedenceApproval,
        identity(),
        ProducerContract::new(v2.clone()),
        support::arguments(),
        support::evidence(SourceTier::Anchor),
    )
    .expect("a fully bound instantiation succeeds");
    assert_eq!(
        not_comparable(ask(&candidate, &conforming(PatternId::PrecedenceApproval))),
        NotComparable::DifferentProducerContract {
            candidate: v2,
            deployed: contract().schema_version().clone(),
        }
    );
}

/// Opposite polarity is not inclusion in either direction: `NotComparable`, and
/// never `Undecided`, which would invite a corpus search that cannot mean
/// anything.
#[test]
fn a_cat1_candidate_and_a_cat4_specification_are_not_comparable() {
    let precedence = conforming(PatternId::PrecedenceApproval);
    let taint = conforming(PatternId::ProvenanceTaint);
    assert_eq!(
        not_comparable(ask(&precedence, &taint)),
        NotComparable::DifferentPattern {
            candidate: PatternId::PrecedenceApproval,
            deployed: PatternId::ProvenanceTaint,
        }
    );
    assert_eq!(
        not_comparable(ask(&taint, &precedence)),
        NotComparable::DifferentPattern {
            candidate: PatternId::ProvenanceTaint,
            deployed: PatternId::PrecedenceApproval,
        }
    );
}

/// Four conditions failing at once report in the documented order — Pattern,
/// Binding attribute, producer contract, then the family's own argument — and
/// each fix exposes the next.
#[test]
fn the_first_precondition_to_fail_is_reported_deterministically() {
    let v2 = || ProducerContract::new(SchemaVersion::new("example-contract-v2").expect("c"));
    let tenant = || ParameterValue::AttributeKey(attr_key("example.tenant.id"));
    let deployed = deployed_cat1();

    // Everything differs: a CAT-4 Candidate under another Binding attribute,
    // another contract, and other sources.
    let mut taint = taint_arguments();
    taint.insert("binding_attribute".to_owned(), tenant());
    let everything = instantiate(
        PatternId::ProvenanceTaint,
        named("example.c"),
        v2(),
        taint,
        support::taint_evidence(SourceTier::Anchor, SourceTier::Anchor),
    )
    .expect("instantiates");
    assert!(matches!(
        not_comparable(ask(&everything, &deployed)),
        NotComparable::DifferentPattern { .. }
    ));

    // Same Pattern; Binding attribute, contract and sources still differ.
    let mut precedence = support::arguments();
    precedence.insert("binding_attribute".to_owned(), tenant());
    let other_sources = || {
        BTreeMap::from([
            (
                "trigger_source".to_owned(),
                EvidenceBinding::new(source(MAIL), SourceTier::Contributing, capable()),
            ),
            (
                "requirement_source".to_owned(),
                EvidenceBinding::new(source(BROKER), SourceTier::Anchor, capable()),
            ),
        ])
    };
    let binding_first = instantiate(
        PatternId::PrecedenceApproval,
        named("example.c"),
        v2(),
        precedence,
        other_sources(),
    )
    .expect("instantiates");
    assert!(matches!(
        not_comparable(ask(&binding_first, &deployed)),
        NotComparable::DifferentBindingAttribute { .. }
    ));

    // Same Binding attribute; contract and sources still differ.
    let contract_next = instantiate(
        PatternId::PrecedenceApproval,
        named("example.c"),
        v2(),
        support::arguments(),
        other_sources(),
    )
    .expect("instantiates");
    assert!(matches!(
        not_comparable(ask(&contract_next, &deployed)),
        NotComparable::DifferentProducerContract { .. }
    ));

    // Well posed; only the family's evidence condition is left to fail.
    let evidence_last = instantiate(
        PatternId::PrecedenceApproval,
        named("example.c"),
        contract(),
        support::arguments(),
        other_sources(),
    )
    .expect("instantiates");
    assert!(matches!(
        undecided(ask(&evidence_last, &deployed)),
        Undecided::RequiredSourcesDiffer { .. }
    ));
}

/// Within a family, selector roles report in declared-parameter order, and the
/// evidence condition only after every role has held.
#[test]
fn within_a_family_roles_report_in_declared_order_before_the_evidence_condition() {
    // Fails the trigger, the requirement and the evidence condition at once.
    let all_three = cat1_with(
        "example.c",
        &[],
        &[
            ("example.approval.outcome", "allow"),
            ("example.approval.scope", "prod"),
        ],
        GATEWAY,
        "example-authz-replica",
    );
    let deployed = cat1(
        "example.deployed",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
    );
    assert!(matches!(
        undecided(ask(&all_three, &deployed)),
        Undecided::SelectorNotIncluded {
            parameter: "trigger",
            ..
        }
    ));

    // Trigger now holds: the requirement is next.
    let requirement_and_evidence = cat1_with(
        "example.c",
        &[("example.region", "eu")],
        &[
            ("example.approval.outcome", "allow"),
            ("example.approval.scope", "prod"),
        ],
        GATEWAY,
        "example-authz-replica",
    );
    assert!(matches!(
        undecided(ask(&requirement_and_evidence, &deployed)),
        Undecided::SelectorNotIncluded {
            parameter: "requirement",
            ..
        }
    ));

    // Both roles hold: only then is the evidence condition reported.
    let evidence_only = cat1_with(
        "example.c",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
        GATEWAY,
        "example-authz-replica",
    );
    assert!(matches!(
        undecided(ask(&evidence_only, &deployed)),
        Undecided::RequiredSourcesDiffer { .. }
    ));
}

/// The divergence speaks in neutral subject/target terms; the reviewer-facing
/// rendering names which side is the Candidate, in both directions.
#[test]
fn the_undecided_display_names_which_side_is_the_candidate() {
    let narrowed_requirement = cat1(
        "example.c",
        &[],
        &[
            ("example.approval.outcome", "allow"),
            ("example.approval.scope", "prod"),
        ],
    );
    assert_eq!(
        ask(&narrowed_requirement, &deployed_cat1()).to_string(),
        "not proven either way: parameter requirement: the deployed Specification's selector \
         must match no more Events than the Candidate's, but the Candidate's demands \
         example.approval.scope = String(\"prod\"), which the deployed Specification's does not"
    );

    let widened_trigger = cat1("example.c", &[], &[("example.approval.outcome", "allow")]);
    let narrow_deployed = cat1(
        "example.deployed",
        &[("example.region", "eu")],
        &[("example.approval.outcome", "allow")],
    );
    assert_eq!(
        ask(&widened_trigger, &narrow_deployed).to_string(),
        "not proven either way: parameter trigger: the Candidate's selector must match no more \
         Events than the deployed Specification's, but the deployed Specification's demands \
         example.region = String(\"eu\"), which the Candidate's does not"
    );
}
