//! End-to-end tracer bullet: recorded evidence to explainable Verdict.
//!
//! Tier A is fidelity against the three **unmodified** vendored captures, with
//! the expected property outcome read from SagaShop's own frozen oracle
//! artifact rather than hardcoded. Tier B is assurance, and every derived
//! fixture is produced in-test by a named mutation helper whose contract is
//! documented in `support/mod.rs` — nothing is checked in pre-mutated.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod support;

use std::collections::BTreeSet;

use sentinel_core::frontier::{CausalIndex, CausalIndexError};
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_ingest::capture::CapturedSpan;
use sentinel_model::{
    classify_delivery_identity, DeliveryIdentityRelation, InconclusiveReason, SettlementBasisKind,
    TierSettlement, VerdictAuthority, VerdictOutcome,
};

use support::{
    adapt, adapted, delete_span, delete_span_leaving_sequence_gap, duplicate_span, evaluate,
    event_named, frozen_property_expectation, is_event, is_event_for_order, load_capture,
    make_mutual_transition_cycle, only_event, only_shipped_order, remove_span_attribute, reordered,
    retarget_logical_parent, reuse_logical_span_id, set_span_attribute, ship_requires_prior_charge,
    source, with_altered_attribute, with_altered_identity, with_altered_slot, ORDER_CHARGE,
    ORDER_SERVICE, ORDER_SHIP, PAYMENT_CHARGE, PAYMENT_SERVICE, SHIPPING_SCHEDULE,
};

fn verdicts_for(scenario: &str) -> Vec<BindingVerdict> {
    evaluate(&ship_requires_prior_charge(), &adapted(scenario)).expect("evaluation succeeds")
}

fn only_verdict(verdicts: &[BindingVerdict]) -> &BindingVerdict {
    assert_eq!(verdicts.len(), 1, "expected exactly one Binding");
    &verdicts[0]
}

// ---------------------------------------------------------------------------
// Tier A — fidelity against the unmodified captures
// ---------------------------------------------------------------------------

/// Test 1. The happy capture settles `Satisfy` on complete, anchored evidence.
#[test]
fn happy_capture_satisfies_ship_requires_prior_charge() {
    let verdicts = verdicts_for("happy");
    let binding = only_verdict(&verdicts);
    let verdict = binding.verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Satisfy);
    assert_eq!(verdict.inconclusive_reason(), None);
    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(verdict.hot_path(), TierSettlement::Settled);
    assert_eq!(verdict.cold_path(), TierSettlement::NotRequired);
    assert_eq!(verdict.observed_sources(), verdict.required_sources());
    assert!(verdict.gaps().is_empty());
    assert!(verdict.uncertainty().is_empty());
    assert!(verdict.unresolved_causal_edges().is_empty());
    assert!(verdict.unhealthy_sources().is_empty());
    assert_eq!(verdict.source_tier_gap(), None);
    assert_eq!(
        verdict.settlement_basis(),
        Some(SettlementBasisKind::CompleteEvidence)
    );
    assert!(verdict.cold_settlement_watermarks().is_empty());

    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);
    let ship = only_event(&evidence, "sagashop.order.ship.requested", &order);

    let anchored = verdict.anchored_event_ids();
    assert!(
        anchored[&source(PAYMENT_SERVICE)].contains(&charge.id()),
        "the anchor-tier witnessing charge is what the conclusion rests on"
    );
    assert_eq!(
        binding.explanation().discharged_by.get(&ship.id()),
        Some(&charge.id()),
        "the ship trigger is discharged by that charge"
    );

    // Evidence Coverage: clock quality is the *absence* of the capability.
    for capabilities in verdict.capabilities().values() {
        assert!(!capabilities.contains(sentinel_model::SourceCapability::BoundedClockUncertainty));
    }
}

/// Test 2. A capture with no trigger produces no Binding, and the vacuity is established.
#[test]
fn cancel_capture_creates_no_obligation() {
    let verdicts = verdicts_for("cancel");
    assert!(
        verdicts.is_empty(),
        "no order ships, so no Binding and no Verdict"
    );

    // Assert the vacuity is *established* rather than assumed: both required
    // sources are observed, complete, and gap-free, so the absence of a trigger
    // is a fact about the system rather than about missing evidence.
    let evidence = adapted("cancel");
    let observed = evidence.observed_sources();
    for service in [ORDER_SERVICE, PAYMENT_SERVICE] {
        let identity = source(service);
        assert!(observed.contains(&identity), "{service} is observed");
        let record = evidence
            .source(&identity)
            .expect("source supplied evidence");
        assert!(record.is_contiguous(), "{service} has no sequence gap");
        assert_eq!(
            record.completeness().complete_through(),
            record.completeness().last_emitted(),
            "{service} is complete through everything it emitted"
        );
    }
}

/// Test 3. Every capture's aggregate outcome matches SagaShop's frozen oracle.
///
/// Aggregation rule, stated explicitly: a scenario's P4 cell is `pass` **iff
/// every Binding's Verdict is `Satisfy`** — no Violate *and* no Inconclusive —
/// because `shared/oracle.ts` returns `pass: true` only via `ok()`. So
/// `hazards.P4 = "pass"` asserts the H2 order is provably `Satisfy`, not merely
/// not-violated.
#[test]
fn hazards_capture_matches_frozen_property_matrix() {
    for scenario in ["happy", "cancel", "hazards"] {
        let expectation = frozen_property_expectation(scenario, "P4");
        let verdicts = verdicts_for(scenario);
        let all_satisfy = verdicts
            .iter()
            .all(|binding| binding.verdict().outcome() == VerdictOutcome::Satisfy);
        assert_eq!(
            all_satisfy,
            expectation == "pass",
            "{scenario}.P4 must agree with the frozen matrix cell {expectation:?}"
        );
    }

    // The hazards capture carries three order sagas, and only the H2 order
    // ships, so P4 creates exactly one Binding there. The H1 double-charge order
    // never ships and the fifteen H3 orders only reserve inventory.
    let verdicts = verdicts_for("hazards");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(
        only_verdict(&verdicts).verdict().outcome(),
        VerdictOutcome::Satisfy
    );
}

/// Test 4. The Verdict is byte-identical however the Events arrive.
///
/// This is an **implementation-determinism** test, and it is named for what it
/// tests. Arrival-order permutations are *not* admissible linearizations — an
/// admissible linearization is a total order consistent with happens-before, and
/// these permutations deliberately include orders that violate it, which is
/// exactly why the Frontier holds and retries. ADR-0020's trace-closure
/// obligation is discharged by the argument in `core/src/precedence.rs`, not
/// here. What this catches is real all the same: `HashMap` iteration, `Vec`
/// accumulation order, and first-witness-wins membership.
#[test]
fn verdict_is_invariant_across_arrival_orders() {
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        let specification = ship_requires_prior_charge();
        let baseline = encoded(&evaluate(&specification, &evidence).expect("baseline evaluates"));

        let mut orders: Vec<ArrivalOrder> = vec![
            Box::new(|_events| {}),
            Box::new(|events| events.reverse()),
            Box::new(|events| events.sort_by_key(sentinel_model::Event::id)),
            Box::new(|events| {
                events.sort_by_key(|event| {
                    (
                        event.source(),
                        event.delivery().epoch().value(),
                        event.delivery().sequence().value(),
                    )
                });
            }),
        ];
        for seed in 0..32_u64 {
            orders.push(Box::new(move |events| shuffle(events, seed)));
        }

        for (index, order) in orders.iter().enumerate() {
            let permuted = reordered(&evidence, order);
            let verdicts = evaluate(&specification, &permuted).expect("permuted evaluates");
            assert_eq!(
                encoded(&verdicts),
                baseline,
                "{scenario} arrival order {index} changed the Verdict"
            );
        }
    }
}

/// One deterministic way of presenting the same Events to the Core.
type ArrivalOrder = Box<dyn Fn(&mut Vec<sentinel_model::Event>)>;

fn encoded(verdicts: &[BindingVerdict]) -> Vec<(String, Vec<u8>)> {
    verdicts
        .iter()
        .map(|binding| {
            (
                binding.binding().to_owned(),
                binding.verdict().encode_to_vec(),
            )
        })
        .collect()
}

/// Deterministic Fisher-Yates driven by a seeded linear congruential generator.
fn shuffle(events: &mut [sentinel_model::Event], seed: u64) {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    for index in (1..events.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let pick = usize::try_from(state >> 33).expect("shift fits in usize") % (index + 1);
        events.swap(index, pick);
    }
}

/// The vendored fixtures stay small enough to belong in the repository.
///
/// The budget exists to signal a future capture change loudly, not to police
/// the current files. Trimming is rejected: it would break the
/// unmodified-artifact provenance the hermeticity argument rests on.
#[test]
fn vendored_fixtures_stay_within_their_size_budget() {
    const BUDGET_BYTES: u64 = 1_048_576;
    for scenario in ["happy", "cancel", "hazards"] {
        let path = support::capture_path(scenario);
        let size = std::fs::metadata(&path)
            .expect("vendored capture exists")
            .len();
        assert!(
            size <= BUDGET_BYTES,
            "{} is {size} bytes, over the {BUDGET_BYTES} byte budget",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// Tier B — assurance against derived fixtures
// ---------------------------------------------------------------------------

/// Test 5. Losing a causal link leaves the charge and the ship unordered.
///
/// `shared/monitor.ts`'s `happensBefore` degrades identically on the same
/// input, so SagaShop's own oracle also reports inconclusive here.
#[test]
fn missing_causal_link_is_inconclusive_for_observational_uncertainty() {
    let mut capture = load_capture("happy");
    remove_span_attribute(
        &mut capture,
        &|span| is_event(span, ORDER_SHIP),
        "sagashop.order.prev_state",
    );
    let evidence = adapt(&capture);
    let verdicts = evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
    let binding = only_verdict(&verdicts);
    let verdict = binding.verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(
        verdict.outcome(),
        VerdictOutcome::Satisfy,
        "a lost causal link must never be silently settled"
    );
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::ObservationalUncertainty)
    );

    let uncertainty = verdict.uncertainty();
    assert_eq!(
        uncertainty.len(),
        1,
        "all concurrent pairs fold into one entry per reason"
    );
    assert!(!uncertainty[0].detail.is_empty());
    assert!(!binding.explanation().concurrent_pairs.is_empty());

    // The conclusion "these cannot be ordered" genuinely rests on the anchor
    // Event being observed, so it is anchored and carries no tier gap.
    let anchored = verdict.anchored_event_ids();
    assert!(!anchored[&source(PAYMENT_SERVICE)].is_empty());
    assert_eq!(verdict.source_tier_gap(), None);
}

/// Test 6. An injected violation settles with an anchor-tier decisive witness.
///
/// Flipping the charge's status empties the requirement set, so this exercises
/// the violate row by absence of a *matching* witness, with the non-matching
/// anchor Event present to witness that absence.
#[test]
fn injected_violation_settles_with_a_decisive_witness() {
    let mut capture = load_capture("happy");
    let order = only_shipped_order(&capture);
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let evidence = adapt(&capture);
    let verdicts = evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
    let verdict = only_verdict(&verdicts).verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(
        verdict.settlement_basis(),
        Some(SettlementBasisKind::DecisiveViolation),
        "a DecisiveViolationBasis is only constructible with both invariance \
         proof rule fields set, which the model's require_text validates"
    );

    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);
    let ship = only_event(&evidence, "sagashop.order.ship.requested", &order);
    let witnesses = verdict.decisive_witness_event_ids();
    assert!(
        witnesses.contains(&charge.id()),
        "the anchor-tier payment Event witnesses the absent successful charge"
    );
    assert!(witnesses.contains(&ship.id()));
    assert!(
        verdict.anchored_event_ids()[&source(PAYMENT_SERVICE)].contains(&charge.id()),
        "the witness must also be declared as anchor evidence"
    );

    // The easy invariance case: no open evidence at all to declare.
    assert!(verdict.outcome_invariant_uncertainty().is_empty());
    assert!(verdict.gaps().is_empty());
    assert!(verdict.unresolved_causal_edges().is_empty());
    assert!(verdict.unhealthy_sources().is_empty());
    assert_eq!(verdict.source_tier_gap(), None);
}

/// Test 6b. Anchor silence is an authoritative violation (ADR-0033).
///
/// A complete, healthy, silent required anchor-tier source settles Violate via
/// `WatermarkAnchoredViolationBasis`. The pin from #10 (`Inconclusive` plus a
/// mandatory `SourceTierGap`) is reversed: silence is usable evidence, not a
/// gap.
#[test]
fn absent_anchor_event_authoritatively_violates() {
    let mut capture = load_capture("hazards");
    let order = only_shipped_order(&capture);
    // The deletion helper renumbers, so `sagashop-payment` stays contiguous and
    // this test exercises watermark-anchored silence rather than the sequence gap.
    delete_span(&mut capture, &|span| {
        is_event_for_order(span, PAYMENT_CHARGE, &order)
    });
    let evidence = adapt(&capture);
    let verdicts = evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
    let verdict = only_verdict(&verdicts).verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
    assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
    assert_ne!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(verdict.observed_sources(), verdict.required_sources());
    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert!(
        verdict.gaps().is_empty(),
        "renumbering keeps the sequence contiguous, so no G2 gap fires"
    );
    assert!(verdict.unhealthy_sources().is_empty());
    assert!(verdict.unresolved_causal_edges().is_empty());
    assert_eq!(verdict.source_tier_gap(), None);
    assert_eq!(
        verdict.settlement_basis(),
        Some(SettlementBasisKind::WatermarkAnchoredViolation)
    );
}

/// Test 7. An absent required source is a Provisional Inconclusive, never a pass.
///
/// ADR-0017's "never turn missing evidence into a pass", and ADR-0019's "without
/// a silent pass caused by missing evidence". The shape is the model's own
/// designed answer for partial coverage.
///
/// Note the admitted oddity this pins: the Verdict is labelled `Provisional` /
/// `cold PENDING` for a Specification that needs no cold-path guard at all. The
/// model has no way to say "Inconclusive, hot settled, no cold guard required, a
/// required source is missing". Issue #12 inherits it; it is not fixed here.
#[test]
fn absent_required_source_is_provisional_not_a_pass() {
    let mut capture = load_capture("happy");
    // `sagashop-payment` emits exactly one span in the happy scenario, so
    // deleting it makes the source *unobserved* rather than *gapped*.
    delete_span(&mut capture, &|span| is_event(span, PAYMENT_CHARGE));
    let evidence = adapt(&capture);
    let verdicts = evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
    let verdict = only_verdict(&verdicts).verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
    assert_ne!(verdict.outcome(), VerdictOutcome::Violate);
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(verdict.hot_path(), TierSettlement::Settled);
    assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    assert!(verdict.cold_settlement_watermarks().is_empty());
    assert_eq!(verdict.settlement_basis(), None);
    assert!(verdict.source_tier_gap().is_some());

    let observed = verdict.observed_sources();
    let required = verdict.required_sources();
    assert!(
        observed.is_subset(&required) && observed != required,
        "observed_sources is a strict subset of required_sources"
    );
    assert!(!observed.contains(&source(PAYMENT_SERVICE)));
}

/// Test 8. A sequence gap is recorded and downgrades every affected Verdict.
#[test]
fn sequence_gap_is_recorded_and_downgrades_the_verdict() {
    let mut capture = load_capture("hazards");
    // The one helper that deliberately leaves the sequence discontinuous.
    delete_span_leaving_sequence_gap(&mut capture, &|span| {
        span.service == PAYMENT_SERVICE && span.source_sequence == 1
    });
    let evidence = adapt(&capture);
    let verdicts = evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
    let verdict = only_verdict(&verdicts).verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    let gaps = verdict.gaps();
    assert_eq!(gaps.len(), 1, "one discontinuity, one recorded gap");
    assert_eq!(gaps[0].source, source(PAYMENT_SERVICE));
    assert!(!gaps[0].effect.is_empty(), "a gap always states its effect");
    assert!(verdict
        .unhealthy_sources()
        .contains(&source(PAYMENT_SERVICE)));
    assert_eq!(verdict.settlement_basis(), None);
    // The envelope-shape rule and the test plan must not drift apart: a gated
    // Inconclusive cites no anchor Event, so the tier gap is mandatory.
    assert!(
        verdict.source_tier_gap().is_some(),
        "a conclusion resting on no anchor Event carries a mandatory tier gap"
    );
}

/// Test 9. Replaying an Event is idempotent, down to the byte.
#[test]
fn duplicate_delivery_is_idempotent() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);
    assert_eq!(
        classify_delivery_identity(&charge, &charge.clone()),
        DeliveryIdentityRelation::Duplicate,
        "a replayed Event is a duplicate, not a conflict"
    );

    let mut events = evidence.events().to_vec();
    events.push(charge);
    let replayed = evidence.with_events(events);

    let specification = ship_requires_prior_charge();
    assert_eq!(
        encoded(&evaluate(&specification, &replayed).expect("replay evaluates")),
        encoded(&evaluate(&specification, &evidence).expect("baseline evaluates")),
        "a dropped duplicate leaves the Verdict byte-identical"
    );
}

/// Test 10. A mixed-outcome Binding declares its uncertainty as an invariant limitation.
///
/// This is the case that exercises the model's hardest validator:
/// `validate_invariant_limitations` requires the proof's declared uncertainty to
/// **exactly mirror** the envelope's, and a Violate whose limitations are all
/// empty never touches it.
///
/// Construction: rewrite the transition chain so the successful charge is
/// provably *after* the first ship trigger (the violate row), and add a second
/// ship trigger that is causally isolated (the observational row). The
/// duplicated ship span drops `prev_state`, so the strict transition predicate
/// excludes it from the candidate set and the original ship stays the unique
/// `SHIPPED` predecessor — the two rules compose rather than collide. The
/// mutation is structurally valid and domain-nonsensical, which is the point.
#[test]
fn mixed_outcome_binding_declares_uncertainty_as_an_invariant_limitation() {
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
    // The copy takes a fresh `sourceSequence`; reusing one would be a
    // SlotConflict, which is a different test's subject.
    duplicate_span(
        &mut capture,
        &|span| is_event(span, ORDER_SHIP),
        "00112233445566aa",
    );
    remove_span_attribute(
        &mut capture,
        &|span: &CapturedSpan| span.logical_span_id == "00112233445566aa",
        "sagashop.order.prev_state",
    );

    let evidence = adapt(&capture);
    let verdicts = evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
    let binding = only_verdict(&verdicts);
    let verdict = binding.verdict();

    assert_eq!(
        verdict.outcome(),
        VerdictOutcome::Violate,
        "one settled violation dominates one unsettled trigger"
    );
    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(
        verdict.settlement_basis(),
        Some(SettlementBasisKind::DecisiveViolation)
    );
    assert!(
        !binding.explanation().concurrent_pairs.is_empty(),
        "the second trigger has a Concurrent witness"
    );

    let uncertainty = verdict.uncertainty();
    assert_eq!(uncertainty.len(), 1);
    assert_eq!(
        uncertainty[0].reason,
        InconclusiveReason::ObservationalUncertainty
    );
    assert_eq!(
        verdict.outcome_invariant_uncertainty(),
        BTreeSet::from([InconclusiveReason::ObservationalUncertainty]),
        "the proof must mirror every open condition it claims cannot change the outcome"
    );
}

/// Test 11. A dangling causal predecessor is Inconclusive, never a Violate.
///
/// The regression test for the finding issue #12 would otherwise inherit
/// verbatim. The dangling edge is made by retargeting a `logicalParentSpanId`
/// at an identifier no span carries, which disturbs no sequence at all, so the
/// downgrade can only come from the evidence gate's G4 disjunct.
///
/// The control half matters as much as the assertion: the same capture without
/// the dangling edge settles `Violate`, so this test demonstrates that G4 is
/// what changed the outcome rather than asserting an outcome that was never in
/// doubt.
#[test]
fn dangling_causal_predecessor_is_inconclusive_not_violate() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );

    let control = evaluate(&ship_requires_prior_charge(), &adapt(&capture))
        .expect("the control capture evaluates");
    assert_eq!(
        only_verdict(&control).verdict().outcome(),
        VerdictOutcome::Violate,
        "without the dangling edge this Binding settles as a violation"
    );

    retarget_logical_parent(
        &mut capture,
        &|span| is_event(span, SHIPPING_SCHEDULE),
        "0000000000000000",
    );
    let verdicts = evaluate(&ship_requires_prior_charge(), &adapt(&capture))
        .expect("the mutated capture evaluates");
    let verdict = only_verdict(&verdicts).verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(
        verdict.outcome(),
        VerdictOutcome::Violate,
        "a violate-by-absence conclusion must not be reached while an Event is held"
    );
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    assert!(
        !verdict.unresolved_causal_edges().is_empty(),
        "the dangling predecessor is surfaced as evidence"
    );
    assert_eq!(verdict.settlement_basis(), None);
    assert!(verdict.source_tier_gap().is_some());
}

/// Test 12. A cycle that is visible only transitively is rejected.
///
/// `sentinel_model::causal_relation` already returns `ContradictoryCycle` for
/// the direct two-Event case, so the Core owns only the transitive case, which
/// is the untested one. `shared/monitor.ts` guards the same shape with its own
/// `cycle in unique order-transition graph` check.
#[test]
fn transitive_causal_cycle_is_rejected() {
    let mut capture = load_capture("happy");
    make_mutual_transition_cycle(
        &mut capture,
        &|span| is_event(span, ORDER_CHARGE),
        &|span| is_event(span, "order.fulfill"),
    );
    let evidence = adapt(&capture);

    assert!(
        matches!(
            CausalIndex::build(evidence.events()),
            Err(CausalIndexError::ContradictoryCycle { .. })
        ),
        "the index must error rather than loop or silently order"
    );
    assert!(matches!(
        evaluate(&ship_requires_prior_charge(), &evidence),
        Err(CoreError::Causality(
            CausalIndexError::ContradictoryCycle { .. }
        ))
    ));
}

/// Test 13. Every delivery conflict is an error, not a silent preference.
///
/// Only `Duplicate` was covered before. The three conflicts are constructed at
/// the Event level because the Adapter refuses to emit two Events for one
/// delivery slot, so a Core-level slot conflict cannot be reached through a
/// capture mutation at all — which is itself asserted here.
#[test]
fn delivery_conflicts_are_rejected() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);
    let specification = ship_requires_prior_charge();

    for (label, conflicting, expected) in [
        (
            "evidence",
            with_altered_attribute(&charge, "otel.span.name", "tampered"),
            DeliveryIdentityRelation::EvidenceConflict,
        ),
        (
            "slot",
            with_altered_identity(&charge, "sagashop:impostor:completed"),
            DeliveryIdentityRelation::SlotConflict,
        ),
        (
            "identity",
            with_altered_slot(&charge, 9_999),
            DeliveryIdentityRelation::IdentityConflict,
        ),
    ] {
        assert_eq!(
            classify_delivery_identity(&charge, &conflicting),
            expected,
            "{label} conflict is classified as {expected:?}"
        );
        let mut events = evidence.events().to_vec();
        events.push(conflicting);
        let result = evaluate(&specification, &evidence.with_events(events));
        assert!(
            matches!(result, Err(CoreError::Delivery(_))),
            "{label} conflict must be an error, not a drop"
        );
    }

    // The same conflict class reached through the artifact: two spans claiming
    // one logical span id produce one Event identity in two delivery slots.
    let mut capture = load_capture("happy");
    reuse_logical_span_id(&mut capture, &|span| is_event(span, ORDER_SHIP), &|span| {
        is_event(span, ORDER_CHARGE)
    });
    assert!(matches!(
        evaluate(&specification, &adapt(&capture)),
        Err(CoreError::Delivery(_))
    ));

    // And the class the Adapter refuses to hand over at all.
    let mut capture = load_capture("happy");
    let position = capture
        .spans
        .iter()
        .position(|span| is_event(span, ORDER_SHIP))
        .expect("the happy capture ships");
    capture.spans[position].source_sequence = 1;
    assert!(
        sentinel_ingest::sagashop::SagaShopAdapter::with_synchronous_call_graph()
            .adapt(&capture)
            .is_err(),
        "a reused sourceSequence never reaches the Core"
    );

    // A sanity check that the helper names a real Event, so the loop above
    // cannot degenerate into asserting against a fabricated identity.
    let _ = event_named(&evidence, charge.id().as_str());
}
