//! The resumable-Core suite: stepping, the evidence contract, facts, and cost.
//!
//! Every test here runs on in-memory fixtures with no database, no network and
//! no clock read (ADR-0012's acceptance criterion 3, satisfied literally). The
//! cold-guard fixtures are recorded fact responses in exactly the sense
//! ADR-0012 means: a [`sentinel_core::facts::FactResponse`] echoes the query it
//! answers, so no response can be fabricated without a query the Core actually
//! issued.
//!
//! `tracer_bullet.rs` remains the #10 regression pin. Nothing here weakens it;
//! the tests below cover what #12 added.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod support;

use std::collections::BTreeSet;

use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::evidence::MonotonicityBreach;
use sentinel_core::facts::{
    ColdGuardId, FactQuery, FactRequest, FactResponse, GuardConclusion, WatermarkFloor,
};
use sentinel_core::slice::Binding;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::{
    InconclusiveReason, SequenceGap, SequenceNumber, SettlementBasisKind, SourceIdentity,
    SourceTier, TierSettlement, VerdictAuthority, VerdictOutcome,
};

use support::{
    adapt, adapted, add_shipped_orders, answer_all, declared_watermarks, delete_span, drive,
    encoded_verdicts, evaluate, event_inputs, inputs, is_event, load_capture,
    make_mutual_transition_cycle, only_event, only_shipped_order, required_watermarks,
    set_span_attribute, ship_requires_prior_charge, ship_requires_prior_charge_with_cold_guards,
    source, source_inputs, with_altered_slot, with_bounds, with_gaps, with_tier, ORDER_SHIP,
    PAYMENT_CHARGE, PAYMENT_SERVICE, SHIPPING_SERVICE,
};

/// Steps every input and returns each step's outcome alongside the evaluator.
fn step_all(evaluator: &mut Evaluator, inputs: Vec<Input>) -> Vec<Step> {
    inputs
        .into_iter()
        .map(|input| evaluator.step(input).expect("the fixture steps cleanly"))
        .collect()
}

/// Returns the batched request the drive produced, or asks for it now.
///
/// Emissions drain before requests, so a hot settlement is never swallowed by
/// the request it creates: the batch is issued by the first step that has
/// nothing new to emit. `resume(Vec::new())` is the driver's way of asking for
/// it without supplying new evidence.
fn request_after(evaluator: &mut Evaluator, steps: &[Step]) -> FactRequest {
    if let Some(Step::NeedsFacts(request)) = steps
        .iter()
        .find(|step| matches!(step, Step::NeedsFacts(_)))
    {
        return request.clone();
    }
    match evaluator
        .resume(Vec::new())
        .expect("an empty resume is legal")
    {
        Step::NeedsFacts(request) => request,
        other => panic!("expected NeedsFacts, got {other:?}"),
    }
}

fn settled_verdicts(step: &Step) -> &[BindingVerdict] {
    match step {
        Step::Settled(verdicts) => verdicts,
        other => panic!("expected Settled, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The batch path is a driver over the stepping API
// ---------------------------------------------------------------------------

/// `evaluate()` is not a second implementation; it is `step()` plus `seal()`.
#[test]
fn evaluate_equals_the_stepping_driver() {
    let specification = ship_requires_prior_charge();
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        let batch = evaluate(&specification, &evidence).expect("the batch path evaluates");
        let stepped = drive(&specification, inputs(&evidence)).expect("the stepper evaluates");
        assert_eq!(
            encoded_verdicts(&stepped),
            encoded_verdicts(&batch),
            "{scenario} must be byte-identical through both paths"
        );
    }

    // And for the derived fixtures, where the interesting envelopes live.
    let mut absent = load_capture("happy");
    delete_span(&mut absent, &|span| is_event(span, PAYMENT_CHARGE));
    let mut violated = load_capture("happy");
    set_span_attribute(
        &mut violated,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    for capture in [absent, violated] {
        let evidence = adapt(&capture);
        assert_eq!(
            encoded_verdicts(&drive(&specification, inputs(&evidence)).expect("stepper")),
            encoded_verdicts(&evaluate(&specification, &evidence).expect("batch")),
        );
    }
}

/// Nothing settles until the last required source's Watermark closes.
#[test]
fn step_is_pending_until_the_watermark_closes() {
    let evidence = adapted("happy");
    let specification = ship_requires_prior_charge();
    let mut evaluator = Evaluator::new(specification.clone());

    for step in step_all(&mut evaluator, event_inputs(&evidence)) {
        assert_eq!(
            step,
            Step::Pending,
            "no Event alone can settle a Binding: the evidence gate has no source record yet"
        );
    }

    // Sources arrive in identity order, so `sagashop-payment` is the last
    // required one and is what opens the gate.
    let mut settled = Vec::new();
    for step in step_all(&mut evaluator, source_inputs(&evidence)) {
        if let Step::Settled(verdicts) = step {
            settled.extend(verdicts);
        }
    }
    assert_eq!(settled.len(), 1, "exactly one Binding settles");
    assert_eq!(settled[0].verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        encoded_verdicts(&settled),
        encoded_verdicts(&evaluate(&specification, &evidence).expect("batch")),
        "the settled emission is the Verdict the batch path produces"
    );
}

/// A gated Binding has no settled conclusion at any frontier, so `seal()` owns it.
#[test]
fn seal_produces_a_verdict_for_gated_bindings() {
    let mut capture = load_capture("happy");
    delete_span(&mut capture, &|span| is_event(span, PAYMENT_CHARGE));
    let evidence = adapt(&capture);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for step in step_all(&mut evaluator, inputs(&evidence)) {
        assert_eq!(step, Step::Pending, "a gated Binding never settles");
    }

    let sealed = evaluator.seal().expect("sealing succeeds");
    assert_eq!(sealed.verdicts().len(), 1);
    let verdict = sealed.verdicts()[0].verdict();
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(verdict.cold_path(), TierSettlement::Pending);
}

// ---------------------------------------------------------------------------
// The evidence monotonicity contract
// ---------------------------------------------------------------------------

/// Clause (a): a completeness signal may not go backwards.
#[test]
fn regressing_complete_through_is_rejected() {
    let evidence = adapted("hazards");
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");
    let sample = only_event(
        &evidence,
        "sagashop.payment.charge.completed",
        &only_shipped_order(&load_capture("hazards")),
    );
    let boundary = record
        .completeness()
        .complete_through()
        .expect("the fixture declares a boundary")
        .value();

    // An open epoch first, so the closed-epoch clause is not what fires.
    let open = with_bounds(record, &sample, 0, Some(boundary), Some(boundary - 2));
    let regressed_complete = with_bounds(record, &sample, 0, Some(boundary), Some(boundary - 4));
    let regressed_emitted = with_bounds(record, &sample, 0, Some(boundary - 2), Some(boundary - 4));

    for (label, second, expected) in [
        (
            "complete_through",
            regressed_complete,
            MonotonicityBreach::RegressedCompleteThrough,
        ),
        (
            "last_emitted",
            regressed_emitted,
            MonotonicityBreach::RegressedLastEmitted,
        ),
    ] {
        let mut evaluator = Evaluator::new(ship_requires_prior_charge());
        evaluator
            .step(Input::source(open.clone()))
            .expect("the first signal is accepted");
        let error = evaluator
            .step(Input::source(second))
            .expect_err("a regressed signal is refused");
        match error {
            CoreError::NonMonotonicEvidence(breach) => {
                assert_eq!(breach.breach(), expected, "{label} regression");
                assert_eq!(breach.source(), &payment);
            }
            other => panic!("{label}: expected NonMonotonicEvidence, got {other:?}"),
        }
    }
}

/// Clause (a), L1: once closed, an epoch may not advance — only a new one may open.
///
/// Without this, a closed epoch silently reopens and flips a Verdict that was
/// already emitted: the Binding settles `Inconclusive` at a boundary of 4, the
/// source declares 6, slots 5 and 6 arrive carrying the matching requirement
/// Event, and the same Binding re-emits as `Satisfy`. There is no Amendment
/// record anywhere in `proto/` or `model/` to retract that with.
#[test]
fn advancing_a_closed_boundary_is_rejected() {
    let evidence = adapted("hazards");
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");
    let sample = only_event(
        &evidence,
        "sagashop.payment.charge.completed",
        &only_shipped_order(&load_capture("hazards")),
    );
    let boundary = record
        .completeness()
        .complete_through()
        .expect("the fixture declares a boundary")
        .value();

    let closed = with_bounds(record, &sample, 0, Some(boundary), Some(boundary));
    let advanced = with_bounds(record, &sample, 0, Some(boundary + 2), Some(boundary + 2));

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    evaluator
        .step(Input::source(closed.clone()))
        .expect("closing an epoch is accepted");
    let error = evaluator
        .step(Input::source(advanced.clone()))
        .expect_err("advancing a closed epoch is refused");
    match error {
        CoreError::NonMonotonicEvidence(breach) => {
            assert_eq!(breach.breach(), MonotonicityBreach::ReopenedClosedEpoch);
            assert_eq!(breach.epoch(), 0);
        }
        other => panic!("expected NonMonotonicEvidence, got {other:?}"),
    }

    // The intended escape hatch: a source with more to say opens a new epoch.
    // Without this half, "a closed epoch stays closed" could be implemented as
    // "a closed epoch never moves at all", which is wrong in the other
    // direction and would only surface when a real source rolls an epoch.
    let next_epoch = with_bounds(record, &sample, 1, Some(boundary + 2), Some(boundary + 2));
    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    evaluator
        .step(Input::source(closed))
        .expect("closing an epoch is accepted");
    evaluator
        .step(Input::source(next_epoch))
        .expect("a new SequenceEpoch is the escape hatch, and it is accepted");

    // And the trace L1 actually described, end to end: a Binding that settled
    // and was emitted at a closed boundary is what the advance would flip.
    // Deleting the shipped order's charge (with renumbering, so the source
    // stays contiguous and closed) leaves the Binding settling a
    // watermark-anchored Violate (ADR-0033). Slots beyond the boundary
    // are exactly where the matching charge would arrive.
    let mut capture = load_capture("hazards");
    let shipped = only_shipped_order(&capture);
    delete_span(&mut capture, &|span| {
        support::is_event_for_order(span, PAYMENT_CHARGE, &shipped)
    });
    let gapless = adapt(&capture);
    let payment_record = gapless
        .source(&payment)
        .expect("payment still supplies evidence for the other orders");
    let payment_sample = gapless
        .events()
        .iter()
        .find(|event| event.source() == payment)
        .expect("payment delivered Events")
        .clone();
    let closed_boundary = payment_record
        .completeness()
        .complete_through()
        .expect("the fixture declares a boundary")
        .value();

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    let steps = step_all(&mut evaluator, inputs(&gapless));
    let emitted: Vec<&BindingVerdict> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Settled(verdicts) => Some(verdicts.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(emitted.len(), 1, "the Binding settles and is emitted");
    assert_eq!(emitted[0].verdict().outcome(), VerdictOutcome::Violate);

    // Reopening that epoch is what would deliver the matching charge and flip
    // the Verdict already emitted above. It is refused.
    let reopened = with_bounds(
        payment_record,
        &payment_sample,
        0,
        Some(closed_boundary + 2),
        Some(closed_boundary + 2),
    );
    let error = evaluator
        .step(Input::source(reopened))
        .expect_err("reopening the epoch a settled Binding rests on is refused");
    assert!(matches!(
        error,
        CoreError::NonMonotonicEvidence(breach) if breach.breach() == MonotonicityBreach::ReopenedClosedEpoch
    ));
}

/// Clause (a): a source's tier may not be restated inside an epoch.
///
/// The completeness bounds are not the only monotone half of a source record.
/// `tier` feeds the anchor-candidate filter and both `source_tier_gap` filters,
/// so restating it flips an already-emitted `Violate` to `Inconclusive` — a
/// second `Step::Settled` for one Binding in one epoch with a different
/// outcome, and no Amendment record anywhere in `proto/` or `model/` to retract
/// the first with. Issue #12's own tier fix is what made `tier` a per-input
/// evidence field, so this is the clause that pays for that fix.
#[test]
fn restating_a_source_at_a_different_tier_is_rejected() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let evidence = adapt(&capture);
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let emitted: Vec<&BindingVerdict> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Settled(verdicts) => Some(verdicts.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(emitted.len(), 1);
    assert_eq!(
        emitted[0].verdict().outcome(),
        VerdictOutcome::Violate,
        "the anchor-tier payment source witnesses the absent successful charge"
    );

    // Same source, same epoch, identical bounds — only the tier moves.
    let demoted = with_tier(record, SourceTier::Contributing);
    let error = evaluator
        .step(Input::source(demoted))
        .expect_err("restating a settled source's tier is refused");
    match error {
        CoreError::NonMonotonicEvidence(breach) => {
            assert_eq!(breach.breach(), MonotonicityBreach::RedeclaredTier);
            assert_eq!(breach.source(), &payment);
        }
        other => panic!("expected NonMonotonicEvidence, got {other:?}"),
    }

    // The same restatement carried on a new SequenceEpoch is the escape hatch,
    // exactly as it is for the closed-epoch clause.
    let sample = only_event(
        &evidence,
        "sagashop.payment.charge.completed",
        &only_shipped_order(&load_capture("happy")),
    );
    let boundary = record
        .completeness()
        .complete_through()
        .expect("the fixture declares a boundary")
        .value();
    let rolled = with_tier(
        &with_bounds(record, &sample, 1, Some(boundary), Some(boundary)),
        SourceTier::Contributing,
    );
    evaluator
        .step(Input::source(rolled))
        .expect("a new SequenceEpoch may restate the deployment facts");
}

/// Clause (a): a source's gaps may not be restated inside an epoch either.
///
/// Milder symptom, same class: G2 shuts the gate so no second `Step::Settled`
/// is produced, but `seal()` would return `Inconclusive` for a Binding already
/// emitted as `Violate`.
#[test]
fn restating_a_source_with_new_gaps_is_rejected() {
    let evidence = adapted("happy");
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    step_all(&mut evaluator, inputs(&evidence));

    let regapped = with_gaps(record, vec![SequenceGap::new(0, 1, 1)]);
    let error = evaluator
        .step(Input::source(regapped))
        .expect_err("restating a source's gaps is refused");
    match error {
        CoreError::NonMonotonicEvidence(breach) => {
            assert_eq!(breach.breach(), MonotonicityBreach::RedeclaredGaps);
        }
        other => panic!("expected NonMonotonicEvidence, got {other:?}"),
    }
}

/// Clause (b): nothing may be delivered above a boundary the source closed.
#[test]
fn an_event_past_a_closed_boundary_is_rejected() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in source_inputs(&evidence) {
        evaluator.step(input).expect("source evidence is accepted");
    }
    let error = evaluator
        .step(Input::event(with_altered_slot(&charge, 9_999)))
        .expect_err("an Event above a closed boundary is refused");
    match error {
        CoreError::NonMonotonicEvidence(breach) => assert_eq!(
            breach.breach(),
            MonotonicityBreach::DeliveryAboveClosedBoundary
        ),
        other => panic!("expected NonMonotonicEvidence, got {other:?}"),
    }
}

/// Clause (b′): a delivery conflict is reported as a conflict, always.
///
/// This is an ordering *rule*, not an accident of the Events-then-Sources
/// driver. Under a Sources-first feed the boundary is already known, so clause
/// (b) would happily reject the slot-9999 push — but `Frontier::step`'s
/// identity-conflict classification runs first, and `delivery_conflicts_are_rejected`
/// pins `CoreError::Delivery` for exactly this input.
#[test]
fn a_delivery_conflict_outranks_the_boundary_check() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in source_inputs(&evidence) {
        evaluator.step(input).expect("source evidence is accepted");
    }
    evaluator
        .step(Input::event(charge.clone()))
        .expect("the real Event is delivered");
    let error = evaluator
        .step(Input::event(with_altered_slot(&charge, 9_999)))
        .expect_err("the same identity in another slot conflicts");
    assert!(
        matches!(error, CoreError::Delivery(_)),
        "a conflicting delivery is a conflict, never NonMonotonicEvidence: got {error:?}"
    );
}

/// Clause (c): the gate stays shut until the Core holds every slot below the boundary.
///
/// The interior case is the one that matters. A check of the *boundary* alone —
/// "max delivered slot equals `complete_through`" — passes here while the Core
/// is missing an interior slot, because `gaps` lives in source-emission space
/// and cannot see a receipt hole. The test asserts both halves explicitly, so
/// the cardinality-versus-boundary distinction is pinned rather than implied.
#[test]
fn incomplete_delivery_to_the_watermark_does_not_settle() {
    let evidence = adapted("hazards");
    let specification = ship_requires_prior_charge();
    let payment = source(PAYMENT_SERVICE);
    let boundary = evidence
        .source(&payment)
        .expect("payment supplied evidence")
        .completeness()
        .complete_through()
        .expect("the fixture declares a boundary")
        .value();

    // Withhold an *interior* payment slot, keeping the top of the range.
    let withheld = evidence
        .events()
        .iter()
        .find(|event| {
            event.source() == payment && event.delivery().sequence().value() == boundary - 2
        })
        .expect("payment delivered an interior slot")
        .clone();

    let mut evaluator = Evaluator::new(specification.clone());
    for event in evidence.events() {
        if event.id() == withheld.id() {
            continue;
        }
        evaluator
            .step(Input::event(event.clone()))
            .expect("delivery succeeds");
    }
    for input in source_inputs(&evidence) {
        let step = evaluator.step(input).expect("source evidence is accepted");
        assert_eq!(
            step,
            Step::Pending,
            "an interior receipt hole must keep the gate shut"
        );
    }

    let ledger = evaluator.ledger();
    assert_eq!(
        ledger.highest_delivered_slot(&payment, 0),
        boundary,
        "boundary equality holds: the Core reached the top of the declared range"
    );
    assert!(
        ledger.delivered_slot_count(&payment, 0) < boundary,
        "cardinality does not: an interior slot is missing"
    );
    assert!(
        !ledger.covers_watermark(&payment, 0, boundary),
        "clause (c) is cardinality, not boundary equality"
    );

    let gated = evaluator.clone().seal().expect("sealing succeeds");
    let verdict = gated.verdicts()[0].verdict();
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert!(
        verdict
            .uncertainty()
            .iter()
            .any(|entry| entry.detail.contains("G5")),
        "the envelope names the disjunct that closed the gate"
    );

    // Deliver the withheld Event and the same Binding settles, to the Verdict
    // the batch path produces from the whole evidence set.
    let settled = evaluator
        .step(Input::event(withheld))
        .expect("the withheld Event delivers");
    assert_eq!(
        encoded_verdicts(settled_verdicts(&settled)),
        encoded_verdicts(&evaluate(&specification, &evidence).expect("batch")),
    );
}

/// Two prefixes reaching the same frontier settle identically (ADR-0020).
#[test]
fn two_prefixes_reaching_the_same_frontier_settle_identically() {
    let evidence = adapted("hazards");
    let specification = ship_requires_prior_charge();

    let forward = drive(&specification, inputs(&evidence)).expect("forward order");
    let mut reversed_events: Vec<Input> = event_inputs(&evidence);
    reversed_events.reverse();
    let reversed = drive(
        &specification,
        reversed_events
            .into_iter()
            .chain(source_inputs(&evidence))
            .collect(),
    )
    .expect("reverse order");

    assert_eq!(
        encoded_verdicts(&forward),
        encoded_verdicts(&reversed),
        "the same frontier and the same ledger settle the same Bindings identically"
    );
}

/// Verdicts do not depend on whether source evidence interleaves with Events.
#[test]
fn verdicts_are_invariant_across_input_interleaving() {
    let evidence = adapted("happy");
    let specification = ship_requires_prior_charge();

    let events_first = drive(&specification, inputs(&evidence)).expect("events first");
    let sources_first = drive(
        &specification,
        source_inputs(&evidence)
            .into_iter()
            .chain(event_inputs(&evidence))
            .collect(),
    )
    .expect("sources first");

    let mut interleaved = Vec::new();
    let mut sources = source_inputs(&evidence).into_iter();
    for (index, event) in event_inputs(&evidence).into_iter().enumerate() {
        interleaved.push(event);
        if index % 4 == 0 {
            if let Some(record) = sources.next() {
                interleaved.push(record);
            }
        }
    }
    interleaved.extend(sources);
    let mixed = drive(&specification, interleaved).expect("interleaved");

    assert_eq!(encoded_verdicts(&events_first), encoded_verdicts(&mixed));
    assert_eq!(
        encoded_verdicts(&events_first),
        encoded_verdicts(&sources_first)
    );
}

/// A contradictory cycle blocks every emission the moment it appears.
///
/// Detection is eager; only the *raise* waits for `seal()`, so that a later
/// delivery conflicting on identity or slot is still reported as the conflict
/// it is rather than masked by a cycle that conflict itself created.
#[test]
fn a_causal_cycle_blocks_every_emission_before_it_is_raised() {
    let mut capture = load_capture("happy");
    make_mutual_transition_cycle(
        &mut capture,
        &|span| is_event(span, "order.charge"),
        &|span| is_event(span, "order.fulfill"),
    );
    let evidence = adapt(&capture);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for step in step_all(&mut evaluator, inputs(&evidence)) {
        assert_eq!(
            step,
            Step::Pending,
            "no partial order exists, so no frontier valuation does either"
        );
    }
    assert!(
        evaluator.causal_contradiction().is_some(),
        "the contradiction is observable immediately, not only at seal()"
    );
    assert!(matches!(evaluator.seal(), Err(CoreError::Causality(_)),));
}

// ---------------------------------------------------------------------------
// The cold-path fact protocol
// ---------------------------------------------------------------------------

/// Every outstanding `(guard, Binding)` pair goes into one round trip.
#[test]
fn needs_facts_batches_every_outstanding_query_into_one_round_trip() {
    let mut capture = load_capture("happy");
    add_shipped_orders(
        &mut capture,
        &[
            ("00112233445566aa", "batch-order-a"),
            ("00112233445566bb", "batch-order-b"),
        ],
    );
    let evidence = adapt(&capture);
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));

    let request = request_after(&mut evaluator, &steps);
    assert_eq!(
        request.queries().len(),
        6,
        "three Bindings times two guards, batched into one request"
    );
    let bindings: BTreeSet<&Binding> = request.queries().iter().map(FactQuery::binding).collect();
    let guards: BTreeSet<&ColdGuardId> = request.queries().iter().map(FactQuery::guard).collect();
    assert_eq!(bindings.len(), 3);
    assert_eq!(guards.len(), 2);
    assert_eq!(
        evaluator.cost().round_trips(),
        1,
        "ADR-0012 requires batching, not one round trip per Event"
    );
}

/// Outstanding queries never block ingestion; they hold the Verdict Provisional.
///
/// This is the deadlock regression test. A `step()` guard that refused input
/// while a query was open would compose with "a behind-floor answer re-issues"
/// into a permanent refusal — the hot path blocked on the cold path, which is
/// exactly the latency coupling ADR-0005 and ADR-0012 exist to forbid.
#[test]
fn stepping_continues_while_queries_are_outstanding() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let request = request_after(&mut evaluator, &steps);
    assert!(!request.queries().is_empty());

    // Ingestion continues with queries wide open, and re-delivering an already
    // delivered Event stays idempotent rather than becoming a mode error.
    for event in evidence.events() {
        evaluator
            .step(Input::event(event.clone()))
            .expect("ingestion continues while queries are outstanding");
    }
    assert_eq!(evaluator.outstanding().len(), request.queries().len());

    let sealed = evaluator.seal().expect("sealing succeeds");
    for binding in sealed.verdicts() {
        assert_eq!(
            binding.verdict().authority(),
            VerdictAuthority::Provisional,
            "an outstanding guard holds the Verdict provisional"
        );
        assert_eq!(binding.verdict().cold_path(), TierSettlement::Pending);
    }
}

/// An unanswered guard is Provisional, cold PENDING, with empty cold Watermarks.
#[test]
fn an_unanswered_guard_is_provisional_with_cold_pending_and_empty_cold_watermarks() {
    let evidence = adapted("happy");
    let verdicts = drive(
        &ship_requires_prior_charge_with_cold_guards(),
        inputs(&evidence),
    )
    .expect("the guarded fixture seals");
    let verdict = verdicts[0].verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Satisfy);
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(verdict.hot_path(), TierSettlement::Settled);
    assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    assert!(
        verdict.cold_settlement_watermarks().is_empty(),
        "the model requires a Pending cold path to record no settlement at all"
    );
    assert_eq!(
        verdict.settlement_basis(),
        None,
        "a Provisional Verdict carries no settlement basis"
    );
}

/// An answer at or after the floor settles the cold path, and upgrades authority.
#[test]
fn resume_at_or_after_the_floor_settles_cold() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());

    let steps = step_all(&mut evaluator, inputs(&evidence));
    let provisional: Vec<BindingVerdict> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Settled(verdicts) => Some(verdicts.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        provisional.len(),
        1,
        "the first emission is at hot settlement"
    );
    assert_eq!(
        provisional[0].verdict().authority(),
        VerdictAuthority::Provisional
    );

    let request = request_after(&mut evaluator, &steps);
    let authoritative = evaluator
        .resume(answer_all(&request, &required))
        .expect("recorded responses apply");
    let settled = settled_verdicts(&authoritative);
    assert_eq!(
        settled.len(),
        1,
        "the second emission is the authority upgrade"
    );

    let first = provisional[0].verdict();
    let second = settled[0].verdict();
    assert_eq!(second.authority(), VerdictAuthority::Authoritative);
    assert_eq!(second.cold_path(), TierSettlement::Settled);
    assert_eq!(
        second
            .cold_settlement_watermarks()
            .keys()
            .collect::<BTreeSet<_>>(),
        required.keys().collect::<BTreeSet<_>>(),
        "an Authoritative SETTLED cold path names exactly the required source set"
    );

    // At most two emissions, and the second is not a retraction: the outcome
    // cannot move, only the tier fields can.
    assert_eq!(first.outcome(), second.outcome());
    assert_eq!(first.inconclusive_reason(), second.inconclusive_reason());
    assert_eq!(first.cold_path(), TierSettlement::Pending);

    // A settled query is never asked again.
    assert!(evaluator.outstanding().is_empty());
    assert!(matches!(
        evaluator
            .resume(Vec::new())
            .expect("an empty resume is legal"),
        Step::Pending
    ));
}

/// A settling answer retires its query; a behind-floor answer does not.
#[test]
fn a_behind_floor_answer_is_reissued() {
    let evidence = adapted("happy");
    let mut behind = required_watermarks(&evidence);
    for point in behind.values_mut() {
        *point = (point.0, SequenceNumber::new(point.1.value() - 1));
    }

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let first = request_after(&mut evaluator, &steps);

    let step = evaluator
        .resume(answer_all(&first, &behind))
        .expect("a behind-floor answer is accepted, not rejected");
    assert!(
        matches!(step, Step::NeedsFacts(_)),
        "the cold path lags by design, so a behind-floor answer means ask again"
    );
    assert_eq!(evaluator.outstanding().len(), first.queries().len());
    assert_eq!(evaluator.cost().round_trips(), 2);
    assert_eq!(
        evaluator.cost().queries_issued(),
        2 * u32::try_from(first.queries().len()).expect("the fixture issues few queries"),
        "queries_issued counts re-issues: a guard asked twice cost two requests"
    );

    // The Verdict stays provisional throughout.
    let sealed = evaluator.seal().expect("sealing succeeds");
    assert_eq!(
        sealed.verdicts()[0].verdict().authority(),
        VerdictAuthority::Provisional
    );
    assert_eq!(
        sealed.verdicts()[0].cost().round_trip_indices(),
        &BTreeSet::from([0, 1]),
        "the re-issue is attributed to a second round trip"
    );
}

/// A superset answer settles, and the envelope carries the required set exactly.
#[test]
fn a_response_naming_extra_sources_settles_with_the_union_restricted_to_required() {
    let evidence = adapted("happy");
    let generous = declared_watermarks(
        &evidence,
        &[support::ORDER_SERVICE, PAYMENT_SERVICE, SHIPPING_SERVICE],
    );
    let required = required_watermarks(&evidence);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let request = request_after(&mut evaluator, &steps);
    let step = evaluator
        .resume(answer_all(&request, &generous))
        .expect("a superset answer is not a Core bug");
    let verdict = settled_verdicts(&step)[0].verdict();

    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(verdict.cold_path(), TierSettlement::Settled);
    assert_eq!(
        verdict
            .cold_settlement_watermarks()
            .keys()
            .cloned()
            .collect::<BTreeSet<SourceIdentity>>(),
        required
            .keys()
            .cloned()
            .collect::<BTreeSet<SourceIdentity>>(),
        "the union is restricted to the required set before it reaches the envelope"
    );
    assert!(!verdict
        .cold_settlement_watermarks()
        .contains_key(&source(SHIPPING_SERVICE)));
}

/// A subset answer degrades; it never errors and it never settles.
#[test]
fn a_response_naming_a_subset_of_required_sources_does_not_settle_the_cold_path() {
    let evidence = adapted("happy");
    let partial = declared_watermarks(&evidence, &[support::ORDER_SERVICE]);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let request = request_after(&mut evaluator, &steps);
    let step = evaluator
        .resume(answer_all(&request, &partial))
        .expect("a subset answer degrades rather than erroring");
    assert!(
        matches!(step, Step::NeedsFacts(_)),
        "the query stays outstanding and re-issues"
    );

    let sealed = evaluator.seal().expect("sealing succeeds");
    let verdict = sealed.verdicts()[0].verdict();
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    assert!(verdict.cold_settlement_watermarks().is_empty());
}

/// The only malformed input: a response echoing a query nobody issued.
#[test]
fn a_response_echoing_a_never_issued_query_is_rejected() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let _ = request_after(&mut evaluator, &steps);

    let fabricated = FactQuery::new(
        ColdGuardId::new("sagashop.p4.invented-guard", "v1"),
        Binding::new("invented-order"),
        WatermarkFloor::default(),
    );
    let error = evaluator
        .resume(vec![FactResponse::new(
            fabricated,
            GuardConclusion::Holds,
            required_watermarks(&evidence),
        )])
        .expect_err("a fabricated response is refused");
    assert!(matches!(error, CoreError::FactResponseMismatch { .. }));
}

/// Applying the same settling answer twice changes nothing.
#[test]
fn a_duplicate_settling_response_is_idempotent() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let request = request_after(&mut evaluator, &steps);
    let responses = answer_all(&request, &required);

    let first = evaluator
        .resume(responses.clone())
        .expect("the first application settles");
    let once = encoded_verdicts(settled_verdicts(&first));

    let second = evaluator
        .resume(responses)
        .expect("a replayed response is accepted");
    assert!(
        matches!(second, Step::Pending),
        "a replayed settling answer emits nothing new"
    );
    let sealed = evaluator.seal().expect("sealing succeeds");
    assert_eq!(encoded_verdicts(sealed.verdicts()), once);
}

/// Verdicts do not depend on how fact responses were batched.
#[test]
fn verdicts_are_invariant_across_fact_response_batching() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let specification = ship_requires_prior_charge_with_cold_guards();

    let all_at_once = {
        let mut evaluator = Evaluator::new(specification.clone());
        let steps = step_all(&mut evaluator, inputs(&evidence));
        let request = request_after(&mut evaluator, &steps);
        evaluator
            .resume(answer_all(&request, &required))
            .expect("one batch");
        evaluator.seal().expect("sealing succeeds")
    };

    let one_at_a_time = {
        let mut evaluator = Evaluator::new(specification);
        let steps = step_all(&mut evaluator, inputs(&evidence));
        let request = request_after(&mut evaluator, &steps);
        for response in answer_all(&request, &required) {
            evaluator.resume(vec![response]).expect("one response");
        }
        evaluator.seal().expect("sealing succeeds")
    };

    assert_eq!(
        encoded_verdicts(all_at_once.verdicts()),
        encoded_verdicts(one_at_a_time.verdicts()),
    );
}

/// The batch path has no fact supplier, so it refuses rather than dropping a guard.
#[test]
fn evaluate_refuses_a_specification_with_cold_guards() {
    let evidence = adapted("happy");
    let error = evaluate(&ship_requires_prior_charge_with_cold_guards(), &evidence)
        .expect_err("the batch path cannot answer a cold guard");
    match error {
        CoreError::FactsRequired { guards } => assert_eq!(guards.len(), 2),
        other => panic!("expected FactsRequired, got {other:?}"),
    }
}

/// The one Authoritative + cold SETTLED + Violate path, exercised end to end.
///
/// This is the combination the model is hardest on: `validate_decisive_event`
/// requires every witness to be settled under `cold_settlement_watermarks` once
/// those are non-empty, so an honest `cold = SETTLED` makes a hot witness
/// *harder* to justify than `NOT_REQUIRED` would (admitted limitation 5). It is
/// also the only path that would catch a regression in the settlement-basis
/// gating, so it is pinned rather than left to inspection.
#[test]
fn a_settled_cold_path_still_admits_a_decisive_violation() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let evidence = adapt(&capture);
    let required = required_watermarks(&evidence);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let request = request_after(&mut evaluator, &steps);
    let step = evaluator
        .resume(answer_all(&request, &required))
        .expect("recorded responses apply without a CoreError::Verdict");
    let verdict = settled_verdicts(&step)[0].verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(verdict.cold_path(), TierSettlement::Settled);
    assert_eq!(
        verdict.settlement_basis(),
        Some(SettlementBasisKind::DecisiveViolation),
        "an Authoritative Violate carries a decisive basis even with the cold path settled"
    );
    assert!(!verdict.decisive_witness_event_ids().is_empty());
    assert_eq!(
        verdict
            .cold_settlement_watermarks()
            .keys()
            .cloned()
            .collect::<BTreeSet<SourceIdentity>>(),
        required
            .keys()
            .cloned()
            .collect::<BTreeSet<SourceIdentity>>(),
    );
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

/// Fact requests per Verdict are countable (ADR-0012), and zero when guard-free.
#[test]
fn fact_requests_per_verdict_are_countable() {
    let evidence = adapted("happy");

    let guard_free = {
        let mut evaluator = Evaluator::new(ship_requires_prior_charge());
        step_all(&mut evaluator, inputs(&evidence));
        evaluator.seal().expect("sealing succeeds")
    };
    assert_eq!(guard_free.cost().round_trips(), 0);
    assert_eq!(guard_free.cost().queries_issued(), 0);
    for binding in guard_free.verdicts() {
        assert_eq!(binding.cost().queries_issued(), 0);
        assert_eq!(binding.cost().responses_applied(), 0);
        assert!(binding.cost().round_trip_indices().is_empty());
    }

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(&evidence));
    let request = request_after(&mut evaluator, &steps);
    evaluator
        .resume(answer_all(&request, &required_watermarks(&evidence)))
        .expect("recorded responses apply");
    let sealed = evaluator.seal().expect("sealing succeeds");

    assert_eq!(sealed.cost().round_trips(), 1);
    assert_eq!(sealed.cost().queries_issued(), 2);
    assert_eq!(sealed.cost().responses_applied(), 2);
    let binding = &sealed.verdicts()[0];
    assert_eq!(binding.cost().queries_issued(), 2, "one query per guard");
    assert_eq!(binding.cost().responses_applied(), 2);
    assert_eq!(binding.cost().round_trip_indices(), &BTreeSet::from([0]));
}

// ---------------------------------------------------------------------------
// The notes-for-#12 fixes
// ---------------------------------------------------------------------------

/// Tier is a property of the deployed source, not of the Specification (ADR-0022).
#[test]
fn source_tier_is_read_from_evidence_not_the_specification() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let evidence = adapt(&capture);
    let specification = ship_requires_prior_charge();
    let payment = source(PAYMENT_SERVICE);

    // Control: with the evidence placing payment at anchor tier, this settles.
    let control = evaluate(&specification, &evidence).expect("the control evaluates");
    assert_eq!(control[0].verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(control[0].verdict().source_tier_gap(), None);

    // Demote the source in the *evidence* and the Violate must degrade, because
    // an authoritative watermark-anchored Violate requires an *anchor-tier*
    // silent source (ADR-0033). A contributing-tier source cannot supply that.
    let demoted = with_tier(
        evidence
            .source(&payment)
            .expect("payment supplied evidence"),
        SourceTier::Contributing,
    );
    let mut steps: Vec<Input> = event_inputs(&evidence);
    for record in evidence.sources().values() {
        steps.push(Input::source(if record.source() == &payment {
            demoted.clone()
        } else {
            record.clone()
        }));
    }
    let verdicts = drive(&specification, steps).expect("the demoted fixture seals");
    let verdict = verdicts[0].verdict();
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    assert!(
        verdict.source_tier_gap().is_some(),
        "with no anchor-tier source the conclusion rests on no anchor Event"
    );
    assert!(
        verdict.anchored_event_ids()[&payment].is_empty(),
        "a contributing source anchors nothing"
    );

    // And the converse: changing only the Specification's declared minimum tier
    // changes nothing the Verdict records.
    let relaxed = {
        let mut required = BTreeSet::new();
        required.insert(sentinel_core::specification::RequiredSource::new(
            source(support::ORDER_SERVICE),
            SourceTier::Contributing,
        ));
        required.insert(sentinel_core::specification::RequiredSource::new(
            payment.clone(),
            SourceTier::Contributing,
        ));
        sentinel_core::specification::PrecedenceSpecification::new(
            specification.identity().clone(),
            specification.binding_attribute().clone(),
            specification.trigger().clone(),
            specification.requirement().clone(),
            required,
            specification.schema_version().clone(),
        )
    };
    let unchanged = evaluate(&relaxed, &evidence).expect("the relaxed Specification evaluates");
    assert_eq!(unchanged[0].verdict().outcome(), VerdictOutcome::Violate);
    assert!(
        !unchanged[0].verdict().anchored_event_ids()[&payment].is_empty(),
        "the recorded tier comes from the evidence, whatever the Specification declares: \
         payment still anchors even though the Specification now asks only for Contributing"
    );
    assert_eq!(unchanged[0].verdict().source_tier_gap(), None);
}

/// G3 is independent of G2: an unhealthy source need have no sequence gap.
#[test]
fn unhealthy_source_is_independent_of_a_sequence_gap() {
    let evidence = adapted("happy");
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");
    let order = only_shipped_order(&load_capture("happy"));
    let sample = only_event(&evidence, "sagashop.payment.charge.completed", &order);

    // The signal claims one fewer emission than the Core actually received,
    // with no gap declared at all. No truthful source produces this.
    let contradicting = with_bounds(record, &sample, 0, Some(1), Some(1));

    let mut steps: Vec<Input> = event_inputs(&evidence);
    for source_record in evidence.sources().values() {
        steps.push(Input::source(if source_record.source() == &payment {
            contradicting.clone()
        } else {
            source_record.clone()
        }));
    }
    let verdicts = drive(&ship_requires_prior_charge(), steps).expect("the fixture seals");
    let verdict = verdicts[0].verdict();

    assert!(
        verdict.unhealthy_sources().contains(&payment),
        "the completeness claim contradicts what the source delivered"
    );
    assert!(
        verdict.gaps().is_empty(),
        "G3 fired with no G2 gap present: the two disjuncts are independent"
    );
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
}

/// The determinism corpus, extended to the mutated fixtures (ADR-0020).
#[test]
fn verdicts_are_invariant_across_arrival_orders_for_mutated_fixtures() {
    let mut violated = load_capture("happy");
    set_span_attribute(
        &mut violated,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let mut absent = load_capture("happy");
    delete_span(&mut absent, &|span| is_event(span, PAYMENT_CHARGE));
    let mut isolated = load_capture("happy");
    support::remove_span_attribute(
        &mut isolated,
        &|span| is_event(span, ORDER_SHIP),
        "sagashop.order.prev_state",
    );

    let specification = ship_requires_prior_charge();
    for (label, capture) in [
        ("injected violation", violated),
        ("absent required source", absent),
        ("observationally uncertain", isolated),
    ] {
        let evidence = adapt(&capture);
        let baseline = encoded_verdicts(&evaluate(&specification, &evidence).expect("baseline"));
        for seed in 0..16_u64 {
            let permuted = support::reordered(&evidence, |events| shuffle(events, seed));
            assert_eq!(
                encoded_verdicts(&evaluate(&specification, &permuted).expect("permuted")),
                baseline,
                "{label}: arrival order {seed} changed the Verdict"
            );
        }
    }
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
