//! What an Adapter hands the Verification Core: Events plus per-source evidence.
//!
//! This is the validated-domain side of the ingest boundary. Everything here is
//! built from already-validated [`sentinel_model`] types, so the Core never
//! touches untrusted storage and never reads a clock (ADR-0012): anything
//! time shaped arrives inside this structure.
//!
//! [`SourceEvidence`] and [`SequenceGap`] are **re-exports**: issue #12 moved
//! them into `sentinel_model`, because they are pure model-built validated
//! types and because the resumable Core takes per-source evidence as an input
//! rather than reading it out of a batch. [`ObservedEvidence`] stayed here: it
//! is an Adapter batch handoff shape, not a domain type.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{Event, SchemaVersion, SourceIdentity};

pub use sentinel_model::{SequenceGap, SourceEvidence};

/// Canonical Events and per-source evidence produced by one Adapter run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ObservedEvidence {
    schema_version: SchemaVersion,
    events: Vec<Event>,
    sources: BTreeMap<SourceIdentity, SourceEvidence>,
}

impl ObservedEvidence {
    /// Composes adapted Events with the per-source evidence that qualifies them.
    #[must_use]
    pub fn new(
        schema_version: SchemaVersion,
        events: Vec<Event>,
        sources: BTreeMap<SourceIdentity, SourceEvidence>,
    ) -> Self {
        Self {
            schema_version,
            events,
            sources,
        }
    }

    /// Returns the producer schema version every Event carries.
    #[must_use]
    pub const fn schema_version(&self) -> &SchemaVersion {
        &self.schema_version
    }

    /// Returns the adapted Events in Adapter emission order.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Returns per-source evidence, keyed deterministically.
    #[must_use]
    pub const fn sources(&self) -> &BTreeMap<SourceIdentity, SourceEvidence> {
        &self.sources
    }

    /// Returns evidence for one source, when it supplied any.
    #[must_use]
    pub fn source(&self, source: &SourceIdentity) -> Option<&SourceEvidence> {
        self.sources.get(source)
    }

    /// Returns every source that supplied at least one Event.
    #[must_use]
    pub fn observed_sources(&self) -> BTreeSet<SourceIdentity> {
        self.events.iter().map(Event::source).collect()
    }

    /// Replaces the Event sequence, preserving the per-source evidence.
    ///
    /// This exists so a test can feed the same evidence in a different arrival
    /// order; per ADR-0020 the Verdict must not depend on that order.
    #[must_use]
    pub fn with_events(&self, events: Vec<Event>) -> Self {
        Self {
            schema_version: self.schema_version.clone(),
            events,
            sources: self.sources.clone(),
        }
    }
}
