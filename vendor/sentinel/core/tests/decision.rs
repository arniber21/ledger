//! The enforcement decision seam's table, one test per row, plus the AC2/AC3
//! contracts, the aggregate rule, the wire assembly and every loud error.
//!
//! Every fixture is built on [`support::pending_world`], which holds exactly
//! the asked action's causal past, so a gate-open fixture cannot silently
//! collapse into row 9 through the dangling rule-2 edge the `happy` capture's
//! ship descendants would otherwise leave behind. Recipes come from the tree
//! — the tracer-bullet failed-charge and `prev_state` rewrites, the G2
//! sequence-gap mutator, the activation-barrier dial — never invented.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use prost::Message;
use sentinel_core::decision::{decide, Decision, DecisionError, DecisionOutcome, DeferReason};
use sentinel_core::evaluator::Evaluator;
use sentinel_core::frontier::{CausalIndexError, Frontier};
use sentinel_core::slice::{Binding, RetirementCause};
use sentinel_core::specification::{
    EventSelector, PrecedenceSpecification, RetirementPolicy, SpecificationId,
    SpecificationIdentity, SpecificationVersion,
};
use sentinel_core::taint::TaintSpecification;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::wire::probe::v1 as raw_probe;
use sentinel_model::{
    aggregate_unresolved, AvailabilityContract, DataClass, Event, EventFilter, EventFilterError,
    EventKind, FilterEpoch, ObservationTime, SentinelClockId, SourceAssurance, SourceEvidence,
    SourceIdentity, SourceTier, UnresolvedResolution,
};

use support::*;

/// The asked action's Event kind: the ship request.
const SHIP_KIND: &str = "sagashop.order.ship.requested";
/// The order service's charge request kind, P6's trigger.
const ORDER_CHARGE_KIND: &str = "sagashop.order.charge.requested";
/// The payment service's charge request kind.
const PAYMENT_CHARGE_KIND: &str = "sagashop.payment.charge.requested";

/// The single order the `happy` capture ships.
fn happy_order() -> String {
    only_shipped_order(&load_capture("happy"))
}

/// One accepted availability contract for a Specification.
fn contract_for(specification: &PrecedenceSpecification) -> AvailabilityContract {
    AvailabilityContract::accept(
        specification.identity().id(),
        specification.identity().version(),
        "sagashop-ops",
    )
}

/// The P4 Specification's selectors under a different identity: a second
/// matched entry that decides with the same table.
fn p4_shaped_as(id: &str) -> PrecedenceSpecification {
    let p4 = ship_requires_prior_charge();
    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new(id).expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        p4.binding_attribute().clone(),
        p4.trigger().clone(),
        p4.requirement().clone(),
        p4.required_sources().clone(),
        p4.schema_version().clone(),
    )
}

/// Seats Specifications behind matched entries over one world's evidence and
/// returns the (filter, evaluators, accepted contracts) triple `decide`
/// takes.
fn seated(
    evidence: &ObservedEvidence,
    specifications: &[(&PrecedenceSpecification, &[&str], raw_model::FailMode)],
    contracted: &[&PrecedenceSpecification],
) -> (
    EventFilter,
    BTreeMap<SpecificationIdentity, Evaluator>,
    BTreeSet<AvailabilityContract>,
) {
    let evaluators: BTreeMap<SpecificationIdentity, Evaluator> = specifications
        .iter()
        .map(|(specification, _, _)| {
            (
                specification.identity().clone(),
                driven_evaluator(specification, evidence),
            )
        })
        .collect();
    let entries = specifications
        .iter()
        .map(|(specification, kinds, fail_mode)| {
            enforcing_entry(
                evidence,
                specification,
                kinds,
                *fail_mode,
                DECLARED_BUDGET_NANOSECONDS,
            )
        })
        .collect();
    let accepted: BTreeSet<AvailabilityContract> = contracted
        .iter()
        .map(|specification| contract_for(specification))
        .collect();
    (enforcing_filter(entries), evaluators, accepted)
}

/// Asks one decision over a seated world.
///
/// The fixture Event is sugar for building the request; `decide` reads the
/// asked action from [`DecideRequest::producer`], not from a parallel Event.
fn ask(
    filter: &EventFilter,
    evaluators: &BTreeMap<SpecificationIdentity, Evaluator>,
    accepted: &BTreeSet<AvailabilityContract>,
    pending: &Event,
    transport_budget_nanoseconds: u64,
) -> Result<DecisionOutcome, DecisionError> {
    decide(
        &decide_request(pending),
        transport_budget_nanoseconds,
        filter,
        evaluators,
        accepted,
    )
}

/// Rebuilds a world with one source's evidence record replaced.
fn with_source_evidence(
    world: &ObservedEvidence,
    identity: SourceIdentity,
    evidence: SourceEvidence,
) -> ObservedEvidence {
    let mut sources = world.sources().clone();
    sources.insert(identity, evidence);
    ObservedEvidence::new(
        world.schema_version().clone(),
        world.events().to_vec(),
        sources,
    )
}

/// Rebuilds a world with one source's evidence record removed entirely: the
/// G1b shape — Events delivered, no completeness declaration.
fn without_source_evidence(
    world: &ObservedEvidence,
    identity: &SourceIdentity,
) -> ObservedEvidence {
    let mut sources = world.sources().clone();
    sources.remove(identity);
    ObservedEvidence::new(
        world.schema_version().clone(),
        world.events().to_vec(),
        sources,
    )
}

/// Wraps one entry as the raw Filter payload without `enforcing_filter`'s
/// expect, so a test can assert the constructor's own refusal of it.
fn raw_filter_with(entry: raw_model::SpecificationFilter) -> raw_model::EventFilter {
    raw_model::EventFilter {
        epoch: Some(DECISION_FILTER_EPOCH),
        specifications: vec![entry],
        baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }),
    }
}

/// AC2 as a helper pin, not a `decide()` pin: two Events that differ only on
/// `ObservationTime`, Data Class and `SourceAssurance` become identical
/// `DecideRequest`s once `decide_request` extracts `producer_event`. Same
/// Decision would then be tautological; asserting `producer()` equality is
/// what fails if the helper ever starts carrying invented fields (it cannot:
/// they are not on `ProducerEvent`).
#[test]
fn decide_request_drops_invented_event_fields() {
    let (_world, pending) = pending_world("happy");
    let other = Event::observe(
        pending.producer(),
        SourceAssurance::new(
            pending.source(),
            if pending.source_tier() == SourceTier::Anchor {
                SourceTier::Contributing
            } else {
                SourceTier::Anchor
            },
            pending.capabilities(),
            pending.integrity(),
            Vec::new(),
            Vec::new(),
            "other-topology",
        )
        .expect("the invented assurance validates"),
        ObservationTime::new(
            SentinelClockId::new("other-sentinel").expect("constant clock id"),
            pending
                .observation_time()
                .sentinel_nanoseconds()
                .saturating_add(1),
        ),
        if pending.data_class() == DataClass::Tenant {
            DataClass::Corpus
        } else {
            DataClass::Tenant
        },
    );
    assert_ne!(
        pending.observation_time(),
        other.observation_time(),
        "the fixtures differ on ObservationTime"
    );
    assert_ne!(
        pending.data_class(),
        other.data_class(),
        "the fixtures differ on Data Class"
    );
    assert_ne!(
        pending.assurance(),
        other.assurance(),
        "the fixtures differ on SourceAssurance"
    );
    assert_eq!(
        decide_request(&pending).producer(),
        decide_request(&other).producer(),
        "invented fields cannot ride ProducerEvent"
    );
}

/// Row 6: a delivered, admitted, anchor-tier witness in the action's declared
/// causal past permits — pinned **while a required source's Watermark lags
/// mid-stream**, which pins the witness test's precedence over the
/// Watermark check (row 6 fires before row 10): lagging Watermarks leave the
/// gate **open** — `complete_through != last_emitted` sets no gate reason —
/// but make absence uncertifiable, and a permit does not wait for that
/// certification. The gate-**closed** shape (a permit does not wait for the
/// gate itself, divergence 2), the contributing-tier shape (divergence 1)
/// and the held-intermediary shape (divergence 3) are each pinned separately
/// below.
#[test]
fn row_6_permits_on_an_anchor_witness_while_watermarks_lag() {
    let (world, pending) = pending_world("happy");
    let order = happy_order();
    let charge = only_event(&world, "sagashop.payment.charge.completed", &order);
    let lagging = with_bounds(
        world
            .source(&source(PAYMENT_SERVICE))
            .expect("payment supplied evidence"),
        &charge,
        0,
        Some(4),
        Some(2),
    );
    let world = with_source_evidence(&world, source(PAYMENT_SERVICE), lagging);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the witnessed action decides");

    assert_eq!(outcome.request_id(), DECISION_REQUEST_ID);
    assert_eq!(outcome.records().len(), 1);
    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
    assert_eq!(outcome.aggregate(), Decision::Permit);
}

/// Divergence 1, pinned: a Permit demands an **anchor-tier** witness. The
/// payment source's evidence is rebuilt at Contributing tier — everything
/// else about the witness (selector match, same Binding, admitted, in the
/// action's declared causal past) is identical to the permitting fixture —
/// and an approval the monitored application could forge is a self-approval,
/// not a discharge (ADR-0023's fourth gate). The witness falls through to
/// row 12: Defer, never Permit. Tier is no gate input, so the gate stays
/// open here — the deferral is the row-6 tier check alone.
#[test]
fn row_6_contributing_tier_witness_defers_never_permits() {
    let (world, pending) = pending_world("happy");
    let contributing = with_tier(
        world
            .source(&source(PAYMENT_SERVICE))
            .expect("payment supplied evidence"),
        SourceTier::Contributing,
    );
    let world = with_source_evidence(&world, source(PAYMENT_SERVICE), contributing);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the forgeable-witness world decides");

    let decision = outcome.records()[0].decision();
    assert_eq!(
        decision,
        Decision::Defer(DeferReason::EvidenceGap),
        "a contributing-tier-only witness falls through to row 12"
    );
    assert_ne!(decision, Decision::Permit);
}

/// Divergence 2, pinned: a Permit does not wait for the gate. The order's
/// deliver span — unrelated to the ship's causal past, so the witness chain
/// is untouched — names an absent logical parent, `retarget_logical_parent`'s
/// designed G4-only shape: the gate closes while the charge witness stays
/// delivered, admitted and reaching. The table still permits by row 6, where
/// `assess_binding` would demote to Inconclusive — the gate exists to certify
/// absences (rows 9–11), and demanding it for permits would defer nearly every
/// decision, since G5 fires mid-stream constantly.
#[test]
fn row_6_permits_while_the_evidence_gate_is_closed() {
    let mut capture = load_capture("happy");
    retarget_logical_parent(
        &mut capture,
        &|span| is_event(span, "order.deliver"),
        "0000000000000000",
    );
    let (world, pending) = pending_world_from(&capture);
    let order = happy_order();
    let witness = only_event(&world, "sagashop.payment.charge.completed", &order);

    // Premise: the deliver span's dangling parent holds it, closing G4,
    // while the witness itself stays admitted.
    let mut frontier = Frontier::new();
    for event in world.events() {
        frontier
            .step(event.clone())
            .expect("the retargeted world steps cleanly");
    }
    assert!(
        !frontier.held().is_empty(),
        "the retargeted deliver span is held, closing G4"
    );
    assert!(frontier.is_admitted(&witness.id()));

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the gate-closed world decides");

    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
    assert_eq!(outcome.aggregate(), Decision::Permit);
}

/// Divergence 3, pinned: a Permit may rest on a path through a
/// delivered-but-held intermediary. The order's fulfill span sits on the
/// witness path — its completion is one of the ship action's declared causal
/// predecessors — and retargeting its logical parent holds both of its Events
/// with zero sequence disturbance, without touching the witness's own
/// admittance. The index resolves over the delivered set, held Events
/// included, so the anchor witness still reaches from the action's declared
/// past through the held intermediary, and the table permits by row 6 — where
/// the monitor, given the same evidence, would hold the trigger and emit
/// nothing.
#[test]
fn row_6_permits_through_a_held_intermediary() {
    let mut capture = load_capture("happy");
    retarget_logical_parent(
        &mut capture,
        &|span| is_event(span, ORDER_FULFILL),
        "0000000000000000",
    );
    let (world, pending) = pending_world_from(&capture);
    let order = happy_order();
    let witness = only_event(&world, "sagashop.payment.charge.completed", &order);

    // Premise: the fulfill Events are delivered but held, and the witness is
    // admitted — so the only path from the action's declared past to the
    // witness runs through held intermediaries.
    let mut frontier = Frontier::new();
    for event in world.events() {
        frontier
            .step(event.clone())
            .expect("the retargeted world steps cleanly");
    }
    assert!(
        !frontier.held().is_empty(),
        "the retargeted fulfill span is held"
    );
    assert!(frontier.is_admitted(&witness.id()));

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the held-intermediary world decides");

    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
    assert_eq!(outcome.aggregate(), Decision::Permit);
}

/// Row 11, the flagship DENY: a failed charge is requirement-kind,
/// anchor-tier and selector-mismatching — an anchor candidate but no witness
/// — so complete evidence certifies the absence. The tracer-bullet test-7
/// recipe, which already proves the Verdict-side Violate for the same shape.
#[test]
fn row_11_denies_on_anchor_witnessed_absence() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let (world, pending) = pending_world_from(&capture);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the unmatched action decides");

    assert_eq!(outcome.records()[0].decision(), Decision::Deny);
    assert_eq!(outcome.aggregate(), Decision::Deny);
}

/// Row 8: the test-10 `prev_state` rewrite orders the successful charge
/// provably *after* the ship — delivered, admitted, matching, ordered wrong —
/// so the decision defers observationally rather than denying on a witness
/// in the action's future.
#[test]
fn row_8_defers_when_the_requirement_is_ordered_after_the_action() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, ORDER_CHARGE),
        "sagashop.order.prev_state",
        "SHIPPED",
    );
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, ORDER_SHIP),
        "sagashop.order.prev_state",
        "RESERVED",
    );
    let (world, pending) = pending_world_from(&capture);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the mis-ordered action decides");

    assert_eq!(
        outcome.records()[0].decision(),
        Decision::Defer(DeferReason::ObservationalUncertainty)
    );
    assert_eq!(
        outcome.aggregate(),
        Decision::Defer(DeferReason::ObservationalUncertainty)
    );
}

/// Row 7: the pending action's declared parent names an id no Event carries —
/// `retarget_logical_parent`'s designed shape. The action's declared past is
/// unclassifiable, which is an evidence gap, not a settled negative and
/// never a pass.
#[test]
fn row_7_defers_on_a_dangling_declared_predecessor() {
    let mut capture = load_capture("happy");
    retarget_logical_parent(
        &mut capture,
        &|span| is_event(span, ORDER_SHIP),
        "0000000000000000",
    );
    let (world, pending) = pending_world_from(&capture);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the dangling action decides");

    assert_eq!(
        outcome.records()[0].decision(),
        Decision::Defer(DeferReason::EvidenceGap)
    );
}

/// Row 9 (G1b): a required source delivered Events but declared no evidence.
/// The witness's tier is unverifiable, so no permit rests on it; the gate
/// closes; the decision defers. Never a Deny: a closed gate outranks the
/// anchor candidate the charge Events would otherwise supply.
#[test]
fn row_9_undeclared_required_source_defers_never_denies() {
    let (world, pending) = pending_world("happy");
    let g1b = without_source_evidence(&world, &source(PAYMENT_SERVICE));

    let specification = ship_requires_prior_charge();
    // The Filter entry is stamped from the world the Probe was shown, whose
    // sources did declare evidence; the monitor state is driven without the
    // declaration, which is exactly the G1b defect being decided over.
    let evaluators = BTreeMap::from([(
        specification.identity().clone(),
        driven_evaluator(&specification, &g1b),
    )]);
    let filter = enforcing_filter(vec![enforcing_entry(
        &world,
        &specification,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    )]);
    let outcome = ask(
        &filter,
        &evaluators,
        &BTreeSet::new(),
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the undeclared source is decided over, not crashed on");

    let decision = outcome.records()[0].decision();
    assert_eq!(decision, Decision::Defer(DeferReason::EvidenceGap));
    assert_ne!(decision, Decision::Deny);
}

/// Row 9 (G2): a sequence gap in a required source closes the gate — the one
/// mutator that leaves the discontinuity in on purpose — and a closed gate
/// defers even with an anchor candidate (the failed duplicate charge) sitting
/// in the Slice.
#[test]
fn row_9_sequence_gap_defers_never_denies() {
    let mut capture = load_capture("happy");
    let original_charge = capture
        .spans
        .iter()
        .find(|span| is_event(span, PAYMENT_CHARGE))
        .expect("the capture charges")
        .logical_span_id
        .clone();
    duplicate_span(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "00112233445566bb",
    );
    set_span_attribute(
        &mut capture,
        &|span| span.logical_span_id == "00112233445566bb",
        "sagashop.payment.status",
        "duplicate",
    );
    delete_span_leaving_sequence_gap(&mut capture, &|span| {
        span.logical_span_id == original_charge
    });
    let (world, pending) = pending_world_from(&capture);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the gapped evidence decides");

    let decision = outcome.records()[0].decision();
    assert_eq!(decision, Decision::Defer(DeferReason::EvidenceGap));
    assert_ne!(decision, Decision::Deny);
}

/// Row 10: the gate is open but a required source's Watermark has not caught
/// up with its emitted epoch. Mid-stream absence is uncertifiable, so the
/// anchor candidate (the failed charge) cannot turn the decision into a Deny.
#[test]
fn row_10_mid_stream_watermark_defers_never_denies() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let (world, pending) = pending_world_from(&capture);
    let order = happy_order();
    let failed = only_event(&world, "sagashop.payment.charge.completed", &order);
    let lagging = with_bounds(
        world
            .source(&source(PAYMENT_SERVICE))
            .expect("payment supplied evidence"),
        &failed,
        0,
        Some(4),
        Some(2),
    );
    let world = with_source_evidence(&world, source(PAYMENT_SERVICE), lagging);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the mid-stream world decides");

    let decision = outcome.records()[0].decision();
    assert_eq!(decision, Decision::Defer(DeferReason::PrefixUncertainty));
    assert_ne!(decision, Decision::Deny);
}

/// Row 12: a brand-new Binding with an empty Slice — a second order with no
/// Events yet. Absence with no anchor witness is uncertifiable: Defer, never
/// a Deny.
#[test]
fn row_12_absence_without_any_anchor_witness_defers() {
    let (world, pending) = pending_world("happy");
    let second_order = with_altered_identity(
        &with_altered_attribute(&pending, "sagashop.order.id", "second-order"),
        "sagashop:pending:second-order:requested",
    );

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &second_order,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the unbindable-by-absence action decides");

    let decision = outcome.records()[0].decision();
    assert_eq!(decision, Decision::Defer(DeferReason::EvidenceGap));
    assert_ne!(decision, Decision::Deny);
}

/// Row 2: a `payment.charge.requested` action against a P4 entry whose kind
/// set was deliberately widened. The Specification's trigger does not govern
/// this action, and deliberate non-coverage is a Permit with no unresolved
/// reason — the monitor's answer to a non-governing trigger is silence.
#[test]
fn row_2_permits_an_action_the_trigger_does_not_govern() {
    let (world, _ship) = pending_world("happy");
    let order = happy_order();
    let action = with_altered_identity(
        &only_event(&world, PAYMENT_CHARGE_KIND, &order),
        "sagashop:pending:payment-charge:requested",
    );

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(
            &specification,
            &[SHIP_KIND, PAYMENT_CHARGE_KIND],
            raw_model::FailMode::Open,
        )],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &action,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the non-governed action decides");

    assert_eq!(outcome.records().len(), 1);
    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
    assert_eq!(outcome.aggregate(), Decision::Permit);
    let wire = outcome.to_wire();
    assert_eq!(wire.action, raw_probe::DecisionAction::Permit as i32);
    assert_eq!(
        wire.specifications[0].unresolved_reason, None,
        "a permit carries no unresolved reason"
    );
}

/// Row 3: a governed action whose Binding attribute is stripped binds
/// nothing, and unevaluable is never a pass — Defer. With a co-matched
/// accepted-CLOSED Specification, the defer-then-deny consequence of §4.3's
/// row note is asserted at the Probe-facing boundary: the all-DEFER output
/// passes `aggregate_unresolved` and resolves Deny, with each record's
/// declared mode echoed unchanged.
#[test]
fn row_3_unbindable_action_defers_and_co_matched_closed_denies_on_defer() {
    let (world, pending) = pending_world("happy");
    let unbound = with_removed_attribute(&pending, "sagashop.order.id");

    let open = ship_requires_prior_charge();
    let closed = p4_shaped_as("sagashop.p4.fail-closed-echo");
    let (filter, evaluators, accepted) = seated(
        &world,
        &[
            (&open, &[SHIP_KIND], raw_model::FailMode::Open),
            (&closed, &[SHIP_KIND], raw_model::FailMode::Closed),
        ],
        &[&closed],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &unbound,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("both matched Specifications decide");

    assert_eq!(outcome.records().len(), 2);
    for record in outcome.records() {
        assert_eq!(record.decision(), Decision::Defer(DeferReason::EvidenceGap));
    }
    // Records are sorted by identity: "fail-closed-echo" < "ship-requires…".
    assert_eq!(outcome.records()[0].identity(), closed.identity());
    assert_eq!(
        outcome.records()[0].fail_mode(),
        raw_model::FailMode::Closed
    );
    assert_eq!(outcome.records()[1].identity(), open.identity());
    assert_eq!(outcome.records()[1].fail_mode(), raw_model::FailMode::Open);
    assert_eq!(
        outcome.aggregate(),
        Decision::Defer(DeferReason::EvidenceGap)
    );

    let matched = BTreeSet::from([
        (closed.identity().id(), closed.identity().version()),
        (open.identity().id(), open.identity().version()),
    ]);
    assert_eq!(
        aggregate_unresolved(&filter, &matched, &outcome.to_wire()),
        Ok(UnresolvedResolution::Deny),
        "the closed co-match turns the all-DEFER answer into a Probe-side deny"
    );
}

/// Row 4: the activation-barrier dial. A pending barrier defers even with the
/// witness present — a permit is a pass, and a warming Specification's
/// evidence is what it does not yet trust (ADR-0017) — and a cleared barrier
/// permits over the same evidence. The barrier is the only difference.
#[test]
fn row_4_pending_barrier_defers_and_cleared_barrier_permits() {
    let (world, pending) = pending_world("happy");
    let warming =
        ship_requires_prior_charge().with_activation_barrier(activation_barrier(&world, 1));
    let activated =
        ship_requires_prior_charge().with_activation_barrier(activation_barrier(&world, 0));

    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&warming, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the warming Specification still decides");
    assert_eq!(
        outcome.records()[0].decision(),
        Decision::Defer(DeferReason::EvidenceGap),
        "a pending barrier defers even with the witness present"
    );

    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&activated, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the activated Specification decides");
    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
}

/// Row 5: a declared cold guard is a conjunct, and a hot permit would be a
/// pass on an unproven one (ADR-0021, one-sided). The seam degrades a
/// guard-carrying Enforce entry to Defer — never Permit, never Deny — even
/// with the witness present, because row 5 precedes row 6.
#[test]
fn row_5_cold_guards_defer_never_permit_or_deny() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge_with_cold_guards();

    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the guard-carrying Specification still decides");

    let decision = outcome.records()[0].decision();
    assert_eq!(decision, Decision::Defer(DeferReason::EvidenceGap));
    assert_ne!(decision, Decision::Permit);
    assert_ne!(decision, Decision::Deny);
}

/// AC2 (row 0): a measured zero transport budget defers every matched record
/// with TIMEOUT — constructible because the at-this-hop remainder is a
/// parameter the periphery measures with its own monotonic clock, not a field
/// the validated wire newtype could carry (it refuses zero, by design).
#[test]
fn row_0_exhausted_transport_budget_defers_with_timeout() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(&filter, &evaluators, &accepted, &pending, 0)
        .expect("an exhausted budget is a decision, not an error");

    assert_eq!(
        outcome.records()[0].decision(),
        Decision::Defer(DeferReason::Timeout)
    );
    assert_eq!(outcome.aggregate(), Decision::Defer(DeferReason::Timeout));
    assert_eq!(
        outcome.to_wire().specifications[0].unresolved_reason,
        Some(raw_probe::UnresolvedReason::Timeout as i32)
    );
}

/// AC2: the record's effective budget is the minimum of the measured
/// transport remainder and the declared Enforcement Latency Budget, in both
/// orderings, and agrees with `EventFilter::effective_budget_nanoseconds` —
/// the one composition the Filter already publishes, so the record never
/// re-derives a different one.
#[test]
fn effective_budget_is_the_minimum_composition() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );

    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the witnessed action decides");
    let record = &outcome.records()[0];
    assert_eq!(
        record.effective_budget_nanoseconds(),
        DECLARED_BUDGET_NANOSECONDS,
        "the declared budget wins when it is the smaller"
    );
    assert_eq!(
        filter.effective_budget_nanoseconds(
            TRANSPORT_BUDGET_NANOSECONDS,
            &BTreeSet::from([specification.identity().id()])
        ),
        record.effective_budget_nanoseconds()
    );

    let outcome = ask(&filter, &evaluators, &accepted, &pending, 10_000_000)
        .expect("the witnessed action decides");
    let record = &outcome.records()[0];
    assert_eq!(
        record.effective_budget_nanoseconds(),
        10_000_000,
        "the measured remainder wins when it is the smaller"
    );
    assert_eq!(
        filter.effective_budget_nanoseconds(
            10_000_000,
            &BTreeSet::from([specification.identity().id()])
        ),
        10_000_000
    );
}

/// AC3: declared CLOSED serves only with an accepted availability contract
/// and echoes the declared mode verbatim; declared OPEN serves with no
/// contract demanded; uncontracted CLOSED is refused loudly, never silently
/// downgraded.
#[test]
fn fail_closed_serves_only_with_an_accepted_contract() {
    let (world, pending) = pending_world("happy");
    let closed = p4_shaped_as("sagashop.p4.contracted-closed");
    let open = ship_requires_prior_charge();

    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&closed, &[SHIP_KIND], raw_model::FailMode::Closed)],
        &[&closed],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("a contracted fail-closed Specification serves");
    assert_eq!(
        outcome.records()[0].fail_mode(),
        raw_model::FailMode::Closed
    );

    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&open, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("fail-open demands no contract");
    assert_eq!(outcome.records()[0].fail_mode(), raw_model::FailMode::Open);

    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&closed, &[SHIP_KIND], raw_model::FailMode::Closed)],
        &[],
    );
    assert_eq!(
        ask(
            &filter,
            &evaluators,
            &accepted,
            &pending,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::FailClosedWithoutContract {
            specification: closed.identity().clone()
        })
    );
}

/// AC3's co-matched consequence: a request co-matched by a contracted CLOSED
/// Specification and an uncontracted CLOSED Specification is refused with the
/// uncontracted one named, and the refusal denies at the Probe — its
/// aggregate accepted mode is CLOSED, which is the record-level fact pinned
/// here alongside the error. That a correctly contracted fail-closed
/// Specification blocks on another's misconfiguration is the accepted cost of
/// refusing loudly; the durable fix belongs to Promotion (#166).
#[test]
fn co_matched_uncontracted_closed_is_refused_and_names_the_specification() {
    let (world, pending) = pending_world("happy");
    let contracted = p4_shaped_as("sagashop.p4.contracted-closed");
    let uncontracted = p4_shaped_as("sagashop.p4.uncontracted-closed");

    let (filter, evaluators, accepted) = seated(
        &world,
        &[
            (&contracted, &[SHIP_KIND], raw_model::FailMode::Closed),
            (&uncontracted, &[SHIP_KIND], raw_model::FailMode::Closed),
        ],
        &[&contracted],
    );
    assert_eq!(
        ask(
            &filter,
            &evaluators,
            &accepted,
            &pending,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::FailClosedWithoutContract {
            specification: uncontracted.identity().clone()
        }),
        "the refusal names the uncontracted Specification"
    );
    assert!(
        filter
            .specifications()
            .iter()
            .all(|entry| entry.fail_mode == raw_model::FailMode::Closed as i32),
        "every matched entry declares CLOSED, so the Probe's aggregate accepted mode denies on this error path"
    );
}

/// The derived `Ord` is the documented dominance order. Nothing in-tree
/// consumes it today (`aggregate_of` matches by variant), so this pin is
/// what would catch a declaration-order shuffle — the previous
/// `Permit < Deny < Defer` derive had no test signal.
#[test]
fn decision_ord_is_permit_then_defer_then_deny() {
    assert!(
        Decision::Permit < Decision::Defer(DeferReason::Timeout)
            && Decision::Defer(DeferReason::Timeout) < Decision::Deny
    );
}

/// The aggregate's defer-over-permit across a split matched set: the P4
/// entry's kind set is deliberately widened (a legal over-approximation, the
/// Filter being sound) so a charge action is co-matched; P4 permits by row 2
/// while P6 decides fully and defers on the empty second-order Slice. The
/// mixed shape is the designed outcome — pinned as intended, because
/// `aggregate_unresolved` governs only the all-DEFER response and the six live
/// gates consume a mixed aggregate action-only.
#[test]
fn aggregate_defer_wins_over_permit_across_a_split_matched_set() {
    let (world, _ship) = pending_world("happy");
    let order = happy_order();
    let charge_action = with_altered_identity(
        &with_altered_attribute(
            &only_event(&world, ORDER_CHARGE_KIND, &order),
            "sagashop.order.id",
            "second-order",
        ),
        "sagashop:pending:charge:requested",
    );

    let p4 = ship_requires_prior_charge();
    let p6 = charge_requires_prior_reserve();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[
            (
                &p4,
                &[SHIP_KIND, ORDER_CHARGE_KIND],
                raw_model::FailMode::Open,
            ),
            (&p6, &[ORDER_CHARGE_KIND], raw_model::FailMode::Open),
        ],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &charge_action,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("both matched Specifications decide");

    assert_eq!(outcome.records().len(), 2);
    // Records are sorted by identity: P4 < P6.
    assert_eq!(outcome.records()[0].identity(), p4.identity());
    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
    assert_eq!(outcome.records()[1].identity(), p6.identity());
    assert_eq!(
        outcome.records()[1].decision(),
        Decision::Defer(DeferReason::EvidenceGap)
    );
    assert_eq!(
        outcome.aggregate(),
        Decision::Defer(DeferReason::EvidenceGap),
        "defer dominates permit"
    );
    assert_eq!(
        outcome.to_wire().action,
        raw_probe::DecisionAction::Defer as i32
    );
}

/// Deny-wins: a failed reserve (selector-mismatching, requirement-kind,
/// anchor-tier) certifies the absence for P6, and one denying record
/// dominates a co-matched permit.
#[test]
fn aggregate_deny_wins_over_permit() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, INVENTORY_RESERVE),
        "sagashop.inventory.reserved",
        "False",
    );
    let (world, _ship) = pending_world_from(&capture);
    let order = happy_order();
    let charge_action = with_altered_identity(
        &only_event(&world, ORDER_CHARGE_KIND, &order),
        "sagashop:pending:charge:requested",
    );

    let p4 = ship_requires_prior_charge();
    let p6 = charge_requires_prior_reserve();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[
            (
                &p4,
                &[SHIP_KIND, ORDER_CHARGE_KIND],
                raw_model::FailMode::Open,
            ),
            (&p6, &[ORDER_CHARGE_KIND], raw_model::FailMode::Open),
        ],
        &[],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &charge_action,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("both matched Specifications decide");

    assert_eq!(outcome.records()[0].decision(), Decision::Permit);
    assert_eq!(outcome.records()[1].decision(), Decision::Deny);
    assert_eq!(outcome.aggregate(), Decision::Deny, "deny dominates permit");
    assert_eq!(
        outcome.to_wire().action,
        raw_probe::DecisionAction::Deny as i32
    );
}

/// The all-DEFER output passes `aggregate_unresolved` — the Probe-side fold
/// the wire contract exists for — and a contract-accepted CLOSED entry
/// resolves it to Deny.
#[test]
fn all_defer_output_passes_aggregate_unresolved_resolving_deny() {
    let (world, _ship) = pending_world("happy");
    let order = happy_order();
    let charge_action = with_altered_identity(
        &with_altered_attribute(
            &only_event(&world, ORDER_CHARGE_KIND, &order),
            "sagashop.order.id",
            "second-order",
        ),
        "sagashop:pending:charge:requested",
    );

    let p6 = charge_requires_prior_reserve();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&p6, &[ORDER_CHARGE_KIND], raw_model::FailMode::Closed)],
        &[&p6],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &charge_action,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the unbindable charge defers");
    assert_eq!(
        outcome.records()[0].decision(),
        Decision::Defer(DeferReason::EvidenceGap)
    );

    let matched = BTreeSet::from([(p6.identity().id(), p6.identity().version())]);
    assert_eq!(
        aggregate_unresolved(&filter, &matched, &outcome.to_wire()),
        Ok(UnresolvedResolution::Deny)
    );
}

/// `to_wire` is byte-deterministic: two assemblies of one outcome encode
/// identically.
#[test]
fn to_wire_is_byte_identical_across_calls() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let (world, pending) = pending_world_from(&capture);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Closed)],
        &[&specification],
    );
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the failing action decides");

    assert_eq!(
        outcome.to_wire().encode_to_vec(),
        outcome.to_wire().encode_to_vec()
    );
    assert_eq!(outcome.to_wire(), outcome.to_wire());
}

/// Row 1: an already-delivered action is a moot question — a loud error, not
/// an uncertain answer and never a second verdict over delivered evidence.
#[test]
fn already_delivered_is_a_loud_error() {
    let (world, _ship) = pending_world("happy");
    let order = happy_order();
    let delivered = only_event(&world, PAYMENT_CHARGE_KIND, &order);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(
            &specification,
            &[SHIP_KIND, PAYMENT_CHARGE_KIND],
            raw_model::FailMode::Open,
        )],
        &[],
    );
    assert_eq!(
        ask(
            &filter,
            &evaluators,
            &accepted,
            &delivered,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::AlreadyDelivered {
            event: delivered.id()
        })
    );
}

/// A matched enforcing entry with no monitor state is a wiring bug, loudly.
#[test]
fn missing_evaluator_is_a_loud_error() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge();
    let filter = enforcing_filter(vec![enforcing_entry(
        &world,
        &specification,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    )]);

    assert_eq!(
        ask(
            &filter,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &pending,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::MissingEvaluator {
            specification: specification.identity().clone()
        })
    );
}

/// ADR-0031's transposition: a map key that disagrees with the identity of
/// the evaluator it holds is refused, rather than allowed to decide one
/// Specification's question with another's selectors silently.
#[test]
fn evaluator_identity_mismatch_is_a_loud_error() {
    let (world, pending) = pending_world("happy");
    let p4 = ship_requires_prior_charge();
    let p6 = charge_requires_prior_reserve();
    let filter = enforcing_filter(vec![enforcing_entry(
        &world,
        &p4,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    )]);
    let evaluators = BTreeMap::from([(p4.identity().clone(), driven_evaluator(&p6, &world))]);

    assert_eq!(
        ask(
            &filter,
            &evaluators,
            &BTreeSet::new(),
            &pending,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::EvaluatorIdentityMismatch {
            key: p4.identity().clone()
        })
    );
}

/// CAT-4 is detect-only (ADR-0023 gate 3). `specification()` is `None`, which
/// is not a `decide()` input: never Permit.
#[test]
fn a_taint_evaluator_is_not_a_decide_input() {
    let (world, pending) = pending_world("happy");
    let p4 = ship_requires_prior_charge();
    let filter = enforcing_filter(vec![enforcing_entry(
        &world,
        &p4,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    )]);
    let taint = TaintSpecification::new(
        p4.identity().clone(),
        p4.binding_attribute().clone(),
        EventSelector::new(
            EventKind::new("example.email.sent").expect("constant"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("example.content.ingested").expect("constant"),
            BTreeMap::new(),
        ),
        BTreeSet::new(),
        p4.schema_version().clone(),
    );
    let evaluators = BTreeMap::from([(p4.identity().clone(), Evaluator::taint(taint))]);

    assert_eq!(
        ask(
            &filter,
            &evaluators,
            &BTreeSet::new(),
            &pending,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::EvaluatorIdentityMismatch {
            key: p4.identity().clone()
        })
    );
}

/// The request's acknowledged epoch must match the handed Filter's, or every
/// record would be computed against the wrong epoch.
#[test]
fn filter_epoch_mismatch_is_a_loud_error() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge();
    let mut entry = enforcing_entry(
        &world,
        &specification,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    );
    entry.target_filter_epoch = Some(DECISION_FILTER_EPOCH + 1);
    let filter = EventFilter::try_from(raw_model::EventFilter {
        epoch: Some(DECISION_FILTER_EPOCH + 1),
        specifications: vec![entry],
        baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }),
    })
    .expect("the one-epoch-ahead fixture validates");

    let evaluators = BTreeMap::from([(
        specification.identity().clone(),
        driven_evaluator(&specification, &world),
    )]);
    assert_eq!(
        ask(
            &filter,
            &evaluators,
            &BTreeSet::new(),
            &pending,
            TRANSPORT_BUDGET_NANOSECONDS
        ),
        Err(DecisionError::FilterEpochMismatch {
            request: FilterEpoch::new(DECISION_FILTER_EPOCH),
            filter: FilterEpoch::new(DECISION_FILTER_EPOCH + 1)
        })
    );
}

/// An Evaluator whose delivered edges admit no partial order is
/// unanswerable, not deferrable: the monitor's own `advance()` returns
/// `Pending` on exactly that condition and `seal()` raises
/// `CoreError::Causality`, so the seam must not answer over the stale
/// pre-contradiction index either. The `make_mutual_transition_cycle`
/// mutator closes a two-cycle between the charge and fulfill spans, and the
/// cross-check loop refuses loudly with the contradiction carried — the
/// wiring/producer-defect class, not a defer.
#[test]
fn causally_contradictory_monitor_state_is_refused_loudly() {
    let mut capture = load_capture("happy");
    make_mutual_transition_cycle(
        &mut capture,
        &|span| is_event(span, ORDER_CHARGE),
        &|span| is_event(span, ORDER_FULFILL),
    );
    let (world, pending) = pending_world_from(&capture);

    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let error = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect_err("a contradictory world is refused, never answered");

    let DecisionError::CausalContradiction { contradiction } = error else {
        panic!("expected a causal-contradiction refusal, got {error:?}");
    };
    assert!(matches!(
        contradiction,
        CausalIndexError::ContradictoryCycle { .. }
    ));
}

/// The matched set is recomputed exactly as the Probe computes it: Warming
/// and Shadow entries are never matched and never decided, and an empty
/// matched set is the mirror of the Probe gate's ship-async no-op — Permit
/// with no records. The single-axis deviants and the constructor guarantees
/// behind the delivery and readiness conditions are pinned in
/// `each_matched_set_condition_deviated_on_alone_never_matches`.
#[test]
fn shadow_and_warming_entries_are_never_matched() {
    let (world, pending) = pending_world("happy");

    let filter = enforcing_filter(vec![shadow_entry("sagashop.warming-shadow", &[SHIP_KIND])]);
    let outcome = ask(
        &filter,
        &BTreeMap::new(),
        &BTreeSet::new(),
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("an empty matched set is a decision, not an error");
    assert_eq!(outcome.aggregate(), Decision::Permit);
    assert!(outcome.records().is_empty());

    let specification = ship_requires_prior_charge();
    let (_, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let filter = enforcing_filter(vec![
        shadow_entry("sagashop.warming-shadow", &[SHIP_KIND]),
        enforcing_entry(
            &world,
            &specification,
            &[SHIP_KIND],
            raw_model::FailMode::Open,
            DECLARED_BUDGET_NANOSECONDS,
        ),
    ]);
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the active entry decides");
    assert_eq!(
        outcome.records().len(),
        1,
        "the warming entry never reaches the table"
    );
    assert_eq!(outcome.records()[0].identity(), specification.identity());
}
/// Each matched-set condition is separately falsifiable, and the two the
/// Filter's constructor guarantees are pinned at the constructor. The
/// kind-evaluation pair is pinned here at the table: an Active entry
/// evaluated in Shadow but delivered ask-and-block (the evaluation axis
/// alone) and an Enforce entry whose kind set excludes the asked action (the
/// kind axis alone) never reach it. The delivery and readiness conditions are
/// pinned one level up: a validated `EventFilter` cannot carry `Enforce`
/// with either violated — enforcement demands ask-and-block delivery and
/// Warming demands Shadow evaluation — so the seam can never be handed the
/// entry the Probe's `isEnforceable` would refuse, and the loud refusals
/// that make that true are asserted here.
#[test]
fn each_matched_set_condition_deviated_on_alone_never_matches() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge();
    let (_, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );

    // Neither deviant entry has an evaluator, so the moment one of them
    // reaches the table the decision fails loudly with `MissingEvaluator` —
    // that is the pin. The evaluation-axis deviant is Active + ask-and-block
    // but Shadow: `EventFilter` now requires required sources and activation
    // Watermarks on every Active entry, so it is stamped from
    // `enforcing_entry` and only the evaluation mode is flipped.
    let mut shadow_evaluated = enforcing_entry(
        &world,
        &p4_shaped_as("sagashop.shadow-evaluation"),
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    );
    shadow_evaluated.evaluation_mode = raw_model::EvaluationMode::Shadow as i32;
    let unmatched_kind = enforcing_entry(
        &world,
        &p4_shaped_as("sagashop.p4.unmatched-kind"),
        &["sagashop.order.create.requested"],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    );
    let filter = enforcing_filter(vec![
        shadow_evaluated,
        unmatched_kind,
        enforcing_entry(
            &world,
            &specification,
            &[SHIP_KIND],
            raw_model::FailMode::Open,
            DECLARED_BUDGET_NANOSECONDS,
        ),
    ]);
    let outcome = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("only the kind-matching enforcing entry decides");
    assert_eq!(outcome.records().len(), 1);
    assert_eq!(outcome.records()[0].identity(), specification.identity());

    // The delivery and readiness conditions are guaranteed by the Filter's
    // own validation — an Enforce entry can never be ship-async or Warming —
    // so the pin is the constructor's loud refusal of each violation.
    let mut warming_enforcing = enforcing_entry(
        &world,
        &specification,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    );
    warming_enforcing.readiness = raw_model::Readiness::Warming as i32;
    assert_eq!(
        EventFilter::try_from(raw_filter_with(warming_enforcing)),
        Err(EventFilterError::InvalidReadinessMode),
        "Warming requires Shadow evaluation, so an Enforce entry is never Warming"
    );

    let mut ship_async_enforcing = enforcing_entry(
        &world,
        &specification,
        &[SHIP_KIND],
        raw_model::FailMode::Open,
        DECLARED_BUDGET_NANOSECONDS,
    );
    ship_async_enforcing
        .event_match
        .as_mut()
        .expect("the fixture stamps an event match")
        .delivery_mode = raw_model::DeliveryMode::ShipAsync as i32;
    assert_eq!(
        EventFilter::try_from(raw_filter_with(ship_async_enforcing)),
        Err(EventFilterError::EnforcementDeliveryMismatch),
        "enforcement demands ask-and-block, so an Enforce entry is never ship-async"
    );
}
/// Retirement is not an enforcement input, and the seam has no way to read it.
///
/// `decide()` reads Slice membership off `Evaluator::slices`, the causal index
/// and the evidence ledger. The lifecycle lives in a separate `SliceLifecycles`
/// held by the `Evaluator` and reachable through no accessor the decision seam
/// has, so "retirement is not a `decide()` input" is compiler-enforced rather
/// than conventional. This pins the behavioural half of that.
///
/// The residue is disclosed rather than hidden: because rows 9-12 read
/// gate-open, covering Watermarks and the raw member set -- none of which
/// retirement touches -- the enforcement surface can `Deny` a Binding whose
/// Verdict the Core has pinned to `Inconclusive`. Making retirement a `Defer`
/// input is #22/#26's decision, and it is recorded as a limitation in
/// `core/src/lib.rs` rather than decided here by accident.
#[test]
fn retirement_does_not_change_an_enforcement_decision() {
    let (world, pending) = pending_world("happy");
    let specification = ship_requires_prior_charge();
    let (filter, evaluators, accepted) = seated(
        &world,
        &[(&specification, &[SHIP_KIND], raw_model::FailMode::Open)],
        &[],
    );
    let baseline = ask(
        &filter,
        &evaluators,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the witnessed action decides");
    assert_eq!(
        baseline.records()[0].decision(),
        Decision::Permit,
        "the baseline must actually decide something, or this compares nothing"
    );

    // The same world, with every Binding retired before the gate ever opened.
    let mut evaluator = Evaluator::new(
        ship_requires_prior_charge().with_retirement_policy(RetirementPolicy::Deadline),
    );
    for input in event_inputs(&world) {
        evaluator.step(input).expect("events step cleanly");
    }
    let bindings: Vec<Binding> = evaluator.slices().bindings().cloned().collect();
    assert!(!bindings.is_empty(), "the fixture world creates Bindings");
    for binding in &bindings {
        evaluator
            .step(retire(
                binding.as_str(),
                1_000_000,
                RetirementCause::Deadline,
            ))
            .expect("retirement before any emission is admissible");
    }
    for input in source_inputs(&world) {
        evaluator.step(input).expect("sources step cleanly");
    }
    let retired: BTreeMap<SpecificationIdentity, Evaluator> =
        BTreeMap::from([(specification.identity().clone(), evaluator)]);

    let outcome = ask(
        &filter,
        &retired,
        &accepted,
        &pending,
        TRANSPORT_BUDGET_NANOSECONDS,
    )
    .expect("the retired world decides");

    assert_eq!(
        outcome.records()[0].decision(),
        baseline.records()[0].decision(),
        "retirement changed an enforcement decision"
    );
    assert_eq!(outcome.aggregate(), baseline.aggregate());
}
