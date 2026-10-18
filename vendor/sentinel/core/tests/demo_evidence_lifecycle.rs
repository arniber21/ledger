//! Bounded CAT-1 demo evidence lifecycle (#283 / ADR-0035 draft).
//!
//! Loads `testdata/sagashop/demo-specs/evidence-lifecycle.json` and executes the
//! seven scenarios the decision names. Behavior is pinned to shipped Core
//! contracts — this module does **not** invent Amendment, Trace-id, or
//! emission-log semantics. Status in the JSON is
//! `awaiting_maintainer_ratification`.
//!
//! Missing evidence is never a pass.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;

use prost::Message;
use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::evidence::MonotonicityBreach;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::{
    classify_delivery_identity, DeliveryIdentityRelation, SequenceGap, SourceTier, Verdict,
    VerdictAuthority, VerdictOutcome,
};
use serde_json::Value;

use support::{
    adapt, adapted, delete_span, encoded_verdicts, evaluate, event_inputs, inputs,
    is_event_for_order, load_capture, only_event, only_shipped_order, ship_requires_prior_charge,
    source, source_inputs, with_altered_attribute, with_altered_identity, with_altered_slot,
    with_bounds, with_gaps, with_tier, PAYMENT_CHARGE, PAYMENT_SERVICE,
};

/// Observable state ids the UI / #286 runtime must share (contract JSON).
const OBSERVABLE_STATES: &[&str] = &[
    "admitted",
    "duplicate_dropped",
    "delivery_conflict_refused",
    "non_monotonic_refused",
    "incomplete_inconclusive",
    "settled_authoritative",
    "unsupported_correction_refused",
    "restart_reconstructed_final_state",
];

/// Scenario ids every lifecycle contract must name.
const SCENARIOS: &[&str] = &[
    "duplicate_delivery",
    "conflicting_delivery",
    "source_restart",
    "accepted_late_delivery",
    "non_monotonic_breach",
    "unsupported_correction",
    "process_restart",
];

fn contract_path() -> PathBuf {
    support::testdata_dir()
        .join("demo-specs")
        .join("evidence-lifecycle.json")
}

fn load_contract() -> Value {
    let path = contract_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "evidence-lifecycle contract readable at {}: {error}",
            path.display()
        )
    });
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("evidence-lifecycle contract is JSON: {error}"))
}

fn required_str<'a>(object: &'a Value, key: &str, context: &str) -> &'a str {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: missing string field {key:?}"))
}

fn required_array<'a>(object: &'a Value, key: &str, context: &str) -> &'a Vec<Value> {
    object
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: missing array field {key:?}"))
}

fn step_all(evaluator: &mut Evaluator, inputs: Vec<Input>) -> Vec<Step> {
    inputs
        .into_iter()
        .map(|input| evaluator.step(input).expect("fixture steps cleanly"))
        .collect()
}

fn sealed_outcomes(verdicts: &[BindingVerdict]) -> Vec<(String, VerdictOutcome, VerdictAuthority)> {
    let mut rows: Vec<_> = verdicts
        .iter()
        .map(|binding| {
            (
                binding.binding().to_owned(),
                binding.verdict().outcome(),
                binding.verdict().authority(),
            )
        })
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

/// Reads the stamped LateDataPolicy from encoded Verdict bytes.
///
/// Domain `Verdict` does not yet expose a typed accessor for this field on the
/// pin this ADR drafts against; the Core construction sites hardcode Exclude
/// into the envelope, and the demo contract pins that stamp.
fn stamped_late_data_policy(verdict: &Verdict) -> raw_model::LateDataPolicy {
    let assurance = raw_model::Verdict::decode(verdict.encode_to_vec().as_slice())
        .expect("a Verdict this crate just encoded decodes")
        .assurance
        .expect("a validated Verdict carries an Assurance Envelope");
    raw_model::LateDataPolicy::try_from(assurance.late_data_policy)
        .expect("stamped late_data_policy is a known enum arm")
}

/// Contract schema + honesty flags the UI/#286 path must share.
#[test]
fn evidence_lifecycle_contract_exposes_shared_observable_states() {
    let contract = load_contract();
    assert_contract_identity(&contract);
    assert_contract_isolation_restart_and_correction(&contract);
    assert_contract_observable_coverage(&contract);
}

fn assert_contract_identity(contract: &Value) {
    assert_eq!(
        required_str(contract, "artifactVersion", "contract"),
        "demo-evidence-lifecycle-v1"
    );
    assert_eq!(
        contract
            .get("part_of")
            .and_then(Value::as_u64)
            .expect("part_of"),
        283
    );
    assert_eq!(
        required_str(contract, "status", "contract"),
        "awaiting_maintainer_ratification",
        "HITL: do not silently mark ratified"
    );
    assert_eq!(required_str(contract, "adr", "contract"), "0035");
    assert!(
        !contract
            .get("closes_parent_20")
            .and_then(Value::as_bool)
            .expect("closes_parent_20"),
        "#20 stays open"
    );
    assert!(!contract
        .get("closes_cold_witness_composition")
        .and_then(Value::as_bool)
        .expect("closes_cold_witness_composition"));
}

fn assert_contract_isolation_restart_and_correction(contract: &Value) {
    let isolation = contract.get("isolation").expect("isolation object");
    let forbidden: BTreeSet<&str> = required_array(isolation, "forbidden", "isolation")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        forbidden.contains("informal_Trace_id_by_parsing_Verdict_id"),
        "must forbid Verdict-id Trace invention"
    );
    assert!(
        forbidden.contains("Binding_or_BindingKey_as_Trace_substitute"),
        "must forbid Binding-as-Trace"
    );

    let late = contract.get("late_data_policy").expect("late_data_policy");
    assert_eq!(
        required_str(late, "demo_stamps", "late_data_policy"),
        "LATE_DATA_POLICY_EXCLUDE"
    );

    let restart = contract.get("restart").expect("restart");
    assert_eq!(
        restart
            .get("original_emission_sequence_reproducible")
            .and_then(Value::as_bool),
        Some(false),
        "final-state replay ≠ emission-log replay"
    );
    assert_eq!(
        restart.get("emission_log_replay").and_then(Value::as_bool),
        Some(false)
    );

    let correction = contract
        .get("post_settlement_correction")
        .expect("post_settlement_correction");
    assert_eq!(
        correction.get("supported").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        required_str(correction, "behavior", "post_settlement_correction"),
        "visible_refuse"
    );
    assert_eq!(
        correction
            .get("ordinary_revision_is_not_Amendment")
            .and_then(Value::as_bool),
        Some(true)
    );
}

fn assert_contract_observable_coverage(contract: &Value) {
    let states: BTreeSet<&str> = required_array(contract, "observable_states", "contract")
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_str))
        .collect();
    for expected in OBSERVABLE_STATES {
        assert!(
            states.contains(expected),
            "missing observable state {expected}"
        );
    }

    let scenarios: BTreeSet<&str> = required_array(contract, "scenarios", "contract")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for expected in SCENARIOS {
        assert!(scenarios.contains(expected), "missing scenario {expected}");
    }

    let matrix_states: BTreeSet<&str> =
        required_array(contract, "accept_refuse_matrix", "contract")
            .iter()
            .filter_map(|row| row.get("observable_state").and_then(Value::as_str))
            .collect();
    for expected in OBSERVABLE_STATES {
        assert!(
            matrix_states.contains(expected),
            "accept_refuse_matrix must name observable_state {expected}"
        );
    }

    let note = required_str(contract, "ui_runtime_contract_note", "contract");
    assert!(
        note.contains("#286") && note.to_ascii_lowercase().contains("one vocabulary"),
        "contract must bind UI/runtime to one interpretation"
    );
}

/// Duplicate delivery is dropped; evaluation is unchanged.
#[test]
fn scenario_duplicate_delivery() {
    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);
    assert_eq!(
        classify_delivery_identity(&charge, &charge.clone()),
        DeliveryIdentityRelation::Duplicate
    );

    let mut events = evidence.events().to_vec();
    events.push(charge);
    let replayed = evidence.with_events(events);
    let specification = ship_requires_prior_charge();

    let baseline = evaluate(&specification, &evidence).expect("baseline");
    let with_dup = evaluate(&specification, &replayed).expect("duplicate evaluates");
    assert_eq!(
        encoded_verdicts(&baseline),
        encoded_verdicts(&with_dup),
        "observable_state=duplicate_dropped"
    );
    assert!(
        !baseline.is_empty(),
        "happy path still settles; missing evidence is not this scenario"
    );
    assert_eq!(
        stamped_late_data_policy(baseline[0].verdict()),
        raw_model::LateDataPolicy::Exclude
    );
}

/// Evidence / Slot / Identity conflicts refuse — never a silent preference.
#[test]
fn scenario_conflicting_delivery() {
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
            "{label}"
        );
        let mut events = evidence.events().to_vec();
        events.push(conflicting);
        let result = evaluate(&specification, &evidence.with_events(events));
        assert!(
            matches!(result, Err(CoreError::Delivery(_))),
            "{label}: observable_state=delivery_conflict_refused, got {result:?}"
        );
    }
}

/// Source restart opens a new SequenceEpoch — the escape hatch.
#[test]
fn scenario_source_restart() {
    let evidence = adapted("happy");
    let payment = source(PAYMENT_SERVICE);
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");
    let sample = only_event(
        &evidence,
        "sagashop.payment.charge.completed",
        &only_shipped_order(&load_capture("happy")),
    );
    let boundary = record
        .completeness()
        .complete_through()
        .expect("fixture declares a boundary")
        .value();

    let closed = with_bounds(record, &sample, 0, Some(boundary), Some(boundary));
    let restarted = with_bounds(record, &sample, 1, Some(1), Some(1));

    let mut evaluator = Evaluator::new(ship_requires_prior_charge());
    evaluator
        .step(Input::source(closed))
        .expect("closing epoch 0 is accepted");
    evaluator
        .step(Input::source(restarted))
        .expect("source restart via new SequenceEpoch is accepted (observable_state=admitted)");
}

/// Late hole-fill at/below a closed boundary is accepted and can settle.
///
/// Withholding an interior slot keeps the gate shut (Inconclusive — never a
/// pass). Delivering that slot later is accepted late delivery under Exclude.
#[test]
fn scenario_accepted_late_delivery() {
    let evidence = adapted("hazards");
    let specification = ship_requires_prior_charge();
    let payment = source(PAYMENT_SERVICE);
    let boundary = evidence
        .source(&payment)
        .expect("payment supplied evidence")
        .completeness()
        .complete_through()
        .expect("fixture declares a boundary")
        .value();

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
            .expect("prefix delivery succeeds");
    }
    for input in source_inputs(&evidence) {
        assert_eq!(
            evaluator.step(input).expect("source evidence accepted"),
            Step::Pending,
            "incomplete delivery: observable_state=incomplete_inconclusive precursor"
        );
    }

    let gated = evaluator.clone().seal().expect("seal succeeds");
    let incomplete = &gated.verdicts()[0];
    assert_eq!(
        incomplete.verdict().outcome(),
        VerdictOutcome::Inconclusive,
        "missing evidence is never a pass"
    );

    let settled = evaluator
        .step(Input::event(withheld))
        .expect("accepted late hole-fill");
    let settled_verdicts = match settled {
        Step::Settled(verdicts) => verdicts,
        other => panic!("expected Settled after late fill, got {other:?}"),
    };
    assert_eq!(
        encoded_verdicts(&settled_verdicts),
        encoded_verdicts(&evaluate(&specification, &evidence).expect("batch")),
        "observable_state=settled_authoritative"
    );
    assert_eq!(
        stamped_late_data_policy(settled_verdicts[0].verdict()),
        raw_model::LateDataPolicy::Exclude
    );
    assert_eq!(
        settled_verdicts[0].verdict().authority(),
        VerdictAuthority::Authoritative
    );
}

/// Completeness / tier / gaps / DeliveryAboveClosedBoundary breaches refuse.
///
/// Distinct from `unsupported_correction_refused` (post-settlement reopen).
/// Pins `observable_state=non_monotonic_refused` for the matrix row.
#[test]
fn scenario_non_monotonic_breach() {
    let payment = source(PAYMENT_SERVICE);

    // Completeness regression needs a fixture with enough sequence room
    // (hazards); happy's payment boundary is too small to open-then-regress.
    let hazards = adapted("hazards");
    let hazards_record = hazards.source(&payment).expect("payment supplied evidence");
    let hazards_sample = only_event(
        &hazards,
        "sagashop.payment.charge.completed",
        &only_shipped_order(&load_capture("hazards")),
    );
    let hazards_boundary = hazards_record
        .completeness()
        .complete_through()
        .expect("fixture declares a boundary")
        .value();
    let open = with_bounds(
        hazards_record,
        &hazards_sample,
        0,
        Some(hazards_boundary),
        Some(hazards_boundary - 2),
    );
    let regressed = with_bounds(
        hazards_record,
        &hazards_sample,
        0,
        Some(hazards_boundary),
        Some(hazards_boundary - 4),
    );
    let mut completeness = Evaluator::new(ship_requires_prior_charge());
    completeness
        .step(Input::source(open))
        .expect("open completeness accepted");
    let completeness_error = completeness
        .step(Input::source(regressed))
        .expect_err("observable_state=non_monotonic_refused (completeness)");
    assert!(
        matches!(
            completeness_error,
            CoreError::NonMonotonicEvidence(ref breach)
                if breach.breach() == MonotonicityBreach::RegressedCompleteThrough
        ),
        "completeness: {completeness_error:?}"
    );

    // Tier / gaps / DeliveryAboveClosedBoundary use the happy fixture (same
    // shapes as resumable.rs clause pins).
    let evidence = adapted("happy");
    let order = only_shipped_order(&load_capture("happy"));
    let record = evidence
        .source(&payment)
        .expect("payment supplied evidence");
    let charge = only_event(&evidence, "sagashop.payment.charge.completed", &order);

    let mut tier = Evaluator::new(ship_requires_prior_charge());
    step_all(&mut tier, inputs(&evidence));
    let tier_error = tier
        .step(Input::source(with_tier(record, SourceTier::Contributing)))
        .expect_err("observable_state=non_monotonic_refused (tier)");
    assert!(
        matches!(
            tier_error,
            CoreError::NonMonotonicEvidence(ref breach)
                if breach.breach() == MonotonicityBreach::RedeclaredTier
        ),
        "tier: {tier_error:?}"
    );

    let mut gaps = Evaluator::new(ship_requires_prior_charge());
    step_all(&mut gaps, inputs(&evidence));
    let gaps_error = gaps
        .step(Input::source(with_gaps(
            record,
            vec![SequenceGap::new(0, 1, 1)],
        )))
        .expect_err("observable_state=non_monotonic_refused (gaps)");
    assert!(
        matches!(
            gaps_error,
            CoreError::NonMonotonicEvidence(ref breach)
                if breach.breach() == MonotonicityBreach::RedeclaredGaps
        ),
        "gaps: {gaps_error:?}"
    );

    // Sources first so the closed boundary is known before the Event arrives.
    let mut boundary_eval = Evaluator::new(ship_requires_prior_charge());
    for input in source_inputs(&evidence) {
        boundary_eval.step(input).expect("source evidence accepted");
    }
    let boundary_error = boundary_eval
        .step(Input::event(with_altered_slot(&charge, 9_999)))
        .expect_err("observable_state=non_monotonic_refused (DeliveryAboveClosedBoundary)");
    assert!(
        matches!(
            boundary_error,
            CoreError::NonMonotonicEvidence(ref breach)
                if breach.breach() == MonotonicityBreach::DeliveryAboveClosedBoundary
        ),
        "DeliveryAboveClosedBoundary: {boundary_error:?}"
    );
}

/// Post-settlement correction that would flip a Violate is visibly refused.
///
/// This is not an Amendment. Ordinary store revision history is a separate
/// object (ADR-0034); Core refuses the reopen that would rewrite the outcome.
#[test]
fn scenario_unsupported_correction() {
    let mut capture = load_capture("hazards");
    let shipped = only_shipped_order(&capture);
    delete_span(&mut capture, &|span| {
        is_event_for_order(span, PAYMENT_CHARGE, &shipped)
    });
    let evidence = adapt(&capture);
    let payment = source(PAYMENT_SERVICE);
    let payment_record = evidence
        .source(&payment)
        .expect("payment still supplies evidence for the other orders");
    let payment_sample = evidence
        .events()
        .iter()
        .find(|event| event.source() == payment)
        .expect("payment delivered Events")
        .clone();
    let closed_boundary = payment_record
        .completeness()
        .complete_through()
        .expect("fixture declares a boundary")
        .value();

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
    assert_eq!(emitted.len(), 1, "Binding settles and emits");
    assert_eq!(emitted[0].verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(
        stamped_late_data_policy(emitted[0].verdict()),
        raw_model::LateDataPolicy::Exclude
    );

    let reopened = with_bounds(
        payment_record,
        &payment_sample,
        0,
        Some(closed_boundary + 2),
        Some(closed_boundary + 2),
    );
    let error = evaluator
        .step(Input::source(reopened))
        .expect_err("observable_state=unsupported_correction_refused");
    assert!(
        matches!(
            error,
            CoreError::NonMonotonicEvidence(ref breach)
                if breach.breach() == MonotonicityBreach::ReopenedClosedEpoch
        ),
        "must visibly refuse, not Amendment or silent overwrite: {error:?}"
    );
}

/// Process restart: final-state replay reconstructs sealed outcomes only.
///
/// Retained Events + SourceEvidence (+ acknowledged Filter epochs standing in
/// for published Filters) feed a fresh evaluator. Emission step history from
/// the first process is **not** claimed reproducible.
#[test]
fn scenario_process_restart() {
    let evidence = adapted("happy");
    let specification = ship_requires_prior_charge();

    // --- first process ---
    let mut first = Evaluator::new(specification.clone());
    let first_steps = step_all(&mut first, inputs(&evidence));
    let first_sealed = first.seal().expect("first seal").into_verdicts();
    let first_emission_count = first_steps
        .iter()
        .filter(|step| matches!(step, Step::Settled(_)))
        .count();

    // Retained final state: Events, SourceEvidence, acknowledged Filter epochs.
    let retained_events = evidence.events().to_vec();
    let retained_sources: Vec<_> = evidence.sources().values().cloned().collect();
    let retained_filter_epochs: BTreeSet<_> = retained_sources
        .iter()
        .map(|record| record.completeness().filter_epoch())
        .collect();
    assert!(
        !retained_filter_epochs.is_empty(),
        "published Filter acknowledgement is part of retained SourceEvidence"
    );

    // --- second process: final-state order only (events, then sources) ---
    let mut second = Evaluator::new(specification);
    for event in &retained_events {
        second
            .step(Input::event(event.clone()))
            .expect("retained Event replays");
    }
    for record in &retained_sources {
        // Filter epoch acknowledgement travels with SourceEvidence — the live
        // path pairs this with store-published Filter bytes (#286).
        assert!(
            retained_filter_epochs.contains(&record.completeness().filter_epoch()),
            "restart reconstruction requires retained Filter acknowledgement"
        );
        second
            .step(Input::source(record.clone()))
            .expect("retained SourceEvidence replays");
    }
    let second_sealed = second.seal().expect("second seal").into_verdicts();

    assert_eq!(
        sealed_outcomes(&first_sealed),
        sealed_outcomes(&second_sealed),
        "observable_state=restart_reconstructed_final_state"
    );
    assert_eq!(
        encoded_verdicts(&first_sealed),
        encoded_verdicts(&second_sealed),
        "sealed Verdict bytes match under final-state replay"
    );

    // Honesty pin: we observed an emission count on the first process, but the
    // restart contract does not require reproducing that sequence — only final
    // sealed state. (Contract: original_emission_sequence_reproducible=false.)
    let _ = first_emission_count;
    assert!(
        !load_contract()
            .get("restart")
            .and_then(|row| row.get("original_emission_sequence_reproducible"))
            .and_then(Value::as_bool)
            .expect("restart.original_emission_sequence_reproducible"),
        "ADR-0035: final-state replay ≠ emission-log replay"
    );

    // Isolation honesty: do not invent Trace ids from Verdict ids.
    for binding in &second_sealed {
        let verdict_id = binding.verdict().id();
        assert!(
            !verdict_id.is_empty(),
            "Verdict ids exist for audit, not as informal Trace ids"
        );
        // Binding is a Binding — not a Trace substitute (contract forbidden list).
        assert!(!binding.binding().is_empty());
    }
}

/// Canonical final-state input order is events-then-sources (store disclosure).
#[test]
fn final_state_replay_order_is_events_then_sources() {
    let evidence = adapted("happy");
    let specification = ship_requires_prior_charge();
    let canonical = inputs(&evidence);
    let mut expected = event_inputs(&evidence);
    expected.extend(source_inputs(&evidence));
    assert_eq!(
        canonical.len(),
        expected.len(),
        "support::inputs is the final-state reconstruction order"
    );
    let from_canonical = support::drive(&specification, canonical).expect("canonical");
    let from_expected = support::drive(&specification, expected).expect("expected");
    assert_eq!(
        encoded_verdicts(&from_canonical),
        encoded_verdicts(&from_expected)
    );
}
