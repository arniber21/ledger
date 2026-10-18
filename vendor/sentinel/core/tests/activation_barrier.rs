//! A Specification warming behind an activation barrier produces no authoritative
//! Verdict, and keeps producing Verdicts.
//!
//! ADR-0017: a new Specification "starts in a warming Shadow state. It cannot produce
//! authoritative Verdicts until all required sources acknowledge the epoch and the
//! activation Watermark passes." This file pins both halves of that sentence and the two
//! things it does **not** say.
//!
//! # The fixture isolates the barrier and nothing else
//!
//! Every test drives the unmodified `happy` capture through
//! [`support::ship_requires_prior_charge`], whose two required sources clear every other
//! gate: complete Evidence Coverage, contiguous delivery, closed epochs, an anchor-tier
//! witness, no cold guard. [`support::activation_barrier`] then builds a barrier whose
//! per-source Watermarks are copied from the evidence those sources actually declared, so
//! the **only** variable across the whole file is the target Filter epoch. A barrier at
//! the epoch the sources acknowledged clears; one epoch higher is pending, for exactly the
//! reason a real rollout is pending — sources still shipping under a stale Event Filter.
//!
//! # The two things ADR-0017 does not say
//!
//! **It does not say a warming Specification stops emitting.** Closing the evidence gate
//! for a pre-activation source would make `step`/`resume` return `Step::Pending` for the
//! entire warming period and emit only from `seal()` — and `Shadow` exists precisely to
//! record Verdicts. [`a_pending_barrier_still_settles_from_step_not_only_from_seal`] is
//! the regression that pins it.
//!
//! **It does not say a warming Specification cannot Violate.** "Missing evidence is never
//! a pass" is asymmetric: it governs passes, and a Violate is the other direction — an
//! over-call, not a silent pass. ADR-0017's own "Shadow **or** Inconclusive" is a
//! disjunction, and `CONTEXT.md` makes Shadow the mechanism for *measuring* false-positive
//! rate before Promotion. A barrier-induced false Violate **is** the false positive Shadow
//! exists to surface, so suppressing it would delete the measurement rather than make
//! anything safer. [`a_warming_specification_still_emits_a_provisional_violate`] pins the
//! trade so it cannot change by accident.

#![allow(clippy::doc_markdown)] // Product names (SagaShop) read as prose here.

mod support;

use std::collections::BTreeSet;

use sentinel_compile::{compile_filter, FilterSource};
use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::evidence::MonotonicityBreach;
use sentinel_core::specification::PrecedenceSpecification;
use sentinel_core::CoreError;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::wire::probe::v1 as raw_probe;
use sentinel_model::{
    aggregate_unresolved, ActivationBarrier, ActivationBarrierStatus, ActivationWatermark,
    DecideRequestError, FilterEpoch, InconclusiveReason, SequenceNumber, SettlementBasisKind,
    TierSettlement, Verdict, VerdictAuthority, VerdictOutcome, BARRIER_PENDING_DETAIL_PREFIX,
};
use support::{
    activation_barrier, adapt, adapted, encoded_verdicts, event_inputs, inputs, is_event,
    load_capture, only_event, only_shipped_order, set_span_attribute, ship_requires_prior_charge,
    source, with_filter_epoch, ORDER_SERVICE, PAYMENT_CHARGE, PAYMENT_SERVICE,
};

/// The Filter epoch the SagaShop Adapter stamps on every completeness signal.
///
/// Read off the Adapter's own constant rather than hardcoded, so a fixture change that
/// moves it makes these tests move with it instead of silently testing nothing.
const ACKNOWLEDGED_EPOCH: u64 = sentinel_ingest::sagashop::FILTER_EPOCH;

/// One epoch past what the sources acknowledged: the barrier a rollout is warming behind.
const TARGET_EPOCH: u64 = ACKNOWLEDGED_EPOCH + 1;

/// Drives a batch through the stepping API and seals it, returning the one Verdict.
///
/// # Panics
///
/// Panics unless exactly one Binding was produced, which the `happy` capture guarantees:
/// it ships exactly one order.
fn sole_verdict(specification: &PrecedenceSpecification, evidence: &ObservedEvidence) -> Verdict {
    let verdicts = support::drive(specification, inputs(evidence)).expect("evaluation succeeds");
    assert_eq!(
        verdicts.len(),
        1,
        "the happy capture ships exactly one order, so exactly one Binding"
    );
    verdicts[0].verdict().clone()
}

/// Returns one Event the named source delivered, for rebuilding its evidence.
///
/// `SourceCompletenessSignal` exposes no accessor for the receiver-owned `SourceAssurance`,
/// so rebuilding a source's record needs a sample Event from the same source; any of them
/// carries the identical one.
///
/// # Panics
///
/// Panics when the source delivered no Event, which the `happy` capture forbids.
fn sample_event<'a>(
    evidence: &'a ObservedEvidence,
    identity: &sentinel_model::SourceIdentity,
) -> &'a sentinel_model::Event {
    evidence
        .events()
        .iter()
        .find(|event| event.source() == *identity)
        .expect("a declaring source delivered at least one Event")
}

/// A Specification warming behind a barrier the evidence cannot clear.
fn warming() -> PrecedenceSpecification {
    ship_requires_prior_charge()
        .with_activation_barrier(activation_barrier(&adapted("happy"), TARGET_EPOCH))
}

/// The same Specification behind a barrier the evidence does clear.
fn activated() -> PrecedenceSpecification {
    ship_requires_prior_charge()
        .with_activation_barrier(activation_barrier(&adapted("happy"), ACKNOWLEDGED_EPOCH))
}

/// A warming Specification still settles from `step`, not only from `seal()`.
///
/// The regression that guards the design decision: the evidence gate is **not** closed for
/// a pre-activation source. Closing it would satisfy "no authoritative Verdict before the
/// barrier passes" by making a warming Specification produce no streamed Verdict at all,
/// which is the opposite of what Shadow is for — `CONTEXT.md` defines Shadow as the state
/// in which "Verdicts [are] recorded". `resumable.rs`'s
/// `a_gated_binding_never_settles_until_seal` is the shape this test exists to stay clear
/// of.
#[test]
fn a_pending_barrier_still_settles_from_step_not_only_from_seal() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(warming());

    let mut streamed = Vec::new();
    for input in inputs(&evidence) {
        match evaluator.step(input).expect("stepping succeeds") {
            Step::Settled(verdicts) => streamed.extend(verdicts),
            Step::Pending => {}
            other => panic!("the fixture declares no cold guard, yet produced {other:?}"),
        }
    }

    assert!(
        !streamed.is_empty(),
        "a warming Specification records Verdicts; it does not go silent until seal()"
    );
    for verdict in &streamed {
        assert_eq!(
            verdict.verdict().authority(),
            VerdictAuthority::Provisional,
            "nothing streamed from a warming Specification may be authoritative"
        );
    }
}

/// The cap: complete evidence, every other gate clear, and still not authoritative.
///
/// The happy path is guard-free, so `cold_path` stays `NOT_REQUIRED`. The barrier
/// does not empty cold watermarks.
#[test]
fn a_warming_specification_produces_no_authoritative_verdict() {
    let verdict = sole_verdict(&warming(), &adapted("happy"));

    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(
        verdict.activation_barrier(),
        ActivationBarrierStatus::Pending
    );
    assert_eq!(verdict.cold_path(), TierSettlement::NotRequired);
    assert_eq!(
        verdict.target_filter_epoch(),
        Some(FilterEpoch::new(TARGET_EPOCH))
    );
    assert_eq!(
        verdict.settlement_basis(),
        None,
        "the model admits no settlement basis on a Provisional Verdict"
    );
}

/// The pending barrier names every source holding it, and why.
///
/// The disclosure rides in `AssuranceEnvelope.uncertainty` rather than in the evidence
/// gate's own audit strings, because the gate is untouched. It folds into the single
/// `PREFIX_UNCERTAINTY` entry the model permits per reason.
#[test]
fn the_assurance_envelope_names_the_sources_holding_the_barrier() {
    let verdict = sole_verdict(&warming(), &adapted("happy"));

    let uncertainty = verdict.uncertainty();
    assert_eq!(
        uncertainty.len(),
        1,
        "the model admits at most one uncertainty entry per reason"
    );
    assert_eq!(
        uncertainty[0].reason,
        InconclusiveReason::PrefixUncertainty,
        "post-activation evidence would settle it, which is what PREFIX names"
    );
    let detail = &uncertainty[0].detail;
    assert!(detail.contains(BARRIER_PENDING_DETAIL_PREFIX), "{detail}");
    for service in [ORDER_SERVICE, PAYMENT_SERVICE] {
        assert!(
            detail.contains(service),
            "{service} holds the barrier and must be named: {detail}"
        );
    }
    assert!(
        detail.contains(&format!(
            "acknowledged Filter epoch {ACKNOWLEDGED_EPOCH} is below the target epoch \
             {TARGET_EPOCH}"
        )),
        "the disclosure states the epoch gap, not merely that one exists: {detail}"
    );
}

/// The same evidence that Satisfies authoritatively without a barrier does not with one.
///
/// The comparison is the assertion: nothing about the evidence changed, so the difference
/// is the barrier and only the barrier.
#[test]
fn the_barrier_is_the_only_difference_between_authoritative_and_warming() {
    let evidence = adapted("happy");

    let unbarriered = sole_verdict(&ship_requires_prior_charge(), &evidence);
    assert_eq!(unbarriered.outcome(), VerdictOutcome::Satisfy);
    assert_eq!(unbarriered.authority(), VerdictAuthority::Authoritative);
    assert_eq!(
        unbarriered.settlement_basis(),
        Some(SettlementBasisKind::CompleteEvidence)
    );

    let warming = sole_verdict(&warming(), &evidence);
    assert_eq!(
        warming.outcome(),
        VerdictOutcome::Satisfy,
        "the barrier caps authority; it does not change the conclusion"
    );
    assert_eq!(warming.authority(), VerdictAuthority::Provisional);
}

/// Once every required source acknowledges the epoch and reaches its Watermark, the
/// identical evidence settles Authoritative. A cleared barrier is no longer invisible:
/// `CLEARED` vs `NOT_DECLARED`, and Cleared carries `target_filter_epoch`. Same outcome
/// and same authority. Not the same bytes.
#[test]
fn a_cleared_barrier_settles_exactly_as_a_specification_declaring_none() {
    let evidence = adapted("happy");

    let cleared = support::drive(&activated(), inputs(&evidence)).expect("evaluation succeeds");
    let unbarriered = support::drive(&ship_requires_prior_charge(), inputs(&evidence))
        .expect("evaluation succeeds");

    let cleared_verdict = cleared[0].verdict();
    let unbarriered_verdict = unbarriered[0].verdict();
    assert_eq!(cleared_verdict.outcome(), unbarriered_verdict.outcome());
    assert_eq!(cleared_verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(
        unbarriered_verdict.authority(),
        VerdictAuthority::Authoritative
    );
    assert_eq!(
        cleared_verdict.activation_barrier(),
        ActivationBarrierStatus::Cleared
    );
    assert_eq!(
        unbarriered_verdict.activation_barrier(),
        ActivationBarrierStatus::NotDeclared
    );
    assert_eq!(
        cleared_verdict.target_filter_epoch(),
        Some(FilterEpoch::new(ACKNOWLEDGED_EPOCH))
    );
    assert_eq!(unbarriered_verdict.target_filter_epoch(), None);
    assert_ne!(
        encoded_verdicts(&cleared),
        encoded_verdicts(&unbarriered),
        "a cleared barrier is typed, so the bytes cannot match an undeclared one"
    );
}

/// A warming Specification still emits a `Provisional Violate`, witness intact.
///
/// Accepted deliberately; see this file's header. Pinned so the trade cannot be reversed
/// silently by someone who reads "missing evidence is never a pass" as symmetric.
///
/// The witness is checked because a Violate that lost its evidence would be a different
/// and much worse artifact: `sentinel_core`'s admitted limitation 3 already flags that the
/// model accepts a `Provisional Violate` carrying no witness at all, so the explanation
/// carrying one is a property of this Core rather than of the model.
#[test]
fn a_warming_specification_still_emits_a_provisional_violate() {
    let mut capture = load_capture("happy");
    let order = only_shipped_order(&capture);
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let evidence = adapt(&capture);
    let specification = ship_requires_prior_charge()
        .with_activation_barrier(activation_barrier(&evidence, TARGET_EPOCH));

    let verdicts = support::drive(&specification, inputs(&evidence)).expect("evaluation succeeds");
    let binding = &verdicts[0];
    let verdict = binding.verdict();

    assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(
        verdict.settlement_basis(),
        None,
        "the model refuses a settlement basis on a Provisional Verdict, so the decisive \
         proof is not on the wire -- which is the deferred half this trade knowingly ships"
    );

    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);
    let ship = only_event(&evidence, "sagashop.order.ship.requested", &order);
    assert!(
        binding
            .explanation()
            .unmet_requirements
            .contains(&ship.id()),
        "the trigger whose obligation nothing discharges is still named"
    );
    assert!(
        verdict.anchored_event_ids()[&source(PAYMENT_SERVICE)].contains(&charge.id()),
        "the anchor-tier Event witnessing the absence is still cited"
    );
}

/// AC2: a warming Shadow Violate is attributable from typed fields alone.
///
/// This is the warming partition — `Violate ∧ Provisional ∧ PENDING` — a conservative
/// overestimate of "caused by a stale Filter." Epoch comparison is not the predicate:
/// a barrier can stay pending after every present acknowledged epoch has reached the
/// target.
#[test]
fn a_warming_violate_is_attributable_from_typed_fields_alone() {
    let mut capture = load_capture("happy");
    set_span_attribute(
        &mut capture,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );
    let evidence = adapt(&capture);
    let specification = ship_requires_prior_charge()
        .with_activation_barrier(activation_barrier(&evidence, TARGET_EPOCH));
    let verdict = sole_verdict(&specification, &evidence);

    assert_eq!(verdict.outcome(), VerdictOutcome::Violate);
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(
        verdict.activation_barrier(),
        ActivationBarrierStatus::Pending
    );
}

/// A barrier can stay PENDING after every present acknowledged epoch meets the target.
///
/// If this fixture is missing, implementers will treat epoch lag as the definition of
/// PENDING and AC2 will regress the moment rollout waits on the Watermark rather than
/// the Filter.
#[test]
fn a_watermark_lag_keeps_the_barrier_pending_after_the_target_is_acknowledged() {
    let evidence = adapted("happy");
    let watermarks = [ORDER_SERVICE, PAYMENT_SERVICE]
        .iter()
        .map(|service| {
            let identity = source(service);
            let signal = evidence
                .source(&identity)
                .unwrap_or_else(|| panic!("{service} supplied evidence"))
                .completeness();
            let required = SequenceNumber::new(
                signal
                    .complete_through()
                    .expect("the fixture declares a completeness boundary")
                    .value()
                    + 1,
            );
            (
                identity,
                ActivationWatermark::new(signal.epoch(), required, signal.observed_at()),
            )
        })
        .collect();
    let specification = ship_requires_prior_charge().with_activation_barrier(
        ActivationBarrier::new(FilterEpoch::new(ACKNOWLEDGED_EPOCH), watermarks)
            .expect("the fixture names two required sources"),
    );
    let verdict = sole_verdict(&specification, &evidence);

    assert_eq!(
        verdict.activation_barrier(),
        ActivationBarrierStatus::Pending
    );
    assert_eq!(
        verdict.target_filter_epoch(),
        Some(FilterEpoch::new(ACKNOWLEDGED_EPOCH))
    );
    for epoch in verdict.filter_epochs().values() {
        assert!(
            *epoch >= FilterEpoch::new(ACKNOWLEDGED_EPOCH),
            "every present acknowledgement meets the target; PENDING is watermark lag"
        );
    }
    assert_ne!(
        verdict.activation_barrier(),
        ActivationBarrierStatus::Cleared
    );
}

/// New Core writes an unset-ack row for a silent required source only while PENDING.
///
/// Observed through [`Verdict::filter_epoch_rows`]: [`Verdict::filter_epochs`] drops
/// unset acknowledgements and cannot pin this write.
#[test]
fn a_pending_barrier_writes_an_unset_ack_row_for_a_silent_required_source() {
    let evidence = adapted("happy");
    let withheld = source(PAYMENT_SERVICE);
    let mut evaluator = Evaluator::new(warming());
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }
    for (identity, record) in evidence.sources() {
        if *identity == withheld {
            continue;
        }
        evaluator
            .step(Input::source(record.clone()))
            .expect("source evidence steps");
    }
    let sealed = evaluator
        .seal()
        .expect("partial coverage still yields a Verdict");
    let verdict = sealed.verdicts()[0].verdict();

    assert_eq!(
        verdict.activation_barrier(),
        ActivationBarrierStatus::Pending
    );
    assert_eq!(verdict.filter_epochs().get(&withheld), None);
    assert_eq!(verdict.filter_epoch_rows().get(&withheld), Some(&None));
    assert_eq!(
        verdict.target_filter_epoch(),
        Some(FilterEpoch::new(TARGET_EPOCH))
    );
}

/// Missing evidence yields Inconclusive, never a pass — barrier or no barrier.
///
/// The barrier caps authority; it must not be able to *rescue* a conclusion either. Half
/// of Evidence Coverage (one required source delivers Events and declares nothing) stays
/// Inconclusive under a cleared barrier, and the source that declared nothing also holds
/// the barrier, so a cleared barrier is unreachable from incomplete coverage in the first
/// place.
#[test]
fn incomplete_coverage_is_inconclusive_under_every_barrier_state() {
    let evidence = adapted("happy");
    let withheld = source(PAYMENT_SERVICE);

    for (name, specification) in [
        ("no barrier", ship_requires_prior_charge()),
        ("cleared barrier", activated()),
        ("pending barrier", warming()),
    ] {
        let mut evaluator = Evaluator::new(specification);
        for input in event_inputs(&evidence) {
            evaluator.step(input).expect("Events step");
        }
        for (identity, record) in evidence.sources() {
            if *identity == withheld {
                continue;
            }
            evaluator
                .step(Input::source(record.clone()))
                .expect("source evidence steps");
        }
        let sealed = evaluator
            .seal()
            .expect("partial coverage still yields a Verdict");
        let verdict = sealed.verdicts()[0].verdict();

        assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive, "{name}");
        assert_eq!(verdict.authority(), VerdictAuthority::Provisional, "{name}");
        assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy, "{name}");
    }
}

/// A required source that declared nothing holds the barrier as `NoEvidence`.
///
/// The conservative direction, stated as its own test because it is the one place the
/// barrier could have been made to clear on silence: `BarrierState::of` reads
/// `Coverage::covered`, which is the *conjunction* of delivered Events and declared
/// evidence, so half of coverage supplies no completeness signal to the barrier at all.
#[test]
fn a_source_that_declared_nothing_holds_the_barrier() {
    let evidence = adapted("happy");
    let withheld = source(PAYMENT_SERVICE);

    let mut evaluator = Evaluator::new(activated());
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }
    for (identity, record) in evidence.sources() {
        if *identity == withheld {
            continue;
        }
        evaluator
            .step(Input::source(record.clone()))
            .expect("source evidence steps");
    }
    let sealed = evaluator
        .seal()
        .expect("partial coverage still yields a Verdict");
    let detail = &sealed.verdicts()[0].verdict().uncertainty()[0].detail;

    assert!(detail.contains(BARRIER_PENDING_DETAIL_PREFIX), "{detail}");
    assert!(
        detail.contains("no completeness signal"),
        "an undeclared source holds the barrier for lack of evidence, not for a \
         Watermark it failed to reach: {detail}"
    );
}

/// The Assurance Envelope records the epoch each source acknowledged, not a constant.
///
/// `sentinel_core` stamped `filter_epoch: Some(0)` on every Verdict until #19, which
/// happens to agree with what the SagaShop Adapter reports — so the field was written,
/// never read, and wrong for any deployment whose sources had moved on. ADR-0006 makes
/// this the field that keeps an unsound Verdict "identifiable rather than invisible", so
/// the fixture is rebuilt at a non-constant epoch to make the disagreement observable.
#[test]
fn the_envelope_records_the_acknowledged_filter_epoch_and_not_zero() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }
    for (identity, record) in evidence.sources() {
        let sample = evidence
            .events()
            .iter()
            .find(|event| event.source() == *identity)
            .expect("a declaring source delivered at least one Event");
        evaluator
            .step(Input::source(with_filter_epoch(record, sample, 7)))
            .expect("source evidence steps");
    }
    let sealed = evaluator.seal().expect("evaluation succeeds");
    let epochs = sealed.verdicts()[0].verdict().filter_epochs();

    assert_eq!(
        epochs.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([source(ORDER_SERVICE), source(PAYMENT_SERVICE)]),
        "every covered required source carries its acknowledged epoch"
    );
    for (identity, epoch) in &epochs {
        assert_eq!(
            *epoch,
            FilterEpoch::new(7),
            "{identity:?} must carry the epoch it acknowledged, not a constant"
        );
    }
}

/// A barrier changes nothing about which errors the evaluator raises.
///
/// The barrier is read only inside `settle`, which is reached after every
/// Specification-validity and Evidence-Coverage refusal. A Specification requiring no
/// source is still refused by name rather than warming forever behind a barrier it could
/// never clear.
#[test]
fn a_barrier_does_not_mask_a_specification_validity_defect() {
    let evidence = adapted("happy");
    let specification = support::ship_requires_prior_charge_without_required_sources()
        .with_activation_barrier(activation_barrier(&evidence, TARGET_EPOCH));

    let error = support::drive(&specification, inputs(&evidence))
        .expect_err("a Specification requiring no source is refused by name");
    assert!(
        matches!(error, CoreError::SpecificationRequiresNoSource { .. }),
        "{error:?}"
    );
}

/// Enforcement is structurally unreachable for a Shadow Specification, end to end.
///
/// AC1's "nothing is enforced" clause, driven as one sequence rather than asserted about
/// each stage separately: a real catalogue Pattern instance is compiled by
/// `sentinel_compile::compile_filter` into a validated Event Filter, and the Filter is
/// then asked the two questions a Probe actually asks it.
///
/// Both refusals are `sentinel_model`'s and neither is new here — which is the point.
/// `#19` adds no suppression to the Probe path because none is needed: a Specification
/// that is not both `Readiness::Active` **and** `EvaluationMode::Enforce` cannot appear in
/// an unresolved decision and cannot contribute a blocking budget. The pin exists so that
/// the follow-on lifecycle work, which changes what `compile_one` stamps on the
/// `Readiness` axis, cannot open the enforcement path by accident while doing it.
#[test]
fn a_shadow_specification_cannot_reach_enforcement_end_to_end() {
    let instance = support::cat1_ship_requires_prior_charge();
    let sources: Vec<&dyn FilterSource> = vec![&instance];
    let compiled =
        compile_filter(&sources, FilterEpoch::new(14)).expect("the catalogue instance compiles");
    let emitted = &compiled.specifications()[0];
    assert_eq!(
        emitted.evaluation_mode,
        raw_model::EvaluationMode::Shadow as i32,
        "a compiled Specification is Shadow, so it is not enforcing whatever its readiness"
    );
    let id = emitted.specification_id.clone();
    let version = emitted.specification_version.clone();

    let response = raw_probe::DecideResponse {
        request_id: "decide-1".to_owned(),
        action: raw_probe::DecisionAction::Defer as i32,
        specifications: vec![raw_probe::SpecificationDecision {
            specification_id: id.clone(),
            specification_version: version.clone(),
            action: raw_probe::DecisionAction::Defer as i32,
            fail_mode: emitted.fail_mode,
            unresolved_reason: Some(raw_probe::UnresolvedReason::Timeout as i32),
        }],
    };
    let matched = BTreeSet::from([(id.as_str(), version.as_str())]);

    assert_eq!(
        aggregate_unresolved(&compiled.filter, &matched, &response),
        Err(DecideRequestError::IrrelevantSpecificationDecision),
        "a Probe cannot even name a Shadow Specification in an unresolved decision, so \
         neither fail-open nor fail-closed is reachable for one"
    );
    assert_eq!(
        compiled
            .filter
            .effective_budget_nanoseconds(100, &BTreeSet::from([id.as_str()])),
        100,
        "a Shadow Specification contributes no blocking budget, so it cannot make a \
         caller wait either"
    );
}

/// Rolling an acknowledged Filter epoch backwards is refused, not re-emitted.
///
/// The barrier is the first rule that turns a source's acknowledged Filter epoch into
/// *authority*, which makes a gap in the evidence-monotonicity contract newly reachable:
/// `EvidenceLedger::record_source` guarded sequence epoch, `last_emitted`,
/// `complete_through`, tier, capabilities and gaps — and not the acknowledged Filter epoch,
/// because until now nothing read it.
///
/// Executed before the guard existed, on this fixture:
///
/// ```text
/// AT FILTER EPOCH 7: [Authoritative]
/// AFTER FILTER-EPOCH DROP re-emitted: [Provisional]
/// ```
///
/// That second emission is an `Authoritative → Provisional` regression, which
/// `sentinel-store` refuses outright (`StoreError::Conflict`, pinned on both backends by
/// `authority_regression_is_refused`). So the Core would have been handing a driver a
/// Verdict it considers correct and the store will not accept — a periphery failure caused
/// by a Core emission, which is the whole thing the monotonicity contract exists to
/// prevent. Refusing the input is the answer that keeps `seal()` sound: it is loud, it is
/// early, and it names the source.
#[test]
fn a_filter_epoch_rollback_is_refused_rather_than_regressing_authority() {
    let evidence = adapted("happy");
    let specification =
        ship_requires_prior_charge().with_activation_barrier(activation_barrier(&evidence, 7));
    let mut evaluator = Evaluator::new(specification);
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }

    let mut authorities = Vec::new();
    for (identity, record) in evidence.sources() {
        let sample = sample_event(&evidence, identity);
        if let Step::Settled(verdicts) = evaluator
            .step(Input::source(with_filter_epoch(record, sample, 7)))
            .expect("acknowledging the target epoch steps")
        {
            authorities = verdicts
                .iter()
                .map(|binding| binding.verdict().authority())
                .collect();
        }
    }
    assert_eq!(
        authorities,
        vec![VerdictAuthority::Authoritative],
        "every required source acknowledged the target epoch, so the barrier cleared"
    );

    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .sources()
        .get(&payment)
        .expect("the payment service is a required source");
    let sample = sample_event(&evidence, &payment);
    let outcome = evaluator.step(Input::source(with_filter_epoch(record, sample, 5)));

    match outcome {
        Err(CoreError::NonMonotonicEvidence(breach)) => {
            assert_eq!(breach.breach(), MonotonicityBreach::RegressedFilterEpoch);
            assert_eq!(breach.source(), &payment);
        }
        other => panic!("a Filter-epoch rollback must be refused by name, got {other:?}"),
    }
}

/// Acknowledging a *newer* Filter epoch is admitted and upgrades authority.
///
/// The other direction of the same guard, and the one that must keep working: rolling
/// forward is how a barrier clears at all. The resulting `Provisional → Authoritative`
/// move is the authority upgrade `sentinel_core`'s limitation 8 blesses and the store
/// permits, so refusing both directions would have made the barrier unclearable.
#[test]
fn acknowledging_a_newer_filter_epoch_clears_the_barrier_and_upgrades_authority() {
    let evidence = adapted("happy");
    let specification =
        ship_requires_prior_charge().with_activation_barrier(activation_barrier(&evidence, 7));
    let mut evaluator = Evaluator::new(specification);
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }

    // Every source starts behind the target epoch.
    let mut authority = None;
    for (identity, record) in evidence.sources() {
        let sample = sample_event(&evidence, identity);
        if let Step::Settled(verdicts) = evaluator
            .step(Input::source(with_filter_epoch(record, sample, 5)))
            .expect("a stale epoch is admissible evidence")
        {
            authority = verdicts.first().map(|b| b.verdict().authority());
        }
    }
    assert_eq!(authority, Some(VerdictAuthority::Provisional));

    // Then each rolls forward past it.
    for (identity, record) in evidence.sources() {
        let sample = sample_event(&evidence, identity);
        if let Step::Settled(verdicts) = evaluator
            .step(Input::source(with_filter_epoch(record, sample, 9)))
            .expect("rolling forward is admissible")
        {
            authority = verdicts.first().map(|b| b.verdict().authority());
        }
    }
    assert_eq!(
        authority,
        Some(VerdictAuthority::Authoritative),
        "a newer epoch ships a superset of the target, so the barrier clears"
    );
}
