//! Characterisation golden for the five catalogued Pattern families (#275).
//!
//! Issue #275 replaces five per-family copies of Verdict construction,
//! settlement and Assurance Envelope assembly with one shared implementation.
//! This file pins what the Verification Core does **today**, family by family,
//! so that refactor can be shown to be behaviour-preserving. It asserts nothing
//! about whether today's behaviour is right; it asserts that it does not move.
//! A row that looks wrong is pinned as it is and raised on #275, never fixed
//! here.
//!
//! # What a row pins
//!
//! Every row drives one Evaluator through its public API only — `step`,
//! `resume`, `seal`, `with_liveness_freshness`, `with_raw_event_retention`, and
//! the batch `sentinel_core::evaluate` for the one refusal only it reaches. No
//! row calls a Verdict builder directly, so the golden survives those builders
//! becoming crate-private. For each `step` or `resume` the row records:
//!
//! - the `Step` variant, so an emission that moves or vanishes mid-stream is a
//!   diff (this is what pins the emission key and "at most two emissions");
//! - the `{:?}` of the `FactRequest` inside `Step::NeedsFacts`, because query
//!   construction reads the declared cold guards through the family dispatch;
//! - for every emitted Binding: the Binding key, the readable labels, every
//!   uncertainty sentence, the `{:?}` of its `Explanation` and
//!   `BindingFactCost`, and the `{:?}` of `Verdict::encode_to_vec()` decoded
//!   back into the generated wire message, so every field the bytes carry is
//!   pinned by name. A Verdict whose encoding repeats one the same row already
//!   rendered is written as a back-reference to it instead.
//!
//! Then the same for `seal()`, plus the sealed cost and closed guards. A
//! refusal is pinned as the `{:?}` of the `CoreError`.
//!
//! Most rows seal one Binding, [`BINDING`]. One row per family seals two
//! Bindings settling to different arms, delivered out of their sorted order,
//! and delivers a third Binding that raises no obligation and must produce no
//! Verdict at all.
//!
//! # Labels are asserted in code, independently of the bytes
//!
//! ADR-0031 clause 4: every row names the arm it claims to reach, and the test
//! checks the sealed Verdict's outcome, reason, authority, cold path,
//! activation-barrier status and settlement-basis kind against that claim. A
//! row named "Violate" fails if it is in fact an Inconclusive, and that failure
//! does not depend on the rendered block. Every row's claim failures are
//! collected and reported together with every row whose block moved.
//!
//! # Regenerating
//!
//! `SENTINEL_UPDATE_GOLDEN=1 cargo test -p sentinel-core --test family_verdict_golden`
//! rewrites `core/tests/golden/family_verdicts.txt`. Every #275 refactor commit
//! must leave that file byte-identical; regenerating it there defeats the net.
//! The flag is off when unset, empty, `0` or `false`; the test refuses to
//! regenerate when `CI` is set, or over any row that misses its claim. The name
//! is deliberately not compile's `UPDATE_GOLDEN`, so a workspace-wide run that
//! refreshes the catalogue snapshot cannot silently rewrite this net too.
//!
//! # Not applicable, stated rather than skipped
//!
//! - **CAT-5 has no `Violate` arm.** No CAT-5 row has a Violate claim, and the
//!   test asserts that no CAT-5 emission or sealed Verdict is a `Violate`.
//! - **CAT-5 forms no ordering judgement**, so it has no concurrent-pair row:
//!   its count is the cold path's, and `AggregateAssessment::concurrent_pairs`
//!   is always empty.
//! - **CAT-5's contributing-tier row** is not a contributing-tier *falsifier*:
//!   a CAT-5 finding is a cold answer, never a Slice Event, so there is no
//!   falsifying Event whose tier could be contributing. The row it has instead
//!   puts the trigger on a contributing-tier source and answers `Holds`.
//! - **Watermark-anchored Violate (ADR-0033)** exists only for CAT-1 and CAT-3;
//!   CAT-2 and CAT-4 convict on a present falsifier, never on silence.
//! - **Cold guards.** Only CAT-1 (`PrecedenceSpecification::with_cold_guards`)
//!   and CAT-5 (`AggregateSpecification::with_cold_guard`) expose a guard
//!   builder; CAT-2, CAT-3 and CAT-4 carry a `cold_guards` field with none. So
//!   the guard-settled and terminal-refusal rows are CAT-1 and CAT-5 only.
//! - **Retirement (O-03)** is admissible only where a retirement policy is
//!   declared: catalogued CAT-3 declares `Deadline`, and CAT-1 is given one by
//!   hand exactly as `support::ship_requires_prior_charge_retiring` does.
//!   CAT-2, CAT-4 and CAT-5 declare `NotRequired`.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use prost::Message as _;
use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance, ProducerContract,
    SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::absence::AbsenceSpecification;
use sentinel_core::aggregate::AggregateSpecification;
use sentinel_core::evaluator::{Evaluator, Input, Step};
use sentinel_core::existence::ExistenceSpecification;
use sentinel_core::facts::{
    ColdGuardId, ColdWitness, FactRequest, FactResponse, GuardConclusion, RefusalReason,
};
use sentinel_core::slice::{Binding, RetirementCause, RetirementSignal, ScopeHorizon};
use sentinel_core::specification::{PrecedenceSpecification, RetirementPolicy};
use sentinel_core::taint::TaintSpecification;
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::{
    ActivationBarrier, ActivationBarrierStatus, ActivationWatermark, AttributeKey, AttributeValue,
    ClockDomainId, CompletenessFreshness, DataClass, Event, EventId, EventKind, FilterEpoch,
    InconclusiveReason, Integrity, MonitoredApplicationId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, RawEventRetentionDays, SchemaVersion, Sensitivity,
    SentinelClockId, SequenceEpoch, SequenceGap, SequenceNumber, SettlementBasisKind,
    SourceActivity, SourceAssurance, SourceCapabilities, SourceCapability,
    SourceCompletenessSignal, SourceEvidence, SourceId, SourceIdentity, SourceLivenessFreshness,
    SourceTier, TierSettlement, Verdict, VerdictAuthority, VerdictError, VerdictOutcome,
};

/// The environment variable that regenerates the golden file.
const UPDATE_GOLDEN: &str = "SENTINEL_UPDATE_GOLDEN";

/// The Binding attribute every family's fixture binds on.
const BINDING_ATTRIBUTE: &str = "example.request.id";
/// The one Binding value every fixture's obligation is raised for.
const BINDING: &str = "req-1";
/// The producer schema every fixture Event is written against.
const SCHEMA: &str = "example-contract-v1";
/// Where every completeness signal is observed on the Sentinel clock.
const SIGNAL_OBSERVED_AT: i128 = 40;
/// A Scope horizon above every fixture Event's observation time.
const HORIZON: i128 = 10_000;
/// The Filter epoch every fixture source acknowledges.
const ACKNOWLEDGED_FILTER_EPOCH: u64 = 1;
/// A kind no family's selectors match, carrying no Binding attribute.
const FILLER_KIND: &str = "example.filler.recorded";
/// The one cold-path guard the guarded CAT-1 rows declare.
const GUARD: &str = "example.approval.reachability";

// ---------------------------------------------------------------------------
// The golden itself
// ---------------------------------------------------------------------------

/// Every family's rows match the characterisation golden, row by row.
///
/// Rows are compared by name, so a failure lists exactly the rows whose
/// recorded behaviour moved rather than one undifferentiated text diff.
#[test]
fn every_family_matches_its_characterisation_golden() {
    let mut rows: Vec<(String, String)> = Vec::new();
    // Claim failures are collected rather than raised, so one mutation that
    // moves many rows' arms names every one of them.
    let mut moved = Vec::new();
    for (name, block, failures) in scenarios().into_iter().map(run) {
        moved.extend(failures);
        rows.push((name, block));
    }
    let mut names = BTreeSet::new();
    for (name, _) in &rows {
        assert!(names.insert(name.clone()), "row {name} is declared twice");
    }
    let actual = render_file(&rows);

    let path = golden_path();
    if flag_set(UPDATE_GOLDEN) {
        assert!(
            !flag_set("CI"),
            "{UPDATE_GOLDEN} is set under CI; the golden is regenerated only by hand"
        );
        assert!(
            moved.is_empty(),
            "refusing to regenerate over rows that miss their claim:\n{}",
            moved.join("\n")
        );
        std::fs::write(&path, &actual)
            .unwrap_or_else(|error| panic!("failed to write golden {}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("golden {} is readable: {error}", path.display()));
    let expected_rows = parse_file(&expected);

    for (name, block) in &rows {
        match expected_rows.get(name) {
            Some(pinned) if pinned == block => {}
            Some(pinned) => moved.push(format!(
                "{name} moved\n--- pinned ---\n{pinned}--- actual ---\n{block}"
            )),
            None => moved.push(format!("{name} is not in the golden")),
        }
    }
    for name in expected_rows.keys() {
        if !names.contains(name) {
            moved.push(format!("{name} is pinned but no longer produced"));
        }
    }
    assert!(
        moved.is_empty() && actual == expected,
        "{} row(s) no longer match {}; rerun with {UPDATE_GOLDEN}=1 only when the change is \
         intended:\n{}",
        moved.len(),
        path.display(),
        moved.join("\n")
    );
}

/// Whether an environment flag is on: set, non-empty, and neither `0` nor
/// `false`, so `SENTINEL_UPDATE_GOLDEN=0` does not rewrite the golden.
fn flag_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("family_verdicts.txt")
}

fn render_file(rows: &[(String, String)]) -> String {
    let mut out = String::from(
        "# Characterisation golden for the five Pattern families (#275).\n\
         # Generated by core/tests/family_verdict_golden.rs; regenerate only with\n\
         # SENTINEL_UPDATE_GOLDEN=1, and never inside a behaviour-preserving refactor.\n",
    );
    for (name, block) in rows {
        out.push_str("\n=== ");
        out.push_str(name);
        out.push('\n');
        out.push_str(block);
    }
    out
}

fn parse_file(text: &str) -> BTreeMap<String, String> {
    let mut rows = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("=== ") {
            if let Some((done, block)) = current.take() {
                rows.insert(done, block);
            }
            current = Some((name.to_owned(), String::new()));
        } else if let Some((_, block)) = current.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    if let Some((done, block)) = current {
        rows.insert(done, block);
    }
    // The renderer separates rows with one blank line, which parsing attaches
    // to the preceding block; strip it so a block compares equal to its render.
    rows.into_iter()
        .map(|(name, block)| {
            let trimmed = block.strip_suffix("\n\n").map_or(block.clone(), |kept| {
                let mut kept = kept.to_owned();
                kept.push('\n');
                kept
            });
            (name, trimmed)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rows, claims and the driver
// ---------------------------------------------------------------------------

/// A named further property of a sealed Verdict.
type Property = (&'static str, fn(&BindingVerdict) -> bool);

/// One characterisation row.
struct Scenario {
    name: String,
    claim: Claim,
    drive: Drive,
    /// A further property of the sealed Verdict the row's name promises.
    also: Option<Property>,
}

/// How a row drives the Core.
enum Drive {
    /// The resumable API: each action in order, then `seal()`.
    Stepped {
        evaluator: Box<Evaluator>,
        actions: Vec<Action>,
    },
    /// The batch path, `sentinel_core::evaluate`.
    Batch {
        evaluator: Box<Evaluator>,
        events: Vec<Event>,
        sources: BTreeMap<SourceIdentity, SourceEvidence>,
    },
}

/// One thing a driver does to an Evaluator.
enum Action {
    /// `step` with one input, under a readable label.
    Feed(String, Input),
    /// `resume(Vec::new())`: "what do you need?"
    Ask,
    /// `resume` answering every query of the last `NeedsFacts` request.
    Answer {
        conclusion: GuardConclusion,
        /// How far above each query's floor the answer claims to have read.
        lift: u64,
    },
    /// `resume` answering each query of the last `NeedsFacts` request with
    /// the conclusion named for its Binding; an unnamed Binding is left
    /// unanswered.
    AnswerEach {
        conclusions: Vec<(&'static str, GuardConclusion)>,
        lift: u64,
    },
}

/// The arm a row claims to reach, checked in code beside the hex.
enum Claim {
    /// One Binding, [`BINDING`], sealed with these labels.
    Sealed(Labels),
    /// Exactly these Bindings, sealed in this order with these labels. A
    /// Binding the row delivers but does not list must never be emitted.
    Bindings(Vec<(&'static str, Labels)>),
    /// The drive ends in a refusal of this `CoreError` variant.
    Refused(&'static str, fn(&CoreError) -> bool),
}

/// The readable labels of one Verdict.
#[derive(Debug, PartialEq)]
struct Labels {
    outcome: VerdictOutcome,
    reason: Option<InconclusiveReason>,
    authority: VerdictAuthority,
    cold_path: TierSettlement,
    barrier: ActivationBarrierStatus,
    basis: Option<SettlementBasisKind>,
}

impl Labels {
    fn of(verdict: &Verdict) -> Self {
        Self {
            outcome: verdict.outcome(),
            reason: verdict.inconclusive_reason(),
            authority: verdict.authority(),
            cold_path: verdict.cold_path(),
            barrier: verdict.activation_barrier(),
            basis: verdict.settlement_basis(),
        }
    }

    fn render(&self) -> String {
        format!(
            "outcome={:?} reason={:?} authority={:?} cold_path={:?} barrier={:?} basis={:?}",
            self.outcome, self.reason, self.authority, self.cold_path, self.barrier, self.basis
        )
    }
}

/// An authoritative Satisfy settled on complete evidence.
fn satisfy(cold_path: TierSettlement, barrier: ActivationBarrierStatus) -> Labels {
    Labels {
        outcome: VerdictOutcome::Satisfy,
        reason: None,
        authority: VerdictAuthority::Authoritative,
        cold_path,
        barrier,
        basis: Some(SettlementBasisKind::CompleteEvidence),
    }
}

/// A Satisfy that a pending barrier keeps Provisional.
fn provisional_satisfy(cold_path: TierSettlement) -> Labels {
    Labels {
        outcome: VerdictOutcome::Satisfy,
        reason: None,
        authority: VerdictAuthority::Provisional,
        cold_path,
        barrier: ActivationBarrierStatus::Pending,
        basis: None,
    }
}

/// An authoritative Violate on the named basis.
fn violate(basis: SettlementBasisKind) -> Labels {
    Labels {
        outcome: VerdictOutcome::Violate,
        reason: None,
        authority: VerdictAuthority::Authoritative,
        cold_path: TierSettlement::NotRequired,
        barrier: ActivationBarrierStatus::NotDeclared,
        basis: Some(basis),
    }
}

/// An Inconclusive with no settlement basis.
fn inconclusive(
    reason: InconclusiveReason,
    authority: VerdictAuthority,
    cold_path: TierSettlement,
    barrier: ActivationBarrierStatus,
) -> Labels {
    Labels {
        outcome: VerdictOutcome::Inconclusive,
        reason: Some(reason),
        authority,
        cold_path,
        barrier,
        basis: None,
    }
}

/// What one row produced: its rendered block and every Verdict it saw.
struct Outcome {
    block: String,
    seen: Vec<BindingVerdict>,
    sealed: Result<Vec<BindingVerdict>, CoreError>,
}

/// Runs one row, checks its claim, and returns its name, rendered block and
/// every way it missed its claim.
fn run(scenario: Scenario) -> (String, String, Vec<String>) {
    let Scenario {
        name,
        claim,
        drive,
        also,
    } = scenario;
    let outcome = match drive {
        Drive::Stepped { evaluator, actions } => drive_stepped(*evaluator, actions),
        Drive::Batch {
            evaluator,
            events,
            sources,
        } => drive_batch(*evaluator, &events, &sources),
    };

    let mut failures = Vec::new();
    if name.starts_with("cat5/") {
        for verdict in outcome.seen.iter().chain(outcome.sealed.iter().flatten()) {
            if verdict.verdict().outcome() == VerdictOutcome::Violate {
                failures.push(format!(
                    "{name}: CAT-5 has no Violate arm, yet one was emitted"
                ));
            }
        }
    }

    let (header, claimed) = match claim {
        Claim::Sealed(labels) => (
            format!("claim sealed {}\n", labels.render()),
            Ok(vec![(BINDING, labels)]),
        ),
        Claim::Bindings(bindings) => (
            bindings
                .iter()
                .fold(String::new(), |mut header, (binding, labels)| {
                    line(
                        &mut header,
                        format_args!("claim sealed {binding} {}", labels.render()),
                    );
                    header
                }),
            Ok(bindings),
        ),
        Claim::Refused(variant, matches) => (
            format!("claim refused {variant}\n"),
            Err((variant, matches)),
        ),
    };
    match (claimed, &outcome.sealed) {
        (Ok(expected), Ok(verdicts)) => {
            let sealed: Vec<&str> = verdicts.iter().map(BindingVerdict::binding).collect();
            let wanted: Vec<&str> = expected.iter().map(|(binding, _)| *binding).collect();
            if sealed != wanted {
                failures.push(format!(
                    "{name}: sealed Bindings {sealed:?}, claimed {wanted:?}"
                ));
            }
            for verdict in &outcome.seen {
                if !wanted.contains(&verdict.binding()) {
                    failures.push(format!(
                        "{name}: emitted a Verdict for unclaimed Binding {}",
                        verdict.binding()
                    ));
                }
            }
            for (verdict, (binding, labels)) in verdicts.iter().zip(&expected) {
                let actual = Labels::of(verdict.verdict());
                if &actual != labels {
                    failures.push(format!(
                        "{name}: Binding {binding} sealed {}, claimed {}",
                        actual.render(),
                        labels.render()
                    ));
                }
                if let Some((property, holds)) = also {
                    if !holds(verdict) {
                        failures.push(format!("{name}: Binding {binding}: expected {property}"));
                    }
                }
            }
        }
        (Err((variant, matches)), Err(error)) => {
            if !matches(error) {
                failures.push(format!("{name}: expected {variant}, got {error:?}"));
            }
        }
        (Ok(_), Err(error)) => failures.push(format!("{name}: claimed a Verdict, got {error:?}")),
        (Err((variant, _)), Ok(verdicts)) => {
            failures.push(format!("{name}: claimed {variant}, got {verdicts:?}"));
        }
    }
    (name, header + &outcome.block, failures)
}

fn drive_stepped(mut evaluator: Evaluator, actions: Vec<Action>) -> Outcome {
    let mut out = String::new();
    let block = &mut out;
    let mut seen = Vec::new();
    let mut prior = Vec::new();
    let mut last_request: Option<FactRequest> = None;
    for action in actions {
        let (label, step) = match action {
            Action::Feed(label, input) => (label, evaluator.step(input)),
            Action::Ask => ("ask".to_owned(), evaluator.resume(Vec::new())),
            Action::Answer { conclusion, lift } => {
                let label = format!("answer {conclusion:?} lift={lift}");
                let Some(request) = &last_request else {
                    line(block, format_args!("{label} -> no request outstanding"));
                    continue;
                };
                let responses = answers(request, lift, |_| Some(conclusion.clone()));
                (label, evaluator.resume(responses))
            }
            Action::AnswerEach { conclusions, lift } => {
                let label = format!("answer each {conclusions:?} lift={lift}");
                let Some(request) = &last_request else {
                    line(block, format_args!("{label} -> no request outstanding"));
                    continue;
                };
                let responses = answers(request, lift, |binding| {
                    conclusions
                        .iter()
                        .find(|(named, _)| *named == binding)
                        .map(|(_, conclusion)| conclusion.clone())
                });
                (label, evaluator.resume(responses))
            }
        };
        match step {
            Ok(Step::Pending) => line(block, format_args!("{label} -> Pending")),
            Ok(Step::NeedsFacts(request)) => {
                line(block, format_args!("{label} -> NeedsFacts {request:?}"));
                last_request = Some(request);
            }
            Ok(Step::Settled(verdicts)) => {
                line(block, format_args!("{label} -> Settled"));
                render_verdicts(block, &verdicts, &mut prior, &label);
                seen.extend(verdicts);
            }
            Ok(other) => line(block, format_args!("{label} -> {other:?}")),
            Err(error) => line(block, format_args!("{label} -> error {error:?}")),
        }
    }
    let sealed = match evaluator.seal() {
        Ok(sealed) => {
            block.push_str("seal -> ok\n");
            render_verdicts(block, sealed.verdicts(), &mut prior, "seal");
            line(block, format_args!("  sealed cost {:?}", sealed.cost()));
            line(
                block,
                format_args!("  sealed closed_guards {:?}", sealed.closed_guards()),
            );
            Ok(sealed.into_verdicts())
        }
        Err(error) => {
            line(block, format_args!("seal -> error {error:?}"));
            Err(error)
        }
    };
    Outcome {
        block: out,
        seen,
        sealed,
    }
}

/// Answers each query of `request` that `conclude` names a conclusion for,
/// claiming to have read `lift` sequences above the query's floor.
fn answers(
    request: &FactRequest,
    lift: u64,
    conclude: impl Fn(&str) -> Option<GuardConclusion>,
) -> Vec<FactResponse> {
    request
        .queries()
        .iter()
        .filter_map(|query| {
            let conclusion = conclude(query.binding().as_str())?;
            let settled_at = query
                .watermark_floor()
                .as_map()
                .iter()
                .map(|(source, (epoch, through))| {
                    (
                        source.clone(),
                        (*epoch, SequenceNumber::new(through.value() + lift)),
                    )
                })
                .collect();
            Some(FactResponse::new(query.clone(), conclusion, settled_at))
        })
        .collect()
}

fn drive_batch(
    evaluator: Evaluator,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Outcome {
    let mut out = String::new();
    let block = &mut out;
    let sealed = sentinel_core::evaluate(evaluator, events, sources);
    match &sealed {
        Ok(verdicts) => {
            block.push_str("batch -> ok\n");
            render_verdicts(block, verdicts, &mut Vec::new(), "batch");
        }
        Err(error) => line(block, format_args!("batch -> error {error:?}")),
    }
    Outcome {
        block: out,
        seen: Vec::new(),
        sealed,
    }
}

/// Appends one formatted line to a row's block.
fn line(block: &mut String, text: std::fmt::Arguments<'_>) {
    block
        .write_fmt(text)
        .expect("writing to a String cannot fail");
    block.push('\n');
}

/// Renders one emission's Verdicts. A Verdict whose encoding matches one an
/// earlier emission of the same row already rendered is written as a
/// back-reference to it, so the repeat stays pinned byte for byte.
fn render_verdicts(
    block: &mut String,
    verdicts: &[BindingVerdict],
    prior: &mut Vec<(Vec<u8>, String)>,
    at: &str,
) {
    for verdict in verdicts {
        line(block, format_args!("  binding {:?}", verdict.key()));
        line(
            block,
            format_args!("    labels {}", Labels::of(verdict.verdict()).render()),
        );
        for entry in verdict.verdict().uncertainty() {
            line(
                block,
                format_args!("    uncertainty {:?}: {}", entry.reason, entry.detail),
            );
        }
        line(
            block,
            format_args!("    explanation {:?}", verdict.explanation()),
        );
        line(block, format_args!("    cost {:?}", verdict.cost()));
        let encoded = verdict.verdict().encode_to_vec();
        let here = format!("{at} binding {}", verdict.binding());
        if let Some((_, first)) = prior.iter().find(|(bytes, _)| *bytes == encoded) {
            line(block, format_args!("    encoded = as at {first}"));
            continue;
        }
        // The wire form decoded back into its generated message: every field
        // the bytes carry, named, so an envelope that moves shows which field
        // moved. One line per Verdict keeps the file small; `git diff
        // --word-diff` reads it field by field.
        let decoded = raw_model::Verdict::decode(encoded.as_slice())
            .expect("an encoded Verdict decodes into its generated message");
        line(block, format_args!("    encoded {decoded:?}"));
        prior.push((encoded, here));
    }
}

// ---------------------------------------------------------------------------
// Family-agnostic fixture vocabulary
// ---------------------------------------------------------------------------

/// One catalogued Specification, whichever family it belongs to.
///
/// A test-local dispatch so one row builder can serve every family. It reaches
/// the Core only through the five public `Evaluator` constructors.
#[derive(Clone)]
enum Spec {
    Precedence(PrecedenceSpecification),
    Absence(AbsenceSpecification),
    Existence(ExistenceSpecification),
    Taint(TaintSpecification),
    Aggregate(AggregateSpecification),
}

impl Spec {
    fn evaluator(self) -> Evaluator {
        match self {
            Self::Precedence(specification) => Evaluator::new(specification),
            Self::Absence(specification) => Evaluator::absence(specification),
            Self::Existence(specification) => Evaluator::existence(specification),
            Self::Taint(specification) => Evaluator::taint(specification),
            Self::Aggregate(specification) => Evaluator::aggregate(specification),
        }
    }

    fn with_barrier(self, barrier: ActivationBarrier) -> Self {
        match self {
            Self::Precedence(specification) => {
                Self::Precedence(specification.with_activation_barrier(barrier))
            }
            Self::Absence(specification) => {
                Self::Absence(specification.with_activation_barrier(barrier))
            }
            Self::Existence(specification) => {
                Self::Existence(specification.with_activation_barrier(barrier))
            }
            Self::Taint(specification) => {
                Self::Taint(specification.with_activation_barrier(barrier))
            }
            Self::Aggregate(specification) => {
                Self::Aggregate(specification.with_activation_barrier(barrier))
            }
        }
    }

    /// The same Specification with its required-source set emptied.
    ///
    /// Every family's `new` is infallible over an empty set, so the Evaluator
    /// has to refuse it by name.
    fn without_required_sources(&self) -> Self {
        match self {
            Self::Precedence(s) => Self::Precedence(PrecedenceSpecification::new(
                s.identity().clone(),
                s.binding_attribute().clone(),
                s.trigger().clone(),
                s.requirement().clone(),
                BTreeSet::new(),
                s.schema_version().clone(),
            )),
            Self::Absence(s) => Self::Absence(AbsenceSpecification::new(
                s.identity().clone(),
                s.binding_attribute().clone(),
                s.after().clone(),
                s.until().clone(),
                s.veto().clone(),
                BTreeSet::new(),
                s.schema_version().clone(),
            )),
            Self::Existence(s) => Self::Existence(ExistenceSpecification::new(
                s.identity().clone(),
                s.binding_attribute().clone(),
                s.open().clone(),
                s.required().clone(),
                s.close().clone(),
                BTreeSet::new(),
                s.schema_version().clone(),
            )),
            Self::Taint(s) => Self::Taint(TaintSpecification::new(
                s.identity().clone(),
                s.binding_attribute().clone(),
                s.action().clone(),
                s.untrusted_content().clone(),
                BTreeSet::new(),
                s.schema_version().clone(),
            )),
            Self::Aggregate(s) => Self::Aggregate(AggregateSpecification::new(
                s.identity().clone(),
                s.binding_attribute().clone(),
                s.trigger().clone(),
                s.member().clone(),
                s.lookback_sequences(),
                s.threshold(),
                BTreeSet::new(),
                s.schema_version().clone(),
            )),
        }
    }
}

/// One family's fixture world: its Specification and the Events of each arm.
struct World {
    /// The row-name prefix, `cat1` through `cat5`.
    prefix: &'static str,
    spec: Spec,
    /// Each required source's deployed tier, as its `SourceEvidence` declares.
    tiers: Vec<(&'static str, SourceTier)>,
    /// Events that settle `Satisfy` once every source declares.
    satisfy: Vec<Event>,
    /// The source the closed-gate row withholds its declaration for.
    withheld: &'static str,
    /// A required source that stays silent, and the Events left without it.
    silent: (&'static str, Vec<Event>),
    /// The source whose evidence the gap, contradiction and edge rows disturb.
    noisy: &'static str,
    /// Whether the driver answers the family's cold-path count (CAT-5).
    answers: bool,
}

impl World {
    fn row(&self, arm: &str) -> String {
        format!("{}/{arm}", self.prefix)
    }

    fn tier_of(&self, app: &str) -> SourceTier {
        self.tiers
            .iter()
            .find(|(declared, _)| *declared == app)
            .map_or_else(|| panic!("{app} is a required source"), |(_, tier)| *tier)
    }

    /// The cold-path tail every gate-open drive of this family needs.
    fn tail(&self) -> Vec<Action> {
        if self.answers {
            vec![
                Action::Ask,
                Action::Answer {
                    conclusion: GuardConclusion::Holds,
                    lift: 0,
                },
            ]
        } else {
            Vec::new()
        }
    }

    /// The cold path's settlement for a gate-open, answered Satisfy.
    fn settled_cold_path(&self) -> TierSettlement {
        if self.answers {
            TierSettlement::Settled
        } else {
            TierSettlement::NotRequired
        }
    }

    /// The barrier every required source's satisfy-time evidence clears at `target`.
    fn barrier(&self, target: u64) -> ActivationBarrier {
        let watermarks = self
            .tiers
            .iter()
            .map(|(app, _)| {
                (
                    source(app),
                    ActivationWatermark::new(
                        SequenceEpoch::new(1),
                        SequenceNumber::new(highest(&self.satisfy, app)),
                        observation(SIGNAL_OBSERVED_AT),
                    ),
                )
            })
            .collect();
        ActivationBarrier::new(FilterEpoch::new(target), watermarks)
            .expect("every fixture names at least one required source")
    }

    /// The `SourceEvidence` every source that delivered one of `events` declares.
    fn declarations(&self, events: &[Event]) -> Vec<SourceEvidence> {
        self.tiers
            .iter()
            .filter(|(app, _)| highest(events, app) > 0)
            .map(|(app, tier)| {
                let through = highest(events, app);
                declared(app, *tier, through, through, Vec::new())
            })
            .collect()
    }
}

/// The highest delivery sequence one source holds among `events`.
fn highest(events: &[Event], app: &str) -> u64 {
    events
        .iter()
        .filter(|event| event.source() == source(app))
        .map(|event| event.delivery().sequence().value())
        .max()
        .unwrap_or(0)
}

/// Feeds every Event, then every declaration — the order the evidence
/// monotonicity contract is written for.
fn feed(events: &[Event], declarations: &[SourceEvidence]) -> Vec<Action> {
    events
        .iter()
        .map(|event| Action::Feed(format!("event {}", event.id()), Input::event(event.clone())))
        .chain(declarations.iter().map(|evidence| {
            Action::Feed(
                format!(
                    "source {}",
                    evidence.source().monitored_application().as_str()
                ),
                Input::source(evidence.clone()),
            )
        }))
        .collect()
}

fn retire() -> Action {
    Action::Feed(
        format!("retire {BINDING} horizon={HORIZON} cause=Deadline"),
        Input::retire(RetirementSignal::new(
            Binding::new(BINDING),
            ScopeHorizon::new(observation(HORIZON)),
            RetirementCause::Deadline,
        )),
    )
}

fn stepped(name: String, claim: Claim, evaluator: Evaluator, actions: Vec<Action>) -> Scenario {
    Scenario {
        name,
        claim,
        drive: Drive::Stepped {
            evaluator: Box::new(evaluator),
            actions,
        },
        also: None,
    }
}

fn source(app: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(app).expect("constant application"),
        SourceId::new("probe").expect("constant source"),
    )
}

fn capable() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

fn occurrence(nanos: i128) -> OccurrenceTime {
    OccurrenceTime::new(
        ClockDomainId::new("example-producer").expect("constant clock domain"),
        nanos,
        0,
    )
}

fn observation(nanos: i128) -> ObservationTime {
    ObservationTime::new(
        SentinelClockId::new("sentinel").expect("constant Sentinel clock"),
        nanos,
    )
}

/// One fixture Event. `attributes` are string-valued; `predecessors` are ids.
fn event(
    id: &str,
    app: &str,
    sequence: u64,
    kind: &str,
    attributes: &[(&str, &str)],
    predecessors: &[&str],
) -> Event {
    event_at(
        id,
        app,
        sequence,
        kind,
        attributes,
        predecessors,
        observation(i128::from(sequence)),
    )
}

/// One fixture Event observed at `observed` rather than at its sequence.
fn event_at(
    id: &str,
    app: &str,
    sequence: u64,
    kind: &str,
    attributes: &[(&str, &str)],
    predecessors: &[&str],
    observed: ObservationTime,
) -> Event {
    let producer = ProducerEvent::new(
        EventId::new(id).expect("constant id"),
        SequenceEpoch::new(1),
        SequenceNumber::new(sequence),
        SchemaVersion::new(SCHEMA).expect("constant schema"),
        FilterEpoch::new(ACKNOWLEDGED_FILTER_EPOCH),
        EventKind::new(kind).expect("constant kind"),
        occurrence(i128::from(sequence)),
        attributes
            .iter()
            .map(|(key, value)| {
                (
                    AttributeKey::new(*key).expect("constant attribute"),
                    AttributeValue::String((*value).to_owned()),
                )
            })
            .collect(),
        capable(),
        Sensitivity::Internal,
        predecessors
            .iter()
            .map(|id| EventId::new(*id).expect("constant predecessor"))
            .collect(),
    )
    .expect("a well-formed producer Event");
    let assurance = SourceAssurance::new(
        source(app),
        SourceTier::Anchor,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("well-formed source assurance");
    Event::observe(producer, assurance, observed, DataClass::Tenant)
}

/// One fixture Event bound to [`BINDING`].
fn bound(
    id: &str,
    app: &str,
    sequence: u64,
    kind: &str,
    attributes: &[(&str, &str)],
    predecessors: &[&str],
) -> Event {
    bound_to(BINDING, id, app, sequence, kind, attributes, predecessors)
}

/// One fixture Event bound to `binding`.
fn bound_to(
    binding: &str,
    id: &str,
    app: &str,
    sequence: u64,
    kind: &str,
    attributes: &[(&str, &str)],
    predecessors: &[&str],
) -> Event {
    let mut all = vec![(BINDING_ATTRIBUTE, binding)];
    all.extend_from_slice(attributes);
    event(id, app, sequence, kind, &all, predecessors)
}

/// A Slice member of [`BINDING`] observed above the retirement horizon.
fn above_horizon(
    id: &str,
    app: &str,
    sequence: u64,
    kind: &str,
    attributes: &[(&str, &str)],
) -> Event {
    let mut all = vec![(BINDING_ATTRIBUTE, BINDING)];
    all.extend_from_slice(attributes);
    event_at(id, app, sequence, kind, &all, &[], observation(HORIZON + 1))
}

/// A Slice member of [`BINDING`] observed on a second Sentinel clock, which
/// the retirement horizon cannot order.
fn on_another_clock(
    id: &str,
    app: &str,
    sequence: u64,
    kind: &str,
    attributes: &[(&str, &str)],
) -> Event {
    let mut all = vec![(BINDING_ATTRIBUTE, BINDING)];
    all.extend_from_slice(attributes);
    event_at(
        id,
        app,
        sequence,
        kind,
        &all,
        &[],
        ObservationTime::new(
            SentinelClockId::new("sentinel-replica").expect("constant Sentinel clock"),
            i128::from(sequence),
        ),
    )
}

/// An Event no selector matches and no Slice admits: evidence bookkeeping only.
fn filler(id: &str, app: &str, sequence: u64, predecessors: &[&str]) -> Event {
    event(id, app, sequence, FILLER_KIND, &[], predecessors)
}

/// One source's declaration: tier, bounds and gaps, observed at a fixed point.
fn declared(
    app: &str,
    tier: SourceTier,
    last_emitted: u64,
    complete_through: u64,
    gaps: Vec<SequenceGap>,
) -> SourceEvidence {
    signalled(
        app,
        tier,
        (last_emitted, complete_through),
        gaps,
        SourceActivity::Active {
            since: occurrence(0),
        },
        SIGNAL_OBSERVED_AT,
    )
}

/// One source's declaration with its activity and observation point given.
fn signalled(
    app: &str,
    tier: SourceTier,
    (last_emitted, complete_through): (u64, u64),
    gaps: Vec<SequenceGap>,
    activity: SourceActivity,
    observed_at: i128,
) -> SourceEvidence {
    let producer = ProducerCompletenessSignal::new(
        SequenceEpoch::new(1),
        Some(SequenceNumber::new(last_emitted)),
        Some(SequenceNumber::new(complete_through)),
        occurrence(0),
        FilterEpoch::new(ACKNOWLEDGED_FILTER_EPOCH),
        activity,
    )
    .expect("valid producer completeness signal");
    let assurance = SourceAssurance::new(
        source(app),
        tier,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("valid source assurance");
    SourceEvidence::new(
        source(app),
        tier,
        capable(),
        SourceCompletenessSignal::observe(producer, assurance, observation(observed_at)),
        gaps,
    )
}

/// Liveness floors every fixture signal (observed at 40) meets, or misses.
fn liveness(active_floor: i128) -> SourceLivenessFreshness {
    SourceLivenessFreshness::try_new(
        CompletenessFreshness::observed_not_before(observation(active_floor)),
        CompletenessFreshness::observed_not_before(observation(0)),
    )
    .expect("same Sentinel clock, idle floor no stricter than active")
}

fn selector(kind: &str, attributes: &[(&str, &str)]) -> ParameterValue {
    ParameterValue::Selector(SelectorValue::new(
        EventKind::new(kind).expect("constant kind"),
        attributes
            .iter()
            .map(|(key, value)| {
                (
                    AttributeKey::new(*key).expect("constant attribute"),
                    AttributeValue::String((*value).to_owned()),
                )
            })
            .collect(),
    ))
}

/// Instantiates one catalogued Pattern over string-keyed arguments and roles.
fn catalogued(
    pattern: PatternId,
    specification_id: &str,
    mut arguments: BTreeMap<String, ParameterValue>,
    roles: &[(&str, &str, SourceTier)],
) -> PatternInstance {
    arguments.insert(
        "binding_attribute".to_owned(),
        ParameterValue::AttributeKey(AttributeKey::new(BINDING_ATTRIBUTE).expect("constant")),
    );
    let evidence = roles
        .iter()
        .map(|(role, app, tier)| {
            (
                (*role).to_owned(),
                EvidenceBinding::new(source(app), *tier, capable()),
            )
        })
        .collect();
    instantiate(
        pattern,
        FilterIdentity {
            specification_id: specification_id.to_owned(),
            specification_version: "v1".to_owned(),
        },
        ProducerContract::new(SchemaVersion::new(SCHEMA).expect("constant schema")),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds")
}

// ---------------------------------------------------------------------------
// The rows every family has
// ---------------------------------------------------------------------------

fn scenarios() -> Vec<Scenario> {
    let mut rows = Vec::new();
    for world in [
        cat1::world(),
        cat2::world(),
        cat3::world(),
        cat4::world(),
        cat5::world(),
    ] {
        rows.extend(common_rows(&world));
        rows.extend(refusal_rows(&world));
    }
    rows.extend(cat1::rows());
    rows.extend(cat2::rows());
    rows.extend(cat3::rows());
    rows.extend(cat4::rows());
    rows.extend(cat5::rows());
    rows
}

/// Rows every family supports, built from its world alone.
#[allow(clippy::too_many_lines)] // One row per arm, each a few lines; splitting hides the matrix.
fn common_rows(world: &World) -> Vec<Scenario> {
    let cold = world.settled_cold_path();
    let unsettled_cold = if world.answers {
        TierSettlement::Pending
    } else {
        TierSettlement::NotRequired
    };
    let authority_without_answer = if world.answers {
        VerdictAuthority::Provisional
    } else {
        VerdictAuthority::Authoritative
    };
    let complete = world.declarations(&world.satisfy);
    let mut rows = Vec::new();

    // Satisfy over complete evidence.
    let mut actions = feed(&world.satisfy, &complete);
    actions.extend(world.tail());
    rows.push(stepped(
        world.row("satisfy"),
        Claim::Sealed(satisfy(cold, ActivationBarrierStatus::NotDeclared)),
        world.spec.clone().evaluator(),
        actions,
    ));

    // Closed gate: one required source never declares (G1b).
    let partial: Vec<SourceEvidence> = complete
        .iter()
        .filter(|evidence| evidence.source() != &source(world.withheld))
        .cloned()
        .collect();
    let mut actions = feed(&world.satisfy, &partial);
    actions.extend(world.tail());
    rows.push(stepped(
        world.row("closed-gate"),
        Claim::Sealed(inconclusive(
            InconclusiveReason::PrefixUncertainty,
            VerdictAuthority::Provisional,
            TierSettlement::Pending,
            ActivationBarrierStatus::NotDeclared,
        )),
        world.spec.clone().evaluator(),
        actions,
    ));

    // Liveness: a barrier that clears by activation, then fresh or stale.
    for (arm, floor, labels) in [
        (
            "liveness-fresh",
            SIGNAL_OBSERVED_AT,
            satisfy(cold, ActivationBarrierStatus::Cleared),
        ),
        (
            "liveness-stale",
            SIGNAL_OBSERVED_AT + 1,
            provisional_satisfy(cold),
        ),
    ] {
        let mut actions = feed(&world.satisfy, &complete);
        actions.extend(world.tail());
        rows.push(stepped(
            world.row(arm),
            Claim::Sealed(labels),
            world
                .spec
                .clone()
                .with_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH))
                .evaluator()
                .with_liveness_freshness(liveness(floor)),
            actions,
        ));
    }

    // A barrier pending on a Filter epoch no source acknowledges yet.
    let mut actions = feed(&world.satisfy, &complete);
    actions.extend(world.tail());
    rows.push(stepped(
        world.row("barrier-pending"),
        Claim::Sealed(provisional_satisfy(cold)),
        world
            .spec
            .clone()
            .with_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH + 1))
            .evaluator(),
        actions,
    ));

    // The same pending barrier with one required source silent throughout.
    let (silent, events) = &world.silent;
    let mut actions = feed(events, &world.declarations(events));
    actions.extend(world.tail());
    assert!(
        world
            .declarations(events)
            .iter()
            .all(|evidence| evidence.source() != &source(silent)),
        "{}: the silent source must deliver nothing",
        world.prefix
    );
    rows.push(stepped(
        world.row("barrier-pending-silent-source"),
        Claim::Sealed(inconclusive(
            InconclusiveReason::PrefixUncertainty,
            VerdictAuthority::Provisional,
            TierSettlement::Pending,
            ActivationBarrierStatus::Pending,
        )),
        world
            .spec
            .clone()
            .with_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH + 1))
            .evaluator(),
        actions,
    ));

    // A source that truthfully reports a gap in what it emitted (G2).
    let noisy = world.noisy;
    let top = highest(&world.satisfy, noisy);
    let mut events = world.satisfy.clone();
    events.push(filler("filler-gapped", noisy, top + 2, &[]));
    let declarations: Vec<SourceEvidence> = complete
        .iter()
        .map(|evidence| {
            if evidence.source() == &source(noisy) {
                declared(
                    noisy,
                    world.tier_of(noisy),
                    top + 2,
                    top + 2,
                    vec![SequenceGap::new(1, top + 1, top + 1)],
                )
            } else {
                evidence.clone()
            }
        })
        .collect();
    let mut actions = feed(&events, &declarations);
    actions.extend(world.tail());
    rows.push(stepped(
        world.row("gapped-source"),
        Claim::Sealed(inconclusive(
            InconclusiveReason::PrefixUncertainty,
            authority_without_answer,
            unsettled_cold,
            ActivationBarrierStatus::NotDeclared,
        )),
        world.spec.clone().evaluator(),
        actions,
    ));

    // A source whose completeness claim contradicts what it delivered (G3).
    let mut events = world.satisfy.clone();
    events.push(filler("filler-contradicting", noisy, top + 1, &[]));
    let mut actions = feed(&events, &complete);
    actions.extend(world.tail());
    rows.push(stepped(
        world.row("contradicting-source"),
        Claim::Sealed(inconclusive(
            InconclusiveReason::PrefixUncertainty,
            authority_without_answer,
            unsettled_cold,
            ActivationBarrierStatus::NotDeclared,
        )),
        world.spec.clone().evaluator(),
        actions,
    ));

    // A delivered Event whose declared predecessor never arrives (G4).
    let mut events = world.satisfy.clone();
    events.push(filler("filler-held", noisy, top + 1, &["never-arrived"]));
    let mut actions = feed(&events, &world.declarations(&events));
    actions.extend(world.tail());
    rows.push(stepped(
        world.row("unresolved-edge"),
        Claim::Sealed(inconclusive(
            InconclusiveReason::PrefixUncertainty,
            authority_without_answer,
            unsettled_cold,
            ActivationBarrierStatus::NotDeclared,
        )),
        world.spec.clone().evaluator(),
        actions,
    ));

    rows
}

/// The three `refuse_unconstructible_envelope`-order refusals, per family.
fn refusal_rows(world: &World) -> Vec<Scenario> {
    let mut rows = vec![
        stepped(
            world.row("refused-no-required-source"),
            Claim::Refused("SpecificationRequiresNoSource", |error| {
                matches!(error, CoreError::SpecificationRequiresNoSource { .. })
            }),
            world.spec.without_required_sources().evaluator(),
            feed(&world.satisfy, &[]),
        ),
        stepped(
            world.row("refused-no-evidence-coverage"),
            Claim::Refused("NoEvidenceCoverage", |error| {
                matches!(error, CoreError::NoEvidenceCoverage { .. })
            }),
            world.spec.clone().evaluator(),
            feed(&world.satisfy, &[]),
        ),
    ];
    if world.answers {
        rows.push(batch_facts_required(
            world.row("refused-facts-required-batch"),
            world.spec.clone().evaluator(),
            world.satisfy.clone(),
            &world.declarations(&world.satisfy),
        ));
    }
    rows
}

fn batch_facts_required(
    name: String,
    evaluator: Evaluator,
    events: Vec<Event>,
    declarations: &[SourceEvidence],
) -> Scenario {
    Scenario {
        name,
        claim: Claim::Refused("FactsRequired", |error| {
            matches!(error, CoreError::FactsRequired { .. })
        }),
        drive: Drive::Batch {
            evaluator: Box::new(evaluator),
            events,
            sources: declarations
                .iter()
                .map(|evidence| (evidence.source().clone(), evidence.clone()))
                .collect(),
        },
        also: None,
    }
}

/// A row whose events settle a family-specific arm over complete declarations.
fn arm(
    world: &World,
    name: &str,
    claim: Labels,
    events: &[Event],
    tiers: &[(&str, SourceTier)],
) -> Scenario {
    let declarations: Vec<SourceEvidence> = world
        .declarations(events)
        .into_iter()
        .map(|evidence| {
            let app = evidence
                .source()
                .monitored_application()
                .as_str()
                .to_owned();
            match tiers.iter().find(|(overridden, _)| *overridden == app) {
                Some((_, tier)) => {
                    let through = highest(events, &app);
                    declared(&app, *tier, through, through, Vec::new())
                }
                None => evidence,
            }
        })
        .collect();
    let mut actions = feed(events, &declarations);
    actions.extend(world.tail());
    stepped(
        world.row(name),
        Claim::Sealed(claim),
        world.spec.clone().evaluator(),
        actions,
    )
}

/// An authoritative observational Inconclusive, the concurrent-pair arm.
fn observational() -> Labels {
    inconclusive(
        InconclusiveReason::ObservationalUncertainty,
        VerdictAuthority::Authoritative,
        TierSettlement::NotRequired,
        ActivationBarrierStatus::NotDeclared,
    )
}

/// An authoritative prefix Inconclusive behind an open gate.
fn settled_prefix() -> Labels {
    inconclusive(
        InconclusiveReason::PrefixUncertainty,
        VerdictAuthority::Authoritative,
        TierSettlement::NotRequired,
        ActivationBarrierStatus::NotDeclared,
    )
}

/// The O-03 rows: retire while one source is withheld, then complete coverage.
///
/// `above` and `elsewhere` are each one further Slice member of [`BINDING`],
/// the first observed above the retirement horizon and the second on another
/// Sentinel clock, so the two exclusion counts the retired Explanation
/// discloses are each pinned non-zero once.
#[allow(clippy::too_many_lines)] // One row per retired arm, each a few lines.
fn retirement_rows(world: &World, above: Event, elsewhere: Event) -> Vec<Scenario> {
    let actions = |events: &[Event]| {
        let (early, late): (Vec<SourceEvidence>, Vec<SourceEvidence>) = world
            .declarations(events)
            .into_iter()
            .partition(|evidence| evidence.source() != &source(world.withheld));
        let mut actions = feed(events, &early);
        actions.push(retire());
        actions.extend(feed(&[], &late));
        actions
    };
    let with = |member: Event| {
        let mut events = world.satisfy.clone();
        events.push(member);
        events
    };
    let retired: Property = ("an Explanation carrying the retirement", |verdict| {
        verdict.explanation().retirement.is_some()
    });
    vec![
        Scenario {
            also: Some(retired),
            ..stepped(
                world.row("retired"),
                Claim::Sealed(settled_prefix()),
                world.spec.clone().evaluator(),
                actions(&world.satisfy),
            )
        },
        Scenario {
            also: Some(retired),
            ..stepped(
                world.row("retired-member-above-horizon"),
                Claim::Sealed(settled_prefix()),
                world.spec.clone().evaluator(),
                actions(&with(above)),
            )
        },
        Scenario {
            also: Some(retired),
            ..stepped(
                world.row("retired-member-on-another-clock"),
                Claim::Sealed(settled_prefix()),
                world.spec.clone().evaluator(),
                actions(&with(elsewhere)),
            )
        },
        // Every source first declares Idle, fresh under the idle floor, so
        // the barrier clears and the retired Binding settles Authoritative
        // once coverage completes. Then one source declares Active, stale
        // under the active floor: the barrier is pending again and authority
        // falls back to Provisional. The retired emission key is sticky once
        // Authoritative, so that regression emits nothing.
        Scenario {
            also: Some(retired),
            ..stepped(
                world.row("retired-liveness-goes-stale"),
                Claim::Sealed(inconclusive(
                    InconclusiveReason::PrefixUncertainty,
                    VerdictAuthority::Provisional,
                    TierSettlement::NotRequired,
                    ActivationBarrierStatus::Pending,
                )),
                world
                    .spec
                    .clone()
                    .with_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH))
                    .evaluator()
                    .with_liveness_freshness(liveness(SIGNAL_OBSERVED_AT + 10)),
                {
                    let idle = |app: &str| {
                        let through = highest(&world.satisfy, app);
                        signalled(
                            app,
                            world.tier_of(app),
                            (through, through),
                            Vec::new(),
                            SourceActivity::Idle {
                                since: occurrence(0),
                            },
                            SIGNAL_OBSERVED_AT,
                        )
                    };
                    let (early, late): (Vec<_>, Vec<_>) = world
                        .tiers
                        .iter()
                        .map(|(app, _)| *app)
                        .partition(|app| *app != world.withheld);
                    let early: Vec<SourceEvidence> = early.into_iter().map(idle).collect();
                    let late: Vec<SourceEvidence> = late.into_iter().map(idle).collect();
                    let noisy = world.noisy;
                    let through = highest(&world.satisfy, noisy);
                    let mut actions = feed(&world.satisfy, &early);
                    actions.push(retire());
                    actions.extend(feed(&[], &late));
                    actions.extend(feed(
                        &[],
                        &[signalled(
                            noisy,
                            world.tier_of(noisy),
                            (through, through),
                            Vec::new(),
                            SourceActivity::Active {
                                since: occurrence(0),
                            },
                            SIGNAL_OBSERVED_AT + 1,
                        )],
                    ));
                    actions
                },
            )
        },
        Scenario {
            also: Some(retired),
            ..stepped(
                world.row("retired-barrier-pending"),
                Claim::Sealed(inconclusive(
                    InconclusiveReason::PrefixUncertainty,
                    VerdictAuthority::Provisional,
                    TierSettlement::NotRequired,
                    ActivationBarrierStatus::Pending,
                )),
                world
                    .spec
                    .clone()
                    .with_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH + 1))
                    .evaluator(),
                actions(&world.satisfy),
            )
        },
    ]
}

/// A family arm's Events driven under an activation barrier still pending on
/// a Filter epoch no source acknowledges.
fn barrier_pending_arm(world: &World, name: &str, claim: Labels, events: &[Event]) -> Scenario {
    let mut actions = feed(events, &world.declarations(events));
    actions.extend(world.tail());
    stepped(
        world.row(name),
        Claim::Sealed(claim),
        world
            .spec
            .clone()
            .with_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH + 1))
            .evaluator(),
        actions,
    )
}

/// A Violate that a pending barrier keeps Provisional.
fn provisional_violate() -> Labels {
    Labels {
        outcome: VerdictOutcome::Violate,
        reason: None,
        authority: VerdictAuthority::Provisional,
        cold_path: TierSettlement::NotRequired,
        barrier: ActivationBarrierStatus::Pending,
        basis: None,
    }
}

/// Several Bindings over complete declarations, each claimed in seal order.
///
/// `events` deliver the Bindings out of their sorted order, so a seal that
/// stopped sorting moves the row as surely as one that reversed the sort.
fn bindings_row(
    world: &World,
    claims: Vec<(&'static str, Labels)>,
    events: &[Event],
    tail: Vec<Action>,
) -> Scenario {
    let mut actions = feed(events, &world.declarations(events));
    actions.extend(tail);
    stepped(
        world.row("two-bindings-and-one-without-obligation"),
        Claim::Bindings(claims),
        world.spec.clone().evaluator(),
        actions,
    )
}

// ---------------------------------------------------------------------------
// CAT-1: precedence.approval
// ---------------------------------------------------------------------------

mod cat1 {
    use super::{
        above_horizon, arm, barrier_pending_arm, batch_facts_required, bindings_row, bound,
        bound_to, catalogued, feed, filler, inconclusive, observational, on_another_clock,
        provisional_satisfy, provisional_violate, retirement_rows, satisfy, selector,
        settled_prefix, stepped, violate, Action, ActivationBarrierStatus, Claim, ColdGuardId,
        GuardConclusion, InconclusiveReason, PatternId, PrecedenceSpecification, Property,
        RefusalReason, RetirementPolicy, Scenario, SettlementBasisKind, SourceTier, Spec,
        TierSettlement, VerdictAuthority, World, ACKNOWLEDGED_FILTER_EPOCH, GUARD,
    };
    use std::collections::{BTreeMap, BTreeSet};

    const GATEWAY: &str = "example-action-gateway";
    const APPROVER: &str = "example-approval-service";
    const TRIGGER: &str = "example.action.requested";
    const REQUIREMENT: &str = "example.approval.recorded";
    const STATUS: &str = "example.approval.status";

    fn specification() -> PrecedenceSpecification {
        let instance = catalogued(
            PatternId::PrecedenceApproval,
            "example.action-requires-prior-approval",
            BTreeMap::from([
                ("trigger".to_owned(), selector(TRIGGER, &[])),
                (
                    "requirement".to_owned(),
                    selector(REQUIREMENT, &[(STATUS, "granted")]),
                ),
            ]),
            &[
                ("trigger_source", GATEWAY, SourceTier::Contributing),
                ("requirement_source", APPROVER, SourceTier::Anchor),
            ],
        );
        PrecedenceSpecification::try_from(
            instance
                .ir()
                .as_precedence()
                .expect("CAT-1 lowers to the precedence monitor"),
        )
        .expect("catalogued CAT-1 IR converts")
    }

    fn approval(status: &str) -> super::Event {
        bound(
            "approve-1",
            APPROVER,
            1,
            REQUIREMENT,
            &[(STATUS, status)],
            &[],
        )
    }

    fn trigger(predecessors: &[&str]) -> super::Event {
        bound("act-1", GATEWAY, 1, TRIGGER, &[], predecessors)
    }

    pub(super) fn world() -> World {
        World {
            prefix: "cat1",
            spec: Spec::Precedence(specification()),
            tiers: vec![
                (GATEWAY, SourceTier::Contributing),
                (APPROVER, SourceTier::Anchor),
            ],
            satisfy: vec![approval("granted"), trigger(&["approve-1"])],
            withheld: APPROVER,
            silent: (APPROVER, vec![trigger(&[])]),
            noisy: GATEWAY,
            answers: false,
        }
    }

    fn guarded() -> PrecedenceSpecification {
        specification().with_cold_guards(BTreeSet::from([ColdGuardId::new(GUARD, "v1")]))
    }

    #[allow(clippy::too_many_lines)] // One row per CAT-1-only arm.
    pub(super) fn rows() -> Vec<Scenario> {
        let world = world();
        let silent_approver = [filler("audit-1", APPROVER, 1, &[]), trigger(&[])];
        let complete = world.declarations(&world.satisfy);
        let guarded_actions = |conclusion: GuardConclusion, lift: u64| {
            let mut actions = feed(&world.satisfy, &complete);
            actions.push(Action::Ask);
            actions.push(Action::Answer { conclusion, lift });
            actions
        };
        let clamped: Property = (
            "cold settlement Watermarks clamped to the evidence",
            |verdict| {
                let verdict = verdict.verdict();
                verdict.cold_settlement_watermarks() == verdict.hot_settlement_watermarks()
            },
        );
        let cold_kept: Property = (
            "cold settlement Watermarks kept under the pending barrier",
            |verdict| !verdict.verdict().cold_settlement_watermarks().is_empty(),
        );
        let two_bindings = [
            bound_to(
                "req-2",
                "approve-2",
                APPROVER,
                1,
                REQUIREMENT,
                &[(STATUS, "denied")],
                &[],
            ),
            bound_to("req-2", "act-2", GATEWAY, 1, TRIGGER, &[], &["approve-2"]),
            bound_to(
                "req-1",
                "approve-1",
                APPROVER,
                2,
                REQUIREMENT,
                &[(STATUS, "granted")],
                &[],
            ),
            bound_to("req-1", "act-1", GATEWAY, 2, TRIGGER, &[], &["approve-1"]),
            // An approval with no action: it raises no obligation.
            bound_to(
                "req-3",
                "approve-3",
                APPROVER,
                3,
                REQUIREMENT,
                &[(STATUS, "granted")],
                &[],
            ),
        ];
        let mut rows = vec![
            arm(
                &world,
                "violate-decisive",
                violate(SettlementBasisKind::DecisiveViolation),
                &[approval("denied"), trigger(&["approve-1"])],
                &[],
            ),
            barrier_pending_arm(
                &world,
                "violate-decisive-barrier-pending",
                provisional_violate(),
                &[approval("denied"), trigger(&["approve-1"])],
            ),
            barrier_pending_arm(
                &world,
                "violate-watermark-anchored-barrier-pending",
                provisional_violate(),
                &silent_approver,
            ),
            bindings_row(
                &world,
                vec![
                    (
                        "req-1",
                        satisfy(
                            TierSettlement::NotRequired,
                            ActivationBarrierStatus::NotDeclared,
                        ),
                    ),
                    ("req-2", violate(SettlementBasisKind::DecisiveViolation)),
                ],
                &two_bindings,
                Vec::new(),
            ),
            Scenario {
                also: Some(cold_kept),
                ..stepped(
                    world.row("guarded-holds-barrier-pending"),
                    Claim::Sealed(provisional_satisfy(TierSettlement::Settled)),
                    super::Evaluator::new(
                        guarded()
                            .with_activation_barrier(world.barrier(ACKNOWLEDGED_FILTER_EPOCH + 1)),
                    ),
                    guarded_actions(GuardConclusion::Holds, 0),
                )
            },
            arm(
                &world,
                "violate-watermark-anchored",
                violate(SettlementBasisKind::WatermarkAnchoredViolation),
                &silent_approver,
                &[],
            ),
            arm(
                &world,
                "contributing-only",
                settled_prefix(),
                &silent_approver,
                &[(APPROVER, SourceTier::Contributing)],
            ),
            arm(
                &world,
                "concurrent",
                observational(),
                &[approval("granted"), trigger(&[])],
                &[],
            ),
            Scenario {
                also: Some(clamped),
                ..stepped(
                    world.row("guarded-holds-clamped"),
                    Claim::Sealed(satisfy(
                        TierSettlement::Settled,
                        ActivationBarrierStatus::NotDeclared,
                    )),
                    super::Evaluator::new(guarded()),
                    guarded_actions(GuardConclusion::Holds, 5),
                )
            },
            stepped(
                world.row("guarded-refused"),
                Claim::Sealed(inconclusive(
                    InconclusiveReason::EpistemicUncertainty,
                    VerdictAuthority::Authoritative,
                    TierSettlement::Settled,
                    ActivationBarrierStatus::NotDeclared,
                )),
                super::Evaluator::new(guarded()),
                guarded_actions(
                    GuardConclusion::Refused {
                        reason: RefusalReason::EvidenceReaped,
                    },
                    0,
                ),
            ),
            batch_facts_required(
                world.row("refused-facts-required-batch"),
                super::Evaluator::new(guarded()),
                world.satisfy.clone(),
                &complete,
            ),
        ];
        let retiring = World {
            spec: Spec::Precedence(
                specification().with_retirement_policy(RetirementPolicy::Deadline),
            ),
            ..world
        };
        rows.extend(retirement_rows(
            &retiring,
            above_horizon(
                "approve-late",
                APPROVER,
                2,
                REQUIREMENT,
                &[(STATUS, "granted")],
            ),
            on_another_clock(
                "approve-replica",
                APPROVER,
                2,
                REQUIREMENT,
                &[(STATUS, "granted")],
            ),
        ));
        rows
    }
}

// ---------------------------------------------------------------------------
// CAT-2: absence.after-until
// ---------------------------------------------------------------------------

mod cat2 {
    use super::{
        arm, barrier_pending_arm, bindings_row, bound, bound_to, catalogued, feed, observational,
        provisional_violate, selector, settled_prefix, stepped, violate, AbsenceSpecification,
        Claim, PatternId, RawEventRetentionDays, Scenario, SettlementBasisKind, SourceTier, Spec,
        World,
    };
    use super::{satisfy, ActivationBarrierStatus, TierSettlement};
    use std::collections::BTreeMap;

    const APPROVAL: &str = "example-approval-service";
    const ACTION: &str = "example-action-gateway";
    const REVOCATION: &str = "example-revocation-bus";
    const AFTER: &str = "example.approval.granted";
    const UNTIL: &str = "example.action.committed";
    const VETO: &str = "example.approval.revoked";

    fn specification() -> AbsenceSpecification {
        let instance = catalogued(
            PatternId::AbsenceAfterUntil,
            "example.no-revoke-before-commit",
            BTreeMap::from([
                ("after".to_owned(), selector(AFTER, &[])),
                ("until".to_owned(), selector(UNTIL, &[])),
                ("veto".to_owned(), selector(VETO, &[])),
            ]),
            &[
                ("after_source", APPROVAL, SourceTier::Contributing),
                ("until_source", ACTION, SourceTier::Anchor),
                ("veto_source", REVOCATION, SourceTier::Anchor),
            ],
        );
        AbsenceSpecification::try_from(
            instance
                .ir()
                .as_absence()
                .expect("CAT-2 lowers to the absence monitor"),
        )
        .expect("catalogued CAT-2 IR converts")
    }

    fn after() -> super::Event {
        bound("after-1", APPROVAL, 1, AFTER, &[], &[])
    }

    fn until(predecessors: &[&str]) -> super::Event {
        bound("until-1", ACTION, 1, UNTIL, &[], predecessors)
    }

    fn veto(id: &str, predecessors: &[&str]) -> super::Event {
        bound(id, REVOCATION, 1, VETO, &[], predecessors)
    }

    pub(super) fn world() -> World {
        World {
            prefix: "cat2",
            spec: Spec::Absence(specification()),
            tiers: vec![
                (APPROVAL, SourceTier::Contributing),
                (ACTION, SourceTier::Anchor),
                (REVOCATION, SourceTier::Anchor),
            ],
            satisfy: vec![
                after(),
                until(&["after-1"]),
                veto("veto-after-close", &["until-1"]),
            ],
            withheld: REVOCATION,
            silent: (REVOCATION, vec![after(), until(&["after-1"])]),
            noisy: ACTION,
            answers: false,
        }
    }

    pub(super) fn rows() -> Vec<Scenario> {
        let world = world();
        let violating = [after(), veto("veto-1", &["after-1"]), until(&["veto-1"])];
        let complete = world.declarations(&world.satisfy);
        let two_bindings = [
            bound_to("req-2", "after-2", APPROVAL, 1, AFTER, &[], &[]),
            bound_to("req-2", "veto-2", REVOCATION, 1, VETO, &[], &["after-2"]),
            bound_to("req-2", "until-2", ACTION, 1, UNTIL, &[], &["veto-2"]),
            bound_to("req-1", "after-1", APPROVAL, 2, AFTER, &[], &[]),
            bound_to("req-1", "until-1", ACTION, 2, UNTIL, &[], &["after-1"]),
            bound_to(
                "req-1",
                "veto-after-close",
                REVOCATION,
                2,
                VETO,
                &[],
                &["until-1"],
            ),
            // A revocation with no approval before it: it raises no obligation.
            bound_to("req-3", "veto-3", REVOCATION, 3, VETO, &[], &[]),
        ];
        vec![
            arm(
                &world,
                "violate-decisive",
                violate(SettlementBasisKind::DecisiveViolation),
                &violating,
                &[],
            ),
            barrier_pending_arm(
                &world,
                "violate-decisive-barrier-pending",
                provisional_violate(),
                &violating,
            ),
            bindings_row(
                &world,
                vec![
                    (
                        "req-1",
                        satisfy(
                            TierSettlement::NotRequired,
                            ActivationBarrierStatus::NotDeclared,
                        ),
                    ),
                    ("req-2", violate(SettlementBasisKind::DecisiveViolation)),
                ],
                &two_bindings,
                Vec::new(),
            ),
            arm(
                &world,
                "contributing-only",
                settled_prefix(),
                &violating,
                &[(REVOCATION, SourceTier::Contributing)],
            ),
            arm(
                &world,
                "concurrent",
                observational(),
                &[after(), until(&[]), veto("veto-1", &["until-1"])],
                &[],
            ),
            stepped(
                world.row("raw-event-retention-override"),
                Claim::Sealed(satisfy(
                    TierSettlement::NotRequired,
                    ActivationBarrierStatus::NotDeclared,
                )),
                world.spec.clone().evaluator().with_raw_event_retention(
                    RawEventRetentionDays::try_new(90).expect("90 days is a valid horizon"),
                ),
                feed(&world.satisfy, &complete),
            ),
        ]
    }
}

// ---------------------------------------------------------------------------
// CAT-3: existence.before-scope-close
// ---------------------------------------------------------------------------

mod cat3 {
    use super::{
        above_horizon, arm, barrier_pending_arm, bindings_row, bound, bound_to, catalogued,
        observational, on_another_clock, provisional_violate, retirement_rows, satisfy, selector,
        settled_prefix, violate, ActivationBarrierStatus, Claim, CoreError, ExistenceSpecification,
        PatternId, Scenario, SettlementBasisKind, SourceTier, Spec, TierSettlement, VerdictError,
        World,
    };
    use std::collections::BTreeMap;

    const ORCHESTRATOR: &str = "example-saga-orchestrator";
    const MONEY: &str = "example-money-system";
    const JOURNAL: &str = "example-broker-journal";
    const OPEN: &str = "example.saga.opened";
    const REQUIRED: &str = "example.saga.compensated";
    const CLOSE: &str = "example.saga.terminated";

    fn specification() -> ExistenceSpecification {
        let instance = catalogued(
            PatternId::ExistenceBeforeScopeClose,
            "example.compensation-before-terminate",
            BTreeMap::from([
                ("open".to_owned(), selector(OPEN, &[])),
                ("required".to_owned(), selector(REQUIRED, &[])),
                ("close".to_owned(), selector(CLOSE, &[])),
            ]),
            &[
                ("open_source", ORCHESTRATOR, SourceTier::Contributing),
                ("required_source", MONEY, SourceTier::Anchor),
                ("close_source", JOURNAL, SourceTier::Contributing),
            ],
        );
        ExistenceSpecification::try_from(
            instance
                .ir()
                .as_existence()
                .expect("CAT-3 lowers to the existence monitor"),
        )
        .expect("catalogued CAT-3 IR converts")
    }

    fn open(id: &str, sequence: u64, predecessors: &[&str]) -> super::Event {
        bound(id, ORCHESTRATOR, sequence, OPEN, &[], predecessors)
    }

    fn required(id: &str, predecessors: &[&str]) -> super::Event {
        bound(id, MONEY, 1, REQUIRED, &[], predecessors)
    }

    fn close(id: &str, sequence: u64, predecessors: &[&str]) -> super::Event {
        bound(id, JOURNAL, sequence, CLOSE, &[], predecessors)
    }

    pub(super) fn world() -> World {
        World {
            prefix: "cat3",
            spec: Spec::Existence(specification()),
            tiers: vec![
                (ORCHESTRATOR, SourceTier::Contributing),
                (MONEY, SourceTier::Anchor),
                (JOURNAL, SourceTier::Contributing),
            ],
            satisfy: vec![
                open("open-1", 1, &[]),
                required("comp-1", &["open-1"]),
                close("close-1", 1, &["comp-1"]),
            ],
            withheld: MONEY,
            silent: (
                MONEY,
                vec![open("open-1", 1, &[]), close("close-1", 1, &["open-1"])],
            ),
            noisy: JOURNAL,
            answers: false,
        }
    }

    #[allow(clippy::too_many_lines)] // One row per family-only arm.
    pub(super) fn rows() -> Vec<Scenario> {
        let world = world();
        let missing_required = [
            open("open-1", 1, &[]),
            close("close-1", 1, &["open-1"]),
            required("comp-after-close", &["close-1"]),
        ];
        let two_bindings = [
            bound_to("req-2", "open-2", ORCHESTRATOR, 1, OPEN, &[], &[]),
            bound_to("req-2", "close-2", JOURNAL, 1, CLOSE, &[], &["open-2"]),
            bound_to(
                "req-2",
                "comp-after-close",
                MONEY,
                1,
                REQUIRED,
                &[],
                &["close-2"],
            ),
            bound_to("req-1", "open-1", ORCHESTRATOR, 2, OPEN, &[], &[]),
            bound_to("req-1", "comp-1", MONEY, 2, REQUIRED, &[], &["open-1"]),
            bound_to("req-1", "close-1", JOURNAL, 2, CLOSE, &[], &["comp-1"]),
            // A compensation with no open Scope: it raises no obligation.
            bound_to("req-3", "comp-3", MONEY, 3, REQUIRED, &[], &[]),
        ];
        let mut rows = vec![
            // CAT-3's DecisiveViolation arm, pinned as the refusal it is today.
            //
            // Two opens in one Binding: the first discharged by an anchor-tier
            // compensation, the second closed without one. The discharge puts
            // an Event in `anchored_events`, so envelope assembly takes the
            // DecisiveViolation arm instead of the watermark-anchored one —
            // but CAT-3's witness set names only the violated `open`, which is
            // never an anchored Event, and the model refuses a decisive basis
            // with no anchored witness. The evidence condition therefore
            // surfaces as `CoreError::Verdict`, which the Core reserves for its
            // own bugs. Pinned as-is, not fixed: raised on #275.
            Scenario {
                claim: Claim::Refused("Verdict(MissingAnchorEvidence)", |error| {
                    matches!(
                        error,
                        CoreError::Verdict(VerdictError::MissingAnchorEvidence)
                    )
                }),
                ..arm(
                    &world,
                    "violate-decisive-unconstructible",
                    violate(SettlementBasisKind::DecisiveViolation),
                    &[
                        open("open-1", 1, &[]),
                        required("comp-1", &["open-1"]),
                        close("close-1", 1, &["comp-1"]),
                        open("open-2", 2, &["close-1"]),
                        close("close-2", 2, &["open-2"]),
                    ],
                    &[],
                )
            },
            arm(
                &world,
                "violate-watermark-anchored",
                violate(SettlementBasisKind::WatermarkAnchoredViolation),
                &missing_required,
                &[],
            ),
            barrier_pending_arm(
                &world,
                "violate-watermark-anchored-barrier-pending",
                provisional_violate(),
                &missing_required,
            ),
            bindings_row(
                &world,
                vec![
                    (
                        "req-1",
                        satisfy(
                            TierSettlement::NotRequired,
                            ActivationBarrierStatus::NotDeclared,
                        ),
                    ),
                    (
                        "req-2",
                        violate(SettlementBasisKind::WatermarkAnchoredViolation),
                    ),
                ],
                &two_bindings,
                Vec::new(),
            ),
            arm(
                &world,
                "contributing-only",
                settled_prefix(),
                &missing_required,
                &[(MONEY, SourceTier::Contributing)],
            ),
            arm(
                &world,
                "concurrent",
                observational(),
                &[
                    open("open-1", 1, &[]),
                    close("close-1", 1, &[]),
                    required("comp-1", &["close-1"]),
                ],
                &[],
            ),
        ];
        rows.extend(retirement_rows(
            &world,
            above_horizon("comp-late", MONEY, 2, REQUIRED, &[]),
            on_another_clock("comp-replica", MONEY, 2, REQUIRED, &[]),
        ));
        rows
    }
}

// ---------------------------------------------------------------------------
// CAT-4: provenance.taint
// ---------------------------------------------------------------------------

mod cat4 {
    use super::{
        arm, barrier_pending_arm, bindings_row, bound, bound_to, catalogued, observational,
        provisional_violate, satisfy, selector, settled_prefix, violate, ActivationBarrierStatus,
        PatternId, Scenario, SettlementBasisKind, SourceTier, Spec, TaintSpecification,
        TierSettlement, World,
    };
    use std::collections::BTreeMap;

    const MAIL: &str = "example-mail-gateway";
    const BROKER: &str = "example-content-broker";
    const ACTION: &str = "example.email.sent";
    const CONTENT: &str = "example.content.ingested";
    const TRUST: &str = "example.content.trust";

    fn specification() -> TaintSpecification {
        let instance = catalogued(
            PatternId::ProvenanceTaint,
            "example.action-untainted",
            BTreeMap::from([
                ("action".to_owned(), selector(ACTION, &[])),
                (
                    "untrusted_content".to_owned(),
                    selector(CONTENT, &[(TRUST, "untrusted")]),
                ),
            ]),
            &[
                ("action_source", MAIL, SourceTier::Anchor),
                ("content_source", BROKER, SourceTier::Anchor),
            ],
        );
        TaintSpecification::try_from(
            instance
                .ir()
                .as_taint()
                .expect("CAT-4 lowers to the taint monitor"),
        )
        .expect("catalogued CAT-4 IR converts")
    }

    fn content(trust: &str) -> super::Event {
        bound("content-1", BROKER, 1, CONTENT, &[(TRUST, trust)], &[])
    }

    fn action(predecessors: &[&str]) -> super::Event {
        bound("mail-1", MAIL, 1, ACTION, &[], predecessors)
    }

    pub(super) fn world() -> World {
        World {
            prefix: "cat4",
            spec: Spec::Taint(specification()),
            tiers: vec![(MAIL, SourceTier::Anchor), (BROKER, SourceTier::Anchor)],
            satisfy: vec![content("trusted"), action(&[])],
            withheld: BROKER,
            silent: (BROKER, vec![action(&[])]),
            noisy: MAIL,
            answers: false,
        }
    }

    pub(super) fn rows() -> Vec<Scenario> {
        let world = world();
        let tainted = [content("untrusted"), action(&["content-1"])];
        let two_bindings = [
            bound_to(
                "req-2",
                "content-2",
                BROKER,
                1,
                CONTENT,
                &[(TRUST, "untrusted")],
                &[],
            ),
            bound_to("req-2", "mail-2", MAIL, 1, ACTION, &[], &["content-2"]),
            bound_to(
                "req-1",
                "content-1",
                BROKER,
                2,
                CONTENT,
                &[(TRUST, "trusted")],
                &[],
            ),
            bound_to("req-1", "mail-1", MAIL, 2, ACTION, &[], &[]),
            // Untrusted content no action reads: it raises no obligation.
            bound_to(
                "req-3",
                "content-3",
                BROKER,
                3,
                CONTENT,
                &[(TRUST, "untrusted")],
                &[],
            ),
        ];
        vec![
            arm(
                &world,
                "violate-decisive",
                violate(SettlementBasisKind::DecisiveViolation),
                &tainted,
                &[],
            ),
            barrier_pending_arm(
                &world,
                "violate-decisive-barrier-pending",
                provisional_violate(),
                &tainted,
            ),
            bindings_row(
                &world,
                vec![
                    (
                        "req-1",
                        satisfy(
                            TierSettlement::NotRequired,
                            ActivationBarrierStatus::NotDeclared,
                        ),
                    ),
                    ("req-2", violate(SettlementBasisKind::DecisiveViolation)),
                ],
                &two_bindings,
                Vec::new(),
            ),
            arm(
                &world,
                "contributing-only",
                settled_prefix(),
                &tainted,
                &[(BROKER, SourceTier::Contributing)],
            ),
            arm(
                &world,
                "concurrent",
                observational(),
                &[content("untrusted"), action(&[])],
                &[],
            ),
        ]
    }
}

// ---------------------------------------------------------------------------
// CAT-5: aggregation.count-within
// ---------------------------------------------------------------------------

mod cat5 {
    use super::{
        bindings_row, bound, bound_to, catalogued, feed, inconclusive, satisfy, selector, source,
        stepped, Action, ActivationBarrierStatus, AggregateSpecification, Claim, ColdWitness,
        CoreError, EventId, GuardConclusion, InconclusiveReason, ParameterValue, PatternId,
        Property, RefusalReason, Scenario, SequenceEpoch, SequenceNumber, SourceTier, Spec,
        TierSettlement, VerdictAuthority, VerdictError, World,
    };
    use std::collections::{BTreeMap, BTreeSet};

    const ASSESSOR: &str = "example-assessor";
    const LEDGER: &str = "example-ledger";
    const TRIGGER: &str = "example.assessment.started";
    const MEMBER: &str = "example.transfer.attempted";

    fn specification() -> AggregateSpecification {
        let instance = catalogued(
            PatternId::AggregationCountWithin,
            "example.transfer-rate-cap",
            BTreeMap::from([
                ("trigger".to_owned(), selector(TRIGGER, &[])),
                ("member".to_owned(), selector(MEMBER, &[])),
                (
                    "lookback".to_owned(),
                    ParameterValue::SequenceLookbackBound(16),
                ),
                ("threshold".to_owned(), ParameterValue::CountThreshold(3)),
            ]),
            &[
                ("trigger_source", ASSESSOR, SourceTier::Anchor),
                ("member_source", LEDGER, SourceTier::Anchor),
            ],
        );
        AggregateSpecification::try_from(
            instance
                .ir()
                .as_aggregate()
                .expect("CAT-5 lowers to the aggregate monitor"),
        )
        .expect("catalogued CAT-5 IR converts")
    }

    fn member(id: &str, sequence: u64) -> super::Event {
        bound(id, LEDGER, sequence, MEMBER, &[], &[])
    }

    fn trigger(predecessors: &[&str]) -> super::Event {
        bound("trigger-1", ASSESSOR, 1, TRIGGER, &[], predecessors)
    }

    pub(super) fn world() -> World {
        World {
            prefix: "cat5",
            spec: Spec::Aggregate(specification()),
            tiers: vec![(ASSESSOR, SourceTier::Anchor), (LEDGER, SourceTier::Anchor)],
            satisfy: vec![
                member("member-1", 1),
                member("member-2", 2),
                trigger(&["member-1", "member-2"]),
            ],
            withheld: LEDGER,
            silent: (LEDGER, vec![trigger(&[])]),
            noisy: ASSESSOR,
            answers: true,
        }
    }

    #[allow(clippy::too_many_lines)] // One row per CAT-5-only arm.
    pub(super) fn rows() -> Vec<Scenario> {
        let world = world();
        let complete = world.declarations(&world.satisfy);
        let answered = |conclusion: GuardConclusion, lift: u64| {
            let mut actions = feed(&world.satisfy, &complete);
            actions.push(Action::Ask);
            actions.push(Action::Answer { conclusion, lift });
            actions
        };
        let clamped: Property = (
            "cold settlement Watermarks clamped to the evidence",
            |verdict| {
                let verdict = verdict.verdict();
                verdict.cold_settlement_watermarks() == verdict.hot_settlement_watermarks()
            },
        );
        let epistemic = |authority| {
            inconclusive(
                InconclusiveReason::EpistemicUncertainty,
                authority,
                TierSettlement::Settled,
                ActivationBarrierStatus::NotDeclared,
            )
        };
        let catalogued = specification();
        let unasked = AggregateSpecification::new(
            catalogued.identity().clone(),
            catalogued.binding_attribute().clone(),
            catalogued.trigger().clone(),
            catalogued.member().clone(),
            catalogued.lookback_sequences(),
            catalogued.threshold(),
            catalogued.required_sources().clone(),
            catalogued.schema_version().clone(),
        );
        let contributing_trigger: Vec<super::SourceEvidence> = complete
            .iter()
            .map(|evidence| {
                if evidence.source() == &source(ASSESSOR) {
                    super::declared(ASSESSOR, SourceTier::Contributing, 1, 1, Vec::new())
                } else {
                    evidence.clone()
                }
            })
            .collect();
        let mut contributing_actions = feed(&world.satisfy, &contributing_trigger);
        contributing_actions.extend(world.tail());
        let two_bindings = [
            bound_to("req-2", "member-3", LEDGER, 1, MEMBER, &[], &[]),
            bound_to(
                "req-2",
                "trigger-2",
                ASSESSOR,
                1,
                TRIGGER,
                &[],
                &["member-3"],
            ),
            bound_to("req-1", "member-1", LEDGER, 2, MEMBER, &[], &[]),
            bound_to("req-1", "member-2", LEDGER, 3, MEMBER, &[], &[]),
            bound_to(
                "req-1",
                "trigger-1",
                ASSESSOR,
                2,
                TRIGGER,
                &[],
                &["member-1", "member-2"],
            ),
            // A member with no assessment: it raises no obligation.
            bound_to("req-3", "member-4", LEDGER, 4, MEMBER, &[], &[]),
        ];
        let per_binding = vec![
            Action::Ask,
            Action::AnswerEach {
                conclusions: vec![
                    ("req-1", GuardConclusion::Holds),
                    (
                        "req-2",
                        GuardConclusion::Fails {
                            witnesses: BTreeSet::from([ColdWitness::new(
                                EventId::new("member-3").expect("constant id"),
                                source(LEDGER),
                                (SequenceEpoch::new(1), SequenceNumber::new(1)),
                            )]),
                        },
                    ),
                ],
                lift: 0,
            },
        ];
        vec![
            bindings_row(
                &world,
                vec![
                    (
                        "req-1",
                        satisfy(
                            TierSettlement::Settled,
                            ActivationBarrierStatus::NotDeclared,
                        ),
                    ),
                    ("req-2", epistemic(VerdictAuthority::Authoritative)),
                ],
                &two_bindings,
                per_binding,
            ),
            Scenario {
                also: Some(clamped),
                ..stepped(
                    world.row("holds-clamped"),
                    Claim::Sealed(satisfy(
                        TierSettlement::Settled,
                        ActivationBarrierStatus::NotDeclared,
                    )),
                    world.spec.clone().evaluator(),
                    answered(GuardConclusion::Holds, 5),
                )
            },
            stepped(
                world.row("over-threshold"),
                Claim::Sealed(epistemic(VerdictAuthority::Authoritative)),
                world.spec.clone().evaluator(),
                answered(
                    GuardConclusion::Fails {
                        witnesses: BTreeSet::from([ColdWitness::new(
                            EventId::new("member-1").expect("constant id"),
                            source(LEDGER),
                            (SequenceEpoch::new(1), SequenceNumber::new(1)),
                        )]),
                    },
                    0,
                ),
            ),
            stepped(
                world.row("refused"),
                Claim::Sealed(epistemic(VerdictAuthority::Authoritative)),
                world.spec.clone().evaluator(),
                answered(
                    GuardConclusion::Refused {
                        reason: RefusalReason::EvidenceReaped,
                    },
                    0,
                ),
            ),
            stepped(
                world.row("unasked"),
                Claim::Sealed(inconclusive(
                    InconclusiveReason::EpistemicUncertainty,
                    VerdictAuthority::Authoritative,
                    TierSettlement::NotRequired,
                    ActivationBarrierStatus::NotDeclared,
                )),
                super::Evaluator::aggregate(unasked),
                feed(&world.satisfy, &complete),
            ),
            // A complete under-threshold count whose trigger sits on a
            // contributing-tier source, pinned as the refusal it is today.
            // `decide` reaches Satisfy and the settlement is Authoritative, but
            // only an anchor-tier trigger is cited, so the envelope carries a
            // tier gap and the model refuses an Authoritative Satisfy resting
            // on no anchor. The evidence condition therefore surfaces as
            // `CoreError::Verdict`, which the Core reserves for its own bugs.
            // Missing evidence is still not a pass. Pinned as-is: raised on #275.
            stepped(
                world.row("contributing-trigger-holds-unconstructible"),
                Claim::Refused("Verdict(MissingAnchorEvidence)", |error| {
                    matches!(
                        error,
                        CoreError::Verdict(VerdictError::MissingAnchorEvidence)
                    )
                }),
                world.spec.clone().evaluator(),
                contributing_actions,
            ),
        ]
    }
}
