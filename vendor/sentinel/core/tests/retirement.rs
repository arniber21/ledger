//! Scope retirement: the admissibility contract, the pin, and the emission bound.
//!
//! Every fixture here is a vendored SagaShop capture plus a hand-built
//! Specification declaring a retirement policy. The Specification is hand-built
//! because CAT-3 `existence.before-scope-close` — the Pattern whose Scope can
//! genuinely fail to close — defers to #210, so the catalogue offers no
//! retirable instance to lower. AC1 is therefore met **by construction rather
//! than by catalogued evidence**, which is the same state the activation
//! barrier shipped in and is disclosed rather than glossed.
//!
//! Every `ObservationTime` is built from an integer literal through
//! `support::observation_time`. `.github/scripts/check-core-purity.sh` scans
//! `core/src` and not `core/tests`, so that discipline is enforced by there
//! being one helper rather than by a gate.
//!
//! # The gate-closed fixture, and why it is the one to run on
//!
//! Retirement's whole novelty is emitting for **one** Binding behind a gate
//! that has not opened. Withholding one required source's `SourceEvidence`
//! closes the gate through G1b while leaving Evidence Coverage non-empty, which
//! is exactly the state the retired emission licence is defined over. On an
//! open-gate fixture the gate has already emitted, and clause (d2) refuses.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod support;

use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::slice::{Binding, Retirement, RetirementBreach, RetirementCause};
use sentinel_core::specification::RetirementPolicy;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::{InconclusiveReason, VerdictAuthority, VerdictOutcome};

use support::{
    adapted, drive, event_inputs, inputs, observation_range, only_shipped_order, retire, retire_at,
    scope_horizon, scope_horizon_on, ship_requires_prior_charge,
    ship_requires_prior_charge_retiring, source, source_inputs, source_inputs_except,
    ORDER_SERVICE,
};

/// A horizon comfortably above every observation in any vendored fixture, so
/// the retired Slice is the whole Slice and nothing is excluded by accident.
const ABOVE_EVERYTHING: i128 = 1_000_000;

fn settled(step: &Step) -> &[BindingVerdict] {
    match step {
        Step::Settled(verdicts) => verdicts,
        other => panic!("expected Settled, got {other:?}"),
    }
}

/// Feeds every Event and every source **except** the order service, leaving the
/// evidence gate closed through G1b with Evidence Coverage non-empty.
fn gate_closed_evaluator(policy: RetirementPolicy) -> (Evaluator, String) {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(policy));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        evaluator.step(input).expect("sources step cleanly");
    }
    assert!(
        !evaluator.evidence_is_settled(),
        "this fixture exists to be gate-closed"
    );
    assert!(
        !evaluator.is_emitted(&Binding::new(&order)),
        "nothing may have emitted before the retirement signal"
    );
    (evaluator, order)
}

/// The prefix-uncertainty detail on one Verdict, or a panic naming what it holds.
fn prefix_detail(verdict: &BindingVerdict) -> String {
    let entries: Vec<_> = verdict
        .verdict()
        .uncertainty()
        .into_iter()
        .filter(|entry| entry.reason == InconclusiveReason::PrefixUncertainty)
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "exactly one PREFIX_UNCERTAINTY entry, or the model rejects the envelope"
    );
    entries[0].detail.clone()
}

// ---------------------------------------------------------------------------
// AC1 — a retired Scope emits prefix-Inconclusive carrying the O-03 detail
// ---------------------------------------------------------------------------

/// A Scope with no reliable close retires and emits prefix uncertainty, with
/// the O-03 sentence first and the anchor-silence sentence absent.
///
/// The three assertions that matter are not the outcome. They are that the
/// detail names the cause **and** the horizon, that the O-03 sentence leads,
/// and that `uncertainty` holds exactly **one** `PREFIX_UNCERTAINTY` entry — a
/// second would be `DuplicateValue("assurance.uncertainty")` from the model,
/// surfacing a periphery input as a Core bug, which is #108's failure class.
#[test]
fn retirement_emits_prefix_inconclusive_with_the_o03_detail() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);

    let step = evaluator
        .step(retire(&order, 4_242, RetirementCause::Deadline))
        .expect("the retirement signal is admissible");
    let verdicts = settled(&step);
    assert_eq!(verdicts.len(), 1, "one retired Binding, one Verdict");

    let verdict = &verdicts[0];
    assert_eq!(verdict.binding(), order);
    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert_eq!(
        verdict.verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );

    let detail = prefix_detail(verdict);
    assert!(
        detail.starts_with("O-03 scope retired: deadline at sentinel-replay:4242"),
        "the O-03 sentence must lead the disclosure, got {detail}"
    );
    assert!(
        !detail.contains("no anchor-tier Event witnesses"),
        "a retired Verdict is not Inconclusive because of anchor silence, got {detail}"
    );
    assert_eq!(
        verdict
            .explanation()
            .retirement
            .as_ref()
            .map(Retirement::cause),
        Some(RetirementCause::Deadline)
    );
}

/// Both declared causes reach the Verdict, and each names itself.
///
/// Without this, `RetirementCause` would be a value the Core accepts and never
/// reports, and clause (d0)'s cause check would be unobservable.
#[test]
fn deadline_and_inactivity_causes_are_both_admissible() {
    for (policy, cause, name) in [
        (
            RetirementPolicy::Deadline,
            RetirementCause::Deadline,
            "deadline",
        ),
        (
            RetirementPolicy::Inactivity,
            RetirementCause::Inactivity,
            "inactivity",
        ),
    ] {
        let (mut evaluator, order) = gate_closed_evaluator(policy);
        let step = evaluator
            .step(retire(&order, ABOVE_EVERYTHING, cause))
            .expect("the declared cause is admissible");
        let detail = prefix_detail(&settled(&step)[0]);
        assert!(
            detail.starts_with(&format!("O-03 scope retired: {name} at")),
            "expected the {name} cause in {detail}"
        );
    }
}

/// Clause (d0): a cause the Specification does not declare is refused.
#[test]
fn a_cause_the_specification_does_not_declare_is_rejected() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);

    assert_eq!(
        evaluator.step(retire(&order, 10, RetirementCause::Inactivity)),
        Err(CoreError::Retirement(RetirementBreach::CauseNotDeclared))
    );
}

/// Clause (d0): a Specification declaring no policy cannot be retired at all.
///
/// This is also how CAT-4 is excluded, and the exclusion comes from the
/// Pattern's own declaration rather than from a family check in the evaluator.
#[test]
fn a_specification_that_declares_no_policy_refuses_retirement() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in inputs(&evidence) {
        evaluator.step(input).expect("the fixture steps cleanly");
    }

    assert_eq!(
        evaluator.step(retire(&order, 10, RetirementCause::Deadline)),
        Err(CoreError::Retirement(RetirementBreach::NotDeclared))
    );
}

// ---------------------------------------------------------------------------
// AC2 — never Violate, never Satisfy
// ---------------------------------------------------------------------------

/// A retired Binding never reports `Violate`, on evidence that otherwise would.
///
/// The fixture is the P4 Specification over the `hazards` capture, where an
/// order ships with no successful charge — the shape
/// `anchor_silence_authoritatively_violates` settles as an authoritative
/// `Violate` once the gate opens. Retired first, it cannot.
#[test]
fn a_retired_binding_never_reports_violate() {
    let evidence = adapted("hazards");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    let retirable: Vec<Binding> = evaluator.slices().bindings().cloned().collect();
    assert!(!retirable.is_empty(), "the fixture creates Bindings");
    for binding in &retirable {
        evaluator
            .step(retire_at(
                binding.as_str(),
                scope_horizon(ABOVE_EVERYTHING),
                RetirementCause::Deadline,
            ))
            .expect("retirement before any emission is admissible");
    }
    for input in source_inputs(&evidence) {
        evaluator.step(input).expect("sources step cleanly");
    }

    let sealed = evaluator
        .seal()
        .expect("the sealed evaluator produces Verdicts");
    assert!(
        !sealed.verdicts().is_empty(),
        "the fixture produces Verdicts"
    );
    for verdict in sealed.verdicts() {
        assert_eq!(
            verdict.verdict().outcome(),
            VerdictOutcome::Inconclusive,
            "{} settled {:?} despite being retired",
            verdict.binding(),
            verdict.verdict().outcome()
        );
        assert!(
            verdict.verdict().settlement_basis().is_none(),
            "a retired Verdict names no settlement basis"
        );
    }
}

/// A retired Binding never reports `Satisfy`, on evidence that otherwise would.
///
/// ADR-0025 licenses `Satisfy` only when the Scope **closes**. A retired Scope
/// did not close, it expired, so reporting a pass would be ADR-0017's silent
/// pass with a timer attached. The baseline is asserted non-empty and asserted
/// to actually contain a `Satisfy`, or this test would compare nothing to
/// nothing.
#[test]
fn a_retired_binding_never_reports_satisfy() {
    let evidence = adapted("happy");
    let specification = ship_requires_prior_charge();
    let baseline = drive(&specification, inputs(&evidence)).expect("the happy path evaluates");
    assert!(!baseline.is_empty(), "the baseline is not empty");
    assert!(
        baseline
            .iter()
            .any(|verdict| verdict.verdict().outcome() == VerdictOutcome::Satisfy),
        "the baseline must contain the Satisfy this test exists to suppress"
    );

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    let bindings: Vec<Binding> = evaluator.slices().bindings().cloned().collect();
    for binding in &bindings {
        evaluator
            .step(retire(
                binding.as_str(),
                ABOVE_EVERYTHING,
                RetirementCause::Deadline,
            ))
            .expect("retirement before any emission is admissible");
    }
    for input in source_inputs(&evidence) {
        evaluator.step(input).expect("sources step cleanly");
    }

    let sealed = evaluator
        .seal()
        .expect("the sealed evaluator produces Verdicts");
    assert_eq!(sealed.verdicts().len(), baseline.len());
    for verdict in sealed.verdicts() {
        assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    }
}

/// A retired Verdict still names the evidence it was reached over.
///
/// The point of assembling the retired envelope from a real assessment rather
/// than from a literal: a Verdict whose Explanation named nothing would report
/// the conclusion without the evidence behind it, which is half of what a
/// Verdict is for.
#[test]
fn a_retired_binding_names_its_evidence() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    let step = evaluator
        .step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline))
        .expect("admissible");

    let explanation = settled(&step)[0].explanation();
    assert_eq!(explanation.binding, order);
    assert!(
        !explanation.discharged_by.is_empty(),
        "the happy path discharges its trigger, and the retired Verdict says so"
    );
    assert!(
        explanation.concurrent_pairs.is_empty(),
        "a retired conclusion does not turn on an unordered pair"
    );
    assert!(explanation.retirement.is_some());
}

// ---------------------------------------------------------------------------
// AC3 — idempotent, monotone, and an earlier signal refused
// ---------------------------------------------------------------------------

/// Clause (d3): the same horizon twice is a no-op and emits nothing new.
#[test]
fn retiring_twice_at_the_same_horizon_is_a_no_op() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    let first = evaluator
        .step(retire(&order, 500, RetirementCause::Deadline))
        .expect("admissible");
    assert_eq!(settled(&first).len(), 1);

    let second = evaluator
        .step(retire(&order, 500, RetirementCause::Deadline))
        .expect("an identical signal is idempotent");
    assert_eq!(
        second,
        Step::Pending,
        "restating the horizon changes no Verdict, so nothing re-emits"
    );
}

/// Clause (d3): a later horizon joins upward, and re-emits nothing.
///
/// The key is one bit and the outcome is a constant, so advancing the horizon
/// cannot change what was emitted — even though it can widen the Slice.
#[test]
fn retiring_at_a_later_horizon_advances_the_join() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    let (lowest, highest) = observation_range(&adapted("happy"));
    assert!(
        lowest < highest,
        "the fixture spans a range of observations"
    );

    evaluator
        .step(retire(&order, lowest, RetirementCause::Deadline))
        .expect("admissible");
    let advanced = evaluator
        .step(retire(&order, highest, RetirementCause::Deadline))
        .expect("a later horizon joins");
    assert_eq!(advanced, Step::Pending);

    // The join is what the sealed Verdict is computed over: the wider Slice.
    let sealed = evaluator.seal().expect("sealing succeeds");
    let detail = prefix_detail(&sealed.verdicts()[0]);
    assert!(
        detail.starts_with(&format!(
            "O-03 scope retired: deadline at sentinel-replay:{highest}"
        )),
        "the join must end at the later horizon, got {detail}"
    );
}

/// Clause (d3): an earlier-expiry signal is rejected. AC3, word for word.
#[test]
fn retiring_at_an_earlier_horizon_is_rejected_as_horizon_regressed() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    evaluator
        .step(retire(&order, 900, RetirementCause::Deadline))
        .expect("admissible");

    assert_eq!(
        evaluator.step(retire(&order, 100, RetirementCause::Deadline)),
        Err(CoreError::Retirement(RetirementBreach::HorizonRegressed))
    );
}

/// Two signals in either order reach the same **sealed** Verdict.
///
/// The join is `max`, which is commutative, so the descending order refuses its
/// second signal and both evaluators end at the same horizon. That is the
/// difference between a join and a freeze: a frozen horizon would have ended at
/// the first signal's value, and the two orders would disagree.
#[test]
fn a_pair_of_retire_inputs_reaches_the_same_state_in_either_order() {
    let (lowest, highest) = observation_range(&adapted("happy"));

    let (mut ascending, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    ascending
        .step(retire(&order, lowest, RetirementCause::Deadline))
        .expect("admissible");
    ascending
        .step(retire(&order, highest, RetirementCause::Deadline))
        .expect("a later horizon joins");

    let (mut descending, _) = gate_closed_evaluator(RetirementPolicy::Deadline);
    descending
        .step(retire(&order, highest, RetirementCause::Deadline))
        .expect("admissible");
    assert_eq!(
        descending.step(retire(&order, lowest, RetirementCause::Deadline)),
        Err(CoreError::Retirement(RetirementBreach::HorizonRegressed))
    );

    let ascending = ascending.seal().expect("sealing succeeds");
    let descending = descending.seal().expect("sealing succeeds after a refusal");
    assert!(
        !ascending.verdicts().is_empty(),
        "the baseline is not empty"
    );
    assert_eq!(
        support::encoded_verdicts(ascending.verdicts()),
        support::encoded_verdicts(descending.verdicts()),
        "the join is order-free within one Sentinel clock"
    );
}

/// A horizon on a different Sentinel clock is refused in both orders.
///
/// Ordering is defined only within one clock. Collapsing the cross-clock case
/// into a comparison would silently join two horizons that are not comparable,
/// which is the two-valued collapse `LateRelation::DifferentSentinelClock`
/// exists to prevent.
#[test]
fn retiring_across_sentinel_clocks_is_rejected_as_incomparable() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    evaluator
        .step(retire(&order, 100, RetirementCause::Deadline))
        .expect("admissible");
    assert_eq!(
        evaluator.step(retire_at(
            &order,
            scope_horizon_on("sentinel-elsewhere", 900),
            RetirementCause::Deadline,
        )),
        Err(CoreError::Retirement(RetirementBreach::IncomparableHorizon))
    );

    let (mut reversed, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    reversed
        .step(retire_at(
            &order,
            scope_horizon_on("sentinel-elsewhere", 900),
            RetirementCause::Deadline,
        ))
        .expect("the first signal defines the clock");
    assert_eq!(
        reversed.step(retire(&order, 100, RetirementCause::Deadline)),
        Err(CoreError::Retirement(RetirementBreach::IncomparableHorizon))
    );

    // Recoverable in both directions.
    assert_eq!(
        reversed
            .resume(Vec::new())
            .expect("an empty resume is legal"),
        Step::Pending
    );
    assert!(reversed.seal().is_ok());
}

// ---------------------------------------------------------------------------
// AC4 — retirement cannot change an already-settled Verdict
// ---------------------------------------------------------------------------

/// Clause (d2): retiring an emitted Binding is refused rather than applied.
///
/// The alternative is silently replacing an emitted Verdict with an
/// `Inconclusive` — an outcome retraction with no Amendment record, which
/// ADR-0021's O-11 forbids and `sentinel-store` refuses. Refusing loudly
/// neither retracts nor fabricates.
#[test]
fn retirement_never_changes_an_emitted_outcome() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in inputs(&evidence) {
        evaluator.step(input).expect("the fixture steps cleanly");
    }
    assert!(
        evaluator.is_emitted(&Binding::new(&order)),
        "the gate opened and the Binding emitted"
    );

    assert_eq!(
        evaluator.step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline)),
        Err(CoreError::Retirement(
            RetirementBreach::BindingAlreadyEmitted
        ))
    );

    let sealed = evaluator
        .seal()
        .expect("sealing succeeds after the refusal");
    assert!(sealed
        .verdicts()
        .iter()
        .any(|verdict| verdict.verdict().outcome() == VerdictOutcome::Satisfy));
}

/// A refused signal leaves the evaluator usable.
///
/// Every clause is validated before any mutation, which is #108's lesson: a
/// mutation committed ahead of an error wedges the evaluator, and every later
/// step fails identically. An empty `resume()` and a `seal()` both still work.
#[test]
fn retirement_rejection_does_not_wedge_the_evaluator() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);

    assert_eq!(
        evaluator.step(retire(&order, 10, RetirementCause::Inactivity)),
        Err(CoreError::Retirement(RetirementBreach::CauseNotDeclared))
    );
    assert_eq!(
        evaluator
            .resume(Vec::new())
            .expect("an empty resume is legal"),
        Step::Pending
    );
    // And the refused signal recorded nothing: a correct one still transitions.
    let step = evaluator
        .step(retire(&order, 10, RetirementCause::Deadline))
        .expect("the declared cause is admissible");
    assert_eq!(settled(&step).len(), 1);

    let sealed = evaluator.seal().expect("sealing still succeeds");
    assert_eq!(sealed.verdicts().len(), 1);
}

// ---------------------------------------------------------------------------
// AC6 — at most two emissions per Binding
// ---------------------------------------------------------------------------

/// Counts every emission per Binding across a whole input sequence.
fn emission_counts(
    evaluator: &mut Evaluator,
    inputs: Vec<Input>,
) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for input in inputs {
        if let Step::Settled(verdicts) = evaluator.step(input).expect("the fixture steps cleanly") {
            for verdict in verdicts {
                *counts.entry(verdict.binding().to_owned()).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// A retired Binding emits at most twice however much evidence arrives after.
///
/// The retirement lands first, then every remaining source declaration and an
/// empty resume. Without the one-bit key the envelope would differ each time
/// coverage or authority moved, and the bound would not hold.
#[test]
fn a_retired_binding_emits_at_most_twice() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    // One source, so Evidence Coverage is non-empty before the signal arrives.
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        evaluator.step(input).expect("sources step cleanly");
    }

    let mut rest = vec![retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline)];
    rest.extend(source_inputs(&evidence));
    let counts = emission_counts(&mut evaluator, rest);

    assert!(!counts.is_empty(), "the Binding emitted at least once");
    for (binding, count) in &counts {
        assert!(
            *count <= 2,
            "{binding} emitted {count} times; the theorem bounds it at two"
        );
    }
    assert!(evaluator.seal().is_ok());
}

/// The three-emission counterexample, run.
///
/// `settlement` derives `cold_path` from coverage and `authority` from
/// `cold_path` **and** the activation barrier, and those move independently. For
/// a guard-free Specification whose coverage completes before its barrier
/// clears, `(authority, cold_path)` takes three values in sequence — and a
/// Binding emitting behind a closed gate sees every one of them. This is the
/// case the envelope key gets wrong and the one-bit key gets right.
///
/// **Mutation evidence, executed.** With `Evaluator::emission_key`'s retired arm
/// replaced by `EmissionKey::Envelope`, this test reports **four** emissions for
/// the Binding rather than two.
#[test]
fn a_retired_binding_behind_a_clearing_barrier_emits_at_most_twice() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let barrier = support::activation_barrier(&evidence, 7);
    let specification = ship_requires_prior_charge_retiring(RetirementPolicy::Deadline)
        .with_activation_barrier(barrier);

    let mut evaluator = Evaluator::new(specification);
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        evaluator.step(input).expect("sources step cleanly");
    }

    let mut rest = vec![retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline)];
    // Coverage completes first, then the barrier clears: two independent moves
    // after the retirement, each of which would change the encoded envelope.
    rest.extend(source_inputs(&evidence));
    for (identity, record) in evidence.sources() {
        let sample = evidence
            .events()
            .iter()
            .find(|event| event.source() == *identity)
            .expect("a declaring source delivered at least one Event");
        rest.push(Input::source(support::with_filter_epoch(record, sample, 7)));
    }
    let counts = emission_counts(&mut evaluator, rest);

    assert_eq!(
        counts.get(&order),
        Some(&2),
        "both admissible emissions must occur, and the third must not: {counts:?}"
    );
    for (binding, count) in &counts {
        assert!(
            *count <= 2,
            "{binding} emitted {count} times behind a clearing barrier"
        );
    }
    assert!(evaluator.seal().is_ok());
}

/// A retired Binding with a terminal cold-path refusal still emits at most twice.
///
/// The reason a retired Verdict carries is **not** unconditional:
/// `epistemic_from_refused` runs after the outcome is fixed and rewrites
/// `PrefixUncertainty` to `EpistemicUncertainty`. That is licensed and is not an
/// outcome change — but a key that carried the reason would see it move and
/// emit a third time. The one-bit key does not.
///
/// **Mutation evidence, executed.** With `Evaluator::emission_key`'s retired arm
/// replaced by `EmissionKey::Envelope`, this test reports **three** emissions.
#[test]
fn a_retired_binding_with_a_refused_guard_still_emits_at_most_twice() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let required = support::required_watermarks(&evidence);
    let specification = support::ship_requires_prior_charge_with_cold_guards()
        .with_retirement_policy(RetirementPolicy::Deadline);

    let mut evaluator = Evaluator::new(specification);
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        evaluator.step(input).expect("sources step cleanly");
    }

    let mut rest = vec![retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline)];
    rest.extend(source_inputs(&evidence));
    let mut counts = std::collections::BTreeMap::new();
    let mut issued = None;
    for input in rest {
        match evaluator.step(input).expect("the fixture steps cleanly") {
            Step::Settled(verdicts) => {
                for verdict in verdicts {
                    *counts.entry(verdict.binding().to_owned()).or_insert(0) += 1;
                }
            }
            Step::NeedsFacts(request) => issued = Some(request),
            _ => {}
        }
    }

    // The gate is open now, so a query exists to refuse.
    let request = match issued {
        Some(request) => request,
        None => match evaluator
            .resume(Vec::new())
            .expect("an empty resume is legal")
        {
            Step::NeedsFacts(request) => request,
            other => panic!("expected NeedsFacts, got {other:?}"),
        },
    };
    let refusal = support::answer_all_as(
        &request,
        &sentinel_core::facts::GuardConclusion::Refused {
            reason: sentinel_core::facts::RefusalReason::EvidenceReaped,
        },
        &required,
    );
    if let Step::Settled(verdicts) = evaluator.resume(refusal).expect("a refusal settles") {
        for verdict in verdicts {
            assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
            assert_eq!(
                verdict.verdict().inconclusive_reason(),
                Some(InconclusiveReason::EpistemicUncertainty),
                "the retired reason is rewritten by a terminal refusal, and that is licensed"
            );
            *counts.entry(verdict.binding().to_owned()).or_insert(0) += 1;
        }
    }

    assert_eq!(
        counts.get(&order),
        Some(&2),
        "the retired Binding emits once when licensed and once on the authority upgrade: {counts:?}"
    );
}

/// Evidence arriving after retirement re-emits once, for the authority upgrade.
///
/// The encoded envelope moves twice here -- the Watermark set grows when the
/// withheld source declares, and the authority upgrades -- while the retired key
/// carries one bit. This is the narrow case: with the retired arm keyed on the
/// envelope instead, the two moves land on the *same* step, so this test alone
/// would not notice. The two that do are recorded on
/// `a_retired_binding_behind_a_clearing_barrier_emits_at_most_twice` and
/// `a_retired_binding_with_a_refused_guard_still_emits_at_most_twice`.
#[test]
fn post_retirement_ledger_movement_does_not_re_emit() {
    let evidence = adapted("happy");
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);

    let first = evaluator
        .step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline))
        .expect("admissible");
    assert_eq!(settled(&first).len(), 1);

    // The withheld source now declares. Coverage completes, the gate opens, and
    // the encoded envelope changes twice over — the Watermark set grows and the
    // authority upgrades — but the retired key carries one bit.
    let mut later = Vec::new();
    for input in source_inputs(&evidence) {
        if let Step::Settled(verdicts) = evaluator.step(input).expect("sources step cleanly") {
            for verdict in verdicts {
                later.push(verdict.verdict().authority());
            }
        }
    }
    assert_eq!(
        later,
        vec![VerdictAuthority::Authoritative],
        "exactly one further emission, and only for the authority upgrade"
    );
}

// ---------------------------------------------------------------------------
// AC7 — the Watermark and retirement are orthogonal, in both directions
// ---------------------------------------------------------------------------

/// A fully-advanced Watermark retires nothing.
///
/// `watermarks_cover_epochs()` means the Core holds every Event each required
/// source claims to have emitted — not that no further Event will arrive for
/// this Scope. Deriving one from the other is the conflation O-03 refuses.
#[test]
fn a_fully_advanced_watermark_does_not_retire_a_scope() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in inputs(&evidence) {
        evaluator.step(input).expect("the fixture steps cleanly");
    }
    assert!(
        evaluator.evidence_is_settled(),
        "every required source's Watermark covers its epoch"
    );

    let sealed = evaluator.seal().expect("sealing succeeds");
    for verdict in sealed.verdicts() {
        assert!(
            verdict.explanation().retirement.is_none(),
            "nothing but Input::Retire may write a lifecycle"
        );
        assert!(!prefix_detail_or_empty(verdict).contains("O-03 scope retired"));
    }
}

fn prefix_detail_or_empty(verdict: &BindingVerdict) -> String {
    verdict
        .verdict()
        .uncertainty()
        .into_iter()
        .map(|entry| entry.detail)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Retirement under an incomplete Watermark yields prefix uncertainty, not an
/// error.
///
/// The other direction of the same orthogonality. The gate is closed and the
/// Watermarks do not cover, and the retired Verdict is still a Verdict.
#[test]
fn retirement_under_an_incomplete_watermark_yields_prefix_inconclusive_not_an_error() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    let step = evaluator
        .step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline))
        .expect("retirement under an incomplete Watermark is not a gate error");

    let verdict = &settled(&step)[0];
    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    assert!(
        prefix_detail(verdict).contains("O-03 scope retired"),
        "the O-03 sentence is present alongside the gate's own"
    );
}

// ---------------------------------------------------------------------------
// AC8 — seal() is an emission licence, not an outcome pin
// ---------------------------------------------------------------------------

/// Under a **closed** gate, sealing and retiring everything agree on outcome and
/// authority per Binding.
///
/// The precondition is asserted in the body, because the claim is false without
/// it: retirement pins unconditionally and assessment demotes only when the gate
/// is closed. Byte-identity is **not** claimed and is false — a retired envelope
/// carries the O-03 sentence.
#[test]
fn seal_and_retire_all_agree_on_outcomes_and_authorities_under_a_closed_gate() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));

    let mut sealing = Evaluator::new(ship_requires_prior_charge());
    for input in event_inputs(&evidence) {
        sealing.step(input).expect("events step cleanly");
    }
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        sealing.step(input).expect("sources step cleanly");
    }
    assert!(
        !sealing.evidence_is_settled(),
        "the claim holds only under a closed gate, and this asserts it"
    );
    let sealed = sealing.seal().expect("sealing succeeds");

    let (mut retiring, _) = gate_closed_evaluator(RetirementPolicy::Deadline);
    retiring
        .step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline))
        .expect("admissible");
    let retired = retiring.seal().expect("sealing succeeds");

    assert!(!sealed.verdicts().is_empty(), "the baseline is not empty");
    assert_eq!(sealed.verdicts().len(), retired.verdicts().len());
    for (left, right) in sealed.verdicts().iter().zip(retired.verdicts()) {
        assert_eq!(left.binding(), right.binding());
        assert_eq!(left.verdict().outcome(), right.verdict().outcome());
        assert_eq!(left.verdict().authority(), right.verdict().authority());
        assert_eq!(left.verdict().cold_path(), right.verdict().cold_path());
    }
    assert_ne!(
        support::encoded_verdicts(sealed.verdicts()),
        support::encoded_verdicts(retired.verdicts()),
        "byte-identity is explicitly not claimed: the O-03 sentence differs"
    );
}

/// On an **open** gate, sealing pins nothing.
///
/// The negative half of the distinction, and the reason the test above needs a
/// precondition rather than a fixture choice.
#[test]
fn seal_does_not_pin_outcomes_on_an_open_gate() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in inputs(&evidence) {
        evaluator.step(input).expect("the fixture steps cleanly");
    }
    assert!(evaluator.evidence_is_settled());

    let sealed = evaluator.seal().expect("sealing succeeds");
    assert!(sealed
        .verdicts()
        .iter()
        .any(|verdict| verdict.verdict().outcome() != VerdictOutcome::Inconclusive));
}

// ---------------------------------------------------------------------------
// The wedge, the licence, and the Bindings the Core has not seen
// ---------------------------------------------------------------------------

/// Retiring before any source declares does not error and does not wedge.
///
/// Without the non-empty-Evidence-Coverage conjunct on the retired licence,
/// `refuse_unconstructible_envelope` returns `NoEvidenceCoverage` from `step()`
/// with the lifecycle mutation already committed, and every later step fails
/// identically until a source arrives.
#[test]
fn retiring_before_any_source_declares_does_not_error_and_does_not_wedge() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }

    assert_eq!(
        evaluator
            .step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline))
            .expect("retirement with no coverage is admissible"),
        Step::Pending,
        "there is no Evidence Coverage to build an envelope from yet"
    );
    assert_eq!(
        evaluator
            .resume(Vec::new())
            .expect("an empty resume is legal"),
        Step::Pending
    );

    // A source arrives, and the retired Verdict appears.
    let mut emitted = false;
    for input in source_inputs(&evidence) {
        if let Step::Settled(verdicts) = evaluator.step(input).expect("sources step cleanly") {
            emitted = true;
            for verdict in verdicts {
                assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
            }
        }
    }
    assert!(emitted, "the retired Binding emits once coverage exists");
    assert!(evaluator.seal().is_ok());
}

/// An open Binding is not emitted just because a sibling retired.
///
/// The licence is per Binding and is applied where the Binding is bound.
/// Widening the branch alone would emit every open Binding behind a closed
/// gate, which destroys the premise the two-emission theorem rests on.
#[test]
fn an_open_binding_is_not_emitted_when_a_sibling_binding_retires() {
    let evidence = adapted("hazards");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        evaluator.step(input).expect("sources step cleanly");
    }
    let bindings: Vec<Binding> = evaluator.slices().bindings().cloned().collect();
    assert!(
        bindings.len() > 1,
        "this test needs a sibling Binding to leave alone"
    );

    let retired = bindings[0].clone();
    let step = evaluator
        .step(retire(
            retired.as_str(),
            ABOVE_EVERYTHING,
            RetirementCause::Deadline,
        ))
        .expect("admissible");

    let emitted = settled(&step);
    assert_eq!(emitted.len(), 1, "only the retired Binding is licensed");
    assert_eq!(emitted[0].binding(), retired.as_str());
    for binding in bindings.iter().skip(1) {
        assert!(
            !evaluator.is_emitted(binding),
            "{binding} is still open and must not have emitted"
        );
    }
}

/// Retiring behind a closed gate declares no cold query.
///
/// The cold floor is snapshotted once and never refreshed, so a query declared
/// behind a closed gate installs a permanently lower floor and lets a cold
/// answer settle a conjunct it never saw. `declare_queries` therefore stays
/// gate-open only, and the floor a retired evaluator eventually records is the
/// gate-open floor.
#[test]
fn retirement_declares_no_cold_query_while_the_gate_is_closed() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&support::load_capture("happy"));
    let specification = support::ship_requires_prior_charge_with_cold_guards()
        .with_retirement_policy(RetirementPolicy::Deadline);

    let mut evaluator = Evaluator::new(specification.clone());
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    for input in source_inputs_except(&evidence, &source(ORDER_SERVICE)) {
        evaluator.step(input).expect("sources step cleanly");
    }
    evaluator
        .step(retire(&order, ABOVE_EVERYTHING, RetirementCause::Deadline))
        .expect("admissible");
    assert!(
        evaluator.outstanding().is_empty(),
        "no query may be declared while the gate is closed"
    );

    // The gate opens; the floor recorded then is the gate-open floor.
    for input in source_inputs(&evidence) {
        evaluator.step(input).expect("sources step cleanly");
    }
    let retired_floor: Vec<_> = evaluator
        .outstanding()
        .iter()
        .map(|query| query.watermark_floor().clone())
        .collect();
    assert!(
        !retired_floor.is_empty(),
        "a query is declared once the gate opens"
    );

    let mut untouched = Evaluator::new(support::ship_requires_prior_charge_with_cold_guards());
    for input in inputs(&evidence) {
        untouched.step(input).expect("the fixture steps cleanly");
    }
    let open_floor: Vec<_> = untouched
        .outstanding()
        .iter()
        .map(|query| query.watermark_floor().clone())
        .collect();
    assert_eq!(retired_floor, open_floor);
}

/// A Binding whose every Event sits above the horizon still emits.
///
/// The empty in-scope Slice is the case a naive implementation drops: the
/// trigger skip removes it and the Verdict silently vanishes. It must emit,
/// carrying the disclosure that the horizon excluded everything.
#[test]
fn a_binding_whose_every_event_sits_above_the_horizon_still_emits() {
    let (mut evaluator, order) = gate_closed_evaluator(RetirementPolicy::Deadline);
    let (lowest, _) = observation_range(&adapted("happy"));

    let step = evaluator
        .step(retire_at(
            &order,
            scope_horizon(lowest - 1),
            RetirementCause::Deadline,
        ))
        .expect("a horizon below every observation is admissible");

    let verdict = &settled(&step)[0];
    assert_eq!(verdict.verdict().outcome(), VerdictOutcome::Inconclusive);
    let detail = prefix_detail(verdict);
    assert!(detail.starts_with("O-03 scope retired: deadline at"));
    assert!(
        detail.contains("were observed above the retirement horizon"),
        "the excluded evidence must be disclosed, got {detail}"
    );
}

/// Retiring a Binding the Core has not seen is admissible and yields no Verdict.
///
/// Refusing it would be order-sensitive on `{Retire(b), e_of_b}` — the exact
/// arrival-position reasoning the contract exists to avoid — and manufacturing
/// an assessment for it would re-open the empty-coverage wedge by a second
/// route.
#[test]
fn retiring_a_binding_the_core_has_not_seen_is_admissible_and_yields_no_verdict() {
    let (mut evaluator, _) = gate_closed_evaluator(RetirementPolicy::Deadline);

    assert_eq!(
        evaluator
            .step(retire(
                "no-such-order",
                ABOVE_EVERYTHING,
                RetirementCause::Deadline
            ))
            .expect("recording a lifecycle for an unseen Binding is admissible"),
        Step::Pending
    );

    let sealed = evaluator.seal().expect("sealing succeeds");
    assert!(
        sealed
            .verdicts()
            .iter()
            .all(|verdict| verdict.binding() != "no-such-order"),
        "an unseen Binding produces no Verdict"
    );
    assert!(!sealed.verdicts().is_empty(), "the seen Bindings still do");
}

/// A retired Verdict at an open gate carries no anchor-silence sentence.
///
/// That sentence attributes an Inconclusive to the evidence — no anchor-tier
/// Event witnesses the absent requirement — and a retired Binding is
/// Inconclusive for an unrelated reason. Naming the wrong cause in the
/// externally visible envelope is the defect this pins.
#[test]
fn a_retired_binding_at_an_open_gate_emits_no_anchor_silence_sentence() {
    let evidence = adapted("hazards");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_retiring(
        RetirementPolicy::Deadline,
    ));
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("events step cleanly");
    }
    let bindings: Vec<Binding> = evaluator.slices().bindings().cloned().collect();
    for binding in &bindings {
        evaluator
            .step(retire(
                binding.as_str(),
                ABOVE_EVERYTHING,
                RetirementCause::Deadline,
            ))
            .expect("admissible");
    }
    for input in source_inputs(&evidence) {
        evaluator.step(input).expect("sources step cleanly");
    }
    assert!(evaluator.evidence_is_settled(), "the gate is open here");

    let sealed = evaluator.seal().expect("sealing succeeds");
    assert!(!sealed.verdicts().is_empty());
    for verdict in sealed.verdicts() {
        let detail = prefix_detail(verdict);
        assert!(detail.starts_with("O-03 scope retired: deadline at"));
        assert!(
            !detail.contains("no anchor-tier Event witnesses"),
            "{}: {detail}",
            verdict.binding()
        );
    }
}
