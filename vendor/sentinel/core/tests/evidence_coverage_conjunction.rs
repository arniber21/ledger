//! Evidence Coverage is the conjunction of delivered Events and declared
//! evidence, and `seal()` never panics when a driver supplies only one of them.
//!
//! Regression coverage for #158. Before the fix, `Coverage.covered` was derived
//! from `observed_sources()` (populated by `record_delivery`) while
//! `assurance_envelope` assumed a matching `SourceEvidence` record (populated
//! only by `record_source`), and asserted the conjunction with
//! `.expect("covered sources supplied evidence")`. Nothing established that
//! invariant, so an ordinary driver — deliver Events, forget one
//! `record_source`, `seal()` — aborted the process.
//!
//! # What each direction of the conjunction is pinned by
//!
//! Coverage fails in two ways, and this file pins both so the conjunction
//! cannot silently collapse into either half:
//!
//! - **Delivered, never declared** (the half that panicked). Owned here:
//!   [`a_source_that_only_delivers_does_not_panic_the_seal`] for the Verdict
//!   shape, [`evidence_coverage_excludes_a_source_that_declared_nothing`] for
//!   the population itself, and
//!   [`delivering_without_declaring_is_refused_by_name`] when *no* required
//!   source declares.
//! - **Declared, never delivered** (the mirror). Owned here by
//!   [`a_source_that_declares_without_delivering_is_not_covered`], and
//!   end-to-end by `tracer_bullet.rs`'s
//!   `absent_required_source_is_provisional_not_a_pass`, whose twelve-assertion
//!   shape block the partial-coverage test here deliberately mirrors: the two
//!   causes are different, the Verdict they produce is the same, and that
//!   sameness is the substantive claim #158's fix makes.
//!
//! ADR-0031 §3: these are executed, not argued. The partial-coverage scenario
//! panicked before the fix, at `core/src/verdict.rs:309: covered sources
//! supplied evidence`.

mod support;

use std::collections::BTreeSet;

use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::evidence::GateReason;
use sentinel_core::CoreError;
use sentinel_model::{
    InconclusiveReason, SourceIdentity, TierSettlement, Verdict, VerdictAuthority, VerdictOutcome,
};
use support::{
    adapt, adapted, delete_all_spans_of_service, event_inputs, load_capture,
    ship_requires_prior_charge, ship_requires_prior_charge_without_required_sources, source,
    ORDER_SERVICE, PAYMENT_SERVICE,
};

/// G1's Assurance Envelope sentence, pinned as text.
///
/// The wording is the deliverable, not an implementation detail: it is what an
/// operator reads off a Verdict to learn what to repair.
const G1_SENTENCE: &str = "G1 a required source supplied no Event";

/// G1b's Assurance Envelope sentence, pinned as text.
const G1B_SENTENCE: &str = "G1b a required source delivered Events but declared no evidence";

/// Drives the `happy` capture with one required source's record withheld.
///
/// The withheld source delivers every Event it has and declares nothing: half
/// of Evidence Coverage, which is not some of it.
fn delivering_but_undeclared(withheld: &SourceIdentity) -> Evaluator {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }
    for (identity, record) in evidence.sources() {
        if identity == withheld {
            continue;
        }
        evaluator
            .step(Input::source(record.clone()))
            .expect("source evidence steps");
    }
    evaluator
}

/// Drives the `happy` capture with one required source's Events withheld.
///
/// The mirror of [`delivering_but_undeclared`]: the withheld source declares
/// itself in full and delivers nothing. Its record is taken from the unmodified
/// capture, because the Adapter emits no record for a service with no spans.
fn declared_but_undelivered(withheld: &str) -> Evaluator {
    let declared = adapted("happy");
    let mut capture = load_capture("happy");
    delete_all_spans_of_service(&mut capture, withheld);
    let delivered = adapt(&capture);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in event_inputs(&delivered) {
        evaluator.step(input).expect("Events step");
    }
    for record in declared.sources().values() {
        evaluator
            .step(Input::source(record.clone()))
            .expect("source evidence steps");
    }
    evaluator
}

/// Seals an evaluator and returns its one Verdict.
///
/// The count assertion is load bearing rather than decorative: every shape
/// assertion in this file reads the Verdict this returns, and a `seal()` that
/// stopped emitting for a gated Binding would otherwise turn those tests
/// green-and-empty while the regression walked back in.
fn sole_verdict(evaluator: Evaluator) -> Verdict {
    let sealed = evaluator
        .seal()
        .expect("partial Evidence Coverage still yields a Verdict");
    assert_eq!(
        sealed.verdicts().len(),
        1,
        "the happy capture ships exactly one order, so exactly one Binding",
    );
    sealed.verdicts()[0].verdict().clone()
}

/// Returns the Assurance Envelope's uncertainty sentences.
fn uncertainty_sentences(verdict: &Verdict) -> Vec<String> {
    verdict
        .uncertainty()
        .into_iter()
        .map(|entry| entry.detail)
        .collect()
}

/// Partial coverage is a Verdict, not a panic and not a refusal.
///
/// One required source declares its evidence, the other only delivers Events.
/// The Core has enough to build an Assurance Envelope, so it must build one —
/// naming the gap rather than refusing or, as before #158, aborting.
///
/// The shape block below is `tracer_bullet.rs`'s
/// `absent_required_source_is_provisional_not_a_pass`, assertion for assertion,
/// over the *other* cause. That the two agree is the claim: a required source
/// that delivered nothing and one that declared nothing are both simply
/// uncovered, and #158's fix is what stops them being different.
#[test]
fn a_source_that_only_delivers_does_not_panic_the_seal() {
    let withheld = source(PAYMENT_SERVICE);
    let verdict = sole_verdict(delivering_but_undeclared(&withheld));

    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
    assert_ne!(verdict.outcome(), VerdictOutcome::Violate);
    assert_eq!(
        verdict.inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(verdict.hot_path(), TierSettlement::Settled);
    assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    assert!(verdict.cold_settlement_watermarks().is_empty());
    assert_eq!(verdict.settlement_basis(), None);

    let observed = verdict.observed_sources();
    let required = verdict.required_sources();
    assert!(
        observed.is_subset(&required) && observed != required,
        "observed_sources is a strict subset of required_sources"
    );
    assert!(required.contains(&withheld));
    assert!(
        !observed.contains(&withheld),
        "a source that declared nothing is not covered, however much it delivered"
    );

    let tier_gap = verdict
        .source_tier_gap()
        .expect("a conclusion resting on no anchor Event carries a mandatory tier gap");
    assert!(
        tier_gap.unanchored_sources.contains(&withheld),
        "the uncovered source supplies no anchor Event, so the gap names it"
    );

    // The envelope names the fault, and names it truly. This source supplied
    // every Event it has, so G1's sentence would be a lie about it — which is
    // the entire reason G1b is a separate disjunct.
    let sentences = uncertainty_sentences(&verdict);
    assert!(
        sentences.iter().any(|detail| detail.contains(G1B_SENTENCE)),
        "the Verdict must carry G1b's sentence, got {sentences:?}"
    );
    assert!(
        !sentences.iter().any(|detail| detail.contains(G1_SENTENCE)),
        "G1's sentence is untrue of a source that delivered plenty, got {sentences:?}"
    );
}

/// The invariant stated where it lives, rather than only where it is consumed.
///
/// `EvidenceLedger::coverage` is the single constructor, so this is the one
/// test that fails if coverage itself regresses rather than only if a
/// downstream reader does.
#[test]
fn evidence_coverage_excludes_a_source_that_declared_nothing() {
    let withheld = source(PAYMENT_SERVICE);
    let declared = source(ORDER_SERVICE);
    let evaluator = delivering_but_undeclared(&withheld);
    let ledger = evaluator.ledger();
    let coverage = ledger.coverage(ship_requires_prior_charge().required_source_identities());

    // The delivery half is present ...
    assert!(
        ledger.observed_sources().contains(&withheld),
        "the withheld source delivered Events"
    );
    // ... and the conjunction is not.
    assert!(!coverage.is_covered(&withheld));
    assert_eq!(coverage.record(&withheld), None);
    assert!(
        coverage.undeclared().contains(&withheld),
        "delivered but never declared is the actionable diagnosis"
    );
    assert!(!coverage.is_complete());
    assert!(
        !coverage.is_empty(),
        "the other required source supplied both halves"
    );

    // A covered source arrives with its record attached. There is no second
    // lookup left to fail, which is the whole of #158's fix.
    assert!(coverage.is_covered(&declared));
    assert_eq!(
        coverage.record(&declared),
        ledger.source(&declared),
        "a covered source carries the ledger's own record"
    );
    assert_eq!(
        coverage.covered().keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([declared]),
        "covered names exactly the sources holding both halves"
    );
    assert!(
        coverage
            .covered()
            .keys()
            .all(|source| coverage.required().contains(source)),
        "covered is drawn from required and nowhere else"
    );
}

/// The mirror half: declaring is no more sufficient than delivering.
///
/// Pins the conjunction in the other direction, and pins the *diagnosis* apart
/// from it — a silent source is uncovered but it is not `undeclared`, because
/// its driver did call `record_source`.
#[test]
fn a_source_that_declares_without_delivering_is_not_covered() {
    let silent = source(PAYMENT_SERVICE);
    let evaluator = declared_but_undelivered(PAYMENT_SERVICE);
    let ledger = evaluator.ledger();
    let coverage = ledger.coverage(ship_requires_prior_charge().required_source_identities());

    // The declaration half is present ...
    assert!(ledger.source(&silent).is_some());
    // ... the delivery half is not ...
    assert!(!ledger.observed_sources().contains(&silent));
    // ... so the conjunction is not.
    assert!(!coverage.is_covered(&silent));
    assert!(!coverage.is_complete());
    assert!(
        !coverage.undeclared().contains(&silent),
        "a silent source declared its evidence: uncovered, but not undeclared"
    );
    assert!(coverage.undeclared().is_empty());

    let verdict = sole_verdict(evaluator);
    assert_eq!(verdict.outcome(), VerdictOutcome::Inconclusive);
    assert_ne!(verdict.outcome(), VerdictOutcome::Satisfy);
    assert!(!verdict.observed_sources().contains(&silent));

    // The other sentence, for the other cause.
    let sentences = uncertainty_sentences(&verdict);
    assert!(
        sentences.iter().any(|detail| detail.contains(G1_SENTENCE)),
        "a source that supplied no Event gets G1's sentence, got {sentences:?}"
    );
    assert!(
        !sentences.iter().any(|detail| detail.contains(G1B_SENTENCE)),
        "nothing here delivered without declaring, got {sentences:?}"
    );
}

/// The two uncovered disjuncts say different things, and both reach a Verdict.
///
/// `GateReason::UndeclaredRequiredSource` exists for exactly one reason: to
/// keep "a required source supplied no Event" out of the Assurance Envelope of
/// a Verdict whose source supplied plenty. Collapsing it back into G1 would be
/// caught by nothing otherwise.
#[test]
fn the_two_uncovered_halves_carry_different_sentences() {
    assert_eq!(GateReason::UnobservedRequiredSource.detail(), G1_SENTENCE);
    assert_eq!(GateReason::UndeclaredRequiredSource.detail(), G1B_SENTENCE);
    assert_ne!(
        GateReason::UndeclaredRequiredSource.detail(),
        GateReason::UnobservedRequiredSource.detail(),
        "two disjuncts that said the same thing would not need to be two"
    );

    let undeclared = uncertainty_sentences(&sole_verdict(delivering_but_undeclared(&source(
        PAYMENT_SERVICE,
    ))));
    let unobserved =
        uncertainty_sentences(&sole_verdict(declared_but_undelivered(PAYMENT_SERVICE)));
    assert_ne!(
        undeclared, unobserved,
        "the same uncovered source, two causes, two envelope sentences"
    );
}

/// Total absence of Evidence Coverage is a typed error, never a panic.
///
/// Every required source delivers Events and none declares evidence. No
/// Assurance Envelope is constructible — `sentinel_model` admits no Verdict at
/// any authority whose `hot_path` is not `SETTLED`, and rejects a settled tier
/// carrying no settlement Watermark — so refusing is the only answer that
/// neither fabricates evidence nor passes.
#[test]
fn delivering_without_declaring_is_refused_by_name() {
    let evidence = adapted("happy");
    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    for input in event_inputs(&evidence) {
        evaluator.step(input).expect("Events step");
    }

    match evaluator.seal() {
        Err(CoreError::NoEvidenceCoverage {
            required,
            undeclared,
        }) => {
            // Named, not compared as sets: `undeclared == required` holds only
            // because both required sources happen to deliver in this capture,
            // and a fixture gaining a third source would break that identity
            // without any behaviour changing.
            assert!(required.contains(&source(ORDER_SERVICE)));
            assert!(required.contains(&source(PAYMENT_SERVICE)));
            assert!(undeclared.contains(&source(ORDER_SERVICE)));
            assert!(undeclared.contains(&source(PAYMENT_SERVICE)));
            assert!(undeclared.is_subset(&required));
        }
        other => panic!("expected NoEvidenceCoverage, got {other:?}"),
    }
}

/// A Specification requiring nothing is refused as a Specification defect.
///
/// An empty required-source set makes Evidence Coverage vacuously empty, and
/// reporting that as [`CoreError::NoEvidenceCoverage`] would name a deployment
/// fault with no sources to instrument. The two defects have different repairs,
/// so they get different errors.
///
/// Note where it fires: with nothing required the evidence gate is trivially
/// open, so `step` reaches Verdict construction and refuses there. For every
/// Specification that does require a source, an open gate *is* complete
/// coverage, which is why `NoEvidenceCoverage` can only come from `seal`.
#[test]
fn a_specification_requiring_no_source_is_refused_as_a_specification_defect() {
    let evidence = adapted("happy");
    let specification = ship_requires_prior_charge_without_required_sources();
    assert!(specification.required_sources().is_empty());

    let mut evaluator = Evaluator::new(specification.clone());
    let mut refusal = None;
    for input in event_inputs(&evidence) {
        if let Err(error) = evaluator.step(input) {
            refusal = Some(error);
            break;
        }
    }

    let Some(error) = refusal else {
        panic!("a Specification requiring no source must be refused while stepping");
    };
    match &error {
        CoreError::SpecificationRequiresNoSource {
            specification: named,
        } => assert_eq!(named, specification.identity()),
        other => panic!("expected SpecificationRequiresNoSource, got {other:?}"),
    }

    // The message must diagnose the Specification, not the evidence.
    let message = error.to_string();
    assert!(
        message.contains("declares no required source"),
        "the message names the defect, got {message:?}"
    );
    assert!(
        !message.contains("Evidence Coverage"),
        "this is not an evidence condition, got {message:?}"
    );
}
