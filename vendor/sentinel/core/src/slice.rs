//! Bindings and the Slice index the monitor evaluates over.
//!
//! CONTEXT.md defines a Slice as "the subsequence of the Trace relevant to one
//! Binding, **and the monitor state evaluating it**". Issue #12 builds only the
//! first half, and the type is named for what it is: an *index*, not a Slice.
//! #10's monitor holds no per-Slice state — it recomputes from the admitted set
//! on every assessment — so there is no monitor state to carry here, and
//! inventing one would be #17's work done badly in advance.
//!
//! Membership here is a Trace fact: an Event either belongs to a Binding's Slice
//! or it does not. How it participates — [`SliceRole::Member`] versus
//! [`SliceRole::ObligationRaising`] — is named because a boolean "trigger" flag
//! is a Pattern-family name (CAT-1's selector), and swapping it silently drops
//! the Events a Verdict is about (ADR-0031).
//!
//! #17 therefore inherits the first half — bounding the membership this index
//! holds. Also not built here, with the issue that owns each: coenable GC and
//! enable-set pruning, per-Slice vector clocks, per-Slice reachability and
//! memory bounds (**#17** — and note `frontier.rs` argues reachability must
//! stay global, because the charge-to-ship chain leaves the order Slice and
//! comes back).
//!
//! # Retirement, and why the lifecycle is not in the index
//!
//! Issue #110 (ADR-0021's O-03) adds the **first** per-Slice monitor state this
//! module has ever held: [`SliceLifecycles`], one Binding's `Open` or `Retired`
//! status and the observation-time [`ScopeHorizon`] it retired at. That is
//! deliberately a *separate type* rather than two more maps on [`SliceIndex`],
//! and the reason is mechanical rather than stylistic.
//!
//! `crate::decision`'s enforcement seam reads Slice membership off
//! `crate::evaluator::Evaluator::slices`, at rows 6 and 11 of its table. With
//! the lifecycle held outside [`SliceIndex`] that seam has **no accessor** to
//! it, so "retirement is not a `decide()` input" is enforced by the compiler
//! rather than by a comment. ADR-0031's worked hazard is two adjacent
//! parameters *of the same type*, which `&SliceIndex` and `&SliceLifecycles`
//! are not, so transposing them is already `error[E0308]`.
//!
//! Membership itself remains a Trace fact and remains in [`SliceIndex`]. What
//! retirement changes is which members a **conclusion** may rest on, and that
//! is computed on demand by [`SliceLifecycles::members_in_scope`] — never
//! written back into the index.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{Event, EventId, ObservationTime};

use crate::frontier::Frontier;

/// The value of a Specification's free variable, naming one Slice.
///
/// A newtype rather than a bare `String` so a Binding cannot be silently
/// swapped for a Specification id, a source id or an Event kind.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Binding(String);

impl Binding {
    /// Names one Binding.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the Binding value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Binding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// How an admitted Event participates in a Binding's Slice.
///
/// Closed: a third role is a new fact, not an extension of these two.
/// [`SliceRole::ObligationRaising`] admits into members **and** the raising
/// set. A raising-instead-of-member enum would change Verdicts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SliceRole {
    /// In the Slice; does not raise an obligation.
    Member,
    /// In the Slice, and this Event is the one Event-local Scope evaluates at
    /// (ADR-0025). CAT-1 assessment reads these as trigger Events; CAT-4
    /// reads them as action Events.
    ObligationRaising,
}

/// Per-Binding membership, maintained as Events are admitted.
///
/// Members are kept as identity sets rather than vectors so membership is
/// order-independent: the assessment reads them back in Event-identity order
/// whatever order the deliveries arrived in.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SliceIndex {
    members: BTreeMap<Binding, BTreeSet<EventId>>,
    obligation_raising: BTreeMap<Binding, BTreeSet<EventId>>,
}

impl SliceIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one admitted Event's membership of a Binding's Slice.
    pub fn admit(&mut self, binding: &Binding, event: EventId, role: SliceRole) {
        match role {
            SliceRole::ObligationRaising => {
                self.obligation_raising
                    .entry(binding.clone())
                    .or_default()
                    .insert(event.clone());
            }
            SliceRole::Member => {}
        }
        self.members
            .entry(binding.clone())
            .or_default()
            .insert(event);
    }

    /// Returns every Binding with at least one admitted Event.
    pub fn bindings(&self) -> impl Iterator<Item = &Binding> {
        self.members.keys()
    }

    /// Returns the admitted Events in one Binding's Slice.
    #[must_use]
    pub fn members(&self, binding: &Binding) -> Option<&BTreeSet<EventId>> {
        self.members.get(binding)
    }

    /// Returns the admitted obligation-raising Events in one Binding's Slice.
    #[must_use]
    pub fn obligation_raising(&self, binding: &Binding) -> Option<&BTreeSet<EventId>> {
        self.obligation_raising.get(binding)
    }

    /// Returns how many Bindings the index covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns whether the index covers no Binding.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

/// The observation-time boundary a retired Scope stops accepting evidence at.
///
/// A newtype over [`ObservationTime`], which is Sentinel's own clock coordinate
/// — a `SentinelClockId` and a signed nanosecond count. Producer clocks are
/// deliberately not an option: the corpus records them as unusable for ordering,
/// and the Core reads no clock of its own (ADR-0012).
///
/// **The Core never measures this.** The horizon arrives inside a
/// [`RetirementSignal`], carrying the decision the periphery already made, in
/// exactly the way `crate::evaluator::Input::Source` carries a completeness
/// decision. Everything this type does is comparison, so it adds no arithmetic
/// and therefore no overflow surface to the Core; the periphery's
/// `slice_horizon(b) + window` does have one, and the Core deliberately does not
/// evaluate it.
///
/// # No `Ord`
///
/// [`ObservationTime`] carries no `Ord` and no `PartialOrd`, so `#[derive(Ord)]`
/// here does not compile — and a hand-written lexicographic order over
/// `(SentinelClockId, i128)` would silently *join* horizons taken on different
/// Sentinel clocks. Ordering is defined only within one clock, which is what
/// [`HorizonOrder`] says out loud and what [`ScopeHorizon::compare`] returns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeHorizon(ObservationTime);

/// How one [`ScopeHorizon`] stands relative to another.
///
/// Four-valued because comparison is **partial**: two horizons on different
/// Sentinel clocks are not ordered at all. That is the same refusal to collapse
/// an unknown into a boolean that `sentinel_model`'s
/// `CompletenessFreshness::accepts` makes by returning `None`, and that
/// `LateRelation::DifferentSentinelClock` makes by naming the case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HorizonOrder {
    /// Earlier on the same Sentinel clock.
    Below,
    /// The same instant on the same Sentinel clock.
    Same,
    /// Later on the same Sentinel clock.
    Above,
    /// Taken on a different Sentinel clock, so the two are not ordered.
    IncomparableClock,
}

/// Whether one observation falls inside a retired Slice.
///
/// Three-valued for [`HorizonOrder`]'s reason, and exhaustively matched at its
/// one construction site so a fourth case cannot be silently folded into
/// "excluded".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Admission {
    /// At or below the horizon on the same Sentinel clock.
    InScope,
    /// Above the horizon on the same Sentinel clock.
    OutOfScope,
    /// Observed on a different Sentinel clock, so the horizon orders nothing.
    IncomparableClock,
}

impl ScopeHorizon {
    /// Names the observation-time boundary a Scope retires at.
    #[must_use]
    pub const fn new(observed_at: ObservationTime) -> Self {
        Self(observed_at)
    }

    /// Returns the boundary as an observation time.
    #[must_use]
    pub const fn observed_at(&self) -> &ObservationTime {
        &self.0
    }

    /// Returns how this horizon stands relative to `other`.
    ///
    /// Read as "self is _Above_ other": the incoming horizon is the receiver
    /// and the horizon already recorded is the argument, which is the direction
    /// clause (d3)'s join is written in.
    #[must_use]
    pub fn compare(&self, other: &Self) -> HorizonOrder {
        if self.0.sentinel_clock() != other.0.sentinel_clock() {
            return HorizonOrder::IncomparableClock;
        }
        match self
            .0
            .sentinel_nanoseconds()
            .cmp(&other.0.sentinel_nanoseconds())
        {
            std::cmp::Ordering::Less => HorizonOrder::Below,
            std::cmp::Ordering::Equal => HorizonOrder::Same,
            std::cmp::Ordering::Greater => HorizonOrder::Above,
        }
    }

    /// Returns whether one observation falls inside the retired Slice.
    ///
    /// **The one implementation**, taking the observation rather than the Event
    /// so that it can be exercised from integer literals without building a
    /// Frontier. [`ScopeHorizon::admits`] is the Event-shaped delegate.
    ///
    /// Inclusive at equality, mirroring `classify_lateness`'s
    /// `Ordering::Equal => ObservedAtSignal` and `accepts`' `>=`: an Event
    /// observed at exactly the horizon was observed before the periphery
    /// stopped waiting.
    #[must_use]
    pub fn admits_observation(&self, observed: &ObservationTime) -> Admission {
        if self.0.sentinel_clock() != observed.sentinel_clock() {
            return Admission::IncomparableClock;
        }
        if observed.sentinel_nanoseconds() <= self.0.sentinel_nanoseconds() {
            Admission::InScope
        } else {
            Admission::OutOfScope
        }
    }

    /// Returns whether an Event's own observation falls inside the retired Slice.
    ///
    /// Admissibility is decided by the Event's **observation time** and never by
    /// its arrival position. An Event observed before the horizon is admitted
    /// even when it is delivered long after the retirement signal, which is what
    /// keeps a Verdict a function of the input multiset.
    #[must_use]
    pub fn admits(&self, event: &Event) -> Admission {
        self.admits_observation(&event.observation_time())
    }
}

/// Why a periphery stopped waiting for a Scope to close.
///
/// Exactly two variants, and the omission is the design. A driver-sealed
/// evaluation is *not* a cause: `crate::evaluator::Evaluator::seal` is a
/// universal emission **licence** and not an outcome pin, so nothing may mint a
/// retirement from it. Forbidding that with a rejected variant would be a
/// runtime refusal; leaving the variant out makes it unconstructible, which is
/// ADR-0031 clause 1's own preference and the shape
/// `crate::taint::ConvictedOutcome` already established. `DriverSealed` survives
/// as the documented name for `seal()`'s licence half and as nothing else.
///
/// No `Ord`, and none is needed: a Pattern declares exactly one retirement
/// policy, and clause (d0) requires a signal's cause to be that one, so two
/// signals with different causes cannot both be admissible on one evaluator.
/// The same-horizon-different-cause case is unconstructible rather than merely
/// ordered, so the join is over the horizon alone.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RetirementCause {
    /// The periphery's declared deadline passed.
    Deadline,
    /// The periphery's declared quiet window elapsed with no further Event.
    Inactivity,
}

impl RetirementCause {
    /// Returns the cause's stable identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Inactivity => "inactivity",
        }
    }
}

/// One Binding's retirement: the horizon it retired at, and why.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Retirement {
    horizon: ScopeHorizon,
    cause: RetirementCause,
}

impl Retirement {
    /// Records a retirement at one horizon, for one cause.
    #[must_use]
    pub const fn new(horizon: ScopeHorizon, cause: RetirementCause) -> Self {
        Self { horizon, cause }
    }

    /// Returns the horizon this Scope retired at.
    #[must_use]
    pub const fn horizon(&self) -> &ScopeHorizon {
        &self.horizon
    }

    /// Returns why the periphery stopped waiting.
    #[must_use]
    pub const fn cause(&self) -> RetirementCause {
        self.cause
    }

    /// Returns the O-03 disclosure sentence for the Assurance Envelope.
    ///
    /// Deterministic in `(cause, clock, nanoseconds)` and containing nothing
    /// Slice-derived, so it is a function of the input multiset. `sentinel_model`
    /// has no structured `InconclusiveReason` for retirement — the enum is
    /// proto-generated and regeneration is a networked maintenance boundary no
    /// gate crosses — so the only vehicle is the envelope's free-text detail.
    /// The signal is therefore unstructured, and that is recorded as a
    /// limitation rather than discovered downstream.
    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "O-03 scope retired: {} at {}:{}",
            self.cause.as_str(),
            self.horizon.observed_at().sentinel_clock(),
            self.horizon.observed_at().sentinel_nanoseconds()
        )
    }
}

/// One Binding's Scope lifecycle.
///
/// `Open` is the default and is not stored: a Binding with no entry in
/// [`SliceLifecycles`] is open. `Open -> Retired` is the only transition into
/// the retired set, and once there the horizon only ever joins upward.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SliceLifecycle {
    /// The Scope has not been retired; every admitted member is in scope.
    Open,
    /// The Scope was retired, and only members at or below the horizon count.
    Retired(Retirement),
}

/// A periphery's statement that it has stopped waiting for one Scope to close.
///
/// The Core's third kind of input, beside an Event and a source's evidence. It
/// carries a decision rather than asking for one: nothing here is measured, and
/// the Core's whole contribution is to compare the horizon against each Event's
/// own observation time.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RetirementSignal {
    binding: Binding,
    horizon: ScopeHorizon,
    cause: RetirementCause,
}

impl RetirementSignal {
    /// Declares that one Binding's Scope has been retired.
    #[must_use]
    pub const fn new(binding: Binding, horizon: ScopeHorizon, cause: RetirementCause) -> Self {
        Self {
            binding,
            horizon,
            cause,
        }
    }

    /// Returns the Binding whose Scope retired.
    #[must_use]
    pub const fn binding(&self) -> &Binding {
        &self.binding
    }

    /// Returns the horizon the Scope retired at.
    #[must_use]
    pub const fn horizon(&self) -> &ScopeHorizon {
        &self.horizon
    }

    /// Returns why the periphery stopped waiting.
    #[must_use]
    pub const fn cause(&self) -> RetirementCause {
        self.cause
    }
}

/// Why a retirement signal is inadmissible.
///
/// A breach is a `crate::CoreError`, not a degrade, on the same grounds as
/// `crate::evidence`'s clauses (a), (b) and (c): the Core is being told
/// something contradictory about the evidence, and quietly absorbing it would
/// resolve uncertainty by fabrication. Every breach is raised **before** any
/// state is mutated, so a rejected signal leaves the evaluator usable.
///
/// There is deliberately no `HorizonBelowObservation` and no `UnknownBinding`.
/// The first rejected a horizon sitting below an already-*delivered*
/// observation, which is arrival-position reasoning; its diagnostic value
/// survives as an assessment-time disclosure. The second would have rejected a
/// Binding the Core has not yet seen an Event for, which is order-sensitive on
/// `{Retire(b), e_of_b}` — recording a lifecycle for an unseen Binding is
/// admissible and simply produces no Verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RetirementBreach {
    /// The Specification declares no retirement policy at all.
    NotDeclared,
    /// The signal's cause is not the one the Specification declares.
    CauseNotDeclared,
    /// The Binding has already emitted a Verdict, so retiring it would retract.
    BindingAlreadyEmitted,
    /// The incoming horizon sits below the horizon already recorded.
    HorizonRegressed,
    /// The incoming horizon is on a different Sentinel clock.
    IncomparableHorizon,
}

impl RetirementBreach {
    /// Returns a stable, human-readable description.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::NotDeclared => "the Specification declares no retirement policy",
            Self::CauseNotDeclared => {
                "the retirement cause is not the one the Specification declares"
            }
            Self::BindingAlreadyEmitted => {
                "the Binding has already emitted, so retiring it would retract a Verdict"
            }
            Self::HorizonRegressed => "the retirement horizon is earlier than the one recorded",
            Self::IncomparableHorizon => {
                "the retirement horizon is on a different Sentinel clock than the one recorded"
            }
        }
    }
}

/// What one admissible retirement signal did to a Binding's lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Transition {
    /// The Binding was open and is now retired.
    Retired,
    /// The Binding was already retired and its horizon advanced.
    HorizonAdvanced,
    /// The signal restated the recorded horizon and cause: an idempotent no-op.
    AlreadyRetired,
}

/// Every Binding's Scope lifecycle, held outside [`SliceIndex`].
///
/// See the module header for why this is a separate type. Only retired Bindings
/// are stored; an absent entry is [`SliceLifecycle::Open`], so a retirement-free
/// evaluation carries an empty map and
/// [`SliceLifecycles::any_retired`] is `false` — which is what keeps its control
/// flow byte-identical to an evaluator that had never heard of O-03.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SliceLifecycles(BTreeMap<Binding, Retirement>);

impl SliceLifecycles {
    /// Creates a lifecycle map in which every Binding is open.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one retirement signal, or refuses it.
    ///
    /// **Clause (d3) then clause (d2), and both before any mutation.** The
    /// order is load-bearing rather than incidental: (d2) rejects a Binding that
    /// has already emitted, and a retired Binding has emitted by the time any
    /// second signal arrives, so checking (d2) first would make idempotence
    /// unreachable. (d3) therefore short-circuits first, and (d2) governs only
    /// the `Open -> Retired` transition.
    ///
    /// Clause (d0) — the Specification declares a retirement policy, and this
    /// cause is the one it declares — is the caller's, because the declaration
    /// is the Specification's and not the Slice's. It runs *before* this call.
    ///
    /// The horizon **joins upward** rather than freezing at entry. A frozen
    /// horizon would leave `{Retire(t1), Retire(t2)}` recoverably order
    /// dependent: both orders reject the second signal, but a driver that
    /// continues to `seal()` would read a different Slice in each. The join
    /// ends at the later horizon either way.
    ///
    /// # Errors
    ///
    /// Returns [`RetirementBreach::HorizonRegressed`] for an earlier horizon,
    /// [`RetirementBreach::IncomparableHorizon`] for one on a different Sentinel
    /// clock, and [`RetirementBreach::BindingAlreadyEmitted`] when an open
    /// Binding that has already emitted is retired.
    pub fn retire(
        &mut self,
        signal: &RetirementSignal,
        already_emitted: bool,
    ) -> Result<Transition, RetirementBreach> {
        // (d3). Validated in full before anything is written.
        let transition = match self.0.get(signal.binding()) {
            Some(recorded) => match signal.horizon().compare(recorded.horizon()) {
                HorizonOrder::Same => return Ok(Transition::AlreadyRetired),
                HorizonOrder::Above => Transition::HorizonAdvanced,
                HorizonOrder::Below => return Err(RetirementBreach::HorizonRegressed),
                HorizonOrder::IncomparableClock => {
                    return Err(RetirementBreach::IncomparableHorizon)
                }
            },
            None => Transition::Retired,
        };
        // (d2), scoped to the transition into the retired set. Advancing an
        // already-retired Binding's horizon re-emits nothing, so it cannot
        // retract a Verdict and is not this clause's business.
        if transition == Transition::Retired && already_emitted {
            return Err(RetirementBreach::BindingAlreadyEmitted);
        }
        self.0.insert(
            signal.binding().clone(),
            Retirement::new(signal.horizon().clone(), signal.cause()),
        );
        Ok(transition)
    }

    /// Returns one Binding's lifecycle.
    #[must_use]
    pub fn lifecycle_of(&self, binding: &Binding) -> SliceLifecycle {
        self.0.get(binding).map_or(SliceLifecycle::Open, |retired| {
            SliceLifecycle::Retired(retired.clone())
        })
    }

    /// Returns one Binding's retirement, when it has one.
    #[must_use]
    pub fn retirement_of(&self, binding: &Binding) -> Option<&Retirement> {
        self.0.get(binding)
    }

    /// Returns whether one Binding has retired.
    #[must_use]
    pub fn is_retired(&self, binding: &Binding) -> bool {
        self.0.contains_key(binding)
    }

    /// Returns whether any Binding has retired.
    ///
    /// Read by `crate::evaluator::Evaluator::advance` as the guard on the whole
    /// retired emission path: while it is `false` the evaluator's control flow
    /// reduces to exactly what it was before O-03, which is the property the
    /// untouched suites prove.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns whether at least one Binding has retired.
    #[must_use]
    pub fn any_retired(&self) -> bool {
        !self.0.is_empty()
    }

    /// Returns the members of one Binding's Slice a conclusion may rest on.
    ///
    /// Every admitted member for an open Binding; the members at or below the
    /// horizon for a retired one. **Recomputed on demand, never written back**
    /// into [`SliceIndex`]: membership is a Trace fact and retirement is not,
    /// so folding the horizon into admission would make the index a function of
    /// arrival order.
    ///
    /// A member the Frontier cannot resolve is skipped, exactly as
    /// `crate::precedence::assess` skips it.
    #[must_use]
    pub fn members_in_scope(
        &self,
        binding: &Binding,
        slices: &SliceIndex,
        frontier: &Frontier,
    ) -> ScopeMembership {
        let members = slices.members(binding).cloned().unwrap_or_default();
        let Some(retired) = self.0.get(binding) else {
            return ScopeMembership {
                in_scope: members,
                above_horizon: 0,
                incomparable_clock: 0,
            };
        };
        let mut in_scope = BTreeSet::new();
        let mut above_horizon = 0_usize;
        let mut incomparable_clock = 0_usize;
        for id in &members {
            let Some(event) = frontier.event(id) else {
                continue;
            };
            match retired.horizon().admits(event) {
                Admission::InScope => {
                    in_scope.insert(id.clone());
                }
                Admission::OutOfScope => above_horizon += 1,
                Admission::IncomparableClock => incomparable_clock += 1,
            }
        }
        ScopeMembership {
            in_scope,
            above_horizon,
            incomparable_clock,
        }
    }
}

/// One Binding's Slice as a retirement horizon leaves it.
///
/// The excluded members are **counted, not discarded silently**: a Verdict that
/// dropped evidence without saying so is the failure mode retirement is designed
/// against. The two exclusion counts are kept apart because they are different
/// facts — observed after the horizon, versus not comparable with it at all.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopeMembership {
    in_scope: BTreeSet<EventId>,
    above_horizon: usize,
    incomparable_clock: usize,
}

impl ScopeMembership {
    /// Returns the members a conclusion may rest on.
    #[must_use]
    pub const fn in_scope(&self) -> &BTreeSet<EventId> {
        &self.in_scope
    }

    /// Returns how many members were observed above the horizon.
    #[must_use]
    pub const fn above_horizon(&self) -> usize {
        self.above_horizon
    }

    /// Returns how many members the horizon cannot order at all.
    #[must_use]
    pub const fn incomparable_clock(&self) -> usize {
        self.incomparable_clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_model::SentinelClockId;

    fn id(value: &str) -> EventId {
        EventId::new(value).expect("valid id")
    }

    /// Obligation-raising Events stay Slice members; Member Events stay out of
    /// the raising set. Poisoning the `ObligationRaising` arm (treating every
    /// admit as `Member`) fails this test.
    #[test]
    fn obligation_raising_contains_exactly_the_events_admitted_as_obligation_raising() {
        let mut slices = SliceIndex::new();
        let binding = Binding::new("order");
        slices.admit(&binding, id("member-only"), SliceRole::Member);
        slices.admit(&binding, id("raising"), SliceRole::ObligationRaising);

        let members = slices.members(&binding).expect("the Binding has members");
        assert!(members.contains(&id("member-only")));
        assert!(members.contains(&id("raising")));
        assert_eq!(members.len(), 2);

        let raising = slices
            .obligation_raising(&binding)
            .expect("the Binding has an obligation-raising Event");
        assert!(raising.contains(&id("raising")));
        assert!(!raising.contains(&id("member-only")));
        assert_eq!(raising.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Retirement (#110, ADR-0021's O-03)
    // -----------------------------------------------------------------------

    /// The Sentinel clock every horizon in this module's tests is taken on.
    const CLOCK: &str = "sentinel-a";

    /// Builds an observation time from integer literals.
    ///
    /// Every observation time in this crate's tests is built this way, on
    /// purpose. `.github/scripts/check-core-purity.sh` scans `core/src` for
    /// clock reads and would catch one here, but the same discipline has to
    /// hold in `core/tests`, which it does not scan — so the habit is
    /// established where the gate is, not only where it is missing.
    fn observed(clock: &str, nanoseconds: i128) -> ObservationTime {
        ObservationTime::new(
            SentinelClockId::new(clock).expect("valid Sentinel clock"),
            nanoseconds,
        )
    }

    fn horizon(nanoseconds: i128) -> ScopeHorizon {
        ScopeHorizon::new(observed(CLOCK, nanoseconds))
    }

    fn signal(binding: &str, nanoseconds: i128, cause: RetirementCause) -> RetirementSignal {
        RetirementSignal::new(Binding::new(binding), horizon(nanoseconds), cause)
    }

    /// Deterministic pseudo-values, so the sweep below is reproducible.
    ///
    /// A 64-bit linear congruential generator with glibc's constants, seeded by
    /// the caller. The Core reads no clock and links no generator crate; this is
    /// arithmetic over a literal seed.
    fn next_value(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *state >> 33
    }

    /// Every observation lands in exactly one of the three admission cases, and
    /// all three are reachable.
    ///
    /// The point of the third case is that it exists at all. Collapsing
    /// "different Sentinel clock" into "out of scope" would report an ordering
    /// the evidence does not supply, which is the two-valued collapse
    /// `LateRelation::DifferentSentinelClock` exists to prevent.
    #[test]
    fn admission_is_total() {
        let boundary = horizon(1_000);

        for (observation, expected) in [
            (observed(CLOCK, 999), Admission::InScope),
            (observed(CLOCK, 1_000), Admission::InScope),
            (observed(CLOCK, 1_001), Admission::OutOfScope),
            (observed("sentinel-b", 1), Admission::IncomparableClock),
            (observed("sentinel-b", 10_000), Admission::IncomparableClock),
        ] {
            assert_eq!(
                boundary.admits_observation(&observation),
                expected,
                "{}:{} against the horizon",
                observation.sentinel_clock(),
                observation.sentinel_nanoseconds()
            );
        }
    }

    /// Membership only ever grows as the horizon advances.
    ///
    /// This is what makes the (d3) join safe: advancing a horizon can admit
    /// evidence, never withdraw it, so a retired Binding's Slice is monotone in
    /// the join and the join is therefore order-free within one Sentinel clock.
    #[test]
    fn retired_membership_is_monotone_in_the_horizon() {
        let mut state = 0x5eed_0110_u64;
        let observations: Vec<ObservationTime> = (0..64)
            .map(|_| observed(CLOCK, i128::from(next_value(&mut state) % 2_048)))
            .collect();

        let mut boundaries: Vec<i128> = (0..24)
            .map(|_| i128::from(next_value(&mut state) % 2_048))
            .collect();
        boundaries.sort_unstable();

        for pair in boundaries.windows(2) {
            let lower = horizon(pair[0]);
            let upper = horizon(pair[1]);
            for observation in &observations {
                if lower.admits_observation(observation) == Admission::InScope {
                    assert_eq!(
                        upper.admits_observation(observation),
                        Admission::InScope,
                        "advancing the horizon from {} to {} withdrew {}",
                        pair[0],
                        pair[1],
                        observation.sentinel_nanoseconds()
                    );
                }
            }
        }
    }

    /// Comparison is partial, and says so rather than inventing an order.
    #[test]
    fn horizons_on_different_sentinel_clocks_are_incomparable() {
        let here = horizon(10);
        let elsewhere = ScopeHorizon::new(observed("sentinel-b", 10));

        assert_eq!(here.compare(&elsewhere), HorizonOrder::IncomparableClock);
        assert_eq!(elsewhere.compare(&here), HorizonOrder::IncomparableClock);
        assert_eq!(here.compare(&horizon(10)), HorizonOrder::Same);
        assert_eq!(here.compare(&horizon(9)), HorizonOrder::Above);
        assert_eq!(here.compare(&horizon(11)), HorizonOrder::Below);
    }

    /// Clause (d3): same is a no-op, later joins, earlier is refused, and a
    /// different clock is refused as incomparable.
    #[test]
    fn the_horizon_joins_upward_and_refuses_a_regression() {
        let mut lifecycles = SliceLifecycles::new();
        let binding = Binding::new("order-1");
        assert!(!lifecycles.any_retired());

        assert_eq!(
            lifecycles.retire(&signal("order-1", 100, RetirementCause::Deadline), false),
            Ok(Transition::Retired)
        );
        assert!(lifecycles.any_retired());
        assert!(lifecycles.is_retired(&binding));

        assert_eq!(
            lifecycles.retire(&signal("order-1", 100, RetirementCause::Deadline), true),
            Ok(Transition::AlreadyRetired)
        );
        assert_eq!(
            lifecycles.retire(&signal("order-1", 200, RetirementCause::Deadline), true),
            Ok(Transition::HorizonAdvanced)
        );
        assert_eq!(
            lifecycles.retire(&signal("order-1", 150, RetirementCause::Deadline), true),
            Err(RetirementBreach::HorizonRegressed)
        );
        assert_eq!(
            lifecycles.retire(
                &RetirementSignal::new(
                    binding.clone(),
                    ScopeHorizon::new(observed("sentinel-b", 900)),
                    RetirementCause::Deadline,
                ),
                true,
            ),
            Err(RetirementBreach::IncomparableHorizon)
        );

        // Every refusal left the recorded state exactly where the join put it.
        let SliceLifecycle::Retired(retired) = lifecycles.lifecycle_of(&binding) else {
            panic!("the Binding is retired");
        };
        assert_eq!(retired.horizon(), &horizon(200));
        assert_eq!(retired.cause(), RetirementCause::Deadline);
    }

    /// Clause (d2) governs the transition into the retired set, and only that.
    ///
    /// Retiring an already-emitted Binding is refused, because it would replace
    /// a Verdict the driver has already seen. Advancing an already-retired
    /// Binding's horizon is not refused: it re-emits nothing, so it retracts
    /// nothing.
    #[test]
    fn only_the_transition_into_the_retired_set_refuses_an_emitted_binding() {
        let mut lifecycles = SliceLifecycles::new();

        assert_eq!(
            lifecycles.retire(&signal("order-1", 100, RetirementCause::Inactivity), true),
            Err(RetirementBreach::BindingAlreadyEmitted)
        );
        // The refusal mutated nothing.
        assert!(!lifecycles.any_retired());
        assert_eq!(
            lifecycles.lifecycle_of(&Binding::new("order-1")),
            SliceLifecycle::Open
        );

        assert_eq!(
            lifecycles.retire(&signal("order-1", 100, RetirementCause::Inactivity), false),
            Ok(Transition::Retired)
        );
        assert_eq!(
            lifecycles.retire(&signal("order-1", 300, RetirementCause::Inactivity), true),
            Ok(Transition::HorizonAdvanced)
        );
    }

    /// Two signals in either order reach the same recorded state.
    ///
    /// The join is `max`, which is commutative — within one Sentinel clock,
    /// which is the only place `ScopeHorizon::compare` is total.
    #[test]
    fn a_pair_of_signals_reaches_the_same_state_in_either_order() {
        let mut ascending = SliceLifecycles::new();
        ascending
            .retire(&signal("order-1", 100, RetirementCause::Deadline), false)
            .expect("admissible");
        ascending
            .retire(&signal("order-1", 400, RetirementCause::Deadline), false)
            .expect("admissible");

        let mut descending = SliceLifecycles::new();
        descending
            .retire(&signal("order-1", 400, RetirementCause::Deadline), false)
            .expect("admissible");
        assert_eq!(
            descending.retire(&signal("order-1", 100, RetirementCause::Deadline), false),
            Err(RetirementBreach::HorizonRegressed)
        );

        assert_eq!(ascending, descending);
    }

    /// The O-03 sentence names the cause, the clock and the horizon, and
    /// nothing Slice-derived.
    #[test]
    fn the_o03_detail_is_a_function_of_the_cause_and_the_horizon() {
        let retirement = Retirement::new(horizon(1_700), RetirementCause::Inactivity);
        assert_eq!(
            retirement.detail(),
            "O-03 scope retired: inactivity at sentinel-a:1700"
        );
        assert_eq!(
            Retirement::new(horizon(1_700), RetirementCause::Deadline).detail(),
            "O-03 scope retired: deadline at sentinel-a:1700"
        );
    }
}
