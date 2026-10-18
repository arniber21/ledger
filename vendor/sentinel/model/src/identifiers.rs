//! Validated identifiers and source delivery coordinates.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Error returned when an identifier is empty or contains only whitespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentifierError;

impl Display for IdentifierError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("identifier must not be empty or whitespace")
    }
}

impl Error for IdentifierError {}

macro_rules! string_identifier {
    ($(#[$metadata:meta])* $name:ident) => {
        $(#[$metadata])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Parses and validates an identifier.
            ///
            /// # Errors
            ///
            /// Returns [`IdentifierError`] when `value` is empty or whitespace.
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdentifierError);
                }
                Ok(Self(value))
            }

            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdentifierError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

string_identifier!(
    /// Stable identity of one Event.
    EventId
);
string_identifier!(
    /// Stable identity of one Monitored Application.
    MonitoredApplicationId
);
string_identifier!(
    /// Stable identity of one Probe or Adapter source.
    SourceId
);
string_identifier!(
    /// Identity of a producer clock domain.
    ClockDomainId
);
string_identifier!(
    /// Identity of a Sentinel-local observation clock.
    SentinelClockId
);
string_identifier!(
    /// Version of the Event schema used by a producer.
    SchemaVersion
);
string_identifier!(
    /// Kind of point transition represented by an Event.
    EventKind
);
string_identifier!(
    /// Name of an Event attribute.
    ///
    /// An Event attribute name is a model identifier, not a Core secret. The
    /// emptiness rule is the same as [`EventId`]: `trim().is_empty()` is
    /// refused, and stored text is **not** trimmed, so lookup against Event
    /// maps stays exact.
    AttributeKey
);

/// Identity of an addressable Event source within a Monitored Application.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceIdentity {
    monitored_application: MonitoredApplicationId,
    source: SourceId,
}

impl SourceIdentity {
    /// Creates a source identity.
    #[must_use]
    pub const fn new(monitored_application: MonitoredApplicationId, source: SourceId) -> Self {
        Self {
            monitored_application,
            source,
        }
    }

    /// Returns the Monitored Application identity.
    #[must_use]
    pub const fn monitored_application(&self) -> &MonitoredApplicationId {
        &self.monitored_application
    }

    /// Returns the Probe or Adapter source identity.
    #[must_use]
    pub const fn source(&self) -> &SourceId {
        &self.source
    }
}

/// Epoch of a source sequence, incremented whenever numbering restarts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SequenceEpoch(u64);

impl SequenceEpoch {
    /// Creates a sequence epoch.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying epoch number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Monotonically increasing sequence number within one source epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    /// Creates a sequence number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying sequence number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Version of the Event Filter acknowledged by a source.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FilterEpoch(u64);

impl FilterEpoch {
    /// Creates a Filter epoch.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying Filter epoch.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Source-qualified delivery coordinate for one Event.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeliverySlot {
    source: SourceIdentity,
    epoch: SequenceEpoch,
    sequence: SequenceNumber,
}

impl DeliverySlot {
    /// Creates a source-qualified delivery slot.
    #[must_use]
    pub const fn new(
        source: SourceIdentity,
        epoch: SequenceEpoch,
        sequence: SequenceNumber,
    ) -> Self {
        Self {
            source,
            epoch,
            sequence,
        }
    }

    /// Returns the source that owns the sequence.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.source
    }

    /// Returns the sequence epoch.
    #[must_use]
    pub const fn epoch(&self) -> SequenceEpoch {
        self.epoch
    }

    /// Returns the sequence number.
    #[must_use]
    pub const fn sequence(&self) -> SequenceNumber {
        self.sequence
    }
}

/// Relationship between two source delivery coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DeliveryRelation {
    /// The coordinates name the same source, epoch, and sequence number.
    SameSlot,
    /// `left` immediately precedes `right` in one source epoch.
    ConsecutiveBefore,
    /// `left` precedes `right`, with the stated count of missing sequence numbers.
    GapBefore {
        /// Count of sequence numbers between the coordinates.
        missing: u64,
    },
    /// `left` immediately follows `right` in one source epoch.
    ConsecutiveAfter,
    /// `left` follows `right`, with the stated count of missing sequence numbers.
    GapAfter {
        /// Count of sequence numbers between the coordinates.
        missing: u64,
    },
    /// The coordinates belong to different sources.
    DifferentSource,
    /// The coordinates belong to different epochs of the same source.
    DifferentEpoch,
}

/// Classifies two source-qualified delivery slots without consulting external state.
#[must_use]
pub fn classify_delivery(left: &DeliverySlot, right: &DeliverySlot) -> DeliveryRelation {
    if left.source != right.source {
        return DeliveryRelation::DifferentSource;
    }
    if left.epoch != right.epoch {
        return DeliveryRelation::DifferentEpoch;
    }

    match left.sequence.0.cmp(&right.sequence.0) {
        std::cmp::Ordering::Equal => DeliveryRelation::SameSlot,
        std::cmp::Ordering::Less => {
            let distance = right.sequence.0.saturating_sub(left.sequence.0);
            if distance == 1 {
                DeliveryRelation::ConsecutiveBefore
            } else {
                DeliveryRelation::GapBefore {
                    missing: distance.saturating_sub(1),
                }
            }
        }
        std::cmp::Ordering::Greater => {
            let distance = left.sequence.0.saturating_sub(right.sequence.0);
            if distance == 1 {
                DeliveryRelation::ConsecutiveAfter
            } else {
                DeliveryRelation::GapAfter {
                    missing: distance.saturating_sub(1),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_empty_and_whitespace_values() {
        assert_eq!(EventId::new(""), Err(IdentifierError));
        assert_eq!(SourceId::new("  \n"), Err(IdentifierError));
        assert_eq!(AttributeKey::new(""), Err(IdentifierError));
        assert_eq!(AttributeKey::new("   "), Err(IdentifierError));
    }

    #[test]
    fn identifiers_store_untrimmed_non_blank_text() {
        assert_eq!(EventId::new("  foo").expect("non-blank").as_str(), "  foo");
        assert_eq!(
            AttributeKey::new("  foo").expect("non-blank").as_str(),
            "  foo"
        );
    }
}
