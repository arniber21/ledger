//! Issue #177's subsumption analysis, executed against the decision tables it
//! restates.
//!
//! `sentinel_compile::subsumption` derives CAT-1's and CAT-4's inclusion
//! arguments by hand from `core/src/precedence.rs` and `core/src/taint.rs`, in
//! a second crate. That is the shape ADR-0027 and issue #112 warn about, and
//! every test in `compile/tests/subsumption.rs` checks the restatement against
//! itself. This suite is the one that can notice the tables drifting: for each
//! pair the analysis **proves**, it replays both instances through the real
//! Verification Core over the same Events and the same source evidence and
//! asserts, per Binding, that a Candidate `Violate` implies a deployed
//! `Violate`.
//!
//! Four things keep that from being a green light that means nothing:
//!
//! - **Anti-vacuity.** `C ⟹ S` over zero Candidate Violates is `true`, which is
//!   exactly the `Iterator::all`-over-empty pass issue #144 refused. Every
//!   replay counts the Candidate Violates it observed and refuses zero, and the
//!   grids refuse zero **per non-reflexive pair** — a reflexive pair says
//!   nothing about inclusion, so its Violates are not counted;
//!   `the_cross_check_refuses_a_replay_with_no_candidate_violate` shows the
//!   refusal firing.
//! - **Both CAT-1 Violate branches** (amendment A8). An anchor-tier Event of the
//!   requirement's kind witnessing the absence rests on kind equality; the
//!   watermark-anchored silence of ADR-0033 rests on required-source equality.
//!   Each branch has its own test, and each asserts its own settlement basis.
//! - **Teeth.** For each selector and required-source shape the analysis
//!   refuses to prove, a found Trace on which the Candidate violates and the
//!   deployed Specification does not — so the refusal is shown necessary rather
//!   than merely cautious. CAT-1: trigger widened, requirement narrowed, and
//!   required sources differing in each direction (the deployed side's extra
//!   source closes its gate; the dropped source was the only silent anchor).
//!   CAT-4: action widened, untrusted content widened, and the deployed side
//!   requiring more sources. `RetirementDiffers` has no catalogued pair to
//!   replay, and `PolarityNotDerived` and `NotComparable` refuse no shape of an
//!   argument that was made. These are the "found in a recorded corpus"
//!   counterexamples ADR-0037 names as the buildable follow-on, in miniature.
//! - **Grids, not only hand-picked pairs.** Two bounded grids — three shapes per
//!   selector role, for CAT-1 and for CAT-4 — replay **every** ordered pair the
//!   analysis proves, so a proof nobody thought to write down is executed too.
//!
//! Two tests pin the limits the analysis states rather than hides (amendment
//! A1): a Refused cold guard on the deployed Specification leaves it
//! `Inconclusive` exactly where a proven-subsumed Candidate violates; and a
//! wider deployed selector that matches an Event from a source neither side
//! requires makes the deployed Specification's Verdict assembly fail. A third
//! pins a claim the analysis rests on: a pair whose required sources differ only
//! in minimum tier violates alike in both directions, which is what "minimum
//! tiers are unread on every decision path" means under execution.
//!
//! One test records a finding rather than a pin. The converged plan proposed
//! proving CAT-4 pairs whose deployed side requires a *subset* of the
//! Candidate's sources, conditional on this suite executing it; executed, the
//! deployed side's Verdict assembly refuses the witness from the dropped
//! source, so CAT-4's evidence condition is equality
//! (`a_deployed_specification_requiring_fewer_sources_cannot_emit_the_violate_its_monitor_reaches`).
//!
//! Only guard-free Specifications go through `sentinel_core::evaluate`, which
//! refuses a Specification declaring cold guards. For those, a Verdict's outcome
//! is the monitor's own outcome whenever the Verdict assembles, which is what
//! the analysis is about.

#![allow(clippy::doc_markdown)]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance, ProducerContract,
    SelectorValue,
};
use sentinel_compile::{
    subsumption, CandidateUnderReview, DeployedSpecification, EvidenceInclusion, FilterIdentity,
    Subsumption, Undecided,
};
use sentinel_core::evaluator::{Evaluator, Step};
use sentinel_core::facts::{GuardConclusion, RefusalReason};
use sentinel_core::specification::PrecedenceSpecification;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::{
    AttributeKey, AttributeValue, Event, EventId, EventKind, InconclusiveReason, SchemaVersion,
    SettlementBasisKind, SourceEvidence, SourceIdentity, SourceTier, VerdictError, VerdictOutcome,
};

use support::taint::{
    capable, catalogued_taint_instance, closed_sources, event, lower_taint,
    selector as taint_selector, source as source_identity, source_evidence, taint_instance, text,
    ACTION_KIND, BINDING as TAINT_BINDING, BROKER, CONTENT_KIND, MAIL, REQUEST,
};
use support::{
    adapt, answer_all_as, cold_guards, delete_span, inputs, is_event, is_event_for_order,
    load_capture, only_shipped_order, required_watermarks, set_span_attribute, source,
    ORDER_SERVICE, ORDER_SHIP, PAYMENT_CHARGE, PAYMENT_SERVICE, SAGASHOP_BINDING_ATTRIBUTE,
    SAGASHOP_SCHEMA_VERSION,
};

// ---------------------------------------------------------------------------
// The replay, and the per-Binding conditional it asserts
// ---------------------------------------------------------------------------

fn ask(candidate: &PatternInstance, deployed: &PatternInstance) -> Subsumption {
    subsumption(
        CandidateUnderReview::new(candidate),
        DeployedSpecification::new(deployed),
    )
}

fn outcomes(verdicts: &[BindingVerdict]) -> BTreeMap<String, VerdictOutcome> {
    verdicts
        .iter()
        .map(|verdict| (verdict.binding().to_owned(), verdict.verdict().outcome()))
        .collect()
}

/// Asserts, per Binding, that every Candidate Violate is a deployed Violate,
/// and returns the Candidate Violates it checked.
fn violate_carries_over(
    candidate: &[BindingVerdict],
    deployed: &[BindingVerdict],
    context: &str,
) -> usize {
    let deployed = outcomes(deployed);
    let mut violates = 0;
    for verdict in candidate {
        if verdict.verdict().outcome() != VerdictOutcome::Violate {
            continue;
        }
        violates += 1;
        assert_eq!(
            deployed.get(verdict.binding()),
            Some(&VerdictOutcome::Violate),
            "{context}: the analysis proved subsumption, yet the Candidate violates on Binding {} \
             where the deployed Specification does not",
            verdict.binding()
        );
    }
    violates
}

/// Refuses a replay whose conditional held only because nothing violated.
fn non_vacuous(candidate_violates: usize, context: &str) {
    assert!(
        candidate_violates >= 1,
        "vacuous cross-check: {context} observed no Candidate Violate, so `C => S` held over \
         the empty set and proved nothing"
    );
}

fn assert_proven(candidate: &PatternInstance, deployed: &PatternInstance) {
    let answer = ask(candidate, deployed);
    assert!(
        answer.is_proven(),
        "the cross-check replays proven pairs only: {answer}"
    );
}

// ---------------------------------------------------------------------------
// CAT-1 over the vendored SagaShop captures
// ---------------------------------------------------------------------------

const SHIP_REQUESTED: &str = "sagashop.order.ship.requested";
const CHARGE_COMPLETED: &str = "sagashop.payment.charge.completed";

fn string(value: &str) -> AttributeValue {
    AttributeValue::String(value.to_owned())
}

fn selector_value(kind: &str, attributes: &[(&str, AttributeValue)]) -> ParameterValue {
    ParameterValue::Selector(SelectorValue::new(
        EventKind::new(kind).expect("constant kind"),
        attributes
            .iter()
            .map(|(key, value)| (AttributeKey::new(*key).expect("fixture"), value.clone()))
            .collect(),
    ))
}

/// A SagaShop service bound at the tier and capabilities its capture declares.
fn sagashop_binding(service: &str) -> EvidenceBinding {
    EvidenceBinding::new(
        source(service),
        sentinel_ingest::sagashop::source_tier(service),
        sentinel_ingest::sagashop::effective_capabilities(),
    )
}

/// A CAT-1 instance over SagaShop's P4 shape — ship triggers, charge
/// requirements, order and payment services — with both selectors supplied.
fn p4_shaped(
    specification_id: &str,
    trigger: &[(&str, AttributeValue)],
    requirement: &[(&str, AttributeValue)],
) -> PatternInstance {
    cat1_instance(
        specification_id,
        trigger,
        requirement,
        sagashop_binding(ORDER_SERVICE),
        sagashop_binding(PAYMENT_SERVICE),
    )
}

/// A CAT-1 instance over P4's kinds with explicit evidence bindings.
///
/// A binding below its role's minimum tier is an instantiation warning, never
/// an error, so the fixtures below can bind a contributing-tier source to the
/// requirement role.
fn cat1_instance(
    specification_id: &str,
    trigger: &[(&str, AttributeValue)],
    requirement: &[(&str, AttributeValue)],
    trigger_source: EvidenceBinding,
    requirement_source: EvidenceBinding,
) -> PatternInstance {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(
                AttributeKey::new(SAGASHOP_BINDING_ATTRIBUTE).expect("fixture"),
            ),
        ),
        (
            "trigger".to_owned(),
            selector_value(SHIP_REQUESTED, trigger),
        ),
        (
            "requirement".to_owned(),
            selector_value(CHARGE_COMPLETED, requirement),
        ),
    ]);
    let evidence = BTreeMap::from([
        ("trigger_source".to_owned(), trigger_source),
        ("requirement_source".to_owned(), requirement_source),
    ]);
    instantiate(
        PatternId::PrecedenceApproval,
        FilterIdentity {
            specification_id: specification_id.to_owned(),
            specification_version: "v1".to_owned(),
        },
        ProducerContract::new(
            SchemaVersion::new(SAGASHOP_SCHEMA_VERSION).expect("constant schema version"),
        ),
        arguments,
        evidence,
    )
    .expect("a fully bound CAT-1 instantiation succeeds")
}

/// SagaShop P4 as deployed: every ship needs a prior successful charge.
fn deployed_p4() -> PatternInstance {
    p4_shaped(
        "sagashop.p4.ship-requires-prior-charge",
        &[],
        &[("sagashop.payment.status", string("success"))],
    )
}

/// The CAT-1 Candidates the analysis proves subsumed by P4, one per shape of
/// the argument: identical under another identity, trigger narrowed, and
/// trigger narrowed with the requirement widened (the contravariant role).
fn proven_cat1_candidates() -> Vec<(&'static str, PatternInstance)> {
    vec![
        (
            "identical under another identity",
            p4_shaped(
                "candidate.p4-copy",
                &[],
                &[("sagashop.payment.status", string("success"))],
            ),
        ),
        (
            "trigger narrowed",
            p4_shaped(
                "candidate.shipped-only",
                &[("sagashop.order.state", string("SHIPPED"))],
                &[("sagashop.payment.status", string("success"))],
            ),
        ),
        (
            "trigger narrowed, requirement widened",
            p4_shaped(
                "candidate.shipped-any-charge",
                &[("sagashop.order.state", string("SHIPPED"))],
                &[],
            ),
        ),
    ]
}

fn lower_precedence(instance: &PatternInstance) -> PrecedenceSpecification {
    PrecedenceSpecification::try_from(
        instance
            .ir()
            .as_precedence()
            .expect("CAT-1 lowers to the precedence monitor"),
    )
    .expect("catalogued CAT-1 IR converts")
}

fn replay_cat1(instance: &PatternInstance, evidence: &ObservedEvidence) -> Vec<BindingVerdict> {
    sentinel_core::evaluate(
        Evaluator::new(lower_precedence(instance)),
        evidence.events(),
        evidence.sources(),
    )
    .expect("a guard-free Specification evaluates")
}

/// The happy capture with the charge's status flipped: the ship's causal past
/// holds an anchor-tier charge Event that no longer matches `status = success`.
/// This is `tracer_bullet.rs`'s injected-violation fixture.
fn anchor_witnessed_absence() -> ObservedEvidence {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    adapt(&capture)
}

/// The hazards capture with the shipped order's charge deleted and the payment
/// sequence renumbered: a complete, healthy, silent anchor-tier source
/// (ADR-0033). This is `tracer_bullet.rs`'s watermark-anchored fixture.
fn watermark_anchored_silence() -> ObservedEvidence {
    silent_payment_anchor(None)
}

/// The same silence, with the ship recorded while the order still read `PAID`.
///
/// No vendored capture ships an order in any state but `SHIPPED`, so without
/// this Trace a trigger narrowed to `order.state = PAID` never fires and every
/// grid pair with that Candidate trigger would be checked over no Violate.
fn watermark_anchored_silence_of_a_paid_ship() -> ObservedEvidence {
    silent_payment_anchor(Some("PAID"))
}

fn silent_payment_anchor(ship_state: Option<&str>) -> ObservedEvidence {
    let mut capture = load_capture("hazards");
    let order = only_shipped_order(&capture);
    delete_span(&mut capture, &|span| {
        is_event_for_order(span, PAYMENT_CHARGE, &order)
    });
    if let Some(state) = ship_state {
        set_span_attribute(
            &mut capture,
            &|span| is_event_for_order(span, ORDER_SHIP, &order),
            "sagashop.order.state",
            state,
        );
    }
    adapt(&capture)
}

/// Replays every proven CAT-1 pair over one Trace and counts the Candidate
/// Violates that settled on the given basis.
fn cat1_branch(evidence: &ObservedEvidence, basis: SettlementBasisKind, branch: &str) -> usize {
    let deployed = deployed_p4();
    let deployed_verdicts = replay_cat1(&deployed, evidence);
    let mut on_basis = 0;
    for (shape, candidate) in proven_cat1_candidates() {
        assert_proven(&candidate, &deployed);
        let candidate_verdicts = replay_cat1(&candidate, evidence);
        violate_carries_over(
            &candidate_verdicts,
            &deployed_verdicts,
            &format!("{branch}, {shape}"),
        );
        on_basis += candidate_verdicts
            .iter()
            .filter(|verdict| {
                verdict.verdict().outcome() == VerdictOutcome::Violate
                    && verdict.verdict().settlement_basis() == Some(basis)
            })
            .count();
    }
    on_basis
}

/// A8, branch one: an anchor-tier requirement-kind Event witnesses the absence.
///
/// Rests on kind equality: the anchor candidates are the Slice's anchor-tier
/// Events of the requirement's **kind**, and inclusion forced the kinds equal.
#[test]
fn a_proven_cat1_subsumption_holds_under_replay_when_an_anchor_event_witnesses_the_absence() {
    let observed = cat1_branch(
        &anchor_witnessed_absence(),
        SettlementBasisKind::DecisiveViolation,
        "anchor-witnessed absence",
    );
    non_vacuous(observed, "the anchor-witnessed branch");
}

/// A8, branch two: ADR-0033's watermark-anchored silence.
///
/// Rests on required-source equality: `has_silent_anchor` reads the
/// Specification's own required sources, which is why CAT-1's evidence
/// condition is equality and not `⊆`.
#[test]
fn a_proven_cat1_subsumption_holds_under_replay_under_watermark_anchored_silence() {
    let observed = cat1_branch(
        &watermark_anchored_silence(),
        SettlementBasisKind::WatermarkAnchoredViolation,
        "watermark-anchored silence",
    );
    non_vacuous(observed, "the watermark-anchored branch");
}

/// The unmodified vendored captures: every Candidate Violate still implies a
/// deployed one, and whatever does not violate is not counted as evidence.
#[test]
fn a_proven_cat1_subsumption_holds_over_every_unmodified_capture() {
    let deployed = deployed_p4();
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapt(&load_capture(scenario));
        let deployed_verdicts = replay_cat1(&deployed, &evidence);
        for (shape, candidate) in proven_cat1_candidates() {
            violate_carries_over(
                &replay_cat1(&candidate, &evidence),
                &deployed_verdicts,
                &format!("{scenario}, {shape}"),
            );
        }
    }
}

/// The anti-vacuity guard, shown firing: the happy capture violates nothing,
/// so the conditional holds over the empty set and must be refused.
#[test]
#[should_panic(expected = "vacuous cross-check")]
fn the_cross_check_refuses_a_replay_with_no_candidate_violate() {
    let evidence = adapt(&load_capture("happy"));
    let deployed = deployed_p4();
    let candidate = &proven_cat1_candidates()[0].1;
    let observed = violate_carries_over(
        &replay_cat1(candidate, &evidence),
        &replay_cat1(&deployed, &evidence),
        "happy",
    );
    non_vacuous(observed, "the unmodified happy capture");
}

/// **Teeth for the contravariance.** A Candidate that narrows CAT-1's
/// requirement violates on a recorded Trace where the deployed Specification
/// is satisfied — a found counterexample, and the reason the analysis leaves
/// the pair `Undecided` instead of proving it.
#[test]
fn a_candidate_that_narrows_the_requirement_violates_where_the_deployed_specification_does_not() {
    let deployed = deployed_p4();
    let candidate = p4_shaped(
        "candidate.named-key-charge",
        &[],
        &[
            ("sagashop.payment.status", string("success")),
            ("sagashop.payment.idempotency_key", string("no-such-key")),
        ],
    );
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::SelectorNotIncluded {
            parameter: "requirement",
            ..
        })
    ));

    let evidence = adapt(&load_capture("happy"));
    let candidate_outcomes = outcomes(&replay_cat1(&candidate, &evidence));
    let deployed_outcomes = outcomes(&replay_cat1(&deployed, &evidence));
    let counterexamples: Vec<&String> = candidate_outcomes
        .iter()
        .filter(|(binding, outcome)| {
            **outcome == VerdictOutcome::Violate
                && deployed_outcomes.get(*binding) == Some(&VerdictOutcome::Satisfy)
        })
        .map(|(binding, _)| binding)
        .collect();
    assert!(
        !counterexamples.is_empty(),
        "expected a Binding on which the narrowed Candidate violates and P4 is satisfied: \
         Candidate {candidate_outcomes:?}, deployed {deployed_outcomes:?}"
    );
}

/// The Bindings on which the Candidate violates and the deployed Specification
/// does not.
fn counterexamples(candidate: &[BindingVerdict], deployed: &[BindingVerdict]) -> Vec<String> {
    let deployed = outcomes(deployed);
    candidate
        .iter()
        .filter(|verdict| {
            verdict.verdict().outcome() == VerdictOutcome::Violate
                && deployed.get(verdict.binding()) != Some(&VerdictOutcome::Violate)
        })
        .map(|verdict| verdict.binding().to_owned())
        .collect()
}

/// **Teeth for CAT-1's covariant trigger.** A Candidate that widens the
/// trigger raises an obligation on a ship the deployed Specification does not
/// look at, and violates there.
#[test]
fn a_candidate_that_widens_the_trigger_violates_where_the_deployed_specification_does_not() {
    let success = [("sagashop.payment.status", string("success"))];
    let deployed = p4_shaped(
        "deployed.shipped-only",
        &[("sagashop.order.state", string("SHIPPED"))],
        &success,
    );
    let candidate = p4_shaped("candidate.any-ship", &[], &success);
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::SelectorNotIncluded {
            parameter: "trigger",
            ..
        })
    ));

    let evidence = watermark_anchored_silence_of_a_paid_ship();
    let found = counterexamples(
        &replay_cat1(&candidate, &evidence),
        &replay_cat1(&deployed, &evidence),
    );
    assert!(
        !found.is_empty(),
        "expected a Binding on which the widened trigger violates and the deployed \
         Specification does not"
    );
}

/// The Binding of the hand-built CAT-1 Traces below.
const HAND_BUILT_ORDER: &str = "order-1";

/// A hand-built SagaShop-shaped Event for one order, from one application.
fn order_event(
    id: &str,
    sequence: u64,
    kind: &str,
    application: &str,
    order: &str,
    attributes: &[(&str, AttributeValue)],
) -> Event {
    let mut attributes = attributes.to_vec();
    attributes.push((SAGASHOP_BINDING_ATTRIBUTE, text(order)));
    event(
        id,
        sequence,
        kind,
        application,
        &attributes,
        BTreeSet::new(),
    )
}

/// A hand-built source bound at an explicit tier.
fn bound(application: &str, tier: SourceTier) -> EvidenceBinding {
    EvidenceBinding::new(source_identity(application), tier, capable())
}

/// Asserts the one hand-built Binding's Candidate Verdict is a
/// watermark-anchored Violate, and returns the deployed outcome and
/// Inconclusive reason there.
fn candidate_violates_by_silence(
    candidate: &PatternInstance,
    deployed: &PatternInstance,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Option<(VerdictOutcome, Option<InconclusiveReason>)> {
    let lower = |instance: &PatternInstance| {
        sentinel_core::evaluate(Evaluator::new(lower_precedence(instance)), events, sources)
            .expect("a guard-free Specification evaluates")
    };
    let candidate_verdicts = lower(candidate);
    let violate = candidate_verdicts
        .iter()
        .find(|verdict| verdict.binding() == HAND_BUILT_ORDER)
        .expect("the hand-built order has a Candidate Verdict");
    assert_eq!(violate.verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(
        violate.verdict().settlement_basis(),
        Some(SettlementBasisKind::WatermarkAnchoredViolation),
        "the Candidate's Violate rests on ADR-0033's silent anchor"
    );
    lower(deployed)
        .iter()
        .find(|verdict| verdict.binding() == HAND_BUILT_ORDER)
        .map(|verdict| {
            (
                verdict.verdict().outcome(),
                verdict.verdict().inconclusive_reason(),
            )
        })
}

/// **Teeth for CAT-1's evidence condition: the deployed side requires more.**
/// The order service never reports, so G1 closes the deployed Specification's
/// gate, leaving it `Inconclusive(PrefixUncertainty)`; the Candidate requires
/// only the payment service, which records the ship request itself and is a
/// complete, silent anchor (ADR-0033).
#[test]
fn a_deployed_cat1_specification_requiring_more_sources_stays_gated_where_the_candidate_violates() {
    let anchor = || bound(PAYMENT_SERVICE, SourceTier::Anchor);
    let success = [("sagashop.payment.status", string("success"))];
    let candidate = cat1_instance("candidate.payment-only", &[], &success, anchor(), anchor());
    let deployed = cat1_instance(
        "deployed.order-and-payment",
        &[],
        &success,
        bound(ORDER_SERVICE, SourceTier::Contributing),
        anchor(),
    );
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            ..
        })
    ));

    let events = vec![order_event(
        "ship-1",
        1,
        SHIP_REQUESTED,
        PAYMENT_SERVICE,
        HAND_BUILT_ORDER,
        &[],
    )];
    let sources = BTreeMap::from([(
        source_identity(PAYMENT_SERVICE),
        source_evidence(PAYMENT_SERVICE, 1, SourceTier::Anchor),
    )]);
    assert_eq!(
        candidate_violates_by_silence(&candidate, &deployed, &events, &sources),
        Some((
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty)
        ))
    );
}

/// **Teeth for CAT-1's evidence condition: the deployed side requires fewer.**
/// The source it drops is the payment service, the only anchor-tier source
/// either side requires. Both observe the same silence over the same complete
/// sources; only the Candidate's `has_silent_anchor` finds an anchor in its own
/// required set, so only the Candidate settles the Violate (ADR-0033); the
/// deployed prefix stays open, `Inconclusive(PrefixUncertainty)`.
#[test]
fn a_deployed_cat1_specification_dropping_the_silent_anchor_stays_inconclusive_where_the_candidate_violates(
) {
    let success = [("sagashop.payment.status", string("success"))];
    let order = || bound(ORDER_SERVICE, SourceTier::Contributing);
    let candidate = cat1_instance(
        "candidate.order-and-payment",
        &[],
        &success,
        order(),
        bound(PAYMENT_SERVICE, SourceTier::Anchor),
    );
    let deployed = cat1_instance("deployed.order-only", &[], &success, order(), order());
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            ..
        })
    ));

    // The payment service is healthy and complete: it charged another order,
    // so G1 stays open on the Candidate, and never this one.
    let events = vec![
        order_event(
            "charge-2",
            1,
            CHARGE_COMPLETED,
            PAYMENT_SERVICE,
            "order-2",
            &[("sagashop.payment.status", string("success"))],
        ),
        order_event(
            "ship-1",
            1,
            SHIP_REQUESTED,
            ORDER_SERVICE,
            HAND_BUILT_ORDER,
            &[],
        ),
    ];
    let sources = BTreeMap::from([
        (
            source_identity(ORDER_SERVICE),
            source_evidence(ORDER_SERVICE, 1, SourceTier::Contributing),
        ),
        (
            source_identity(PAYMENT_SERVICE),
            source_evidence(PAYMENT_SERVICE, 1, SourceTier::Anchor),
        ),
    ]);
    assert_eq!(
        candidate_violates_by_silence(&candidate, &deployed, &events, &sources),
        Some((
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty)
        ))
    );
}

/// **The minimum-tier claim, pinned by execution.** The two instances swap
/// which SagaShop service fills which role, so their required-source identity
/// sets are equal and each source's minimum tier is the other role's. The
/// analysis proves the pair both ways because no Core decision path reads a
/// minimum tier. If one ever does — gating on a bound source below its role's
/// minimum, say — one direction loses its Violates or its conditional here,
/// and the evidence condition has to start comparing tiers.
#[test]
fn required_sources_differing_only_in_minimum_tier_violate_alike_under_replay() {
    let success = [("sagashop.payment.status", string("success"))];
    let p4 = deployed_p4();
    let swapped = cat1_instance(
        "candidate.roles-swapped",
        &[],
        &success,
        sagashop_binding(PAYMENT_SERVICE),
        sagashop_binding(ORDER_SERVICE),
    );
    assert_ne!(
        swapped.ir().required_sources(),
        p4.ir().required_sources(),
        "the fixture must actually differ in minimum tier"
    );
    let traces = [anchor_witnessed_absence(), watermark_anchored_silence()];
    for (candidate, deployed, direction) in [
        (&swapped, &p4, "roles swapped under P4"),
        (&p4, &swapped, "P4 under roles swapped"),
    ] {
        assert_proven(candidate, deployed);
        let violates: usize = traces
            .iter()
            .map(|evidence| {
                violate_carries_over(
                    &replay_cat1(candidate, evidence),
                    &replay_cat1(deployed, evidence),
                    direction,
                )
            })
            .sum();
        non_vacuous(violates, direction);
    }
}

/// **A1: the stated limit, pinned.** The analysis proves subsumption on the
/// monitors' own outcomes; a Refused cold guard on the deployed Specification
/// still rewrites its Violate to `Inconclusive(EpistemicUncertainty)` at
/// Verdict assembly (`epistemic_from_refused` in `core/src/verdict.rs`).
///
/// Cold guards are Core-side rollout state and never reach the IR, so the two
/// sides here are the **same** instance: the analysis cannot tell them apart
/// and proves the pair. The Verdicts can tell them apart.
#[test]
fn a_refused_cold_guard_leaves_the_deployed_specification_inconclusive_where_the_candidate_violates(
) {
    let instance = deployed_p4();
    assert_proven(&instance, &instance);

    let evidence = anchor_witnessed_absence();
    let candidate = replay_cat1(&instance, &evidence);
    assert_eq!(
        outcomes(&candidate).into_values().collect::<Vec<_>>(),
        vec![VerdictOutcome::Violate],
        "the guard-free Candidate settles Violate"
    );

    // The deployed side declares cold guards, so it is stepped rather than
    // batch-evaluated (`evaluate` refuses a Specification that needs facts).
    let mut deployed = Evaluator::new(lower_precedence(&instance).with_cold_guards(cold_guards()));
    let mut request = None;
    for input in inputs(&evidence) {
        if let Step::NeedsFacts(asked) = deployed.step(input).expect("the fixture steps cleanly") {
            request = Some(asked);
        }
    }
    let request = match request {
        Some(request) => request,
        None => match deployed
            .resume(Vec::new())
            .expect("an empty resume is legal")
        {
            Step::NeedsFacts(request) => request,
            other => panic!("expected NeedsFacts, got {other:?}"),
        },
    };
    let settled = deployed
        .resume(answer_all_as(
            &request,
            &GuardConclusion::Refused {
                reason: RefusalReason::EvidenceReaped,
            },
            &required_watermarks(&evidence),
        ))
        .expect("a terminal refusal is a settling answer");
    let Step::Settled(verdicts) = settled else {
        panic!("expected Settled, got {settled:?}");
    };

    assert_eq!(verdicts.len(), 1);
    let verdict = verdicts[0].verdict();
    assert_eq!(
        verdicts[0].binding(),
        candidate[0].binding(),
        "the same Binding on both sides"
    );
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::EpistemicUncertainty)
    );
}

// ---------------------------------------------------------------------------
// CAT-4 over hand-built Events
// ---------------------------------------------------------------------------

const RECIPIENT_DOMAIN: &str = "example.recipient.domain";
const CONTENT_ORIGIN: &str = "example.content.origin";
const CONTENT_TRUST: &str = "example.content.trust";

/// The Monitored Application of a relay no fixture Specification requires.
const RELAY: &str = "example-mail-relay";

fn content_from(
    application: &str,
    id: &str,
    sequence: u64,
    trust: &str,
    origin: Option<&str>,
) -> Event {
    let mut attributes = vec![(TAINT_BINDING, text(REQUEST)), (CONTENT_TRUST, text(trust))];
    if let Some(origin) = origin {
        attributes.push((CONTENT_ORIGIN, text(origin)));
    }
    event(
        id,
        sequence,
        CONTENT_KIND,
        application,
        &attributes,
        BTreeSet::new(),
    )
}

fn content(id: &str, sequence: u64, trust: &str, origin: Option<&str>) -> Event {
    content_from(BROKER, id, sequence, trust, origin)
}

fn sent(id: &str, sequence: u64, domain: &str, predecessors: &[&str]) -> Event {
    sent_from(MAIL, id, sequence, domain, predecessors)
}

fn sent_from(
    application: &str,
    id: &str,
    sequence: u64,
    domain: &str,
    predecessors: &[&str],
) -> Event {
    event(
        id,
        sequence,
        ACTION_KIND,
        application,
        &[
            (TAINT_BINDING, text(REQUEST)),
            (RECIPIENT_DOMAIN, text(domain)),
        ],
        predecessors
            .iter()
            .map(|id| EventId::new(*id).expect("constant"))
            .collect(),
    )
}

type Replay = (
    &'static str,
    Vec<Event>,
    BTreeMap<SourceIdentity, SourceEvidence>,
);

/// Traces spanning every CAT-4 decision row the pairs below can reach.
fn cat4_traces() -> Vec<Replay> {
    let closed = || closed_sources(1, 1, SourceTier::Anchor);
    vec![
        (
            "untrusted web content reaches an external send",
            vec![
                content("content-1", 1, "untrusted", Some("web")),
                sent("mail-1", 1, "external", &["content-1"]),
            ],
            closed(),
        ),
        (
            "untrusted content of no named origin reaches an external send",
            vec![
                content("content-1", 1, "untrusted", None),
                sent("mail-1", 1, "external", &["content-1"]),
            ],
            closed(),
        ),
        (
            "untrusted web content reaches an internal send",
            vec![
                content("content-1", 1, "untrusted", Some("web")),
                sent("mail-1", 1, "internal", &["content-1"]),
            ],
            closed(),
        ),
        (
            "trusted content reaches an external send",
            vec![
                content("content-1", 1, "trusted", Some("web")),
                sent("mail-1", 1, "external", &["content-1"]),
            ],
            closed(),
        ),
        (
            "untrusted web content concurrent with an external send",
            vec![
                content("content-1", 1, "untrusted", Some("web")),
                sent("mail-1", 1, "external", &[]),
            ],
            closed(),
        ),
        (
            "the broker claims an Event the Core never received",
            vec![
                content("content-1", 1, "untrusted", Some("web")),
                sent("mail-1", 1, "external", &["content-1"]),
            ],
            closed_sources(1, 2, SourceTier::Anchor),
        ),
    ]
}

fn replay_cat4(
    instance: &PatternInstance,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Vec<BindingVerdict> {
    sentinel_core::evaluate(Evaluator::taint(lower_taint(instance)), events, sources)
        .expect("a guard-free Specification evaluates")
}

/// Replays one proven CAT-4 pair over every trace and counts Candidate Violates.
fn cat4_pair(candidate: &PatternInstance, deployed: &PatternInstance, pair: &str) -> usize {
    assert_proven(candidate, deployed);
    cat4_traces()
        .iter()
        .map(|(trace, events, sources)| {
            violate_carries_over(
                &replay_cat4(candidate, events, sources),
                &replay_cat4(deployed, events, sources),
                &format!("{pair}, {trace}"),
            )
        })
        .sum()
}

#[test]
fn a_proven_cat4_subsumption_holds_under_replay() {
    let deployed = catalogued_taint_instance();
    let candidate = taint_instance(
        FilterIdentity {
            specification_id: "candidate.external-web-taint".to_owned(),
            specification_version: "v1".to_owned(),
        },
        taint_selector(ACTION_KIND, &[(RECIPIENT_DOMAIN, "external")]),
        taint_selector(
            CONTENT_KIND,
            &[(CONTENT_TRUST, "untrusted"), (CONTENT_ORIGIN, "web")],
        ),
        MAIL,
        BROKER,
    );
    let observed = cat4_pair(&candidate, &deployed, "both selectors narrowed");
    non_vacuous(observed, "the CAT-4 selector pair");
}

/// **The finding that set CAT-4's evidence condition to equality.**
///
/// The converged plan proposed proving a deployed Specification that requires
/// a *subset* of the Candidate's sources, on the argument that CAT-4's evidence
/// gate is monotone in the required set — and made that conditional on this
/// suite executing it. Executed: the deployed Specification drops the content
/// source, its monitor reaches `Violate` (the refusal below is raised only while
/// assembling a decisive Violate basis), and Verdict assembly refuses the
/// untrusted-content witness because its source is not required. The Candidate
/// emits the Violate; the deployed Specification emits an evaluation error. So
/// the pair is left `Undecided`.
#[test]
fn a_deployed_specification_requiring_fewer_sources_cannot_emit_the_violate_its_monitor_reaches() {
    let candidate = catalogued_taint_instance();
    let deployed = taint_instance(
        FilterIdentity {
            specification_id: "deployed.mail-only-evidence".to_owned(),
            specification_version: "v1".to_owned(),
        },
        taint_selector(ACTION_KIND, &[]),
        taint_selector(CONTENT_KIND, &[(CONTENT_TRUST, "untrusted")]),
        MAIL,
        MAIL,
    );
    assert_eq!(deployed.ir().required_sources().len(), 1);
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            ..
        })
    ));

    let events = vec![
        content("content-1", 1, "untrusted", Some("web")),
        sent("mail-1", 1, "external", &["content-1"]),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    assert_eq!(
        outcomes(&replay_cat4(&candidate, &events, &sources)).get(REQUEST),
        Some(&VerdictOutcome::Violate)
    );
    let deployed_result =
        sentinel_core::evaluate(Evaluator::taint(lower_taint(&deployed)), &events, &sources);
    assert!(
        matches!(
            deployed_result,
            Err(CoreError::Verdict(VerdictError::InvalidDecisiveWitness(
                "source"
            )))
        ),
        "expected Verdict assembly to refuse the witness from the dropped source, got \
         {deployed_result:?}"
    );
}

/// **A1's second limit, pinned.** Equal required sources keep witness
/// validation identical for every Event *both* sides cite, but a wider
/// deployed selector can cite an Event the Candidate's does not. Here the
/// deployed action selector also matches a send from a relay neither side
/// requires; its monitor reaches `Violate` on both sends, and Verdict assembly
/// refuses the relay's witness. The analysis proves the pair — at the monitor
/// level it is right — and the Verdicts disagree.
#[test]
fn a_wider_deployed_selector_matching_an_event_from_an_unrequired_source_fails_verdict_assembly() {
    let deployed = catalogued_taint_instance();
    let candidate = taint_instance(
        FilterIdentity {
            specification_id: "candidate.external-only".to_owned(),
            specification_version: "v1".to_owned(),
        },
        taint_selector(ACTION_KIND, &[(RECIPIENT_DOMAIN, "external")]),
        taint_selector(CONTENT_KIND, &[(CONTENT_TRUST, "untrusted")]),
        MAIL,
        BROKER,
    );
    assert_proven(&candidate, &deployed);

    let events = vec![
        content("content-1", 1, "untrusted", Some("web")),
        sent("mail-1", 1, "external", &["content-1"]),
        sent_from(RELAY, "relay-1", 1, "internal", &["content-1"]),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    assert_eq!(
        outcomes(&replay_cat4(&candidate, &events, &sources)).get(REQUEST),
        Some(&VerdictOutcome::Violate)
    );
    let deployed_result =
        sentinel_core::evaluate(Evaluator::taint(lower_taint(&deployed)), &events, &sources);
    assert!(
        matches!(
            deployed_result,
            Err(CoreError::Verdict(VerdictError::InvalidDecisiveWitness(
                "source"
            )))
        ),
        "expected Verdict assembly to refuse the relay's witness, got {deployed_result:?}"
    );
}

/// **Teeth for CAT-4's covariance.** Treat more content as untrusted and the
/// Candidate violates on content the deployed Specification does not consider
/// untrusted at all.
#[test]
fn a_candidate_that_widens_the_untrusted_content_selector_violates_where_the_deployed_specification_does_not(
) {
    let deployed = catalogued_taint_instance();
    let candidate = taint_instance(
        FilterIdentity {
            specification_id: "candidate.any-content".to_owned(),
            specification_version: "v1".to_owned(),
        },
        taint_selector(ACTION_KIND, &[]),
        taint_selector(CONTENT_KIND, &[]),
        MAIL,
        BROKER,
    );
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::SelectorNotIncluded {
            parameter: "untrusted_content",
            ..
        })
    ));

    let events = vec![
        content("content-1", 1, "trusted", Some("web")),
        sent("mail-1", 1, "external", &["content-1"]),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    assert_eq!(
        outcomes(&replay_cat4(&candidate, &events, &sources)).get(REQUEST),
        Some(&VerdictOutcome::Violate)
    );
    assert_eq!(
        outcomes(&replay_cat4(&deployed, &events, &sources)).get(REQUEST),
        Some(&VerdictOutcome::Satisfy)
    );
}

/// **Teeth for CAT-4's covariant action.** Watch every send rather than
/// external ones and the Candidate violates on an internal send the deployed
/// Specification does not watch.
#[test]
fn a_candidate_that_widens_the_action_selector_violates_where_the_deployed_specification_does_not()
{
    let untrusted = || taint_selector(CONTENT_KIND, &[(CONTENT_TRUST, "untrusted")]);
    let deployed = taint_instance(
        FilterIdentity {
            specification_id: "deployed.external-only".to_owned(),
            specification_version: "v1".to_owned(),
        },
        taint_selector(ACTION_KIND, &[(RECIPIENT_DOMAIN, "external")]),
        untrusted(),
        MAIL,
        BROKER,
    );
    let candidate = catalogued_taint_instance();
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::SelectorNotIncluded {
            parameter: "action",
            ..
        })
    ));

    let events = vec![
        content("content-1", 1, "untrusted", Some("web")),
        sent("mail-1", 1, "internal", &["content-1"]),
    ];
    let sources = closed_sources(1, 1, SourceTier::Anchor);
    assert_eq!(
        counterexamples(
            &replay_cat4(&candidate, &events, &sources),
            &replay_cat4(&deployed, &events, &sources),
        ),
        vec![REQUEST.to_owned()]
    );
}

/// **Teeth for CAT-4's evidence condition, the other way round.** A deployed
/// Specification requiring a source the Candidate does not stays gated where
/// the Candidate violates. Every witness the Candidate cites comes from the one
/// source it requires, so its own Verdict assembles.
#[test]
fn a_deployed_specification_requiring_more_sources_stays_gated_where_the_candidate_violates() {
    let candidate = taint_instance(
        FilterIdentity {
            specification_id: "candidate.mail-only-evidence".to_owned(),
            specification_version: "v1".to_owned(),
        },
        taint_selector(ACTION_KIND, &[]),
        taint_selector(CONTENT_KIND, &[(CONTENT_TRUST, "untrusted")]),
        MAIL,
        MAIL,
    );
    let deployed = catalogued_taint_instance();
    assert!(matches!(
        ask(&candidate, &deployed),
        Subsumption::Undecided(Undecided::RequiredSourcesDiffer {
            condition: EvidenceInclusion::Equal,
            ..
        })
    ));

    // The mail gateway ingests the content itself and never hears from the
    // broker, which reports nothing at all.
    let events = vec![
        content_from(MAIL, "content-1", 1, "untrusted", Some("web")),
        sent("mail-1", 2, "external", &["content-1"]),
    ];
    let sources = BTreeMap::from([(
        source_identity(MAIL),
        source_evidence(MAIL, 2, SourceTier::Anchor),
    )]);
    assert_eq!(
        outcomes(&replay_cat4(&candidate, &events, &sources)).get(REQUEST),
        Some(&VerdictOutcome::Violate)
    );
    assert_eq!(
        outcomes(&replay_cat4(&deployed, &events, &sources)).get(REQUEST),
        Some(&VerdictOutcome::Inconclusive)
    );
}

// ---------------------------------------------------------------------------
// Bounded grids: every proven pair, not only the pairs written down above
// ---------------------------------------------------------------------------

/// Names a grid instance by its position, for failure messages.
fn grid_identity(family: &str, index: usize) -> FilterIdentity {
    FilterIdentity {
        specification_id: format!("grid.{family}.{index}"),
        specification_version: "v1".to_owned(),
    }
}

/// Replays every ordered pair from a grid of instances over every Trace, and
/// asserts `C Violate => S Violate` for **each** pair the analysis proves.
///
/// `verdicts[trace][instance]` is computed once per instance, so the pair loop
/// replays nothing. Refuses a grid that proved only the reflexive pairs, which
/// would exercise no inclusion, and refuses **each** non-reflexive proven pair
/// whose Candidate violated on no Trace: a reflexive pair says nothing about
/// inclusion, so its Violates cannot stand in for another pair's.
fn every_proven_pair_holds(
    family: &str,
    instances: &[PatternInstance],
    traces: &[&str],
    verdicts: &[Vec<Vec<BindingVerdict>>],
) {
    let mut proven = 0;
    for (c, candidate) in instances.iter().enumerate() {
        for (s, deployed) in instances.iter().enumerate() {
            if !ask(candidate, deployed).is_proven() {
                continue;
            }
            proven += 1;
            let candidate_violates: usize = traces
                .iter()
                .enumerate()
                .map(|(t, trace)| {
                    violate_carries_over(
                        &verdicts[t][c],
                        &verdicts[t][s],
                        &format!("{family} grid, {c} under {s}, {trace}"),
                    )
                })
                .sum();
            if c != s {
                non_vacuous(
                    candidate_violates,
                    &format!("the {family} grid pair {c} under {s}"),
                );
            }
        }
    }
    assert!(
        proven > instances.len(),
        "the {family} grid proved only its {proven} reflexive pairs and exercised no inclusion"
    );
}

/// CAT-1: three trigger shapes by three requirement shapes over P4's kinds,
/// replayed over both Violate branches, the silence of a ship recorded as
/// `PAID` (so the `PAID` trigger shape fires at all), and every unmodified
/// capture.
#[test]
fn every_proven_cat1_pair_in_a_bounded_grid_holds_under_replay() {
    let shapes = |key: &'static str, values: [&str; 2]| {
        let mut shapes: Vec<Vec<(&str, AttributeValue)>> = vec![Vec::new()];
        shapes.extend(values.iter().map(|value| vec![(key, string(value))]));
        shapes
    };
    let triggers = shapes("sagashop.order.state", ["SHIPPED", "PAID"]);
    let requirements = shapes("sagashop.payment.status", ["success", "duplicate"]);
    let instances: Vec<PatternInstance> = triggers
        .iter()
        .flat_map(|trigger| {
            requirements
                .iter()
                .map(move |requirement| (trigger, requirement))
        })
        .enumerate()
        .map(|(index, (trigger, requirement))| {
            p4_shaped(
                &grid_identity("cat1", index).specification_id,
                trigger,
                requirement,
            )
        })
        .collect();

    let traces = [
        ("anchor-witnessed absence", anchor_witnessed_absence()),
        ("watermark-anchored silence", watermark_anchored_silence()),
        (
            "watermark-anchored silence of a PAID ship",
            watermark_anchored_silence_of_a_paid_ship(),
        ),
        ("happy", adapt(&load_capture("happy"))),
        ("cancel", adapt(&load_capture("cancel"))),
        ("hazards", adapt(&load_capture("hazards"))),
    ];
    let verdicts: Vec<Vec<Vec<BindingVerdict>>> = traces
        .iter()
        .map(|(_, evidence)| {
            instances
                .iter()
                .map(|instance| replay_cat1(instance, evidence))
                .collect()
        })
        .collect();
    let names: Vec<&str> = traces.iter().map(|(name, _)| *name).collect();
    every_proven_pair_holds("CAT-1", &instances, &names, &verdicts);
}

/// CAT-4: three action shapes by three untrusted-content shapes, replayed over
/// every CAT-4 Trace this suite holds.
#[test]
fn every_proven_cat4_pair_in_a_bounded_grid_holds_under_replay() {
    let actions: [&[(&str, &str)]; 3] = [
        &[],
        &[(RECIPIENT_DOMAIN, "external")],
        &[(RECIPIENT_DOMAIN, "internal")],
    ];
    let contents: [&[(&str, &str)]; 3] = [
        &[],
        &[(CONTENT_TRUST, "untrusted")],
        &[(CONTENT_TRUST, "untrusted"), (CONTENT_ORIGIN, "web")],
    ];
    let instances: Vec<PatternInstance> = actions
        .iter()
        .flat_map(|action| contents.iter().map(move |content| (*action, *content)))
        .enumerate()
        .map(|(index, (action, content))| {
            taint_instance(
                grid_identity("cat4", index),
                taint_selector(ACTION_KIND, action),
                taint_selector(CONTENT_KIND, content),
                MAIL,
                BROKER,
            )
        })
        .collect();

    let traces = cat4_traces();
    let verdicts: Vec<Vec<Vec<BindingVerdict>>> = traces
        .iter()
        .map(|(_, events, sources)| {
            instances
                .iter()
                .map(|instance| replay_cat4(instance, events, sources))
                .collect()
        })
        .collect();
    let names: Vec<&str> = traces.iter().map(|(name, _, _)| *name).collect();
    every_proven_pair_holds("CAT-4", &instances, &names, &verdicts);
}
