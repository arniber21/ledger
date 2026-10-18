//! The monitor for one bounded precedence Specification, and its trace-closure argument.
//!
//! # Trace closure (ADR-0020)
//!
//! ADR-0020 is explicit that trace-closure is "a compiler obligation ... a test
//! obligation on the catalogue, not a runtime check". For a hand-written
//! Specification the obligation is discharged by argument, and this doc comment
//! **is** that discharge. The arrival-order test in `core/tests` is an
//! implementation-determinism test; it does not discharge this obligation, and
//! it does not claim to.
//!
//! The obligation to discharge: every verdict this monitor *settles* is the same
//! in every admissible linearization of the observed evidence. An admissible
//! linearization is a total order consistent with happens-before.
//!
//! **Step 1 — the verdict is a function of the partial order alone.** The
//! decision procedure reads exactly two things: the transitive reachability
//! relation over admitted Events, and those Events' attribute valuations.
//! Neither is a function of arrival order. Reachability is the transitive
//! closure of the producer-declared edge set, which is fixed by the evidence;
//! attribute valuations are fixed per Event. No clock is read (every source
//! withholds `BoundedClockUncertainty`, so `compare_occurrence_times` cannot
//! contribute order even if it were consulted), and no linearization is ever
//! materialised.
//!
//! **Step 2 — every settled row is invariant across linearizations.** The
//! per-trigger table is ordered, and first match wins:
//!
//! | # | condition | outcome |
//! |---|---|---|
//! | 1 | `Before(t)` non-empty | discharged |
//! | 2 | `Concurrent(t)` non-empty | observationally uncertain |
//! | 3 | `W == After(t)`, possibly empty, and the Watermark covers the epoch | violated |
//! | 4 | otherwise | prefix-open |
//!
//! Row 1 fires when some witness `w` satisfies `w ->* t`. Every admissible
//! linearization respects happens-before, so `w` precedes `t` in *all* of them,
//! and the existential obligation `exists w. w ->* t` is discharged in all of
//! them. The verdict is invariant.
//!
//! Row 3 fires when no witness can discharge `t` in *any* linearization: `W` is
//! exhausted — every requirement Event either does not exist, or lies in `t`'s
//! causal future, where no linearization may move it before `t` — and the
//! Watermark says no further requirement Event is outstanding. The verdict is
//! invariant.
//!
//! Row 2 fires exactly when neither invariant holds: some witness is Concurrent
//! with `t`, so some admissible linearization places it before `t` (discharging
//! the obligation) and another places it after (leaving it undischarged). The
//! monitor therefore does not settle; it returns `Inconclusive(observational)`
//! and names the concrete concurrent pair.
//!
//! Note carefully what row ordering does to this argument: rows 1 and 2 can both
//! be *true* — one witness `Before t`, another `Concurrent` with `t` — and row 1
//! preempts row 2. That is correct, and it is why the invariance claim is
//! "invariant whenever row 1 fires **or** row 3 fires", not "the only
//! order-sensitive case is a Concurrent witness". A Concurrent witness alongside
//! a `Before` witness changes nothing: the existential is already discharged in
//! every linearization. `shared/oracle.ts` makes the same choice, checking
//! `chargeSpans.some((c) => happensBefore(spans, c, shipSpan))` before it
//! considers incomparability.
//!
//! Row 4 does not settle either, and reports prefix uncertainty.
//!
//! **Step 3 — aggregation preserves invariance.** The Binding's obligation is
//! universally quantified over its triggers, so the Binding verdict is the
//! dominance maximum `Violate > Inconclusive(observational) >
//! Inconclusive(prefix) > Satisfy` over per-trigger outcomes. One trigger's
//! settled violation falsifies the conjunction in every linearization; one
//! trigger's uncertainty makes the conjunction uncertain. A settled Binding
//! verdict is therefore invariant whenever every trigger's outcome is, which is
//! exactly steps 1 and 2.
//!
//! Together: every settled verdict is invariant across all admissible
//! linearizations, and no linearization is ever materialised.
//!
//! # Evidence gate
//!
//! The gate itself is [`crate::evidence::EvidenceGate`]. CAT-1 consults it
//! **before** any obligation analysis and downgrades every Binding to
//! `Inconclusive(prefix)`. Its fourth disjunct is soundness critical: without
//! it, an Event that arrived, matched the requirement selector, and was never
//! admitted because a declared causal predecessor never arrived would leave `W`
//! empty over *admitted* Events while a discharging witness demonstrably exists
//! in the input. Row 3 would then fire an authoritative Violate on evidence that
//! disproves it, and `sentinel_model` cannot catch it: it rejects a *witness*
//! touched by an unresolved edge, but a violate-by-absence conclusion has no
//! witness to reject.
//!
//! The gate is deliberately **global** rather than per-Binding in this slice:
//! any held Event anywhere downgrades every Binding. That is coarse, and it is
//! chosen knowingly. Scoping held-Event reachability to the affected Slice is a
//! **#17** refinement — it is a precision fix to precedence semantics with
//! nothing to do with resumability, so #12 deliberately left it alone — and
//! over-approximating toward Inconclusive is the safe direction.
//!
//! # The gate's disjuncts under stepping (issue #12)
//!
//! G1 and G1b are the two halves of Evidence Coverage failing separately, and
//! they are deliberately not one disjunct. Coverage is the conjunction of "the
//! Core received an Event from this source" and "this source declared its
//! `SourceEvidence`" (`crate::evidence::Coverage`); G1 is the first half
//! failing, G1b the second. Before issue #158 only G1 existed and it silently
//! covered both, so a Verdict whose source had delivered sixteen Events could
//! carry the sentence "a required source supplied no Event".
//!
//! G2 and G3 are no longer inserted together. G2 is the source truthfully
//! reporting its own discontinuity; G3 is the source's completeness claim
//! **contradicting the Core's own receipts**, which no truthful source
//! produces. G5 is the third distinct claim: the Core has not yet received
//! every Event up to a boundary the source declared. G5 fires mid-stream
//! constantly, and that is correct rather than spurious — it claims only "the
//! Core has not caught up", which is true whenever it fires. It is a gate that
//! closes, not a health bit that lies.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{Event, EventId, SourceTier};

use crate::evidence::{EvidenceGate, EvidenceLedger};
use crate::frontier::{CausalIndex, CausalOrder, Frontier};
use crate::slice::{Binding, Retirement, SliceIndex, SliceLifecycles};
use crate::specification::PrecedenceSpecification;

/// Named proof rule under which a violation is invariant across linearizations.
pub const INVARIANCE_PROOF_RULE_ID: &str = "precedence.universal-conjunction-dominance";
/// Version of [`INVARIANCE_PROOF_RULE_ID`].
pub const INVARIANCE_PROOF_RULE_VERSION: &str = "v1";

/// The conclusion reached for one Binding.
///
/// The ordering is the dominance order the monitor aggregates with: a larger
/// value dominates a smaller one.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum BindingOutcome {
    /// Every trigger's obligation is discharged.
    Satisfy,
    /// At least one trigger's evidence is open, and none is violated.
    InconclusivePrefix,
    /// At least one trigger has a Concurrent witness, and none is violated.
    InconclusiveObservational,
    /// At least one trigger's obligation is falsified.
    Violate,
}

/// What the monitor concluded about one Binding, and the evidence it cites.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BindingAssessment {
    binding: Binding,
    outcome: BindingOutcome,
    discharged_by: BTreeMap<EventId, EventId>,
    concurrent_pairs: BTreeSet<(EventId, EventId)>,
    unmet_requirements: BTreeSet<EventId>,
    anchored_events: BTreeSet<EventId>,
    witness_events: BTreeSet<EventId>,
}

impl BindingAssessment {
    /// Returns the Binding value.
    #[must_use]
    pub fn binding(&self) -> &str {
        self.binding.as_str()
    }

    /// Returns the Binding this conclusion is about.
    #[must_use]
    pub const fn key(&self) -> &Binding {
        &self.binding
    }

    /// Returns the conclusion for this Binding.
    #[must_use]
    pub const fn outcome(&self) -> BindingOutcome {
        self.outcome
    }

    /// Returns the witness that discharges each discharged trigger.
    ///
    /// Read-side only, so a single representative is enough: the minimum by
    /// Event identity of the triggering Event's `Before` set.
    #[must_use]
    pub const fn discharged_by(&self) -> &BTreeMap<EventId, EventId> {
        &self.discharged_by
    }

    /// Returns every `(witness, trigger)` pair the evidence leaves unordered.
    #[must_use]
    pub const fn concurrent_pairs(&self) -> &BTreeSet<(EventId, EventId)> {
        &self.concurrent_pairs
    }

    /// Returns triggers whose obligation no witness discharges.
    #[must_use]
    pub const fn unmet_requirements(&self) -> &BTreeSet<EventId> {
        &self.unmet_requirements
    }

    /// Returns the anchor-tier Events the conclusion rests on.
    ///
    /// This is a **set**, never a pick: `canonicalize_assurance` sorts the wire
    /// vectors but cannot repair membership that varies with arrival order.
    #[must_use]
    pub const fn anchored_events(&self) -> &BTreeSet<EventId> {
        &self.anchored_events
    }

    /// Returns the Events a decisive violation witness names.
    #[must_use]
    pub const fn witness_events(&self) -> &BTreeSet<EventId> {
        &self.witness_events
    }

    /// Returns a deterministic description of the unordered pairs.
    #[must_use]
    pub fn concurrency_detail(&self) -> String {
        self.concurrent_pairs
            .iter()
            .map(|(witness, trigger)| format!("{witness} || {trigger}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// What the monitor recorded about a **retired** Binding, and the evidence it
/// cites.
///
/// A [`BindingAssessment`] with the outcome removed. That absence is the whole
/// point: ADR-0021's O-03 pins a retired Scope to `Inconclusive`, and this type
/// makes [`BindingOutcome::Violate`] and [`BindingOutcome::Satisfy`]
/// **unconstructible** on the retired path rather than demoted on it. There is
/// no field to hold either value and no accessor to read one back, so the
/// guarantee is a property of the type instead of a rule a later edit can undo
/// by moving a branch.
///
/// That is ADR-0031 clause 1's shape, and this repository has already
/// adjudicated it once: [`crate::taint::ConvictedOutcome`] is "an enum with no
/// `Satisfy` member, and the defective fold is therefore a compile error". Same
/// move here — a wider value is computed upstream by [`assess_binding`] and
/// narrowed by a type at the seam.
///
/// # Why the wider value is computed at all
///
/// [`assess_binding`] runs unchanged over the in-scope Slice, and its outcome is
/// then dropped by [`RetiredAssessment::from_assessment`]. That is deliberately
/// wasted work rather than an extraction: `assess_binding` carries the ADR-0033
/// silent-anchor branch and the trace-closure argument this module's header
/// discharges, and refactoring it to reuse only its classification loop would
/// put the crate's most load-bearing function at risk for a guarantee this type
/// already delivers.
///
/// Three fields are dropped rather than carried, and each is a field
/// `assess_binding` itself would have cleared for an
/// [`BindingOutcome::InconclusivePrefix`]: `anchored_events` and
/// `witness_events` because a retired Verdict rests on no anchor and names no
/// decisive witness, and `concurrent_pairs` because the conclusion does not turn
/// on an unordered pair. What is **kept** is `discharged_by` and
/// `unmet_requirements` — the evidence a reader needs for the Explanation, so
/// that a retired Verdict still reports the conclusion *and the evidence behind
/// it* rather than naming nothing at all.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RetiredAssessment {
    binding: Binding,
    discharged_by: BTreeMap<EventId, EventId>,
    unmet_requirements: BTreeSet<EventId>,
    observations_above_horizon: usize,
    observations_on_another_clock: usize,
}

impl RetiredAssessment {
    /// Narrows a full assessment to the retired one, dropping its outcome.
    ///
    /// Total: every [`BindingAssessment`] narrows, whatever outcome it reached.
    /// A Binding whose evidence would otherwise have settled `Violate` narrows
    /// to exactly the same shape as one that would have settled `Satisfy`,
    /// which is the pin stated as a function rather than as a branch.
    ///
    /// The two counts are the Binding's members the horizon **excluded** from
    /// the retired Slice, kept apart because they are different facts:
    /// `observations_above_horizon` were observed after it, and
    /// `observations_on_another_clock` carry an observation the horizon cannot
    /// be compared with at all. Both are supplied by the caller because both
    /// are computed at assessment time, over the whole delivered set: computing
    /// either when the retirement signal arrived would make it a function of
    /// arrival position, and it lands inside the encoded Verdict.
    #[must_use]
    pub fn from_assessment(
        assessment: BindingAssessment,
        observations_above_horizon: usize,
        observations_on_another_clock: usize,
    ) -> Self {
        Self {
            binding: assessment.binding,
            discharged_by: assessment.discharged_by,
            unmet_requirements: assessment.unmet_requirements,
            observations_above_horizon,
            observations_on_another_clock,
        }
    }

    /// Builds a retired assessment from its constituent fields.
    ///
    /// Used by Pattern families that do not share [`BindingAssessment`] (CAT-3
    /// existence) but still pin retirement through this narrowing type.
    #[must_use]
    pub fn from_parts(
        binding: Binding,
        discharged_by: BTreeMap<EventId, EventId>,
        unmet_requirements: BTreeSet<EventId>,
        observations_above_horizon: usize,
        observations_on_another_clock: usize,
    ) -> Self {
        Self {
            binding,
            discharged_by,
            unmet_requirements,
            observations_above_horizon,
            observations_on_another_clock,
        }
    }

    /// Returns the Binding value.
    #[must_use]
    pub fn binding(&self) -> &str {
        self.binding.as_str()
    }

    /// Returns the Binding this retired conclusion is about.
    #[must_use]
    pub const fn key(&self) -> &Binding {
        &self.binding
    }

    /// Returns the witness that discharges each discharged trigger.
    #[must_use]
    pub const fn discharged_by(&self) -> &BTreeMap<EventId, EventId> {
        &self.discharged_by
    }

    /// Returns triggers whose obligation no witness discharges.
    #[must_use]
    pub const fn unmet_requirements(&self) -> &BTreeSet<EventId> {
        &self.unmet_requirements
    }

    /// Returns how many of the Binding's members sit above the horizon.
    ///
    /// Zero is the ordinary case and is not a disclosure. A non-zero count is
    /// the honest form of the rejected clause that would have refused a horizon
    /// below an already-delivered observation: the evidence exists, the
    /// periphery's horizon excludes it, and the Verdict says so rather than the
    /// Core deciding on the driver's behalf.
    #[must_use]
    pub const fn observations_above_horizon(&self) -> usize {
        self.observations_above_horizon
    }

    /// Returns how many of the Binding's members the horizon cannot order.
    ///
    /// An Event whose observation sits on a different Sentinel clock is out of
    /// the retired Slice, because a horizon orders nothing across clocks. It is
    /// counted separately rather than folded into
    /// [`RetiredAssessment::observations_above_horizon`]: "observed after the
    /// horizon" and "not comparable with the horizon" are different facts, and
    /// reporting the second as the first would claim an ordering the evidence
    /// does not supply. Excluded evidence that went unreported would be exactly
    /// the silent vanish O-03 is designed against.
    #[must_use]
    pub const fn observations_on_another_clock(&self) -> usize {
        self.observations_on_another_clock
    }
}

/// One Binding's conclusion, in whichever of the two shapes it took.
///
/// Open Bindings carry the full [`BindingAssessment`] and reach the ordered
/// outcome table unchanged. A retired Binding carries a [`RetiredAssessment`]
/// and its [`Retirement`], so the outcome it would have reached is gone by the
/// time envelope assembly sees it.
///
/// The `Retirement` is carried rather than looked up again at the Verdict seam:
/// a lookup would need an `expect` for a case the type already rules out.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BindingConclusion {
    /// The Binding's Scope has not been retired.
    Open(BindingAssessment),
    /// The Binding's Scope was retired, pinning it to prefix uncertainty.
    Retired(RetiredAssessment, Retirement),
}

impl BindingConclusion {
    /// Returns the Binding this conclusion is about.
    #[must_use]
    pub const fn key(&self) -> &Binding {
        match self {
            Self::Open(assessment) => assessment.key(),
            Self::Retired(assessment, _) => assessment.key(),
        }
    }
}

/// Per-trigger outcome from the ordered decision table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObligationOutcome {
    Discharged,
    ObservationallyUncertain,
    Violated,
    PrefixOpen,
}

/// Evaluates the Specification over every Binding a trigger Event creates.
///
/// **Bindings are created by trigger Events.** An order that never shipped
/// creates no Binding and no Verdict: a vacuous Satisfy would rest on no
/// anchoring Event, and ADR-0017 forbids inferring an absent application from
/// no telemetry without a Topology expectation, which this slice has none of.
/// `shared/oracle.ts` makes the identical call — `if (shipSpans.length === 0)
/// continue; // property is vacuously true for orders that never shipped` —
/// which is what makes the fidelity assertion against SagaShop's frozen matrix
/// meaningful rather than coincidental.
///
/// # Retirement
///
/// The **iteration set is unchanged**: `slices.bindings()`, and nothing else.
/// A retired Binding the Core has admitted no Event for is not iterated and
/// produces no Verdict, which is this function's existing rule rather than a
/// new one. Iterating the union of the Slice index and the lifecycles would
/// manufacture an assessment for a Binding the Core has never seen, and would
/// re-open the empty-Evidence-Coverage wedge by a second route.
///
/// Two things are conditional on retirement, and only two. The Slice and the
/// trigger set are narrowed to the members at or below the horizon, and the
/// **empty-trigger skip is bypassed** — a Binding whose only trigger sits above
/// the horizon still emits, because a retired Binding vanishing silently is the
/// failure mode O-03 is designed against.
#[must_use]
pub fn assess(
    specification: &PrecedenceSpecification,
    index: &CausalIndex,
    frontier: &Frontier,
    slices: &SliceIndex,
    lifecycles: &SliceLifecycles,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> Vec<BindingConclusion> {
    let mut conclusions = Vec::new();
    for binding in slices.bindings() {
        let retirement = lifecycles.retirement_of(binding);
        // Identical to `slices.members(binding)` for an open Binding, with both
        // exclusion counts zero, so the open path is unchanged.
        let membership = lifecycles.members_in_scope(binding, slices, frontier);
        let slice: Vec<&Event> = membership
            .in_scope()
            .iter()
            .filter_map(|id| frontier.event(id))
            .collect();
        let triggers: Vec<&Event> = slices
            .obligation_raising(binding)
            .into_iter()
            .flatten()
            .filter(|id| membership.in_scope().contains(*id))
            .filter_map(|id| frontier.event(id))
            .collect();
        if retirement.is_none() && triggers.is_empty() {
            continue;
        }
        let assessment = assess_binding(
            specification,
            index,
            gate,
            ledger,
            binding.clone(),
            &slice,
            &triggers,
        );
        conclusions.push(match retirement {
            None => BindingConclusion::Open(assessment),
            Some(retired) => BindingConclusion::Retired(
                RetiredAssessment::from_assessment(
                    assessment,
                    membership.above_horizon(),
                    membership.incomparable_clock(),
                ),
                retired.clone(),
            ),
        });
    }
    conclusions.sort_by(|left, right| left.key().cmp(right.key()));
    conclusions
}

#[allow(clippy::too_many_lines)] // The ordered decision table stays readable in one place.
fn assess_binding(
    specification: &PrecedenceSpecification,
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
    binding: Binding,
    slice: &[&Event],
    triggers: &[&Event],
) -> BindingAssessment {
    let requirements: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.requirement().matches(event))
        .collect();

    // Anchor-tier Events of the requirement's kind in this Slice, whether or not
    // they match the requirement's attribute valuations. A violate row rests on
    // these: they are what makes the absence of a *matching* requirement Event
    // an anchor-witnessed fact rather than mere silence.
    let anchor_candidates: BTreeSet<EventId> = slice
        .iter()
        .filter(|event| {
            specification.requirement().matches_kind(event)
                && ledger.tier_of(&event.source()) == Some(SourceTier::Anchor)
        })
        .map(|event| event.id())
        .collect();

    let mut discharged_by = BTreeMap::new();
    let mut concurrent_pairs = BTreeSet::new();
    let mut unmet_requirements = BTreeSet::new();
    let mut anchored_events = BTreeSet::new();
    let mut witness_events = BTreeSet::new();
    let mut outcome = BindingOutcome::Satisfy;

    for trigger in triggers {
        let trigger_id = trigger.id();
        let mut before = BTreeSet::new();
        let mut concurrent = BTreeSet::new();
        for requirement in &requirements {
            match index.classify(&requirement.id(), &trigger_id) {
                CausalOrder::Before => {
                    before.insert(requirement.id());
                }
                CausalOrder::Concurrent => {
                    concurrent.insert(requirement.id());
                }
                CausalOrder::After => {}
            }
        }

        let trigger_outcome = if !before.is_empty() {
            ObligationOutcome::Discharged
        } else if !concurrent.is_empty() {
            ObligationOutcome::ObservationallyUncertain
        } else if gate.watermarks_cover_epochs() {
            ObligationOutcome::Violated
        } else {
            ObligationOutcome::PrefixOpen
        };

        match trigger_outcome {
            ObligationOutcome::Discharged => {
                let witness = before.iter().next().expect("non-empty before set").clone();
                discharged_by.insert(trigger_id.clone(), witness);
                anchored_events.extend(
                    before
                        .iter()
                        .filter(|id| anchor_candidates.contains(*id))
                        .cloned(),
                );
            }
            ObligationOutcome::ObservationallyUncertain => {
                unmet_requirements.insert(trigger_id.clone());
                for witness in &concurrent {
                    concurrent_pairs.insert((witness.clone(), trigger_id.clone()));
                }
                anchored_events.extend(
                    concurrent
                        .iter()
                        .filter(|id| anchor_candidates.contains(*id))
                        .cloned(),
                );
                outcome = outcome.max(BindingOutcome::InconclusiveObservational);
            }
            ObligationOutcome::Violated => {
                unmet_requirements.insert(trigger_id.clone());
                if anchor_candidates.is_empty() {
                    // ADR-0033: watermark-complete silence of a required
                    // *anchor-tier* source settles a Violate. If no required
                    // source is anchor-tier, silence cannot anchor; stay
                    // Inconclusive. `anchored_events` stays empty so envelope
                    // assembly emits `WatermarkAnchoredViolationBasis`.
                    let has_silent_anchor = specification
                        .required_source_identities()
                        .iter()
                        .any(|source| ledger.tier_of(source) == Some(SourceTier::Anchor));
                    if has_silent_anchor {
                        witness_events.insert(trigger_id.clone());
                        outcome = outcome.max(BindingOutcome::Violate);
                    } else {
                        outcome = outcome.max(BindingOutcome::InconclusivePrefix);
                    }
                } else {
                    anchored_events.extend(anchor_candidates.iter().cloned());
                    witness_events.insert(trigger_id.clone());
                    witness_events.extend(anchor_candidates.iter().cloned());
                    outcome = outcome.max(BindingOutcome::Violate);
                }
            }
            ObligationOutcome::PrefixOpen => {
                unmet_requirements.insert(trigger_id.clone());
                outcome = outcome.max(BindingOutcome::InconclusivePrefix);
            }
        }
    }

    if !gate.is_open() {
        // The gate runs before obligation analysis, so it wins outright rather
        // than by dominance: it must be able to demote a Violate.
        outcome = BindingOutcome::InconclusivePrefix;
        anchored_events.clear();
        witness_events.clear();
    }

    if outcome != BindingOutcome::Violate {
        witness_events.clear();
    }
    if matches!(
        outcome,
        BindingOutcome::Satisfy | BindingOutcome::InconclusivePrefix
    ) {
        // Concurrency is only cited when it is what the conclusion turns on.
        // A Satisfy never records any; a gated prefix Inconclusive rests on the
        // gate, not on an unordered pair.
        concurrent_pairs.clear();
    }

    BindingAssessment {
        binding,
        outcome,
        discharged_by,
        concurrent_pairs,
        unmet_requirements,
        anchored_events,
        witness_events,
    }
}

#[cfg(test)]
mod tests {
    use super::{Binding, BindingAssessment, BindingOutcome, EventId, RetiredAssessment};
    use std::collections::{BTreeMap, BTreeSet};

    fn id(value: &str) -> EventId {
        EventId::new(value).expect("valid id")
    }

    /// A settled Violate, complete with the anchor and witness sets that make it
    /// authoritative.
    fn convicting_assessment() -> BindingAssessment {
        BindingAssessment {
            binding: Binding::new("order-1"),
            outcome: BindingOutcome::Violate,
            discharged_by: BTreeMap::from([(id("trigger-a"), id("witness-a"))]),
            concurrent_pairs: BTreeSet::from([(id("witness-b"), id("trigger-b"))]),
            unmet_requirements: BTreeSet::from([id("trigger-b")]),
            anchored_events: BTreeSet::from([id("witness-a")]),
            witness_events: BTreeSet::from([id("trigger-b")]),
        }
    }

    /// The same Binding and the same discharge evidence, reaching the opposite
    /// end of the outcome lattice and citing nothing.
    fn acquitting_assessment() -> BindingAssessment {
        BindingAssessment {
            binding: Binding::new("order-1"),
            outcome: BindingOutcome::Satisfy,
            discharged_by: BTreeMap::from([(id("trigger-a"), id("witness-a"))]),
            concurrent_pairs: BTreeSet::new(),
            unmet_requirements: BTreeSet::from([id("trigger-b")]),
            anchored_events: BTreeSet::new(),
            witness_events: BTreeSet::new(),
        }
    }

    /// The evidence a reader needs survives the narrowing.
    ///
    /// ADR-0017's product statement is that a Verdict reports the conclusion
    /// **and the evidence behind it**. A retired Verdict assembled from an
    /// assessment that had dropped `discharged_by` and `unmet_requirements`
    /// would carry an Explanation naming no evidence at all, which is the
    /// failure mode this type is built to avoid rather than to cause.
    #[test]
    fn a_retired_assessment_carries_the_explanation_forward() {
        let retired = RetiredAssessment::from_assessment(convicting_assessment(), 2, 1);

        assert_eq!(retired.key(), &Binding::new("order-1"));
        assert_eq!(retired.binding(), "order-1");
        assert_eq!(
            retired.discharged_by().get(&id("trigger-a")),
            Some(&id("witness-a"))
        );
        assert!(retired.unmet_requirements().contains(&id("trigger-b")));
        assert_eq!(retired.observations_above_horizon(), 2);
        assert_eq!(retired.observations_on_another_clock(), 1);
    }

    /// Narrowing is outcome-blind, and drops every citation a settled outcome
    /// would have rested on.
    ///
    /// The two inputs sit at opposite ends of the dominance order and disagree
    /// on `anchored_events`, `witness_events` and `concurrent_pairs`. They
    /// narrow to the **same** value, which is "never `Violate`, never `Satisfy`"
    /// as an executable statement rather than as a branch someone could later
    /// move. It is also what makes an ADR-0033 watermark-anchored Violate
    /// unreachable from a retired Binding: the basis has no `witness_events` to
    /// be assembled from.
    #[test]
    fn a_retired_assessment_cites_no_anchor_or_witness_events() {
        let from_violate = RetiredAssessment::from_assessment(convicting_assessment(), 0, 0);
        let from_satisfy = RetiredAssessment::from_assessment(acquitting_assessment(), 0, 0);

        assert_eq!(from_violate, from_satisfy);
    }
}
