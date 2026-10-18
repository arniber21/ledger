//! Assurance envelope assembly, and the read-side Explanation.
//!
//! **The Core does not get to decide what a trustworthy Verdict is; the model
//! does.** Every Verdict here is built as a `wire::model::v1::Verdict` and then
//! passed through `sentinel_model::Verdict::try_from`, so ADR-0017/0021/0022's
//! invariants are enforced by `sentinel_model`'s Verdict validators. An unjustified
//! authoritative Satisfy is literally unconstructible: the validators reject it
//! before this crate ever sees a `Verdict`.
//!
//! Evidence Coverage is not a new type. Per CONTEXT.md and ADR-0017 it *is* the
//! required-versus-observed / gaps / capabilities / clock-quality view of this
//! envelope. Note in particular that clock quality is expressed by the
//! **absence** of `BoundedClockUncertainty` from `capabilities`, which is the
//! honest statement for a recorded artifact whose wall time is diagnostics only.
//!
//! # Authority and tier settlement
//!
//! `authority`, `cold_path` and `cold_settlement_watermarks` are **not**
//! computed here. They come from [`crate::settlement`], which reads both
//! coverage completeness and the state of every declared cold guard. For a
//! Specification declaring **no** cold guard that rule reproduces #10's
//! coverage-driven table exactly:
//!
//! | coverage | outcome | authority | hot | cold | cold watermarks | basis |
//! |---|---|---|---|---|---|---|
//! | complete | Satisfy | Authoritative | SETTLED | NOT_REQUIRED | empty | `CompleteEvidenceBasis` |
//! | complete | Violate | Authoritative | SETTLED | NOT_REQUIRED | empty | `DecisiveViolationBasis` or `WatermarkAnchoredViolationBasis` |
//! | complete | Inconclusive | Authoritative | SETTLED | NOT_REQUIRED | empty | none |
//! | partial | Inconclusive | Provisional | SETTLED | PENDING | empty | none |
//!
//! and a Specification that declares one adds exactly two rows: `Provisional` /
//! `PENDING` / empty while a guard is unanswered, and `Authoritative` /
//! `SETTLED` / exactly-the-required-set once every guard settles.
//!
//! A flat `SETTLED`/`NOT_REQUIRED` would make the partial-coverage case
//! **unconstructible**: `Authoritative` fails `require_authoritative_coverage`,
//! which is applied unconditionally on outcome, and `Provisional` fails
//! `Verdict::try_from`'s demand for a pending cold path **or** a pending
//! activation barrier.
//!
//! **Admitted limitation, flagged not fixed (issue #18 inherits it):** the
//! partial-coverage row labels the Verdict `Provisional` / `cold PENDING` for a
//! Specification that requires no cold-path guard at all. ADR-0021 defines
//! provisional as "settled by the hot path but awaiting cold-path
//! confirmation", which is not what is happening. The model has no way to say
//! "Inconclusive, hot settled, no cold guard required, a required source is
//! missing". Issue #12 preserved this behaviour verbatim rather than silently
//! replacing it; naming it here keeps it an admitted limitation rather than a
//! silent misuse.
//!
//! # `anchored_event_ids` and `source_tier_gap`
//!
//! > `anchored_event_ids` for a source lists exactly the anchor-tier Events the
//! > Binding's conclusion cites. It is empty exactly when the conclusion rests
//! > on no anchor-tier Event. When it is empty for every anchor-tier required
//! > source, the Verdict carries a `SourceTierGap`.
//!
//! Most Inconclusive Verdicts therefore carry a tier gap. That is honest — the
//! conclusion rests on no anchor evidence — and it is a stated rule rather than
//! an emergent surprise. `unanchored_sources` is `required - valid_anchor`, so
//! when nothing anchors it names **every** required source, not only the
//! anchor-tier one. ADR-0033 extends valid anchor evidence to watermark-complete
//! silence: a `WatermarkAnchoredViolationBasis` means those silent sources count,
//! the gap is not mandatory, and the outcome is an authoritative Violate.
//!
//! # Who may build a Verdict (#304)
//!
//! Only [`crate::evaluator::Evaluator`]. The seven builders here — `build`,
//! `build_retired`, `build_taint`, `build_absence`, `build_aggregate`,
//! `build_existence` and `build_existence_retired` — are crate-private, and each
//! takes a `&Settlement` that cannot be obtained outside this crate, because
//! every producer of one is crate-private too (see the `# Sealed (#304)` section
//! on `crate::settlement::Settlement`, which also lists the guards). The
//! `Evaluator` derives the Settlement, the `EvidenceGate` and the family's
//! assessment from the single Specification it holds, so nothing outside this
//! crate can pair any of the three with a Specification it was not derived from.
//!
//! Before #304 the builders were public and checked none of those three against
//! the Specification beside them: #19's fix had moved the unchecked pairing one
//! call frame down from `settle` to here, not removed it.
//!
//! What #304 does not close: a Verdict binds its Specification by
//! `SpecificationIdentity` only, so a same-identity Specification with different
//! content — its cold guards stripped, for example — yields an indistinguishable
//! Verdict through the public `Evaluator` and through `decide`. That is #309.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::wire::model::v1 as raw;
use sentinel_model::{
    EventId, InconclusiveReason, RawEventRetentionDays, SourceCapability, SourceIdentity,
    SourceTier, Verdict, VerdictAuthority, VerdictError, VerdictOutcome,
};

use crate::absence::{
    AbsenceAssessment, AbsenceOutcome, AbsenceSpecification,
    INVARIANCE_PROOF_RULE_ID as ABSENCE_PROOF_RULE_ID,
    INVARIANCE_PROOF_RULE_VERSION as ABSENCE_PROOF_RULE_VERSION,
};
use crate::aggregate::{
    AggregateAssessment, AggregateOutcome, AggregateSpecification, CountEvidence,
};
use crate::cost::BindingFactCost;
use crate::evidence::{Coverage, EvidenceGate, EvidenceLedger};
use crate::existence::{
    ExistenceAssessment, ExistenceOutcome, ExistenceSpecification,
    INVARIANCE_PROOF_RULE_ID as EXISTENCE_PROOF_RULE_ID,
    INVARIANCE_PROOF_RULE_VERSION as EXISTENCE_PROOF_RULE_VERSION,
};
use crate::frontier::Frontier;
use crate::precedence::{
    BindingAssessment, BindingOutcome, RetiredAssessment, INVARIANCE_PROOF_RULE_ID,
    INVARIANCE_PROOF_RULE_VERSION,
};
use crate::settlement::Settlement;
use crate::slice::{Binding, Retirement};
use crate::specification::PrecedenceSpecification;
use crate::taint::{
    TaintAssessment, TaintOutcome, TaintSpecification,
    INVARIANCE_PROOF_RULE_ID as TAINT_PROOF_RULE_ID,
    INVARIANCE_PROOF_RULE_VERSION as TAINT_PROOF_RULE_VERSION,
};

/// Why an Inconclusive conclusion could not settle, in reader-facing form.
///
/// Gate-open absence of a matching requirement Event, with no remaining prefix,
/// settles as a watermark-anchored Violate (ADR-0033). This string remains the
/// envelope detail when the evidence gate is still open and the Binding is
/// Inconclusive prefix: an Event may still arrive.
const ANCHOR_SILENCE_DETAIL: &str =
    "no anchor-tier Event witnesses the absent requirement for triggers";

/// Envelope detail for a terminal cold-path [`crate::facts::GuardConclusion::Refused`].
///
/// The model reason this sits under is [`InconclusiveReason::EpistemicUncertainty`],
/// which names unstructured extraction. A refused guard is not extraction: it
/// is "the question cannot be answered because the window is gone or was
/// never retained." Same least-wrong shape as [`ANCHOR_SILENCE_DETAIL`] using
/// prefix uncertainty for a still-open gate. A dedicated model reason
/// (fact-gone / window-unholdable) is a tracked follow-up; this constant
/// stays pointed at epistemic until that lands.
pub const REFUSED_GUARD_DETAIL: &str =
    "a terminal cold-path refusal: the asked window is not holdable";

/// Extraction-fact identity stamped beside [`REFUSED_GUARD_DETAIL`].
pub const REFUSED_GUARD_FACT_ID: &str = "cold-path.refused-guard";

/// Cold `Fails` on CAT-5 until #20 owns cold-witness composition (AMEND 3).
pub const AGGREGATE_FAILS_DETAIL: &str =
    "cold count-within Fails: Authoritative Inconclusive until Amendment composition (#20)";

/// A CAT-5 Specification whose member count was never asked for at all.
pub const AGGREGATE_UNASKED_DETAIL: &str =
    "no cold-path count-within guard is bound, so no member count was requested";

/// Contributing-tier-only falsifier: presence is settled, the witness is not.
const CONTRIBUTING_TIER_DETAIL: &str =
    "a falsifying ancestor is present and witnessed only at contributing tier";

/// Human-readable account of how one Binding's conclusion was reached.
///
/// `model` carries assurance, not explanation, so this is the one read-side
/// structure the Core adds. It is never encoded on the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Explanation {
    /// The Binding value this conclusion is about.
    pub binding: String,
    /// The conclusion reached.
    pub outcome: VerdictOutcome,
    /// The named reason an Inconclusive conclusion could not settle.
    pub reason: Option<InconclusiveReason>,
    /// Trigger to the witnessing requirement Event that discharges it.
    pub discharged_by: BTreeMap<EventId, EventId>,
    /// `(witness, trigger)` pairs the evidence leaves unordered.
    pub concurrent_pairs: BTreeSet<(EventId, EventId)>,
    /// Triggers whose obligation no witness discharges.
    pub unmet_requirements: BTreeSet<EventId>,
    /// The retirement that pinned this conclusion, when one did.
    ///
    /// `None` for every Binding whose Scope is still open, which is every
    /// Binding in a deployment that never retires one. When it is `Some`, the
    /// Inconclusive outcome above is the retirement's and not the evidence's,
    /// and a reader that only had the outcome could not tell the two apart.
    pub retirement: Option<Retirement>,
}

/// One Verdict, its Binding, and the explanation that accompanies it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BindingVerdict {
    binding: Binding,
    verdict: Verdict,
    explanation: Explanation,
    cost: BindingFactCost,
}

impl BindingVerdict {
    /// Returns the Binding value.
    #[must_use]
    pub fn binding(&self) -> &str {
        self.binding.as_str()
    }

    /// Returns the Binding this Verdict is about.
    #[must_use]
    pub const fn key(&self) -> &Binding {
        &self.binding
    }

    /// Returns what this Verdict cost in cold-path round trips.
    ///
    /// Read-side only, and never on the wire: the envelope carries evidence
    /// conditions, not operational cost.
    #[must_use]
    pub const fn cost(&self) -> &BindingFactCost {
        &self.cost
    }

    /// Returns the validated Verdict.
    #[must_use]
    pub const fn verdict(&self) -> &Verdict {
        &self.verdict
    }

    /// Returns the explanation accompanying the Verdict.
    #[must_use]
    pub const fn explanation(&self) -> &Explanation {
        &self.explanation
    }
}

/// What envelope assembly reads off a CAT-1 conclusion.
///
/// One parameter where a `&BindingAssessment` used to sit, so that a retired
/// Binding — which has **no outcome** to read — can reach the same assembler
/// without one being invented for it. The three things the assembler asks for
/// are the concurrency disclosure, the prefix disclosure, and the witness set a
/// decisive violation names; a retired conclusion answers `None`, its own
/// sentence, and nothing.
enum PrecedenceCitations<'a> {
    /// A Binding whose Scope is still open.
    Open(&'a BindingAssessment),
    /// A Binding whose Scope the periphery retired.
    Retired(&'a RetiredAssessment, &'a Retirement),
}

impl PrecedenceCitations<'_> {
    /// Returns the observational-uncertainty sentence, when there is one.
    ///
    /// Always `None` for a retired Binding. Concurrency is cited only when the
    /// conclusion turns on it, and a retired conclusion turns on the horizon.
    fn concurrency_detail(&self) -> Option<String> {
        match self {
            Self::Open(assessment) if !assessment.concurrent_pairs().is_empty() => {
                Some(assessment.concurrency_detail())
            }
            Self::Open(_) | Self::Retired(_, _) => None,
        }
    }

    /// Returns the prefix-uncertainty sentences, in envelope order.
    ///
    /// The retired branch **owns the first entry**, and the anchor-silence
    /// sentence is not reachable from it. That sentence claims no anchor-tier
    /// Event witnesses an absent requirement, which is a statement about the
    /// evidence; a retired Binding is Inconclusive because the periphery
    /// stopped waiting, and saying otherwise would attribute the conclusion to
    /// the wrong cause in the externally visible envelope.
    ///
    /// Both exclusion counts are disclosed when non-zero. Evidence the horizon
    /// removed from the Slice is evidence the Core holds and the Verdict does
    /// not rest on, and a Verdict that dropped it in silence would be the
    /// failure mode retirement is designed against.
    ///
    /// The gate's own sentence is still appended for a closed gate, because it
    /// is still true and is still worth recording.
    fn prefix_details(&self, gate: &EvidenceGate) -> Vec<String> {
        match self {
            Self::Retired(assessment, retirement) => {
                let mut details = vec![retirement.detail()];
                if assessment.observations_above_horizon() > 0 {
                    details.push(format!(
                        "{} Slice member(s) were observed above the retirement horizon \
                         and are out of scope",
                        assessment.observations_above_horizon()
                    ));
                }
                if assessment.observations_on_another_clock() > 0 {
                    details.push(format!(
                        "{} Slice member(s) were observed on another Sentinel clock, \
                         which the retirement horizon cannot order",
                        assessment.observations_on_another_clock()
                    ));
                }
                if !gate.is_open() {
                    details.push(gate.detail());
                }
                details
            }
            Self::Open(assessment)
                if assessment.outcome() == BindingOutcome::InconclusivePrefix =>
            {
                vec![if gate.is_open() {
                    format!(
                        "{ANCHOR_SILENCE_DETAIL}: {}",
                        assessment
                            .unmet_requirements()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    gate.detail()
                }]
            }
            Self::Open(_) => Vec::new(),
        }
    }

    /// Returns the Events a decisive violation witness names, when there are any.
    ///
    /// `None` for a retired Binding, which cannot reach a Violate at all — so
    /// the branch that reads this is unreachable for one, and there is nothing
    /// for a settlement basis to be assembled from.
    fn witness_events(&self) -> Option<&BTreeSet<EventId>> {
        match self {
            Self::Open(assessment) => Some(assessment.witness_events()),
            Self::Retired(_, _) => None,
        }
    }
}

/// Builds and validates the Verdict for one Binding.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant. That is a Core bug rather than an evidence condition: the model
/// is the authority on what a trustworthy Verdict is, and this crate is not
/// permitted to work around it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    specification: &PrecedenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &BindingAssessment,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    // [`Coverage::is_complete`] is exactly the negation of the evidence gate's
    // G1 disjunct — which is why gate-open implies complete coverage, and
    // therefore why a first emission can only be `Provisional` because of an
    // unsettled guard and never because of a missing source.
    // `crate::evidence::EvidenceLedger::coverage` is the one constructor;
    // `crate::evaluator` reads the same population to feed `crate::settlement`,
    // and the evidence gate reads it to decide G1.
    let coverage = ledger.coverage(specification.required_source_identities());
    let (hot_outcome, hot_reason) = match assessment.outcome() {
        BindingOutcome::Satisfy => (VerdictOutcome::Satisfy, None),
        BindingOutcome::Violate => (VerdictOutcome::Violate, None),
        BindingOutcome::InconclusiveObservational => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::ObservationalUncertainty),
        ),
        BindingOutcome::InconclusivePrefix => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty),
        ),
    };
    let (outcome, reason) = epistemic_from_refused(hot_outcome, hot_reason, any_refused);

    let events_by_source = anchored_by_source(frontier, assessment.anchored_events());
    let assurance = assurance_envelope(
        specification,
        ledger,
        frontier,
        gate,
        &PrecedenceCitations::Open(assessment),
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        // These three were hand-rolled wildcard matches over
        // `#[non_exhaustive]` domain enums until #133 published `model`'s
        // `to_raw`. That is #91's defect class, and this is the crate that
        // *produces* every Verdict `store` writes, so it was the first of the
        // two encoders in series and the one that decided the value the second
        // ever saw. Executed, in the tree where the four proto variants existed
        // and `store/src/codec.rs` failed to compile four times:
        // `cargo build -p sentinel-core` finished clean and emitted
        // `VERDICT_AUTHORITY_PROVISIONAL` for the new authority. `authority`
        // was the demonstrably harmful one -- ADR-0021's SA3 check refuses
        // walking an `Authoritative` Verdict back, and a laundered
        // `provisional` is exactly the transition it exists to refuse, arriving
        // in a form its `!=` cannot see. `inconclusive_reason` was the second:
        // it attributed an Inconclusive to a *named wrong cause* in the
        // externally visible Assurance Envelope. `outcome` understates rather
        // than overstates, which is the harmless direction and still a claim
        // about variants this build cannot see.
        //
        // Where the compile error lands now is worth saying exactly, because
        // the obvious reading is wrong: it does **not** land here. A proto
        // variant breaks `model`'s `from_raw` and `store/src/codec.rs`'s
        // `declared_*_text`; a domain variant breaks `to_raw` itself, in
        // `model/src/wire_helpers.rs`. Executed both ways -- with the four
        // proto variants added, `cargo build -p sentinel-core` finishes clean,
        // and that is the point rather than a gap. The decision about a new
        // variant belongs in the one crate that can see every variant, and
        // this file's job is to stop having an opinion about it.
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: assessment.discharged_by().clone(),
            concurrent_pairs: assessment.concurrent_pairs().clone(),
            unmet_requirements: assessment.unmet_requirements().clone(),
            retirement: None,
        },
    })
}

/// Builds and validates the Verdict for one **retired** CAT-1 Binding.
///
/// The outcome is `Inconclusive`, unconditionally and by construction: there is
/// no outcome on a [`RetiredAssessment`] to map, so the `match` that produces
/// `Satisfy` and `Violate` in [`build`] has nothing here to run over. That is
/// ADR-0021's O-03 pin as a property of the types rather than as a demotion
/// applied after the fact — never `Violate`, because a deadline bounds patience
/// and not existence (ADR-0018: a Watermark is an operational finality boundary,
/// not proof that no older Event exists, and a retirement signal is strictly
/// weaker than a Watermark); never `Satisfy`, because ADR-0025 licenses that
/// only when the Scope **closes**, and a retired Scope did not close, it expired.
///
/// The reason is `PrefixUncertainty` — post-horizon evidence would have settled
/// it, which is literally true — **unless** a terminal cold-path refusal rewrites
/// it to `EpistemicUncertainty` through the same
/// [`epistemic_from_refused`] every other path uses. That rewrite is licensed
/// and is not an outcome change.
///
/// An ADR-0033 watermark-anchored Violate is unreachable twice over: the outcome
/// cannot be `Violate`, and [`PrecedenceCitations::witness_events`] answers
/// `None`, so the basis has nothing to be assembled from. Fabricating
/// `silence_through_watermarks` from a retirement would claim a source's
/// completeness coordinate witnessed a silence that no source witnessed.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant, on [`build`]'s terms.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_retired(
    specification: &PrecedenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &RetiredAssessment,
    retirement: &Retirement,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    let coverage = ledger.coverage(specification.required_source_identities());
    let (outcome, reason) = epistemic_from_refused(
        VerdictOutcome::Inconclusive,
        Some(InconclusiveReason::PrefixUncertainty),
        any_refused,
    );

    // A retired conclusion rests on no anchor-tier Event, so the map is empty
    // and every anchor-tier required source records an empty anchored set —
    // which is what makes the Verdict carry a `SourceTierGap`. That is honest
    // rather than a defect: the conclusion really does rest on no anchor.
    let events_by_source = anchored_by_source(frontier, &BTreeSet::new());
    let assurance = assurance_envelope(
        specification,
        ledger,
        frontier,
        gate,
        &PrecedenceCitations::Retired(assessment, retirement),
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: assessment.discharged_by().clone(),
            concurrent_pairs: BTreeSet::new(),
            unmet_requirements: assessment.unmet_requirements().clone(),
            retirement: Some(retirement.clone()),
        },
    })
}

/// Builds and validates the Verdict for one CAT-4 Binding.
///
/// Distinct from [`build`]: sharing the assessment type would invite a polarity
/// flag, which `crate::taint` exists to refuse. Envelope fields that only need
/// identity, coverage, gate, settlement and cited Event ids go through the
/// same private assembler.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_taint(
    specification: &TaintSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &TaintAssessment,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    let coverage = ledger.coverage(specification.required_source_identities());
    let (hot_outcome, hot_reason) = match assessment.outcome() {
        TaintOutcome::Satisfy => (VerdictOutcome::Satisfy, None),
        TaintOutcome::Violate => (VerdictOutcome::Violate, None),
        TaintOutcome::InconclusiveObservational => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::ObservationalUncertainty),
        ),
        TaintOutcome::InconclusivePrefix => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty),
        ),
    };
    let (outcome, reason) = epistemic_from_refused(hot_outcome, hot_reason, any_refused);

    let events_by_source = anchored_by_source(frontier, assessment.anchored_events());
    let assurance = assurance_envelope_taint(
        specification,
        ledger,
        frontier,
        gate,
        assessment,
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: BTreeMap::new(),
            concurrent_pairs: assessment.concurrent_pairs().clone(),
            unmet_requirements: BTreeSet::new(),
            // CAT-4's Pattern declares no retirement policy, so clause (d0)
            // refuses every retirement signal on a CAT-4 evaluator and no CAT-4
            // Binding can be retired. The family that lowers a retiring policy
            // owns building the retired path here.
            retirement: None,
        },
    })
}

/// Builds and validates the Verdict for one CAT-2 Binding.
///
/// Distinct from [`build`] and [`build_taint`]: sharing assessment types would
/// invite a polarity or interval flag the decision tables refuse. Envelope
/// fields that only need identity, coverage, gate, settlement and cited Event
/// ids go through the same private assembler.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_absence(
    specification: &AbsenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &AbsenceAssessment,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    let coverage = ledger.coverage(specification.required_source_identities());
    let (hot_outcome, hot_reason) = match assessment.outcome() {
        AbsenceOutcome::Satisfy => (VerdictOutcome::Satisfy, None),
        AbsenceOutcome::Violate => (VerdictOutcome::Violate, None),
        AbsenceOutcome::InconclusiveObservational => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::ObservationalUncertainty),
        ),
        AbsenceOutcome::InconclusivePrefix => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty),
        ),
    };
    let (outcome, reason) = epistemic_from_refused(hot_outcome, hot_reason, any_refused);

    let events_by_source = anchored_by_source(frontier, assessment.anchored_events());
    let assurance = assurance_envelope_absence(
        specification,
        ledger,
        frontier,
        gate,
        assessment,
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: BTreeMap::new(),
            concurrent_pairs: assessment.concurrent_pairs().clone(),
            unmet_requirements: BTreeSet::new(),
            // CAT-2 closes at `until` and declares NotRequired; no retired path.
            retirement: None,
        },
    })
}

/// Builds and validates the Verdict for one CAT-5 Binding.
///
/// Distinct from [`build`], [`build_taint`] and [`build_absence`]: Aggregate
/// never mints Violate from cold `Fails` (AMEND 3), and never places cold
/// witnesses in `anchored_event_ids`.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_aggregate(
    specification: &AggregateSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &AggregateAssessment,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    let coverage = ledger.coverage(specification.required_source_identities());
    let (hot_outcome, hot_reason) = match assessment.outcome() {
        AggregateOutcome::Satisfy => (VerdictOutcome::Satisfy, None),
        AggregateOutcome::OverThreshold | AggregateOutcome::InconclusiveEpistemic => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::EpistemicUncertainty),
        ),
        AggregateOutcome::InconclusivePrefix => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty),
        ),
    };
    let (outcome, reason) = epistemic_from_refused(hot_outcome, hot_reason, any_refused);

    let events_by_source = anchored_by_source(frontier, assessment.anchored_events());
    let assurance = assurance_envelope_aggregate(
        specification,
        ledger,
        frontier,
        gate,
        assessment,
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: BTreeMap::new(),
            concurrent_pairs: BTreeSet::new(),
            unmet_requirements: BTreeSet::new(),
            // CAT-5 lookback is not a never-closing Scope; declares NotRequired.
            retirement: None,
        },
    })
}

/// Rewrites a hot-path pair when any settled query for the Binding is `Refused`.
///
/// Guard-free paths pass `false` and stay byte-identical. A `Fails` answer
/// does **not** enter this path for CAT-1: limitation 8 / #20 own that flip.
/// CAT-5 maps Fails in [`crate::aggregate::decide`] instead, so P5's
/// Fails→hot Satisfy path is untouched.
fn epistemic_from_refused(
    hot_outcome: VerdictOutcome,
    hot_reason: Option<InconclusiveReason>,
    any_refused: bool,
) -> (VerdictOutcome, Option<InconclusiveReason>) {
    if any_refused {
        (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::EpistemicUncertainty),
        )
    } else {
        (hot_outcome, hot_reason)
    }
}

fn refused_extraction_facts(any_refused: bool) -> Vec<raw::ExtractionFact> {
    if any_refused {
        vec![raw::ExtractionFact {
            fact_id: REFUSED_GUARD_FACT_ID.to_owned(),
            available: false,
            detail: REFUSED_GUARD_DETAIL.to_owned(),
        }]
    } else {
        Vec::new()
    }
}

/// Groups the cited anchor Events by source, reading the Frontier's deliveries.
///
/// Reading `Frontier::delivered()` rather than an Adapter batch is safe for
/// byte identity: `canonicalize_assurance` sorts every source-keyed vector and
/// `canonicalize_settlement_basis` sorts `witness_events` by `event_id`, so
/// emission-order dependence is erased before `Verdict::try_from` returns.
fn anchored_by_source(
    frontier: &Frontier,
    anchored: &BTreeSet<EventId>,
) -> BTreeMap<SourceIdentity, BTreeSet<EventId>> {
    let mut by_source: BTreeMap<SourceIdentity, BTreeSet<EventId>> = BTreeMap::new();
    for event in &frontier.delivered() {
        let id = event.id();
        if anchored.contains(&id) {
            by_source.entry(event.source()).or_default().insert(id);
        }
    }
    by_source
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
// Every envelope field stays in one auditable place, next to the rule that fills it.
fn assurance_envelope(
    specification: &PrecedenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    citations: &PrecedenceCitations<'_>,
    settlement: &Settlement,
    coverage: &Coverage<'_>,
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
    outcome: VerdictOutcome,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> raw::AssuranceEnvelope {
    let mut filter_epochs = Vec::new();
    let mut watermarks = Vec::new();
    let mut gaps = Vec::new();
    let mut source_health = Vec::new();
    let mut capabilities = Vec::new();
    let mut tiers = Vec::new();

    // A covered source arrives with its record attached, so there is no lookup
    // here to fail — which is the whole of #158's fix. The predecessor read the
    // record back out of the ledger and asserted the result with `.expect`,
    // over a set that named every *delivering* required source whether or not
    // it had declared any evidence.
    for (source, record) in coverage.covered() {
        let healthy = record.is_contiguous() && !ledger.contradicts_delivery(source);
        let wire_source = source.to_wire();

        let completeness = record.completeness();

        // The epoch the source's own completeness signal acknowledged, not the
        // constant `0` this wrote until #19. ADR-0017 makes `filter_epochs` the
        // per-required-source record of which Event Filter the evidence was
        // collected under. The Specification-level target sits on the envelope
        // (`target_filter_epoch`), not cloned onto every row.
        filter_epochs.push(raw::FilterEpochBySource {
            source: Some(wire_source.clone()),
            filter_epoch: Some(completeness.filter_epoch().value()),
        });

        watermarks.push(raw::Watermark {
            source: Some(wire_source.clone()),
            sequence_epoch: completeness.epoch().value(),
            complete_through: completeness
                .complete_through()
                .map_or(0, sentinel_model::SequenceNumber::value),
            observed_at: Some(completeness.observed_at().to_wire()),
        });

        for gap in record.gaps() {
            gaps.push(raw::SourceGap {
                source: Some(wire_source.clone()),
                sequence_epoch: gap.sequence_epoch,
                first_missing: gap.first_missing,
                last_missing: gap.last_missing,
                effect: "delivery sequence is discontinuous, so the observed prefix is open"
                    .to_owned(),
            });
        }

        source_health.push(raw::SourceHealth {
            source: Some(wire_source.clone()),
            healthy,
            detail: if healthy {
                String::new()
            } else if record.is_contiguous() {
                "source's completeness signal contradicts what it delivered".to_owned()
            } else {
                "source delivered a discontinuous sequence".to_owned()
            },
        });

        capabilities.push(raw::SourceCapabilityRecord {
            source: Some(wire_source.clone()),
            capabilities: record
                .capabilities()
                .iter()
                .map(SourceCapability::to_raw)
                .collect(),
        });

        // ADR-0022: tier is a property of the deployed source, so it is read
        // from the evidence and never from the Specification, which declares
        // only the *minimum* tier a trustworthy conclusion needs. Read off the
        // covered record rather than through `EvidenceLedger::tier_of`, whose
        // `unwrap_or(Contributing)` would silently downgrade a source with no
        // record instead of declining to cover it.
        let tier = record.tier();
        tiers.push(raw::SourceTierRecord {
            source: Some(wire_source),
            tier: tier.to_raw(),
            anchored_event_ids: if tier == SourceTier::Anchor {
                anchored
                    .get(source)
                    .map(|events| events.iter().map(ToString::to_string).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        });
    }

    let mut uncertainty = Vec::new();
    if let Some(detail) = citations.concurrency_detail() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::ObservationalUncertainty as i32,
            detail,
        });
    }
    // Every prefix-shaped disclosure folds into **one** entry, because
    // `sentinel_model`'s `validate_uncertainty` admits at most one entry per
    // reason and rejects a second as `DuplicateValue("assurance.uncertainty")`.
    // Pushing the barrier separately would therefore have made an Inconclusive
    // prefix Verdict behind a pending barrier unconstructible — and a second
    // entry for the O-03 sentence would surface a periphery input as a Core
    // bug, which is #108's failure class.
    //
    // Order is fixed and the join is unconditional-shaped: with no barrier
    // declared and nothing retired the vector holds exactly the string this
    // pushed before #19, so every existing pinned sentence is byte-identical.
    let mut prefix_details = citations.prefix_details(gate);
    // The barrier disclosure is a *separate* sentence rather than an append to
    // `gate.detail()`: the evidence gate is deliberately not closed for a
    // pre-activation source, so its own audit strings are untouched. It is
    // `PREFIX_UNCERTAINTY` for the same least-wrong reason `ANCHOR_SILENCE_DETAIL`
    // is -- post-activation evidence would settle it, which is literally true --
    // and the model has no barrier reason to name.
    if let Some(pending) = settlement.barrier().pending() {
        prefix_details.push(pending.detail());
    }
    if !prefix_details.is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::PrefixUncertainty as i32,
            detail: prefix_details.join("; "),
        });
    }
    // The model reason names unstructured extraction. A Refused guard is not
    // extraction; it is "the question cannot be answered because the window
    // is gone or was never retained." Same least-wrong shape as
    // `ANCHOR_SILENCE_DETAIL` using prefix uncertainty. Tracked `model`
    // follow-up: a dedicated fact-gone reason. Until that lands this push
    // stays on `EpistemicUncertainty` and
    // `a_terminal_refused_settles_authoritative_inconclusive_epistemic`
    // is the test that fails if it is skipped.
    if any_refused {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
            detail: REFUSED_GUARD_DETAIL.to_owned(),
        });
    }

    let unresolved_causal_edges = frontier
        .unresolved_causal_edges()
        .into_iter()
        .map(|(event, predecessor)| raw::UnresolvedCausalEdge {
            event_id: event.to_string(),
            missing_predecessor_id: predecessor.to_string(),
        })
        .collect::<Vec<_>>();

    let silent_anchors = silent_complete_anchors(coverage, &source_health, anchored);
    let watermark_anchored_violate = settlement.authority() == VerdictAuthority::Authoritative
        && outcome == VerdictOutcome::Violate
        && anchored.values().all(BTreeSet::is_empty)
        && !silent_anchors.is_empty();
    let no_extra_anchors = BTreeSet::new();
    let extra_valid = if watermark_anchored_violate {
        &silent_anchors
    } else {
        &no_extra_anchors
    };
    let source_tier_gap = source_tier_gap(coverage, anchored, &tiers, extra_valid);

    // A settlement basis is only constructible for an **Authoritative**
    // Verdict: the model accepts `(Provisional, any outcome, no basis)` and
    // rejects `(Provisional, _, Some(_))` outright. For a guard-free
    // Specification `authoritative` is exactly `complete`, so this reproduces
    // #10's table unchanged; with a guard outstanding it is what keeps a
    // Provisional Satisfy constructible at complete coverage.
    // Unset-ack rows are a PENDING write only. CLEARED forbids an unset
    // acknowledgement; writing one when a barrier is merely *declared* would
    // make Core's own CLEARED output fail try_from.
    if settlement.barrier().is_pending() {
        let covered: BTreeSet<_> = coverage.covered().keys().collect();
        for source in coverage.required() {
            if !covered.contains(source) {
                filter_epochs.push(raw::FilterEpochBySource {
                    source: Some(source.to_wire()),
                    filter_epoch: None,
                });
            }
        }
    }

    let (activation_barrier, target_filter_epoch) = match specification.activation_barrier() {
        None => (raw::ActivationBarrierStatus::NotDeclared as i32, None),
        Some(barrier) if settlement.barrier().is_pending() => (
            raw::ActivationBarrierStatus::Pending as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
        Some(barrier) => (
            raw::ActivationBarrierStatus::Cleared as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
    };

    let authoritative = settlement.authority() == VerdictAuthority::Authoritative;
    // A retired conclusion names no decisive witness, and cannot reach the arm
    // that would read one. Beside `no_extra_anchors` above, and for the same
    // reason: the borrow has to outlive the `match`.
    let no_witness_events = BTreeSet::new();
    let settlement_basis = match (authoritative, outcome, watermark_anchored_violate) {
        (true, VerdictOutcome::Satisfy, _) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::CompleteEvidence(
                raw::CompleteEvidenceBasis {},
            )),
        }),
        (true, VerdictOutcome::Violate, true) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::WatermarkAnchoredViolation(
                watermark_anchored_violation_basis(
                    &silent_anchors,
                    &watermarks,
                    INVARIANCE_PROOF_RULE_ID,
                    INVARIANCE_PROOF_RULE_VERSION,
                    &gaps,
                    &uncertainty,
                ),
            )),
        }),
        (true, VerdictOutcome::Violate, false) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::DecisiveViolation(
                decisive_violation_basis(
                    frontier,
                    citations.witness_events().unwrap_or(&no_witness_events),
                    INVARIANCE_PROOF_RULE_ID,
                    INVARIANCE_PROOF_RULE_VERSION,
                    &gaps,
                    &uncertainty,
                ),
            )),
        }),
        _ => None,
    };

    raw::AssuranceEnvelope {
        specification_id: specification.identity().id().to_owned(),
        specification_version: specification.identity().version().to_owned(),
        schema_versions: vec![specification.schema_version().to_string()],
        filter_epochs,
        watermarks: watermarks.clone(),
        required_sources: coverage
            .required()
            .iter()
            .map(SourceIdentity::to_wire)
            .collect(),
        // Narrower than the field's name suggests, and deliberately so: this
        // carries the *covered* sources — the conjunction — not every source
        // that delivered an Event. A consumer reading it as "which sources sent
        // Events" under-reports a source that delivered plenty and declared
        // nothing; that source is named by the gate's G1b uncertainty detail
        // instead, and by `CoreError::NoEvidenceCoverage::undeclared`.
        observed_sources: coverage
            .covered()
            .keys()
            .map(SourceIdentity::to_wire)
            .collect(),
        gaps,
        source_health,
        capabilities,
        source_tiers_and_anchors: tiers,
        unresolved_causal_edges,
        // Populated from a terminal Refused (`epistemic_from_refused`). The
        // model reason names unstructured extraction; a Refused guard is
        // "the asked window is not holdable." Guard-free Verdicts still
        // write an empty vec, so they stay byte-identical.
        extraction_facts: refused_extraction_facts(any_refused),
        uncertainty,
        // A closed recorded artifact: nothing arrives late, ever.
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        // The fourth of the wildcards #133 swept; see `binding_verdict`. This
        // one understated settlement, which is the safe direction -- and still
        // a runtime answer to a question `model` can answer at compile time.
        // `hot_path` above is a literal rather than a mapping: this Core
        // evaluates a closed recorded artifact, so its hot path is settled by
        // construction and there is no domain value to encode.
        cold_path: settlement.cold_path().to_raw(),
        hot_settlement_watermarks: watermarks,
        cold_settlement_watermarks: cold_settlement_watermarks(ledger, settlement),
        source_tier_gap,
        settlement_basis,
        activation_barrier,
        target_filter_epoch,
        // Caller-supplied RawEventRetentionDays (#245). Defaults to platform
        // default on Evaluator construction; api/harness paths that know a
        // Tenant horizon override via `with_raw_event_retention`.
        raw_event_retention_days: Some(raw_event_retention.value()),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        // Forecast-at-settle inventory (#244). Core does not evaluate Tenant
        // redaction policy and must not invent claimed paths.
        redacted_attributes: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn assurance_envelope_taint(
    specification: &TaintSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &TaintAssessment,
    settlement: &Settlement,
    coverage: &Coverage<'_>,
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
    outcome: VerdictOutcome,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> raw::AssuranceEnvelope {
    let mut filter_epochs = Vec::new();
    let mut watermarks = Vec::new();
    let mut gaps = Vec::new();
    let mut source_health = Vec::new();
    let mut capabilities = Vec::new();
    let mut tiers = Vec::new();

    for (source, record) in coverage.covered() {
        let healthy = record.is_contiguous() && !ledger.contradicts_delivery(source);
        let wire_source = source.to_wire();
        let completeness = record.completeness();

        filter_epochs.push(raw::FilterEpochBySource {
            source: Some(wire_source.clone()),
            filter_epoch: Some(completeness.filter_epoch().value()),
        });

        watermarks.push(raw::Watermark {
            source: Some(wire_source.clone()),
            sequence_epoch: completeness.epoch().value(),
            complete_through: completeness
                .complete_through()
                .map_or(0, sentinel_model::SequenceNumber::value),
            observed_at: Some(completeness.observed_at().to_wire()),
        });

        for gap in record.gaps() {
            gaps.push(raw::SourceGap {
                source: Some(wire_source.clone()),
                sequence_epoch: gap.sequence_epoch,
                first_missing: gap.first_missing,
                last_missing: gap.last_missing,
                effect: "delivery sequence is discontinuous, so the observed prefix is open"
                    .to_owned(),
            });
        }

        source_health.push(raw::SourceHealth {
            source: Some(wire_source.clone()),
            healthy,
            detail: if healthy {
                String::new()
            } else if record.is_contiguous() {
                "source's completeness signal contradicts what it delivered".to_owned()
            } else {
                "source delivered a discontinuous sequence".to_owned()
            },
        });

        capabilities.push(raw::SourceCapabilityRecord {
            source: Some(wire_source.clone()),
            capabilities: record
                .capabilities()
                .iter()
                .map(SourceCapability::to_raw)
                .collect(),
        });

        let tier = record.tier();
        tiers.push(raw::SourceTierRecord {
            source: Some(wire_source),
            tier: tier.to_raw(),
            anchored_event_ids: if tier == SourceTier::Anchor {
                anchored
                    .get(source)
                    .map(|events| events.iter().map(ToString::to_string).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        });
    }

    let mut uncertainty = Vec::new();
    if !assessment.concurrent_pairs().is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::ObservationalUncertainty as i32,
            detail: assessment.concurrency_detail(),
        });
    }
    let mut prefix_details = Vec::new();
    if assessment.outcome() == TaintOutcome::InconclusivePrefix {
        prefix_details.push(if gate.is_open() {
            CONTRIBUTING_TIER_DETAIL.to_owned()
        } else {
            gate.detail()
        });
    }
    if let Some(pending) = settlement.barrier().pending() {
        prefix_details.push(pending.detail());
    }
    if !prefix_details.is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::PrefixUncertainty as i32,
            detail: prefix_details.join("; "),
        });
    }
    // Twin of the precedence push: CAT-4 declares no cold guards today, so
    // `any_refused` is a no-op unless a later ticket adds one. The comment
    // that epistemic cannot arise is deleted; the field is populated from
    // `epistemic_from_refused` when a terminal Refused settles.
    if any_refused {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
            detail: REFUSED_GUARD_DETAIL.to_owned(),
        });
    }

    let unresolved_causal_edges = frontier
        .unresolved_causal_edges()
        .into_iter()
        .map(|(event, predecessor)| raw::UnresolvedCausalEdge {
            event_id: event.to_string(),
            missing_predecessor_id: predecessor.to_string(),
        })
        .collect::<Vec<_>>();

    let source_tier_gap = source_tier_gap(coverage, anchored, &tiers, &BTreeSet::new());

    // Unset-ack rows are a PENDING write only. CLEARED forbids an unset
    // acknowledgement; writing one when a barrier is merely *declared* would
    // make Core's own CLEARED output fail try_from.
    if settlement.barrier().is_pending() {
        let covered: BTreeSet<_> = coverage.covered().keys().collect();
        for source in coverage.required() {
            if !covered.contains(source) {
                filter_epochs.push(raw::FilterEpochBySource {
                    source: Some(source.to_wire()),
                    filter_epoch: None,
                });
            }
        }
    }

    let (activation_barrier, target_filter_epoch) = match specification.activation_barrier() {
        None => (raw::ActivationBarrierStatus::NotDeclared as i32, None),
        Some(barrier) if settlement.barrier().is_pending() => (
            raw::ActivationBarrierStatus::Pending as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
        Some(barrier) => (
            raw::ActivationBarrierStatus::Cleared as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
    };

    let authoritative = settlement.authority() == VerdictAuthority::Authoritative;
    let settlement_basis = match (authoritative, outcome) {
        (true, VerdictOutcome::Satisfy) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::CompleteEvidence(
                raw::CompleteEvidenceBasis {},
            )),
        }),
        (true, VerdictOutcome::Violate) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::DecisiveViolation(
                decisive_violation_basis(
                    frontier,
                    assessment.witness_events(),
                    TAINT_PROOF_RULE_ID,
                    TAINT_PROOF_RULE_VERSION,
                    &gaps,
                    &uncertainty,
                ),
            )),
        }),
        _ => None,
    };

    raw::AssuranceEnvelope {
        specification_id: specification.identity().id().to_owned(),
        specification_version: specification.identity().version().to_owned(),
        schema_versions: vec![specification.schema_version().to_string()],
        filter_epochs,
        watermarks: watermarks.clone(),
        required_sources: coverage
            .required()
            .iter()
            .map(SourceIdentity::to_wire)
            .collect(),
        observed_sources: coverage
            .covered()
            .keys()
            .map(SourceIdentity::to_wire)
            .collect(),
        gaps,
        source_health,
        capabilities,
        source_tiers_and_anchors: tiers,
        unresolved_causal_edges,
        extraction_facts: refused_extraction_facts(any_refused),
        uncertainty,
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        cold_path: settlement.cold_path().to_raw(),
        hot_settlement_watermarks: watermarks,
        cold_settlement_watermarks: cold_settlement_watermarks(ledger, settlement),
        source_tier_gap,
        settlement_basis,
        activation_barrier,
        target_filter_epoch,
        // Caller-supplied RawEventRetentionDays (#245). Defaults to platform
        // default on Evaluator construction; api/harness paths that know a
        // Tenant horizon override via `with_raw_event_retention`.
        raw_event_retention_days: Some(raw_event_retention.value()),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        // Forecast-at-settle inventory (#244). Core does not evaluate Tenant
        // redaction policy and must not invent claimed paths.
        redacted_attributes: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn assurance_envelope_absence(
    specification: &AbsenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &AbsenceAssessment,
    settlement: &Settlement,
    coverage: &Coverage<'_>,
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
    outcome: VerdictOutcome,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> raw::AssuranceEnvelope {
    let mut filter_epochs = Vec::new();
    let mut watermarks = Vec::new();
    let mut gaps = Vec::new();
    let mut source_health = Vec::new();
    let mut capabilities = Vec::new();
    let mut tiers = Vec::new();

    for (source, record) in coverage.covered() {
        let healthy = record.is_contiguous() && !ledger.contradicts_delivery(source);
        let wire_source = source.to_wire();
        let completeness = record.completeness();

        filter_epochs.push(raw::FilterEpochBySource {
            source: Some(wire_source.clone()),
            filter_epoch: Some(completeness.filter_epoch().value()),
        });

        watermarks.push(raw::Watermark {
            source: Some(wire_source.clone()),
            sequence_epoch: completeness.epoch().value(),
            complete_through: completeness
                .complete_through()
                .map_or(0, sentinel_model::SequenceNumber::value),
            observed_at: Some(completeness.observed_at().to_wire()),
        });

        for gap in record.gaps() {
            gaps.push(raw::SourceGap {
                source: Some(wire_source.clone()),
                sequence_epoch: gap.sequence_epoch,
                first_missing: gap.first_missing,
                last_missing: gap.last_missing,
                effect: "delivery sequence is discontinuous, so the observed prefix is open"
                    .to_owned(),
            });
        }

        source_health.push(raw::SourceHealth {
            source: Some(wire_source.clone()),
            healthy,
            detail: if healthy {
                String::new()
            } else if record.is_contiguous() {
                "source's completeness signal contradicts what it delivered".to_owned()
            } else {
                "source delivered a discontinuous sequence".to_owned()
            },
        });

        capabilities.push(raw::SourceCapabilityRecord {
            source: Some(wire_source.clone()),
            capabilities: record
                .capabilities()
                .iter()
                .map(SourceCapability::to_raw)
                .collect(),
        });

        let tier = record.tier();
        tiers.push(raw::SourceTierRecord {
            source: Some(wire_source),
            tier: tier.to_raw(),
            anchored_event_ids: if tier == SourceTier::Anchor {
                anchored
                    .get(source)
                    .map(|events| events.iter().map(ToString::to_string).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        });
    }

    let mut uncertainty = Vec::new();
    if !assessment.concurrent_pairs().is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::ObservationalUncertainty as i32,
            detail: assessment.concurrency_detail(),
        });
    }
    let mut prefix_details = Vec::new();
    if assessment.outcome() == AbsenceOutcome::InconclusivePrefix {
        prefix_details.push(if gate.is_open() {
            CONTRIBUTING_TIER_DETAIL.to_owned()
        } else {
            gate.detail()
        });
    }
    if let Some(pending) = settlement.barrier().pending() {
        prefix_details.push(pending.detail());
    }
    if !prefix_details.is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::PrefixUncertainty as i32,
            detail: prefix_details.join("; "),
        });
    }
    if any_refused {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
            detail: REFUSED_GUARD_DETAIL.to_owned(),
        });
    }

    let unresolved_causal_edges = frontier
        .unresolved_causal_edges()
        .into_iter()
        .map(|(event, predecessor)| raw::UnresolvedCausalEdge {
            event_id: event.to_string(),
            missing_predecessor_id: predecessor.to_string(),
        })
        .collect::<Vec<_>>();

    let source_tier_gap = source_tier_gap(coverage, anchored, &tiers, &BTreeSet::new());

    if settlement.barrier().is_pending() {
        let covered: BTreeSet<_> = coverage.covered().keys().collect();
        for source in coverage.required() {
            if !covered.contains(source) {
                filter_epochs.push(raw::FilterEpochBySource {
                    source: Some(source.to_wire()),
                    filter_epoch: None,
                });
            }
        }
    }

    let (activation_barrier, target_filter_epoch) = match specification.activation_barrier() {
        None => (raw::ActivationBarrierStatus::NotDeclared as i32, None),
        Some(barrier) if settlement.barrier().is_pending() => (
            raw::ActivationBarrierStatus::Pending as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
        Some(barrier) => (
            raw::ActivationBarrierStatus::Cleared as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
    };

    let authoritative = settlement.authority() == VerdictAuthority::Authoritative;
    let settlement_basis = match (authoritative, outcome) {
        (true, VerdictOutcome::Satisfy) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::CompleteEvidence(
                raw::CompleteEvidenceBasis {},
            )),
        }),
        (true, VerdictOutcome::Violate) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::DecisiveViolation(
                decisive_violation_basis(
                    frontier,
                    assessment.witness_events(),
                    ABSENCE_PROOF_RULE_ID,
                    ABSENCE_PROOF_RULE_VERSION,
                    &gaps,
                    &uncertainty,
                ),
            )),
        }),
        _ => None,
    };

    raw::AssuranceEnvelope {
        specification_id: specification.identity().id().to_owned(),
        specification_version: specification.identity().version().to_owned(),
        schema_versions: vec![specification.schema_version().to_string()],
        filter_epochs,
        watermarks: watermarks.clone(),
        required_sources: coverage
            .required()
            .iter()
            .map(SourceIdentity::to_wire)
            .collect(),
        observed_sources: coverage
            .covered()
            .keys()
            .map(SourceIdentity::to_wire)
            .collect(),
        gaps,
        source_health,
        capabilities,
        source_tiers_and_anchors: tiers,
        unresolved_causal_edges,
        extraction_facts: refused_extraction_facts(any_refused),
        uncertainty,
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        cold_path: settlement.cold_path().to_raw(),
        hot_settlement_watermarks: watermarks,
        cold_settlement_watermarks: cold_settlement_watermarks(ledger, settlement),
        source_tier_gap,
        settlement_basis,
        activation_barrier,
        target_filter_epoch,
        // Caller-supplied RawEventRetentionDays (#245). Defaults to platform
        // default on Evaluator construction; api/harness paths that know a
        // Tenant horizon override via `with_raw_event_retention`.
        raw_event_retention_days: Some(raw_event_retention.value()),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        // Forecast-at-settle inventory (#244). Core does not evaluate Tenant
        // redaction policy and must not invent claimed paths.
        redacted_attributes: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn assurance_envelope_aggregate(
    specification: &AggregateSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &AggregateAssessment,
    settlement: &Settlement,
    coverage: &Coverage<'_>,
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
    outcome: VerdictOutcome,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> raw::AssuranceEnvelope {
    let mut filter_epochs = Vec::new();
    let mut watermarks = Vec::new();
    let mut gaps = Vec::new();
    let mut source_health = Vec::new();
    let mut capabilities = Vec::new();
    let mut tiers = Vec::new();

    for (source, record) in coverage.covered() {
        let healthy = record.is_contiguous() && !ledger.contradicts_delivery(source);
        let wire_source = source.to_wire();
        let completeness = record.completeness();

        filter_epochs.push(raw::FilterEpochBySource {
            source: Some(wire_source.clone()),
            filter_epoch: Some(completeness.filter_epoch().value()),
        });

        watermarks.push(raw::Watermark {
            source: Some(wire_source.clone()),
            sequence_epoch: completeness.epoch().value(),
            complete_through: completeness
                .complete_through()
                .map_or(0, sentinel_model::SequenceNumber::value),
            observed_at: Some(completeness.observed_at().to_wire()),
        });

        for gap in record.gaps() {
            gaps.push(raw::SourceGap {
                source: Some(wire_source.clone()),
                sequence_epoch: gap.sequence_epoch,
                first_missing: gap.first_missing,
                last_missing: gap.last_missing,
                effect: "delivery sequence is discontinuous, so the observed prefix is open"
                    .to_owned(),
            });
        }

        source_health.push(raw::SourceHealth {
            source: Some(wire_source.clone()),
            healthy,
            detail: if healthy {
                String::new()
            } else if record.is_contiguous() {
                "source's completeness signal contradicts what it delivered".to_owned()
            } else {
                "source delivered a discontinuous sequence".to_owned()
            },
        });

        capabilities.push(raw::SourceCapabilityRecord {
            source: Some(wire_source.clone()),
            capabilities: record
                .capabilities()
                .iter()
                .map(SourceCapability::to_raw)
                .collect(),
        });

        let tier = record.tier();
        tiers.push(raw::SourceTierRecord {
            source: Some(wire_source),
            tier: tier.to_raw(),
            // AMEND 3: never invent cold-witness ids here. Satisfy cites
            // trigger Events only via `anchored`.
            anchored_event_ids: if tier == SourceTier::Anchor {
                anchored
                    .get(source)
                    .map(|events| events.iter().map(ToString::to_string).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        });
    }

    let mut uncertainty = Vec::new();
    let mut prefix_details = Vec::new();
    if assessment.outcome() == AggregateOutcome::InconclusivePrefix {
        prefix_details.push(if gate.is_open() {
            "aggregation cold guard unsettled or Evidence Coverage still open".to_owned()
        } else {
            gate.detail()
        });
    }
    if let Some(pending) = settlement.barrier().pending() {
        prefix_details.push(pending.detail());
    }
    if !prefix_details.is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::PrefixUncertainty as i32,
            detail: prefix_details.join("; "),
        });
    }
    // Folded into **one** entry, exactly as `prefix_details` is folded above and
    // for the same model rule: `validate_uncertainty` admits at most one entry
    // per reason and rejects a second as `DuplicateValue("assurance.uncertainty")`.
    // This family is the first that can genuinely produce two epistemic
    // sentences at once — a Specification carrying two guards through
    // `with_cold_guard` can have one answer `Fails` while another answers
    // `Refused`, which is the case `CountEvidence::of`'s ordering exists to
    // decide. Pushing them separately made that Verdict unconstructible, and it
    // surfaced as `CoreError::Verdict`, which this crate reserves for its own
    // bugs.
    let mut epistemic_details = Vec::new();
    match assessment.outcome() {
        AggregateOutcome::OverThreshold => {
            epistemic_details.push(AGGREGATE_FAILS_DETAIL.to_owned());
        }
        // Only an *unasked* count says "never asked". A refused one was asked
        // and answered "never", and `REFUSED_GUARD_DETAIL` below is the honest
        // sentence for it; claiming both would name two different causes for
        // one conclusion.
        AggregateOutcome::InconclusiveEpistemic if assessment.count() == CountEvidence::Unasked => {
            epistemic_details.push(AGGREGATE_UNASKED_DETAIL.to_owned());
        }
        AggregateOutcome::Satisfy
        | AggregateOutcome::InconclusivePrefix
        | AggregateOutcome::InconclusiveEpistemic => {}
    }
    if any_refused {
        epistemic_details.push(REFUSED_GUARD_DETAIL.to_owned());
    }
    if !epistemic_details.is_empty() {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
            detail: epistemic_details.join("; "),
        });
    }

    let unresolved_causal_edges = frontier
        .unresolved_causal_edges()
        .into_iter()
        .map(|(event, predecessor)| raw::UnresolvedCausalEdge {
            event_id: event.to_string(),
            missing_predecessor_id: predecessor.to_string(),
        })
        .collect::<Vec<_>>();

    let source_tier_gap = source_tier_gap(coverage, anchored, &tiers, &BTreeSet::new());

    if settlement.barrier().is_pending() {
        let covered: BTreeSet<_> = coverage.covered().keys().collect();
        for source in coverage.required() {
            if !covered.contains(source) {
                filter_epochs.push(raw::FilterEpochBySource {
                    source: Some(source.to_wire()),
                    filter_epoch: None,
                });
            }
        }
    }

    let (activation_barrier, target_filter_epoch) = match specification.activation_barrier() {
        None => (raw::ActivationBarrierStatus::NotDeclared as i32, None),
        Some(barrier) if settlement.barrier().is_pending() => (
            raw::ActivationBarrierStatus::Pending as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
        Some(barrier) => (
            raw::ActivationBarrierStatus::Cleared as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
    };

    let authoritative = settlement.authority() == VerdictAuthority::Authoritative;
    // Never Violate from cold Fails (AMEND 3) — no DecisiveViolationBasis arm.
    let settlement_basis = match (authoritative, outcome) {
        (true, VerdictOutcome::Satisfy) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::CompleteEvidence(
                raw::CompleteEvidenceBasis {},
            )),
        }),
        _ => None,
    };

    raw::AssuranceEnvelope {
        specification_id: specification.identity().id().to_owned(),
        specification_version: specification.identity().version().to_owned(),
        schema_versions: vec![specification.schema_version().to_string()],
        filter_epochs,
        watermarks: watermarks.clone(),
        required_sources: coverage
            .required()
            .iter()
            .map(SourceIdentity::to_wire)
            .collect(),
        observed_sources: coverage
            .covered()
            .keys()
            .map(SourceIdentity::to_wire)
            .collect(),
        gaps,
        source_health,
        capabilities,
        source_tiers_and_anchors: tiers,
        unresolved_causal_edges,
        extraction_facts: refused_extraction_facts(any_refused),
        uncertainty,
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        cold_path: settlement.cold_path().to_raw(),
        hot_settlement_watermarks: watermarks,
        cold_settlement_watermarks: cold_settlement_watermarks(ledger, settlement),
        source_tier_gap,
        settlement_basis,
        activation_barrier,
        target_filter_epoch,
        // Caller-supplied RawEventRetentionDays (#245). Defaults to platform
        // default on Evaluator construction; api/harness paths that know a
        // Tenant horizon override via `with_raw_event_retention`.
        raw_event_retention_days: Some(raw_event_retention.value()),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        // Forecast-at-settle inventory (#244). Core does not evaluate Tenant
        // redaction policy and must not invent claimed paths.
        redacted_attributes: Vec::new(),
    }
}

/// Builds the cold path's settlement Watermarks, or none at all.
///
/// The observation time is the one the source's own completeness signal was
/// observed at: a fact response records a Watermark, not a clock reading, and
/// the Core has no clock to stamp one with.
fn cold_settlement_watermarks(
    ledger: &EvidenceLedger,
    settlement: &Settlement,
) -> Vec<raw::Watermark> {
    settlement
        .cold_settlement_watermarks()
        .iter()
        .filter_map(|(source, (epoch, sequence))| {
            let record = ledger.source(source)?;
            Some(raw::Watermark {
                source: Some(source.to_wire()),
                sequence_epoch: epoch.value(),
                complete_through: sequence.value(),
                observed_at: Some(record.completeness().observed_at().to_wire()),
            })
        })
        .collect()
}

fn source_tier_gap(
    coverage: &Coverage<'_>,
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
    tiers: &[raw::SourceTierRecord],
    extra_valid_anchors: &BTreeSet<SourceIdentity>,
) -> Option<raw::SourceTierGap> {
    let mut valid_anchor: BTreeSet<SourceIdentity> = coverage
        .covered()
        .iter()
        .filter(|(source, record)| {
            record.tier() == SourceTier::Anchor
                && anchored
                    .get(*source)
                    .is_some_and(|events| !events.is_empty())
        })
        .map(|(source, _)| source.clone())
        .collect();
    valid_anchor.extend(extra_valid_anchors.iter().cloned());
    // `tiers` is built from `coverage.covered()`, so the set it names *is* the
    // covered set: this length test is `sentinel_model`'s own
    // `tier_evidence.sources != required`, over the same two sets.
    let gap_required = tiers.len() != coverage.required().len() || valid_anchor.is_empty();
    if !gap_required {
        return None;
    }
    let contributing: Vec<raw::SourceIdentity> = coverage
        .covered()
        .iter()
        .filter(|(_, record)| record.tier() != SourceTier::Anchor)
        .map(|(source, _)| source.to_wire())
        .collect();
    let unanchored: Vec<raw::SourceIdentity> = coverage
        .required()
        .difference(&valid_anchor)
        .map(SourceIdentity::to_wire)
        .collect();
    Some(raw::SourceTierGap {
        contributing_sources: contributing,
        detail: "the conclusion rests on no anchor-tier Event".to_owned(),
        unanchored_sources: unanchored,
    })
}

fn decisive_violation_basis(
    frontier: &Frontier,
    cited_witnesses: &BTreeSet<EventId>,
    proof_rule_id: &str,
    proof_rule_version: &str,
    gaps: &[raw::SourceGap],
    uncertainty: &[raw::Uncertainty],
) -> raw::DecisiveViolationBasis {
    let mut witness_events = Vec::new();
    for event in &frontier.delivered() {
        let id = event.id();
        if !cited_witnesses.contains(&id) {
            continue;
        }
        let slot = event.delivery();
        witness_events.push(raw::DecisiveEvent {
            event_id: id.to_string(),
            source: Some(event.source().to_wire()),
            sequence_epoch: Some(slot.epoch().value()),
            sequence_number: Some(slot.sequence().value()),
        });
    }

    raw::DecisiveViolationBasis {
        witness_events,
        required_extraction_fact_ids: Vec::new(),
        invariance_proof_rule_id: proof_rule_id.to_owned(),
        invariance_proof_rule_version: proof_rule_version.to_owned(),
        outcome_invariant_limitations: Some(raw::OutcomeInvariantLimitations {
            gaps: gaps.to_vec(),
            unhealthy_sources: Vec::new(),
            unresolved_causal_edges: Vec::new(),
            unavailable_extraction_fact_ids: Vec::new(),
            uncertainty: uncertainty
                .iter()
                .map(|uncertainty| uncertainty.reason)
                .collect(),
        }),
    }
}

fn silent_complete_anchors(
    coverage: &Coverage<'_>,
    source_health: &[raw::SourceHealth],
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
) -> BTreeSet<SourceIdentity> {
    coverage
        .covered()
        .iter()
        .filter(|(source, record)| {
            record.tier() == SourceTier::Anchor
                && anchored.get(*source).is_none_or(BTreeSet::is_empty)
                && source_health.iter().any(|health| {
                    health.healthy && health.source.as_ref() == Some(&source.to_wire())
                })
        })
        .map(|(source, _)| source.clone())
        .collect()
}

fn watermark_anchored_violation_basis(
    silent: &BTreeSet<SourceIdentity>,
    watermarks: &[raw::Watermark],
    proof_rule_id: &str,
    proof_rule_version: &str,
    gaps: &[raw::SourceGap],
    uncertainty: &[raw::Uncertainty],
) -> raw::WatermarkAnchoredViolationBasis {
    let silent_wires: Vec<raw::SourceIdentity> =
        silent.iter().map(SourceIdentity::to_wire).collect();
    let silence_through_watermarks = watermarks
        .iter()
        .filter(|watermark| {
            watermark
                .source
                .as_ref()
                .is_some_and(|source| silent_wires.iter().any(|silent| silent == source))
        })
        .cloned()
        .collect();
    raw::WatermarkAnchoredViolationBasis {
        silent_anchor_sources: silent_wires,
        silence_through_watermarks,
        invariance_proof_rule_id: proof_rule_id.to_owned(),
        invariance_proof_rule_version: proof_rule_version.to_owned(),
        outcome_invariant_limitations: Some(raw::OutcomeInvariantLimitations {
            gaps: gaps.to_vec(),
            unhealthy_sources: Vec::new(),
            unresolved_causal_edges: Vec::new(),
            unavailable_extraction_fact_ids: Vec::new(),
            uncertainty: uncertainty
                .iter()
                .map(|uncertainty| uncertainty.reason)
                .collect(),
        }),
    }
}

/// Builds and validates the Verdict for one CAT-3 Binding.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_existence(
    specification: &ExistenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &ExistenceAssessment,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    let coverage = ledger.coverage(specification.required_source_identities());
    let (hot_outcome, hot_reason) = match assessment.outcome() {
        ExistenceOutcome::Satisfy => (VerdictOutcome::Satisfy, None),
        ExistenceOutcome::Violate => (VerdictOutcome::Violate, None),
        ExistenceOutcome::InconclusiveObservational => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::ObservationalUncertainty),
        ),
        ExistenceOutcome::InconclusivePrefix => (
            VerdictOutcome::Inconclusive,
            Some(InconclusiveReason::PrefixUncertainty),
        ),
    };
    let (outcome, reason) = epistemic_from_refused(hot_outcome, hot_reason, any_refused);

    let events_by_source = anchored_by_source(frontier, assessment.anchored_events());
    let assurance = assurance_envelope_existence(
        specification,
        ledger,
        frontier,
        gate,
        &ExistenceCitations::Open(assessment),
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: assessment.discharged_by().clone(),
            concurrent_pairs: assessment.concurrent_pairs().clone(),
            unmet_requirements: assessment.unmet_opens().clone(),
            retirement: None,
        },
    })
}

/// Builds and validates the Verdict for one **retired** CAT-3 Binding.
///
/// # Errors
///
/// Returns [`VerdictError`] when the assembled envelope violates a model
/// invariant.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_existence_retired(
    specification: &ExistenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    assessment: &RetiredAssessment,
    retirement: &Retirement,
    settlement: &Settlement,
    cost: BindingFactCost,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> Result<BindingVerdict, VerdictError> {
    let coverage = ledger.coverage(specification.required_source_identities());
    let (outcome, reason) = epistemic_from_refused(
        VerdictOutcome::Inconclusive,
        Some(InconclusiveReason::PrefixUncertainty),
        any_refused,
    );

    let events_by_source = anchored_by_source(frontier, &BTreeSet::new());
    let assurance = assurance_envelope_existence(
        specification,
        ledger,
        frontier,
        gate,
        &ExistenceCitations::Retired(assessment, retirement),
        settlement,
        &coverage,
        &events_by_source,
        outcome,
        any_refused,
        raw_event_retention,
    );

    let verdict = raw::Verdict {
        id: format!(
            "{}:{}:{}",
            specification.identity().id(),
            specification.identity().version(),
            assessment.binding()
        ),
        outcome: outcome.to_raw(),
        inconclusive_reason: reason.map(InconclusiveReason::to_raw),
        authority: settlement.authority().to_raw(),
        assurance: Some(assurance),
    };

    Ok(BindingVerdict {
        binding: assessment.key().clone(),
        verdict: Verdict::try_from(verdict)?,
        cost,
        explanation: Explanation {
            binding: assessment.binding().to_owned(),
            outcome,
            reason,
            discharged_by: assessment.discharged_by().clone(),
            concurrent_pairs: BTreeSet::new(),
            unmet_requirements: assessment.unmet_requirements().clone(),
            retirement: Some(retirement.clone()),
        },
    })
}

enum ExistenceCitations<'a> {
    Open(&'a ExistenceAssessment),
    Retired(&'a RetiredAssessment, &'a Retirement),
}

impl ExistenceCitations<'_> {
    fn concurrency_detail(&self) -> Option<String> {
        match self {
            Self::Open(assessment) if !assessment.concurrent_pairs().is_empty() => {
                Some(assessment.concurrency_detail())
            }
            Self::Open(_) | Self::Retired(_, _) => None,
        }
    }

    fn witness_events(&self) -> BTreeSet<EventId> {
        match self {
            Self::Open(assessment) => assessment.witness_events().clone(),
            Self::Retired(_, _) => BTreeSet::new(),
        }
    }

    fn prefix_details(&self, gate: &EvidenceGate) -> Vec<String> {
        match self {
            Self::Open(assessment) => {
                if assessment.outcome() == ExistenceOutcome::InconclusivePrefix {
                    vec![if gate.is_open() {
                        CONTRIBUTING_TIER_DETAIL.to_owned()
                    } else {
                        gate.detail()
                    }]
                } else {
                    Vec::new()
                }
            }
            Self::Retired(assessment, retirement) => {
                let mut details = vec![retirement.detail()];
                if assessment.observations_above_horizon() > 0 {
                    details.push(format!(
                        "{} Slice member(s) were observed above the retirement horizon \
                         and are excluded from the retired Slice",
                        assessment.observations_above_horizon()
                    ));
                }
                if assessment.observations_on_another_clock() > 0 {
                    details.push(format!(
                        "{} Slice member(s) carry an observation on another Sentinel clock, \
                         which the retirement horizon cannot order",
                        assessment.observations_on_another_clock()
                    ));
                }
                details
            }
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn assurance_envelope_existence(
    specification: &ExistenceSpecification,
    ledger: &EvidenceLedger,
    frontier: &Frontier,
    gate: &EvidenceGate,
    citations: &ExistenceCitations<'_>,
    settlement: &Settlement,
    coverage: &Coverage<'_>,
    anchored: &BTreeMap<SourceIdentity, BTreeSet<EventId>>,
    outcome: VerdictOutcome,
    any_refused: bool,
    raw_event_retention: RawEventRetentionDays,
) -> raw::AssuranceEnvelope {
    let mut filter_epochs = Vec::new();
    let mut watermarks = Vec::new();
    let mut gaps = Vec::new();
    let mut source_health = Vec::new();
    let mut capabilities = Vec::new();
    let mut tiers = Vec::new();

    for (source, record) in coverage.covered() {
        let healthy = record.is_contiguous() && !ledger.contradicts_delivery(source);
        let wire_source = source.to_wire();
        let completeness = record.completeness();

        filter_epochs.push(raw::FilterEpochBySource {
            source: Some(wire_source.clone()),
            filter_epoch: Some(completeness.filter_epoch().value()),
        });

        watermarks.push(raw::Watermark {
            source: Some(wire_source.clone()),
            sequence_epoch: completeness.epoch().value(),
            complete_through: completeness
                .complete_through()
                .map_or(0, sentinel_model::SequenceNumber::value),
            observed_at: Some(completeness.observed_at().to_wire()),
        });

        for gap in record.gaps() {
            gaps.push(raw::SourceGap {
                source: Some(wire_source.clone()),
                sequence_epoch: gap.sequence_epoch,
                first_missing: gap.first_missing,
                last_missing: gap.last_missing,
                effect: "delivery sequence is discontinuous, so the observed prefix is open"
                    .to_owned(),
            });
        }

        source_health.push(raw::SourceHealth {
            source: Some(wire_source.clone()),
            healthy,
            detail: if healthy {
                String::new()
            } else if record.is_contiguous() {
                "source's completeness signal contradicts what it delivered".to_owned()
            } else {
                "source delivered a discontinuous sequence".to_owned()
            },
        });

        capabilities.push(raw::SourceCapabilityRecord {
            source: Some(wire_source.clone()),
            capabilities: record
                .capabilities()
                .iter()
                .map(SourceCapability::to_raw)
                .collect(),
        });

        let tier = record.tier();
        tiers.push(raw::SourceTierRecord {
            source: Some(wire_source),
            tier: tier.to_raw(),
            anchored_event_ids: if tier == SourceTier::Anchor {
                anchored
                    .get(source)
                    .map(|events| events.iter().map(ToString::to_string).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        });
    }

    let mut uncertainty = Vec::new();
    if matches!(citations, ExistenceCitations::Retired(_, _)) {
        let details = citations.prefix_details(gate);
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::PrefixUncertainty as i32,
            detail: details.join("; "),
        });
    } else {
        if let Some(detail) = citations.concurrency_detail() {
            uncertainty.push(raw::Uncertainty {
                reason: raw::InconclusiveReason::ObservationalUncertainty as i32,
                detail,
            });
        }
        let mut prefix_details = citations.prefix_details(gate);
        if let Some(pending) = settlement.barrier().pending() {
            prefix_details.push(pending.detail());
        }
        if !prefix_details.is_empty() {
            uncertainty.push(raw::Uncertainty {
                reason: raw::InconclusiveReason::PrefixUncertainty as i32,
                detail: prefix_details.join("; "),
            });
        }
    }
    if any_refused {
        uncertainty.push(raw::Uncertainty {
            reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
            detail: REFUSED_GUARD_DETAIL.to_owned(),
        });
    }

    let unresolved_causal_edges = frontier
        .unresolved_causal_edges()
        .into_iter()
        .map(|(event, predecessor)| raw::UnresolvedCausalEdge {
            event_id: event.to_string(),
            missing_predecessor_id: predecessor.to_string(),
        })
        .collect::<Vec<_>>();

    // Missing-required Violate is watermark-anchored silence (ADR-0033), not a
    // DecisiveViolation citing the open Event: there is no falsifying Event to
    // witness — the Anchor-tier required source's settled silence is the proof.
    let silent_anchors = silent_complete_anchors(coverage, &source_health, anchored);
    let watermark_anchored_violate = settlement.authority() == VerdictAuthority::Authoritative
        && outcome == VerdictOutcome::Violate
        && anchored.values().all(BTreeSet::is_empty)
        && !silent_anchors.is_empty();
    let no_extra_anchors = BTreeSet::new();
    let extra_valid = if watermark_anchored_violate {
        &silent_anchors
    } else {
        &no_extra_anchors
    };
    let source_tier_gap = source_tier_gap(coverage, anchored, &tiers, extra_valid);

    if settlement.barrier().is_pending() {
        let covered: BTreeSet<_> = coverage.covered().keys().collect();
        for source in coverage.required() {
            if !covered.contains(source) {
                filter_epochs.push(raw::FilterEpochBySource {
                    source: Some(source.to_wire()),
                    filter_epoch: None,
                });
            }
        }
    }

    let (activation_barrier, target_filter_epoch) = match specification.activation_barrier() {
        None => (raw::ActivationBarrierStatus::NotDeclared as i32, None),
        Some(barrier) if settlement.barrier().is_pending() => (
            raw::ActivationBarrierStatus::Pending as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
        Some(barrier) => (
            raw::ActivationBarrierStatus::Cleared as i32,
            Some(barrier.target_filter_epoch().value()),
        ),
    };

    let authoritative = settlement.authority() == VerdictAuthority::Authoritative;
    let retired = matches!(citations, ExistenceCitations::Retired(_, _));
    let settlement_basis = match (authoritative, outcome, retired, watermark_anchored_violate) {
        (true, VerdictOutcome::Satisfy, false, _) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::CompleteEvidence(
                raw::CompleteEvidenceBasis {},
            )),
        }),
        (true, VerdictOutcome::Violate, false, true) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::WatermarkAnchoredViolation(
                watermark_anchored_violation_basis(
                    &silent_anchors,
                    &watermarks,
                    EXISTENCE_PROOF_RULE_ID,
                    EXISTENCE_PROOF_RULE_VERSION,
                    &gaps,
                    &uncertainty,
                ),
            )),
        }),
        (true, VerdictOutcome::Violate, false, false) => Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::DecisiveViolation(
                decisive_violation_basis(
                    frontier,
                    &citations.witness_events(),
                    EXISTENCE_PROOF_RULE_ID,
                    EXISTENCE_PROOF_RULE_VERSION,
                    &gaps,
                    &uncertainty,
                ),
            )),
        }),
        // Retired conclusions and non-authoritative open ones share the empty basis.
        (true | false, _, true | false, true | false) => None,
    };

    raw::AssuranceEnvelope {
        specification_id: specification.identity().id().to_owned(),
        specification_version: specification.identity().version().to_owned(),
        schema_versions: vec![specification.schema_version().to_string()],
        filter_epochs,
        watermarks: watermarks.clone(),
        required_sources: coverage
            .required()
            .iter()
            .map(SourceIdentity::to_wire)
            .collect(),
        observed_sources: coverage
            .covered()
            .keys()
            .map(SourceIdentity::to_wire)
            .collect(),
        gaps,
        source_health,
        capabilities,
        source_tiers_and_anchors: tiers,
        unresolved_causal_edges,
        extraction_facts: refused_extraction_facts(any_refused),
        uncertainty,
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        cold_path: settlement.cold_path().to_raw(),
        hot_settlement_watermarks: watermarks,
        cold_settlement_watermarks: cold_settlement_watermarks(ledger, settlement),
        source_tier_gap,
        settlement_basis,
        activation_barrier,
        target_filter_epoch,
        // Caller-supplied RawEventRetentionDays (#245). Defaults to platform
        // default 30 via Evaluator; Tenant horizon override via
        // `with_raw_event_retention`.
        raw_event_retention_days: Some(raw_event_retention.value()),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        // Forecast-at-settle inventory (#244). Core does not evaluate Tenant
        // redaction policy and must not invent claimed paths.
        redacted_attributes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    /// Names every builder the `compile_fail` doctests on
    /// [`crate::settlement::Settlement`] list.
    ///
    /// Stable rustdoc does not check a `compile_fail` block's error code, so a block naming
    /// a renamed or removed builder fails with `E0425` and still passes. This test makes
    /// that rename or removal break the `--lib` build instead, and whoever made it must
    /// carry the doctest list forward. It guards **names only**: privacy is guarded by the
    /// doctests themselves and, for any builder taking a `&Settlement`, by `Settlement`
    /// being sealed (#304).
    #[test]
    fn builders_named_by_the_sealed_doctests_still_exist() {
        let _ = super::build;
        let _ = super::build_retired;
        let _ = super::build_taint;
        let _ = super::build_absence;
        let _ = super::build_aggregate;
        let _ = super::build_existence;
        let _ = super::build_existence_retired;
    }
}
