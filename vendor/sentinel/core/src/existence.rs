//! The decision table for one CAT-3 existence obligation over a Between Scope.
//!
//! CAT-3 `existence.before-scope-close` states: for every Event matching the
//! `open` selector in a Binding's Slice, **at least one** Event matching the
//! `required` selector lies in the open-closed interval `(open, close]` before
//! the Scope closes at `close`. That is an existence claim about a *presence*
//! inside a causally delimited Scope that **can fail to close**, which is why
//! this family declares [`crate::specification::RetirementPolicy::Deadline`] and
//! why O-03's retirement path is reachable from catalogue data.
//!
//! # Missing evidence is never a pass
//!
//! - No `open` that raises an obligation → no Verdict (never vacuous Satisfy).
//! - `open` present, no `close` yet, not retired → prefix-open.
//! - `close` present, `required` absent, Coverage open → prefix-open.
//! - `close` present, `required` absent, Coverage settled, required-source only
//!   contributing → prefix-open (no contributing-only Violate).
//! - Retirement is absorbing at assessment: a retired Binding always becomes
//!   [`ExistenceConclusion::Retired`] / structural Inconclusive via
//!   [`crate::precedence::RetiredAssessment`], even if Coverage would later
//!   license Satisfy or Violate. Later Coverage must not invent a pass or fail
//!   after O-03 retirement (#210 pin 2).
//! - Demotion of an *already emitted* Satisfy/Violate is refused on
//!   [`crate::evaluator::Input::Retire`] by clause (d2)
//!   (`BindingAlreadyEmitted`), not by leaving retirement off the assess path
//!   (ADR-0025; #210 AMEND 2).
//!
//! # The table (per `open` Event; first match wins)
//!
//! | # | condition | outcome |
//! |---|---|---|
//! | 0 | evidence gate closed | prefix-open |
//! | 1 | Concurrent open/required/close where order matters | observational |
//! | 2 | no `close` yet | prefix-open |
//! | 3 | `close` present; `required` in `(open, close]`; Coverage settled | Satisfy |
//! | 4 | `close` present; `required` absent; Coverage settled; anchor silence | Violate |
//! | 5 | `close` present; `required` absent; contributing-tier only / Coverage open | prefix-open |
//!
//! Binding aggregate is the Ord maximum over opens — universal conjunction —
//! matching sibling hot families (`Violate > Inconclusive(observational) > …`).
//!
//! # Retirement vs Scope-close
//!
//! [`assess`] takes [`crate::slice::SliceLifecycles`]. Any Binding with a
//! recorded retirement becomes [`ExistenceConclusion::Retired`] — same
//! absorbing shape as precedence — regardless of what the open decision table
//! would have said.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{Event, EventId, SourceTier};

use crate::evidence::{EvidenceGate, EvidenceLedger};
use crate::facts::ColdGuardId;
use crate::frontier::{CausalIndex, CausalOrder, Frontier};
use crate::precedence::RetiredAssessment;
use crate::slice::{Binding, Retirement, SliceIndex, SliceLifecycles};
use crate::specification::{
    AttributeKey, EventSelector, RequiredSource, RetirementPolicy, SpecificationConversionError,
    SpecificationId, SpecificationIdentity, SpecificationVersion,
};
use sentinel_compile::ir::ExistenceIr;
use sentinel_model::{ActivationBarrier, SchemaVersion};

/// Named proof rule under which an existence violation is invariant across linearizations.
pub const INVARIANCE_PROOF_RULE_ID: &str = "existence.absence-above-tier";
/// Version of [`INVARIANCE_PROOF_RULE_ID`].
pub const INVARIANCE_PROOF_RULE_VERSION: &str = "v1";

/// An existence obligation over one Binding attribute and a Between Scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExistenceSpecification {
    identity: SpecificationIdentity,
    binding_attribute: AttributeKey,
    open: EventSelector,
    required: EventSelector,
    close: EventSelector,
    required_sources: BTreeSet<RequiredSource>,
    schema_version: SchemaVersion,
    cold_guards: BTreeSet<ColdGuardId>,
    activation_barrier: Option<ActivationBarrier>,
    retirement_policy: RetirementPolicy,
}

impl ExistenceSpecification {
    /// Declares an existence obligation parameterised by one Event attribute.
    #[must_use]
    pub fn new(
        identity: SpecificationIdentity,
        binding_attribute: AttributeKey,
        open: EventSelector,
        required: EventSelector,
        close: EventSelector,
        required_sources: BTreeSet<RequiredSource>,
        schema_version: SchemaVersion,
    ) -> Self {
        Self {
            identity,
            binding_attribute,
            open,
            required,
            close,
            required_sources,
            schema_version,
            cold_guards: BTreeSet::new(),
            activation_barrier: None,
            retirement_policy: RetirementPolicy::NotRequired,
        }
    }

    /// Declares what this Specification says about a Scope that may never close.
    ///
    /// Catalogued CAT-3 declares [`RetirementPolicy::Deadline`]. Hand-built
    /// fixtures that exercise the mechanism may set Deadline or Inactivity.
    #[must_use]
    pub const fn with_retirement_policy(mut self, retirement_policy: RetirementPolicy) -> Self {
        self.retirement_policy = retirement_policy;
        self
    }

    /// Returns what this Specification declares about a never-closing Scope.
    #[must_use]
    pub const fn retirement_policy(&self) -> RetirementPolicy {
        self.retirement_policy
    }

    /// Declares the activation barrier this Specification is warming behind.
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

    /// Returns the declared cold-path guards (empty for catalogued CAT-3).
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

    /// Returns the selector for Events that open the Scope and raise the obligation.
    #[must_use]
    pub const fn open(&self) -> &EventSelector {
        &self.open
    }

    /// Returns the selector for Events that must occur inside `(open, close]`.
    #[must_use]
    pub const fn required(&self) -> &EventSelector {
        &self.required
    }

    /// Returns the selector for Events that close the Scope.
    #[must_use]
    pub const fn close(&self) -> &EventSelector {
        &self.close
    }

    /// Returns the sources the conclusion is not trustworthy without.
    #[must_use]
    pub const fn required_sources(&self) -> &BTreeSet<RequiredSource> {
        &self.required_sources
    }

    /// Returns the required source identities.
    #[must_use]
    pub fn required_source_identities(&self) -> BTreeSet<sentinel_model::SourceIdentity> {
        self.required_sources
            .iter()
            .map(|required| required.source().clone())
            .collect()
    }

    /// Returns the producer schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &SchemaVersion {
        &self.schema_version
    }

    /// Reads the Binding value this Specification would file an Event under.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        use sentinel_compile::ir::binding_of;
        binding_of(&self.binding_attribute, event)
    }
}

impl TryFrom<&ExistenceIr> for ExistenceSpecification {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &ExistenceIr) -> Result<Self, Self::Error> {
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
                lowered.open.kind.clone(),
                lowered.open.required_attributes.clone(),
            ),
            EventSelector::new(
                lowered.required.kind.clone(),
                lowered.required.required_attributes.clone(),
            ),
            EventSelector::new(
                lowered.close.kind.clone(),
                lowered.close.required_attributes.clone(),
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

/// Whether a matching `close` Event reaches after this `open`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScopeClose {
    /// Some Event matching `close` is After `open` in every admissible linearization.
    Present,
    /// No such Event closes the Scope yet.
    Absent,
}

/// Whether a matching `required` Event lies in `(open, close]`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RequiredPresence {
    /// Some Event matching `required` lies in the open-closed interval.
    Present,
    /// No such Event lies in the interval.
    Absent,
}

/// Whether open/required/close concurrency blocks ordering.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ConcurrentUncertainty {
    /// Some delimiter or required Event is unordered where order matters.
    Present,
    /// None is.
    Absent,
}

/// Whether a required-source silence can anchor a Violate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnchorSilence {
    /// A required source is Anchor tier, so watermark-complete silence convicts.
    Available,
    /// Only contributing-tier required sources — silence cannot convict.
    Unavailable,
}

/// Whether Evidence Coverage has closed for every required source.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CoverageState {
    /// Every required source's Watermark covers its epoch.
    Settled,
    /// At least one does not, so more evidence is outstanding.
    Open,
}

/// What the evidence says about one `open` Event's Between Scope.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct ExistenceObservation {
    /// Whether the pre-obligation evidence gate is open.
    pub gate: GateState,
    /// Whether a matching close is After this open.
    pub scope_close: ScopeClose,
    /// Whether a matching required lies in `(open, close]`.
    pub required: RequiredPresence,
    /// Whether concurrency blocks ordering.
    pub concurrent: ConcurrentUncertainty,
    /// Whether required-source silence can anchor a Violate.
    pub anchor_silence: AnchorSilence,
    /// Whether Evidence Coverage has closed.
    pub coverage: CoverageState,
}

impl ExistenceObservation {
    /// Records one `open` Event's Between-Scope facts.
    #[must_use]
    pub const fn new(
        gate: GateState,
        scope_close: ScopeClose,
        required: RequiredPresence,
        concurrent: ConcurrentUncertainty,
        anchor_silence: AnchorSilence,
        coverage: CoverageState,
    ) -> Self {
        Self {
            gate,
            scope_close,
            required,
            concurrent,
            anchor_silence,
            coverage,
        }
    }
}

/// The conclusion reached for one `open` Event, or for one Binding.
///
/// Ord is the Binding-aggregate dominance maximum (universal conjunction):
/// `Violate > InconclusiveObservational > InconclusivePrefix > Satisfy`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ExistenceOutcome {
    /// A required Event lies in `(open, close]` and Coverage is closed.
    Satisfy,
    /// The evidence is open: more of it could still settle this either way.
    InconclusivePrefix,
    /// An open/required/close concurrency the evidence cannot order.
    InconclusiveObservational,
    /// No required Event in `(open, close]`, Coverage closed, anchor silence.
    Violate,
}

/// Applies the ordered decision table to one `open` Event's observation.
#[must_use]
pub const fn decide(observation: ExistenceObservation) -> ExistenceOutcome {
    if matches!(observation.gate, GateState::Closed) {
        return ExistenceOutcome::InconclusivePrefix;
    }
    if matches!(observation.concurrent, ConcurrentUncertainty::Present) {
        return ExistenceOutcome::InconclusiveObservational;
    }
    if matches!(observation.scope_close, ScopeClose::Absent) {
        return ExistenceOutcome::InconclusivePrefix;
    }
    if matches!(observation.required, RequiredPresence::Present) {
        return match observation.coverage {
            CoverageState::Settled => ExistenceOutcome::Satisfy,
            CoverageState::Open => ExistenceOutcome::InconclusivePrefix,
        };
    }
    // required Absent, close Present
    match (observation.coverage, observation.anchor_silence) {
        (CoverageState::Settled, AnchorSilence::Available) => ExistenceOutcome::Violate,
        (CoverageState::Settled, AnchorSilence::Unavailable) | (CoverageState::Open, _) => {
            ExistenceOutcome::InconclusivePrefix
        }
    }
}

/// Folds one Binding's per-`open` outcomes into the Binding's conclusion.
#[must_use]
pub fn aggregate(outcomes: impl IntoIterator<Item = ExistenceOutcome>) -> Option<ExistenceOutcome> {
    outcomes.into_iter().max()
}

/// What the monitor concluded about one Binding whose Scope is still open.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ExistenceAssessment {
    pub(crate) binding: Binding,
    pub(crate) outcome: ExistenceOutcome,
    pub(crate) concurrent_pairs: BTreeSet<(EventId, EventId)>,
    pub(crate) discharged_by: BTreeMap<EventId, EventId>,
    pub(crate) unmet_opens: BTreeSet<EventId>,
    pub(crate) anchored_events: BTreeSet<EventId>,
    pub(crate) witness_events: BTreeSet<EventId>,
}

impl ExistenceAssessment {
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
    pub const fn outcome(&self) -> ExistenceOutcome {
        self.outcome
    }

    /// Returns every unordered pair the evidence leaves Concurrent.
    #[must_use]
    pub const fn concurrent_pairs(&self) -> &BTreeSet<(EventId, EventId)> {
        &self.concurrent_pairs
    }

    /// Returns open → required witness edges that discharge an obligation.
    #[must_use]
    pub const fn discharged_by(&self) -> &BTreeMap<EventId, EventId> {
        &self.discharged_by
    }

    /// Returns open Events whose required obligation is unmet.
    #[must_use]
    pub const fn unmet_opens(&self) -> &BTreeSet<EventId> {
        &self.unmet_opens
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

    /// Returns a deterministic description of the unordered pairs.
    #[must_use]
    pub fn concurrency_detail(&self) -> String {
        self.concurrent_pairs
            .iter()
            .map(|(left, right)| format!("{left} || {right}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Open or retired conclusion for one Binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExistenceConclusion {
    /// Scope still open for assessment (no retirement recorded).
    Open(ExistenceAssessment),
    /// Periphery retired this Binding — absorbing; Coverage cannot later Satisfy/Violate.
    Retired(RetiredAssessment, Retirement),
}

impl ExistenceConclusion {
    /// Returns the Binding this conclusion is about.
    #[must_use]
    pub const fn key(&self) -> &Binding {
        match self {
            Self::Open(assessment) => assessment.key(),
            Self::Retired(assessment, _) => assessment.key(),
        }
    }
}

/// Returns whether `required` lies in the open-closed interval `(open, close]`.
fn required_in_interval(
    index: &CausalIndex,
    open: &EventId,
    required: &EventId,
    close: &EventId,
) -> bool {
    if required == close {
        return true;
    }
    index.reaches(open, required) && index.reaches(required, close)
}

/// Produces the facts [`decide`] consumes for one `open` Event.
#[must_use]
pub fn observe(
    specification: &ExistenceSpecification,
    open: &Event,
    slice: &[&Event],
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> ExistenceObservation {
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

    let open_id = open.id();
    let mut scope_close = ScopeClose::Absent;
    let mut concurrent = ConcurrentUncertainty::Absent;
    let mut required_presence = RequiredPresence::Absent;

    let closes: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.close().matches(event))
        .collect();
    let requireds: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.required().matches(event))
        .collect();

    for close in &closes {
        match index.classify(&open_id, &close.id()) {
            CausalOrder::Before => scope_close = ScopeClose::Present,
            CausalOrder::Concurrent => concurrent = ConcurrentUncertainty::Present,
            CausalOrder::After => {}
        }
    }

    for required in &requireds {
        match index.classify(&open_id, &required.id()) {
            CausalOrder::Concurrent => concurrent = ConcurrentUncertainty::Present,
            CausalOrder::Before | CausalOrder::After => {}
        }
    }

    if matches!(scope_close, ScopeClose::Present) {
        for close in &closes {
            if !matches!(index.classify(&open_id, &close.id()), CausalOrder::Before) {
                continue;
            }
            for required in &requireds {
                match index.classify(&required.id(), &close.id()) {
                    CausalOrder::Concurrent => concurrent = ConcurrentUncertainty::Present,
                    CausalOrder::Before | CausalOrder::After => {}
                }
                if required_in_interval(index, &open_id, &required.id(), &close.id()) {
                    required_presence = RequiredPresence::Present;
                }
            }
        }
    }

    let anchor_silence = if specification
        .required_source_identities()
        .iter()
        .any(|source| ledger.tier_of(source) == Some(SourceTier::Anchor))
    {
        AnchorSilence::Available
    } else {
        AnchorSilence::Unavailable
    };

    ExistenceObservation::new(
        gate_state,
        scope_close,
        required_presence,
        concurrent,
        anchor_silence,
        coverage,
    )
}

/// Evaluates the Specification over every Binding an `open` Event creates.
#[must_use]
pub fn assess(
    specification: &ExistenceSpecification,
    index: &CausalIndex,
    frontier: &Frontier,
    slices: &SliceIndex,
    lifecycles: &SliceLifecycles,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> Vec<ExistenceConclusion> {
    let mut conclusions = Vec::new();
    for binding in slices.bindings() {
        let retirement = lifecycles.retirement_of(binding);
        let membership = lifecycles.members_in_scope(binding, slices, frontier);
        let slice: Vec<&Event> = membership
            .in_scope()
            .iter()
            .filter_map(|id| frontier.event(id))
            .collect();
        let opens: Vec<&Event> = slices
            .obligation_raising(binding)
            .into_iter()
            .flatten()
            .filter(|id| membership.in_scope().contains(*id))
            .filter_map(|id| frontier.event(id))
            .collect();
        if retirement.is_none() && opens.is_empty() {
            continue;
        }
        let assessment = match assess_binding(
            specification,
            index,
            gate,
            ledger,
            binding.clone(),
            &slice,
            &opens,
        ) {
            Some(assessment) => assessment,
            None if retirement.is_some() => ExistenceAssessment {
                binding: binding.clone(),
                outcome: ExistenceOutcome::InconclusivePrefix,
                concurrent_pairs: BTreeSet::new(),
                discharged_by: BTreeMap::new(),
                unmet_opens: BTreeSet::new(),
                anchored_events: BTreeSet::new(),
                witness_events: BTreeSet::new(),
            },
            None => continue,
        };

        // Retirement is absorbing (pin 2 / precedence parity). Demotion of an
        // already-emitted Satisfy/Violate is clause (d2) on Input::Retire, not
        // a hole that lets later Coverage un-retire through Open(Satisfy).
        conclusions.push(match retirement {
            None => ExistenceConclusion::Open(assessment),
            Some(retired) => ExistenceConclusion::Retired(
                RetiredAssessment::from_parts(
                    assessment.key().clone(),
                    assessment.discharged_by().clone(),
                    assessment.unmet_opens().clone(),
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

fn assess_binding(
    specification: &ExistenceSpecification,
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
    binding: Binding,
    slice: &[&Event],
    opens: &[&Event],
) -> Option<ExistenceAssessment> {
    let requireds: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.required().matches(event))
        .collect();
    let closes: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.close().matches(event))
        .collect();

    let mut outcomes = Vec::with_capacity(opens.len());
    let mut concurrent_pairs = BTreeSet::new();
    let mut discharged_by = BTreeMap::new();
    let mut unmet_opens = BTreeSet::new();
    let mut anchored_events = BTreeSet::new();
    let mut witness_events = BTreeSet::new();

    for open in opens {
        let observation = observe(specification, open, slice, index, gate, ledger);
        let outcome = decide(observation);
        outcomes.push(outcome);

        let open_id = open.id();
        for close in &closes {
            if matches!(
                index.classify(&open_id, &close.id()),
                CausalOrder::Concurrent
            ) {
                concurrent_pairs.insert((open_id.clone(), close.id()));
            }
        }
        for required in &requireds {
            if matches!(
                index.classify(&open_id, &required.id()),
                CausalOrder::Concurrent
            ) {
                concurrent_pairs.insert((open_id.clone(), required.id()));
            }
        }

        match outcome {
            ExistenceOutcome::Satisfy => {
                // Cite the first required in any closing interval.
                for close in &closes {
                    if !matches!(index.classify(&open_id, &close.id()), CausalOrder::Before) {
                        continue;
                    }
                    for required in &requireds {
                        if required_in_interval(index, &open_id, &required.id(), &close.id()) {
                            discharged_by.insert(open_id.clone(), required.id());
                            if ledger.tier_of(&required.source()) == Some(SourceTier::Anchor) {
                                anchored_events.insert(required.id());
                            }
                            break;
                        }
                    }
                    if discharged_by.contains_key(&open_id) {
                        break;
                    }
                }
                if ledger.tier_of(&open.source()) == Some(SourceTier::Anchor) {
                    anchored_events.insert(open_id.clone());
                }
            }
            ExistenceOutcome::Violate => {
                unmet_opens.insert(open_id.clone());
                witness_events.insert(open_id.clone());
            }
            ExistenceOutcome::InconclusivePrefix | ExistenceOutcome::InconclusiveObservational => {
                unmet_opens.insert(open_id);
            }
        }
    }

    let outcome = aggregate(outcomes)?;

    if !gate.is_open() {
        anchored_events.clear();
        witness_events.clear();
    }
    if outcome != ExistenceOutcome::Violate {
        witness_events.clear();
    }
    if matches!(
        outcome,
        ExistenceOutcome::Satisfy | ExistenceOutcome::InconclusivePrefix
    ) {
        concurrent_pairs.clear();
    }

    Some(ExistenceAssessment {
        binding,
        outcome,
        concurrent_pairs,
        discharged_by,
        unmet_opens,
        anchored_events,
        witness_events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominance_orders_violate_above_observational() {
        assert!(ExistenceOutcome::Violate > ExistenceOutcome::InconclusiveObservational);
        assert!(ExistenceOutcome::InconclusiveObservational > ExistenceOutcome::InconclusivePrefix);
        assert!(ExistenceOutcome::InconclusivePrefix > ExistenceOutcome::Satisfy);
    }

    #[test]
    fn decide_pins_interval_rows() {
        let base = ExistenceObservation::new(
            GateState::Open,
            ScopeClose::Present,
            RequiredPresence::Present,
            ConcurrentUncertainty::Absent,
            AnchorSilence::Available,
            CoverageState::Settled,
        );
        assert_eq!(decide(base), ExistenceOutcome::Satisfy);

        let absent = ExistenceObservation::new(
            GateState::Open,
            ScopeClose::Present,
            RequiredPresence::Absent,
            ConcurrentUncertainty::Absent,
            AnchorSilence::Available,
            CoverageState::Settled,
        );
        assert_eq!(decide(absent), ExistenceOutcome::Violate);

        let open_scope = ExistenceObservation::new(
            GateState::Open,
            ScopeClose::Absent,
            RequiredPresence::Absent,
            ConcurrentUncertainty::Absent,
            AnchorSilence::Available,
            CoverageState::Open,
        );
        assert_eq!(decide(open_scope), ExistenceOutcome::InconclusivePrefix);

        let concurrent = ExistenceObservation::new(
            GateState::Open,
            ScopeClose::Present,
            RequiredPresence::Present,
            ConcurrentUncertainty::Present,
            AnchorSilence::Available,
            CoverageState::Settled,
        );
        assert_eq!(
            decide(concurrent),
            ExistenceOutcome::InconclusiveObservational
        );
    }
}
