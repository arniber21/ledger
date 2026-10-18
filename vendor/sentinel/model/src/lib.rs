//! Canonical Event model and assurance contracts.
//!
//! Implements the domain types defined in ADR-0015 (Events are points on
//! transitions) and ADR-0022 (assurance-tiered source classification):
//! the canonical [`Event`], source tiers, clock-domain metadata, causality
//! edge sets, and the source-completeness signal contract.

#[path = "event.rs"]
mod canonical;
#[path = "completeness.rs"]
mod contract;
mod decision;
mod evidence;
mod filter;
mod identifiers;
mod rollout;
mod verdict;
pub mod wire;
mod wire_helpers;

pub use canonical::{
    causal_relation, classify_delivery_identity, compare_occurrence_times, AttributeDouble,
    AttributeValue, CausalRelation, ClockRelation, DataClass, DeliveryIdentityRelation, Event,
    EventError, Integrity, MissingClockCapability, ObservationTime, OccurrenceTime, ProducerEvent,
    Sensitivity, SourceAssurance, SourceCapabilities, SourceCapability, SourceTier,
};
pub use contract::{
    classify_completeness, classify_lateness, CompletenessEvidence, CompletenessFreshness,
    CompletenessSignal, CompletenessSignalError, LateRelation, ProducerCompletenessSignal,
    SourceActivity, SourceCompletenessSignal, SourceLivenessFreshness,
    SourceLivenessFreshnessError,
};
pub use decision::{aggregate_unresolved, DecideRequest, DecideRequestError, UnresolvedResolution};
pub use evidence::{SequenceGap, SourceEvidence};
pub use filter::{EventFilter, EventFilterError};
pub use identifiers::{
    classify_delivery, AttributeKey, ClockDomainId, DeliveryRelation, DeliverySlot, EventId,
    EventKind, FilterEpoch, IdentifierError, MonitoredApplicationId, SchemaVersion,
    SentinelClockId, SequenceEpoch, SequenceNumber, SourceId, SourceIdentity,
};
pub use rollout::{
    ActivationBarrier, ActivationBarrierError, ActivationWatermark, AvailabilityContract,
    BarrierCleared, BarrierPending, SourceBarrierEvidence, BARRIER_PENDING_DETAIL_PREFIX,
};
pub use verdict::{
    ActivationBarrierStatus, EvidenceExtractRetentionDays, InconclusiveReason,
    RawEventRetentionDays, RedactedAttributeRef, RedactionPolicyVersion, RetentionWindow,
    SettlementBasisKind, SourceGapView, SourceTierGapView, TierSettlement, UncertaintyView,
    Verdict, VerdictAuthority, VerdictError, VerdictOutcome,
    DEFAULT_CONFIGURED_EVIDENCE_EXTRACT_RETENTION_DAYS, EVIDENCE_EXTRACT_RETENTION_DAYS_MAX,
    EVIDENCE_EXTRACT_RETENTION_DAYS_MIN, PLATFORM_DEFAULT_RAW_EVENT_RETENTION_DAYS,
    RAW_EVENT_RETENTION_DAYS_MAX, RAW_EVENT_RETENTION_DAYS_MIN,
};
