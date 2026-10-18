//! The activation barrier a warming Specification clears before it may be authoritative.
//!
//! ADR-0006 settles Filter-epoch rollout with an **activation barrier** rather than
//! version stamping alone, and ADR-0017 makes that barrier a precondition of authority:
//! a new Specification "starts in a warming Shadow state. It cannot produce authoritative
//! Verdicts until all required sources acknowledge the epoch and the activation Watermark
//! passes."
//!
//! [`ActivationBarrier`] is that precondition as data: a target Filter epoch plus one
//! [`ActivationWatermark`] per required source. [`ActivationBarrier::clear`] answers, for
//! one evidence set, whether every required source has both acknowledged the epoch **and**
//! reached its Watermark, and hands back either a [`BarrierCleared`] proof or a
//! [`BarrierPending`] naming what is still missing and why.
//!
//! # Why this lives in `sentinel-model`
//!
//! Two reasons, and the second is the load-bearing one.
//!
//! 1. `sentinel-store` and `sentinel-export` must not depend on `sentinel-core`
//!    (`store/src/record.rs` states the remaining store prohibition). `sentinel-api`
//!    may depend on `sentinel-core` for the published-Filter evaluation driver. A
//!    rollout type the control plane and the Verification Core both read cannot
//!    live in `core/`.
//! 2. [`ActivationBarrier::admit`] is a thin wrapper over
//!    [`crate::classify_completeness`], and [`crate::CompletenessEvidence`] is
//!    `#[non_exhaustive]`. Compiled from a *downstream* crate that wrapper would be forced
//!    into a wildcard arm, and a variant added to `CompletenessEvidence` later would be
//!    silently folded into whatever that arm names. Compiled from **this** crate the match
//!    is exhaustive, so a new variant is a build break here rather than a silent
//!    misclassification of source-completeness evidence.
//!
//! # What this module deliberately does not do
//!
//! It reads **no clock**. [`crate::CompletenessFreshness`] takes its boundary as a
//! parameter, and the boundary [`ActivationBarrier::clear`] uses is the barrier's own
//! [`ActivationWatermark::observed_at`] — a Sentinel observation time the *declarer* of
//! the barrier chose, never `now`. The idle-source liveness half of O-12 is
//! [`ActivationBarrier::clear_within`]: it still takes freshness as a parameter
//! ([`crate::SourceLivenessFreshness`]); only the periphery supplies `now`.
//!
//! It also does not decide what a Verdict says. Suppression is
//! `sentinel_core::settlement`'s: the barrier caps authority, and nothing in this module
//! can turn a conclusion into a pass.

#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::canonical::source_from_wire;
use crate::{
    classify_completeness, CompletenessEvidence, CompletenessFreshness, EventFilter, FilterEpoch,
    ObservationTime, SequenceEpoch, SequenceNumber, SourceEvidence, SourceIdentity,
    SourceLivenessFreshness,
};

/// Frozen decode needle for historical #156 warming payloads.
///
/// [`BarrierPending::detail`] interpolates `{prefix} {sources}`. Relabel matches with
/// `contains`, not `starts_with`, because Core folds gate detail and this sentence with
/// `"; "` into one `PREFIX_UNCERTAINTY` entry.
///
/// Changing this string silently stops matching stored bytes and relabels them
/// `NOT_DECLARED`. A later sentence change must add a second matcher, not replace this.
pub const BARRIER_PENDING_DETAIL_PREFIX: &str = "activation barrier is not cleared for:";

/// The point one required source must reach before a barrier can clear for it.
///
/// A sequence coordinate plus the Sentinel observation time the barrier was declared at.
/// All three fields are load bearing; none is a round-trip carrier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationWatermark {
    sequence_epoch: SequenceEpoch,
    complete_through: SequenceNumber,
    observed_at: ObservationTime,
}

impl ActivationWatermark {
    /// Declares the coordinate and freshness boundary one source must reach.
    #[must_use]
    pub const fn new(
        sequence_epoch: SequenceEpoch,
        complete_through: SequenceNumber,
        observed_at: ObservationTime,
    ) -> Self {
        Self {
            sequence_epoch,
            complete_through,
            observed_at,
        }
    }

    /// Returns the sequence epoch the Watermark is expressed in.
    #[must_use]
    pub const fn sequence_epoch(&self) -> SequenceEpoch {
        self.sequence_epoch
    }

    /// Returns the completeness boundary the source must have reached.
    #[must_use]
    pub const fn complete_through(&self) -> SequenceNumber {
        self.complete_through
    }

    /// Returns the freshness boundary: the Sentinel observation time the barrier was
    /// declared at.
    ///
    /// **This field is load bearing and is not a round-trip carrier.** It is passed to
    /// [`crate::CompletenessFreshness::observed_not_before`], so a completeness signal
    /// Sentinel observed *before* this instant is not post-activation evidence and is
    /// classified [`SourceBarrierEvidence::NoPostBarrierEvidence`] rather than admitted.
    /// Whatever writes `SpecificationFilter.activation_watermarks[].observed_at` therefore
    /// decides how much post-publication evidence every required source must emit before
    /// the barrier can clear: set it at the instant the Event Filter epoch was published
    /// and each source must re-report completeness after the publication; set it earlier
    /// and a pre-publication report suffices, which defeats the barrier. A source that
    /// heartbeats rarely — an `INTENTIONALLY_IDLE` one especially — holds the barrier open
    /// until it reports again, which is the intended behaviour and not a defect.
    ///
    /// Clones: [`ObservationTime`] is not `Copy`, matching
    /// [`crate::SourceCompletenessSignal::observed_at`].
    #[must_use]
    pub fn observed_at(&self) -> ObservationTime {
        self.observed_at.clone()
    }
}

/// What one required source's evidence says about the activation barrier.
///
/// A renaming of [`CompletenessEvidence`] into the barrier's own vocabulary, plus the one
/// comparison `classify_completeness` cannot make (the sequence boundary against the
/// barrier's own Watermark) and the one distinction it collapses
/// ([`Self::NoEvidence`] versus [`Self::NoPostBarrierEvidence`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceBarrierEvidence {
    /// The source acknowledged the target Filter epoch and reached its Watermark.
    Admitted,
    /// The source supplied no completeness signal at all.
    ///
    /// Distinct from [`Self::ReportedNoEvents`], which is a source *saying* it emitted
    /// nothing. That distinction is O-12's: no-Event is evidence, no-evidence is not.
    NoEvidence,
    /// A completeness signal exists but Sentinel observed it before the barrier was
    /// declared, so it says nothing about post-activation evidence.
    ///
    /// Kept apart from [`Self::NoEvidence`] because [`classify_completeness`] collapses
    /// the two into `CompletenessEvidence::NoEvidence` and O-12 needs them apart.
    NoPostBarrierEvidence,
    /// The signal was authenticated to a different source than the one asked about.
    SourceMismatch,
    /// The barrier names no Watermark for this source, so it requires nothing of it.
    ///
    /// Distinct from [`Self::SourceMismatch`], and the distinction is not pedantic: there
    /// the *signal* is about someone else, here the signal is about exactly the source
    /// asked about and the *barrier* has no opinion on it. Folding the two together made
    /// [`ActivationBarrier::admit`] answer "completeness signal names another source"
    /// about a signal that named the right one. Unreachable from
    /// [`ActivationBarrier::clear`], which only ever asks about sources the barrier names.
    SourceNotRequired,
    /// The signal and the barrier use different Sentinel clocks, so freshness is
    /// undecidable.
    SentinelClockMismatch,
    /// The signal reports a different sequence epoch than the barrier's Watermark.
    SequenceEpochMismatch,
    /// The source has not yet acknowledged the target Filter epoch.
    FilterEpochUnacknowledged {
        /// The Filter epoch the source acknowledged.
        acknowledged: FilterEpoch,
        /// The Filter epoch the barrier targets.
        required: FilterEpoch,
    },
    /// The source explicitly reports that it emitted no Event.
    ReportedNoEvents {
        /// Whether the source declares itself intentionally idle.
        idle: bool,
    },
    /// The source emitted Events but declared no completed boundary.
    NoCompleteBoundary {
        /// The highest sequence the source says it emitted.
        last_emitted: SequenceNumber,
        /// Whether the source declares itself intentionally idle.
        idle: bool,
    },
    /// The source declared a boundary below the barrier's Watermark.
    BelowBarrierWatermark {
        /// The boundary the source declared.
        complete_through: SequenceNumber,
        /// The boundary the barrier requires.
        required: SequenceNumber,
        /// Whether the source declares itself intentionally idle.
        idle: bool,
    },
    /// The source cleared the activation barrier, but its completeness signal fails the
    /// caller-supplied liveness floor for its activity.
    ///
    /// Distinct from [`Self::NoPostBarrierEvidence`]: that name is reserved for a signal
    /// that predates the barrier's own `observed_at`. A post-activation heartbeat that
    /// then goes silent must fail **by this name**, never by silence and never by the
    /// pre-barrier wording (O-12 / #163).
    StaleCompletenessSignal {
        /// Whether the source declares itself intentionally idle.
        idle: bool,
    },
}

impl Display for SourceBarrierEvidence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admitted => formatter.write_str("admitted"),
            Self::NoEvidence => formatter.write_str("no completeness signal"),
            Self::NoPostBarrierEvidence => {
                formatter.write_str("completeness signal predates the activation barrier")
            }
            Self::SourceMismatch => formatter.write_str("completeness signal names another source"),
            Self::SourceNotRequired => {
                formatter.write_str("the activation barrier requires nothing of this source")
            }
            Self::SentinelClockMismatch => {
                formatter.write_str("completeness signal uses another Sentinel clock")
            }
            Self::SequenceEpochMismatch => {
                formatter.write_str("completeness signal reports another sequence epoch")
            }
            Self::FilterEpochUnacknowledged {
                acknowledged,
                required,
            } => write!(
                formatter,
                "acknowledged Filter epoch {} is below the target epoch {}",
                acknowledged.value(),
                required.value()
            ),
            Self::ReportedNoEvents { idle } => {
                write!(formatter, "source reports no Event ({})", idle_text(*idle))
            }
            Self::NoCompleteBoundary { last_emitted, idle } => write!(
                formatter,
                "source emitted through {} and declared no completed boundary ({})",
                last_emitted.value(),
                idle_text(*idle)
            ),
            Self::BelowBarrierWatermark {
                complete_through,
                required,
                idle,
            } => write!(
                formatter,
                "completed through {} is below the activation Watermark {} ({})",
                complete_through.value(),
                required.value(),
                idle_text(*idle)
            ),
            Self::StaleCompletenessSignal { idle } => write!(
                formatter,
                "completeness signal is stale for liveness ({})",
                idle_text(*idle)
            ),
        }
    }
}

const fn idle_text(idle: bool) -> &'static str {
    if idle {
        "intentionally idle"
    } else {
        "active"
    }
}

/// Refusal to construct an [`ActivationBarrier`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ActivationBarrierError {
    /// The barrier names no source, so it would be cleared vacuously.
    NoRequiredSources,
    /// The Event Filter carries no entry with the requested Specification identity.
    SpecificationNotFound,
    /// The Event Filter entry declares no activation Watermark.
    ///
    /// What every `sentinel_compile::filter::compile_filter` entry emits today: the
    /// Specification is stamped `Readiness::Warming` with an empty
    /// `activation_watermarks`. Reading a barrier out of such an entry must fail rather
    /// than invent an empty barrier that clears immediately.
    NoActivationBarrier,
}

impl Display for ActivationBarrierError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRequiredSources => {
                formatter.write_str("an activation barrier must name at least one required source")
            }
            Self::SpecificationNotFound => {
                formatter.write_str("the Event Filter carries no such Specification")
            }
            Self::NoActivationBarrier => {
                formatter.write_str("the Specification declares no activation Watermark")
            }
        }
    }
}

impl Error for ActivationBarrierError {}

/// The Filter epoch and per-source Watermarks a Specification must clear to be
/// authoritative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationBarrier {
    target_filter_epoch: FilterEpoch,
    watermarks: BTreeMap<SourceIdentity, ActivationWatermark>,
}

impl ActivationBarrier {
    /// Declares an activation barrier over one Filter epoch and its required sources.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationBarrierError::NoRequiredSources`] for an empty Watermark map.
    /// A barrier over no source clears vacuously, which is exactly the state ADR-0017
    /// forbids a warming Specification from reaching, so it is refused in the type rather
    /// than checked at each call site (ADR-0031).
    pub fn new(
        target_filter_epoch: FilterEpoch,
        watermarks: BTreeMap<SourceIdentity, ActivationWatermark>,
    ) -> Result<Self, ActivationBarrierError> {
        if watermarks.is_empty() {
            return Err(ActivationBarrierError::NoRequiredSources);
        }
        Ok(Self {
            target_filter_epoch,
            watermarks,
        })
    }

    /// Reads the barrier out of one entry of a **validated** published Event Filter.
    ///
    /// The path a driver actually has: ADR-0006 makes the Event Filter the only channel
    /// through which Sentinel asks for evidence, so the barrier a deployment is warming
    /// behind is the one its published Filter declares.
    ///
    /// **It takes an [`EventFilter`], not a `raw_model::SpecificationFilter`, and that is
    /// the point.** `EventFilter::try_from` is the only constructor, and it refuses
    /// `InvalidReadinessMode`, `TargetEpochMismatch`, `ActivationBarrierIncomplete`, a
    /// duplicate activation-Watermark source and a malformed `observed_at`. A signature
    /// taking the raw per-Specification message would accept one the caller decoded
    /// itself, which is every one of those refusals bypassed — the same defect shape as a
    /// forgeable proof token. Reading through the validated newtype means the entry this
    /// function sees has already passed all of them, which is why the field reads below
    /// may `expect` rather than re-derive an error taxonomy that would be unreachable.
    ///
    /// # Errors
    ///
    /// Returns [`ActivationBarrierError::SpecificationNotFound`] when the Filter carries
    /// no entry with that identity, and [`ActivationBarrierError::NoActivationBarrier`]
    /// when the entry declares none — which is what every Specification compiled today
    /// emits.
    ///
    /// # Panics
    ///
    /// Never. Every `expect` below rests on an invariant `EventFilter::try_from`
    /// established: `validate_specification_sources` runs `validate_watermarks` over
    /// `activation_watermarks` for **every** entry, whatever its evaluation mode, so each
    /// Watermark carries a valid source and a valid `observed_at`; and
    /// `target_filter_epoch` was compared for equality with the envelope's `epoch`, which
    /// `MissingScalar("event_filter.epoch")` proves present.
    pub fn try_from_event_filter(
        filter: &EventFilter,
        specification_id: &str,
        specification_version: &str,
    ) -> Result<Self, ActivationBarrierError> {
        let specification = filter
            .specifications()
            .iter()
            .find(|candidate| {
                candidate.specification_id == specification_id
                    && candidate.specification_version == specification_version
            })
            .ok_or(ActivationBarrierError::SpecificationNotFound)?;
        if specification.activation_watermarks.is_empty() {
            return Err(ActivationBarrierError::NoActivationBarrier);
        }
        let target_filter_epoch = FilterEpoch::new(
            specification
                .target_filter_epoch
                .expect("a validated Event Filter entry carries its target epoch"),
        );
        let watermarks = specification
            .activation_watermarks
            .iter()
            .map(|watermark| {
                let source = source_from_wire(
                    watermark
                        .source
                        .as_ref()
                        .expect("a validated activation Watermark names a source"),
                )
                .expect("a validated activation Watermark names a valid source");
                let observed_at = ObservationTime::try_from_wire(
                    watermark
                        .observed_at
                        .clone()
                        .expect("a validated activation Watermark carries an observation time"),
                )
                .expect("a validated activation Watermark carries a valid observation time");
                (
                    source,
                    ActivationWatermark::new(
                        SequenceEpoch::new(watermark.sequence_epoch),
                        SequenceNumber::new(watermark.complete_through),
                        observed_at,
                    ),
                )
            })
            .collect();
        Self::new(target_filter_epoch, watermarks)
    }

    /// Returns the Filter epoch every required source must acknowledge.
    #[must_use]
    pub const fn target_filter_epoch(&self) -> FilterEpoch {
        self.target_filter_epoch
    }

    /// Returns the Watermark each required source must reach.
    #[must_use]
    pub const fn watermarks(&self) -> &BTreeMap<SourceIdentity, ActivationWatermark> {
        &self.watermarks
    }

    /// Classifies one source's evidence against the barrier.
    ///
    /// A thin wrapper over [`classify_completeness`]: every refusal that function makes —
    /// including `DifferentSource` and `DifferentSentinelClock`, which a hand-written
    /// decision order forgets — reaches the caller by construction rather than by
    /// remembering. Pure; reads no clock.
    #[must_use]
    pub fn admit(
        &self,
        source: &SourceIdentity,
        evidence: Option<&SourceEvidence>,
    ) -> SourceBarrierEvidence {
        self.classify(source, evidence, self.target_filter_epoch)
    }

    /// Answers whether every required source cleared the barrier.
    ///
    /// A source absent from `evidence` is [`SourceBarrierEvidence::NoEvidence`] and is
    /// never admitted: **missing evidence is never a pass**, and a barrier that cleared
    /// on silence would make a warming Specification authoritative precisely when the
    /// rollout had not reached it.
    ///
    /// # Errors
    ///
    /// Returns [`BarrierPending`] naming each unadmitted source and why. Never empty: the
    /// error is only constructed when at least one source failed, and
    /// [`ActivationBarrier::new`] refuses a barrier naming no source at all.
    pub fn clear(
        &self,
        evidence: &BTreeMap<SourceIdentity, &SourceEvidence>,
    ) -> Result<BarrierCleared, BarrierPending> {
        let mut acknowledged = BTreeMap::new();
        let mut pending = BTreeMap::new();
        for source in self.watermarks.keys() {
            let Some(record) = evidence.get(source).copied() else {
                // Answered here rather than through `admit` even though `admit`'s `None`
                // arm returns exactly this value. Splitting the absent case out is what
                // makes the admitted arm below able to read the acknowledged epoch off a
                // record it is holding, with no unwrap and therefore no panic path to
                // document. `a_silent_source_clears_nothing_by_either_route` pins the two
                // routes agreeing.
                pending.insert(source.clone(), SourceBarrierEvidence::NoEvidence);
                continue;
            };
            match self.admit(source, Some(record)) {
                SourceBarrierEvidence::Admitted => {
                    acknowledged.insert(source.clone(), record.completeness().filter_epoch());
                }
                other => {
                    pending.insert(source.clone(), other);
                }
            }
        }
        if pending.is_empty() {
            Ok(BarrierCleared { acknowledged })
        } else {
            Err(BarrierPending { pending })
        }
    }

    /// Classifies one source against the barrier, then against caller-supplied liveness.
    ///
    /// Two-phase (O-12 / #163):
    /// 1. Run [`Self::admit`] (activation freshness = each Watermark's `observed_at`).
    /// 2. If that is not [`SourceBarrierEvidence::Admitted`], return it unchanged — so a
    ///    pre-barrier signal keeps the [`SourceBarrierEvidence::NoPostBarrierEvidence`]
    ///    wording and is never renamed into a liveness failure.
    /// 3. If it is `Admitted`, select the liveness floor from
    ///    [`SourceLivenessFreshness::for_activity`] and reuse
    ///    [`CompletenessFreshness::accepts`]. Failure becomes
    ///    [`SourceBarrierEvidence::StaleCompletenessSignal`], never `NoEvidence` and
    ///    never `NoPostBarrierEvidence`.
    ///
    /// Pure; reads no clock.
    #[must_use]
    pub fn admit_within(
        &self,
        source: &SourceIdentity,
        evidence: Option<&SourceEvidence>,
        liveness: &SourceLivenessFreshness,
    ) -> SourceBarrierEvidence {
        match self.admit(source, evidence) {
            SourceBarrierEvidence::Admitted => {
                let Some(record) = evidence else {
                    return SourceBarrierEvidence::NoEvidence;
                };
                let signal = record.completeness();
                let floor = liveness.for_activity(&signal.activity());
                match floor.accepts(signal) {
                    Some(true) => SourceBarrierEvidence::Admitted,
                    Some(false) => SourceBarrierEvidence::StaleCompletenessSignal {
                        idle: signal.activity().is_idle(),
                    },
                    None => SourceBarrierEvidence::SentinelClockMismatch,
                }
            }
            other => other,
        }
    }

    /// Answers whether every required source cleared the barrier under liveness floors.
    ///
    /// Same absence rule as [`Self::clear`]: a missing source is
    /// [`SourceBarrierEvidence::NoEvidence`]. When both floors equal every Watermark's
    /// `observed_at` (the shared publication-instant case from #162), this agrees with
    /// [`Self::clear`].
    ///
    /// # Errors
    ///
    /// Returns [`BarrierPending`] naming each unadmitted source and why.
    pub fn clear_within(
        &self,
        evidence: &BTreeMap<SourceIdentity, &SourceEvidence>,
        liveness: &SourceLivenessFreshness,
    ) -> Result<BarrierCleared, BarrierPending> {
        let mut acknowledged = BTreeMap::new();
        let mut pending = BTreeMap::new();
        for source in self.watermarks.keys() {
            let Some(record) = evidence.get(source).copied() else {
                pending.insert(source.clone(), SourceBarrierEvidence::NoEvidence);
                continue;
            };
            match self.admit_within(source, Some(record), liveness) {
                SourceBarrierEvidence::Admitted => {
                    acknowledged.insert(source.clone(), record.completeness().filter_epoch());
                }
                other => {
                    pending.insert(source.clone(), other);
                }
            }
        }
        if pending.is_empty() {
            Ok(BarrierCleared { acknowledged })
        } else {
            Err(BarrierPending { pending })
        }
    }

    /// The one classifier, parameterised by the Filter epoch being demanded.
    ///
    /// The match carries **no wildcard arm**, which is the guarantee this module's header
    /// argues for: [`CompletenessEvidence`] is `#[non_exhaustive]`, and only a same-crate
    /// match can be exhaustive over it, so a variant added there is a build break here
    /// rather than a silent misclassification.
    fn classify(
        &self,
        source: &SourceIdentity,
        evidence: Option<&SourceEvidence>,
        required_filter_epoch: FilterEpoch,
    ) -> SourceBarrierEvidence {
        let Some(watermark) = self.watermarks.get(source) else {
            return SourceBarrierEvidence::SourceNotRequired;
        };
        let freshness = CompletenessFreshness::observed_not_before(watermark.observed_at());
        let signal = evidence.map(SourceEvidence::completeness);
        match classify_completeness(
            signal,
            source,
            watermark.sequence_epoch(),
            required_filter_epoch,
            &freshness,
        ) {
            // `classify_completeness` collapses "no signal" and "a signal older than the
            // freshness boundary" into one variant. O-12 needs them apart, and the
            // scrutinee is right here to tell them apart with.
            CompletenessEvidence::NoEvidence => {
                if signal.is_none() {
                    SourceBarrierEvidence::NoEvidence
                } else {
                    SourceBarrierEvidence::NoPostBarrierEvidence
                }
            }
            CompletenessEvidence::DifferentSource => SourceBarrierEvidence::SourceMismatch,
            CompletenessEvidence::DifferentEpoch => SourceBarrierEvidence::SequenceEpochMismatch,
            CompletenessEvidence::DifferentSentinelClock => {
                SourceBarrierEvidence::SentinelClockMismatch
            }
            CompletenessEvidence::StaleFilter { reported, required } => {
                SourceBarrierEvidence::FilterEpochUnacknowledged {
                    acknowledged: reported,
                    required,
                }
            }
            // A *newer* Filter epoch ships a superset of what the barrier asked for, so it
            // is not a refusal. Re-entering with the reported epoch reaches the boundary
            // arms below and cannot recurse twice: the second call's required epoch equals
            // the reported one, so `StaleFilter`/`FutureFilter` are both unreachable in it.
            // Deriving this by re-entry rather than by a hand-written arm is what keeps it
            // from drifting away from `FutureFilter`'s own non-fatal treatment.
            CompletenessEvidence::FutureFilter { reported, .. } => {
                self.classify(source, evidence, reported)
            }
            CompletenessEvidence::ReportedNoEvents { idle } => {
                SourceBarrierEvidence::ReportedNoEvents { idle }
            }
            CompletenessEvidence::EmissionsNotComplete { last_emitted, idle } => {
                SourceBarrierEvidence::NoCompleteBoundary { last_emitted, idle }
            }
            CompletenessEvidence::CompleteThrough { sequence, idle, .. } => {
                if sequence >= watermark.complete_through() {
                    SourceBarrierEvidence::Admitted
                } else {
                    SourceBarrierEvidence::BelowBarrierWatermark {
                        complete_through: sequence,
                        required: watermark.complete_through(),
                        idle,
                    }
                }
            }
        }
    }
}

/// Proof that every required source cleared the activation barrier.
///
/// Private field, no `Default`, and [`ActivationBarrier::clear`] is the only constructor,
/// so a value of this type cannot be manufactured to assert a barrier that never cleared.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarrierCleared {
    acknowledged: BTreeMap<SourceIdentity, FilterEpoch>,
}

impl BarrierCleared {
    /// Returns the Filter epoch each required source acknowledged.
    ///
    /// Exactly what ADR-0017 has `AssuranceEnvelope.filter_epochs` record: the epoch the
    /// source reported, which is at or above the barrier's target.
    #[must_use]
    pub const fn acknowledged_filter_epochs(&self) -> &BTreeMap<SourceIdentity, FilterEpoch> {
        &self.acknowledged
    }
}

/// The required sources that have not cleared the activation barrier, and why.
///
/// Private field and never empty by construction: [`ActivationBarrier::clear`] returns
/// this only when at least one source failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarrierPending {
    pending: BTreeMap<SourceIdentity, SourceBarrierEvidence>,
}

impl BarrierPending {
    /// Returns each unadmitted source and the evidence condition holding it back.
    #[must_use]
    pub const fn pending(&self) -> &BTreeMap<SourceIdentity, SourceBarrierEvidence> {
        &self.pending
    }

    /// Returns the operator-facing sentence naming every unadmitted source.
    ///
    /// Deterministic: [`BTreeMap`] iteration order is the source identities' own.
    #[must_use]
    pub fn detail(&self) -> String {
        let sources = self
            .pending
            .iter()
            .map(|(source, evidence)| {
                format!(
                    "{}/{} ({evidence})",
                    source.monitored_application(),
                    source.source()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{BARRIER_PENDING_DETAIL_PREFIX} {sources}")
    }
}

/// Proof that a customer accepted the availability contract for one
/// Specification's fail-closed enforcement (ADR-0023, ADR-0028).
///
/// ADR-0031: fail-closed is unreachable without this proof. No `Default`, no
/// public field; [`AvailabilityContract::accept`] is the only constructor, and
/// it demands the operator identity that accepted, so a fail-closed deny in an
/// incident post-mortem names who agreed to be blocked. No clock: acceptance is
/// a recorded fact the caller supplies; when it was observed is the persisting
/// caller's stamp.
///
/// The future Promotion boundary (#166) persists and supplies this proof; this
/// type exists so the enforcement decision seam can *demand* it (issue #22)
/// rather than treat fail-closed as a default it is not.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AvailabilityContract {
    specification_id: String,
    specification_version: String,
    accepted_by: String,
}

impl AvailabilityContract {
    /// Records one operator's acceptance of a Specification's fail-closed cost.
    ///
    /// # Panics
    ///
    /// Panics when any component is empty or whitespace-only: that is a
    /// promotion-time wiring bug, not runtime input, and the proof type must
    /// not be constructible without all three parts.
    #[must_use]
    pub fn accept(
        specification_id: impl Into<String>,
        specification_version: impl Into<String>,
        accepted_by: impl Into<String>,
    ) -> Self {
        let specification_id = specification_id.into();
        let specification_version = specification_version.into();
        let accepted_by = accepted_by.into();
        assert!(
            !specification_id.trim().is_empty(),
            "specification id must not be empty"
        );
        assert!(
            !specification_version.trim().is_empty(),
            "specification version must not be empty"
        );
        assert!(
            !accepted_by.trim().is_empty(),
            "accepting operator identity must not be empty"
        );
        Self {
            specification_id,
            specification_version,
            accepted_by,
        }
    }

    /// Returns the identity of the Specification the contract covers.
    #[must_use]
    pub fn specification_id(&self) -> &str {
        &self.specification_id
    }

    /// Returns the version of the Specification the contract covers.
    #[must_use]
    pub fn specification_version(&self) -> &str {
        &self.specification_version
    }

    /// Returns the operator identity that accepted the contract.
    #[must_use]
    pub fn accepted_by(&self) -> &str {
        &self.accepted_by
    }

    /// Returns whether this contract covers the given Specification identity.
    ///
    /// Keyed by `(id, version)` alone: `accepted_by` is audit metadata, not
    /// part of the key, so any recorded acceptance for the identity counts.
    #[must_use]
    pub fn accepts(&self, id: &str, version: &str) -> bool {
        self.specification_id == id && self.specification_version == version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::model::v1 as raw_model;
    use crate::{
        ClockDomainId, Integrity, MonitoredApplicationId, OccurrenceTime,
        ProducerCompletenessSignal, SentinelClockId, SourceActivity, SourceAssurance,
        SourceCapabilities, SourceCompletenessSignal, SourceId, SourceTier,
    };

    const BARRIER_OBSERVED_AT: i128 = 20;

    fn source(name: &str) -> SourceIdentity {
        SourceIdentity::new(
            MonitoredApplicationId::new(name).expect("valid id"),
            SourceId::new("gateway").expect("valid id"),
        )
    }

    fn observation(nanoseconds: i128) -> ObservationTime {
        ObservationTime::new(
            SentinelClockId::new("sentinel").expect("valid id"),
            nanoseconds,
        )
    }

    /// One source's evidence, every knob the classifier reads exposed.
    fn evidence(
        identity: &SourceIdentity,
        epoch: u64,
        last_emitted: Option<u64>,
        complete_through: Option<u64>,
        filter_epoch: u64,
        idle: bool,
        observed_at: ObservationTime,
    ) -> SourceEvidence {
        let occurrence =
            OccurrenceTime::new(ClockDomainId::new("producer").expect("valid id"), 1, 0);
        let activity = if idle {
            SourceActivity::Idle {
                since: occurrence.clone(),
            }
        } else {
            SourceActivity::Active {
                since: occurrence.clone(),
            }
        };
        let producer = ProducerCompletenessSignal::new(
            SequenceEpoch::new(epoch),
            last_emitted.map(SequenceNumber::new),
            complete_through.map(SequenceNumber::new),
            occurrence,
            FilterEpoch::new(filter_epoch),
            activity,
        )
        .expect("valid producer signal");
        let assurance = SourceAssurance::new(
            identity.clone(),
            SourceTier::Anchor,
            SourceCapabilities::new([]),
            Integrity::Authenticated,
            Vec::new(),
            Vec::new(),
            "topology",
        )
        .expect("valid assurance");
        SourceEvidence::new(
            identity.clone(),
            SourceTier::Anchor,
            SourceCapabilities::new([]),
            SourceCompletenessSignal::observe(producer, assurance, observed_at),
            Vec::new(),
        )
    }

    /// The reference evidence: acknowledges epoch 9 and reaches the Watermark exactly.
    fn admissible(identity: &SourceIdentity) -> SourceEvidence {
        evidence(
            identity,
            1,
            Some(4),
            Some(4),
            9,
            false,
            observation(BARRIER_OBSERVED_AT),
        )
    }

    fn barrier(identity: &SourceIdentity) -> ActivationBarrier {
        ActivationBarrier::new(
            FilterEpoch::new(9),
            BTreeMap::from([(
                identity.clone(),
                ActivationWatermark::new(
                    SequenceEpoch::new(1),
                    SequenceNumber::new(4),
                    observation(BARRIER_OBSERVED_AT),
                ),
            )]),
        )
        .expect("a barrier naming one source is constructible")
    }

    /// An Event Filter carrying one shadow-mode Specification.
    ///
    /// Warming + Shadow so that the empty-Watermark case validates:
    /// `validate_activation_barrier` now runs on the Readiness axis, and Warming with
    /// empty watermarks remains the unpublished compile path.
    fn published_filter(
        activation_watermarks: Vec<raw_model::Watermark>,
    ) -> raw_model::EventFilter {
        raw_model::EventFilter {
            epoch: Some(9),
            specifications: vec![raw_model::SpecificationFilter {
                specification_id: "spec".to_owned(),
                specification_version: "v1".to_owned(),
                event_match: Some(raw_model::EventMatch {
                    event_kinds: vec!["tool.requested".to_owned()],
                    projected_attribute_keys: Vec::new(),
                    delivery_mode: raw_model::DeliveryMode::ShipAsync as i32,
                }),
                fail_mode: raw_model::FailMode::Open as i32,
                latency_budget_nanoseconds: None,
                evaluation_mode: raw_model::EvaluationMode::Shadow as i32,
                readiness: raw_model::Readiness::Warming as i32,
                target_filter_epoch: Some(9),
                required_sources: vec![source("app").to_wire()],
                activation_watermarks,
                enforcement_gates: None,
            }],
            baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
                source_health: true,
                completeness_signals: true,
            }),
        }
    }

    /// The wire form of the barrier the tests above declare by hand.
    fn wire_watermark() -> raw_model::Watermark {
        raw_model::Watermark {
            source: Some(source("app").to_wire()),
            sequence_epoch: 1,
            complete_through: 4,
            observed_at: Some(observation(BARRIER_OBSERVED_AT).to_wire()),
        }
    }

    /// Every `SourceBarrierEvidence` variant paired with its exact `Display` output.
    fn evidence_display_cases() -> Vec<(SourceBarrierEvidence, &'static str)> {
        vec![
            (SourceBarrierEvidence::Admitted, "admitted"),
            (SourceBarrierEvidence::NoEvidence, "no completeness signal"),
            (
                SourceBarrierEvidence::NoPostBarrierEvidence,
                "completeness signal predates the activation barrier",
            ),
            (
                SourceBarrierEvidence::SourceMismatch,
                "completeness signal names another source",
            ),
            (
                SourceBarrierEvidence::SourceNotRequired,
                "the activation barrier requires nothing of this source",
            ),
            (
                SourceBarrierEvidence::SentinelClockMismatch,
                "completeness signal uses another Sentinel clock",
            ),
            (
                SourceBarrierEvidence::SequenceEpochMismatch,
                "completeness signal reports another sequence epoch",
            ),
            (
                SourceBarrierEvidence::FilterEpochUnacknowledged {
                    acknowledged: FilterEpoch::new(8),
                    required: FilterEpoch::new(9),
                },
                "acknowledged Filter epoch 8 is below the target epoch 9",
            ),
            (
                SourceBarrierEvidence::ReportedNoEvents { idle: true },
                "source reports no Event (intentionally idle)",
            ),
            (
                SourceBarrierEvidence::NoCompleteBoundary {
                    last_emitted: SequenceNumber::new(4),
                    idle: false,
                },
                "source emitted through 4 and declared no completed boundary (active)",
            ),
            (
                SourceBarrierEvidence::BelowBarrierWatermark {
                    complete_through: SequenceNumber::new(2),
                    required: SequenceNumber::new(4),
                    idle: false,
                },
                "completed through 2 is below the activation Watermark 4 (active)",
            ),
            (
                SourceBarrierEvidence::StaleCompletenessSignal { idle: false },
                "completeness signal is stale for liveness (active)",
            ),
        ]
    }

    #[test]
    fn evidence_display_is_stable() {
        let cases = evidence_display_cases();
        assert_eq!(
            cases.len(),
            12,
            "every SourceBarrierEvidence variant must be pinned"
        );
        for (evidence, expected) in cases {
            assert_eq!(evidence.to_string(), expected, "{evidence:?}");
        }
    }

    #[test]
    fn barrier_error_display_is_stable() {
        let cases = [
            (
                ActivationBarrierError::NoRequiredSources,
                "an activation barrier must name at least one required source",
            ),
            (
                ActivationBarrierError::SpecificationNotFound,
                "the Event Filter carries no such Specification",
            ),
            (
                ActivationBarrierError::NoActivationBarrier,
                "the Specification declares no activation Watermark",
            ),
        ];
        assert_eq!(
            cases.len(),
            3,
            "every ActivationBarrierError variant must be pinned"
        );
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?}");
        }
    }

    /// A barrier naming no source would clear vacuously, which is the state ADR-0017
    /// forbids a warming Specification from reaching.
    #[test]
    fn a_barrier_over_no_source_is_refused() {
        assert_eq!(
            ActivationBarrier::new(FilterEpoch::new(9), BTreeMap::new()),
            Err(ActivationBarrierError::NoRequiredSources)
        );
    }

    /// The wrapper's mapping, one case per reachable classification.
    ///
    /// The three O-12 cases are asserted as **three distinct values**: a source that
    /// supplied nothing, a source whose signal predates the barrier, and a source that
    /// says it emitted no Event are different facts, and `classify_completeness` collapses
    /// the first two.
    ///
    /// `SourceMismatch` and `SentinelClockMismatch` appear here for a reason worth
    /// naming: a hand-written decision order over these conditions forgets exactly those
    /// two, and the wrapper gets them by construction because it defers to
    /// `classify_completeness` rather than reimplementing it.
    #[test]
    fn every_classification_maps_to_its_own_barrier_evidence() {
        let identity = source("app");
        let barrier = barrier(&identity);
        // Every case varies exactly one knob against the reference evidence.
        let fresh = |epoch, last, through, filter, idle| {
            evidence(
                &identity,
                epoch,
                last,
                through,
                filter,
                idle,
                observation(BARRIER_OBSERVED_AT),
            )
        };

        let cases: Vec<(SourceBarrierEvidence, Option<SourceEvidence>)> = vec![
            (SourceBarrierEvidence::Admitted, Some(admissible(&identity))),
            (SourceBarrierEvidence::NoEvidence, None),
            (
                SourceBarrierEvidence::NoPostBarrierEvidence,
                Some(evidence(
                    &identity,
                    1,
                    Some(4),
                    Some(4),
                    9,
                    false,
                    observation(BARRIER_OBSERVED_AT - 1),
                )),
            ),
            (
                SourceBarrierEvidence::SourceMismatch,
                Some(admissible(&source("other-app"))),
            ),
            (
                SourceBarrierEvidence::SentinelClockMismatch,
                Some(evidence(
                    &identity,
                    1,
                    Some(4),
                    Some(4),
                    9,
                    false,
                    ObservationTime::new(
                        SentinelClockId::new("other-sentinel").expect("valid id"),
                        BARRIER_OBSERVED_AT,
                    ),
                )),
            ),
            (
                SourceBarrierEvidence::SequenceEpochMismatch,
                Some(fresh(2, Some(4), Some(4), 9, false)),
            ),
            (
                SourceBarrierEvidence::FilterEpochUnacknowledged {
                    acknowledged: FilterEpoch::new(8),
                    required: FilterEpoch::new(9),
                },
                Some(fresh(1, Some(4), Some(4), 8, false)),
            ),
            (
                SourceBarrierEvidence::ReportedNoEvents { idle: true },
                Some(fresh(1, None, None, 9, true)),
            ),
            (
                SourceBarrierEvidence::NoCompleteBoundary {
                    last_emitted: SequenceNumber::new(4),
                    idle: false,
                },
                Some(fresh(1, Some(4), None, 9, false)),
            ),
            (
                SourceBarrierEvidence::BelowBarrierWatermark {
                    complete_through: SequenceNumber::new(2),
                    required: SequenceNumber::new(4),
                    idle: false,
                },
                Some(fresh(1, Some(4), Some(2), 9, false)),
            ),
        ];

        assert_eq!(
            cases.len(),
            10,
            "every SourceBarrierEvidence variant reachable for a source the barrier \
             *does* name must have a driving case; the eleventh, `SourceNotRequired`, \
             turns on the queried source rather than on its evidence and is driven by \
             `a_source_the_barrier_does_not_name_is_refused`"
        );
        for (expected, record) in &cases {
            assert_eq!(
                barrier.admit(&identity, record.as_ref()),
                *expected,
                "{expected:?}"
            );
        }
    }

    /// `clear` answers a silent source directly; `admit` answers it through
    /// `classify_completeness`. The two must agree, because `clear` splits the case out to
    /// avoid an unwrap rather than to change the answer.
    fn liveness_at_barrier() -> SourceLivenessFreshness {
        SourceLivenessFreshness::try_new(
            CompletenessFreshness::observed_not_before(observation(BARRIER_OBSERVED_AT)),
            CompletenessFreshness::observed_not_before(observation(BARRIER_OBSERVED_AT)),
        )
        .expect("identical floors are valid")
    }

    fn liveness_after(active_floor: i128, idle_floor: i128) -> SourceLivenessFreshness {
        SourceLivenessFreshness::try_new(
            CompletenessFreshness::observed_not_before(observation(active_floor)),
            CompletenessFreshness::observed_not_before(observation(idle_floor)),
        )
        .expect("valid dual floors")
    }

    /// `clear` answers a silent source directly; `admit` answers it through
    /// `classify_completeness`. The two must agree, because `clear` splits the case out to
    /// avoid an unwrap rather than to change the answer.
    #[test]
    fn a_silent_source_clears_nothing_by_either_route() {
        let identity = source("app");
        let barrier = barrier(&identity);
        assert_eq!(
            barrier.admit(&identity, None),
            SourceBarrierEvidence::NoEvidence
        );
        let pending = barrier
            .clear(&BTreeMap::new())
            .expect_err("a silent required source holds the barrier");
        assert_eq!(
            pending.pending(),
            &BTreeMap::from([(identity, SourceBarrierEvidence::NoEvidence)])
        );
    }

    /// When both liveness floors equal every Watermark `observed_at`, `clear_within`
    /// agrees with `clear` (AC3 / shared publication-instant case).
    #[test]
    fn clear_within_agrees_with_clear_at_barrier_observation_time() {
        let identity = source("app");
        let barrier = barrier(&identity);
        let record = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            9,
            false,
            observation(BARRIER_OBSERVED_AT),
        );
        let evidence_map = BTreeMap::from([(identity.clone(), &record)]);
        let cleared = barrier.clear(&evidence_map).expect("activation clear");
        let within = barrier
            .clear_within(&evidence_map, &liveness_at_barrier())
            .expect("liveness at barrier observation equals clear");
        assert_eq!(cleared, within);
    }

    /// A signal that cleared activation but sits below the active liveness floor fails
    /// by name as `StaleCompletenessSignal`, not as silence and not as pre-barrier.
    #[test]
    fn a_post_barrier_but_stale_active_signal_is_named_stale() {
        let identity = source("app");
        let barrier = barrier(&identity);
        let record = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            9,
            false,
            observation(BARRIER_OBSERVED_AT + 5),
        );
        assert_eq!(
            barrier.admit(&identity, Some(&record)),
            SourceBarrierEvidence::Admitted
        );
        let liveness = liveness_after(BARRIER_OBSERVED_AT + 10, BARRIER_OBSERVED_AT);
        assert_eq!(
            barrier.admit_within(&identity, Some(&record), &liveness),
            SourceBarrierEvidence::StaleCompletenessSignal { idle: false }
        );
        let evidence_map = BTreeMap::from([(identity.clone(), &record)]);
        let pending = barrier
            .clear_within(&evidence_map, &liveness)
            .expect_err("stale active signal holds the barrier");
        assert_eq!(
            pending.pending().get(&identity),
            Some(&SourceBarrierEvidence::StaleCompletenessSignal { idle: false })
        );
    }

    /// Intentionally-idle sources use the longer (earlier) idle floor: the same age that
    /// is stale for active remains admitted while idle.
    #[test]
    fn an_intentionally_idle_source_uses_the_stated_idle_floor() {
        let identity = source("app");
        let barrier = barrier(&identity);
        let active = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            9,
            false,
            observation(BARRIER_OBSERVED_AT + 5),
        );
        let idle = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            9,
            true,
            observation(BARRIER_OBSERVED_AT + 5),
        );
        let liveness = liveness_after(BARRIER_OBSERVED_AT + 10, BARRIER_OBSERVED_AT);
        assert_eq!(
            barrier.admit_within(&identity, Some(&active), &liveness),
            SourceBarrierEvidence::StaleCompletenessSignal { idle: false }
        );
        assert_eq!(
            barrier.admit_within(&identity, Some(&idle), &liveness),
            SourceBarrierEvidence::Admitted
        );
    }

    /// Pre-barrier signals keep the pre-barrier wording under `admit_within` — they are
    /// never renamed into a liveness failure.
    #[test]
    fn a_pre_barrier_signal_keeps_pre_barrier_wording_under_admit_within() {
        let identity = source("app");
        let barrier = barrier(&identity);
        let record = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            9,
            false,
            observation(BARRIER_OBSERVED_AT - 1),
        );
        let liveness = liveness_after(BARRIER_OBSERVED_AT + 10, BARRIER_OBSERVED_AT);
        assert_eq!(
            barrier.admit_within(&identity, Some(&record), &liveness),
            SourceBarrierEvidence::NoPostBarrierEvidence
        );
    }

    /// A source whose signal is one nanosecond fresher than the boundary is admitted, and
    /// one nanosecond staler is not.
    ///
    /// The boundary is inclusive — `CompletenessFreshness::accepts` is `>=` — so the
    /// reference case above sits exactly on it.
    #[test]
    fn the_freshness_boundary_is_the_barriers_own_observation_time() {
        let identity = source("app");
        let barrier = barrier(&identity);
        for (offset, expected) in [
            (1_i128, SourceBarrierEvidence::Admitted),
            (0, SourceBarrierEvidence::Admitted),
            (-1, SourceBarrierEvidence::NoPostBarrierEvidence),
        ] {
            let record = evidence(
                &identity,
                1,
                Some(4),
                Some(4),
                9,
                false,
                observation(BARRIER_OBSERVED_AT + offset),
            );
            assert_eq!(
                barrier.admit(&identity, Some(&record)),
                expected,
                "{offset}"
            );
        }
    }

    /// A *newer* acknowledged Filter epoch ships a superset, so it is admitted.
    ///
    /// Derived by re-entering `classify_completeness` with the reported epoch rather than
    /// by a hand-written arm, so it cannot drift from `FutureFilter`'s own treatment. The
    /// re-entry terminates in one step.
    #[test]
    fn a_newer_acknowledged_filter_epoch_is_admitted() {
        let identity = source("app");
        let barrier = barrier(&identity);
        let record = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            10,
            false,
            observation(BARRIER_OBSERVED_AT),
        );
        assert_eq!(
            barrier.admit(&identity, Some(&record)),
            SourceBarrierEvidence::Admitted
        );
    }

    /// A source the barrier does not name is answered as such, not as a mismatch.
    ///
    /// The signal here is impeccable and names exactly the source asked about; it is the
    /// *barrier* that requires nothing of it. Answering `SourceMismatch` — "completeness
    /// signal names another source" — would have been a false sentence in an
    /// operator-facing string, so the two conditions get their own variants.
    #[test]
    fn a_source_the_barrier_does_not_name_is_refused() {
        let identity = source("app");
        let stranger = source("stranger");
        let barrier = barrier(&identity);
        assert_eq!(
            barrier.admit(&stranger, Some(&admissible(&stranger))),
            SourceBarrierEvidence::SourceNotRequired,
            "the signal names the source asked about; the barrier simply does not name it"
        );
        assert_eq!(
            barrier.admit(&identity, Some(&admissible(&stranger))),
            SourceBarrierEvidence::SourceMismatch,
            "this is the real mismatch: the barrier names the source, the signal does not"
        );
    }

    /// Clearing needs every named source; one silent source holds the barrier.
    #[test]
    fn clearing_requires_every_required_source_and_silence_never_clears() {
        let first = source("app-one");
        let second = source("app-two");
        let barrier = ActivationBarrier::new(
            FilterEpoch::new(9),
            BTreeMap::from([
                (
                    first.clone(),
                    ActivationWatermark::new(
                        SequenceEpoch::new(1),
                        SequenceNumber::new(4),
                        observation(BARRIER_OBSERVED_AT),
                    ),
                ),
                (
                    second.clone(),
                    ActivationWatermark::new(
                        SequenceEpoch::new(1),
                        SequenceNumber::new(4),
                        observation(BARRIER_OBSERVED_AT),
                    ),
                ),
            ]),
        )
        .expect("a barrier naming two sources is constructible");

        let first_record = admissible(&first);
        let second_record = admissible(&second);

        let pending = barrier
            .clear(&BTreeMap::from([(first.clone(), &first_record)]))
            .expect_err("a silent required source holds the barrier");
        assert_eq!(
            pending.pending(),
            &BTreeMap::from([(second.clone(), SourceBarrierEvidence::NoEvidence)])
        );
        assert_eq!(
            pending.detail(),
            "activation barrier is not cleared for: app-two/gateway (no completeness signal)"
        );

        let cleared = barrier
            .clear(&BTreeMap::from([
                (first.clone(), &first_record),
                (second.clone(), &second_record),
            ]))
            .expect("every required source cleared");
        assert_eq!(
            cleared.acknowledged_filter_epochs(),
            &BTreeMap::from([(first, FilterEpoch::new(9)), (second, FilterEpoch::new(9))])
        );
    }

    /// The acknowledged epoch recorded is the one the source **reported**, not the target.
    #[test]
    fn the_cleared_proof_records_the_reported_filter_epoch() {
        let identity = source("app");
        let barrier = barrier(&identity);
        let record = evidence(
            &identity,
            1,
            Some(4),
            Some(4),
            11,
            false,
            observation(BARRIER_OBSERVED_AT),
        );
        let cleared = barrier
            .clear(&BTreeMap::from([(identity.clone(), &record)]))
            .expect("a newer epoch clears");
        assert_eq!(
            cleared.acknowledged_filter_epochs(),
            &BTreeMap::from([(identity, FilterEpoch::new(11))])
        );
    }

    /// `BarrierPending::detail` is deterministic and sorted by source identity.
    #[test]
    fn pending_detail_is_deterministic_in_source_order() {
        let first = source("app-a");
        let second = source("app-b");
        let barrier = ActivationBarrier::new(
            FilterEpoch::new(9),
            BTreeMap::from([
                (
                    second.clone(),
                    ActivationWatermark::new(
                        SequenceEpoch::new(1),
                        SequenceNumber::new(4),
                        observation(BARRIER_OBSERVED_AT),
                    ),
                ),
                (
                    first.clone(),
                    ActivationWatermark::new(
                        SequenceEpoch::new(1),
                        SequenceNumber::new(4),
                        observation(BARRIER_OBSERVED_AT),
                    ),
                ),
            ]),
        )
        .expect("a barrier naming two sources is constructible");

        let pending = barrier
            .clear(&BTreeMap::new())
            .expect_err("two silent sources hold the barrier");
        assert_eq!(
            pending.detail(),
            "activation barrier is not cleared for: app-a/gateway (no completeness signal), \
             app-b/gateway (no completeness signal)"
        );
    }

    /// A published Event Filter entry declaring activation Watermarks reads back as a
    /// barrier.
    ///
    /// The signature takes the **validated** [`EventFilter`] rather than the raw
    /// per-Specification message, so the entry reaching `try_from_event_filter` has
    /// already passed `InvalidReadinessMode`, `TargetEpochMismatch`,
    /// `ActivationBarrierIncomplete`, the duplicate-source check and the `observed_at`
    /// validity check. There is no second construction path around those refusals.
    #[test]
    fn a_published_event_filter_entry_reads_back_as_a_barrier() {
        let filter = EventFilter::try_from(published_filter(vec![wire_watermark()]))
            .expect("the fixture is a valid Event Filter");

        let barrier = ActivationBarrier::try_from_event_filter(&filter, "spec", "v1")
            .expect("the entry declares an activation barrier");

        assert_eq!(barrier.target_filter_epoch(), FilterEpoch::new(9));
        assert_eq!(
            barrier.watermarks(),
            &BTreeMap::from([(
                source("app"),
                ActivationWatermark::new(
                    SequenceEpoch::new(1),
                    SequenceNumber::new(4),
                    observation(BARRIER_OBSERVED_AT),
                ),
            )])
        );
    }

    /// An entry declaring no activation Watermark is refused rather than read as an empty
    /// barrier that clears immediately.
    ///
    /// This is what every Specification compiled today emits, so the refusal is the live
    /// case rather than a defensive one.
    #[test]
    fn an_entry_declaring_no_activation_watermark_is_refused() {
        let filter = EventFilter::try_from(published_filter(Vec::new()))
            .expect("a shadow entry needs no activation Watermark to validate");

        assert_eq!(
            ActivationBarrier::try_from_event_filter(&filter, "spec", "v1"),
            Err(ActivationBarrierError::NoActivationBarrier)
        );
        assert_eq!(
            ActivationBarrier::try_from_event_filter(&filter, "other", "v1"),
            Err(ActivationBarrierError::SpecificationNotFound)
        );
    }

    /// The drift guard the module header argues for, in executable form.
    ///
    /// The match below is over `CompletenessEvidence`, is exhaustive, and carries **no
    /// wildcard** — which only a same-crate match can be, because the enum is
    /// `#[non_exhaustive]`. Adding a variant to `CompletenessEvidence` fails to compile
    /// here **and** in `ActivationBarrier::classify`, so the wrapper cannot silently fold
    /// a new source-completeness condition into an existing barrier answer. The count
    /// assertion catches a rename that keeps the arity.
    #[test]
    fn the_completeness_evidence_match_carries_no_wildcard() {
        const fn name(evidence: CompletenessEvidence) -> &'static str {
            match evidence {
                CompletenessEvidence::NoEvidence => "NoEvidence",
                CompletenessEvidence::DifferentSource => "DifferentSource",
                CompletenessEvidence::DifferentEpoch => "DifferentEpoch",
                CompletenessEvidence::DifferentSentinelClock => "DifferentSentinelClock",
                CompletenessEvidence::StaleFilter { .. } => "StaleFilter",
                CompletenessEvidence::FutureFilter { .. } => "FutureFilter",
                CompletenessEvidence::ReportedNoEvents { .. } => "ReportedNoEvents",
                CompletenessEvidence::EmissionsNotComplete { .. } => "EmissionsNotComplete",
                CompletenessEvidence::CompleteThrough { .. } => "CompleteThrough",
            }
        }

        let epochs = (FilterEpoch::new(8), FilterEpoch::new(9));
        let sequence = SequenceNumber::new(4);
        let variants = [
            CompletenessEvidence::NoEvidence,
            CompletenessEvidence::DifferentSource,
            CompletenessEvidence::DifferentEpoch,
            CompletenessEvidence::DifferentSentinelClock,
            CompletenessEvidence::StaleFilter {
                reported: epochs.0,
                required: epochs.1,
            },
            CompletenessEvidence::FutureFilter {
                reported: epochs.1,
                required: epochs.0,
            },
            CompletenessEvidence::ReportedNoEvents { idle: false },
            CompletenessEvidence::EmissionsNotComplete {
                last_emitted: sequence,
                idle: false,
            },
            CompletenessEvidence::CompleteThrough {
                sequence,
                last_emitted: sequence,
                idle: false,
            },
        ];
        assert_eq!(
            variants.len(),
            9,
            "every CompletenessEvidence variant must be named"
        );
        let names: Vec<&'static str> = variants.into_iter().map(name).collect();
        assert_eq!(names.len(), 9);
    }

    #[test]
    fn availability_contract_accessors_return_what_accept_recorded() {
        let contract =
            AvailabilityContract::accept("sagashop.p4.ship-requires-prior-charge", "v1", "ops");
        assert_eq!(
            contract.specification_id(),
            "sagashop.p4.ship-requires-prior-charge"
        );
        assert_eq!(contract.specification_version(), "v1");
        assert_eq!(contract.accepted_by(), "ops");
    }

    #[test]
    #[should_panic(expected = "specification id must not be empty")]
    fn availability_contract_refuses_an_empty_specification_id() {
        let _ = AvailabilityContract::accept("  ", "v1", "ops");
    }

    #[test]
    #[should_panic(expected = "specification version must not be empty")]
    fn availability_contract_refuses_an_empty_specification_version() {
        let _ = AvailabilityContract::accept("spec", "", "ops");
    }

    #[test]
    #[should_panic(expected = "accepting operator identity must not be empty")]
    fn availability_contract_refuses_an_empty_operator_identity() {
        let _ = AvailabilityContract::accept("spec", "v1", "");
    }

    #[test]
    fn availability_contract_acceptance_is_keyed_by_specification_identity() {
        let contract = AvailabilityContract::accept("sagashop.p4", "v1", "ops");
        assert!(contract.accepts("sagashop.p4", "v1"));
        assert!(!contract.accepts("sagashop.p4", "v2"));
        assert!(!contract.accepts("sagashop.p6", "v1"));
    }

    #[test]
    fn availability_contract_orders_by_identity_then_operator() {
        let left = AvailabilityContract::accept("a", "v1", "ops");
        let same = AvailabilityContract::accept("a", "v1", "ops");
        let other_version = AvailabilityContract::accept("a", "v2", "ops");
        let other_operator = AvailabilityContract::accept("a", "v1", "zmp");
        assert_eq!(left, same);
        assert!(left < other_version);
        assert!(left < other_operator);
    }
}
