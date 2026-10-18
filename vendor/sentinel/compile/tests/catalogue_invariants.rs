//! Invariants the Pattern catalogue must hold, checked against its own data.
//!
//! These are the checks that only make sense because the catalogue is data
//! rather than code (ADR-0008, and the Q3 decision recorded on issue #14): a
//! trait with one impl per Pattern could not be enumerated, so none of the
//! assertions below could be written at all.
//!
//! Two of them exist specifically to keep the honesty discipline mechanical
//! rather than rhetorical:
//! `no_first_catalogue_pattern_claims_partner_evidence` and
//! `schema_version_is_not_a_pattern_parameter`.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::collections::BTreeSet;

use sentinel_compile::catalogue::{
    catalogue, declaration, EvaluationTier, EvidenceGrade, PatternId, RetirementPolicy,
};
use sentinel_model::SourceTier;

/// The acceptance criterion asks for three to five Patterns including both
/// flagship families. Issue #141 supplies the families and the P-B absence
/// Shape; issue #173 adds CAT-5 aggregation; issue #210 adds CAT-3 existence:
/// this ships five.
///
/// The assertion is deliberately exact rather than a range: a catalogue padded
/// so a checked box could be ticked would be worse than a short one, and this
/// test failing is how a later commit is forced to say what it added.
#[test]
fn the_catalogue_ships_five_patterns() {
    assert_eq!(
        catalogue().len(),
        5,
        "the catalogue changed size without this test being updated"
    );
    assert_eq!(catalogue()[0].id, PatternId::PrecedenceApproval);
    assert_eq!(catalogue()[0].id.as_str(), "precedence.approval");
    assert_eq!(catalogue()[1].id, PatternId::ProvenanceTaint);
    assert_eq!(catalogue()[1].id.as_str(), "provenance.taint");
    assert_eq!(catalogue()[2].id, PatternId::AbsenceAfterUntil);
    assert_eq!(catalogue()[2].id.as_str(), "absence.after-until");
    assert_eq!(catalogue()[3].id, PatternId::AggregationCountWithin);
    assert_eq!(catalogue()[3].id.as_str(), "aggregation.count-within");
    assert_eq!(catalogue()[4].id, PatternId::ExistenceBeforeScopeClose);
    assert_eq!(catalogue()[4].id.as_str(), "existence.before-scope-close");
}

/// **The catalogue spans both flagship families.**
///
/// ADR-0008 names two and pairs them at opposite Filter widths:
/// precedence/approval and provenance/taint. #14a shipped one, said so plainly,
/// and left the other as the gap `docs/research/m0-evidence-corpus.md` records.
/// This is the criterion those two documents share, asserted rather than
/// counted — a catalogue of five precedence Patterns would satisfy a count and
/// fail this.
///
/// The family is read off the identifier's first dotted segment, which is the
/// catalogue's own convention and is **not** the temporal shape: CAT-4's family
/// is `provenance` and its shape is `reachability`. That divergence is the
/// reason the two are separate fields at all.
#[test]
fn the_catalogue_spans_both_flagship_families() {
    let families: BTreeSet<&str> = catalogue()
        .iter()
        .filter_map(|pattern| pattern.id.as_str().split('.').next())
        .collect();

    for flagship in ["precedence", "provenance"] {
        assert!(
            families.contains(flagship),
            "no catalogued Pattern is in the {flagship} family; ADR-0008 names two flagships              and the catalogue holds {families:?}"
        );
    }
}

/// Every Pattern identifier is `family.qualifier`, with both halves populated.
///
/// The convention `the_catalogue_spans_both_flagship_families` reads, pinned so
/// that test cannot start silently measuring nothing when an identifier without
/// a dot is added.
#[test]
fn every_pattern_id_names_a_family_and_a_qualifier() {
    for pattern in catalogue() {
        let identifier = pattern.id.as_str();
        let mut halves = identifier.split('.');
        let family = halves.next().unwrap_or_default();
        let qualifier = halves.next().unwrap_or_default();
        assert!(
            !family.is_empty() && !qualifier.is_empty() && halves.next().is_none(),
            "{identifier} is not a family.qualifier identifier"
        );
    }
}

/// Pattern identities are unique, and every declaration is reachable by id.
///
/// `declaration()` is an exhaustive match rather than a search, so a Pattern
/// added to `PatternId` without a declaration is a compile error; this pins the
/// other direction, that the match points at the right entry.
#[test]
fn pattern_ids_are_unique_and_resolve_to_their_own_declaration() {
    let ids: BTreeSet<PatternId> = catalogue().iter().map(|pattern| pattern.id).collect();
    assert_eq!(ids.len(), catalogue().len(), "duplicate PatternId");

    let strings: BTreeSet<&str> = ids.iter().map(|id| id.as_str()).collect();
    assert_eq!(strings.len(), ids.len(), "duplicate Pattern id string");

    for pattern in catalogue() {
        assert_eq!(
            declaration(pattern.id).id,
            pattern.id,
            "declaration({}) resolved to the wrong entry",
            pattern.id
        );
    }
}

/// Every Pattern declares evidence requirements, a width budget and a cost
/// class — acceptance criterion 2, at the level of the grammar rather than one
/// fixture.
///
/// The anchor-tier assertion is the substantive half. A Pattern whose every
/// requirement is satisfiable by a contributing-tier source is a Pattern that
/// can never settle an authoritative Violate, because the source inside the
/// Monitored Application is exactly the one the Monitored Application could have
/// suppressed (ADR-0022).
#[test]
fn every_pattern_declares_evidence_a_budget_and_a_cost_class() {
    for pattern in catalogue() {
        assert!(
            !pattern.evidence.is_empty(),
            "{}: no evidence requirements",
            pattern.id
        );
        assert!(
            pattern
                .evidence
                .iter()
                .any(|requirement| requirement.minimum_tier == SourceTier::Anchor),
            "{}: no requirement demands anchor tier, so it could never settle a Violate",
            pattern.id
        );
        for requirement in pattern.evidence {
            assert!(
                !requirement.rationale.is_empty(),
                "{}: role {} declares a tier with no rationale",
                pattern.id,
                requirement.role
            );
        }
        assert!(
            pattern.width.budget.event_kinds > 0,
            "{}: a Pattern that admits no Event kind matches nothing",
            pattern.id
        );
        assert!(
            !pattern.width.note.is_empty(),
            "{}: width budget carries no note",
            pattern.id
        );
        assert!(
            !pattern.parameters.is_empty(),
            "{}: a Pattern with no parameters is a Specification, not a template",
            pattern.id
        );
    }
}

/// A hot Pattern declares zero cold-path fact requests per Verdict.
///
/// This is the declaration `core/src/cost.rs` exists to score: it counts the
/// measured actual and its module doc assigns the declaration to #14/#15. A hot
/// Pattern claiming a non-zero round-trip count would be a cold Pattern with
/// the wrong tier on it. Cold Patterns (CAT-5) may declare a non-zero count;
/// this test does not force them to zero.
#[test]
fn hot_patterns_declare_no_cold_path_round_trips() {
    for pattern in catalogue() {
        if pattern.cost.tier == EvaluationTier::Hot {
            assert_eq!(
                pattern.cost.expected_fact_requests_per_verdict, 0,
                "{}: hot tier with a non-zero declared fact-request count",
                pattern.id
            );
        }
    }
}

/// A cold Pattern declares at least one cold-path fact request per Verdict.
///
/// The dual of [`hot_patterns_declare_no_cold_path_round_trips`]: a cold Pattern
/// claiming zero round-trips would be a hot Pattern with the wrong tier on it.
#[test]
fn cold_patterns_declare_at_least_one_cold_path_round_trip() {
    for pattern in catalogue() {
        if pattern.cost.tier == EvaluationTier::Cold {
            assert!(
                pattern.cost.expected_fact_requests_per_verdict >= 1,
                "{}: cold tier with a zero declared fact-request count",
                pattern.id
            );
        }
    }
}

/// A Scope that cannot fail to close declares no retirement requirement, and a
/// Scope that can must declare one.
///
/// This is O-03's obligation stated at the level of the grammar. #14a shipped
/// only the closing half — an Event-local Scope closes at its trigger — and
/// issue #110 supplies the retirement mechanism and the policy variants that go
/// with it. Stating the invariant is what makes adding a never-closing Scope
/// without a policy a test failure rather than a review oversight.
///
/// Since #110 it is asserted as a **biconditional**, over
/// [`RetirementPolicy::is_retirable`] rather than over inequality with one
/// variant. `assert_ne!(NotRequired)` was satisfiable by any future variant,
/// including one that retires nothing; `is_retirable` is the predicate the
/// Verification Core's clause (d0) actually reads, so this now pins what the
/// evaluator consults rather than a proxy for it.
#[test]
fn scopes_that_can_fail_to_close_declare_a_retirement_policy() {
    for pattern in catalogue() {
        assert_eq!(
            pattern.retirement.is_retirable(),
            !pattern.scope.always_closes(),
            "{}: Scope {} always_closes={} but declares retirement {}",
            pattern.id,
            pattern.scope.as_str(),
            pattern.scope.always_closes(),
            pattern.retirement.as_str()
        );
        if pattern.scope.always_closes() {
            assert_eq!(
                pattern.retirement,
                RetirementPolicy::NotRequired,
                "{}: a Scope that always closes must not demand retirement",
                pattern.id
            );
        } else {
            assert_ne!(
                pattern.retirement,
                RetirementPolicy::NotRequired,
                "{}: Scope {} may never close, so retirement is not optional (O-03)",
                pattern.id,
                pattern.scope.as_str()
            );
        }
    }
}

/// The two retiring policies are declarable, distinct, and separately named.
///
/// The converse half of the invariant above: that one constrains what a Pattern
/// may declare, this constrains what there is to declare. A `Deadline` that
/// stringified like an `Inactivity` would leave the golden snapshot unable to
/// tell the two apart, and would make the Verification Core's clause (d0) —
/// which compares a retirement signal's cause against the declaration — a
/// distinction no reviewer could check.
#[test]
fn deadline_and_inactivity_are_retirable_and_separately_named() {
    assert!(RetirementPolicy::Deadline.is_retirable());
    assert!(RetirementPolicy::Inactivity.is_retirable());
    assert!(!RetirementPolicy::NotRequired.is_retirable());

    let names = BTreeSet::from([
        RetirementPolicy::NotRequired.as_str(),
        RetirementPolicy::Deadline.as_str(),
        RetirementPolicy::Inactivity.as_str(),
    ]);
    assert_eq!(names.len(), 3, "each policy needs its own identifier");
    assert_eq!(RetirementPolicy::Deadline.as_str(), "deadline");
    assert_eq!(RetirementPolicy::Inactivity.as_str(), "inactivity");
}

/// **No Pattern claims partner evidence, because there is none.**
///
/// Issue #1 was closed on an explicit maintainer decision to proceed on internal
/// validation; no design partner has supplied a policy, an incident or a trace.
/// `EvidenceGrade::PartnerEvidenced` exists so that fact is visible in the data
/// and testable here, rather than surviving only as a paragraph in a PR body
/// that nobody re-reads.
///
/// When a partner artifact does land, this test is the thing that must be
/// deliberately changed — which is the point.
#[test]
fn no_first_catalogue_pattern_claims_partner_evidence() {
    for pattern in catalogue() {
        assert_ne!(
            pattern.grade,
            EvidenceGrade::PartnerEvidenced,
            "{}: claims partner evidence; no partner has supplied any",
            pattern.id
        );
    }
}

/// A corpus-evidenced Pattern cites the corpus, and an unevidenced one still
/// cites something.
///
/// A grade is a claim, and O-07 asks the claim to be traceable.
/// `every_grounding_reference_resolves` then checks the citation actually says
/// what it is cited for.
#[test]
fn every_grade_is_backed_by_a_citation() {
    for pattern in catalogue() {
        assert!(
            !pattern.grounding.is_empty(),
            "{}: no grounding references",
            pattern.id
        );
        if pattern.grade == EvidenceGrade::CorpusEvidenced {
            assert!(
                pattern
                    .grounding
                    .iter()
                    .any(|reference| reference.document == "docs/research/m0-evidence-corpus.md"),
                "{}: graded corpus-evidenced but cites no corpus scenario",
                pattern.id
            );
        }
        if pattern.grade == EvidenceGrade::Unevidenced {
            assert!(
                pattern
                    .grounding
                    .iter()
                    .any(|reference| reference.document.starts_with("docs/adr/")),
                "{}: graded unevidenced, so an ADR is the only thing left to stand on and it \
                 cites none",
                pattern.id
            );
        }
        for reference in pattern.grounding {
            assert!(
                !reference.anchor.is_empty() && !reference.supports.is_empty(),
                "{}: grounding reference to {} is not usable",
                pattern.id,
                reference.document
            );
        }
    }
}

/// **`SchemaVersion` is not a Pattern parameter.**
///
/// `PrecedenceSpecification::new` requires one, so an instantiation has to get
/// it from somewhere — and the tempting shortcut is to add it to the parameter
/// list, where #21 would then be asked to generate it and #35 to render a form
/// field for it. It is neither: it is a producer-contract fact about the Events
/// a Specification is written against, identical for every Pattern, and it
/// travels in `ProducerContract` instead. This test is the guard on that
/// decision.
#[test]
fn schema_version_is_not_a_pattern_parameter() {
    for pattern in catalogue() {
        for parameter in pattern.parameters {
            let name = parameter.name.to_ascii_lowercase();
            assert!(
                !name.contains("schema"),
                "{}: parameter {} looks like the producer schema contract, which is not a \
                 Pattern parameter",
                pattern.id,
                parameter.name
            );
        }
    }
}

/// Parameter names and evidence roles are unique within their Pattern, and
/// every display field a catalogue UI reads is populated.
///
/// Duplicates would make instantiation's argument lookup ambiguous. The display
/// fields are `dashboard/src/data/mock.ts`'s `Pattern` interface; an empty one
/// renders as a blank card in #35.
#[test]
fn names_are_unique_and_display_fields_are_populated() {
    for pattern in catalogue() {
        let parameters: BTreeSet<&str> = pattern
            .parameters
            .iter()
            .map(|parameter| parameter.name)
            .collect();
        assert_eq!(
            parameters.len(),
            pattern.parameters.len(),
            "{}: duplicate parameter name",
            pattern.id
        );

        let roles: BTreeSet<&str> = pattern
            .evidence
            .iter()
            .map(|requirement| requirement.role)
            .collect();
        assert_eq!(
            roles.len(),
            pattern.evidence.len(),
            "{}: duplicate evidence role",
            pattern.id
        );

        for (field, value) in [
            ("name", pattern.name),
            ("summary", pattern.summary),
            ("formula", pattern.formula),
            ("scope_note", pattern.scope_note),
        ] {
            assert!(
                !value.trim().is_empty(),
                "{}: display field {field} is empty",
                pattern.id
            );
        }
    }
}
