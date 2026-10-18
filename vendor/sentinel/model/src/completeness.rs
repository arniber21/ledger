//! Validated source-completeness lifecycle over generated wire storage.
#![allow(clippy::missing_panics_doc)]

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use prost::Message;

use crate::canonical::{compare_occurrence_intervals, source_from_wire};
use crate::wire::model::v1 as raw;
use crate::{
    ClockRelation, Event, FilterEpoch, ObservationTime, OccurrenceTime, SequenceEpoch,
    SequenceNumber, SourceAssurance, SourceIdentity,
};

/// Producer-reported source activity.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceActivity {
    /// The source can emit Events.
    Active {
        /// Producer-clock state start.
        since: OccurrenceTime,
    },
    /// The source is intentionally idle.
    Idle {
        /// Producer-clock state start.
        since: OccurrenceTime,
    },
}

impl SourceActivity {
    /// Returns when the state began.
    #[must_use]
    pub const fn since(&self) -> &OccurrenceTime {
        match self {
            Self::Active { since } | Self::Idle { since } => since,
        }
    }

    /// Returns whether the source is intentionally idle.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self, Self::Idle { .. })
    }
}

/// Completeness validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompletenessSignalError {
    /// A required nested message is absent.
    MissingField(&'static str),
    /// A required enum is zero or unknown.
    InvalidActivity(i32),
    /// A completion bound exists without an emitted bound.
    CompleteWithoutEmission,
    /// Completion exceeds the highest emitted sequence.
    CompleteBeyondLastEmitted,
    /// Activity and report time use different producer clocks.
    ActivityClockMismatch,
    /// Activity begins definitively after its report.
    ActivitySinceAfterReport,
    /// An uncertainty interval overflowed i128.
    ActivityTimeUnrepresentable,
    /// Receiver-owned source assurance is invalid.
    InvalidAssurance(crate::EventError),
    /// Producer-authored occurrence time is invalid.
    InvalidOccurrence(crate::EventError),
    /// Receiver-owned observation time is invalid.
    InvalidObservation(crate::EventError),
    /// The bytes did not contain a valid Protobuf message.
    Decode(String),
}

impl Display for CompletenessSignalError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => {
                write!(formatter, "required completeness field is missing: {field}")
            }
            Self::InvalidActivity(value) => {
                write!(formatter, "invalid source activity enum: {value}")
            }
            Self::CompleteWithoutEmission => {
                formatter.write_str("complete-through requires last-emitted")
            }
            Self::CompleteBeyondLastEmitted => {
                formatter.write_str("complete-through exceeds last-emitted")
            }
            Self::ActivityClockMismatch => {
                formatter.write_str("activity and report times use different clocks")
            }
            Self::ActivitySinceAfterReport => {
                formatter.write_str("activity begins after producer report")
            }
            Self::ActivityTimeUnrepresentable => {
                formatter.write_str("activity time interval is unrepresentable")
            }
            Self::InvalidAssurance(error) => {
                write!(formatter, "invalid completeness assurance: {error}")
            }
            Self::InvalidOccurrence(error) => {
                write!(formatter, "invalid producer occurrence time: {error}")
            }
            Self::InvalidObservation(error) => {
                write!(formatter, "invalid completeness observation: {error}")
            }
            Self::Decode(error) => write!(formatter, "invalid protobuf bytes: {error}"),
        }
    }
}

impl Error for CompletenessSignalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidAssurance(error)
            | Self::InvalidOccurrence(error)
            | Self::InvalidObservation(error) => Some(error),
            _ => None,
        }
    }
}

/// Producer-authored activity and sequence bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct ProducerCompletenessSignal(raw::ProducerCompletenessSignal);

impl ProducerCompletenessSignal {
    /// Constructs producer completeness evidence.
    ///
    /// # Errors
    ///
    /// Returns an error for inconsistent sequence bounds, clocks, or activity time.
    pub fn new(
        epoch: SequenceEpoch,
        last_emitted: Option<SequenceNumber>,
        complete_through: Option<SequenceNumber>,
        producer_reported_at: OccurrenceTime,
        filter_epoch: FilterEpoch,
        activity: SourceActivity,
    ) -> Result<Self, CompletenessSignalError> {
        let (activity_value, since) = match activity {
            SourceActivity::Active { since } => (raw::SourceActivityState::Active as i32, since),
            SourceActivity::Idle { since } => {
                (raw::SourceActivityState::IntentionallyIdle as i32, since)
            }
        };
        Self::try_from(raw::ProducerCompletenessSignal {
            sequence_epoch: epoch.value(),
            last_emitted: last_emitted.map(SequenceNumber::value),
            complete_through: complete_through.map(SequenceNumber::value),
            producer_reported_at: Some(producer_reported_at.0),
            acknowledged_filter_epoch: Some(filter_epoch.value()),
            activity: activity_value,
            activity_since: Some(since.0),
        })
    }

    /// Returns sequence epoch.
    #[must_use]
    pub const fn epoch(&self) -> SequenceEpoch {
        SequenceEpoch::new(self.0.sequence_epoch)
    }
    /// Returns highest emitted sequence.
    #[must_use]
    pub const fn last_emitted(&self) -> Option<SequenceNumber> {
        match self.0.last_emitted {
            Some(value) => Some(SequenceNumber::new(value)),
            None => None,
        }
    }
    /// Returns completeness boundary.
    #[must_use]
    pub const fn complete_through(&self) -> Option<SequenceNumber> {
        match self.0.complete_through {
            Some(value) => Some(SequenceNumber::new(value)),
            None => None,
        }
    }
    /// Returns producer report time.
    #[must_use]
    pub fn producer_reported_at(&self) -> OccurrenceTime {
        OccurrenceTime(
            self.0
                .producer_reported_at
                .clone()
                .expect("validated report time"),
        )
    }
    /// Returns acknowledged Filter epoch.
    #[must_use]
    pub fn filter_epoch(&self) -> FilterEpoch {
        FilterEpoch::new(
            self.0
                .acknowledged_filter_epoch
                .expect("validated Filter epoch"),
        )
    }
    /// Returns source activity.
    #[must_use]
    pub fn activity(&self) -> SourceActivity {
        let since = OccurrenceTime(
            self.0
                .activity_since
                .clone()
                .expect("validated activity time"),
        );
        match raw::SourceActivityState::try_from(self.0.activity).expect("validated activity") {
            raw::SourceActivityState::Active => SourceActivity::Active { since },
            raw::SourceActivityState::IntentionallyIdle => SourceActivity::Idle { since },
            raw::SourceActivityState::Unspecified => unreachable!("validated activity"),
        }
    }
}

impl TryFrom<raw::ProducerCompletenessSignal> for ProducerCompletenessSignal {
    type Error = CompletenessSignalError;

    fn try_from(value: raw::ProducerCompletenessSignal) -> Result<Self, Self::Error> {
        if value.acknowledged_filter_epoch.is_none() {
            return Err(CompletenessSignalError::MissingField(
                "acknowledged_filter_epoch",
            ));
        }
        match (value.last_emitted, value.complete_through) {
            (None, Some(_)) => return Err(CompletenessSignalError::CompleteWithoutEmission),
            (Some(last), Some(complete)) if complete > last => {
                return Err(CompletenessSignalError::CompleteBeyondLastEmitted)
            }
            _ => {}
        }
        let report = OccurrenceTime::try_from_wire(value.producer_reported_at.clone().ok_or(
            CompletenessSignalError::MissingField("producer_reported_at"),
        )?)
        .map_err(CompletenessSignalError::InvalidOccurrence)?;
        let since = OccurrenceTime::try_from_wire(
            value
                .activity_since
                .clone()
                .ok_or(CompletenessSignalError::MissingField("activity_since"))?,
        )
        .map_err(CompletenessSignalError::InvalidOccurrence)?;
        match raw::SourceActivityState::try_from(value.activity) {
            Ok(raw::SourceActivityState::Active | raw::SourceActivityState::IntentionallyIdle) => {}
            _ => return Err(CompletenessSignalError::InvalidActivity(value.activity)),
        }
        if since.clock_domain() != report.clock_domain() {
            return Err(CompletenessSignalError::ActivityClockMismatch);
        }
        match compare_occurrence_intervals(&since, &report) {
            ClockRelation::After => return Err(CompletenessSignalError::ActivitySinceAfterReport),
            ClockRelation::UnrepresentableInterval => {
                return Err(CompletenessSignalError::ActivityTimeUnrepresentable)
            }
            ClockRelation::Before
            | ClockRelation::Overlapping
            | ClockRelation::DifferentClockDomain
            | ClockRelation::MissingBoundedClockUncertainty { .. } => {}
        }
        Ok(Self(value))
    }
}

/// Sentinel-canonical completeness evidence with verified source and observation.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceCompletenessSignal(raw::SourceCompletenessSignal);

/// Compatibility name for the canonical completeness signal.
pub type CompletenessSignal = SourceCompletenessSignal;

impl SourceCompletenessSignal {
    /// Composes producer evidence with receiver-owned fields.
    #[must_use]
    pub fn observe(
        producer: ProducerCompletenessSignal,
        assurance: SourceAssurance,
        observed_at: ObservationTime,
    ) -> Self {
        Self(raw::SourceCompletenessSignal {
            producer_signal: Some(producer.0),
            assurance: Some(assurance.0),
            observed_at: Some(observed_at.0),
        })
    }

    /// Encodes the canonical completeness signal in Protobuf wire format.
    ///
    /// Persisting `SourceEvidence` faithfully requires this: `SourceAssurance`
    /// exposes no getters for `authenticated_identity_evidence`,
    /// `integrity_evidence` or `topology_id`, so a column-wise round trip of
    /// this signal is lossy. A store that needs the signal back byte-for-byte
    /// keeps the encoded form rather than re-deriving it from projected
    /// columns.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        self.0.encode_to_vec()
    }

    /// Decodes and validates an untrusted wire message.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed bytes or a decoded message that
    /// violates canonical completeness-signal invariants.
    pub fn decode(bytes: &[u8]) -> Result<Self, CompletenessSignalError> {
        let raw = raw::SourceCompletenessSignal::decode(bytes)
            .map_err(|error| CompletenessSignalError::Decode(error.to_string()))?;
        Self::try_from(raw)
    }

    /// Returns producer evidence.
    #[must_use]
    pub fn producer(&self) -> ProducerCompletenessSignal {
        ProducerCompletenessSignal(
            self.0
                .producer_signal
                .clone()
                .expect("validated producer signal"),
        )
    }
    /// Returns authenticated source.
    #[must_use]
    pub fn source(&self) -> SourceIdentity {
        let assurance = self.0.assurance.as_ref().expect("validated assurance");
        source_from_wire(assurance.source.as_ref().expect("validated source"))
            .expect("validated source")
    }
    /// Returns sequence epoch.
    #[must_use]
    pub fn epoch(&self) -> SequenceEpoch {
        self.producer().epoch()
    }
    /// Returns last emitted.
    #[must_use]
    pub fn last_emitted(&self) -> Option<SequenceNumber> {
        self.producer().last_emitted()
    }
    /// Returns complete-through.
    #[must_use]
    pub fn complete_through(&self) -> Option<SequenceNumber> {
        self.producer().complete_through()
    }
    /// Returns producer report time.
    #[must_use]
    pub fn producer_reported_at(&self) -> OccurrenceTime {
        self.producer().producer_reported_at()
    }
    /// Returns Sentinel observation time.
    #[must_use]
    pub fn observed_at(&self) -> ObservationTime {
        ObservationTime(self.0.observed_at.clone().expect("validated observation"))
    }
    /// Returns acknowledged Filter epoch.
    #[must_use]
    pub fn filter_epoch(&self) -> FilterEpoch {
        self.producer().filter_epoch()
    }
    /// Returns activity.
    #[must_use]
    pub fn activity(&self) -> SourceActivity {
        self.producer().activity()
    }
}

impl TryFrom<raw::SourceCompletenessSignal> for SourceCompletenessSignal {
    type Error = CompletenessSignalError;

    fn try_from(mut value: raw::SourceCompletenessSignal) -> Result<Self, Self::Error> {
        let producer = ProducerCompletenessSignal::try_from(
            value
                .producer_signal
                .take()
                .ok_or(CompletenessSignalError::MissingField("producer_signal"))?,
        )?;
        let assurance = SourceAssurance::try_from(
            value
                .assurance
                .take()
                .ok_or(CompletenessSignalError::MissingField("assurance"))?,
        )
        .map_err(CompletenessSignalError::InvalidAssurance)?;
        let observation = ObservationTime::try_from_wire(
            value
                .observed_at
                .take()
                .ok_or(CompletenessSignalError::MissingField("observed_at"))?,
        )
        .map_err(CompletenessSignalError::InvalidObservation)?;
        value.producer_signal = Some(producer.0);
        value.assurance = Some(assurance.0);
        value.observed_at = Some(observation.0);
        Ok(Self(value))
    }
}

/// Caller-supplied Sentinel-clock freshness boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletenessFreshness {
    observed_not_before: ObservationTime,
}

impl CompletenessFreshness {
    /// Creates a freshness boundary.
    #[must_use]
    pub const fn observed_not_before(observation_time: ObservationTime) -> Self {
        Self {
            observed_not_before: observation_time,
        }
    }
    /// Returns the boundary.
    #[must_use]
    pub const fn boundary(&self) -> &ObservationTime {
        &self.observed_not_before
    }
    /// Returns whether `signal` meets this freshness bound on the same Sentinel clock.
    ///
    /// `None` means the signal and the bound name different Sentinel clocks, so freshness
    /// is undecidable. Kept `pub(crate)` so [`crate::ActivationBarrier`] can reuse the
    /// comparison for liveness without re-coding the `>=` (ADR-0027).
    #[must_use]
    pub(crate) fn accepts(&self, signal: &SourceCompletenessSignal) -> Option<bool> {
        let observed = signal.observed_at();
        if observed.sentinel_clock() == self.observed_not_before.sentinel_clock() {
            Some(observed.sentinel_nanoseconds() >= self.observed_not_before.sentinel_nanoseconds())
        } else {
            None
        }
    }
}

/// Dual Sentinel-clock freshness floors for active versus intentionally-idle sources.
///
/// O-12's liveness half (#163): a source that cleared the activation barrier and then
/// went silent must stop being admitted **by name**, and an intentionally-idle source
/// is held to a separately stated (longer) bound. Both floors are parameters — this
/// type reads no clock — and the constructor refuses a mismatched Sentinel clock or an
/// idle floor stricter than the active floor (ADR-0031 Mode A).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLivenessFreshness {
    active: CompletenessFreshness,
    idle: CompletenessFreshness,
}

/// Refusal to construct a [`SourceLivenessFreshness`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceLivenessFreshnessError {
    /// The active and idle floors name different Sentinel clocks.
    SentinelClockMismatch,
    /// The idle floor requires a fresher signal than the active floor.
    IdleStricterThanActive,
}

impl std::fmt::Display for SourceLivenessFreshnessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SentinelClockMismatch => formatter
                .write_str("active and idle liveness floors must use the same Sentinel clock"),
            Self::IdleStricterThanActive => formatter
                .write_str("idle liveness floor must not be stricter than the active floor"),
        }
    }
}

impl std::error::Error for SourceLivenessFreshnessError {}

impl SourceLivenessFreshness {
    /// Builds dual liveness floors.
    ///
    /// # Errors
    ///
    /// Returns [`SourceLivenessFreshnessError::SentinelClockMismatch`] when the floors
    /// disagree on Sentinel clock, and
    /// [`SourceLivenessFreshnessError::IdleStricterThanActive`] when the idle floor's
    /// bound is later than the active floor's (idle silence window shorter than active).
    pub fn try_new(
        active: CompletenessFreshness,
        idle: CompletenessFreshness,
    ) -> Result<Self, SourceLivenessFreshnessError> {
        let active_boundary = active.boundary();
        let idle_boundary = idle.boundary();
        if active_boundary.sentinel_clock() != idle_boundary.sentinel_clock() {
            return Err(SourceLivenessFreshnessError::SentinelClockMismatch);
        }
        if idle_boundary.sentinel_nanoseconds() > active_boundary.sentinel_nanoseconds() {
            return Err(SourceLivenessFreshnessError::IdleStricterThanActive);
        }
        Ok(Self { active, idle })
    }

    /// Returns the active-source liveness floor.
    #[must_use]
    pub const fn active(&self) -> &CompletenessFreshness {
        &self.active
    }

    /// Returns the intentionally-idle liveness floor.
    #[must_use]
    pub const fn idle(&self) -> &CompletenessFreshness {
        &self.idle
    }

    /// Selects the floor that applies to `activity`.
    #[must_use]
    pub const fn for_activity(&self, activity: &SourceActivity) -> &CompletenessFreshness {
        if activity.is_idle() {
            &self.idle
        } else {
            &self.active
        }
    }
}

/// Evidence a completeness signal supplies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompletenessEvidence {
    /// No fresh signal.
    NoEvidence,
    /// Another source.
    DifferentSource,
    /// Another sequence epoch.
    DifferentEpoch,
    /// Different Sentinel clock.
    DifferentSentinelClock,
    /// Older Filter epoch.
    StaleFilter {
        /// Reported epoch.
        reported: FilterEpoch,
        /// Required epoch.
        required: FilterEpoch,
    },
    /// Newer Filter epoch.
    FutureFilter {
        /// Reported epoch.
        reported: FilterEpoch,
        /// Required epoch.
        required: FilterEpoch,
    },
    /// Explicitly no Events.
    ReportedNoEvents {
        /// Whether source is intentionally idle.
        idle: bool,
    },
    /// Emissions without completed boundary.
    EmissionsNotComplete {
        /// Highest emitted sequence.
        last_emitted: SequenceNumber,
        /// Whether source is intentionally idle.
        idle: bool,
    },
    /// Completed boundary.
    CompleteThrough {
        /// Completed sequence boundary.
        sequence: SequenceNumber,
        /// Highest emitted sequence.
        last_emitted: SequenceNumber,
        /// Whether source is intentionally idle.
        idle: bool,
    },
}

/// Classifies source evidence without I/O or hidden clock reads.
#[must_use]
pub fn classify_completeness(
    signal: Option<&SourceCompletenessSignal>,
    required_source: &SourceIdentity,
    required_epoch: SequenceEpoch,
    required_filter_epoch: FilterEpoch,
    freshness: &CompletenessFreshness,
) -> CompletenessEvidence {
    let Some(signal) = signal else {
        return CompletenessEvidence::NoEvidence;
    };
    if signal.source() != *required_source {
        return CompletenessEvidence::DifferentSource;
    }
    if signal.epoch() != required_epoch {
        return CompletenessEvidence::DifferentEpoch;
    }
    match freshness.accepts(signal) {
        None => return CompletenessEvidence::DifferentSentinelClock,
        Some(false) => return CompletenessEvidence::NoEvidence,
        Some(true) => {}
    }
    if signal.filter_epoch() < required_filter_epoch {
        return CompletenessEvidence::StaleFilter {
            reported: signal.filter_epoch(),
            required: required_filter_epoch,
        };
    }
    if signal.filter_epoch() > required_filter_epoch {
        return CompletenessEvidence::FutureFilter {
            reported: signal.filter_epoch(),
            required: required_filter_epoch,
        };
    }
    let idle = signal.activity().is_idle();
    match (signal.last_emitted(), signal.complete_through()) {
        (None, None) => CompletenessEvidence::ReportedNoEvents { idle },
        (Some(last_emitted), None) => {
            CompletenessEvidence::EmissionsNotComplete { last_emitted, idle }
        }
        (Some(last_emitted), Some(sequence)) => CompletenessEvidence::CompleteThrough {
            sequence,
            last_emitted,
            idle,
        },
        (None, Some(_)) => unreachable!("validated completeness invariant"),
    }
}

/// Whether an Event arrived behind a completeness boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LateRelation {
    /// Different source.
    DifferentSource,
    /// Different epoch.
    DifferentEpoch,
    /// Different producer clock.
    DifferentClockDomain,
    /// The Event and signal use different Sentinel clocks.
    DifferentSentinelClock,
    /// Sentinel observed the Event before it observed the signal.
    ObservedBeforeSignal,
    /// Sentinel observed the Event at the same clock reading as the signal.
    ObservedAtSignal,
    /// The signal does not establish a completed sequence boundary.
    NoCompleteBoundary,
    /// The Event sequence is ahead of the completed boundary.
    AheadOfBoundary,
    /// The Event sequence is at or behind the boundary.
    BehindBoundary {
        /// Number of positions from the boundary; zero means exactly at it.
        positions: u64,
    },
}

/// Classifies lateness after matching clock domains and enforcing observation order.
///
/// A completed sequence boundary makes an Event late only when Sentinel observed
/// the signal strictly before the Event. An Event observed before or at the signal
/// is classified explicitly and cannot be labelled late from producer sequence alone.
#[must_use]
pub fn classify_lateness(event: &Event, signal: &SourceCompletenessSignal) -> LateRelation {
    if event.source() != signal.source() {
        return LateRelation::DifferentSource;
    }
    if event.delivery().epoch() != signal.epoch() {
        return LateRelation::DifferentEpoch;
    }
    if event.occurrence_time().clock_domain() != signal.producer_reported_at().clock_domain() {
        return LateRelation::DifferentClockDomain;
    }
    if event.observation_time().sentinel_clock() != signal.observed_at().sentinel_clock() {
        return LateRelation::DifferentSentinelClock;
    }
    match event
        .observation_time()
        .sentinel_nanoseconds()
        .cmp(&signal.observed_at().sentinel_nanoseconds())
    {
        std::cmp::Ordering::Less => return LateRelation::ObservedBeforeSignal,
        std::cmp::Ordering::Equal => return LateRelation::ObservedAtSignal,
        std::cmp::Ordering::Greater => {}
    }
    let Some(boundary) = signal.complete_through() else {
        return LateRelation::NoCompleteBoundary;
    };
    let sequence = event.delivery().sequence().value();
    if sequence > boundary.value() {
        LateRelation::AheadOfBoundary
    } else {
        LateRelation::BehindBoundary {
            positions: boundary.value().saturating_sub(sequence),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClockDomainId, Integrity, MonitoredApplicationId, SentinelClockId, SourceCapabilities,
        SourceId, SourceIdentity, SourceTier,
    };

    fn signal() -> SourceCompletenessSignal {
        let source = SourceIdentity::new(
            MonitoredApplicationId::new("app").expect("valid id"),
            SourceId::new("gateway").expect("valid id"),
        );
        let occurrence =
            OccurrenceTime::new(ClockDomainId::new("producer").expect("valid id"), 10, 0);
        let producer = ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            Some(SequenceNumber::new(4)),
            Some(SequenceNumber::new(4)),
            occurrence.clone(),
            FilterEpoch::new(9),
            SourceActivity::Active { since: occurrence },
        )
        .expect("valid producer signal");
        let assurance = SourceAssurance::new(
            source,
            SourceTier::Anchor,
            SourceCapabilities::new([]),
            Integrity::Authenticated,
            Vec::new(),
            Vec::new(),
            "topology",
        )
        .expect("valid assurance");
        let observed_at =
            ObservationTime::new(SentinelClockId::new("sentinel").expect("valid id"), 20);
        SourceCompletenessSignal::observe(producer, assurance, observed_at)
    }

    #[test]
    fn source_completeness_signal_decode_round_trips_through_encode_to_vec() {
        // Store/src/lib.rs (#16, ADR-0011) needs the completeness signal back
        // byte-for-byte: `SourceAssurance` exposes no getters for
        // `authenticated_identity_evidence`, `integrity_evidence` or
        // `topology_id`, so a column-wise round trip is lossy. This pins the
        // encode/decode round trip the store depends on, with no `.proto`
        // change (the message already exists in `gen/rust`).
        let original = signal();
        let bytes = original.encode_to_vec();
        let decoded = SourceCompletenessSignal::decode(&bytes).expect("decodes its own bytes");
        assert_eq!(decoded, original);
        assert_eq!(decoded.encode_to_vec(), bytes);
    }

    #[test]
    fn source_completeness_signal_decode_rejects_garbage_and_invalid_messages() {
        assert!(matches!(
            SourceCompletenessSignal::decode(&[0xFF, 0x00, 0xFF]),
            Err(CompletenessSignalError::Decode(_))
        ));

        let empty = raw::SourceCompletenessSignal::default().encode_to_vec();
        assert_eq!(
            SourceCompletenessSignal::decode(&empty),
            Err(CompletenessSignalError::MissingField("producer_signal"))
        );
    }
}
