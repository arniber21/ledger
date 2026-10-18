//! Causal reachability and the causally-ready admission frontier.
//!
//! [`CausalIndex`] adds the one thing `sentinel_model` deliberately does not
//! provide: **transitive** reachability. `sentinel_model::causal_relation`
//! classifies only the direct pairwise edge, because a pairwise function cannot
//! see a path. The Core owns the closure, and therefore owns rejecting a cycle
//! that only shows up transitively.
//!
//! [`Frontier`] is CONTEXT.md's frontier: "the set of maximal Events per causal
//! source in a seen prefix, closed under happens-before". It admits an Event
//! only when every declared causal predecessor is already admitted, holding it
//! and retrying otherwise. That hold-and-retry is what makes the Core advance
//! causally ready frontiers rather than consume a linearized stream, and it is
//! what makes an unresolved causal edge visible as evidence instead of silently
//! disappearing.
//!
//! Per-Slice vector clocks and per-Slice reachability are **#17**
//! optimisations and are deliberately not built here.
//!
//! # Rebuild timing versus incrementality (issue #12)
//!
//! [`CausalIndex::build`] is a full rebuild of the transitive closure over the
//! whole *delivered* set, and the resumable evaluator rebuilds it **eagerly on
//! every step whose delivered set changed** — `O(|delivered|²)` per step where
//! the batch path paid it once.
//!
//! That cost is accepted deliberately, because the alternative is unsound
//! rather than merely slower. Deferring the rebuild until a Verdict is demanded
//! would let a stepping driver emit settled Verdicts and only discover at
//! `seal()` that the declared edges admit no partial order at all — and with no
//! Amendment record anywhere in `proto/` or `model/`, those emissions could not
//! be retracted. **Detection timing is not deferred; only incrementality is,
//! and it belongs to #17**, which is literally that issue's subject.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::{
    classify_delivery_identity, DeliveryIdentityRelation, Event, EventId, SourceIdentity,
};

/// Causal relationship between two Events under transitive reachability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CausalOrder {
    /// The left Event is in the right Event's causal past.
    Before,
    /// The right Event is in the left Event's causal past.
    After,
    /// Neither Event is in the other's causal past.
    Concurrent,
}
/// Whether an Event is in the causal past of a hypothetical, not-yet-delivered Event.
///
/// [`CausalIndex::reaches`] classifies only Events the index covers. The
/// enforcement decision seam (issue #22) must classify the **asked action** —
/// an Event that has not been delivered — against the index, using only the
/// causal predecessors the action declares. That question needs a
/// three-valued answer: the "cannot classify" case is evidence about the
/// action's declared past, not a settled negative.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HypotheticalReach {
    /// The Event is a declared predecessor, or in one's transitive past.
    Reaches,
    /// Every declared predecessor is indexed, and none reaches the Event.
    NotReaches,
    /// A declared predecessor names an Event the index never saw, so the
    /// action's declared past cannot be classified from this evidence.
    PastIncomplete,
}

/// Failure building the transitive reachability index.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CausalIndexError {
    /// The declared causal edges contain a cycle, so no partial order exists.
    ContradictoryCycle {
        /// An Event that reaches itself.
        event: EventId,
    },
}

impl Display for CausalIndexError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContradictoryCycle { event } => write!(
                formatter,
                "declared causal edges form a cycle through {event}"
            ),
        }
    }
}

impl Error for CausalIndexError {}

/// Memoized transitive causal past over a set of Events.
///
/// Reachability is computed over the **whole** Event set rather than per Slice,
/// because a cross-service precedence path traverses spans outside the Binding's
/// own Slice: the charge-to-ship chain in the testbed leaves the order Slice and
/// comes back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CausalIndex {
    positions: BTreeMap<EventId, usize>,
    ancestors: Vec<BTreeSet<usize>>,
}

impl CausalIndex {
    /// Builds the transitive causal past of every Event.
    ///
    /// Declared predecessors that name an Event outside `events` are skipped:
    /// a dangling edge cannot order anything, and it is surfaced as evidence by
    /// [`Frontier::unresolved_causal_edges`] rather than by an ordering claim.
    ///
    /// # Errors
    ///
    /// Returns [`CausalIndexError::ContradictoryCycle`] when an Event reaches
    /// itself, which means the declared edges admit no partial order at all.
    ///
    /// # Panics
    ///
    /// Panics only if the resolver leaves a position unresolved without
    /// reporting a cycle, which its own loop makes impossible.
    pub fn build(events: &[Event]) -> Result<Self, CausalIndexError> {
        let mut positions = BTreeMap::new();
        for (position, event) in events.iter().enumerate() {
            positions.insert(event.id(), position);
        }

        let direct: Vec<BTreeSet<usize>> = events
            .iter()
            .map(|event| {
                event
                    .causal_predecessors()
                    .iter()
                    .filter_map(|predecessor| positions.get(predecessor).copied())
                    .collect()
            })
            .collect();

        let mut ancestors: Vec<Option<BTreeSet<usize>>> = vec![None; events.len()];
        for start in 0..events.len() {
            resolve(
                start,
                &direct,
                &mut ancestors,
                &mut vec![false; events.len()],
            )
            .map_err(|position| CausalIndexError::ContradictoryCycle {
                event: events[position].id(),
            })?;
        }

        Ok(Self {
            positions,
            ancestors: ancestors
                .into_iter()
                .map(|resolved| resolved.expect("every position was resolved"))
                .collect(),
        })
    }

    /// Returns whether `earlier` is in `later`'s transitive causal past.
    #[must_use]
    pub fn reaches(&self, earlier: &EventId, later: &EventId) -> bool {
        let (Some(earlier), Some(later)) = (self.positions.get(earlier), self.positions.get(later))
        else {
            return false;
        };
        self.ancestors[*later].contains(earlier)
    }

    /// Classifies two Events under transitive reachability.
    ///
    /// Events absent from the index are [`CausalOrder::Concurrent`]: an Event
    /// the index never saw orders nothing.
    #[must_use]
    pub fn classify(&self, left: &EventId, right: &EventId) -> CausalOrder {
        if self.reaches(left, right) {
            CausalOrder::Before
        } else if self.reaches(right, left) {
            CausalOrder::After
        } else {
            CausalOrder::Concurrent
        }
    }

    /// Returns how many Events the index covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ancestors.len()
    }

    /// Returns whether `earlier` is in the transitive causal past of a
    /// hypothetical Event whose declared direct predecessors are `declared`.
    ///
    /// [`HypotheticalReach::PastIncomplete`] the moment any declared
    /// predecessor is absent from the index — an unclassifiable past is not a
    /// settled negative. [`HypotheticalReach::Reaches`] when `earlier` is one
    /// of the declared predecessors or in any of their ancestor sets;
    /// [`HypotheticalReach::NotReaches`] otherwise. An empty `declared` set is
    /// vacuously complete and reaches nothing.
    ///
    /// Resolution runs over the same index the Verdict surface reads — the
    /// delivered set, held Events included, their own dangling edges skipped —
    /// so a path through a delivered-but-held intermediary is a real path
    /// here. That is the decision seam's deliberate trust position (issue
    /// #22 §3.1), recorded rather than implied.
    #[must_use]
    pub fn reaches_from_declared(
        &self,
        earlier: &EventId,
        declared: &[EventId],
    ) -> HypotheticalReach {
        // Completeness is decided over the whole declared set before any
        // reachability is read, so the answer cannot depend on the order the
        // predecessors happen to be listed in: one absent predecessor makes
        // the past unclassifiable, which is evidence about the action, not a
        // fact about any single edge.
        for predecessor in declared {
            if !self.positions.contains_key(predecessor) {
                return HypotheticalReach::PastIncomplete;
            }
        }
        let earlier_position = self.positions.get(earlier);
        for predecessor in declared {
            let position = self.positions[predecessor];
            let reaches = earlier_position.is_some_and(|&earlier| {
                earlier == position || self.ancestors[position].contains(&earlier)
            });
            if reaches {
                return HypotheticalReach::Reaches;
            }
        }
        HypotheticalReach::NotReaches
    }

    /// Returns whether the index covers no Events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ancestors.is_empty()
    }
}

/// Resolves one position's transitive ancestors, reporting the position of a cycle.
fn resolve(
    position: usize,
    direct: &[BTreeSet<usize>],
    ancestors: &mut [Option<BTreeSet<usize>>],
    visiting: &mut Vec<bool>,
) -> Result<(), usize> {
    if ancestors[position].is_some() {
        return Ok(());
    }
    if visiting[position] {
        return Err(position);
    }
    visiting[position] = true;
    let mut resolved = BTreeSet::new();
    for &predecessor in &direct[position] {
        if predecessor == position {
            return Err(position);
        }
        resolve(predecessor, direct, ancestors, visiting)?;
        resolved.insert(predecessor);
        resolved.extend(
            ancestors[predecessor]
                .as_ref()
                .expect("predecessor was resolved")
                .iter()
                .copied(),
        );
    }
    if resolved.contains(&position) {
        return Err(position);
    }
    visiting[position] = false;
    ancestors[position] = Some(resolved);
    Ok(())
}

/// Failure admitting a delivered Event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FrontierError {
    /// Two deliveries conflict on identity, slot, or evidence.
    DeliveryConflict {
        /// How the two deliveries conflict.
        relation: DeliveryIdentityRelation,
        /// The Event being delivered.
        delivered: EventId,
        /// The Event already held for the same identity or slot.
        existing: EventId,
    },
}

impl Display for FrontierError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeliveryConflict {
                relation,
                delivered,
                existing,
            } => write!(
                formatter,
                "delivery of {delivered} conflicts with {existing}: {relation:?}"
            ),
        }
    }
}

impl Error for FrontierError {}

type SlotKey = (SourceIdentity, u64, u64);

fn slot_key(event: &Event) -> SlotKey {
    let slot = event.delivery();
    (
        slot.source().clone(),
        slot.epoch().value(),
        slot.sequence().value(),
    )
}

/// What one delivery changed about the frontier.
///
/// The newly-admitted set is what lets `SliceIndex` be maintained as Events
/// arrive instead of being rebuilt by rescanning the admitted set. It is a
/// *set*, so it stays order-independent: admission order within one `advance`
/// is not recorded anywhere.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct Admission {
    delivered: bool,
    newly_admitted: BTreeSet<EventId>,
}

impl Admission {
    /// Returns whether the delivery occupied a slot for the first time.
    ///
    /// False exactly when the delivery was suppressed as a duplicate, which is
    /// what keeps the evidence ledger's distinct-slot counter honest.
    #[must_use]
    pub const fn is_new_delivery(&self) -> bool {
        self.delivered
    }

    /// Returns the Events this delivery made causally ready.
    #[must_use]
    pub const fn newly_admitted(&self) -> &BTreeSet<EventId> {
        &self.newly_admitted
    }
}

/// The causally-ready admission frontier over a delivered Event prefix.
#[derive(Clone, Debug, Default)]
pub struct Frontier {
    delivered: BTreeMap<EventId, Event>,
    by_slot: BTreeMap<SlotKey, EventId>,
    admitted: BTreeSet<EventId>,
    duplicates: usize,
}

impl Frontier {
    /// Creates an empty frontier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Delivers one Event, admitting whatever became causally ready.
    ///
    /// A duplicate delivery is dropped, so replay is idempotent. Every other
    /// identity or slot conflict is an error rather than a silent preference
    /// for one of two contradictory claims.
    ///
    /// # Errors
    ///
    /// Returns [`FrontierError::DeliveryConflict`] for an evidence, slot, or
    /// identity conflict.
    pub fn step(&mut self, event: Event) -> Result<Admission, FrontierError> {
        let id = event.id();
        let existing = self
            .delivered
            .get(&id)
            .or_else(|| {
                self.by_slot
                    .get(&slot_key(&event))
                    .and_then(|id| self.delivered.get(id))
            })
            .cloned();

        if let Some(existing) = existing {
            match classify_delivery_identity(&existing, &event) {
                DeliveryIdentityRelation::Duplicate => {
                    self.duplicates += 1;
                    return Ok(Admission::default());
                }
                DeliveryIdentityRelation::Distinct => {}
                relation => {
                    return Err(FrontierError::DeliveryConflict {
                        relation,
                        delivered: id,
                        existing: existing.id(),
                    })
                }
            }
        }

        self.by_slot.insert(slot_key(&event), id.clone());
        self.delivered.insert(id, event);
        Ok(Admission {
            delivered: true,
            newly_admitted: self.advance(),
        })
    }

    /// Admits every held Event whose declared predecessors are all admitted.
    ///
    /// Nothing is recorded during admission. The admitted *set* is
    /// order-independent at fixpoint, but any tie-break recorded while retrying
    /// would not be, so the retry loop records nothing but membership — and the
    /// set it returns is likewise a set, for the same reason.
    fn advance(&mut self) -> BTreeSet<EventId> {
        let mut newly_admitted = BTreeSet::new();
        loop {
            let ready: Vec<EventId> = self
                .delivered
                .iter()
                .filter(|(id, event)| {
                    !self.admitted.contains(*id)
                        && event
                            .causal_predecessors()
                            .iter()
                            .all(|predecessor| self.admitted.contains(predecessor))
                })
                .map(|(id, _)| id.clone())
                .collect();
            if ready.is_empty() {
                return newly_admitted;
            }
            newly_admitted.extend(ready.iter().cloned());
            self.admitted.extend(ready);
        }
    }

    /// Returns one delivered Event by identity.
    #[must_use]
    pub fn event(&self, id: &EventId) -> Option<&Event> {
        self.delivered.get(id)
    }

    /// Returns the admitted Events, ordered by Event identity.
    #[must_use]
    pub fn admitted(&self) -> Vec<Event> {
        self.admitted
            .iter()
            .map(|id| self.delivered[id].clone())
            .collect()
    }

    /// Returns every delivered Event after duplicate suppression, by identity.
    #[must_use]
    pub fn delivered(&self) -> Vec<Event> {
        self.delivered.values().cloned().collect()
    }

    /// Iterates every delivered Event after duplicate suppression, by
    /// identity, borrowing rather than cloning.
    ///
    /// The enforcement decision seam's row-8 scan reads identity, Binding,
    /// requirement match and ordering off every delivered Event once per
    /// matched Specification, on the synchronous decision call ADR-0023 puts
    /// on every ask-and-block action's critical path — so it must not pay
    /// for an owned copy of the whole delivered set per entry. `pub(crate)`:
    /// the seam is the only consumer; [`Frontier::delivered`] stays the
    /// owned shape every other caller had.
    pub(crate) fn delivered_events(&self) -> impl Iterator<Item = &Event> {
        self.delivered.values()
    }

    /// Returns whether an Event was admitted.
    #[must_use]
    pub fn is_admitted(&self, id: &EventId) -> bool {
        self.admitted.contains(id)
    }

    /// Returns delivered Events that never became causally ready.
    #[must_use]
    pub fn held(&self) -> BTreeSet<EventId> {
        self.delivered
            .keys()
            .filter(|id| !self.admitted.contains(*id))
            .cloned()
            .collect()
    }

    /// Returns declared causal edges whose predecessor was never admitted.
    #[must_use]
    pub fn unresolved_causal_edges(&self) -> BTreeSet<(EventId, EventId)> {
        let mut edges = BTreeSet::new();
        for (id, event) in &self.delivered {
            for predecessor in event.causal_predecessors() {
                if !self.admitted.contains(&predecessor) {
                    edges.insert((id.clone(), predecessor));
                }
            }
        }
        edges
    }

    /// Returns the maximal admitted Event per source, by delivery coordinate.
    ///
    /// This is the frontier proper. The maximum is taken over delivery slots
    /// rather than over admission order, so it does not depend on arrival order.
    #[must_use]
    pub fn maximal_per_source(&self) -> BTreeMap<SourceIdentity, EventId> {
        let mut maximal: BTreeMap<SourceIdentity, (u64, u64, EventId)> = BTreeMap::new();
        for id in &self.admitted {
            let event = &self.delivered[id];
            let slot = event.delivery();
            let coordinate = (slot.epoch().value(), slot.sequence().value(), id.clone());
            maximal
                .entry(event.source())
                .and_modify(|current| {
                    if coordinate > *current {
                        *current = coordinate.clone();
                    }
                })
                .or_insert(coordinate);
        }
        maximal
            .into_iter()
            .map(|(source, (_, _, id))| (source, id))
            .collect()
    }

    /// Returns how many deliveries were dropped as duplicates.
    #[must_use]
    pub const fn dropped_duplicates(&self) -> usize {
        self.duplicates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_model::{
        ClockDomainId, DataClass, EventKind, FilterEpoch, Integrity, MonitoredApplicationId,
        ObservationTime, OccurrenceTime, ProducerEvent, SchemaVersion, Sensitivity,
        SentinelClockId, SequenceEpoch, SequenceNumber, SourceAssurance, SourceCapabilities,
        SourceId, SourceIdentity, SourceTier,
    };

    fn source(name: &str) -> SourceIdentity {
        SourceIdentity::new(
            MonitoredApplicationId::new(name).expect("valid id"),
            SourceId::new("gateway").expect("valid id"),
        )
    }

    /// One minimal valid Event with the given identity and declared predecessors.
    ///
    /// The only fields [`CausalIndex`] reads are identity and causal
    /// predecessors, so the rest is the smallest evidence the model accepts.
    fn event(id: &str, sequence: u64, predecessors: &[&str]) -> Event {
        let producer = ProducerEvent::new(
            EventId::new(id).expect("valid id"),
            SequenceEpoch::new(0),
            SequenceNumber::new(sequence),
            SchemaVersion::new("schema").expect("valid schema"),
            FilterEpoch::new(0),
            EventKind::new("tool.requested").expect("valid kind"),
            OccurrenceTime::new(ClockDomainId::new("producer").expect("valid id"), 1, 0),
            BTreeMap::new(),
            SourceCapabilities::new([]),
            Sensitivity::Internal,
            predecessors
                .iter()
                .map(|predecessor| EventId::new(*predecessor).expect("valid predecessor id"))
                .collect(),
        )
        .expect("valid producer evidence");
        let assurance = SourceAssurance::new(
            source("app"),
            SourceTier::Anchor,
            SourceCapabilities::new([]),
            Integrity::Authenticated,
            Vec::new(),
            Vec::new(),
            "topology",
        )
        .expect("valid assurance");
        Event::observe(
            producer,
            assurance,
            ObservationTime::new(SentinelClockId::new("sentinel").expect("valid id"), 0),
            DataClass::Tenant,
        )
    }

    fn id(value: &str) -> EventId {
        EventId::new(value).expect("valid id")
    }

    /// `a -> b -> c`, plus an unrelated `d`: a two-hop chain and a Concurrent.
    fn chain() -> CausalIndex {
        CausalIndex::build(&[
            event("a", 1, &[]),
            event("b", 2, &["a"]),
            event("c", 3, &["b"]),
            event("d", 4, &[]),
        ])
        .expect("the chain is acyclic")
    }

    #[test]
    fn hypothetical_reach_traverses_a_two_hop_chain() {
        let index = chain();
        assert_eq!(
            index.reaches_from_declared(&id("a"), &[id("c")]),
            HypotheticalReach::Reaches,
            "a is in c's transitive past through b"
        );
        assert_eq!(
            index.reaches_from_declared(&id("b"), &[id("c")]),
            HypotheticalReach::Reaches
        );
        assert_eq!(
            index.reaches_from_declared(&id("a"), &[id("b"), id("d")]),
            HypotheticalReach::Reaches,
            "any reaching predecessor suffices"
        );
        assert_eq!(
            index.reaches_from_declared(&id("c"), &[id("c")]),
            HypotheticalReach::Reaches,
            "a declared predecessor reaches itself directly"
        );
    }

    #[test]
    fn hypothetical_reach_over_a_complete_past_settles_negative() {
        let index = chain();
        assert_eq!(
            index.reaches_from_declared(&id("c"), &[id("b")]),
            HypotheticalReach::NotReaches,
            "c is in b's future, not its past"
        );
        assert_eq!(
            index.reaches_from_declared(&id("d"), &[id("c")]),
            HypotheticalReach::NotReaches,
            "d is Concurrent with the whole chain"
        );
    }

    #[test]
    fn a_dangling_declared_predecessor_is_incomplete_not_negative() {
        let index = chain();
        assert_eq!(
            index.reaches_from_declared(&id("a"), &[id("c"), id("absent")]),
            HypotheticalReach::PastIncomplete,
            "an unclassifiable past outranks a settled answer on the same set"
        );
        assert_eq!(
            index.reaches_from_declared(&id("d"), &[id("absent")]),
            HypotheticalReach::PastIncomplete
        );
    }

    #[test]
    fn an_empty_declared_past_is_vacuously_complete_and_reaches_nothing() {
        let index = chain();
        assert_eq!(
            index.reaches_from_declared(&id("a"), &[]),
            HypotheticalReach::NotReaches
        );
        assert_eq!(
            index.reaches_from_declared(&id("absent"), &[]),
            HypotheticalReach::NotReaches,
            "an unindexed Event over an empty past is still a settled negative"
        );
    }
}
