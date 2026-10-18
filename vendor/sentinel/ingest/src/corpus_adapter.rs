//! The corpus Adapter: `trace-corpus-v1` records to canonical Events.
//!
//! # The class boundary
//!
//! Every Event this Adapter emits is tagged [`DataClass::Corpus`] (ADR-0029).
//! Corpus data is donated, sanitized evidence under a DUA; it never receives
//! `tenant_id`/`deployment_id` scoping, never flows into the monitored-Tenant
//! path, and no Verdict ever rests on it. The SagaShop Adapter is the other
//! class, and [`sentinel_model::Event::observe`] accepts no default between
//! them — the class is named here, at the only boundary that can know it.
//!
//! # Two Events per record, observed at span end
//!
//! ADR-0015 makes Events points on transitions and a Span a derived view, so
//! each corpus record becomes a `requested` Event at `start` and a
//! `completed` Event at `end`. Both are observed when the record is read — the
//! corpus is a recorded artifact, and a recorded span is observable only once
//! it has ended. A record with `end: null` emitted **only** its `requested`
//! Event, and the even delivery slot it would have occupied stays empty: a
//! truncated record is a gap in the evidence, which the Core's G2 gate
//! downgrades honestly, never a silently skipped sequence number.
//!
//! # Causality is copied, never derived
//!
//! trace-grab ADR-0010 is normative: the corpus's causal content is
//! `parent_id` plus `links[]`, and nothing else. This Adapter translates
//! exactly that structure and derives nothing on top of it:
//!
//! 1. `r.requested -> r.completed`, for every record — the ADR-0015 split's
//!    own edge, part of the split rather than a derivation from vendor data.
//! 2. `parent.requested -> child.requested`, from `parent_id`. An unresolved
//!    `parent_id` is a **dangling causal edge**: evidence to reason about, not
//!    a parse error (ADR-0019, same posture as [`crate::capture`]).
//! 3. `child.completed -> parent.completed`, **only** under
//!    [`CorpusAdapter::with_synchronous_call_graph`] — see below.
//! 4. Link edges, from `links[]` — see [`Self::adapt`]'s link reading.
//!
//! There is deliberately **no transition logic**: edge class 4 of the SagaShop
//! Adapter is a port of `shared/monitor.ts` and rests on SagaShop's order-state
//! machine. A generic corpus has no state machine, and encoding one here would
//! emit edges no vendor licensed.
//!
//! # The synchronous-call-graph premise is opt-in and never inherited
//!
//! `parent_id` expresses nesting, not ordering of completions: a fire-and-forget
//! child outlives its parent, so `child.completed -> parent.completed` is false
//! for asynchronous systems and true only under a synchronous-await contract.
//! [`CorpusAdapter::with_synchronous_call_graph`] exists for the one corpus that
//! has such a contract to name, exactly as
//! [`crate::sagashop::SagaShopAdapter::with_synchronous_call_graph`] does. The
//! default asserts nothing.
//!
//! # Links: the strongest edges a link can soundly license
//!
//! trace-grab's `Link` is the OTLP link object, and OTLP leaves a link's
//! meaning loose. This Adapter reads a link on record `r` targeting record `t`
//! as: **`t`'s span entirely precedes `r`'s span**, and emits the three edges
//! that claim licenses — `t.requested -> r.requested`,
//! `t.completed -> r.requested`, and `t.completed -> r.completed`. All three
//! are sound under the reading, all three are what a link-authored corpus
//! (issue #78's converter, which encodes derived SagaShop edges as links)
//! needs, and a link pointing at a record absent from the bundle is a dangling
//! edge like an unresolved parent. Anything weaker loses causal content the
//! corpus explicitly carried; anything stronger would order events the link
//! does not relate.
//!
//! # Source identity, tiers and clocks
//!
//! A corpus record names no system. The Adapter takes the application half of
//! [`SourceIdentity`] from the record's `attributes["service.name"]` — the
//! OTel resource convention SCHEMA.md's "OTel-style" attributes anticipate —
//! and falls back to `source.vendor` when absent, so an application is either
//! a named service or the parser that produced the record, never a guess.
//!
//! Tiers default to [`SourceTier::Contributing`]: a donated, sanitized corpus
//! has no argument entitling any source to anchor status (ADR-0022), and
//! [`CorpusAdapter::with_source_tiers`] exists so a corpus whose deployment
//! *does* have an assurance argument can state it. Clock domains are per
//! application, so cross-application pairs are never comparable; the
//! occurrence-time uncertainty is the timestamp's own parsed precision, and
//! `BoundedClockUncertainty` is withheld regardless — an honestly coarse
//! clock, inert by construction (ADR-0018).
//!
//! # Sequences
//!
//! The corpus carries no producer sequence numbers; its only total order is
//! file order. Each application's delivery sequence is therefore its records'
//! occurrence order in the file, contiguous from one by construction, and each
//! record occupies the even/odd pair `2k-1`/`2k` within its application —
//! mirroring the SagaShop Adapter's per-span pair.

#![allow(clippy::doc_markdown)] // Product names (trace-grab, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::{
    AttributeDouble, AttributeKey, AttributeValue, ClockDomainId, DataClass, Event, EventId,
    EventKind, FilterEpoch, Integrity, MonitoredApplicationId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId,
    SequenceEpoch, SequenceNumber, SourceActivity, SourceAssurance, SourceCapabilities,
    SourceCapability, SourceCompletenessSignal, SourceId, SourceIdentity, SourceTier,
};

use crate::corpus::{Corpus, CorpusRecord, CorpusValue};
use crate::evidence::{ObservedEvidence, SequenceGap, SourceEvidence};

/// The constant `SourceId` every adapted Event carries.
pub const SOURCE_ID: &str = "trace-corpus";
/// The Sentinel-local clock identity used when reading a recorded corpus.
pub const SENTINEL_CLOCK_ID: &str = "sentinel-replay";
/// The producer schema version every adapted Event carries.
pub const SCHEMA_VERSION: &str = "trace-corpus-v1";
/// The single Filter epoch this slice uses. No Filter is compiled or pushed.
pub const FILTER_EPOCH: u64 = 0;
/// The single sequence epoch this slice uses. No numbering restart is modelled.
pub const SEQUENCE_EPOCH: u64 = 0;

/// The `requested` phase suffix.
pub const PHASE_REQUESTED: &str = "requested";
/// The `completed` phase suffix.
pub const PHASE_COMPLETED: &str = "completed";

/// The attribute key naming the emitting service, per the OTel resource
/// convention SCHEMA.md's "OTel-style" attributes anticipate.
pub const SERVICE_NAME_ATTRIBUTE: &str = "service.name";

/// The reserved attribute namespace this Adapter's structural context lives
/// under, mirroring the `otel.*` namespace the SagaShop Adapter uses.
const CORPUS_NAMESPACE: &str = "corpus.";

/// Failure adapting a structurally valid corpus into canonical Events.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AdapterError {
    /// Two records claim the same `id`.
    DuplicateRecordId {
        /// The reused identifier.
        id: String,
        /// One-based line number of the second record carrying it.
        line: usize,
    },
    /// A canonical Event or completeness signal failed model validation.
    InvalidEvidence(String),
}

impl Display for AdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRecordId { id, line } => {
                write!(formatter, "corpus line {line} reuses record id {id}")
            }
            Self::InvalidEvidence(detail) => {
                write!(formatter, "adapted evidence is not valid: {detail}")
            }
        }
    }
}

impl Error for AdapterError {}

fn invalid_evidence(error: impl Display) -> AdapterError {
    AdapterError::InvalidEvidence(error.to_string())
}

/// The capabilities a corpus actually supports.
///
/// `CausalEdges` and `CompletenessSignals` only. `BoundedClockUncertainty`
/// and `ObserveBeforeEffect` are withheld: the corpus's clocks are vendor wall
/// clocks of unknown relationship to each other, and its records exist only
/// after the span they describe has ended — ADR-0002/ADR-0015's "Adapters
/// differ in capability ... must be surfaced, not silently degraded", applied
/// rather than quoted.
#[must_use]
pub fn effective_capabilities() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

/// Builds the Event identity for one phase of one record.
///
/// Record ids are corpus-unique (enforced by [`CorpusAdapter::adapt`]), so the
/// record id alone identifies the pair; the trace id is deliberately not part
/// of the identity.
#[must_use]
pub fn event_id(record_id: &str, phase: &str) -> String {
    format!("{record_id}:{phase}")
}

fn attribute_key(key: impl Into<String>) -> Result<AttributeKey, AdapterError> {
    AttributeKey::new(key).map_err(invalid_evidence)
}

fn attribute_value(value: &CorpusValue) -> Result<Option<AttributeValue>, AdapterError> {
    match value {
        // A `null` attribute carries no evidence; dropping it is honest, and
        // the canonical attribute model has no null to invent instead.
        CorpusValue::Null => Ok(None),
        CorpusValue::Bool(value) => Ok(Some(AttributeValue::Bool(*value))),
        CorpusValue::String(value) => Ok(Some(AttributeValue::String(value.clone()))),
        CorpusValue::Number(value) => Ok(Some(number_attribute(*value))),
        CorpusValue::Array(values) => {
            let mut items = Vec::with_capacity(values.len());
            for value in values {
                if let Some(value) = attribute_value(value)? {
                    items.push(value);
                }
            }
            Ok(Some(AttributeValue::Array(items)))
        }
        CorpusValue::Object(values) => {
            let mut entries = BTreeMap::new();
            for (key, value) in values {
                if let Some(value) = attribute_value(value)? {
                    entries.insert(attribute_key(key.clone())?, value);
                }
            }
            Ok(Some(AttributeValue::KeyValueList(entries)))
        }
    }
}

fn number_attribute(value: f64) -> AttributeValue {
    #[allow(clippy::cast_possible_truncation)]
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        AttributeValue::Integer(value as i64)
    } else {
        AttributeValue::Double(
            AttributeDouble::try_from(value).expect("corpus numbers are validated finite"),
        )
    }
}

/// The scalar structural context: identifiers, name, kind, status and vendor.
fn scalar_structural_attributes(
    record: &CorpusRecord,
    line: usize,
    attributes: &mut BTreeMap<AttributeKey, AttributeValue>,
) -> Result<(), AdapterError> {
    let mut insert = |key: String, value: AttributeValue| -> Result<(), AdapterError> {
        attributes.insert(attribute_key(key)?, value);
        Ok(())
    };
    insert(
        format!("{CORPUS_NAMESPACE}record.id"),
        AttributeValue::String(record.id.clone()),
    )?;
    insert(
        format!("{CORPUS_NAMESPACE}trace.id"),
        AttributeValue::String(record.trace_id.clone()),
    )?;
    if let Some(parent) = &record.parent_id {
        insert(
            format!("{CORPUS_NAMESPACE}parent.id"),
            AttributeValue::String(parent.clone()),
        )?;
    }
    insert(
        format!("{CORPUS_NAMESPACE}name"),
        AttributeValue::String(record.name.clone()),
    )?;
    insert(
        format!("{CORPUS_NAMESPACE}kind"),
        AttributeValue::String(record.kind.clone()),
    )?;
    insert(
        format!("{CORPUS_NAMESPACE}status"),
        AttributeValue::String(record.status.to_str().to_owned()),
    )?;
    if let Some(error) = &record.error {
        insert(
            format!("{CORPUS_NAMESPACE}error.kind"),
            AttributeValue::String(error.kind.clone()),
        )?;
    }
    insert(
        format!("{CORPUS_NAMESPACE}source.vendor"),
        AttributeValue::String(record.vendor.clone()),
    )?;
    insert(
        format!("{CORPUS_NAMESPACE}record.line"),
        AttributeValue::Integer(i64::try_from(line).expect("line numbers fit in i64")),
    )?;
    insert(
        format!("{CORPUS_NAMESPACE}end.recorded"),
        AttributeValue::Bool(record.end.is_some()),
    )?;
    Ok(())
}

/// The structural context every adapted Event carries alongside the record's
/// own attributes: what the corpus record *is*, as opposed to what it *says*.
///
/// The `corpus.*` namespace is reserved by this Adapter the way `otel.*` is
/// reserved by the SagaShop one; a record attribute colliding with it is
/// overwritten, deliberately and loudly, because the Adapter's own context is
/// the more trustworthy claim about the record's structure.
fn structural_attributes(
    record: &CorpusRecord,
    line: usize,
) -> Result<BTreeMap<AttributeKey, AttributeValue>, AdapterError> {
    let mut attributes = BTreeMap::new();
    scalar_structural_attributes(record, line, &mut attributes)?;
    let mut insert = |key: String, value: AttributeValue| -> Result<(), AdapterError> {
        attributes.insert(attribute_key(key)?, value);
        Ok(())
    };
    if let Some(value) = attribute_value(&record.inputs)? {
        insert(format!("{CORPUS_NAMESPACE}inputs"), value)?;
    }
    if let Some(value) = attribute_value(&record.outputs)? {
        insert(format!("{CORPUS_NAMESPACE}outputs"), value)?;
    }
    if let Some(value) = attribute_value(&CorpusValue::Object(record.unmapped.clone()))? {
        insert(format!("{CORPUS_NAMESPACE}unmapped"), value)?;
    }
    if !record.labels.is_empty() {
        let mut labels = Vec::with_capacity(record.labels.len());
        for label in &record.labels {
            let Some(value) = attribute_value(&label.value)? else {
                continue;
            };
            let mut entry = BTreeMap::new();
            entry.insert(
                attribute_key("key")?,
                AttributeValue::String(label.key.clone()),
            );
            entry.insert(attribute_key("value")?, value);
            if let Some(comment) = &label.comment {
                entry.insert(
                    attribute_key("comment")?,
                    AttributeValue::String(comment.clone()),
                );
            }
            labels.push(AttributeValue::KeyValueList(entry));
        }
        insert(
            format!("{CORPUS_NAMESPACE}labels"),
            AttributeValue::Array(labels),
        )?;
    }
    if !record.links.is_empty() {
        let mut links = Vec::with_capacity(record.links.len());
        for link in &record.links {
            let mut entry = BTreeMap::new();
            entry.insert(
                attribute_key("trace_id")?,
                AttributeValue::String(link.trace_id.clone()),
            );
            entry.insert(
                attribute_key("span_id")?,
                AttributeValue::String(link.span_id.clone()),
            );
            let mut link_attributes = BTreeMap::new();
            for (key, value) in &link.attributes {
                if let Some(value) = attribute_value(value)? {
                    link_attributes.insert(attribute_key(key.clone())?, value);
                }
            }
            entry.insert(
                attribute_key("attributes")?,
                AttributeValue::KeyValueList(link_attributes),
            );
            links.push(AttributeValue::KeyValueList(entry));
        }
        insert(
            format!("{CORPUS_NAMESPACE}links"),
            AttributeValue::Array(links),
        )?;
    }
    Ok(attributes)
}

/// The record's own attributes, carried under their own keys.
///
/// These keys are the vocabulary the corpus's policy and any analysis over it
/// speak: dotted paths over vendor-shaped structure (trace-grab ADR-0010).
fn record_attributes(
    record: &CorpusRecord,
) -> Result<BTreeMap<AttributeKey, AttributeValue>, AdapterError> {
    let mut attributes = BTreeMap::new();
    for (key, value) in &record.attributes {
        if let Some(value) = attribute_value(value)? {
            attributes.insert(attribute_key(key.clone())?, value);
        }
    }
    Ok(attributes)
}

/// Adapts `trace-corpus-v1` bundles into canonical, corpus-class Events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CorpusAdapter {
    synchronous_call_graph: bool,
    source_tiers: BTreeMap<String, SourceTier>,
}

impl Default for CorpusAdapter {
    /// The Adapter that asserts nothing about the corpus beyond its structure.
    fn default() -> Self {
        Self::new()
    }
}

impl CorpusAdapter {
    /// Builds an Adapter that asserts nothing about the corpus it adapts.
    ///
    /// No synchronous-call premise (edge class 3 is withheld), and every
    /// source is Contributing-tier: a donated corpus carries no argument
    /// entitling any source to anchor status (ADR-0022), and an unrecognised
    /// source defaults to Contributing for the same reason.
    #[must_use]
    pub fn new() -> Self {
        Self {
            synchronous_call_graph: false,
            source_tiers: BTreeMap::new(),
        }
    }

    /// Returns an Adapter that emits `child.completed -> parent.completed`.
    ///
    /// **This method asserts a premise about the system the corpus records:
    /// every parent/child record pair is a synchronous awaited call, so the
    /// child completes before the parent does.** A corpus produced from a
    /// system with a synchronous route contract may license it; a generic
    /// corpus does not, which is why the premise is named here at the call
    /// site rather than defaulted. See [`crate::sagashop::SagaShopAdapter`]'s
    /// constructor for the same argument in the SagaShop path.
    #[must_use]
    pub fn with_synchronous_call_graph(mut self) -> Self {
        self.synchronous_call_graph = true;
        self
    }

    /// Returns whether edge class 3 is emitted.
    #[must_use]
    pub const fn emits_synchronous_call_edges(&self) -> bool {
        self.synchronous_call_graph
    }

    /// Supplies the assurance tiers for named applications (ADR-0022).
    ///
    /// Applications absent from the map stay Contributing. This exists because
    /// tier placement is a property of the deployment a corpus came from, and
    /// a corpus adapted for evaluation against anchor-requiring Specifications
    /// has to state that placement rather than inherit a blanket one.
    #[must_use]
    pub fn with_source_tiers(
        mut self,
        tiers: impl IntoIterator<Item = (String, SourceTier)>,
    ) -> Self {
        self.source_tiers = tiers.into_iter().collect();
        self
    }

    /// Returns the tier for one application.
    #[must_use]
    fn tier_for(&self, application: &str) -> SourceTier {
        self.source_tiers
            .get(application)
            .copied()
            .unwrap_or(SourceTier::Contributing)
    }

    /// Adapts one corpus into canonical, corpus-class Events and per-source
    /// evidence.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when two records claim the same id, or when
    /// adapted evidence fails `sentinel_model` validation.
    ///
    /// # Panics
    ///
    /// Panics only if a compile-time constant identity in this module is empty,
    /// which cannot happen.
    pub fn adapt(&self, corpus: &Corpus) -> Result<ObservedEvidence, AdapterError> {
        // Bundle-level structure the decoder cannot check: id uniqueness is a
        // property of the whole bundle, and a duplicate would produce two
        // Events with one identity — an IdentityConflict the Core would have
        // to diagnose from wreckage rather than the Adapter from a line number.
        let mut seen_ids: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, record) in corpus.records.iter().enumerate() {
            let line = index + 1;
            if seen_ids.contains_key(record.id.as_str()) {
                return Err(AdapterError::DuplicateRecordId {
                    id: record.id.clone(),
                    line,
                });
            }
            seen_ids.insert(record.id.as_str(), line);
        }

        // Per-application occurrence order: the corpus's only total order.
        // `ordinals[i]` is record i's one-based ordinal among its own
        // application's records, so delivery sequences are contiguous from one
        // by construction.
        let mut occurrence: BTreeMap<&str, u64> = BTreeMap::new();
        let mut ordinals = Vec::with_capacity(corpus.records.len());
        for record in &corpus.records {
            let next = occurrence.entry(application_of(record)).or_insert(0);
            *next += 1;
            ordinals.push(*next);
        }

        let edges = self.causal_edges(corpus);
        let schema_version = SchemaVersion::new(SCHEMA_VERSION).expect("constant schema version");
        let mut events = Vec::with_capacity(corpus.records.len() * 2);
        for (index, record) in corpus.records.iter().enumerate() {
            let line = index + 1;
            let application = application_of(record);
            let ordinal = ordinals[index];

            let assurance = self.source_assurance(application)?;
            let mut attributes = record_attributes(record)?;
            attributes.extend(structural_attributes(record, line)?);

            let requested_id = event_id(&record.id, PHASE_REQUESTED);
            let completed_id = event_id(&record.id, PHASE_COMPLETED);
            let no_predecessors = BTreeSet::new();

            events.push(Event::observe(
                Self::producer(
                    &requested_id,
                    ordinal * 2 - 1,
                    record,
                    PHASE_REQUESTED,
                    record.start.total_nanoseconds(),
                    u64::from(record.start.resolution_nanoseconds),
                    attributes.clone(),
                    edges.get(&requested_id).unwrap_or(&no_predecessors).clone(),
                    &schema_version,
                )?,
                assurance.clone(),
                observation_time(line),
                DataClass::Corpus,
            ));
            if let Some(end) = record.end {
                events.push(Event::observe(
                    Self::producer(
                        &completed_id,
                        ordinal * 2,
                        record,
                        PHASE_COMPLETED,
                        end.total_nanoseconds(),
                        u64::from(end.resolution_nanoseconds),
                        attributes,
                        edges.get(&completed_id).cloned().unwrap_or_default(),
                        &schema_version,
                    )?,
                    assurance.clone(),
                    observation_time(line),
                    DataClass::Corpus,
                ));
            }
        }

        let sources = self.source_evidence(corpus)?;
        Ok(ObservedEvidence::new(schema_version, events, sources))
    }

    #[allow(clippy::too_many_arguments)]
    fn producer(
        id: &str,
        sequence: u64,
        record: &CorpusRecord,
        phase: &str,
        nanoseconds: i128,
        uncertainty_nanoseconds: u64,
        attributes: BTreeMap<AttributeKey, AttributeValue>,
        predecessors: BTreeSet<EventId>,
        schema_version: &SchemaVersion,
    ) -> Result<ProducerEvent, AdapterError> {
        ProducerEvent::new(
            EventId::new(id.to_owned()).map_err(invalid_evidence)?,
            SequenceEpoch::new(SEQUENCE_EPOCH),
            SequenceNumber::new(sequence),
            schema_version.clone(),
            FilterEpoch::new(FILTER_EPOCH),
            EventKind::new(format!("{}.{}", record.name, phase)).map_err(invalid_evidence)?,
            occurrence_time(application_of(record), nanoseconds, uncertainty_nanoseconds),
            attributes,
            effective_capabilities(),
            Sensitivity::Internal,
            predecessors,
        )
        .map_err(invalid_evidence)
    }

    /// Returns every causal edge the corpus licenses, keyed by the Event the
    /// edge points *into*.
    ///
    /// Building the whole edge set before any Event exists is what lets rule 3
    /// add an edge to the *parent's* completed Event while walking the child,
    /// and keeps dangling parent and link targets representable as edges no
    /// Event satisfies.
    #[must_use]
    fn causal_edges(&self, corpus: &Corpus) -> BTreeMap<String, BTreeSet<EventId>> {
        let record_ids: BTreeSet<&str> = corpus
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect();
        let mut edges: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        let mut edge = |event: String, predecessor: &str| {
            edges
                .entry(event)
                .or_default()
                .insert(EventId::new(predecessor.to_owned()).expect("composed identity"));
        };
        for record in &corpus.records {
            let requested = event_id(&record.id, PHASE_REQUESTED);
            let completed = event_id(&record.id, PHASE_COMPLETED);

            // Rule 1: a record is requested before it completes.
            if record.end.is_some() {
                edge(completed.clone(), &requested);
            }

            // Rule 2: a parent is requested before its child is requested.
            // Dangling parent ids are emitted as-is: evidence, not an error.
            if let Some(parent_id) = &record.parent_id {
                edge(requested.clone(), &event_id(parent_id, PHASE_REQUESTED));

                // Rule 3: a child completes before its parent completes — only
                // under the named synchronous premise, and only when the
                // parent record exists to carry the edge's target.
                if self.synchronous_call_graph && record_ids.contains(parent_id.as_str()) {
                    edge(event_id(parent_id, PHASE_COMPLETED), &completed);
                }
            }

            // Rule 4: link edges. A link on this record targeting `t` reads as
            // "t's span entirely precedes this record's span" and licenses the
            // three edges that claim entails. A dangling target is emitted
            // as-is, exactly like a dangling parent.
            for link in &record.links {
                edge(requested.clone(), &event_id(&link.span_id, PHASE_REQUESTED));
                edge(requested.clone(), &event_id(&link.span_id, PHASE_COMPLETED));
                if record.end.is_some() {
                    edge(completed.clone(), &event_id(&link.span_id, PHASE_COMPLETED));
                }
            }
        }
        edges
    }

    fn source_evidence(
        &self,
        corpus: &Corpus,
    ) -> Result<BTreeMap<SourceIdentity, SourceEvidence>, AdapterError> {
        // Per-application emission state: the highest event sequence emitted
        // (odd when the last record lacks an end), and the even slots a
        // truncated record never filled.
        let mut highest: BTreeMap<&str, u64> = BTreeMap::new();
        let mut truncated: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
        let mut ordinal: BTreeMap<&str, u64> = BTreeMap::new();
        for record in &corpus.records {
            let application = application_of(record);
            let position = ordinal.entry(application).or_insert(0);
            *position += 1;
            let slot = if record.end.is_some() {
                *position * 2
            } else {
                *position * 2 - 1
            };
            let entry = highest.entry(application).or_insert(0);
            if slot > *entry {
                *entry = slot;
            }
            if record.end.is_none() {
                truncated
                    .entry(application)
                    .or_default()
                    .push(*position * 2);
            }
        }

        let mut sources = BTreeMap::new();
        for (application, last_emitted) in highest {
            let source = source_identity(application).map_err(invalid_evidence)?;
            let last_emitted = SequenceNumber::new(last_emitted);
            // The corpus is a closed recording: everything its sources will
            // ever emit was serialized before the file was written, so
            // complete_through and last_emitted coincide.
            let completeness = SequenceNumber::new(last_emitted.value());

            let producer = ProducerCompletenessSignal::new(
                SequenceEpoch::new(SEQUENCE_EPOCH),
                Some(completeness),
                Some(completeness),
                OccurrenceTime::new(
                    ClockDomainId::new(format!("corpus:{application}"))
                        .expect("composed clock domain is never empty"),
                    i128::MAX / 2,
                    1,
                ),
                FilterEpoch::new(FILTER_EPOCH),
                SourceActivity::Idle {
                    since: OccurrenceTime::new(
                        ClockDomainId::new(format!("corpus:{application}"))
                            .expect("composed clock domain is never empty"),
                        0,
                        1,
                    ),
                },
            )
            .map_err(invalid_evidence)?;

            // A record with no recorded end leaves its even slot unfilled: a
            // gap the source itself produced, reported as one rather than
            // hidden, because a truncated span is incomplete evidence.
            let gaps = truncated.get(application).map_or_else(Vec::new, |slots| {
                slots
                    .iter()
                    .map(|slot| SequenceGap::new(SEQUENCE_EPOCH, *slot, *slot))
                    .collect()
            });

            sources.insert(
                source.clone(),
                SourceEvidence::new(
                    source,
                    self.tier_for(application),
                    effective_capabilities(),
                    SourceCompletenessSignal::observe(
                        producer,
                        self.source_assurance(application)?,
                        observation_time(corpus.records.len()),
                    ),
                    gaps,
                ),
            );
        }
        Ok(sources)
    }

    fn source_assurance(&self, application: &str) -> Result<SourceAssurance, AdapterError> {
        let source = source_identity(application).map_err(invalid_evidence)?;
        SourceAssurance::new(
            source,
            self.tier_for(application),
            effective_capabilities(),
            // A donated corpus file carries no transport authentication, and
            // its contents are the sanitizer's word about a partner's records.
            Integrity::Unverified,
            Vec::new(),
            Vec::new(),
            "trace-corpus",
        )
        .map_err(invalid_evidence)
    }
}

/// Returns the application half of a record's source identity.
///
/// `attributes["service.name"]` when present — the OTel resource convention —
/// else the vendor that parsed the record. A record names no system, so the
/// choice is between the one convention the schema's OTel-style attributes
/// anticipate and the parser identity SCHEMA.md guarantees.
#[must_use]
pub fn application_of(record: &CorpusRecord) -> &str {
    record
        .text_attribute(SERVICE_NAME_ATTRIBUTE)
        .unwrap_or(&record.vendor)
}

fn source_identity(application: &str) -> Result<SourceIdentity, sentinel_model::IdentifierError> {
    Ok(SourceIdentity::new(
        MonitoredApplicationId::new(application)?,
        SourceId::new(SOURCE_ID)?,
    ))
}

fn occurrence_time(
    application: &str,
    nanoseconds: i128,
    uncertainty_nanoseconds: u64,
) -> OccurrenceTime {
    OccurrenceTime::new(
        ClockDomainId::new(format!("corpus:{application}"))
            .expect("composed clock domain is never empty"),
        nanoseconds,
        uncertainty_nanoseconds,
    )
}

fn observation_time(line: usize) -> ObservationTime {
    ObservationTime::new(
        SentinelClockId::new(SENTINEL_CLOCK_ID).expect("constant clock identity"),
        i128::try_from(line).expect("line numbers fit in i128"),
    )
}
