//! End-to-end oracle harness: P1-P8 properties over SagaShop scenarios.
//!
//! Tier A is **fidelity**: every property's outcome matches SagaShop's own
//! frozen oracle matrix for all three vendored captures, and every
//! PrecedenceSpecification's verdict is byte-identical across arrival orders.
//!
//! Tier B is **assurance**: injected violations settle authoritatively (Violate
//! for precedence, Fail for oracle), watermark-complete anchor silence settles
//! Violate (ADR-0033), missing evidence is never a silent pass, and concurrent
//! events and late arrivals are handled correctly.
//!
//! P1, P2, P3, P5, P7 are **oracle checks** — direct evidence inspection that
//! bypasses the Core's anchor rule. P4, P6, P8 are **PrecedenceSpecifications**
//! evaluated by the Core. P7 is an oracle check because its requirement
//! (`order.create.requested`) comes from a Contributing-tier source, so the
//! Core's anchor rule could never produce an authoritative verdict for it.
//!
//! ADR-0019: hermetic recorded evidence — no network, no wall clock.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod support;

use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::slice::{Binding, RetirementBreach, RetirementCause};
use sentinel_core::specification::{PrecedenceSpecification, RetirementPolicy};
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::{
    InconclusiveReason, SettlementBasisKind, TierSettlement, VerdictAuthority, VerdictOutcome,
};

use support::{
    adapt, adapted, all_precedence_specifications, charge_requires_prior_reserve,
    delete_all_spans_of_service, delete_span, delete_span_leaving_sequence_gap, duplicate_span,
    encoded_verdicts, evaluate, event_inputs, frozen_property_expectation,
    fulfill_requires_prior_charge, is_event, is_event_for_order, load_capture, only_shipped_order,
    oracle_check, orders_with_event, reordered, retire, set_span_attribute,
    ship_requires_prior_charge, source_inputs, OracleOutcome, INVENTORY_RESERVE, INVENTORY_SERVICE,
    ORDER_CHARGE, ORDER_FULFILL, ORDER_SHIP, PAYMENT_CHARGE, PAYMENT_SERVICE,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns whether every Binding's Verdict is Satisfy — the aggregation rule
/// for a precedence property's scenario cell (pass = all Satisfy, no Violate,
/// no Inconclusive). Vacuously true when there are no Bindings.
fn all_satisfy(verdicts: &[BindingVerdict]) -> bool {
    verdicts
        .iter()
        .all(|b| b.verdict().outcome() == VerdictOutcome::Satisfy)
}

/// Returns the Binding Verdict for one order, panicking if it does not exist.
fn verdict_for_order<'a>(verdicts: &'a [BindingVerdict], order: &str) -> &'a BindingVerdict {
    verdicts
        .iter()
        .find(|b| b.binding() == order)
        .unwrap_or_else(|| panic!("no binding for order {order}"))
}

/// One deterministic way of presenting the same Events to the Core.
type ArrivalOrder = Box<dyn Fn(&mut Vec<sentinel_model::Event>)>;

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

/// Returns 36 deterministic arrival-order permutations (4 named + 32 shuffles).
fn arrival_orders() -> Vec<ArrivalOrder> {
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
    orders
}

// ---------------------------------------------------------------------------
// Tier A — Fidelity
// ---------------------------------------------------------------------------

/// Test 1. All 8 properties match the frozen oracle matrix for all 3 scenarios.
///
/// Precedence properties (P4, P6, P8): pass iff every Binding's Verdict is
/// Satisfy. Oracle properties (P1, P2, P3, P5, P7): pass iff `oracle_check`
/// returns `Pass`.
#[test]
fn all_properties_match_frozen_matrix() {
    let precedence = all_precedence_specifications();
    let oracle_properties = ["P1", "P2", "P3", "P5", "P7"];

    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);

        // Precedence properties via Core verdict.
        for (id, spec) in &precedence {
            let verdicts = evaluate(spec, &evidence).expect("evaluation succeeds");
            let expected = frozen_property_expectation(scenario, id);
            let actual_pass = all_satisfy(&verdicts);
            assert_eq!(
                actual_pass,
                expected == "pass",
                "{scenario}.{id}: Core says {actual_pass}, matrix says {expected:?}"
            );
        }

        // Oracle properties via direct evidence inspection.
        for id in oracle_properties {
            let outcome = oracle_check(id, &evidence);
            let expected = frozen_property_expectation(scenario, id);
            assert_eq!(
                outcome,
                if expected == "pass" {
                    OracleOutcome::Pass
                } else {
                    OracleOutcome::Fail
                },
                "{scenario}.{id}: oracle says {outcome:?}, matrix says {expected:?}"
            );
        }
    }
}

/// Test 2. PrecedenceSpecification verdicts are byte-identical across 36
/// arrival-order permutations, for P4, P6, P8, over all 3 scenarios.
///
/// This is an implementation-determinism test: arrival-order permutations are
/// not admissible linearizations, but they catch HashMap iteration, Vec
/// accumulation order, and first-witness-wins membership.
#[test]
fn precedence_verdicts_invariant_across_arrival_orders() {
    let orders = arrival_orders();
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for (id, spec) in all_precedence_specifications() {
            let baseline =
                encoded_verdicts(&evaluate(&spec, &evidence).expect("baseline evaluates"));

            for (index, order) in orders.iter().enumerate() {
                let permuted = reordered(&evidence, order);
                let verdicts = evaluate(&spec, &permuted).expect("permuted evaluates");
                assert_eq!(
                    encoded_verdicts(&verdicts),
                    baseline,
                    "{scenario} {id} arrival order {index} changed the Verdict"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tier A — Arrival-order invariance with retirement interleaved (#110, AC5)
// ---------------------------------------------------------------------------

/// The scenario the corpus designates for O-03.
///
/// `docs/research/m0-evidence-corpus.md` names S4 -- the hazards capture's
/// never-closing compensation Scope -- as the O-03 test scenario. The
/// retirement-crossed variant is held to one scenario and one Specification
/// deliberately: the unbounded cross-product of three scenarios, three
/// Specifications, thirty-six orders and every insertion position is roughly ten
/// thousand evaluations, and `precedence_verdicts_invariant_across_arrival_orders`
/// above already runs the full breadth without retirement and stays green
/// untouched. No coverage is lost; only the crossed variant is narrow, and the
/// narrowing is a cost bound rather than a claim.
const RETIREMENT_SCENARIO: &str = "hazards";

/// Returns the Binding this pair retires, and a horizon strictly inside its own
/// observation range.
///
/// "Strictly inside" is the whole point. With the horizon above everything the
/// retired Slice equals the full member set, admission discriminates nothing,
/// and the invariance assertion degenerates into asserting that a constant is
/// constant -- the same family of defect commit `7c4fc5f` removed from this
/// file. Both sides of the split are asserted non-empty here.
fn retirement_subject(
    evidence: &ObservedEvidence,
    specification: &PrecedenceSpecification,
) -> (String, i128) {
    let mut by_binding: std::collections::BTreeMap<String, Vec<i128>> =
        std::collections::BTreeMap::new();
    for event in evidence.events() {
        if let Some(binding) = specification.binding_of(event) {
            by_binding
                .entry(binding)
                .or_default()
                .push(event.observation_time().sentinel_nanoseconds());
        }
    }
    let (binding, mut times) = by_binding
        .into_iter()
        .filter(|(_, times)| times.len() >= 2)
        .max_by_key(|(binding, times)| (times.len(), binding.clone()))
        .expect("the fixture has a Binding with at least two observations");
    times.sort_unstable();
    let horizon = times[times.len() / 2 - 1];
    assert!(
        times.iter().any(|time| *time <= horizon),
        "the in-scope Slice must be non-empty"
    );
    assert!(
        times.iter().any(|time| *time > horizon),
        "the in-scope Slice must be a *proper* subset"
    );
    (binding, horizon)
}

/// Drives one input vector to a seal, returning encoded Verdicts.
fn seal_encoded(
    specification: &PrecedenceSpecification,
    inputs: Vec<Input>,
) -> Vec<(String, Vec<u8>)> {
    let mut evaluator = Evaluator::new(specification.clone());
    for input in inputs {
        evaluator.step(input).expect("the fixture steps cleanly");
    }
    encoded_verdicts(evaluator.seal().expect("sealing succeeds").verdicts())
}

/// AC5(a). Sealed Verdicts are invariant across arrival orders with a `Retire`
/// input interleaved strictly before the Binding's first emission.
///
/// The Verdict is a function of the input **multiset**, not of arrival position,
/// because admissibility compares the horizon against each Event's own
/// observation time. An Event observed before the horizon is admitted even when
/// it is delivered after the retirement signal.
///
/// Positions are covered in two passes rather than one, for cost: all
/// thirty-six orders run at the first, middle and last position, and the
/// identity order runs at **every** position. The union is what "every position
/// in every order" would assert, at a fraction of the evaluations -- the full
/// cross-product over this fixture is thirty-six times sixty-two evaluations of
/// a sixty-one-span capture, which would dominate this harness.
#[test]
fn verdicts_are_invariant_across_arrival_orders_with_retire_before_first_emission() {
    let evidence = adapted(RETIREMENT_SCENARIO);
    let specification =
        charge_requires_prior_reserve().with_retirement_policy(RetirementPolicy::Deadline);
    let (binding, horizon) = retirement_subject(&evidence, &specification);

    let baseline = {
        let mut inputs = event_inputs(&evidence);
        inputs.insert(0, retire(&binding, horizon, RetirementCause::Deadline));
        inputs.extend(source_inputs(&evidence));
        seal_encoded(&specification, inputs)
    };
    assert!(
        !baseline.is_empty(),
        "the baseline must contain Verdicts, or this compares nothing to nothing"
    );

    let events = event_inputs(&evidence).len();
    let spread = [0, events / 2, events];
    for (index, order) in arrival_orders().iter().enumerate() {
        let permuted = reordered(&evidence, order);
        for position in spread {
            let mut inputs = event_inputs(&permuted);
            inputs.insert(
                position,
                retire(&binding, horizon, RetirementCause::Deadline),
            );
            inputs.extend(source_inputs(&permuted));
            assert_eq!(
                seal_encoded(&specification, inputs),
                baseline,
                "order {index} with retire at {position} changed the sealed Verdict"
            );
        }
    }

    for position in 0..=events {
        let mut inputs = event_inputs(&evidence);
        inputs.insert(
            position,
            retire(&binding, horizon, RetirementCause::Deadline),
        );
        inputs.extend(source_inputs(&evidence));
        assert_eq!(
            seal_encoded(&specification, inputs),
            baseline,
            "retire at position {position} changed the sealed Verdict"
        );
    }
}

/// AC5(b). At or after the Binding's first emission, every position returns
/// exactly `BindingAlreadyEmitted` -- and the evaluator survives it.
///
/// This is the residue the amended criterion asserts rather than excludes.
/// Clause (d2) rejects on prior emission, not on observation time, and the
/// driver feeds Events then sources, so every position at or after the step that
/// opens the gate is inadmissible for **any** horizon. Refusing is right: the
/// alternative is silently overwriting an emitted Verdict with an
/// `Inconclusive`, an outcome retraction with no Amendment record.
#[test]
fn retiring_at_or_after_the_first_emission_is_rejected_at_every_position() {
    let evidence = adapted(RETIREMENT_SCENARIO);
    let specification =
        charge_requires_prior_reserve().with_retirement_policy(RetirementPolicy::Deadline);
    let (binding, horizon) = retirement_subject(&evidence, &specification);

    let mut all_inputs = event_inputs(&evidence);
    all_inputs.extend(source_inputs(&evidence));
    let key = Binding::new(&binding);

    let mut admitted = 0_usize;
    let mut refused = 0_usize;
    for position in 0..=all_inputs.len() {
        let mut evaluator = Evaluator::new(specification.clone());
        for input in all_inputs.iter().take(position) {
            evaluator
                .step(input.clone())
                .expect("the fixture steps cleanly");
        }
        let emitted = evaluator.is_emitted(&key);
        let outcome = evaluator.step(retire(&binding, horizon, RetirementCause::Deadline));
        if emitted {
            refused += 1;
            assert_eq!(
                outcome,
                Err(CoreError::Retirement(
                    RetirementBreach::BindingAlreadyEmitted
                )),
                "position {position} sits at or after the first emission"
            );
            assert!(
                evaluator.resume(Vec::new()).is_ok(),
                "the refusal must not wedge the evaluator"
            );
            assert!(evaluator.seal().is_ok());
        } else {
            admitted += 1;
            assert!(
                outcome.is_ok(),
                "position {position} sits before the first emission: {outcome:?}"
            );
        }
    }
    assert!(admitted > 0, "some positions must be admissible");
    assert!(
        refused > 0,
        "the residue this test exists for must be reachable"
    );
}

// ---------------------------------------------------------------------------
// Tier B — Injected Violations
// ---------------------------------------------------------------------------

/// Test 4. P4/P6/P8 injected violations settle with an authoritative Violate.
///
/// Each mutation flips an **attribute** on the requirement Event (not deletes
/// it), so the Event stays present as an anchor candidate (kind match, anchor
/// tier) while failing the attribute match — producing Violate rather than the
/// Inconclusive the anchor-downgrade rule would impose on deletion.
#[test]
fn injected_violation_settles_authoritative_violate() {
    // P4: flip payment.charge status → no matching successful charge.
    {
        let mut capture = load_capture("happy");
        set_span_attribute(
            &mut capture,
            &|span| is_event(span, PAYMENT_CHARGE),
            "sagashop.payment.status",
            "duplicate",
        );
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let verdict =
            verdict_for_order(&verdicts, &only_shipped_order(&load_capture("happy"))).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
        assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
        assert_eq!(
            verdict.settlement_basis(),
            Some(SettlementBasisKind::DecisiveViolation)
        );
    }

    // P6: flip inventory.reserve reserved → no matching reserved reserve.
    {
        let mut capture = load_capture("happy");
        set_span_attribute(
            &mut capture,
            &|span| is_event(span, INVENTORY_RESERVE),
            "sagashop.inventory.reserved",
            "False",
        );
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&charge_requires_prior_reserve(), &evidence).expect("evaluation succeeds");
        let order = &orders_with_event(&load_capture("happy"), ORDER_CHARGE)[0];
        let verdict = verdict_for_order(&verdicts, order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
        assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
        assert_eq!(
            verdict.settlement_basis(),
            Some(SettlementBasisKind::DecisiveViolation)
        );
    }

    // P8: flip payment.charge status → no matching successful charge for fulfill.
    {
        let mut capture = load_capture("happy");
        set_span_attribute(
            &mut capture,
            &|span| is_event(span, PAYMENT_CHARGE),
            "sagashop.payment.status",
            "duplicate",
        );
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&fulfill_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let order = &orders_with_event(&load_capture("happy"), ORDER_FULFILL)[0];
        let verdict = verdict_for_order(&verdicts, order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
        assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
        assert_eq!(
            verdict.settlement_basis(),
            Some(SettlementBasisKind::DecisiveViolation)
        );
    }
}

/// Test 5. P1/P2/P3/P5/P7 injected violations make `oracle_check` return Fail.
#[test]
fn oracle_injected_violations_return_fail() {
    // P1: ship from a cancelled order.
    {
        let mut capture = load_capture("happy");
        set_span_attribute(
            &mut capture,
            &|span| is_event(span, ORDER_SHIP),
            "sagashop.order.prev_state",
            "CANCELLED",
        );
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P1", &evidence), OracleOutcome::Fail);
    }

    // P2: two successful charges for one order.
    {
        let mut capture = load_capture("happy");
        duplicate_span(
            &mut capture,
            &|span| is_event(span, PAYMENT_CHARGE),
            "aabbccddeeff0011",
        );
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P2", &evidence), OracleOutcome::Fail);
    }

    // P3: an event after cancellation that is not a refund.
    {
        let mut capture = load_capture("happy");
        set_span_attribute(
            &mut capture,
            &|span| is_event(span, ORDER_SHIP),
            "sagashop.order.prev_state",
            "CANCELLED",
        );
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P3", &evidence), OracleOutcome::Fail);
    }

    // P5: two concurrent reserves with the same stock_after (no decrement).
    {
        let mut capture = load_capture("happy");
        duplicate_span(
            &mut capture,
            &|span| is_event(span, INVENTORY_RESERVE),
            "1122334455667788",
        );
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P5", &evidence), OracleOutcome::Fail);
    }

    // P7: a reserve whose order has no prior create.
    {
        let mut capture = load_capture("happy");
        set_span_attribute(
            &mut capture,
            &|span| is_event(span, INVENTORY_RESERVE),
            "sagashop.order.id",
            "no-create-order",
        );
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P7", &evidence), OracleOutcome::Fail);
    }
}

/// Test 6. P4/P6/P8 anchor silence (delete requirement Event, renumber) yields
/// an authoritative Violate (ADR-0033).
///
/// Deleting the requirement Event for one Binding empties the matching
/// requirement set. The required anchor-tier source stays observed (other
/// orders' Events keep it alive) and complete, so silence through its Watermark
/// settles the absence. P7 is excluded: it is an oracle check, not a
/// Core-evaluated PrecedenceSpecification.
#[test]
fn anchor_silence_authoritatively_violates() {
    // P4: delete the shipped order's payment.charge in hazards.
    {
        let mut capture = load_capture("hazards");
        let order = only_shipped_order(&capture);
        delete_span(&mut capture, &|span| {
            is_event_for_order(span, PAYMENT_CHARGE, &order)
        });
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
        assert_eq!(
            verdict.settlement_basis(),
            Some(SettlementBasisKind::WatermarkAnchoredViolation)
        );
        assert_eq!(verdict.source_tier_gap(), None);
    }

    // P6: delete a charged order's inventory.reserve in hazards.
    {
        let mut capture = load_capture("hazards");
        let charged = orders_with_event(&capture, ORDER_CHARGE);
        let reserved = orders_with_event(&capture, INVENTORY_RESERVE);
        let order = charged
            .iter()
            .find(|o| reserved.contains(o))
            .expect("an order has both a charge and a reserve")
            .clone();
        delete_span(&mut capture, &|span| {
            is_event_for_order(span, INVENTORY_RESERVE, &order)
        });
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&charge_requires_prior_reserve(), &evidence).expect("evaluation succeeds");
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
        assert_eq!(
            verdict.settlement_basis(),
            Some(SettlementBasisKind::WatermarkAnchoredViolation)
        );
        assert_eq!(verdict.source_tier_gap(), None);
    }

    // P8: delete the fulfilled order's payment.charge in hazards.
    {
        let mut capture = load_capture("hazards");
        let order = orders_with_event(&capture, ORDER_FULFILL)[0].clone();
        delete_span(&mut capture, &|span| {
            is_event_for_order(span, PAYMENT_CHARGE, &order)
        });
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&fulfill_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
        assert_eq!(
            verdict.settlement_basis(),
            Some(SettlementBasisKind::WatermarkAnchoredViolation)
        );
        assert_eq!(verdict.source_tier_gap(), None);
    }
}

// ---------------------------------------------------------------------------
// Tier B — Missing Evidence
// ---------------------------------------------------------------------------

/// Test 7. P4/P6/P8 missing required source (delete all spans of a required
/// source) yields a Provisional Inconclusive, never a silent pass.
///
/// ADR-0017: "never turn missing evidence into a pass."
#[test]
fn absent_required_source_is_provisional_inconclusive() {
    // P4: delete all sagashop-payment spans from happy.
    {
        let mut capture = load_capture("happy");
        delete_all_spans_of_service(&mut capture, PAYMENT_SERVICE);
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let order = only_shipped_order(&load_capture("happy"));
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
        assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
        assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
        assert_eq!(verdict.hot_path(), TierSettlement::Settled);
        assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    }

    // P6: delete all sagashop-inventory spans from happy.
    {
        let mut capture = load_capture("happy");
        delete_all_spans_of_service(&mut capture, INVENTORY_SERVICE);
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&charge_requires_prior_reserve(), &evidence).expect("evaluation succeeds");
        let order = orders_with_event(&load_capture("happy"), ORDER_CHARGE)[0].clone();
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
        assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
        assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
        assert_eq!(verdict.hot_path(), TierSettlement::Settled);
        assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    }

    // P8: delete all sagashop-payment spans from happy.
    {
        let mut capture = load_capture("happy");
        delete_all_spans_of_service(&mut capture, PAYMENT_SERVICE);
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&fulfill_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let order = orders_with_event(&load_capture("happy"), ORDER_FULFILL)[0].clone();
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
        assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
        assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
        assert_eq!(verdict.hot_path(), TierSettlement::Settled);
        assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    }
}

/// Test 8. P4/P6/P8 sequence gap (delete a non-last span, leaving the sequence
/// discontinuous) yields Inconclusive with PrefixUncertainty.
#[test]
fn sequence_gap_yields_prefix_uncertainty() {
    // P4: gap in sagashop-payment (hazards).
    {
        let mut capture = load_capture("hazards");
        delete_span_leaving_sequence_gap(&mut capture, &|span| {
            span.service == PAYMENT_SERVICE && span.source_sequence == 1
        });
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let order = only_shipped_order(&load_capture("hazards"));
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
        assert_eq!(
            verdict.inconclusive_reason(),
            Some(InconclusiveReason::PrefixUncertainty)
        );
        assert!(!verdict.gaps().is_empty());
    }

    // P6: gap in sagashop-inventory (hazards).
    {
        let mut capture = load_capture("hazards");
        delete_span_leaving_sequence_gap(&mut capture, &|span| {
            span.service == INVENTORY_SERVICE && span.source_sequence == 1
        });
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&charge_requires_prior_reserve(), &evidence).expect("evaluation succeeds");
        // At least one charge binding must be Inconclusive with prefix uncertainty.
        assert!(!verdicts.is_empty(), "P6 creates bindings in hazards");
        let any_inconclusive = verdicts.iter().any(|b| {
            b.verdict().outcome() == VerdictOutcome::Inconclusive
                && b.verdict().inconclusive_reason() == Some(InconclusiveReason::PrefixUncertainty)
        });
        assert!(
            any_inconclusive,
            "at least one P6 binding is Inconclusive with PrefixUncertainty"
        );
        // No binding should be Violate — the gap downgrades, it does not falsify.
        assert!(
            verdicts
                .iter()
                .all(|b| b.verdict().outcome() != VerdictOutcome::Violate),
            "a sequence gap must never produce a Violate"
        );
    }

    // P8: gap in sagashop-payment (hazards).
    {
        let mut capture = load_capture("hazards");
        delete_span_leaving_sequence_gap(&mut capture, &|span| {
            span.service == PAYMENT_SERVICE && span.source_sequence == 1
        });
        let evidence = adapt(&capture);
        let verdicts =
            evaluate(&fulfill_requires_prior_charge(), &evidence).expect("evaluation succeeds");
        let order = orders_with_event(&load_capture("hazards"), ORDER_FULFILL)[0].clone();
        let verdict = verdict_for_order(&verdicts, &order).verdict();
        assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
        assert_eq!(
            verdict.inconclusive_reason(),
            Some(InconclusiveReason::PrefixUncertainty)
        );
        assert!(!verdict.gaps().is_empty());
    }
}

/// Test 9. P1/P2/P3/P5/P7 missing evidence makes `oracle_check` return Pass
/// (vacuous truth), not Unevaluated.
#[test]
fn oracle_missing_evidence_is_vacuous_pass() {
    // P1: no ship events → Pass. The cancel capture ships nothing.
    assert_eq!(oracle_check("P1", &adapted("cancel")), OracleOutcome::Pass);

    // P2: no successful charges → Pass.
    {
        let mut capture = load_capture("cancel");
        delete_span(&mut capture, &|span| is_event(span, PAYMENT_CHARGE));
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P2", &evidence), OracleOutcome::Pass);
    }

    // P3: no event with prev_state=CANCELLED → Pass. The happy capture has none.
    assert_eq!(oracle_check("P3", &adapted("happy")), OracleOutcome::Pass);

    // P5: no reserves → Pass.
    {
        let mut capture = load_capture("cancel");
        delete_span(&mut capture, &|span| is_event(span, INVENTORY_RESERVE));
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P5", &evidence), OracleOutcome::Pass);
    }

    // P7: no reserves → Pass (vacuous: nothing to check).
    {
        let mut capture = load_capture("cancel");
        delete_span(&mut capture, &|span| is_event(span, INVENTORY_RESERVE));
        let evidence = adapt(&capture);
        assert_eq!(oracle_check("P7", &evidence), OracleOutcome::Pass);
    }
}

// ---------------------------------------------------------------------------
// Tier B — Concurrent Events and Late Arrivals
// ---------------------------------------------------------------------------

/// Test 10. Concurrent events (hazards H3: 15 concurrent reserves).
///
/// P5 detects `stock_after` reuse: all 15 h3-item reserves report
/// `stock_after = 9` and are pairwise concurrent, so the "no decrement at all"
/// condition fires. P7 detects missing creates: the h3-* orders have no
/// `order.create.requested` Events.
#[test]
fn concurrent_hazards_detected_by_oracle() {
    let evidence = adapted("hazards");

    // P5: 15 concurrent h3-item reserves, all stock_after=9.
    assert_eq!(
        oracle_check("P5", &evidence),
        OracleOutcome::Fail,
        "P5 must fail: 15 concurrent reserves reuse stock_after=9"
    );

    // P7: h3-* reserves have no matching order.create.
    assert_eq!(
        oracle_check("P7", &evidence),
        OracleOutcome::Fail,
        "P7 must fail: h3-* reserves have no prior order.create"
    );
}

/// Test 11. Late arrivals (reverse causal order) produce byte-identical
/// verdicts for P4, P6, P8.
#[test]
fn late_arrivals_produce_identical_verdicts() {
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for (id, spec) in all_precedence_specifications() {
            let baseline =
                encoded_verdicts(&evaluate(&spec, &evidence).expect("baseline evaluates"));
            let reversed = reordered(&evidence, |events| events.reverse());
            let verdicts = evaluate(&spec, &reversed).expect("reversed evaluates");
            assert_eq!(
                encoded_verdicts(&verdicts),
                baseline,
                "{scenario} {id}: reversed arrival order changed the Verdict"
            );
        }
    }
}

/// Test 12. Replaying a requirement Event is idempotent, down to the byte,
/// for P4, P6, P8.
#[test]
fn duplicate_delivery_is_idempotent() {
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for (id, spec) in all_precedence_specifications() {
            let baseline =
                encoded_verdicts(&evaluate(&spec, &evidence).expect("baseline evaluates"));

            // Find a requirement Event to replay.
            let Some(event) = evidence
                .events()
                .iter()
                .find(|e| spec.requirement().matches_kind(e))
            else {
                // No requirement Event in this scenario: nothing to replay,
                // and the baseline is already verified by Test 1.
                continue;
            };

            let mut events = evidence.events().to_vec();
            events.push(event.clone());
            let replayed = evidence.with_events(events);
            let verdicts = evaluate(&spec, &replayed).expect("replay evaluates");
            assert_eq!(
                encoded_verdicts(&verdicts),
                baseline,
                "{scenario} {id}: a replayed Event changed the Verdict"
            );
        }
    }
}
