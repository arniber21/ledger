//! The decision table for one CAT-2 absence obligation, and why it inherits
//! CAT-4's presence-above-tier shape.
//!
//! CAT-2 `absence.after-until` states the corpus's P-B property: for every
//! Event matching the `until` selector in a Binding's Slice, **no** Event
//! matching the `veto` selector lies in the open-closed interval
//! `(after, until]` — after some Event matching `after` that reaches `until`,
//! up to and including the obligation-raising close. That is an absence claim
//! about a *presence* of a veto inside a causally delimited Scope, so the
//! silent-pass hole CAT-4 closed applies verbatim:
//!
//! > where a Pattern's Violate rests on the presence of an Event, the
//! > source-tier test sits strictly below the presence test and may only ever
//! > choose among non-`Satisfy` outcomes.
//!
//! # Extra facts vs CAT-4
//!
//! CAT-4's Scope is the action Event's entire causal past. CAT-2's Scope is an
//! **interval** that may never open: an `until` with no `after` that reaches it
//! raises no obligation once Evidence Coverage is settled. That path is
//! [`decide`]'s `None` — **NoObligation** — and it is never rendered as
//! `Satisfy`. A Binding whose every `until` is skipped contributes nothing to
//! [`aggregate`]; an empty aggregate is `None`, and [`assess`] emits no Verdict
//! for it. That is the third place this module refuses a vacuous pass.
//!
//! Concurrent `after` with `until`, or Concurrent `veto` with `until`, is
//! observational uncertainty: some linearization opens (or places the veto
//! inside) the interval and another does not.
//!
//! # The table
//!
//! Per `until` Event `u`; ordered, first match wins. `None` means this `until`
//! contributes nothing to the Binding aggregate.
//!
//! | # | condition | outcome |
//! |---|---|---|
//! | 0 | the evidence gate is closed | prefix-open |
//! | 1 | some veto in `(after, until]` with an anchor-tier kind witness | violated |
//! | 2 | some veto in `(after, until]`, contributing-tier only | prefix-open |
//! | 3 | some `after` or `veto` Concurrent with `until` | observationally uncertain |
//! | 4 | no `after` reaches `until`, and Evidence Coverage is closed | *no obligation* |
//! | 5 | no `after` reaches `until`, coverage still open | prefix-open |
//! | 6 | interval opened, clean, coverage closed | discharged |
//! | 7 | otherwise | prefix-open |
//!
//! Rows 1 and 2 share one identical premise — a falsifying veto *is present*
//! inside the interval — and [`ConvictedOutcome`] is why the tier test cannot
//! answer `Satisfy`. Row 4 is deliberately not a pass: a Scope that never
//! opened has nothing to discharge.
//!
//! # Traversal
//!
//! [`observe`] produces an [`AbsenceObservation`] from a Binding's Slice, the
//! [`crate::frontier::CausalIndex`], and the [`crate::evidence::EvidenceGate`].
//! [`assess`] folds those observations through [`decide`] and [`aggregate`].
//! [`crate::evaluator::Evaluator`] files CAT-2 `until` Events as
//! [`crate::slice::SliceRole::ObligationRaising`] and `after`/`veto` Events as
//! [`crate::slice::SliceRole::Member`], then calls [`assess`] at seal.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeSet;

use sentinel_compile::ir::AbsenceIr;
use sentinel_model::{
    ActivationBarrier, Event, EventId, SchemaVersion, SourceIdentity, SourceTier,
};

use crate::evidence::{EvidenceGate, EvidenceLedger};
use crate::facts::ColdGuardId;
use crate::frontier::{CausalIndex, CausalOrder, Frontier};
use crate::slice::{Binding, SliceIndex};
use crate::specification::{
    AttributeKey, EventSelector, RequiredSource, RetirementPolicy, SpecificationConversionError,
    SpecificationId, SpecificationIdentity, SpecificationVersion,
};

/// Named proof rule under which an absence violation is invariant across linearizations.
pub const INVARIANCE_PROOF_RULE_ID: &str = "absence.presence-above-tier";
/// Version of [`INVARIANCE_PROOF_RULE_ID`].
pub const INVARIANCE_PROOF_RULE_VERSION: &str = "v1";

/// An absence obligation over one Binding attribute and an open interval.
///
/// The Core's own form of CAT-2, sibling to
/// [`TaintSpecification`](crate::taint::TaintSpecification) and deliberately a
/// distinct type: the interval premise and the NoObligation path have no home
/// in CAT-4's table, and a shared struct with a flag would invite folding the
/// two tables into one `assess`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AbsenceSpecification {
    identity: SpecificationIdentity,
    binding_attribute: AttributeKey,
    after: EventSelector,
    until: EventSelector,
    veto: EventSelector,
    required_sources: BTreeSet<RequiredSource>,
    schema_version: SchemaVersion,
    cold_guards: BTreeSet<ColdGuardId>,
    activation_barrier: Option<ActivationBarrier>,
    retirement_policy: RetirementPolicy,
}

impl AbsenceSpecification {
    /// Declares an absence obligation parameterised by one Event attribute.
    #[must_use]
    pub fn new(
        identity: SpecificationIdentity,
        binding_attribute: AttributeKey,
        after: EventSelector,
        until: EventSelector,
        veto: EventSelector,
        required_sources: BTreeSet<RequiredSource>,
        schema_version: SchemaVersion,
    ) -> Self {
        Self {
            identity,
            binding_attribute,
            after,
            until,
            veto,
            required_sources,
            schema_version,
            cold_guards: BTreeSet::new(),
            activation_barrier: None,
            retirement_policy: RetirementPolicy::NotRequired,
        }
    }

    /// Returns what this Specification declares about a never-closing Scope.
    ///
    /// CAT-2's catalogued Pattern declares [`RetirementPolicy::NotRequired`] —
    /// the Scope closes at `until`. Carried so clause (d0) reads the Pattern's
    /// own declaration rather than a family check.
    #[must_use]
    pub const fn retirement_policy(&self) -> RetirementPolicy {
        self.retirement_policy
    }

    fn with_retirement_policy(mut self, retirement_policy: RetirementPolicy) -> Self {
        self.retirement_policy = retirement_policy;
        self
    }

    /// Declares the activation barrier this Specification is warming behind.
    ///
    /// `None` — the default — is a Specification that declares no barrier.
    /// `TryFrom<&AbsenceIr>` cannot carry one: `AbsenceIr` has no such field.
    #[must_use]
    pub fn with_activation_barrier(mut self, activation_barrier: ActivationBarrier) -> Self {
        self.activation_barrier = Some(activation_barrier);
        self
    }

    /// Returns the activation barrier, when this Specification declares one.
    #[must_use]
    pub const fn activation_barrier(&self) -> Option<&ActivationBarrier> {
        self.activation_barrier.as_ref()
    }

    /// Returns the declared cold-path guards.
    ///
    /// Empty for every CAT-2 Specification: the catalogue declares
    /// `EvaluationTier::Hot` and zero fact requests.
    #[must_use]
    pub const fn cold_guards(&self) -> &BTreeSet<ColdGuardId> {
        &self.cold_guards
    }

    /// Returns the Specification identity.
    #[must_use]
    pub const fn identity(&self) -> &SpecificationIdentity {
        &self.identity
    }

    /// Returns the attribute whose value names a Binding.
    #[must_use]
    pub const fn binding_attribute(&self) -> &AttributeKey {
        &self.binding_attribute
    }

    /// Returns the selector for Events that open the interval.
    #[must_use]
    pub const fn after(&self) -> &EventSelector {
        &self.after
    }

    /// Returns the selector for Events that close the interval and raise the obligation.
    #[must_use]
    pub const fn until(&self) -> &EventSelector {
        &self.until
    }

    /// Returns the selector for Events prohibited inside `(after, until]`.
    #[must_use]
    pub const fn veto(&self) -> &EventSelector {
        &self.veto
    }

    /// Returns the sources the conclusion is not trustworthy without.
    #[must_use]
    pub const fn required_sources(&self) -> &BTreeSet<RequiredSource> {
        &self.required_sources
    }

    /// Returns the required source identities.
    #[must_use]
    pub fn required_source_identities(&self) -> BTreeSet<SourceIdentity> {
        self.required_sources
            .iter()
            .map(|required| required.source().clone())
            .collect()
    }

    /// Returns the producer schema version this Specification is written against.
    #[must_use]
    pub const fn schema_version(&self) -> &SchemaVersion {
        &self.schema_version
    }

    /// Returns the Binding an Event belongs to, when it carries the free variable.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        sentinel_compile::ir::binding_of(&self.binding_attribute, event)
    }
}

/// Lowers a catalogued CAT-2 instance into the Core's own Specification.
///
/// Barrier-free: `AbsenceIr` has no activation-barrier field.
impl TryFrom<&AbsenceIr> for AbsenceSpecification {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &AbsenceIr) -> Result<Self, Self::Error> {
        let id = SpecificationId::new(lowered.specification_id.clone())
            .map_err(|_| SpecificationConversionError::BlankSpecificationId)?;
        let version = SpecificationVersion::new(lowered.specification_version.clone())
            .map_err(|_| SpecificationConversionError::BlankSpecificationVersion)?;
        let binding_attribute = AttributeKey::new(lowered.binding_attribute.clone())
            .map_err(|_| SpecificationConversionError::BlankBindingAttribute)?;
        Ok(Self::new(
            SpecificationIdentity::new(id, version),
            binding_attribute,
            EventSelector::new(
                lowered.after.kind.clone(),
                lowered.after.required_attributes.clone(),
            ),
            EventSelector::new(
                lowered.until.kind.clone(),
                lowered.until.required_attributes.clone(),
            ),
            EventSelector::new(
                lowered.veto.kind.clone(),
                lowered.veto.required_attributes.clone(),
            ),
            lowered
                .required_sources
                .iter()
                .map(|(source, tier)| RequiredSource::new(source.clone(), *tier))
                .collect(),
            lowered.schema_version.clone(),
        )
        .with_retirement_policy(RetirementPolicy::from(lowered.retirement)))
    }
}

/// The decision table's collapsed Open/Closed input.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GateState {
    /// Every required source is covered, contiguous, healthy and caught up.
    Open,
    /// At least one gate disjunct fired, so nothing may settle.
    Closed,
}

/// Whether some `after` Event reaches this `until` (the interval opened).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IntervalPremise {
    /// Some Event matching `after` reaches `until` in every admissible linearization.
    Opened,
    /// No such Event reaches it.
    NeverOpened,
}

/// Whether a falsifying veto lies inside `(after, until]`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausalPast {
    /// Some Event matching the veto selector lies in the open interval.
    CarriesFalsifier,
    /// No such Event lies in the interval in any admissible linearization.
    Clean,
}

/// Whether the Slice holds an anchor-tier Event of the falsifying kind.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnchorWitness {
    /// An anchor-tier source witnessed the falsifying kind in this Slice.
    Present,
    /// Only contributing-tier sources did.
    Absent,
}

/// Whether an `after` or a `veto` is Concurrent with `until`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ConcurrentFalsifier {
    /// Some delimiter or veto is unordered with respect to `until`.
    Present,
    /// None is.
    Absent,
}

/// Whether Evidence Coverage has closed for every required source.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CoverageState {
    /// Every required source's Watermark covers its epoch.
    Settled,
    /// At least one does not, so more evidence is outstanding.
    Open,
}

/// What the evidence says about one `until` Event's open interval.
///
/// Six independent facts a monitor computes and this table consumes. Distinct
/// two-variant types rather than booleans (ADR-0031 clause 1): a transposition
/// is `error[E0308]`, and the exhaustive suite enumerates all sixty-four
/// assignments — including combinations a real traversal would never produce —
/// so the `Satisfy` row stays unreachable under a present falsifier or a
/// never-opened settled interval.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct AbsenceObservation {
    /// Whether the pre-obligation evidence gate is open.
    pub gate: GateState,
    /// Whether some `after` reaches this `until`.
    pub interval: IntervalPremise,
    /// Whether a falsifying veto lies inside `(after, until]`.
    pub causal_past: CausalPast,
    /// Whether the Slice holds an anchor-tier Event of the falsifying kind.
    pub anchor_witness: AnchorWitness,
    /// Whether an `after` or a `veto` is Concurrent with `until`.
    pub concurrent: ConcurrentFalsifier,
    /// Whether Evidence Coverage has closed.
    pub coverage: CoverageState,
}

impl AbsenceObservation {
    /// Records one `until` Event's interval facts.
    #[must_use]
    pub const fn new(
        gate: GateState,
        interval: IntervalPremise,
        causal_past: CausalPast,
        anchor_witness: AnchorWitness,
        concurrent: ConcurrentFalsifier,
        coverage: CoverageState,
    ) -> Self {
        Self {
            gate,
            interval,
            causal_past,
            anchor_witness,
            concurrent,
            coverage,
        }
    }
}

/// The conclusion reached for one `until` Event, or for one Binding.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AbsenceOutcome {
    /// No veto lies in the open interval, and Evidence Coverage is closed.
    Satisfy,
    /// The evidence is open: more of it could still settle this either way.
    InconclusivePrefix,
    /// An `after` or a `veto` is Concurrent with `until` and cannot be ordered.
    InconclusiveObservational,
    /// A veto lies in the open interval, witnessed at anchor tier.
    Violate,
}

/// What a **settled falsifying witness** can conclude, once tier is consulted.
///
/// Deliberately narrower than [`AbsenceOutcome`]: `Satisfy` is not a member.
/// Inherited from CAT-4 — see [`crate::taint::ConvictedOutcome`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ConvictedOutcome {
    /// The witness is not authoritative, so the conclusion does not settle.
    InconclusivePrefix,
    /// The witness is anchor-tier, so the violation settles.
    Violate,
}

impl ConvictedOutcome {
    /// Widens a settled-witness conclusion to the full outcome lattice.
    #[must_use]
    pub const fn widen(self) -> AbsenceOutcome {
        match self {
            Self::InconclusivePrefix => AbsenceOutcome::InconclusivePrefix,
            Self::Violate => AbsenceOutcome::Violate,
        }
    }
}

impl From<ConvictedOutcome> for AbsenceOutcome {
    fn from(convicted: ConvictedOutcome) -> Self {
        convicted.widen()
    }
}

/// Applies the ordered decision table to one `until` Event's observation.
///
/// Returns [`None`] when this `until` raises no obligation (coverage settled
/// and the interval never opened). That is **not** [`AbsenceOutcome::Satisfy`].
#[must_use]
pub const fn decide(observation: AbsenceObservation) -> Option<AbsenceOutcome> {
    // Row 0. The gate runs before obligation analysis and wins outright.
    if matches!(observation.gate, GateState::Closed) {
        return Some(AbsenceOutcome::InconclusivePrefix);
    }
    // Rows 1 and 2. Presence inside the interval is settled here; tier only
    // chooses among non-Satisfy outcomes.
    if matches!(observation.causal_past, CausalPast::CarriesFalsifier) {
        return Some(convict(observation.anchor_witness).widen());
    }
    // Row 3. Concurrent after∥until or veto∥until.
    if matches!(observation.concurrent, ConcurrentFalsifier::Present) {
        return Some(AbsenceOutcome::InconclusiveObservational);
    }
    // Rows 4 and 5. Never opened: NoObligation when settled, prefix while open.
    if matches!(observation.interval, IntervalPremise::NeverOpened) {
        return match observation.coverage {
            CoverageState::Settled => None,
            CoverageState::Open => Some(AbsenceOutcome::InconclusivePrefix),
        };
    }
    // Row 6. Opened, clean, coverage closed — ADR-0025's licence at `until`.
    if matches!(observation.coverage, CoverageState::Settled) {
        return Some(AbsenceOutcome::Satisfy);
    }
    // Row 7.
    Some(AbsenceOutcome::InconclusivePrefix)
}

/// Chooses between the two conclusions a present falsifying witness supports.
const fn convict(anchor_witness: AnchorWitness) -> ConvictedOutcome {
    match anchor_witness {
        AnchorWitness::Present => ConvictedOutcome::Violate,
        AnchorWitness::Absent => ConvictedOutcome::InconclusivePrefix,
    }
}

/// Folds one Binding's per-`until` outcomes into the Binding's conclusion.
///
/// `None` when the Binding raised no obligation — every `until` was skipped, or
/// none occurred. Never a vacuous `Satisfy`.
#[must_use]
pub fn aggregate(outcomes: impl IntoIterator<Item = AbsenceOutcome>) -> Option<AbsenceOutcome> {
    outcomes.into_iter().max()
}

/// What the monitor concluded about one Binding, and the evidence it cites.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AbsenceAssessment {
    binding: Binding,
    outcome: AbsenceOutcome,
    concurrent_pairs: BTreeSet<(EventId, EventId)>,
    falsifying_ancestors: BTreeSet<EventId>,
    anchored_events: BTreeSet<EventId>,
    witness_events: BTreeSet<EventId>,
}

impl AbsenceAssessment {
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
    pub const fn outcome(&self) -> AbsenceOutcome {
        self.outcome
    }

    /// Returns every `(falsifier_or_after, until)` pair the evidence leaves unordered.
    #[must_use]
    pub const fn concurrent_pairs(&self) -> &BTreeSet<(EventId, EventId)> {
        &self.concurrent_pairs
    }

    /// Returns a deterministic description of the unordered pairs.
    #[must_use]
    pub fn concurrency_detail(&self) -> String {
        self.concurrent_pairs
            .iter()
            .map(|(left, until)| format!("{left} || {until}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Returns veto Events that lie inside an open `(after, until]` interval.
    #[must_use]
    pub const fn falsifying_ancestors(&self) -> &BTreeSet<EventId> {
        &self.falsifying_ancestors
    }

    /// Returns the anchor-tier Events the conclusion rests on.
    #[must_use]
    pub const fn anchored_events(&self) -> &BTreeSet<EventId> {
        &self.anchored_events
    }

    /// Returns the Events a decisive violation witness names.
    #[must_use]
    pub const fn witness_events(&self) -> &BTreeSet<EventId> {
        &self.witness_events
    }
}

/// Returns whether `veto` lies in the open-closed interval `(after, until]`.
///
/// `after` must already reach `until` (the interval is open under that
/// delimiter). The close itself is included: a veto that *is* the `until`
/// Event counts. Reachability is strict, so `after` itself is excluded.
fn veto_in_interval(index: &CausalIndex, after: &EventId, veto: &EventId, until: &EventId) -> bool {
    if veto == until {
        return true;
    }
    index.reaches(after, veto) && index.reaches(veto, until)
}

/// Produces the six facts [`decide`] consumes for one `until` Event.
#[must_use]
pub fn observe(
    specification: &AbsenceSpecification,
    until: &Event,
    slice: &[&Event],
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> AbsenceObservation {
    let gate_state = if gate.is_open() {
        GateState::Open
    } else {
        GateState::Closed
    };
    let coverage = if gate.watermarks_cover_epochs() {
        CoverageState::Settled
    } else {
        CoverageState::Open
    };

    let until_id = until.id();
    let mut interval = IntervalPremise::NeverOpened;
    let mut concurrent = ConcurrentFalsifier::Absent;
    let mut causal_past = CausalPast::Clean;

    for event in slice {
        if specification.after().matches(event) {
            match index.classify(&event.id(), &until_id) {
                CausalOrder::Before => interval = IntervalPremise::Opened,
                CausalOrder::Concurrent => concurrent = ConcurrentFalsifier::Present,
                CausalOrder::After => {}
            }
        }
    }

    for event in slice {
        if !specification.veto().matches(event) {
            continue;
        }
        let veto_id = event.id();
        match index.classify(&veto_id, &until_id) {
            CausalOrder::Concurrent => concurrent = ConcurrentFalsifier::Present,
            CausalOrder::Before | CausalOrder::After => {}
        }
        if matches!(interval, IntervalPremise::Opened) {
            for after in slice.iter().filter(|e| specification.after().matches(e)) {
                if index.reaches(&after.id(), &until_id)
                    && veto_in_interval(index, &after.id(), &veto_id, &until_id)
                {
                    causal_past = CausalPast::CarriesFalsifier;
                }
            }
        }
    }

    let anchor_witness = if slice.iter().any(|event| {
        specification.veto().matches_kind(event)
            && ledger.tier_of(&event.source()) == Some(SourceTier::Anchor)
    }) {
        AnchorWitness::Present
    } else {
        AnchorWitness::Absent
    };

    AbsenceObservation::new(
        gate_state,
        interval,
        causal_past,
        anchor_witness,
        concurrent,
        coverage,
    )
}

/// Evaluates the Specification over every Binding an `until` Event creates.
///
/// Bindings are created by Events carrying the Binding attribute. An empty
/// `until` set, or a set whose every member is NoObligation, produces no
/// Binding assessment and no Verdict.
#[must_use]
pub fn assess(
    specification: &AbsenceSpecification,
    index: &CausalIndex,
    frontier: &Frontier,
    slices: &SliceIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> Vec<AbsenceAssessment> {
    let mut assessments = Vec::new();
    for binding in slices.bindings() {
        let Some(members) = slices.members(binding) else {
            continue;
        };
        let slice: Vec<&Event> = members.iter().filter_map(|id| frontier.event(id)).collect();
        let untils: Vec<&Event> = slices
            .obligation_raising(binding)
            .into_iter()
            .flatten()
            .filter_map(|id| frontier.event(id))
            .collect();
        if untils.is_empty() {
            continue;
        }
        if let Some(assessment) = assess_binding(
            specification,
            index,
            gate,
            ledger,
            binding.clone(),
            &slice,
            &untils,
        ) {
            assessments.push(assessment);
        }
    }
    assessments.sort_by(|left, right| left.binding.cmp(&right.binding));
    assessments
}

fn assess_binding(
    specification: &AbsenceSpecification,
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
    binding: Binding,
    slice: &[&Event],
    untils: &[&Event],
) -> Option<AbsenceAssessment> {
    let vetos: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.veto().matches(event))
        .collect();
    let afters: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.after().matches(event))
        .collect();

    let anchor_candidates: BTreeSet<EventId> = slice
        .iter()
        .filter(|event| {
            specification.veto().matches_kind(event)
                && ledger.tier_of(&event.source()) == Some(SourceTier::Anchor)
        })
        .map(|event| event.id())
        .collect();

    let mut outcomes = Vec::with_capacity(untils.len());
    let mut concurrent_pairs = BTreeSet::new();
    let mut falsifying_ancestors = BTreeSet::new();
    let mut anchored_events = BTreeSet::new();
    let mut witness_events = BTreeSet::new();

    for until in untils {
        let observation = observe(specification, until, slice, index, gate, ledger);
        let Some(outcome) = decide(observation) else {
            continue;
        };
        outcomes.push(outcome);

        let until_id = until.id();
        for event in &afters {
            if matches!(
                index.classify(&event.id(), &until_id),
                CausalOrder::Concurrent
            ) {
                concurrent_pairs.insert((event.id(), until_id.clone()));
            }
        }
        for event in &vetos {
            let veto_id = event.id();
            match index.classify(&veto_id, &until_id) {
                CausalOrder::Concurrent => {
                    concurrent_pairs.insert((veto_id.clone(), until_id.clone()));
                }
                CausalOrder::Before | CausalOrder::After => {}
            }
            for after in &afters {
                if index.reaches(&after.id(), &until_id)
                    && veto_in_interval(index, &after.id(), &veto_id, &until_id)
                {
                    falsifying_ancestors.insert(veto_id.clone());
                }
            }
        }

        match outcome {
            AbsenceOutcome::Violate => {
                anchored_events.extend(anchor_candidates.iter().cloned());
                witness_events.insert(until_id);
                witness_events.extend(anchor_candidates.iter().cloned());
            }
            AbsenceOutcome::Satisfy => {
                // An Authoritative Satisfy must cite an anchor-tier Event; the
                // until is the Event-local Scope close the conclusion rests on.
                if ledger.tier_of(&until.source()) == Some(SourceTier::Anchor) {
                    anchored_events.insert(until_id);
                }
            }
            AbsenceOutcome::InconclusivePrefix | AbsenceOutcome::InconclusiveObservational => {}
        }
    }

    let outcome = aggregate(outcomes)?;

    if !gate.is_open() {
        anchored_events.clear();
        witness_events.clear();
    }
    if outcome != AbsenceOutcome::Violate {
        witness_events.clear();
    }
    if matches!(
        outcome,
        AbsenceOutcome::Satisfy | AbsenceOutcome::InconclusivePrefix
    ) {
        concurrent_pairs.clear();
    }

    Some(AbsenceAssessment {
        binding,
        outcome,
        concurrent_pairs,
        falsifying_ancestors,
        anchored_events,
        witness_events,
    })
}
