//! The ceiling law: what a fact response answering *ahead* of the Core does.
//!
//! `core/tests/resumable.rs` already pins the floor (a behind-floor answer
//! re-issues), the superset case and the subset case. This file pins the
//! remaining edge — an answer **above** the Core's own evidence Watermark —
//! and it exists because that case used to be a hard `CoreError::Verdict` out
//! of `resume()`, which both violated `facts.rs`'s "a non-settling response
//! degrades; it never errors" discipline and *wedged* the evaluator: the
//! poisoned `QueryState::Settled` survived the error, `outstanding()` went to
//! zero, and the driver lost the Provisional Verdict it could have sealed.
//!
//! Two behaviours, deliberately not one:
//!
//! - **Same epoch, above the ceiling → clamp.** Monotone-sound: an answer at
//!   `(0, 17)` does imply the cold path saw through `(0, 16)`.
//! - **Different epoch → degrade.** An epoch bump is a sequence reset, so an
//!   answer at `(1, 0)` implies nothing about coverage through `(0, 16)`.
//!
//! Pure fixtures throughout: no store, no clock, no network (ADR-0012).

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod support;

use std::collections::BTreeMap;

use prost::Message;
use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::facts::{FactRequest, WatermarkPoint};
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::{
    SequenceEpoch, SequenceNumber, SourceIdentity, TierSettlement, VerdictAuthority,
};

/// Drops the epoch, matching what a Verdict's Watermark view exposes.
fn sequences(
    watermarks: &BTreeMap<SourceIdentity, WatermarkPoint>,
) -> BTreeMap<SourceIdentity, u64> {
    watermarks
        .iter()
        .map(|(source, (_, sequence))| (source.clone(), sequence.value()))
        .collect()
}

use support::{
    adapt, adapted, answer_all, delete_span, drive, encoded_verdicts, event_inputs, inputs,
    is_event, load_capture, required_watermarks, set_span_attribute, ship_requires_prior_charge,
    ship_requires_prior_charge_with_cold_guards, source, with_bounds, PAYMENT_CHARGE,
    PAYMENT_SERVICE,
};

/// Steps every input, discarding the steps the ceiling tests do not read.
fn step_all(evaluator: &mut Evaluator, inputs: Vec<Input>) -> Vec<Step> {
    inputs
        .into_iter()
        .map(|input| evaluator.step(input).expect("the fixture steps cleanly"))
        .collect()
}

/// Returns the batched request the drive produced, or asks for it now.
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

/// Shifts every required Watermark point by the given epoch and sequence delta.
fn shifted(
    watermarks: &BTreeMap<SourceIdentity, WatermarkPoint>,
    epochs: u64,
    sequences: u64,
) -> BTreeMap<SourceIdentity, WatermarkPoint> {
    watermarks
        .iter()
        .map(|(source, (epoch, sequence))| {
            (
                source.clone(),
                (
                    SequenceEpoch::new(epoch.value() + epochs),
                    SequenceNumber::new(sequence.value() + sequences),
                ),
            )
        })
        .collect()
}

/// An evaluator driven to the point where its cold-guard batch is outstanding.
fn at_the_request(evidence: &ObservedEvidence) -> (Evaluator, FactRequest) {
    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let steps = step_all(&mut evaluator, inputs(evidence));
    let request = request_after(&mut evaluator, &steps);
    (evaluator, request)
}

/// Above the ceiling within one epoch: settles, at the Core's own coordinate.
#[test]
fn a_same_epoch_answer_above_the_evidence_watermark_settles_at_the_clamped_coordinate() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let ahead = shifted(&required, 0, 1);
    let (mut evaluator, request) = at_the_request(&evidence);

    let step = evaluator
        .resume(answer_all(&request, &ahead))
        .expect("an answer ahead of the Core's evidence is a periphery input, not a Core bug");
    let verdict = match &step {
        Step::Settled(verdicts) => verdicts[0].verdict().clone(),
        other => panic!("expected Settled, got {other:?}"),
    };

    assert_eq!(verdict.authority(), VerdictAuthority::Authoritative);
    assert_eq!(verdict.cold_path(), TierSettlement::Settled);
    assert_eq!(
        verdict.cold_settlement_watermarks(),
        sequences(&required),
        "the recorded cold settlement is clamped to the Core's own evidence Watermark, \
         never to the higher coordinate the periphery claimed"
    );

    // The evidence Watermark *is* the hot settlement Watermark (`core/src/
    // verdict.rs:487` and `:507` share one `Vec`), so the clamp is observable
    // as `cold == hot` on every required source.
    let hot = verdict.hot_settlement_watermarks();
    for (source, sequence) in verdict.cold_settlement_watermarks() {
        assert_eq!(hot.get(&source), Some(&sequence));
    }
}

/// Rejoins the wire's high/low words into the `i128` they encode.
///
/// Hand-written rather than reached for from `sentinel_model`, whose own
/// splitter is private on purpose: an oracle that shares code with the encoder
/// it checks is not an oracle.
fn join_words(high: i64, low: u64) -> i128 {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&high.to_be_bytes());
    bytes[8..].copy_from_slice(&low.to_be_bytes());
    i128::from_be_bytes(bytes)
}

/// A settled cold Watermark carries the observation its own signal was seen at.
///
/// This is the only assertion in the workspace on the value
/// `core/src/verdict.rs`'s `cold_settlement_watermarks` writes into
/// `observed_at`, and it exists because the two candidates that look like they
/// cover it do not.
///
/// `guard_free_specifications_produce_byte_identical_verdicts` below pins five
/// FNV-1a digests over encoded Verdicts, so a moved byte anywhere is visible --
/// but its Specification declares **no guard**, `core/src/settlement.rs` takes
/// the `TierSettlement::NotRequired` branch when `!guards_declared`, and cold
/// Watermarks are populated only under `Settled`. All five fixtures therefore
/// emit an empty `cold_settlement_watermarks`, and the cold-path encoder
/// contributes no bytes to any pinned digest. Poisoning
/// `ObservationTime::to_wire` in `sentinel-model` cannot reveal that either: it
/// flows through both call sites, so it comes back red for the hot one and
/// establishes nothing about this one.
///
/// The test directly above reaches a settled cold path, but reads it through
/// `Verdict::cold_settlement_watermarks()`, which returns
/// `BTreeMap<SourceIdentity, u64>` and discards `observed_at` entirely. Hence
/// the decode: the field is only observable in the bytes.
///
/// The expected value is taken from the Adapter's own evidence and the words
/// are rejoined by [`join_words`] rather than by the encoder under test, so
/// this fails if the field is dropped or if the `i128` split is encoded
/// differently. Both were verified by mutation.
///
/// It does **not** establish the per-source binding, and saying so would be
/// an ADR-0031 Mode-B claim this fixture cannot support: every source in the
/// `happy` capture carries the same `ObservationTime` (`sentinel-replay`,
/// 17ns), so swapping the expected value to a different source's signal
/// leaves the test green. That was checked, not assumed. The binding itself
/// is real -- `verdict.rs` does a per-source `ledger.source(source)` lookup
/// inside the closure -- but demonstrating it needs a capture whose sources
/// observe at different times, which `happy` is not.
#[test]
fn a_settled_cold_watermark_carries_its_own_sources_completeness_observation() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let (mut evaluator, request) = at_the_request(&evidence);

    let step = evaluator
        .resume(answer_all(&request, &required))
        .expect("answering every cold guard at the Core's own coordinate settles");
    let verdict = match &step {
        Step::Settled(verdicts) => verdicts[0].verdict().clone(),
        other => panic!("expected Settled, got {other:?}"),
    };
    assert_eq!(verdict.cold_path(), TierSettlement::Settled);

    let assurance = raw_model::Verdict::decode(verdict.encode_to_vec().as_slice())
        .expect("a Verdict this crate just encoded decodes")
        .assurance
        .expect("a validated Verdict carries an Assurance Envelope");
    assert_eq!(
        assurance.cold_settlement_watermarks.len(),
        required.len(),
        "a settled cold path records one Watermark per required source"
    );

    for watermark in &assurance.cold_settlement_watermarks {
        let wire_source = watermark
            .source
            .as_ref()
            .expect("every Watermark names its source");
        let expected = evidence
            .sources()
            .values()
            .find(|record| record.source().to_wire() == *wire_source)
            .expect("every settled source supplied the evidence being verified")
            .completeness()
            .observed_at();

        let observed_at = watermark
            .observed_at
            .as_ref()
            .expect("a cold Watermark records when the completeness signal was observed");
        assert_eq!(
            observed_at.sentinel_clock_id,
            expected.sentinel_clock().to_string(),
            "{wire_source:?}: the cold Watermark names the Sentinel clock the signal was read on"
        );
        let words = observed_at
            .nanoseconds
            .as_ref()
            .expect("the observation time carries its nanoseconds");
        assert_eq!(
            join_words(words.high, words.low),
            expected.sentinel_nanoseconds(),
            "{wire_source:?}: the cold Watermark's observation coordinate"
        );
        assert_ne!(
            expected.sentinel_nanoseconds(),
            0,
            "{wire_source:?}: the fixture must supply a non-zero observation, \
             or an encoder that dropped the field would satisfy this test"
        );
    }
}

/// A different epoch is a sequence reset, so it settles nothing and re-issues.
#[test]
fn an_answer_naming_a_different_epoch_does_not_settle_and_reissues() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let other_epoch = shifted(&required, 1, 0);
    let (mut evaluator, request) = at_the_request(&evidence);

    let step = evaluator
        .resume(answer_all(&request, &other_epoch))
        .expect("an epoch mismatch degrades rather than erroring");
    assert!(
        matches!(step, Step::NeedsFacts(_)),
        "the query stays outstanding and re-issues, got {step:?}"
    );
    assert_eq!(
        evaluator.outstanding().len(),
        request.queries().len(),
        "every query is asked again; none was retired by the mismatched answer"
    );

    let sealed = evaluator.seal().expect("sealing succeeds");
    let verdict = sealed.verdicts()[0].verdict();
    assert_eq!(verdict.authority(), VerdictAuthority::Provisional);
    assert_eq!(verdict.cold_path(), TierSettlement::Pending);
    assert!(
        verdict.cold_settlement_watermarks().is_empty(),
        "nothing may be recorded from an answer in a different sequence space"
    );
}

/// The wedge regression: an above-ceiling answer must leave the evaluator usable.
///
/// This is the criterion that distinguishes "the error is gone" from "the bug
/// is fixed". Before the fix `resume()` applied every response through
/// `FactLedger::apply` *before* `advance()`, so the error left a
/// `QueryState::Settled` the ledger could never retract: `outstanding()` went
/// to zero, the query never re-issued, a later empty `resume()` raised the same
/// error and `seal()` raised it too. The driver lost a Provisional Verdict it
/// was entitled to.
#[test]
fn the_evaluator_recovers_after_an_above_ceiling_answer() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);

    for (label, answer) in [
        ("same epoch, above the ceiling", shifted(&required, 0, 1)),
        ("a different epoch", shifted(&required, 1, 0)),
        ("both", shifted(&required, 1, 1)),
    ] {
        let (mut evaluator, request) = at_the_request(&evidence);
        evaluator
            .resume(answer_all(&request, &answer))
            .unwrap_or_else(|error| panic!("{label}: resume errored with {error:?}"));

        evaluator
            .resume(Vec::new())
            .unwrap_or_else(|error| panic!("{label}: the empty resume errored with {error:?}"));

        let sealed = evaluator
            .seal()
            .unwrap_or_else(|error| panic!("{label}: seal errored with {error:?}"));
        assert_eq!(
            sealed.verdicts().len(),
            1,
            "{label}: the driver keeps the Verdict it was entitled to"
        );
    }
}

/// No periphery-supplied `settled_at` may ever surface as a `CoreError`.
///
/// `core/src/facts.rs`'s module doc reserves `CoreError::Verdict` for a Core
/// bug and `CoreError::FactResponseMismatch` for exactly one malformed input:
/// a response echoing a query that was never issued. Everything else degrades.
/// A required source with no completeness boundary declares no query — and it
/// takes **two** independent gate conditions to stop it, not one.
///
/// This started as a test claiming it would fail when #17 relaxes
/// `watermarks_cover_epochs`, and that claim was false. Recording why, because
/// the false version is the more natural thing to write.
///
/// `declare_queries` is reached only from `advance`'s
/// `gate.is_open() && gate.watermarks_cover_epochs()`. Stripping a required
/// source's `complete_through` does set `watermarks_cover_epochs` false — but
/// it *also* closes `is_open()`, because `EvidenceGate::evaluate` then computes
/// `boundary = complete_through.map_or(0, ..)` and asks
/// `covers_watermark(source, epoch, 0)`, which requires
/// `receipts.distinct == 0 && receipts.highest == 0`. A source that delivered
/// Events fails that, so `GateReason::IncompleteDeliveryToWatermark` is
/// inserted and the gate shuts on a reason rather than on the coverage flag.
///
/// Verified by mutation: with `gate.watermarks_cover_epochs()` deleted from the
/// branch — the exact #17 shape — this test still passes. The same harness with
/// the substitution removed *does* fail under that mutation, so the harness is
/// sound and it really is the second guard doing the work.
///
/// So the honest claim is narrower than the one this test used to make:
/// `clamp_to_evidence`'s `Ordering::Greater` arm is guarded twice over, and
/// relaxing the coverage flag alone does **not** make it reachable. Whoever
/// lands #17 has to defeat `IncompleteDeliveryToWatermark` as well before the
/// arm goes live, and this test is what tells them the first change was not
/// enough.
#[test]
fn a_required_source_without_a_boundary_declares_no_query() {
    let evidence = adapted("happy");
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");
    let sample = evidence
        .events()
        .iter()
        .find(|event| {
            event
                .producer()
                .kind()
                .as_str()
                .starts_with("sagashop.payment.")
        })
        .expect("payment emitted an Event")
        .clone();

    // Emitted through its last sequence but declared no completeness boundary:
    // observed, contiguous, and simply silent about how far it is complete.
    // `last_emitted` must stay populated or the record reads as non-contiguous
    // and `RequiredSourceGap` closes the gate for an unrelated reason, which
    // would mask the clause under test.
    let last_emitted = record
        .completeness()
        .last_emitted()
        .expect("the fixture declares a last emitted sequence")
        .value();
    let unbounded = with_bounds(record, &sample, 0, Some(last_emitted), None);

    let mut evaluator = Evaluator::new(ship_requires_prior_charge_with_cold_guards());
    let mut steps = Vec::new();
    for input in event_inputs(&evidence) {
        steps.push(evaluator.step(input).expect("events step cleanly"));
    }
    for held in evidence.sources().values() {
        let substituted = if held.source() == &payment {
            unbounded.clone()
        } else {
            held.clone()
        };
        steps.push(
            evaluator
                .step(Input::source(substituted))
                .expect("sources step cleanly"),
        );
    }
    // Queries are declared inside `advance`, which runs on `step` as well as on
    // `resume`, so both have to be inspected. Looking only at `resume` was the
    // first version of this test, and it stayed green under the mutation below
    // because the request had already been handed back during stepping.
    steps.push(
        evaluator
            .resume(Vec::new())
            .expect("an empty resume is accepted"),
    );

    for step in &steps {
        assert!(
            !matches!(step, Step::NeedsFacts(_)),
            "the gate admitted a query while a required source declared no \
             completeness boundary. Both guards that prevent this -- the \
             watermarks_cover_epochs flag and GateReason::IncompleteDeliveryToWatermark \
             -- have now been defeated, so clamp_to_evidence's Ordering::Greater \
             arm is reachable and its documentation needs revisiting"
        );
    }
}

/// Every declared query's floor names every required source.
///
/// This is the invariant that makes `clamp_to_evidence`'s [`Ordering::Greater`]
/// arm unreachable from the `Evaluator`, and it is worth asserting rather than
/// deriving because the derivation runs through four files. `declare_queries`
/// is reached only inside `advance`'s
/// `gate.is_open() && gate.watermarks_cover_epochs()` branch, and that flag is
/// false whenever a required source lacks a `complete_through` or has no ledger
/// record — exactly the two cases where `evidence_watermarks` would omit it.
///
/// **The point of the test is that it fails when that stops being true.** The
/// gate is deliberately global rather than per-Binding (`precedence.rs`), and
/// **#17** owns making it per-Binding. A per-Binding gate is precisely a gate
/// that can open while some required source still lacks a completeness
/// boundary — at which point the floor omits a source, `settles` no longer pins
/// its epoch, and the `Greater` arm goes live. That is a real behavioural
/// change: a Verdict that was `Provisional` becomes a degrade.
///
/// Testing the arm's behaviour would not catch that; the arm behaves correctly
/// either way. Testing the invariant catches it, and fails exactly when someone
/// must come back and read the argument again.
#[test]
fn every_declared_query_floor_names_every_required_source() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);
    let (_evaluator, request) = at_the_request(&evidence);

    assert!(
        !request.queries().is_empty(),
        "no queries were declared, so the invariant would hold vacuously"
    );

    for query in request.queries() {
        for source in required.keys() {
            assert!(
                query.watermark_floor().contains_key(source),
                "query for {:?} declared a floor omitting required source {source:?}; \
                 if this is #17 making the evidence gate per-Binding, then \
                 clamp_to_evidence's Ordering::Greater arm is now reachable and \
                 its documentation needs revisiting",
                query.binding()
            );
        }
    }
}

#[test]
fn a_periphery_response_never_produces_a_core_error() {
    let evidence = adapted("happy");
    let required = required_watermarks(&evidence);

    let hostile: [(&str, BTreeMap<SourceIdentity, WatermarkPoint>); 6] = [
        ("one past the ceiling", shifted(&required, 0, 1)),
        ("far past the ceiling", shifted(&required, 0, 1_000_000)),
        ("the next epoch", shifted(&required, 1, 0)),
        ("a far epoch", shifted(&required, u64::from(u32::MAX), 0)),
        ("both dimensions ahead", shifted(&required, 1, 1_000_000)),
        (
            "one source ahead, the rest exact",
            first_source_shifted(&required),
        ),
    ];

    for (label, answer) in hostile {
        let (mut evaluator, request) = at_the_request(&evidence);
        let outcome = evaluator.resume(answer_all(&request, &answer));
        assert!(
            outcome.is_ok(),
            "{label}: a periphery input surfaced as {:?}",
            outcome.err()
        );
        assert!(
            evaluator.seal().is_ok(),
            "{label}: the evaluator was left unsealable"
        );
    }
}

/// Shifts only the lowest-ordered source: one source ahead is sufficient.
fn first_source_shifted(
    watermarks: &BTreeMap<SourceIdentity, WatermarkPoint>,
) -> BTreeMap<SourceIdentity, WatermarkPoint> {
    let mut shifted = watermarks.clone();
    if let Some((_, point)) = shifted.iter_mut().next() {
        *point = (point.0, SequenceNumber::new(point.1.value() + 1));
    }
    shifted
}

/// A Specification declaring no guard is byte-for-byte untouched by all of this.
///
/// The settlement rule reproduces #10's coverage-driven behaviour exactly when
/// no guard is declared (`core/src/settlement.rs`'s module doc), and the
/// ceiling fix must not move a single byte of it. The digests below were taken
/// from `main` **before** the clamp fix landed, then retaken when #164 started
/// writing `activation_barrier = NOT_DECLARED` (proto3 omits `UNSPECIFIED`,
/// not `NOT_DECLARED`), and again for #206 when the envelope gained retention /
/// redaction fields 24–28. A guard-free Verdict that moves again breaks this
/// test rather than being discovered downstream. `violated` is
/// included because it is the fixture carrying a `DecisiveViolation`
/// settlement basis, whose witness bound is the field the clamp writes into.
///
/// A digest is the only instrument that can see byte-identity, and it is a
/// poor diagnostic: red here says a byte moved and nothing about which. The
/// encoders whose output is inside these bytes are
/// `SourceIdentity::to_wire` and `ObservationTime::to_wire`, so start there.
/// Note what is *not* inside them: these fixtures declare no guard, so the
/// cold path is `NotRequired`, `cold_settlement_watermarks` is empty in all
/// five, and `core/src/verdict.rs`'s cold-path encoder contributes nothing
/// here -- `a_settled_cold_watermark_carries_its_own_sources_completeness_observation`
/// is what covers that one.
#[test]
fn guard_free_specifications_produce_byte_identical_verdicts() {
    let specification = ship_requires_prior_charge();

    let mut absent = load_capture("happy");
    delete_span(&mut absent, &|span| is_event(span, PAYMENT_CHARGE));
    let mut violated = load_capture("happy");
    set_span_attribute(
        &mut violated,
        &|span| is_event(span, PAYMENT_CHARGE),
        "sagashop.payment.status",
        "duplicate",
    );

    let fixtures: [(&str, usize, u64, ObservedEvidence); 5] = [
        ("happy", 1, 0x033c_74e9_bb49_d5cb, adapted("happy")),
        // The cancel capture ships no Binding at all; pinned as zero so a
        // future fixture change that starts emitting one is visible here.
        ("cancel", 0, 0xcbf2_9ce4_8422_2325, adapted("cancel")),
        ("hazards", 1, 0xba33_e387_6d65_fc3b, adapted("hazards")),
        ("absent-anchor", 1, 0x369d_e964_835a_36ed, adapt(&absent)),
        ("violated", 1, 0xbbee_8cd2_cc2c_3653, adapt(&violated)),
    ];

    for (label, count, digest, evidence) in fixtures {
        let sealed = drive(&specification, inputs(&evidence)).expect("the guard-free spec seals");
        assert_eq!(sealed.len(), count, "{label}: the Binding count moved");
        assert_eq!(
            fnv1a(&encoded_verdicts(&sealed)),
            digest,
            "{label}: a guard-free Verdict moved"
        );
    }
}

/// FNV-1a over every encoded Verdict, Binding key included.
///
/// A hand-rolled digest rather than a dev-dependency: the property under test
/// is "these bytes did not move", and sixteen lines buy that without adding a
/// crate to a workspace whose Core links two dependencies on purpose.
fn fnv1a(verdicts: &[(String, Vec<u8>)]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (binding, encoded) in verdicts {
        for byte in binding.as_bytes().iter().chain(encoded.iter()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}
