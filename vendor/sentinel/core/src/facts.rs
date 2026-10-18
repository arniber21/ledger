//! The cold-path fact protocol: what the Core asks, and what an answer settles.
//!
//! # The Core carries the protocol and invents no query language
//!
//! [`FactQuery`] is **deliberately uninterpreted by the Core**. It names *which
//! declared guard, for which Binding, at what Watermark floor* — and nothing
//! about reachability, joins or acyclicity. Inventing `FactQuery::Reachability`
//! before a partner-derived Specification demands it is exactly what ADR-0019
//! forbids ("general cold-path machinery is added when a partner-derived
//! Specification requires it, not merely because the final architecture
//! anticipates it"), and it would hand #18 a query language it has to undo.
//! #18 owns what a guard *evaluates*; #12 owns the round trip, the batching,
//! the dedup, the Watermark comparison and the counter.
//!
//! The four-tuple ADR-0021 makes reconciliation a pure function of is (hot
//! verdict, hot Watermark, cold verdict, cold Watermark). **#12 built the two
//! Watermark components.** This crate now also carries [`GuardConclusion`]:
//! what a declared guard answered. That is still not a query language —
//! [`FactQuery`] stays an uninterpreted `(guard, binding, floor)` triple.
//! The Core names no operator. What a guard *evaluates* lives in
//! `sentinel-coldpath`.
//!
//! Outcome composition and the typed Amendment record stay in **#20**,
//! *because* they are one problem (ADR-0021's O-11). An Authoritative
//! cold-driven `Violate` is still not constructible here:
//! `validate_decisive_violation_basis` demands an anchor-tier witness Event
//! listed in that source's `anchored_event_ids`, and putting a
//! [`ColdWitness`] into that field is the 4a trust-boundary question #20
//! owns. A terminal [`GuardConclusion::Refused`] is **not** composition: it
//! settles the query and maps to Authoritative `Inconclusive(epistemic)`.
//!
//! # What a response does
//!
//! A response **settles** its query when its conclusion is terminal
//! ([`GuardConclusion::Holds`], [`GuardConclusion::Fails`], or
//! [`GuardConclusion::Refused`]), *and* for every source the query's
//! `watermark_floor` names its `settled_at` sits in the floor's *own* epoch
//! at or after the floor's sequence, *and* it covers every required source.
//! [`GuardConclusion::Deferred`] never settles: it is accepted and counted
//! but leaves the query outstanding, and it re-issues in the next batch —
//! the cold path lags and settles at a later Watermark by design
//! (ADR-0021), so a behind-floor or otherwise retryable answer means "not
//! yet" and the Core must ask again.
//!
//! The epoch is compared for **equality** and not for "at or after", because a
//! `SequenceEpoch` bump resets the sequence rather than continuing it; see
//! [`FactResponse::settles`] for the argument.
//!
//! A non-settling response **degrades; it never errors.** It leaves
//! `cold_path = PENDING`, `cold_settlement_watermarks` empty and authority
//! `Provisional`, so it never reaches `Verdict::try_from` and no periphery
//! input is ever surfaced as [`crate::CoreError::Verdict`], which
//! `crate::verdict::build` reserves for a Core bug.
//! [`crate::CoreError::FactResponseMismatch`] is reserved for exactly one case:
//! a response echoing a [`FactQuery`] that was never issued.
//!
//! **No retry budget and no staleness bound.** A periphery that answers
//! behind-floor forever re-issues forever. This slice examined O-21a's retry
//! half and left it open on #18; the staleness half (a Violate-class
//! outcome change) stays **#20**. The Core still has no clock to bound
//! either with.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};

use sentinel_model::{EventId, SequenceEpoch, SequenceNumber, SourceIdentity};

use crate::slice::Binding;

/// A Watermark coordinate: how far one source has settled.
pub type WatermarkPoint = (SequenceEpoch, SequenceNumber);

/// The union of the settling answers' coverage for one Binding.
///
/// # Why this is a newtype rather than the map it wraps
///
/// `crate::settlement::settle` takes this and [`EvidenceWatermarks`]
/// adjacently, and the two were once the same type. Transposing them inverts
/// the clamp — it would pin the Core's evidence to the periphery's answer
/// instead of the reverse — and the compiler could not object. Worse, the
/// entire `sentinel-core` suite passed with them transposed, because the only
/// arm any test reached was [`std::cmp::Ordering::Equal`], whose body is a
/// commutative `min`. Distinct types make that mutation a compile error, which
/// is the only remedy that survives a future edit adding a fourth arm.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettledWatermarks(BTreeMap<SourceIdentity, WatermarkPoint>);

/// The Core's own per-source Watermark: the ceiling a cold answer is clamped to.
///
/// The same coordinate `crate::verdict` writes into `assurance.watermarks`. It
/// is the ceiling because `sentinel_model`'s `validate_settlement_reached`
/// makes it one, requiring `evidence[s] >= cold[s]` for every source. See
/// [`SettledWatermarks`] for why these are two types and not one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvidenceWatermarks(BTreeMap<SourceIdentity, WatermarkPoint>);

/// Generates the shared surface of the two Watermark wrappers.
///
/// One macro rather than two hand-written impls because the *shared* surface
/// genuinely is identical and drifting it would be a bug. This does not assert
/// the two types should stay identical: they should not — one is a ceiling and
/// one is the thing clamped against it. A method wanted on only one of them
/// goes in an ordinary `impl` block beside this invocation, which the macro
/// neither prevents nor complicates.
macro_rules! watermark_map {
    ($name:ident) => {
        impl $name {
            /// Wraps a per-source Watermark map.
            ///
            /// All three wrappers are structurally identical, so this cannot
            /// reject a map of the wrong provenance — `EvidenceWatermarks::new`
            /// accepts the settling union quite happily. What makes the
            /// distinction trustworthy is that each has exactly **one**
            /// construction site in this crate: the settling union in
            /// `FactLedger::settled_watermarks`, the ceiling in
            /// `Evaluator::evidence_watermarks`, and the floor only via
            /// [`EvidenceWatermarks::into_floor`]. A second construction site is
            /// where that stops being true, so add one deliberately or not at
            /// all.
            #[must_use]
            pub const fn new(points: BTreeMap<SourceIdentity, WatermarkPoint>) -> Self {
                Self(points)
            }

            /// Returns the coordinate recorded for `source`, if any.
            #[must_use]
            pub fn get(&self, source: &SourceIdentity) -> Option<&WatermarkPoint> {
                self.0.get(source)
            }

            /// Returns whether `source` is present.
            #[must_use]
            pub fn contains_key(&self, source: &SourceIdentity) -> bool {
                self.0.contains_key(source)
            }

            /// Returns the wrapped map.
            #[must_use]
            pub const fn as_map(&self) -> &BTreeMap<SourceIdentity, WatermarkPoint> {
                &self.0
            }

            /// Consumes the wrapper, returning the map.
            #[must_use]
            pub fn into_map(self) -> BTreeMap<SourceIdentity, WatermarkPoint> {
                self.0
            }
        }

        impl FromIterator<(SourceIdentity, WatermarkPoint)> for $name {
            fn from_iter<T: IntoIterator<Item = (SourceIdentity, WatermarkPoint)>>(
                iter: T,
            ) -> Self {
                Self(iter.into_iter().collect())
            }
        }
    };
}

/// The evidence Watermark as it stood when a query was declared.
///
/// Not the same thing as [`EvidenceWatermarks`], despite being read from the
/// same function. The floor is a **snapshot**, taken once in
/// `Evaluator::declare_queries` and cloned into every [`FactQuery`]; the
/// ceiling is read later, at Verdict-building time, and may have advanced —
/// including through a sequence reset. `clamp_to_evidence`'s `Less` and
/// `Greater` arms exist *only* because the two can diverge: either the floor
/// omitted a required source, or it is simply older.
///
/// So they are two observations of one quantity at different times, and that
/// relationship is the whole basis of the clamp. Giving the snapshot its own
/// type keeps a later edit from passing the live ceiling where the floor is
/// expected — which would restore a transposition one call frame up, inside
/// [`FactResponse::settles`], where the comparison would then always succeed.
// `Ord`/`PartialOrd` because `FactQuery` derives them and this is one of its
// fields; the other two wrappers are never ordered, so they do not carry them.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct WatermarkFloor(BTreeMap<SourceIdentity, WatermarkPoint>);

watermark_map!(SettledWatermarks);
watermark_map!(EvidenceWatermarks);
watermark_map!(WatermarkFloor);

impl EvidenceWatermarks {
    /// Freezes the live evidence Watermark into the floor a query is declared at.
    ///
    /// Named rather than a bare `into_map()` because taking the snapshot *is*
    /// the operation: everything the clamp later reasons about follows from the
    /// floor being older than the ceiling it is eventually compared against.
    #[must_use]
    pub fn into_floor(self) -> WatermarkFloor {
        WatermarkFloor::new(self.into_map())
    }
}

/// Stable identity of one declared cold-path guard.
///
/// Versioned string pair. #18 declined a typed operator payload here:
/// that would be `FactQuery::Reachability` in all but name, and a
/// `#[non_exhaustive]` enum in Core recreates the #112 silent-fallback
/// hazard. The strong type lives in `sentinel-coldpath`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct ColdGuardId {
    id: String,
    version: String,
}

impl ColdGuardId {
    /// Names a cold-path guard and the version of its definition.
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
        }
    }

    /// Returns the guard identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the guard definition version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// What a cold-path guard concluded about one Binding.
///
/// Closed, not a `String`: the Core reads the discriminant to decide whether
/// the query settles. [`Self::Deferred`] re-issues (`settles()` is false).
/// [`Self::Holds`], [`Self::Fails`], and [`Self::Refused`] are terminal
/// (`settles()` is true) once the floor comparison and required-source
/// coverage also hold. Composition of a `Fails` into a `Violate` is #20;
/// a terminal `Refused` is this crate's envelope, not composition.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GuardConclusion {
    /// The guard's property holds over the window it was answered against.
    Holds,
    /// The property fails, naming the coordinates that witness it.
    ///
    /// Stored, never composed: this crate does not mint a `Violate` from
    /// these witnesses and does not put them in `anchored_event_ids`.
    Fails {
        /// Coordinates the operator named as the failure.
        witnesses: BTreeSet<ColdWitness>,
    },
    /// No answer yet, and asking again may produce one. Does not settle.
    Deferred {
        /// Why the periphery could not answer this round.
        reason: DeferralReason,
    },
    /// No answer will ever exist. The cold path answered "never". Settles.
    Refused {
        /// Why the asked window is permanently unholdable.
        reason: RefusalReason,
    },
}

impl GuardConclusion {
    /// Returns whether this conclusion is a terminal answer.
    ///
    /// Only [`Self::Deferred`] re-issues. The other three settle the query
    /// once [`FactResponse::settles`] also confirms the floor and required
    /// sources.
    #[must_use]
    pub const fn settles(&self) -> bool {
        !matches!(self, Self::Deferred { .. })
    }
}

/// A coordinate a cold guard's conclusion rests on.
///
/// Exactly the four fields `raw::DecisiveEvent` needs, so #20 can consume it
/// without a re-shape. This crate stores witnesses and composes nothing.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct ColdWitness {
    event: EventId,
    source: SourceIdentity,
    epoch: SequenceEpoch,
    sequence: SequenceNumber,
}

impl ColdWitness {
    /// Names one Event at one source's Watermark coordinate.
    #[must_use]
    pub const fn new(event: EventId, source: SourceIdentity, at: WatermarkPoint) -> Self {
        Self {
            event,
            source,
            epoch: at.0,
            sequence: at.1,
        }
    }

    /// Returns the Event this witness names.
    #[must_use]
    pub const fn event(&self) -> &EventId {
        &self.event
    }

    /// Returns the source that produced the Event.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.source
    }

    /// Returns the Watermark coordinate the Event sits at.
    #[must_use]
    pub const fn at(&self) -> WatermarkPoint {
        (self.epoch, self.sequence)
    }
}

/// Why a guard could not answer this round and should be asked again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DeferralReason {
    /// The retained projection has not reached the query's Watermark floor.
    ProjectionBehindFloor,
    /// The projection changed underneath a paginated read.
    ProjectionChangedUnderRead,
    /// The periphery could not reach its projection at all.
    ProjectionUnreachable,
    /// Causal edges inside the window name Events the window does not resolve.
    CausalEdgesUnresolved,
}

impl Display for DeferralReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectionBehindFloor => {
                formatter.write_str("retained projection has not reached the Watermark floor")
            }
            Self::ProjectionChangedUnderRead => {
                formatter.write_str("projection changed underneath a paginated read")
            }
            Self::ProjectionUnreachable => {
                formatter.write_str("periphery could not reach its projection")
            }
            Self::CausalEdgesUnresolved => {
                formatter.write_str("causal edges name Events the asked window does not resolve")
            }
        }
    }
}

/// Why a guard will never be able to answer the question that was asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RefusalReason {
    /// Part of the window is past the retention horizon.
    EvidenceReaped,
    /// Part of the window was refused at append and never retained.
    EvidenceNeverRetained,
    /// The window's declared edges admit no partial order.
    CausalEdgesContradictory,
    /// No catalogued guard matches the query's [`ColdGuardId`].
    GuardUnknown,
}

impl Display for RefusalReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EvidenceReaped => formatter.write_str("asked window includes a reaped range"),
            Self::EvidenceNeverRetained => {
                formatter.write_str("asked window includes a never-retained range")
            }
            Self::CausalEdgesContradictory => {
                formatter.write_str("asked window's declared edges admit no partial order")
            }
            Self::GuardUnknown => formatter.write_str("no catalogued guard matches the query"),
        }
    }
}

/// One outstanding cold-path question, for one guard and one Binding.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct FactQuery {
    guard: ColdGuardId,
    binding: Binding,
    watermark_floor: WatermarkFloor,
}

impl FactQuery {
    /// Asks one guard about one Binding, no earlier than the given floor.
    #[must_use]
    pub const fn new(
        guard: ColdGuardId,
        binding: Binding,
        watermark_floor: WatermarkFloor,
    ) -> Self {
        Self {
            guard,
            binding,
            watermark_floor,
        }
    }

    /// Returns the guard being asked.
    #[must_use]
    pub const fn guard(&self) -> &ColdGuardId {
        &self.guard
    }

    /// Returns the Binding being asked about.
    #[must_use]
    pub const fn binding(&self) -> &Binding {
        &self.binding
    }

    /// Returns the hot settlement point an answer must be at least as recent as.
    #[must_use]
    pub const fn watermark_floor(&self) -> &WatermarkFloor {
        &self.watermark_floor
    }
}

/// One recorded answer, echoing the query it answers.
///
/// The echo is what makes a response impossible to misattribute, and it makes a
/// fabricated response impossible to construct without a real issued query —
/// which is the shape a pure Core fixture needs (ADR-0012: "Core tests are pure
/// fixtures: recorded fact responses, no database, no clock").
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct FactResponse {
    query: FactQuery,
    conclusion: GuardConclusion,
    settled_at: BTreeMap<SourceIdentity, WatermarkPoint>,
}

impl FactResponse {
    /// Answers one query with one conclusion at a recorded Watermark.
    #[must_use]
    pub const fn new(
        query: FactQuery,
        conclusion: GuardConclusion,
        settled_at: BTreeMap<SourceIdentity, WatermarkPoint>,
    ) -> Self {
        Self {
            query,
            conclusion,
            settled_at,
        }
    }

    /// Returns the echoed query.
    #[must_use]
    pub const fn query(&self) -> &FactQuery {
        &self.query
    }

    /// Returns what the guard concluded.
    #[must_use]
    pub const fn conclusion(&self) -> &GuardConclusion {
        &self.conclusion
    }

    /// Returns the Watermark the answer was computed at.
    #[must_use]
    pub const fn settled_at(&self) -> &BTreeMap<SourceIdentity, WatermarkPoint> {
        &self.settled_at
    }

    /// Returns whether this answer settles its query.
    ///
    /// All four halves are load-bearing. The conclusion must be terminal
    /// ([`GuardConclusion::Holds`], [`GuardConclusion::Fails`], or
    /// [`GuardConclusion::Refused`]); [`GuardConclusion::Deferred`] re-issues.
    /// The floor comparison is ADR-0021's "authoritative verdicts require
    /// both tiers at a common Watermark"; the coverage requirement is what
    /// keeps `require_authoritative_coverage`'s set equality satisfiable at
    /// all.
    ///
    /// # Why the epoch must *match* rather than merely be at or after
    ///
    /// A [`SequenceEpoch`] bump is a **sequence reset**, not a continuation:
    /// the source restarts numbering, so `(1, 0)` is not "later than"
    /// `(0, 16)` in any sense the cold path can use. An answer computed in a
    /// different epoch therefore implies nothing whatsoever about coverage
    /// through the floor's epoch, and treating it as settling would let
    /// `crate::settlement::settle` record a `cold_settlement_watermarks`
    /// coordinate the answer never justified — fabrication by the Core, in the
    /// exact field `validate_decisive_event` uses to bound witnesses.
    ///
    /// So it does not settle, and per this module's discipline that is a
    /// **degrade, not an error**: the query returns to the unissued pool and
    /// re-issues, exactly as a behind-floor answer does. The floor is the
    /// Core's own evidence Watermark at the moment the query was declared
    /// (`Evaluator::evidence_watermarks`) and evidence epochs are monotone
    /// (`MonotonicityBreach::RegressedEpoch`), so this comparison is against a
    /// sequence space the Core has genuinely observed.
    ///
    /// Pinning the epoch to the floor's does **not** pin it to the epoch the
    /// ledger has reached by the time a Verdict is assembled: the floor is a
    /// snapshot and the ledger moves on. That is the gap
    /// [`crate::settlement`]'s stale-epoch arm exists for, and it is why this
    /// equality closes the fabrication hole without making the clamp
    /// single-armed.
    #[must_use]
    pub fn settles(&self, required: &BTreeSet<SourceIdentity>) -> bool {
        if !self.conclusion.settles() {
            return false;
        }
        let reaches_floor = self
            .query
            .watermark_floor
            .as_map()
            .iter()
            .all(|(source, floor)| {
                self.settled_at
                    .get(source)
                    .is_some_and(|(epoch, sequence)| *epoch == floor.0 && *sequence >= floor.1)
            });
        let covers_required = required
            .iter()
            .all(|source| self.settled_at.contains_key(source));
        reaches_floor && covers_required
    }
}

/// One batch of queries handed to the periphery.
///
/// Owned, with no lifetime. A borrowed form holding `&mut Evaluator` was tried
/// and rejected: it compiles but enforces nothing — dropping the request is the
/// natural way to ignore it, and the borrow ends when it drops — while making
/// the type invariant, unsendable, unstorable beside the `Evaluator` that
/// produced it, and unreturnable from a trait method. #16's async store, #18's
/// cold operators and #22 all need to own an `Evaluator`, hand a request to a
/// fetcher and apply the answer later, so `FactRequest` is `'static + Send`.
///
/// ADR-0012's "cold-path access on the hot path is a compile error" sits in the
/// consequences of rejecting the *injected async store*: the guarantee is that
/// the Core holds no store handle to await at all, and
/// `.github/scripts/check-core-purity.sh` enforces that far better than a
/// lifetime does.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct FactRequest {
    queries: Vec<FactQuery>,
    round_trip: u32,
}

impl FactRequest {
    /// Batches the outstanding queries into one round trip.
    #[must_use]
    pub const fn new(queries: Vec<FactQuery>, round_trip: u32) -> Self {
        Self {
            queries,
            round_trip,
        }
    }

    /// Returns every query in the batch.
    #[must_use]
    pub fn queries(&self) -> &[FactQuery] {
        &self.queries
    }

    /// Returns this batch's round-trip index.
    #[must_use]
    pub const fn round_trip(&self) -> u32 {
        self.round_trip
    }
}

/// Where one `(binding, guard)` question stands.
#[derive(Clone, Debug, Eq, PartialEq)]
enum QueryState {
    /// Not yet in a batch; the next [`FactRequest`] carries it.
    Unissued,
    /// In a batch, waiting for an answer.
    Awaiting,
    /// Answered by a settling response, at this Watermark, with this conclusion.
    ///
    /// Absorbing. [`GuardConclusion::Refused`] and [`GuardConclusion::Fails`]
    /// land here too — they settle — and [`FactLedger::closed`] derives the
    /// closed set from those two. There is no non-settling `Closed`
    /// discriminant: a refusal that does not settle would keep
    /// `every_query_settled` false and hide the envelope this crate owns.
    Settled {
        settled_at: BTreeMap<SourceIdentity, WatermarkPoint>,
        conclusion: GuardConclusion,
    },
}

/// The Core's record of every question it has asked.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FactLedger {
    queries: BTreeMap<(Binding, ColdGuardId), FactQuery>,
    states: BTreeMap<(Binding, ColdGuardId), QueryState>,
    issued: BTreeSet<FactQuery>,
    round_trips: u32,
}

impl FactLedger {
    /// Creates an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a question, if it is not already asked or answered.
    ///
    /// Idempotent by `(binding, guard)`: this is the dedup half of ADR-0012's
    /// batching requirement.
    pub fn declare(&mut self, query: FactQuery) {
        let key = (query.binding().clone(), query.guard().clone());
        self.states
            .entry(key.clone())
            .or_insert(QueryState::Unissued);
        self.queries.entry(key).or_insert(query);
    }

    /// Returns whether any question is waiting to be batched.
    #[must_use]
    pub fn has_unissued(&self) -> bool {
        self.states
            .values()
            .any(|state| matches!(state, QueryState::Unissued))
    }

    /// Batches every unissued question into one round trip.
    ///
    /// ADR-0012: "The Core must batch fact requests. A naive implementation
    /// round-trips repeatedly per Event and the resumption protocol becomes the
    /// bottleneck it was meant to make visible." Every outstanding
    /// `(guard, binding)` pair across every Slice goes into **one** request.
    pub fn issue(&mut self) -> Option<(FactRequest, Vec<Binding>)> {
        let pending: Vec<(Binding, ColdGuardId)> = self
            .states
            .iter()
            .filter(|(_, state)| matches!(state, QueryState::Unissued))
            .map(|(key, _)| key.clone())
            .collect();
        if pending.is_empty() {
            return None;
        }
        let round_trip = self.round_trips;
        self.round_trips += 1;

        let mut queries = Vec::with_capacity(pending.len());
        let mut bindings = Vec::with_capacity(pending.len());
        for key in pending {
            let query = self.queries[&key].clone();
            self.issued.insert(query.clone());
            self.states.insert(key.clone(), QueryState::Awaiting);
            bindings.push(key.0);
            queries.push(query);
        }
        Some((FactRequest::new(queries, round_trip), bindings))
    }

    /// Returns every question not yet answered by a settling response.
    ///
    /// A driver that drops a [`FactRequest`] can recover the queries here
    /// rather than lose them.
    #[must_use]
    pub fn outstanding(&self) -> Vec<FactQuery> {
        self.states
            .iter()
            .filter(|(_, state)| !matches!(state, QueryState::Settled { .. }))
            .map(|(key, _)| self.queries[key].clone())
            .collect()
    }

    /// Returns whether a response echoes a query this ledger ever issued.
    #[must_use]
    pub fn was_issued(&self, query: &FactQuery) -> bool {
        self.issued.contains(query)
    }

    /// Applies one accepted response, returning whether it settled its query.
    ///
    /// A non-settling response returns the query to the unissued pool, so the
    /// next batch asks again.
    pub fn apply(&mut self, response: &FactResponse, required: &BTreeSet<SourceIdentity>) -> bool {
        let key = (
            response.query().binding().clone(),
            response.query().guard().clone(),
        );
        if !self.states.contains_key(&key) {
            return false;
        }
        if matches!(self.states.get(&key), Some(QueryState::Settled { .. })) {
            return true;
        }
        if response.settles(required) {
            self.states.insert(
                key,
                QueryState::Settled {
                    settled_at: response.settled_at().clone(),
                    conclusion: response.conclusion().clone(),
                },
            );
            true
        } else {
            self.states.insert(key, QueryState::Unissued);
            false
        }
    }

    /// Returns whether every declared question for a Binding has settled.
    #[must_use]
    pub fn all_settled(&self, binding: &Binding) -> bool {
        self.states
            .iter()
            .filter(|((asked, _), _)| asked == binding)
            .all(|(_, state)| matches!(state, QueryState::Settled { .. }))
    }

    /// Returns whether any settled query for `binding` concluded [`GuardConclusion::Refused`].
    #[must_use]
    pub fn any_refused(&self, binding: &Binding) -> bool {
        self.states.iter().any(|((asked, _), state)| {
            asked == binding
                && matches!(
                    state,
                    QueryState::Settled {
                        conclusion: GuardConclusion::Refused { .. },
                        ..
                    }
                )
        })
    }

    /// Returns settled [`GuardConclusion::Refused`] and [`GuardConclusion::Fails`]
    /// answers for `binding`.
    ///
    /// Derived from settlement, not a parallel close. Empty when every
    /// settled query is [`GuardConclusion::Holds`] or none have settled.
    #[must_use]
    pub fn closed(&self, binding: &Binding) -> BTreeMap<ColdGuardId, GuardConclusion> {
        self.closed_states()
            .filter(|((asked, _), _)| asked == binding)
            .map(|((_, guard), conclusion)| (guard.clone(), conclusion.clone()))
            .collect()
    }

    /// Returns every settled refusal or failure across Bindings.
    #[must_use]
    pub fn closed_all(&self) -> BTreeMap<(Binding, ColdGuardId), GuardConclusion> {
        self.closed_states()
            .map(|(key, conclusion)| (key.clone(), conclusion.clone()))
            .collect()
    }

    fn closed_states(&self) -> impl Iterator<Item = (&(Binding, ColdGuardId), &GuardConclusion)> {
        self.states.iter().filter_map(|(key, state)| match state {
            QueryState::Settled {
                conclusion:
                    conclusion @ (GuardConclusion::Refused { .. } | GuardConclusion::Fails { .. }),
                ..
            } => Some((key, conclusion)),
            QueryState::Unissued | QueryState::Awaiting | QueryState::Settled { .. } => None,
        })
    }

    /// Returns whether any question has been declared for a Binding.
    #[must_use]
    pub fn has_queries(&self, binding: &Binding) -> bool {
        self.states.keys().any(|(asked, _)| asked == binding)
    }

    /// Returns the union of the settling answers' Watermarks for a Binding.
    #[must_use]
    pub fn settled_watermarks(&self, binding: &Binding) -> SettledWatermarks {
        let mut union: BTreeMap<SourceIdentity, WatermarkPoint> = BTreeMap::new();
        for ((asked, _), state) in &self.states {
            if asked != binding {
                continue;
            }
            if let QueryState::Settled { settled_at, .. } = state {
                for (source, point) in settled_at {
                    union
                        .entry(source.clone())
                        .and_modify(|current| {
                            if point > current {
                                *current = *point;
                            }
                        })
                        .or_insert(*point);
                }
            }
        }
        SettledWatermarks::new(union)
    }

    /// Returns how many batches have been issued.
    #[must_use]
    pub const fn round_trips(&self) -> u32 {
        self.round_trips
    }
}

#[cfg(test)]
mod tests {
    use sentinel_model::{MonitoredApplicationId, SourceId};

    use super::*;

    fn source(name: &str) -> SourceIdentity {
        SourceIdentity::new(
            MonitoredApplicationId::new(name).expect("non-empty"),
            SourceId::new(name).expect("non-empty"),
        )
    }

    fn query_at(floor: WatermarkPoint) -> (FactQuery, SourceIdentity) {
        let identity = source("orders");
        let mut points = BTreeMap::new();
        points.insert(identity.clone(), floor);
        let query = FactQuery::new(
            ColdGuardId::new("fixture.guard", "v1"),
            Binding::new("order-1"),
            WatermarkFloor::new(points),
        );
        (query, identity)
    }

    fn covering(
        identity: &SourceIdentity,
        at: WatermarkPoint,
    ) -> BTreeMap<SourceIdentity, WatermarkPoint> {
        let mut settled_at = BTreeMap::new();
        settled_at.insert(identity.clone(), at);
        settled_at
    }

    #[test]
    fn a_terminal_conclusion_settles_and_a_deferred_one_does_not() {
        assert!(GuardConclusion::Holds.settles());
        assert!(GuardConclusion::Fails {
            witnesses: BTreeSet::new()
        }
        .settles());
        assert!(GuardConclusion::Refused {
            reason: RefusalReason::EvidenceReaped
        }
        .settles());
        assert!(!GuardConclusion::Deferred {
            reason: DeferralReason::ProjectionBehindFloor
        }
        .settles());
    }

    #[test]
    fn a_deferred_response_never_settles_even_when_the_floor_is_reached() {
        let floor = (SequenceEpoch::new(0), SequenceNumber::new(4));
        let (query, identity) = query_at(floor);
        let required = BTreeSet::from([identity.clone()]);
        let response = FactResponse::new(
            query,
            GuardConclusion::Deferred {
                reason: DeferralReason::CausalEdgesUnresolved,
            },
            covering(&identity, floor),
        );
        assert!(!response.settles(&required));
    }

    #[test]
    fn holds_fails_and_refused_settle_once_the_floor_and_required_sources_hold() {
        let floor = (SequenceEpoch::new(0), SequenceNumber::new(4));
        let (query, identity) = query_at(floor);
        let required = BTreeSet::from([identity.clone()]);
        let settled_at = covering(&identity, floor);
        for conclusion in [
            GuardConclusion::Holds,
            GuardConclusion::Fails {
                witnesses: BTreeSet::new(),
            },
            GuardConclusion::Refused {
                reason: RefusalReason::EvidenceNeverRetained,
            },
        ] {
            let response = FactResponse::new(query.clone(), conclusion, settled_at.clone());
            assert!(response.settles(&required));
        }
    }

    #[test]
    fn a_terminal_conclusion_behind_the_floor_does_not_settle() {
        let floor = (SequenceEpoch::new(0), SequenceNumber::new(4));
        let behind = (SequenceEpoch::new(0), SequenceNumber::new(2));
        let (query, identity) = query_at(floor);
        let required = BTreeSet::from([identity.clone()]);
        let response =
            FactResponse::new(query, GuardConclusion::Holds, covering(&identity, behind));
        assert!(!response.settles(&required));
    }
}
