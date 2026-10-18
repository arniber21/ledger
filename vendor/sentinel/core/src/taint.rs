//! The decision table for one CAT-4 provenance prohibition, and why its shape
//! is not CAT-1's.
//!
//! CAT-4 `provenance.taint` states ADR-0016's second example of the property
//! Sentinel exists to check: an action that "was influenced by untrusted
//! content". For every action Event `t` in a Binding's Slice, **no** Event
//! matching the untrusted-content selector lies in `t`'s causal past —
//! `AG(action -> !EP(untrusted))`. That is
//! [`crate::precedence`]'s operator with the polarity inverted, and the
//! inversion is the whole subject of this module.
//!
//! # The silent pass this table exists to refuse
//!
//! `crate::precedence` folds its anchor-tier condition into its violation row,
//! and is right to. A precedence violation is by **absence** of a requirement
//! Event, so demoting it for want of an anchor-tier witness turns an accusation
//! into "not proven"; its `Satisfy` row demands a *positive* witness in the
//! trigger's causal past and the fold cannot reach it.
//!
//! Invert the polarity and the same fold is a silent pass. Here the violation is
//! by **presence** and the pass row *is* the absence row, so anything subtracted
//! from the violation row is added to the pass row. Written the obvious way —
//! row 1 reading "a falsifying Event reaches the action *and* an anchor-tier
//! witness exists" — a genuine violation observed only at contributing tier
//! fails row 1, fails the Concurrent row because it is `Before` rather than
//! unordered, and lands in `Satisfy`. A Specification would then report success
//! *because* its evidence was too weak to convict, on evidence that contains the
//! violating Event. That is the failure this system exists to prevent, and it is
//! the vacuous-instance pass #14a closed one layer up, one layer down.
//!
//! # The table
//!
//! Per action Event `t`; ordered, first match wins. `U` is the set of Slice
//! Events matching the untrusted-content selector.
//!
//! | # | condition | outcome |
//! |---|---|---|
//! | 0 | the evidence gate is closed | prefix-open |
//! | 1 | some `u` in `U` with `u ->* t`, and an anchor-tier Event of that kind is in the Slice | violated |
//! | 2 | some `u` in `U` with `u ->* t`, witnessed at contributing tier only | prefix-open |
//! | 3 | some `u` in `U` Concurrent with `t` | observationally uncertain |
//! | 4 | no such `u` either way, and Evidence Coverage is closed | discharged |
//! | 5 | otherwise | prefix-open |
//!
//! Rows 1 and 2 share one identical premise — a falsifying Event *is present* in
//! the action's causal past — and the source-tier test only chooses between
//! them. Row 4 is reachable only when that premise is false. In one line, which
//! is the line `compile/src/catalogue/mod.rs` cites as this table's grounding:
//!
//! > presence is tested above tier, and the tier test cannot reach Satisfy
//!
//! That holds whatever the deployment's tiers turn out to be, because tier is
//! never consulted on the path to row 4 at all.
//!
//! Row order alone would be a convention a later edit could undo, so the
//! invariant is in the type as well (ADR-0031 clause 1): the tier test returns a
//! [`ConvictedOutcome`], an enum with **no `Satisfy` member**, and the defective
//! fold is therefore a compile error rather than a failing test.
//! `a_contributing_tier_only_violation_never_reaches_satisfy` in
//! `core/tests/taint_decision_table.rs` pins the behaviour beside it, and was
//! demonstrated to fail against the folded table before this one replaced it.
//!
//! Stated once in the general form, because CAT-2 `absence.after-until` inherits
//! it: **where a Pattern's Violate rests on the presence of an Event, the
//! source-tier test sits strictly below the presence test and may only ever
//! choose among non-`Satisfy` outcomes.**
//!
//! # When `Satisfy` is licensed, and the ADR-0025 reconciliation
//!
//! ADR-0025 says two things that bear on row 4. First, "absence can settle a
//! Verdict only after every required causal predecessor and source-completeness
//! obligation is closed" — that is the `coverage_settled` conjunct, and it is
//! why a `Satisfy` here is never read off an incomplete prefix. Second,
//! "`Satisfy` settles only when the Specification's Scope closes without a bad
//! prefix" — and *that* is the clause a naive table gets wrong.
//!
//! The conflict is precise. A table that licenses `Satisfy` on "the Watermark
//! covers the epoch" licenses it on a **completeness** condition. A Watermark
//! says "no further Event up to here"; a Scope close says "the extent this
//! property is quantified over has ended". For an open-ended Scope the first is
//! satisfiable arbitrarily often while the second never happens, so such a table
//! emits a `Satisfy` ADR-0025 licenses nowhere.
//!
//! CAT-4 resolves it by construction rather than by adding a row. Its Scope is
//! Event-local and **its close is the trigger**: the obligation is evaluated at
//! the action Event, whose causal past is the entire extent, so "the Scope
//! closes" is not a further condition to wait for — it is the Event that raised
//! the obligation, already in evidence. `ScopeKind::EventLocal::always_closes()`
//! being `true` is that clause discharged, and `RetirementPolicy::NotRequired`
//! follows from it rather than from convenience.
//!
//! The residue, stated rather than hidden: this works **only** where the Scope's
//! close is the trigger. Dwyer's open-ended "after Q", and a Global Scope, have
//! no such Event, so ADR-0025 licenses no `Satisfy` for them at all — which is a
//! second and independent reason `ScopeKind::Global` is undeclarable, beyond the
//! unrepresented retirement policy that issue #110 owns.
//!
//! One divergence from ADR-0025 is surfaced rather than silently resolved. Its
//! first clause continues "until then the Verdict is `Inconclusive(observational)`",
//! while CONTEXT.md defines observational uncertainty as "Concurrent Events
//! cannot be ordered" and prefix uncertainty as "more Events may settle it", and
//! `crate::precedence` accordingly reports a closed evidence gate as
//! `Inconclusive(prefix)`. Rows 0, 2 and 5 follow CONTEXT.md and the existing
//! monitor: an unclosed Evidence Coverage is by CONTEXT.md's own definition the
//! prefix case. Both readings agree on the only thing the invariant needs —
//! neither is a pass.
//!
//! # Traversal
//!
//! [`observe`] produces a [`TaintObservation`] from a Binding's Slice, the
//! [`crate::frontier::CausalIndex`], and the post-#198
//! [`crate::evidence::EvidenceGate`]. [`assess`] folds those observations
//! through [`decide`] and [`aggregate`]. The table, [`convict`], and
//! [`ConvictedOutcome`] are unchanged: presence is classified onto
//! [`CausalPast::CarriesFalsifier`] before [`AnchorWitness`] is consulted, so a
//! contributing-tier-only falsifier cannot fall through to [`TaintOutcome::Satisfy`].
//!
//! [`crate::evaluator::Evaluator`] files CAT-4 action Events as
//! [`crate::slice::SliceRole::ObligationRaising`] and untrusted-content Events
//! as [`crate::slice::SliceRole::Member`], then calls [`assess`] at seal.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeSet;

use sentinel_compile::ir::TaintIr;
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

/// Named proof rule under which a taint violation is invariant across linearizations.
pub const INVARIANCE_PROOF_RULE_ID: &str = "taint.presence-above-tier";
/// Version of [`INVARIANCE_PROOF_RULE_ID`].
pub const INVARIANCE_PROOF_RULE_VERSION: &str = "v1";

/// A causal-provenance prohibition over one Binding attribute.
///
/// The Core's own form of CAT-4, sibling to
/// [`PrecedenceSpecification`](crate::specification::PrecedenceSpecification)
/// and deliberately a distinct type rather than that one with a polarity flag:
/// the polarity decides which row of [`decide`] is the default, and a `bool`
/// read the wrong way round would turn a Violate into a Satisfy.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TaintSpecification {
    identity: SpecificationIdentity,
    binding_attribute: AttributeKey,
    action: EventSelector,
    untrusted_content: EventSelector,
    required_sources: BTreeSet<RequiredSource>,
    schema_version: SchemaVersion,
    cold_guards: BTreeSet<ColdGuardId>,
    activation_barrier: Option<ActivationBarrier>,
    retirement_policy: RetirementPolicy,
}

impl TaintSpecification {
    /// Declares a provenance prohibition parameterised by one Event attribute.
    #[must_use]
    pub fn new(
        identity: SpecificationIdentity,
        binding_attribute: AttributeKey,
        action: EventSelector,
        untrusted_content: EventSelector,
        required_sources: BTreeSet<RequiredSource>,
        schema_version: SchemaVersion,
    ) -> Self {
        Self {
            identity,
            binding_attribute,
            action,
            untrusted_content,
            required_sources,
            schema_version,
            cold_guards: BTreeSet::new(),
            activation_barrier: None,
            retirement_policy: RetirementPolicy::NotRequired,
        }
    }

    /// Returns what this Specification declares about a never-closing Scope.
    ///
    /// Set only by `TryFrom<&TaintIr>`, from the Pattern's own declaration.
    /// There is deliberately **no** `with_retirement_policy` builder here, and
    /// the asymmetry with
    /// [`PrecedenceSpecification`](crate::specification::PrecedenceSpecification)
    /// is the point: `crate::verdict::build_taint` has no retired path, so a
    /// hand-built CAT-4 Specification that could declare a retiring policy
    /// would let clause (d0) admit a signal this crate cannot honour, and the
    /// Binding would emit an unpinned Verdict behind a closed gate.
    ///
    /// Every catalogued CAT-4 Pattern declares
    /// [`RetirementPolicy::NotRequired`] — the action Event whose provenance is
    /// constrained *is* the Scope's close — so this is `NotRequired` on every
    /// path that exists today, clause (d0) refuses every CAT-4 retirement, and
    /// a CAT-4 evaluator's lifecycle map is necessarily empty. **The family that
    /// lowers a retiring policy for CAT-4 owns building the retired path**, and
    /// this paragraph is the note that says so.
    #[must_use]
    pub const fn retirement_policy(&self) -> RetirementPolicy {
        self.retirement_policy
    }

    /// Records the retirement policy the lowered IR declared.
    ///
    /// `pub(crate)`, and the only writer: see
    /// [`TaintSpecification::retirement_policy`] for why no public builder
    /// exists.
    pub(crate) const fn with_lowered_retirement(mut self, policy: RetirementPolicy) -> Self {
        self.retirement_policy = policy;
        self
    }

    /// Declares the activation barrier this Specification is warming behind.
    ///
    /// `None` — the default — is a Specification that declares no barrier.
    /// `TryFrom<&TaintIr>` cannot carry one: `TaintIr` has no such field, so a
    /// catalogued instance stays barrier-free until a test or driver attaches
    /// one here, the same additive builder CAT-1 uses.
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
    /// Empty for every CAT-4 Specification: the catalogue declares
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

    /// Returns the selector for Events whose provenance is constrained.
    #[must_use]
    pub const fn action(&self) -> &EventSelector {
        &self.action
    }

    /// Returns the selector for Events that falsify the property.
    #[must_use]
    pub const fn untrusted_content(&self) -> &EventSelector {
        &self.untrusted_content
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
    ///
    /// Delegates to `sentinel_compile::ir::binding_of` for
    /// [`PrecedenceSpecification::binding_of`](crate::specification::PrecedenceSpecification::binding_of)'s
    /// reason: one implementation in the workspace, so a Slice this Pattern
    /// files an Event into cannot disagree with the one CAT-1 would.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        sentinel_compile::ir::binding_of(&self.binding_attribute, event)
    }
}

/// Lowers a catalogued CAT-4 instance into the Core's own Specification.
///
/// The one direction the crate graph permits, for the reason
/// `impl TryFrom<&PrecedenceIr>` gives: `compile` cannot depend on `core`, so a
/// Pattern lowers to plain data in `compile` and the Core reads it here.
///
/// Fallible on the same grounds as CAT-1: IR fields are still public `String`s,
/// so a cloned-and-poisoned Binding attribute or identity returns
/// [`SpecificationConversionError`] rather than constructing a Specification
/// whose `evaluate` would return `Ok(vec![])`. Instantiated catalogue IR cannot
/// take that path. A well-formed but simply wrong attribute key still converts,
/// because whether a key names anything is a fact about the evidence. That
/// residue is `PatternInstance::validate_against`'s.
///
/// Barrier-free: `TaintIr` has no activation-barrier field, so that Core-only
/// builder stays off this path. The **retirement policy does** cross, because
/// `TaintIr` carries it: a retirement declaration is a Pattern property, and
/// carrying it is what makes a catalogued CAT-4 evaluator refuse retirement
/// because of its own declaration rather than because of a family check.
impl TryFrom<&TaintIr> for TaintSpecification {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &TaintIr) -> Result<Self, Self::Error> {
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
                lowered.action.kind.clone(),
                lowered.action.required_attributes.clone(),
            ),
            EventSelector::new(
                lowered.untrusted_content.kind.clone(),
                lowered.untrusted_content.required_attributes.clone(),
            ),
            lowered
                .required_sources
                .iter()
                .map(|(source, tier)| RequiredSource::new(source.clone(), *tier))
                .collect(),
            lowered.schema_version.clone(),
        )
        .with_lowered_retirement(RetirementPolicy::from(lowered.retirement)))
    }
}

/// The decision table's collapsed Open/Closed input.
///
/// This is not a second G1–G5 implementation. The gate itself is
/// [`crate::evidence::EvidenceGate`]. [`crate::evidence::EvidenceGate::is_open`]
/// maps to [`GateState::Open`], otherwise [`GateState::Closed`]. Do not map
/// [`crate::evidence::EvidenceGate::watermarks_cover_epochs`] here — that is
/// [`CoverageState`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GateState {
    /// Every required source is covered, contiguous, healthy and caught up.
    Open,
    /// At least one gate disjunct fired, so nothing may settle.
    Closed,
}

/// Whether a falsifying Event lies in the action's causal past.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausalPast {
    /// Some Event matching the untrusted-content selector reaches the action.
    CarriesFalsifier,
    /// No such Event reaches it in any admissible linearization.
    Clean,
}

/// Whether the Slice holds an anchor-tier Event of the falsifying kind.
///
/// Kind rather than full match, and Slice-wide rather than per witness, for the
/// reason `crate::precedence`'s `anchor_candidates` gives: what makes a
/// conclusion anchor-witnessed is that the source which would have reported such
/// an Event is one the Monitored Application cannot suppress.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnchorWitness {
    /// An anchor-tier source witnessed the falsifying kind in this Slice.
    Present,
    /// Only contributing-tier sources did.
    Absent,
}

/// Whether a falsifying Event is Concurrent with the action.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ConcurrentFalsifier {
    /// Some falsifying Event is unordered with respect to the action.
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

/// What the evidence says about one action Event's provenance.
///
/// Five independent facts a monitor computes and this table consumes.
///
/// # Why five types rather than five `bool`s
///
/// The first version of this struct took five adjacent booleans, and
/// `TaintObservation::new(true, true, false, false, true)` is exactly the shape
/// ADR-0031 clause 1 was written about: transposing two arguments changes the
/// meaning and no compiler objects. Clippy's `fn_params_excessive_bools`
/// reported it, and the fix is the ADR's rather than an `allow` — five distinct
/// two-variant types, so a transposition is `error[E0308]`.
///
/// They stay five separate types rather than one classification enum for a
/// second reason. The exhaustive test enumerates all thirty-two assignments,
/// including combinations a real traversal would never produce, and proves the
/// `Satisfy` row is unreachable under **any** of them. A pre-collapsed
/// classification would let the collapse hide the case the table exists to
/// refuse, which is the defect in a different costume.
///
/// None of the five is `#[non_exhaustive]`, for the reason
/// `sentinel_compile::catalogue::TriggerExercise` gives about itself: they are
/// closed two-valued facts, and a caller folding one into a decision should be
/// able to write an exhaustive `match`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct TaintObservation {
    /// Whether the pre-obligation evidence gate is open.
    pub gate: GateState,
    /// Whether a falsifying Event lies in the action's causal past.
    pub causal_past: CausalPast,
    /// Whether the Slice holds an anchor-tier Event of the falsifying kind.
    pub anchor_witness: AnchorWitness,
    /// Whether a falsifying Event is Concurrent with the action.
    pub concurrent: ConcurrentFalsifier,
    /// Whether Evidence Coverage has closed.
    pub coverage: CoverageState,
}

impl TaintObservation {
    /// Records one action Event's provenance facts.
    #[must_use]
    pub const fn new(
        gate: GateState,
        causal_past: CausalPast,
        anchor_witness: AnchorWitness,
        concurrent: ConcurrentFalsifier,
        coverage: CoverageState,
    ) -> Self {
        Self {
            gate,
            causal_past,
            anchor_witness,
            concurrent,
            coverage,
        }
    }
}

/// The conclusion reached for one action Event, or for one Binding.
///
/// The ordering is the dominance order a Binding aggregates with, and it is
/// [`crate::precedence::BindingOutcome`]'s: a larger value dominates a smaller
/// one. The two enums are deliberately not shared. They name the same four
/// Verdict states but they are reached by different tables, and a single type
/// would invite a single `assess` with a polarity argument — which is the shape
/// this module's header argues against.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TaintOutcome {
    /// No falsifying Event reaches the action, and Evidence Coverage is closed.
    Satisfy,
    /// The evidence is open: more of it could still settle this either way.
    InconclusivePrefix,
    /// A falsifying Event is Concurrent with the action and cannot be ordered.
    InconclusiveObservational,
    /// A falsifying Event reaches the action, witnessed at anchor tier.
    Violate,
}

/// What a **settled falsifying witness** can conclude, once tier is consulted.
///
/// Deliberately narrower than [`TaintOutcome`]: `Satisfy` is not a member, and
/// cannot be added without a reviewer noticing what it would mean. This is the
/// invariant of this module's table expressed the way ADR-0031 clause 1 asks —
/// in a type the caller cannot bypass — rather than as row order, which is a
/// convention a later edit can undo without a compiler objecting.
///
/// The value of that is narrow and worth stating exactly: it does not stop
/// someone rewriting [`decide`] to return `Satisfy` directly from the presence
/// branch. What it stops is the specific edit that produced the defect —
/// folding the tier condition into the violation row's guard and letting the
/// fall-through continue down the table.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ConvictedOutcome {
    /// The witness is not authoritative, so the conclusion does not settle.
    ///
    /// ADR-0022: a contributing-tier source "cannot, alone, anchor a Verdict for
    /// a high-assurance property". The observation is **not** discarded — it is
    /// what keeps this action out of the `Satisfy` row.
    InconclusivePrefix,
    /// The witness is anchor-tier, so the violation settles.
    Violate,
}

impl ConvictedOutcome {
    /// Widens a settled-witness conclusion to the full outcome lattice.
    #[must_use]
    pub const fn widen(self) -> TaintOutcome {
        match self {
            Self::InconclusivePrefix => TaintOutcome::InconclusivePrefix,
            Self::Violate => TaintOutcome::Violate,
        }
    }
}

impl From<ConvictedOutcome> for TaintOutcome {
    fn from(convicted: ConvictedOutcome) -> Self {
        convicted.widen()
    }
}

/// Applies the ordered decision table to one action Event's observation.
///
/// Read this module's header for why the rows are in this order and why the
/// presence test is above the tier test.
#[must_use]
pub const fn decide(observation: TaintObservation) -> TaintOutcome {
    // Row 0. The gate runs before obligation analysis and wins outright rather
    // than by dominance, exactly as in `crate::precedence`: it must be able to
    // demote a Violate, so it cannot be folded in as another dominated row.
    if matches!(observation.gate, GateState::Closed) {
        return TaintOutcome::InconclusivePrefix;
    }
    // Rows 1 and 2, as one premise. Presence is settled here; only the strength
    // of the conclusion is left for the tier test, and `ConvictedOutcome` is why
    // that test cannot answer `Satisfy`.
    if matches!(observation.causal_past, CausalPast::CarriesFalsifier) {
        return convict(observation.anchor_witness).widen();
    }
    // Row 3. Some linearization places the falsifier before the action and
    // another places it after, so no admissible order settles it.
    if matches!(observation.concurrent, ConcurrentFalsifier::Present) {
        return TaintOutcome::InconclusiveObservational;
    }
    // Row 4. ADR-0025's licence: the Scope closed at this Event, and Evidence
    // Coverage is closed too.
    if matches!(observation.coverage, CoverageState::Settled) {
        return TaintOutcome::Satisfy;
    }
    // Row 5.
    TaintOutcome::InconclusivePrefix
}

/// Chooses between the two conclusions a present falsifying witness supports.
///
/// The return type is the point: see [`ConvictedOutcome`].
const fn convict(anchor_witness: AnchorWitness) -> ConvictedOutcome {
    match anchor_witness {
        AnchorWitness::Present => ConvictedOutcome::Violate,
        AnchorWitness::Absent => ConvictedOutcome::InconclusivePrefix,
    }
}

/// Folds one Binding's per-action outcomes into the Binding's conclusion.
///
/// `None` when the Binding raised no obligation — no action Event occurred, so
/// there is nothing for a Verdict to be about. That is the third place this
/// module refuses a vacuous pass, and it is the one an `Iterator::all` would get
/// wrong: `all` over an empty set is `true`, and `Satisfy` is what `true` would
/// render as. `crate::precedence::assess` makes the identical call by skipping a
/// Binding whose trigger set is empty, and `shared/oracle.ts` makes it too.
///
/// Otherwise the dominance maximum: the obligation is universally quantified
/// over the Binding's actions, so one settled violation falsifies the
/// conjunction in every linearization and one action's uncertainty makes the
/// conjunction uncertain.
#[must_use]
pub fn aggregate(outcomes: impl IntoIterator<Item = TaintOutcome>) -> Option<TaintOutcome> {
    outcomes.into_iter().max()
}

/// What the monitor concluded about one Binding, and the evidence it cites.
///
/// Distinct from [`crate::precedence::BindingAssessment`]: sharing them would
/// invite a single `assess` with a polarity `bool`, which this module exists
/// to refuse.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TaintAssessment {
    binding: Binding,
    outcome: TaintOutcome,
    concurrent_pairs: BTreeSet<(EventId, EventId)>,
    falsifying_ancestors: BTreeSet<EventId>,
    anchored_events: BTreeSet<EventId>,
    witness_events: BTreeSet<EventId>,
}

impl TaintAssessment {
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
    pub const fn outcome(&self) -> TaintOutcome {
        self.outcome
    }

    /// Returns every `(falsifier, action)` pair the evidence leaves unordered.
    #[must_use]
    pub const fn concurrent_pairs(&self) -> &BTreeSet<(EventId, EventId)> {
        &self.concurrent_pairs
    }

    /// Returns a deterministic description of the unordered pairs.
    #[must_use]
    pub fn concurrency_detail(&self) -> String {
        self.concurrent_pairs
            .iter()
            .map(|(falsifier, action)| format!("{falsifier} || {action}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Returns untrusted Events that reach an action in the causal past.
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

/// Produces the five facts [`decide`] consumes for one action Event.
///
/// Presence (`u ->* t`) is classified onto [`CausalPast::CarriesFalsifier`]
/// independently of tier. [`AnchorWitness`] is kind + Slice-wide, the same
/// rule [`crate::precedence::assess`] uses for `anchor_candidates`:
/// `matches_kind` and `ledger.tier_of == Some(SourceTier::Anchor)`, not a
/// full selector match on the witnessing Event.
///
/// Gate open/closed maps from [`EvidenceGate::is_open`]. Coverage maps from
/// [`EvidenceGate::watermarks_cover_epochs`]. Those are two facts; they are
/// not folded together.
#[must_use]
pub fn observe(
    specification: &TaintSpecification,
    action: &Event,
    slice: &[&Event],
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> TaintObservation {
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

    let mut causal_past = CausalPast::Clean;
    let mut concurrent = ConcurrentFalsifier::Absent;
    for event in slice {
        if !specification.untrusted_content().matches(event) {
            continue;
        }
        match index.classify(&event.id(), &action.id()) {
            CausalOrder::Before => causal_past = CausalPast::CarriesFalsifier,
            CausalOrder::Concurrent => concurrent = ConcurrentFalsifier::Present,
            CausalOrder::After => {}
        }
    }

    let anchor_witness = if slice.iter().any(|event| {
        specification.untrusted_content().matches_kind(event)
            && ledger.tier_of(&event.source()) == Some(SourceTier::Anchor)
    }) {
        AnchorWitness::Present
    } else {
        AnchorWitness::Absent
    };

    TaintObservation::new(
        gate_state,
        causal_past,
        anchor_witness,
        concurrent,
        coverage,
    )
}

/// Evaluates the Specification over every Binding an action Event creates.
///
/// Bindings are created by action Events. An empty action set produces no
/// Binding and no Verdict: [`aggregate`] over empty is `None`, which is the
/// third place this module refuses a vacuous pass.
#[must_use]
pub fn assess(
    specification: &TaintSpecification,
    index: &CausalIndex,
    frontier: &Frontier,
    slices: &SliceIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
) -> Vec<TaintAssessment> {
    let mut assessments = Vec::new();
    for binding in slices.bindings() {
        let Some(members) = slices.members(binding) else {
            continue;
        };
        let slice: Vec<&Event> = members.iter().filter_map(|id| frontier.event(id)).collect();
        let actions: Vec<&Event> = slices
            .obligation_raising(binding)
            .into_iter()
            .flatten()
            .filter_map(|id| frontier.event(id))
            .collect();
        if actions.is_empty() {
            continue;
        }
        assessments.push(assess_binding(
            specification,
            index,
            gate,
            ledger,
            binding.clone(),
            &slice,
            &actions,
        ));
    }
    assessments.sort_by(|left, right| left.binding.cmp(&right.binding));
    assessments
}

fn assess_binding(
    specification: &TaintSpecification,
    index: &CausalIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
    binding: Binding,
    slice: &[&Event],
    actions: &[&Event],
) -> TaintAssessment {
    let untrusted: Vec<&Event> = slice
        .iter()
        .copied()
        .filter(|event| specification.untrusted_content().matches(event))
        .collect();

    let anchor_candidates: BTreeSet<EventId> = slice
        .iter()
        .filter(|event| {
            specification.untrusted_content().matches_kind(event)
                && ledger.tier_of(&event.source()) == Some(SourceTier::Anchor)
        })
        .map(|event| event.id())
        .collect();

    let mut outcomes = Vec::with_capacity(actions.len());
    let mut concurrent_pairs = BTreeSet::new();
    let mut falsifying_ancestors = BTreeSet::new();
    let mut anchored_events = BTreeSet::new();
    let mut witness_events = BTreeSet::new();

    for action in actions {
        let observation = observe(specification, action, slice, index, gate, ledger);
        let outcome = decide(observation);
        outcomes.push(outcome);

        for event in &untrusted {
            match index.classify(&event.id(), &action.id()) {
                CausalOrder::Before => {
                    falsifying_ancestors.insert(event.id());
                }
                CausalOrder::Concurrent => {
                    concurrent_pairs.insert((event.id(), action.id()));
                }
                CausalOrder::After => {}
            }
        }

        match outcome {
            TaintOutcome::Violate => {
                anchored_events.extend(anchor_candidates.iter().cloned());
                witness_events.insert(action.id());
                witness_events.extend(anchor_candidates.iter().cloned());
            }
            TaintOutcome::Satisfy => {
                // An Authoritative Satisfy must cite an anchor-tier Event; the
                // action is the Event-local Scope close the conclusion rests on.
                if ledger.tier_of(&action.source()) == Some(SourceTier::Anchor) {
                    anchored_events.insert(action.id());
                }
            }
            TaintOutcome::InconclusivePrefix | TaintOutcome::InconclusiveObservational => {}
        }
    }

    let outcome = aggregate(outcomes).expect("actions is non-empty");

    if !gate.is_open() {
        anchored_events.clear();
        witness_events.clear();
    }
    if outcome != TaintOutcome::Violate {
        witness_events.clear();
    }
    if matches!(
        outcome,
        TaintOutcome::Satisfy | TaintOutcome::InconclusivePrefix
    ) {
        concurrent_pairs.clear();
    }

    TaintAssessment {
        binding,
        outcome,
        concurrent_pairs,
        falsifying_ancestors,
        anchored_events,
        witness_events,
    }
}
