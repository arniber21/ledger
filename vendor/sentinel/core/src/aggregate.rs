//! The decision table for one CAT-5 aggregation obligation, and why every row
//! that is not a settled under-threshold count is a non-pass.
//!
//! CAT-5 `aggregation.count-within` states: for every Event matching the
//! `trigger` selector in a Binding's Slice, the count of Events matching the
//! `member` selector inside a **sequence** lookback stays strictly below the
//! declared threshold. The threshold is exclusive on both sides of the seam —
//! `sentinel_coldpath`'s `CountWithin` Holds on `count < threshold` and Fails on
//! `count >= threshold`, and [`AggregateSpecification::threshold`] carries the
//! same bound.
//!
//! # What makes this table different from the other three
//!
//! [`crate::precedence`], [`crate::taint`] and [`crate::absence`] all read their
//! falsifier off the Slice the hot path already holds. This one cannot. The
//! lookback the count is drawn from is a **retained** window of causal evidence
//! rather than the Slice's own admitted Events, so whether that window is
//! complete is a retention question, and ADR-0005 puts exactly that class of
//! question — "aggregation across Bindings" — on the cold path. The hot path's
//! whole job here is to raise one obligation per trigger, declare one query per
//! Binding, and read the answer back.
//!
//! So the falsifying fact is a [`crate::facts::GuardConclusion`], and
//! [`CountEvidence`] is the reduction of the Binding's settled cold answers into
//! the one fact this table consumes. `sentinel-coldpath` owns what the question
//! means; this module owns what its answer licenses.
//!
//! # Missing evidence is never a pass, in the shape this family takes
//!
//! A cold Pattern has two ways to have no count, and neither may reach
//! [`AggregateOutcome::Satisfy`]:
//!
//! - [`CountEvidence::Unasked`] — no cold guard is bound, so the count was never
//!   requested. Hand-built Specifications start empty until a driver attaches a
//!   guard through [`AggregateSpecification::with_cold_guard`]; catalogued
//!   `TryFrom<&AggregateIr>` binds the derived count-within guard so Candidate
//!   replay refuses without a fact supplier rather than measuring an unasked
//!   count as a clean rate.
//! - [`CountEvidence::Outstanding`] — a guard is bound and has not settled.
//!
//! `Iterator::all` over an empty answer set is `true`, and `Satisfy` is what
//! `true` would render as. Both of the above are therefore separate values
//! rather than an absent one, and the table below answers Inconclusive for each.
//!
//! # The table
//!
//! Per trigger Event; ordered, first match wins.
//!
//! | # | condition | outcome |
//! |---|---|---|
//! | 0 | the evidence gate is closed | prefix-open |
//! | 1 | a complete lookback met or exceeded the threshold | over-threshold |
//! | 2 | no answer will ever exist for the asked lookback | epistemic |
//! | 3 | no guard is bound, so no count was asked for | epistemic |
//! | 4 | a guard is bound and has not settled | prefix-open |
//! | 5 | complete lookback under threshold, Evidence Coverage closed | discharged |
//! | 6 | otherwise | prefix-open |
//!
//! Row 1 sits **above** rows 2 through 4 for [`crate::taint`]'s reason in this
//! family's shape: the over-threshold count is a settled positive finding, and a
//! row that let a second guard's refusal or a still-open query subtract from it
//! would move the finding towards the pass row. Row 5 is reachable only when the
//! count is settled *and* under threshold *and* Evidence Coverage has closed.
//!
//! # A cold `Fails` is an Authoritative `Inconclusive`, never a `Violate`
//!
//! [`AggregateOutcome::OverThreshold`] maps to `VerdictOutcome::Inconclusive`,
//! not to `Violate`. That is not caution about the count; it is the hot/cold
//! composition boundary. ADR-0021 makes a Verdict authoritative only once "every
//! cold-path guard the Specification requires has settled at a Watermark" and
//! defines the reconciliation protocol for the two tiers; `sentinel-core`'s
//! admitted limitation 8 forbids a cold `Violate` over a hot `Satisfy`, and
//! `sentinel_coldpath`'s crate docs record the hand-off in terms: *"a complete
//! lookback that meets or exceeds the count threshold returns `Fails`; Core maps
//! that to Authoritative `Inconclusive` until #20's cold-witness trust boundary
//! lands."*
//! A `ColdWitness` is stored on [`crate::facts::FactLedger`] and never enters
//! `anchored_event_ids` or a `WatermarkAnchoredViolationBasis`, so this module
//! cites no witness for the finding it reports.
//!
//! The consequence worth stating plainly: **CAT-5 mints no `Violate` today.**
//! `crate::verdict::build_aggregate` therefore assembles no violation basis, and
//! `aggregate_mints_no_violate` pins that rather than leaving it to be inferred
//! from an absent arm.
//!
//! Both non-pass finding rows land on `InconclusiveReason::EpistemicUncertainty`
//! because the model has no fourth reason for "a cold finding awaiting
//! composition"; the envelope's uncertainty detail is what separates them, and
//! adding a reason is a `proto/` change #20 owns (ADR-0024).
//!
//! # Detect-only
//!
//! ADR-0023's third enforcement gate requires a Verdict settlable by the
//! frontier monitor *"without a cold-path guard"*. A cold Pattern fails that
//! gate by construction, so CAT-5 is detect-only and is not a `decide()` input:
//! [`crate::evaluator::Evaluator::specification`] answers `None` for it, exactly
//! as it does for CAT-4 and CAT-2.
//!
//! # Traversal
//!
//! [`observe`] pairs one Binding's [`CountEvidence`] with the gate and coverage
//! states. [`assess`] folds one observation per trigger through [`decide`] and
//! [`aggregate`]. [`crate::evaluator::Evaluator`] files CAT-5 trigger Events as
//! [`crate::slice::SliceRole::ObligationRaising`] and member Events as
//! [`crate::slice::SliceRole::Member`], then calls [`assess`] at seal.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::ir::AggregateIr;
use sentinel_model::{
    ActivationBarrier, Event, EventId, SchemaVersion, SourceIdentity, SourceTier,
};

use crate::evidence::{EvidenceGate, EvidenceLedger};
use crate::facts::{ColdGuardId, ColdWitness, FactLedger, GuardConclusion};
use crate::frontier::Frontier;
use crate::slice::{Binding, SliceIndex};
use crate::specification::{
    AttributeKey, EventSelector, RequiredSource, RetirementPolicy, SpecificationConversionError,
    SpecificationId, SpecificationIdentity, SpecificationVersion,
};

/// Name half of the [`ColdGuardId`] a CAT-5 count is asked under.
///
/// Equal to the catalogued Pattern id string, so a deployment reading a
/// [`crate::facts::FactQuery`] can tell which Pattern family asked without
/// parsing the version half.
pub const COUNT_WITHIN_GUARD_NAME: &str = "aggregation.count-within";

/// Derives the [`ColdGuardId`] a CAT-5 Specification asks its count under.
///
/// # Why the format lives in two crates
///
/// `sentinel-coldpath`'s `GuardDefinition::cold_guard_id` composes exactly this
/// string, from `Operator::identity_payload` and its `LookbackBound`. That is
/// the definition; this is a second copy of it, and the duplication is forced
/// rather than chosen: `sentinel-coldpath` depends on `sentinel-core`, so the
/// call cannot come back the other way without a cycle (ADR-0012).
///
/// The two must agree byte for byte, because this string is the `FactQuery`
/// key. A byte of drift is not a crash — `GuardCatalogue::get` misses, the
/// periphery answers [`crate::facts::RefusalReason::GuardUnknown`], and this
/// module's table reads that as [`CountEvidence::Unholdable`] and reports
/// Inconclusive forever. So the failure mode of drift is a Pattern that never
/// settles while looking like it is merely waiting. `coldpath`'s
/// `core_derives_the_same_cold_guard_id` is the pin that refuses it, and it
/// lives there because that is the only crate that can see both sides.
#[must_use]
pub fn count_within_cold_guard_id(
    member: &EventSelector,
    threshold: u64,
    lookback_sequences: u64,
) -> ColdGuardId {
    // Assembled in coldpath's own two steps -- selector payload, then the
    // operator and lookback envelope -- so the two functions read as the same
    // format rather than as two strings that happen to match today.
    let mut selector = vec![member.kind().as_str().to_owned()];
    for (key, value) in member.required_attributes() {
        selector.push(format!("{key}={value:?}"));
    }
    ColdGuardId::new(
        COUNT_WITHIN_GUARD_NAME,
        format!(
            "cw:{}|t={threshold}|lb={lookback_sequences}",
            selector.join(",")
        ),
    )
}

/// A sequence-lookback count obligation over one Binding attribute.
///
/// The Core's own form of CAT-5, and a distinct type from
/// [`AbsenceSpecification`](crate::absence::AbsenceSpecification) for the reason
/// the other three families are distinct from each other: the falsifying fact
/// here is a cold-path conclusion rather than a Slice Event, so the traversal
/// has no row in common with theirs and a shared struct would invite one
/// `assess` with a tier flag.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AggregateSpecification {
    identity: SpecificationIdentity,
    binding_attribute: AttributeKey,
    trigger: EventSelector,
    member: EventSelector,
    lookback_sequences: u64,
    threshold: u64,
    required_sources: BTreeSet<RequiredSource>,
    schema_version: SchemaVersion,
    cold_guards: BTreeSet<ColdGuardId>,
    activation_barrier: Option<ActivationBarrier>,
    retirement_policy: RetirementPolicy,
}

impl AggregateSpecification {
    /// Declares a count obligation parameterised by one Event attribute.
    ///
    /// `lookback_sequences` is a count of **sequence positions** of causal
    /// evidence. It is never a wall-clock duration: `sentinel-coldpath`'s
    /// `LookbackBound` is the same sequence bound, and ADR-0020 rejects
    /// timestamp ordering as *"Unsound under ordinary clock skew"*. Nothing on
    /// this path converts a duration into a position count.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // Lookback + threshold are CAT-5's two numeric parameters.
    pub fn new(
        identity: SpecificationIdentity,
        binding_attribute: AttributeKey,
        trigger: EventSelector,
        member: EventSelector,
        lookback_sequences: u64,
        threshold: u64,
        required_sources: BTreeSet<RequiredSource>,
        schema_version: SchemaVersion,
    ) -> Self {
        Self {
            identity,
            binding_attribute,
            trigger,
            member,
            lookback_sequences,
            threshold,
            required_sources,
            schema_version,
            cold_guards: BTreeSet::new(),
            activation_barrier: None,
            retirement_policy: RetirementPolicy::NotRequired,
        }
    }

    /// Returns what this Specification declares about a never-closing Scope.
    ///
    /// CAT-5's catalogued Pattern declares [`RetirementPolicy::NotRequired`]:
    /// lookback is a sequence cold parameter, not a Scope that can fail to
    /// close. Carried for retirement-free workload byte-identity.
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
    /// `TryFrom<&AggregateIr>` cannot carry one: `AggregateIr` has no such
    /// field, the same additive builder CAT-1, CAT-4 and CAT-2 use.
    #[must_use]
    pub fn with_activation_barrier(mut self, activation_barrier: ActivationBarrier) -> Self {
        self.activation_barrier = Some(activation_barrier);
        self
    }

    /// Declares one cold-path guard this Specification's count comes from.
    ///
    /// Additive for `TryFrom<&AggregateIr>`'s reason: the IR carries no
    /// [`ColdGuardId`], because which catalogued cold operator answers a
    /// Specification is a deployment binding rather than a Pattern parameter —
    /// `sentinel-coldpath` derives the id from the operator's own identity
    /// payload, and `compile` cannot depend on that crate.
    ///
    /// A Specification with no guard bound is **not** a Specification whose
    /// count holds. It is [`CountEvidence::Unasked`], which this module's table
    /// answers Inconclusive.
    #[must_use]
    pub fn with_cold_guard(mut self, guard: ColdGuardId) -> Self {
        self.cold_guards.insert(guard);
        self
    }

    /// Returns the activation barrier, when this Specification declares one.
    #[must_use]
    pub const fn activation_barrier(&self) -> Option<&ActivationBarrier> {
        self.activation_barrier.as_ref()
    }

    /// Returns the declared cold-path guards.
    ///
    /// Unlike CAT-1, CAT-4 and CAT-2 this is the family where a non-empty set is
    /// the intended deployment: the catalogue declares `EvaluationTier::Cold`
    /// with a non-zero fact-request count per Verdict.
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

    /// Returns the selector for Events that raise the count obligation.
    #[must_use]
    pub const fn trigger(&self) -> &EventSelector {
        &self.trigger
    }

    /// Returns the selector for Events the lookback counts.
    #[must_use]
    pub const fn member(&self) -> &EventSelector {
        &self.member
    }

    /// Returns how many sequence positions of causal evidence the count spans.
    #[must_use]
    pub const fn lookback_sequences(&self) -> u64 {
        self.lookback_sequences
    }

    /// Returns the **exclusive** upper bound on the member count.
    ///
    /// `count < threshold` is the pass; `count >= threshold` is the finding.
    /// Identical to `sentinel_coldpath`'s `CountWithin::threshold`, which is the
    /// value that actually performs the comparison.
    #[must_use]
    pub const fn threshold(&self) -> u64 {
        self.threshold
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

/// Lowers a catalogued CAT-5 instance into the Core's own Specification.
///
/// Barrier-free, as the other three families are: `AggregateIr` has no
/// activation-barrier field, so that stays an additive builder.
///
/// **Unlike them, this is not guard-free.** [`count_within_cold_guard_id`]
/// fingerprints the member selector, threshold and lookback, and the guard is
/// declared here. The alternative — lower guard-free and let a driver attach
/// one — was rejected: such a Specification declares no cold guard, so
/// [`crate::evaluate`] does not raise [`crate::CoreError::FactsRequired`],
/// nothing asks the periphery, and every Binding reports
/// [`CountEvidence::Unasked`]. No individual Verdict would be unsound, because
/// each is Inconclusive. But the catalogue declares this Pattern
/// `EvaluationTier::Cold` at one fact request per Verdict, and a lowering that
/// silently asks zero makes that declaration false — which is how Candidate
/// replay would report a rate it never measured. Missing evidence is never a
/// pass, and a demand for evidence is not something to drop quietly either.
impl TryFrom<&AggregateIr> for AggregateSpecification {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &AggregateIr) -> Result<Self, Self::Error> {
        let id = SpecificationId::new(lowered.specification_id.clone())
            .map_err(|_| SpecificationConversionError::BlankSpecificationId)?;
        let version = SpecificationVersion::new(lowered.specification_version.clone())
            .map_err(|_| SpecificationConversionError::BlankSpecificationVersion)?;
        let binding_attribute = AttributeKey::new(lowered.binding_attribute.clone())
            .map_err(|_| SpecificationConversionError::BlankBindingAttribute)?;
        let member = EventSelector::new(
            lowered.member.kind.clone(),
            lowered.member.required_attributes.clone(),
        );
        Ok(Self::new(
            SpecificationIdentity::new(id, version),
            binding_attribute,
            EventSelector::new(
                lowered.trigger.kind.clone(),
                lowered.trigger.required_attributes.clone(),
            ),
            member.clone(),
            lowered.lookback_sequences,
            lowered.threshold,
            lowered
                .required_sources
                .iter()
                .map(|(source, tier)| RequiredSource::new(source.clone(), *tier))
                .collect(),
            lowered.schema_version.clone(),
        )
        .with_cold_guard(count_within_cold_guard_id(
            &member,
            lowered.threshold,
            lowered.lookback_sequences,
        ))
        .with_retirement_policy(RetirementPolicy::from(lowered.retirement)))
    }
}

/// The decision table's collapsed Open/Closed input.
///
/// Maps from [`EvidenceGate::is_open`]. Do not map
/// [`EvidenceGate::watermarks_cover_epochs`] here — that is [`CoverageState`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GateState {
    /// Every required source is covered, contiguous, healthy and caught up.
    Open,
    /// At least one gate disjunct fired, so nothing may settle.
    Closed,
}

/// What the cold path has said about one Binding's member count.
///
/// Five values, and the two that mean "no count" are separate from the three
/// that mean "a count" for this module's header's reason: an absent answer must
/// not be foldable into the pass row.
///
/// Derived by [`CountEvidence::of`] from [`crate::facts::FactLedger`] rather than
/// constructed by a caller, so the reduction from guard conclusions to this fact
/// is written once.
///
/// **Deliberately not `#[non_exhaustive]`.** A closed reduction with no sixth
/// answer is the point: a consumer folding this into a decision must be forced
/// to recompile when a value is added.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CountEvidence {
    /// No cold guard is bound, so no count was ever asked for.
    Unasked,
    /// A bound guard has not settled yet.
    Outstanding,
    /// A complete lookback counted strictly fewer members than the threshold.
    UnderThreshold,
    /// A complete lookback counted at or above the threshold.
    ///
    /// The finding. `sentinel-coldpath` answers `Fails` for it, and this crate
    /// maps that to an Authoritative `Inconclusive` — see the module header.
    AtOrOverThreshold,
    /// No answer will ever exist for the asked lookback.
    ///
    /// A terminal `Refused`: the window was reaped, never retained, or is
    /// otherwise permanently unholdable. Never a pass, and — unlike
    /// [`CountEvidence::Outstanding`] — never going to become one.
    Unholdable,
}

impl CountEvidence {
    /// Reduces one Binding's declared queries and settled answers to one fact.
    ///
    /// `Fails` outranks `Refused` when both are present, which is the same
    /// polarity choice [`decide`]'s row order makes and for the same reason: a
    /// settled positive finding is not weakened by a second guard's refusal.
    /// Neither is a pass, so the choice cannot reach [`AggregateOutcome::Satisfy`]
    /// either way.
    #[must_use]
    pub fn of(
        declared_guards: &BTreeSet<ColdGuardId>,
        queried: bool,
        every_query_settled: bool,
        closed: &BTreeMap<ColdGuardId, GuardConclusion>,
    ) -> Self {
        if declared_guards.is_empty() || !queried {
            return Self::Unasked;
        }
        if !every_query_settled {
            return Self::Outstanding;
        }
        if closed
            .values()
            .any(|conclusion| matches!(conclusion, GuardConclusion::Fails { .. }))
        {
            return Self::AtOrOverThreshold;
        }
        if closed
            .values()
            .any(|conclusion| matches!(conclusion, GuardConclusion::Refused { .. }))
        {
            return Self::Unholdable;
        }
        // `FactLedger::closed` returns only Refused and Fails, so an empty map
        // under a fully settled query set means every answer was `Holds` — and
        // `CountWithin` Holds only on a complete lookback under threshold.
        Self::UnderThreshold
    }

    /// Returns whether this fact could still change with more evidence.
    #[must_use]
    pub const fn could_still_settle(self) -> bool {
        matches!(self, Self::Outstanding)
    }
}

/// Whether Evidence Coverage has closed for every required source.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CoverageState {
    /// Every required source's Watermark covers its epoch.
    Settled,
    /// At least one does not, so more evidence is outstanding.
    Open,
}

/// What the evidence says about one trigger Event's count obligation.
///
/// Three independent facts, as three distinct types rather than a tuple of
/// booleans and an enum (ADR-0031 clause 1): a transposition is `error[E0308]`,
/// and the exhaustive suite enumerates all twenty assignments — including
/// combinations a real traversal would never produce — so the `Satisfy` row
/// stays unreachable under any absent, unholdable or over-threshold count.
///
/// The count is a per-**Binding** fact today, because a cold query is keyed by
/// `(Binding, guard)` rather than by trigger. It is carried per trigger anyway
/// so this module's shape matches the other three and so a per-trigger lookback
/// does not have to reshape the table.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct AggregateObservation {
    /// Whether the pre-obligation evidence gate is open.
    pub gate: GateState,
    /// What the cold path has said about the member count.
    pub count: CountEvidence,
    /// Whether Evidence Coverage has closed.
    pub coverage: CoverageState,
}

impl AggregateObservation {
    /// Records one trigger Event's count facts.
    #[must_use]
    pub const fn new(gate: GateState, count: CountEvidence, coverage: CoverageState) -> Self {
        Self {
            gate,
            count,
            coverage,
        }
    }
}

/// The conclusion reached for one trigger Event, or for one Binding.
///
/// The ordering is the dominance order a Binding aggregates with, and it is
/// [`crate::taint::TaintOutcome`]'s: a larger value dominates a smaller one.
/// [`AggregateOutcome::OverThreshold`] is the maximum because it is the only
/// settled finding, and a Binding one of whose triggers assessed over threshold
/// is over threshold whatever its other triggers concluded.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AggregateOutcome {
    /// A complete lookback counted under the threshold, and Evidence Coverage
    /// is closed.
    Satisfy,
    /// The evidence is open: more of it could still settle this either way.
    InconclusivePrefix,
    /// The count is unavailable and will stay unavailable, or was never asked
    /// for at all.
    InconclusiveEpistemic,
    /// A complete lookback met or exceeded the threshold.
    ///
    /// An Authoritative `Inconclusive`, never a `Violate`: see the module
    /// header on the ADR-0021 composition boundary and #20.
    OverThreshold,
}

/// What a **settled cold answer** can conclude about the count.
///
/// Deliberately narrower than [`AggregateOutcome`]: `Satisfy` is not a member.
/// Inherited from [`crate::taint::ConvictedOutcome`] and for the same reason —
/// the invariant that a finding row can never fall through to the pass row is
/// expressed in a type a caller cannot bypass, rather than in row order a later
/// edit can undo.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SettledCount {
    /// The count is permanently unavailable, or no guard was ever asked.
    InconclusiveEpistemic,
    /// A complete lookback met or exceeded the threshold.
    OverThreshold,
}

impl SettledCount {
    /// Widens a settled-count conclusion to the full outcome lattice.
    #[must_use]
    pub const fn widen(self) -> AggregateOutcome {
        match self {
            Self::InconclusiveEpistemic => AggregateOutcome::InconclusiveEpistemic,
            Self::OverThreshold => AggregateOutcome::OverThreshold,
        }
    }
}

impl From<SettledCount> for AggregateOutcome {
    fn from(settled: SettledCount) -> Self {
        settled.widen()
    }
}

/// Applies the ordered decision table to one trigger Event's observation.
///
/// Read this module's header for why the finding row sits above the absent-count
/// rows and why no row returns `Violate`.
#[must_use]
pub const fn decide(observation: AggregateObservation) -> AggregateOutcome {
    // Row 0. The gate runs before obligation analysis and wins outright, as in
    // every other family's table: it must be able to demote a finding, so it
    // cannot be folded in as another dominated row.
    if matches!(observation.gate, GateState::Closed) {
        return AggregateOutcome::InconclusivePrefix;
    }
    // Rows 1 through 3. A settled cold answer is classified here, and
    // `SettledCount` is why none of these can fall through to `Satisfy`.
    match observation.count {
        CountEvidence::AtOrOverThreshold => return SettledCount::OverThreshold.widen(),
        CountEvidence::Unholdable | CountEvidence::Unasked => {
            return SettledCount::InconclusiveEpistemic.widen()
        }
        // Row 4. A bound guard that has not answered is the prefix case: more
        // evidence — the answer itself — could still settle it either way.
        CountEvidence::Outstanding => return AggregateOutcome::InconclusivePrefix,
        CountEvidence::UnderThreshold => {}
    }
    // Row 5. ADR-0025's licence: the Scope closed at this trigger, the count is
    // settled under threshold, and Evidence Coverage is closed too.
    if matches!(observation.coverage, CoverageState::Settled) {
        return AggregateOutcome::Satisfy;
    }
    // Row 6.
    AggregateOutcome::InconclusivePrefix
}

/// Folds one Binding's per-trigger outcomes into the Binding's conclusion.
///
/// `None` when the Binding raised no obligation — no trigger Event occurred, so
/// there is nothing for a Verdict to be about. That is the place this module
/// refuses a vacuous pass that no row of [`decide`] can reach, and it is the one
/// an `Iterator::all` would get wrong.
#[must_use]
pub fn aggregate(outcomes: impl IntoIterator<Item = AggregateOutcome>) -> Option<AggregateOutcome> {
    outcomes.into_iter().max()
}

/// What the monitor concluded about one Binding, and the evidence it cites.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AggregateAssessment {
    binding: Binding,
    outcome: AggregateOutcome,
    count: CountEvidence,
    triggers: BTreeSet<EventId>,
    members: BTreeSet<EventId>,
    cold_witnesses: BTreeSet<ColdWitness>,
    anchored_events: BTreeSet<EventId>,
}

impl AggregateAssessment {
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
    pub const fn outcome(&self) -> AggregateOutcome {
        self.outcome
    }

    /// Returns the cold-path fact the conclusion rests on.
    #[must_use]
    pub const fn count(&self) -> CountEvidence {
        self.count
    }

    /// Returns the trigger Events that raised an obligation in this Binding.
    #[must_use]
    pub const fn triggers(&self) -> &BTreeSet<EventId> {
        &self.triggers
    }

    /// Returns the member Events the hot path admitted to this Slice.
    ///
    /// **Not the counted population.** The lookback the count is drawn from is
    /// the cold path's retained window, which is wider than the Slice the hot
    /// path holds; these are the Events the Filter admitted, reported so a
    /// reader can see the Specification is matching something.
    #[must_use]
    pub const fn members(&self) -> &BTreeSet<EventId> {
        &self.members
    }

    /// Returns the coordinates a settled cold `Fails` named.
    ///
    /// Stored, never composed. A `ColdWitness` may not enter
    /// `anchored_event_ids` or a `WatermarkAnchoredViolationBasis` — #20 owns
    /// the trust boundary that would let it — so these travel on the read side
    /// only and [`AggregateAssessment::anchored_events`] never contains them.
    #[must_use]
    pub const fn cold_witnesses(&self) -> &BTreeSet<ColdWitness> {
        &self.cold_witnesses
    }

    /// Returns the anchor-tier Events the conclusion rests on.
    #[must_use]
    pub const fn anchored_events(&self) -> &BTreeSet<EventId> {
        &self.anchored_events
    }

    /// Returns the `(witness, trigger)` pairs the evidence leaves unordered.
    ///
    /// Always empty, and that is a statement rather than a stub: the count is
    /// answered by the cold path over a Watermark-consistent lookback, so the
    /// hot path forms no ordering judgement between a member and a trigger and
    /// has no observational uncertainty of its own to report. The accessor
    /// exists because `crate::verdict` reads one from every family.
    #[must_use]
    pub fn concurrent_pairs(&self) -> BTreeSet<(EventId, EventId)> {
        BTreeSet::new()
    }
}

/// Produces the three facts [`decide`] consumes for one Binding's triggers.
///
/// Gate open/closed maps from [`EvidenceGate::is_open`]; coverage maps from
/// [`EvidenceGate::watermarks_cover_epochs`]. Those are two facts and they are
/// not folded together.
#[must_use]
pub fn observe(gate: &EvidenceGate, count: CountEvidence) -> AggregateObservation {
    AggregateObservation::new(
        if gate.is_open() {
            GateState::Open
        } else {
            GateState::Closed
        },
        count,
        if gate.watermarks_cover_epochs() {
            CoverageState::Settled
        } else {
            CoverageState::Open
        },
    )
}

/// Evaluates the Specification over every Binding a trigger Event creates.
///
/// Takes [`FactLedger`] where the other three families take only the Slice and
/// the causal index, and takes no [`crate::frontier::CausalIndex`] at all:
/// this family's falsifier is a cold answer rather than a Slice Event, and it
/// forms no ordering judgement (see
/// [`AggregateAssessment::concurrent_pairs`]). Both differences are the
/// module header's point, made in the signature.
///
/// Bindings are created by trigger Events. An empty trigger set produces no
/// Binding assessment and no Verdict: [`aggregate`] over empty is `None`, and
/// that is what keeps a Slice full of members and no trigger from discharging
/// anything.
#[must_use]
pub fn assess(
    specification: &AggregateSpecification,
    frontier: &Frontier,
    slices: &SliceIndex,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
    facts: &FactLedger,
) -> Vec<AggregateAssessment> {
    let mut assessments = Vec::new();
    for binding in slices.bindings() {
        let Some(members) = slices.members(binding) else {
            continue;
        };
        let slice: Vec<&Event> = members.iter().filter_map(|id| frontier.event(id)).collect();
        let triggers: Vec<&Event> = slices
            .obligation_raising(binding)
            .into_iter()
            .flatten()
            .filter_map(|id| frontier.event(id))
            .collect();
        if triggers.is_empty() {
            continue;
        }
        let closed = facts.closed(binding);
        let count = CountEvidence::of(
            specification.cold_guards(),
            facts.has_queries(binding),
            facts.all_settled(binding),
            &closed,
        );
        assessments.push(assess_binding(
            specification,
            gate,
            ledger,
            binding.clone(),
            &slice,
            &triggers,
            count,
            cold_witnesses(&closed),
        ));
    }
    assessments.sort_by(|left, right| left.binding.cmp(&right.binding));
    assessments
}

/// Collects the coordinates every settled `Fails` named, for the read side only.
///
/// These never reach `anchored_event_ids`; see
/// [`AggregateAssessment::cold_witnesses`].
fn cold_witnesses(closed: &BTreeMap<ColdGuardId, GuardConclusion>) -> BTreeSet<ColdWitness> {
    closed
        .values()
        .flat_map(|conclusion| match conclusion {
            GuardConclusion::Fails { witnesses } => witnesses.clone(),
            GuardConclusion::Holds
            | GuardConclusion::Deferred { .. }
            | GuardConclusion::Refused { .. } => BTreeSet::new(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // One Binding's inputs, each named; a struct would only rename them.
fn assess_binding(
    specification: &AggregateSpecification,
    gate: &EvidenceGate,
    ledger: &EvidenceLedger,
    binding: Binding,
    slice: &[&Event],
    triggers: &[&Event],
    count: CountEvidence,
    cold_witnesses: BTreeSet<ColdWitness>,
) -> AggregateAssessment {
    let observation = observe(gate, count);
    let mut outcomes = Vec::with_capacity(triggers.len());
    let mut anchored_events = BTreeSet::new();
    let mut trigger_ids = BTreeSet::new();

    for trigger in triggers {
        let outcome = decide(observation);
        outcomes.push(outcome);
        trigger_ids.insert(trigger.id());

        if matches!(outcome, AggregateOutcome::Satisfy)
            && ledger.tier_of(&trigger.source()) == Some(SourceTier::Anchor)
        {
            // An Authoritative Satisfy must cite an anchor-tier Event; the
            // trigger is the Event-local Scope close the conclusion rests on.
            // The counted members are not cited: they live in the cold path's
            // lookback, not in this Slice, and a cold coordinate may not anchor
            // a Verdict until #20.
            anchored_events.insert(trigger.id());
        }
    }

    let outcome = aggregate(outcomes).expect("triggers is non-empty");

    if !gate.is_open() {
        anchored_events.clear();
    }

    AggregateAssessment {
        binding,
        outcome,
        count,
        triggers: trigger_ids,
        members: slice
            .iter()
            .filter(|event| specification.member().matches(event))
            .map(|event| event.id())
            .collect(),
        cold_witnesses,
        anchored_events,
    }
}
