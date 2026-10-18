//! Validated views over the generated canonical Event wire storage.
#![allow(clippy::missing_panics_doc, clippy::needless_pass_by_value)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use prost::Message;

use crate::wire::model::v1 as raw;
use crate::{
    classify_delivery, AttributeKey, ClockDomainId, DeliveryRelation, DeliverySlot, EventId,
    EventKind, FilterEpoch, IdentifierError, MonitoredApplicationId, SchemaVersion,
    SentinelClockId, SequenceEpoch, SequenceNumber, SourceId, SourceIdentity,
};

fn int128_to_wire(value: i128) -> raw::Int128 {
    let bytes = value.to_be_bytes();
    let high = i64::from_be_bytes(
        bytes[..8]
            .try_into()
            .expect("i128 high word is eight bytes"),
    );
    let low = u64::from_be_bytes(bytes[8..].try_into().expect("i128 low word is eight bytes"));
    raw::Int128 { high, low }
}

fn int128_from_wire(value: &raw::Int128) -> i128 {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&value.high.to_be_bytes());
    bytes[8..].copy_from_slice(&value.low.to_be_bytes());
    i128::from_be_bytes(bytes)
}

fn require_text(value: &str, field: &'static str) -> Result<(), EventError> {
    if value.trim().is_empty() {
        Err(EventError::MissingScalar(field))
    } else {
        Ok(())
    }
}

/// Validation failure for an untrusted generated message.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EventError {
    /// A required nested message or oneof is absent.
    MissingField(&'static str),
    /// A required scalar is absent or empty.
    MissingScalar(&'static str),
    /// A raw enum number is not known to this schema.
    UnknownEnum {
        /// Enum field name.
        field: &'static str,
        /// Unrecognized numeric value.
        value: i32,
    },
    /// An enum used its deliberately invalid zero sentinel.
    UnspecifiedEnum(&'static str),
    /// A set-like repeated field contained a duplicate.
    DuplicateValue(&'static str),
    /// An attribute map contained the same key more than once.
    DuplicateAttributeKey,
    /// An Event listed itself as a causal predecessor.
    SelfCausalPredecessor,
    /// A floating-point attribute was NaN or infinite.
    NonFiniteAttributeDouble,
    /// An identifier field was empty or whitespace.
    InvalidIdentifier {
        /// Identifier field name.
        field: &'static str,
        /// Identifier validation failure.
        source: IdentifierError,
    },
    /// The bytes did not contain a valid Protobuf message.
    Decode(String),
}

impl Display for EventError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => {
                write!(formatter, "required message or oneof is missing: {field}")
            }
            Self::MissingScalar(field) => write!(formatter, "required scalar is missing: {field}"),
            Self::UnknownEnum { field, value } => {
                write!(formatter, "unknown {field} enum value: {value}")
            }
            Self::UnspecifiedEnum(field) => write!(formatter, "{field} must not be unspecified"),
            Self::DuplicateValue(field) => {
                write!(formatter, "duplicate value in set-like field: {field}")
            }
            Self::DuplicateAttributeKey => {
                formatter.write_str("attribute map contains a duplicate key")
            }
            Self::SelfCausalPredecessor => {
                formatter.write_str("event must not be its own causal predecessor")
            }
            Self::NonFiniteAttributeDouble => {
                formatter.write_str("attribute double must be finite")
            }
            Self::InvalidIdentifier { field, source } => {
                write!(formatter, "invalid identifier in {field}: {source}")
            }
            Self::Decode(error) => write!(formatter, "invalid protobuf bytes: {error}"),
        }
    }
}

impl Error for EventError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidIdentifier { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn invalid_identifier(field: &'static str, source: IdentifierError) -> EventError {
    EventError::InvalidIdentifier { field, source }
}

/// Finite IEEE-754 value suitable for deterministic Event equality.
#[derive(Clone, Copy, Debug)]
pub struct AttributeDouble(f64);

impl PartialEq for AttributeDouble {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for AttributeDouble {}

impl AttributeDouble {
    /// Returns the finite floating-point value.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for AttributeDouble {
    type Error = EventError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value.is_finite() {
            // IEEE-754 defines both zero encodings as numerically equal. Store one
            // representation so equality and Protobuf encoding agree.
            Ok(Self(if value == 0.0 { 0.0 } else { value }))
        } else {
            Err(EventError::NonFiniteAttributeDouble)
        }
    }
}

/// Validated, deterministic view of a Protobuf attribute value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AttributeValue {
    /// UTF-8 string value.
    String(String),
    /// Boolean value.
    Bool(bool),
    /// Signed 64-bit integer value.
    Integer(i64),
    /// Finite IEEE-754 value.
    Double(AttributeDouble),
    /// Arbitrary bytes.
    Bytes(Vec<u8>),
    /// Ordered array value.
    Array(Vec<Self>),
    /// Deterministically ordered key/value list.
    KeyValueList(BTreeMap<AttributeKey, Self>),
}

impl AttributeValue {
    fn try_from_wire(value: &raw::AttributeValue) -> Result<Self, EventError> {
        use raw::attribute_value::Value;
        match value
            .value
            .as_ref()
            .ok_or(EventError::MissingField("attribute.value"))?
        {
            Value::StringValue(value) => Ok(Self::String(value.clone())),
            Value::BoolValue(value) => Ok(Self::Bool(*value)),
            Value::IntegerValue(value) => Ok(Self::Integer(*value)),
            Value::DoubleValue(value) => Ok(Self::Double(AttributeDouble::try_from(*value)?)),
            Value::BytesValue(value) => Ok(Self::Bytes(value.clone())),
            Value::ArrayValue(array) => array
                .values
                .iter()
                .map(Self::try_from_wire)
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Array),
            Value::MapValue(map) => entries_from_wire(&map.entries).map(Self::KeyValueList),
        }
    }

    fn to_wire(&self) -> raw::AttributeValue {
        use raw::attribute_value::Value;
        let value = match self {
            Self::String(value) => Value::StringValue(value.clone()),
            Self::Bool(value) => Value::BoolValue(*value),
            Self::Integer(value) => Value::IntegerValue(*value),
            Self::Double(value) => Value::DoubleValue(value.value()),
            Self::Bytes(value) => Value::BytesValue(value.clone()),
            Self::Array(values) => Value::ArrayValue(raw::AttributeArray {
                values: values.iter().map(Self::to_wire).collect(),
            }),
            Self::KeyValueList(values) => Value::MapValue(raw::AttributeMap {
                entries: entries_to_wire(values),
            }),
        };
        raw::AttributeValue { value: Some(value) }
    }
}

fn entries_from_wire(
    entries: &[raw::AttributeEntry],
) -> Result<BTreeMap<AttributeKey, AttributeValue>, EventError> {
    let mut values = BTreeMap::new();
    for entry in entries {
        let key = AttributeKey::new(entry.key.clone())
            .map_err(|source| invalid_identifier("attribute.key", source))?;
        let value = entry
            .value
            .as_ref()
            .ok_or(EventError::MissingField("attribute.entry.value"))?;
        if values
            .insert(key, AttributeValue::try_from_wire(value)?)
            .is_some()
        {
            return Err(EventError::DuplicateAttributeKey);
        }
    }
    Ok(values)
}

fn entries_to_wire(values: &BTreeMap<AttributeKey, AttributeValue>) -> Vec<raw::AttributeEntry> {
    values
        .iter()
        .map(|(key, value)| raw::AttributeEntry {
            key: key.as_str().to_owned(),
            value: Some(value.to_wire()),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        })
        .collect()
}

fn canonical_entries(
    entries: Vec<raw::AttributeEntry>,
) -> Result<Vec<raw::AttributeEntry>, EventError> {
    entries_from_wire(&entries).map(|values| entries_to_wire(&values))
}

/// When an Event happened on its producer's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceTime(pub(crate) raw::OccurrenceTime);

impl Eq for OccurrenceTime {}

impl OccurrenceTime {
    /// Creates a producer-clock occurrence time and uncertainty bound.
    #[must_use]
    pub fn new(
        clock_domain: ClockDomainId,
        nanoseconds: i128,
        uncertainty_nanoseconds: u64,
    ) -> Self {
        Self(raw::OccurrenceTime {
            clock_domain_id: clock_domain.to_string(),
            nanoseconds: Some(int128_to_wire(nanoseconds)),
            uncertainty_nanoseconds,
        })
    }

    pub(crate) fn try_from_wire(value: raw::OccurrenceTime) -> Result<Self, EventError> {
        ClockDomainId::new(value.clock_domain_id.clone())
            .map_err(|source| invalid_identifier("occurrence_time.clock_domain_id", source))?;
        if value.nanoseconds.is_none() {
            return Err(EventError::MissingField("occurrence_time.nanoseconds"));
        }
        Ok(Self(value))
    }

    /// Returns the producer clock domain.
    #[must_use]
    pub fn clock_domain(&self) -> ClockDomainId {
        ClockDomainId::new(self.0.clock_domain_id.clone()).expect("validated clock domain")
    }

    /// Returns the producer-clock time in nanoseconds.
    #[must_use]
    pub fn nanoseconds(&self) -> i128 {
        int128_from_wire(self.0.nanoseconds.as_ref().expect("validated nanoseconds"))
    }

    /// Returns the symmetric uncertainty bound in nanoseconds.
    #[must_use]
    pub const fn uncertainty_nanoseconds(&self) -> u64 {
        self.0.uncertainty_nanoseconds
    }

    fn bounds(&self) -> Option<(i128, i128)> {
        let uncertainty = i128::from(self.uncertainty_nanoseconds());
        Some((
            self.nanoseconds().checked_sub(uncertainty)?,
            self.nanoseconds().checked_add(uncertainty)?,
        ))
    }
}

/// When Sentinel first observed an Event, expressed on Sentinel's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservationTime(pub(crate) raw::ObservationTime);

impl Eq for ObservationTime {}

impl ObservationTime {
    /// Creates a Sentinel-local observation time.
    #[must_use]
    pub fn new(sentinel_clock: SentinelClockId, sentinel_nanoseconds: i128) -> Self {
        Self(raw::ObservationTime {
            sentinel_clock_id: sentinel_clock.to_string(),
            nanoseconds: Some(int128_to_wire(sentinel_nanoseconds)),
        })
    }

    pub(crate) fn try_from_wire(value: raw::ObservationTime) -> Result<Self, EventError> {
        SentinelClockId::new(value.sentinel_clock_id.clone())
            .map_err(|source| invalid_identifier("observation_time.sentinel_clock_id", source))?;
        if value.nanoseconds.is_none() {
            return Err(EventError::MissingField("observation_time.nanoseconds"));
        }
        Ok(Self(value))
    }

    /// Returns the Sentinel-local clock identity.
    #[must_use]
    pub fn sentinel_clock(&self) -> SentinelClockId {
        SentinelClockId::new(self.0.sentinel_clock_id.clone()).expect("validated Sentinel clock")
    }

    /// Returns the Sentinel-clock time in nanoseconds.
    #[must_use]
    pub fn sentinel_nanoseconds(&self) -> i128 {
        int128_from_wire(self.0.nanoseconds.as_ref().expect("validated nanoseconds"))
    }
}

/// Sound ordering supplied by producer clock intervals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ClockRelation {
    /// The left interval is wholly before the right interval.
    Before,
    /// The left interval is wholly after the right interval.
    After,
    /// The uncertainty intervals overlap.
    Overlapping,
    /// The times use different producer clocks.
    DifferentClockDomain,
    /// At least one interval overflowed i128.
    UnrepresentableInterval,
    /// One or both sources lack bounded-clock capability.
    MissingBoundedClockUncertainty {
        /// Sources missing the capability.
        missing: MissingClockCapability,
    },
}

/// Which Event sources lack bounded clock uncertainty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MissingClockCapability {
    /// Only the left source lacks it.
    Left,
    /// Only the right source lacks it.
    Right,
    /// Both sources lack it.
    Both,
}

pub(crate) fn compare_occurrence_intervals(
    left: &OccurrenceTime,
    right: &OccurrenceTime,
) -> ClockRelation {
    if left.clock_domain() != right.clock_domain() {
        return ClockRelation::DifferentClockDomain;
    }
    let (Some((left_earliest, left_latest)), Some((right_earliest, right_latest))) =
        (left.bounds(), right.bounds())
    else {
        return ClockRelation::UnrepresentableInterval;
    };
    if left_latest < right_earliest {
        ClockRelation::Before
    } else if left_earliest > right_latest {
        ClockRelation::After
    } else {
        ClockRelation::Overlapping
    }
}

/// Capability verified for an Event source.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceCapability {
    /// Reports an Event before the effect.
    ObserveBeforeEffect,
    /// Reports explicit causal edges.
    CausalEdges,
    /// Reports completeness signals.
    CompletenessSignals,
    /// Supplies bounded clock uncertainty.
    BoundedClockUncertainty,
}

impl SourceCapability {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: i32, field: &'static str) -> Result<Self, EventError> {
        match raw::SourceCapability::try_from(value)
            .map_err(|_| EventError::UnknownEnum { field, value })?
        {
            raw::SourceCapability::Unspecified => Err(EventError::UnspecifiedEnum(field)),
            raw::SourceCapability::ObserveBeforeEffect => Ok(Self::ObserveBeforeEffect),
            raw::SourceCapability::CausalEdges => Ok(Self::CausalEdges),
            raw::SourceCapability::CompletenessSignals => Ok(Self::CompletenessSignals),
            raw::SourceCapability::BoundedClockUncertainty => Ok(Self::BoundedClockUncertainty),
        }
    }
}

/// Deterministically ordered capability set.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceCapabilities(BTreeSet<SourceCapability>);

impl SourceCapabilities {
    /// Creates a set, removing duplicates from trusted domain input.
    #[must_use]
    pub fn new(capabilities: impl IntoIterator<Item = SourceCapability>) -> Self {
        Self(capabilities.into_iter().collect())
    }

    fn try_from_wire(values: &[i32], field: &'static str) -> Result<Self, EventError> {
        let mut capabilities = BTreeSet::new();
        for value in values {
            if !capabilities.insert(SourceCapability::from_raw(*value, field)?) {
                return Err(EventError::DuplicateValue(field));
            }
        }
        Ok(Self(capabilities))
    }

    /// Tests set membership.
    #[must_use]
    pub fn contains(&self, capability: SourceCapability) -> bool {
        self.0.contains(&capability)
    }

    /// Iterates in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = SourceCapability> + '_ {
        self.0.iter().copied()
    }

    fn to_wire(&self) -> Vec<i32> {
        self.iter().map(SourceCapability::to_raw).collect()
    }
}

/// Receiver-verified integrity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Integrity {
    /// No independent authentication.
    Unverified,
    /// Transport-authenticated source.
    Authenticated,
    /// Independently verifiable tamper evidence.
    TamperEvident,
}

impl Integrity {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: i32) -> Result<Self, EventError> {
        match raw::Integrity::try_from(value).map_err(|_| EventError::UnknownEnum {
            field: "integrity",
            value,
        })? {
            raw::Integrity::Unspecified => Err(EventError::UnspecifiedEnum("integrity")),
            raw::Integrity::Unverified => Ok(Self::Unverified),
            raw::Integrity::Authenticated => Ok(Self::Authenticated),
            raw::Integrity::TamperEvident => Ok(Self::TamperEvident),
        }
    }
}

/// Producer sensitivity claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Sensitivity {
    /// Public data.
    Public,
    /// Deployment-internal data.
    Internal,
    /// Confidential data.
    Confidential,
    /// Most restricted data.
    Restricted,
}

impl Sensitivity {
    fn from_raw(value: i32) -> Result<Self, EventError> {
        match raw::Sensitivity::try_from(value).map_err(|_| EventError::UnknownEnum {
            field: "sensitivity",
            value,
        })? {
            raw::Sensitivity::Unspecified => Err(EventError::UnspecifiedEnum("sensitivity")),
            raw::Sensitivity::Public => Ok(Self::Public),
            raw::Sensitivity::Internal => Ok(Self::Internal),
            raw::Sensitivity::Confidential => Ok(Self::Confidential),
            raw::Sensitivity::Restricted => Ok(Self::Restricted),
        }
    }

    const fn to_raw(self) -> i32 {
        match self {
            Self::Public => raw::Sensitivity::Public as i32,
            Self::Internal => raw::Sensitivity::Internal as i32,
            Self::Confidential => raw::Sensitivity::Confidential as i32,
            Self::Restricted => raw::Sensitivity::Restricted as i32,
        }
    }
}

/// Which data class an Event belongs to (ADR-0029).
///
/// Monitored-Tenant data is governed by ADR-0026: tenancy, RLS, retention and
/// evidence extracts all key off it. Corpus data is donated, sanitized evidence
/// under a DUA and none of that governance applies — different retention,
/// different access semantics, no Verdicts ever rest on it. The two classes
/// must never mix, and a classless Event is the silent-wrong failure ADR-0029
/// exists to prevent, so [`crate::Event`] rejects `Unspecified` rather than
/// defaulting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DataClass {
    /// Monitored-Tenant data under ADR-0026 governance.
    Tenant,
    /// Donated corpus data under a DUA (ADR-0029), never tenant-scoped.
    Corpus,
}

impl DataClass {
    fn from_raw(value: i32) -> Result<Self, EventError> {
        match raw::DataClass::try_from(value).map_err(|_| EventError::UnknownEnum {
            field: "data_class",
            value,
        })? {
            raw::DataClass::Unspecified => Err(EventError::UnspecifiedEnum("data_class")),
            raw::DataClass::Tenant => Ok(Self::Tenant),
            raw::DataClass::Corpus => Ok(Self::Corpus),
        }
    }

    const fn to_raw(self) -> i32 {
        match self {
            Self::Tenant => raw::DataClass::Tenant as i32,
            Self::Corpus => raw::DataClass::Corpus as i32,
        }
    }
}

/// Deployment-configured assurance tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceTier {
    /// Source the Monitored Application cannot suppress or forge.
    Anchor,
    /// Source inside the Monitored Application.
    Contributing,
}

impl SourceTier {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: i32) -> Result<Self, EventError> {
        match raw::SourceTier::try_from(value).map_err(|_| EventError::UnknownEnum {
            field: "source_tier",
            value,
        })? {
            raw::SourceTier::Unspecified => Err(EventError::UnspecifiedEnum("source_tier")),
            raw::SourceTier::Anchor => Ok(Self::Anchor),
            raw::SourceTier::Contributing => Ok(Self::Contributing),
        }
    }
}

pub(crate) fn source_from_wire(source: &raw::SourceIdentity) -> Result<SourceIdentity, EventError> {
    let application = MonitoredApplicationId::new(source.monitored_application_id.clone())
        .map_err(|error| invalid_identifier("source.monitored_application_id", error))?;
    let source_id = SourceId::new(source.source_id.clone())
        .map_err(|error| invalid_identifier("source.source_id", error))?;
    Ok(SourceIdentity::new(application, source_id))
}

/// Producer-authored Event evidence, without receiver-owned assurance fields.
#[derive(Clone, Debug, PartialEq)]
pub struct ProducerEvent(pub(crate) raw::ProducerEvent);

impl ProducerEvent {
    /// Constructs trusted producer evidence using domain values.
    ///
    /// # Errors
    ///
    /// Returns an error if nested values violate the canonical wire contract.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: EventId,
        epoch: SequenceEpoch,
        sequence: SequenceNumber,
        schema_version: SchemaVersion,
        filter_epoch: FilterEpoch,
        kind: EventKind,
        occurrence_time: OccurrenceTime,
        attributes: BTreeMap<AttributeKey, AttributeValue>,
        claimed_capabilities: SourceCapabilities,
        sensitivity: Sensitivity,
        causal_predecessors: BTreeSet<EventId>,
    ) -> Result<Self, EventError> {
        Self::try_from(raw::ProducerEvent {
            id: id.to_string(),
            sequence: Some(raw::SequenceCoordinate {
                epoch: epoch.value(),
                sequence: sequence.value(),
            }),
            schema_version: schema_version.to_string(),
            acknowledged_filter_epoch: Some(filter_epoch.value()),
            kind: kind.to_string(),
            occurrence_time: Some(occurrence_time.0),
            attributes: entries_to_wire(&attributes),
            claimed_capabilities: claimed_capabilities.to_wire(),
            claimed_sensitivity: sensitivity.to_raw(),
            causal_predecessor_ids: causal_predecessors
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
        })
    }

    /// Returns the Event identity.
    #[must_use]
    pub fn id(&self) -> EventId {
        EventId::new(self.0.id.clone()).expect("validated id")
    }
    /// Returns the sequence coordinate.
    #[must_use]
    pub fn delivery(&self, source: SourceIdentity) -> DeliverySlot {
        let sequence = self.0.sequence.as_ref().expect("validated sequence");
        DeliverySlot::new(
            source,
            SequenceEpoch::new(sequence.epoch),
            SequenceNumber::new(sequence.sequence),
        )
    }
    /// Returns the schema version.
    #[must_use]
    pub fn schema_version(&self) -> SchemaVersion {
        SchemaVersion::new(self.0.schema_version.clone()).expect("validated schema")
    }
    /// Returns the acknowledged Filter epoch.
    #[must_use]
    pub fn filter_epoch(&self) -> FilterEpoch {
        FilterEpoch::new(
            self.0
                .acknowledged_filter_epoch
                .expect("validated Filter epoch"),
        )
    }
    /// Returns the Event kind.
    #[must_use]
    pub fn kind(&self) -> EventKind {
        EventKind::new(self.0.kind.clone()).expect("validated kind")
    }
    /// Returns occurrence time.
    #[must_use]
    pub fn occurrence_time(&self) -> OccurrenceTime {
        OccurrenceTime(
            self.0
                .occurrence_time
                .clone()
                .expect("validated occurrence time"),
        )
    }
    /// Returns attributes in deterministic key order.
    #[must_use]
    pub fn attributes(&self) -> BTreeMap<AttributeKey, AttributeValue> {
        entries_from_wire(&self.0.attributes).expect("validated attributes")
    }
    /// Returns claimed source capabilities.
    #[must_use]
    pub fn claimed_capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::try_from_wire(&self.0.claimed_capabilities, "claimed_capabilities")
            .expect("validated capabilities")
    }
    /// Returns claimed sensitivity.
    #[must_use]
    pub fn sensitivity(&self) -> Sensitivity {
        Sensitivity::from_raw(self.0.claimed_sensitivity).expect("validated sensitivity")
    }
    /// Returns causal predecessor identities.
    #[must_use]
    pub fn causal_predecessors(&self) -> BTreeSet<EventId> {
        self.0
            .causal_predecessor_ids
            .iter()
            .map(|id| EventId::new(id.clone()).expect("validated predecessor"))
            .collect()
    }
}

impl TryFrom<raw::ProducerEvent> for ProducerEvent {
    type Error = EventError;

    fn try_from(mut value: raw::ProducerEvent) -> Result<Self, Self::Error> {
        EventId::new(value.id.clone())
            .map_err(|source| invalid_identifier("producer_event.id", source))?;
        SchemaVersion::new(value.schema_version.clone())
            .map_err(|source| invalid_identifier("producer_event.schema_version", source))?;
        EventKind::new(value.kind.clone())
            .map_err(|source| invalid_identifier("producer_event.kind", source))?;
        if value.acknowledged_filter_epoch.is_none() {
            return Err(EventError::MissingScalar(
                "producer_event.acknowledged_filter_epoch",
            ));
        }
        if value.sequence.is_none() {
            return Err(EventError::MissingField("producer_event.sequence"));
        }
        let occurrence_time = OccurrenceTime::try_from_wire(
            value
                .occurrence_time
                .take()
                .ok_or(EventError::MissingField("producer_event.occurrence_time"))?,
        )?;
        value.occurrence_time = Some(occurrence_time.0);
        value.attributes = canonical_entries(value.attributes)?;
        let capabilities =
            SourceCapabilities::try_from_wire(&value.claimed_capabilities, "claimed_capabilities")?;
        value.claimed_capabilities = capabilities.to_wire();
        Sensitivity::from_raw(value.claimed_sensitivity)?;
        let mut predecessors = BTreeSet::new();
        for predecessor in &value.causal_predecessor_ids {
            EventId::new(predecessor.clone())
                .map_err(|source| invalid_identifier("causal_predecessor_ids", source))?;
            if predecessor == &value.id {
                return Err(EventError::SelfCausalPredecessor);
            }
            if !predecessors.insert(predecessor) {
                return Err(EventError::DuplicateValue("causal_predecessor_ids"));
            }
        }
        value.causal_predecessor_ids = predecessors.into_iter().cloned().collect();
        Ok(Self(value))
    }
}

/// Transport-verified and deployment-configured source assurance.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceAssurance(pub(crate) raw::SourceAssurance);

impl SourceAssurance {
    /// Creates verified assurance owned by Sentinel's ingestion boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if source assurance is incomplete or invalid.
    pub fn new(
        source: SourceIdentity,
        tier: SourceTier,
        capabilities: SourceCapabilities,
        integrity: Integrity,
        authenticated_identity_evidence: Vec<u8>,
        integrity_evidence: Vec<u8>,
        topology_id: impl Into<String>,
    ) -> Result<Self, EventError> {
        Self::try_from(raw::SourceAssurance {
            source: Some(source.to_wire()),
            tier: tier.to_raw(),
            effective_capabilities: capabilities.to_wire(),
            integrity: integrity.to_raw(),
            authenticated_identity_evidence,
            integrity_evidence,
            topology_id: topology_id.into(),
        })
    }

    /// Returns the authenticated source identity.
    #[must_use]
    pub fn source(&self) -> SourceIdentity {
        source_from_wire(self.0.source.as_ref().expect("validated source"))
            .expect("validated source")
    }
    /// Returns configured source tier.
    #[must_use]
    pub fn tier(&self) -> SourceTier {
        SourceTier::from_raw(self.0.tier).expect("validated tier")
    }
    /// Returns effective verified capabilities.
    #[must_use]
    pub fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::try_from_wire(&self.0.effective_capabilities, "effective_capabilities")
            .expect("validated capabilities")
    }
    /// Returns verified integrity.
    #[must_use]
    pub fn integrity(&self) -> Integrity {
        Integrity::from_raw(self.0.integrity).expect("validated integrity")
    }
}

impl TryFrom<raw::SourceAssurance> for SourceAssurance {
    type Error = EventError;

    fn try_from(mut value: raw::SourceAssurance) -> Result<Self, Self::Error> {
        source_from_wire(
            value
                .source
                .as_ref()
                .ok_or(EventError::MissingField("assurance.source"))?,
        )?;
        SourceTier::from_raw(value.tier)?;
        let capabilities = SourceCapabilities::try_from_wire(
            &value.effective_capabilities,
            "effective_capabilities",
        )?;
        value.effective_capabilities = capabilities.to_wire();
        Integrity::from_raw(value.integrity)?;
        require_text(&value.topology_id, "assurance.topology_id")?;
        Ok(Self(value))
    }
}

/// Validated canonical point Event backed only by the generated wire message.
#[derive(Clone, Debug, PartialEq)]
pub struct Event(raw::Event);

impl Event {
    /// Composes producer evidence with receiver-owned assurance and observation.
    ///
    /// The data class is a required argument, not a default: ADR-0029 makes a
    /// corpus Event mistaken for Tenant data (or the reverse) a silent-wrong
    /// failure, so every producer of Events must name the class at the call
    /// site. `tenant_id` and `deployment_id` stay empty here either way —
    /// Sentinel-assigned scoping is a later, tenant-side step (ADR-0026 §1) and
    /// corpus Events never receive it.
    #[must_use]
    pub fn observe(
        producer: ProducerEvent,
        assurance: SourceAssurance,
        observation_time: ObservationTime,
        data_class: DataClass,
    ) -> Self {
        Self(raw::Event {
            producer_event: Some(producer.0),
            assurance: Some(assurance.0),
            observation_time: Some(observation_time.0),
            tenant_id: String::new(),
            deployment_id: String::new(),
            data_class: data_class.to_raw(),
        })
    }

    /// Decodes and validates an untrusted wire message.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed bytes or a decoded message that violates
    /// canonical Event invariants.
    pub fn decode(bytes: &[u8]) -> Result<Self, EventError> {
        let raw =
            raw::Event::decode(bytes).map_err(|error| EventError::Decode(error.to_string()))?;
        Self::try_from(raw)
    }

    /// Encodes the canonical Event in Protobuf wire format.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        self.0.encode_to_vec()
    }
    /// Returns producer evidence.
    #[must_use]
    pub fn producer(&self) -> ProducerEvent {
        ProducerEvent(self.0.producer_event.clone().expect("validated producer"))
    }
    /// Returns source assurance.
    #[must_use]
    pub fn assurance(&self) -> SourceAssurance {
        SourceAssurance(self.0.assurance.clone().expect("validated assurance"))
    }
    /// Returns receiver observation time.
    #[must_use]
    pub fn observation_time(&self) -> ObservationTime {
        ObservationTime(
            self.0
                .observation_time
                .clone()
                .expect("validated observation"),
        )
    }
    /// Returns Event identity.
    #[must_use]
    pub fn id(&self) -> EventId {
        self.producer().id()
    }
    /// Returns source identity.
    #[must_use]
    pub fn source(&self) -> SourceIdentity {
        self.assurance().source()
    }
    /// Returns source-qualified delivery slot.
    #[must_use]
    pub fn delivery(&self) -> DeliverySlot {
        self.producer().delivery(self.source())
    }
    /// Returns occurrence time.
    #[must_use]
    pub fn occurrence_time(&self) -> OccurrenceTime {
        self.producer().occurrence_time()
    }
    /// Returns effective capabilities, not producer claims.
    #[must_use]
    pub fn capabilities(&self) -> SourceCapabilities {
        self.assurance().capabilities()
    }
    /// Returns verified integrity.
    #[must_use]
    pub fn integrity(&self) -> Integrity {
        self.assurance().integrity()
    }
    /// Returns configured source tier.
    #[must_use]
    pub fn source_tier(&self) -> SourceTier {
        self.assurance().tier()
    }
    /// Returns the data class this Event belongs to (ADR-0029).
    #[must_use]
    pub fn data_class(&self) -> DataClass {
        DataClass::from_raw(self.0.data_class).expect("validated data class")
    }
    /// Returns causal predecessors.
    #[must_use]
    pub fn causal_predecessors(&self) -> BTreeSet<EventId> {
        self.producer().causal_predecessors()
    }

    fn has_same_producer_evidence(&self, other: &Self) -> bool {
        self.0.producer_event == other.0.producer_event && self.0.assurance == other.0.assurance
    }
}

impl TryFrom<raw::Event> for Event {
    type Error = EventError;

    fn try_from(mut value: raw::Event) -> Result<Self, Self::Error> {
        let producer = ProducerEvent::try_from(
            value
                .producer_event
                .take()
                .ok_or(EventError::MissingField("event.producer_event"))?,
        )?;
        let assurance = SourceAssurance::try_from(
            value
                .assurance
                .take()
                .ok_or(EventError::MissingField("event.assurance"))?,
        )?;
        let observation = ObservationTime::try_from_wire(
            value
                .observation_time
                .take()
                .ok_or(EventError::MissingField("event.observation_time"))?,
        )?;
        DataClass::from_raw(value.data_class)?;
        value.producer_event = Some(producer.0);
        value.assurance = Some(assurance.0);
        value.observation_time = Some(observation.0);
        Ok(Self(value))
    }
}

/// Compares Event occurrence intervals when both verified sources support it.
#[must_use]
pub fn compare_occurrence_times(left: &Event, right: &Event) -> ClockRelation {
    let left_bounded = left
        .capabilities()
        .contains(SourceCapability::BoundedClockUncertainty);
    let right_bounded = right
        .capabilities()
        .contains(SourceCapability::BoundedClockUncertainty);
    match (left_bounded, right_bounded) {
        (false, false) => {
            return ClockRelation::MissingBoundedClockUncertainty {
                missing: MissingClockCapability::Both,
            }
        }
        (false, true) => {
            return ClockRelation::MissingBoundedClockUncertainty {
                missing: MissingClockCapability::Left,
            }
        }
        (true, false) => {
            return ClockRelation::MissingBoundedClockUncertainty {
                missing: MissingClockCapability::Right,
            }
        }
        (true, true) => {}
    }
    compare_occurrence_intervals(&left.occurrence_time(), &right.occurrence_time())
}

/// Direct causal relationship visible from a pair of Events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CausalRelation {
    /// Left is an explicit predecessor of right.
    Before,
    /// Right is an explicit predecessor of left.
    After,
    /// No direct pairwise edge is present.
    ConcurrentOrIndirect,
    /// Each Event directly names the other.
    ContradictoryCycle,
}

/// Classifies directly visible causal edges.
#[must_use]
pub fn causal_relation(left: &Event, right: &Event) -> CausalRelation {
    let left_id = left.id();
    let right_id = right.id();
    let left_before_right = right.causal_predecessors().contains(&left_id);
    let right_before_left = left.causal_predecessors().contains(&right_id);
    match (left_before_right, right_before_left) {
        (true, false) => CausalRelation::Before,
        (false, true) => CausalRelation::After,
        (false, false) => CausalRelation::ConcurrentOrIndirect,
        (true, true) => CausalRelation::ContradictoryCycle,
    }
}

/// Relationship between two delivered Event identities and slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DeliveryIdentityRelation {
    /// Different identity and slot.
    Distinct,
    /// Same identity, slot, and producer evidence.
    Duplicate,
    /// Same identity and slot with conflicting evidence.
    EvidenceConflict,
    /// Different identity claims the same slot.
    SlotConflict,
    /// Same identity claims a different slot.
    IdentityConflict,
}

/// Detects duplicate delivery and identity/slot conflicts.
#[must_use]
pub fn classify_delivery_identity(left: &Event, right: &Event) -> DeliveryIdentityRelation {
    let same_slot =
        classify_delivery(&left.delivery(), &right.delivery()) == DeliveryRelation::SameSlot;
    let same_id = left.id() == right.id();
    match (same_slot, same_id) {
        (true, true) if left.has_same_producer_evidence(right) => {
            DeliveryIdentityRelation::Duplicate
        }
        (true, true) => DeliveryIdentityRelation::EvidenceConflict,
        (true, false) => DeliveryIdentityRelation::SlotConflict,
        (false, true) => DeliveryIdentityRelation::IdentityConflict,
        (false, false) => DeliveryIdentityRelation::Distinct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int128_words_roundtrip_extremes() {
        for value in [i128::MIN, -1, 0, 1, i128::MAX] {
            assert_eq!(int128_from_wire(&int128_to_wire(value)), value);
        }
    }

    /// Checks [`ObservationTime::to_wire`] against hand-written expected words.
    ///
    /// Deliberately **not** written against `int128_to_wire`. [`ObservationTime::new`]
    /// *is* `raw::ObservationTime { sentinel_clock_id: .., nanoseconds:
    /// Some(int128_to_wire(n)) }`, so a test that rebuilds that expression and compares
    /// it against `to_wire()` evaluates the same code on both sides and checks nothing.
    /// The high/low words below are written out by hand at the `i128` extremes -- where
    /// a sign or a word order got the wrong way round is visible -- so this is an oracle
    /// for the split rather than a restatement of it.
    ///
    /// It is also a deliberate **compile tripwire**, and that is half of what it is for.
    /// The expected value is a struct literal, so a field added to
    /// `raw::ObservationTime` in the proto stops this test compiling with `E0063` here,
    /// in the crate that owns the type, rather than being answered with a `: 0` in a
    /// downstream re-derivation that would then drop the field on every Verdict. Do not
    /// "simplify" this to `assert_eq!(time.to_wire(), time.0.clone())`: that deletes the
    /// oracle and the tripwire together, and nothing else in the workspace replaces
    /// either.
    #[test]
    fn to_wire_splits_the_nanoseconds_into_the_words_the_wire_names() {
        let clock = SentinelClockId::new("sentinel-a").expect("a non-empty clock id is valid");

        for (nanoseconds, high, low) in [
            (i128::MIN, i64::MIN, 0_u64),
            (-1_i128, -1_i64, u64::MAX),
            (0_i128, 0_i64, 0_u64),
            (1_i128, 0_i64, 1_u64),
            (i128::MAX, i64::MAX, u64::MAX),
        ] {
            assert_eq!(
                ObservationTime::new(clock.clone(), nanoseconds).to_wire(),
                raw::ObservationTime {
                    sentinel_clock_id: "sentinel-a".to_owned(),
                    nanoseconds: Some(raw::Int128 { high, low }),
                },
                "the wire words for {nanoseconds}"
            );
        }
    }

    /// Pins that `to_wire` returns the accepted message rather than a rebuild of it.
    ///
    /// `try_from_wire` is fed a message whose `sentinel_clock_id` carries surrounding
    /// whitespace -- [`SentinelClockId::new`] stores its input verbatim and rejects only
    /// empty and whitespace-only, so this is an accepted value that a re-derivation
    /// through the accessors happens to reproduce today -- together with `nanoseconds`
    /// at `i128::MIN`, the extreme where a naive low-word cast breaks. The assertion is
    /// identity against an **externally supplied** message rather than against the
    /// encoder's own output, so it is not a tautology.
    ///
    /// A hand-back and a re-derivation both satisfy it today, and that is the point:
    /// this is the pin that goes red the day [`SentinelClockId`] gains normalization, at
    /// which moment a re-derivation starts returning a different message while
    /// `to_wire_splits_the_nanoseconds_into_the_words_the_wire_names` stays green. It
    /// stands in for the compile-checked hand-back guarantee that `to_wire(&self)`
    /// declines to give and that `as_wire(&self) -> &_` would have given; the trade is
    /// recorded on `to_wire` in `wire_helpers.rs`.
    #[test]
    fn to_wire_returns_the_message_try_from_wire_accepted_not_a_re_derivation() {
        let accepted = raw::ObservationTime {
            sentinel_clock_id: "  sentinel-a  ".to_owned(),
            nanoseconds: Some(raw::Int128 {
                high: i64::MIN,
                low: 0,
            }),
        };

        let observation = ObservationTime::try_from_wire(accepted.clone())
            .expect("a padded clock id is accepted verbatim");

        assert_eq!(observation.to_wire(), accepted);
        assert_eq!(observation.sentinel_nanoseconds(), i128::MIN);
    }

    /// Pins the *absolute* binding of each identifier to its proto field number.
    ///
    /// The round-trip test below is a *relative* property: it pins only that the two
    /// conversions are mutual inverses, so a swap applied consistently to both
    /// directions would still pass it. This test is what fixes which identifier lands in
    /// which field, independent of how the decoder reads it back.
    #[test]
    fn source_to_wire_maps_each_identifier_to_its_own_field() {
        let source = SourceIdentity::new(
            MonitoredApplicationId::new("app").expect("valid identifier"),
            SourceId::new("gateway").expect("valid identifier"),
        );
        let wire = source.to_wire();
        assert_eq!(wire.monitored_application_id, "app");
        assert_eq!(wire.source_id, "gateway");
    }

    /// Pins that `to_wire` and `source_from_wire` are mutual inverses.
    ///
    /// A relative property, and not sufficient on its own for a wire encoder -- see the
    /// field-mapping test above. It is what lets `source_from_wire` stay private: the
    /// only ingress is the crate's whole-message boundaries, and this asserts that
    /// anything this crate emits re-validates into the value it came from. Distinct
    /// values throughout, because equal ones would make a swap invisible.
    #[test]
    fn source_wire_round_trips_through_validation() {
        let source = SourceIdentity::new(
            MonitoredApplicationId::new("app").expect("valid identifier"),
            SourceId::new("gateway").expect("valid identifier"),
        );
        assert_eq!(
            source_from_wire(&source.to_wire()).expect("encoded identity re-validates"),
            source
        );
    }

    /// Round-trips every variant of both `#[non_exhaustive]` source enums.
    ///
    /// `sentinel-core` carried copies of these two encoders that had to end in a
    /// wildcard, because rustc requires one when matching a foreign
    /// `#[non_exhaustive]` enum. The exhaustive match now lives only here, in the crate
    /// that declares the enums and where adding a variant is therefore a compile error
    /// rather than a silent fallback. These lists are extended alongside the enums.
    #[test]
    fn source_enums_round_trip_every_variant() {
        for capability in [
            SourceCapability::ObserveBeforeEffect,
            SourceCapability::CausalEdges,
            SourceCapability::CompletenessSignals,
            SourceCapability::BoundedClockUncertainty,
        ] {
            assert_eq!(
                SourceCapability::from_raw(capability.to_raw(), "capability")
                    .expect("encoded capability re-validates"),
                capability
            );
        }

        for tier in [SourceTier::Anchor, SourceTier::Contributing] {
            assert_eq!(
                SourceTier::from_raw(tier.to_raw()).expect("encoded tier re-validates"),
                tier
            );
        }
    }

    #[test]
    fn non_finite_attribute_double_is_rejected() {
        assert_eq!(
            AttributeDouble::try_from(f64::NAN),
            Err(EventError::NonFiniteAttributeDouble)
        );
    }
}
