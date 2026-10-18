//! The SagaShop Adapter: `capture-envelope-v1` spans to canonical Events.
//!
//! # Source naming
//!
//! `MonitoredApplicationId` is the span's `service`, and `SourceId` is the
//! constant [`SOURCE_ID`]. `SourceKey` is the pair, so every required-source,
//! tier and capability table in the Core discriminates purely on the
//! **application half** — the SagaShop service name (`sagashop-order`,
//! `sagashop-payment`, ...). Do not later "improve" [`SOURCE_ID`] into
//! something that varies per span: that would silently collapse or split
//! sources under Sentinel's assurance validation.
//!
//! # Two Events per span
//!
//! ADR-0015 makes Events points on transitions, and CONTEXT.md is explicit that
//! "a Span is a derived view over a matched pair of Events. Never a primitive."
//! So each captured span becomes a `requested` Event and a `completed` Event.
//! Both carry the span's **end** as Observation Time, per ADR-0015's stated
//! consequence that an OpenTelemetry Adapter emits both Events at span end with
//! the `requested` Event "correctly stamped at span start but honestly late in
//! Observation Time".
//!
//! # Causality
//!
//! Four edge classes, all derived from the artifact's own structure and never
//! from a clock:
//!
//! 1. `s.requested -> s.completed`, for every span.
//! 2. `parent.requested -> child.requested`, from `logicalParentSpanId`.
//! 3. `child.completed -> parent.completed` — see
//!    [`SagaShopAdapter::with_synchronous_call_graph`].
//! 4. The order-state transition link — see [`transition`].
//!
//! Only rule 4 is a port of `shared/monitor.ts`. Rule 3 is a *substitute* for
//! that file's `transitionAnchor` mechanism, and the difference is load bearing
//! enough to have its own constructor.
//!
//! # Clocks are inert here, three times over
//!
//! 1. `BoundedClockUncertainty` is withheld from every source, so
//!    `sentinel_model::compare_occurrence_times` short-circuits before it ever
//!    reads a timestamp. ADR-0018 by construction rather than by convention.
//! 2. `ClockDomainId` is per service, so two Events from different services are
//!    in different clock domains regardless of capability. This is the guard
//!    that survives if a later slice grants the capability per source.
//! 3. `uncertainty_nanoseconds` is the artifact's 1 ms wall-clock resolution,
//!    not `0`. `0` would be a false claim that happens to be unread today.
//!
//! The captures bear this out: the `ts` port stamps `startMs`/`endMs` only on
//! SERVER spans, and even between SERVER spans a callee can carry a later
//! `endMs` than its caller. Wall time in these artifacts is diagnostics only,
//! exactly as SagaShop's own README says.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::{
    AttributeDouble, AttributeKey, AttributeValue, ClockDomainId, DataClass, Event, EventError,
    EventId, EventKind, FilterEpoch, Integrity, MonitoredApplicationId, ObservationTime,
    OccurrenceTime, ProducerCompletenessSignal, ProducerEvent, SchemaVersion, Sensitivity,
    SentinelClockId, SequenceEpoch, SequenceNumber, SourceActivity, SourceAssurance,
    SourceCapabilities, SourceCapability, SourceCompletenessSignal, SourceId, SourceIdentity,
    SourceTier,
};

use crate::capture::{Capture, CaptureAttribute, CapturedSpan};
use crate::evidence::{ObservedEvidence, SequenceGap, SourceEvidence};

/// The constant `SourceId` every adapted Event carries.
pub const SOURCE_ID: &str = "sagashop-capture";
/// The Sentinel-local clock identity used when replaying a recorded artifact.
pub const SENTINEL_CLOCK_ID: &str = "sentinel-replay";
/// The producer schema version every adapted Event carries.
pub const SCHEMA_VERSION: &str = "sagashop-contract-v1";
/// Wall-clock resolution of the artifact, in nanoseconds.
pub const OCCURRENCE_UNCERTAINTY_NANOSECONDS: u64 = 1_000_000;
/// The single Filter epoch this slice uses. No Filter is compiled or pushed.
pub const FILTER_EPOCH: u64 = 0;
/// The single sequence epoch this slice uses. No numbering restart is modelled.
pub const SEQUENCE_EPOCH: u64 = 0;

/// The `requested` phase suffix.
pub const PHASE_REQUESTED: &str = "requested";
/// The `completed` phase suffix.
pub const PHASE_COMPLETED: &str = "completed";

const ATTRIBUTE_ORDER_ID: &str = "sagashop.order.id";
const ATTRIBUTE_ORDER_STATE: &str = "sagashop.order.state";
const ATTRIBUTE_ORDER_PREV_STATE: &str = "sagashop.order.prev_state";
const ATTRIBUTE_EVENT: &str = "sagashop.event";

/// Failure adapting a structurally valid capture into canonical Events.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AdapterError {
    /// Two spans of one service claim the same `sourceSequence`.
    DuplicateSourceSequence {
        /// The service that reused a sequence.
        service: String,
        /// The reused sequence.
        sequence: u64,
    },
    /// A canonical Event or completeness signal failed model validation.
    InvalidEvidence(String),
}

impl Display for AdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSourceSequence { service, sequence } => write!(
                formatter,
                "service {service} reuses sourceSequence {sequence}"
            ),
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

/// The order-state transition a span records, when it records a complete one.
///
/// This is a line-for-line port of `shared/monitor.ts`'s `transition()`: a span
/// is a transition **only** when it carries all three of `sagashop.order.id`,
/// `sagashop.order.prev_state` and `sagashop.order.state` as strings. A span
/// carrying `state` without `prev_state` is deliberately *not* a candidate.
/// Loosening this inflates the candidate set, breaks uniqueness, suppresses the
/// edge that carries the charge-to-ship chain, and turns the happy path from
/// `Satisfy` into `Inconclusive`.
#[must_use]
pub fn transition(span: &CapturedSpan) -> Option<(&str, &str, &str)> {
    let order = span.sagashop_text(ATTRIBUTE_ORDER_ID)?;
    let previous = span.sagashop_text(ATTRIBUTE_ORDER_PREV_STATE)?;
    let current = span.sagashop_text(ATTRIBUTE_ORDER_STATE)?;
    Some((order, previous, current))
}

/// Assurance tiers for the SagaShop deployment (ADR-0022).
///
/// `sagashop-payment`, `-inventory` and `-shipping` are **anchor** tier: they
/// are the systems the order service calls *through*, so the application under
/// verification can neither suppress nor forge their evidence. `sagashop-order`
/// and `sagashop-driver` are **contributing**: they are inside it. Any other
/// service defaults to contributing, because an unrecognised source has no
/// argument entitling it to anchor status.
#[must_use]
pub fn source_tier(service: &str) -> SourceTier {
    match service {
        "sagashop-payment" | "sagashop-inventory" | "sagashop-shipping" => SourceTier::Anchor,
        _ => SourceTier::Contributing,
    }
}

/// The capabilities a recorded SagaShop capture actually supports.
///
/// `CausalEdges` and `CompletenessSignals` only. `BoundedClockUncertainty` and
/// `ObserveBeforeEffect` are withheld, which is ADR-0002/ADR-0015's "Adapters
/// differ in capability ... must be surfaced, not silently degraded" applied
/// rather than merely quoted.
#[must_use]
pub fn effective_capabilities() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

/// Builds the Event identity for one phase of one span.
#[must_use]
pub fn event_id(logical_trace_id: &str, logical_span_id: &str, phase: &str) -> String {
    format!("{logical_trace_id}:{logical_span_id}:{phase}")
}

fn attribute_key(key: impl Into<String>) -> Result<AttributeKey, AdapterError> {
    AttributeKey::new(key).map_err(invalid_evidence)
}

fn attribute_value(value: &CaptureAttribute) -> Result<Option<AttributeValue>, AdapterError> {
    match value {
        // A `null` attribute carries no evidence. Dropping it is honest; the
        // canonical attribute model has no null and inventing one would be a
        // claim the artifact does not make.
        CaptureAttribute::Null => Ok(None),
        CaptureAttribute::Bool(value) => Ok(Some(AttributeValue::Bool(*value))),
        CaptureAttribute::String(value) => Ok(Some(AttributeValue::String(value.clone()))),
        CaptureAttribute::Number(value) => Ok(Some(number_attribute(*value))),
        CaptureAttribute::Array(values) => {
            let mut items = Vec::with_capacity(values.len());
            for value in values {
                if let Some(value) = attribute_value(value)? {
                    items.push(value);
                }
            }
            Ok(Some(AttributeValue::Array(items)))
        }
        CaptureAttribute::Object(values) => {
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
            AttributeDouble::try_from(value).expect("capture numbers are validated finite"),
        )
    }
}

fn span_attributes(
    span: &CapturedSpan,
) -> Result<BTreeMap<AttributeKey, AttributeValue>, AdapterError> {
    let mut attributes = BTreeMap::new();
    for (key, value) in &span.attributes {
        if !key.starts_with("sagashop.") {
            continue;
        }
        if let Some(value) = attribute_value(value)? {
            attributes.insert(attribute_key(key.clone())?, value);
        }
    }
    let mut insert = |key: &str, value: AttributeValue| -> Result<(), AdapterError> {
        attributes.insert(attribute_key(key)?, value);
        Ok(())
    };
    insert(
        "otel.span.id",
        AttributeValue::String(span.logical_span_id.clone()),
    )?;
    insert(
        "otel.trace.id",
        AttributeValue::String(span.logical_trace_id.clone()),
    )?;
    if let Some(parent) = &span.logical_parent_span_id {
        insert(
            "otel.parent_span.id",
            AttributeValue::String(parent.clone()),
        )?;
    }
    insert("otel.span.name", AttributeValue::String(span.name.clone()))?;
    insert(
        "otel.span.kind",
        AttributeValue::String(format!("{:?}", span.kind).to_uppercase()),
    )?;
    insert(
        "otel.span.status",
        AttributeValue::String(span.status.clone()),
    )?;
    insert(
        "otel.service.name",
        AttributeValue::String(span.service.clone()),
    )?;
    insert(
        "otel.span.end_unix_nanos",
        AttributeValue::Integer(span.end_ms.saturating_mul(1_000_000)),
    )?;
    Ok(attributes)
}

/// Adapts SagaShop capture artifacts into canonical Events and source evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SagaShopAdapter {
    synchronous_call_graph: bool,
}

impl SagaShopAdapter {
    /// Builds an Adapter that emits edge class 3, `child.completed -> parent.completed`.
    ///
    /// **This constructor asserts a premise about the system under observation:
    /// every parent/child span pair is a synchronous awaited call, so the child
    /// completes before the parent does.** SagaShop's route contract licenses
    /// it — every route call is an awaited HTTP request. The captures' span-kind
    /// structure (a SERVER span's logical parent is always a CLIENT span in the
    /// calling service, and a CLIENT span's parent is always a SERVER or
    /// INTERNAL span in its own service) is only RPC-shaped hygiene consistent
    /// with that contract, not independent evidence of it — a fire-and-forget
    /// call would produce the identical shape, since OTel context propagation
    /// parents spans the same way regardless of whether the caller awaits. The
    /// await premise is licensed by the contract alone and is not checkable
    /// against these artifacts (see `ingest/tests/sagashop_adapter.rs`'s
    /// `edge_class_three_premise_holds_structurally_and_not_on_the_wall_clock`
    /// for why).
    ///
    /// It is deliberately **not** the default, and there is no `new()` or
    /// `Default` that turns it on. For an asynchronous system the premise is
    /// simply false, and ADR-0022's whole thesis is that causal edges are the
    /// adversary-controlled input. A future async Adapter must not inherit this
    /// rule by copy-paste; it has to name it at the call site.
    ///
    /// This is a *stronger* claim than `shared/monitor.ts` makes. That file's
    /// `transitionAnchor` walks up the parent chain to the nearest ancestor
    /// carrying a transition triple, which is a **containment** claim — the
    /// charge happened inside the `order.charge` critical section — not a
    /// synchronous-await claim. The claim is stated here rather than assumed.
    #[must_use]
    pub const fn with_synchronous_call_graph() -> Self {
        Self {
            synchronous_call_graph: true,
        }
    }

    /// Builds an Adapter that emits only the edges any span tree licenses.
    ///
    /// Edge class 3 is withheld. Without it the `payment.charge` Event has no
    /// outgoing edge at all, so this Adapter cannot prove a cross-service
    /// precedence property; it exists so the premise in
    /// [`Self::with_synchronous_call_graph`] can be isolated and tested.
    #[must_use]
    pub const fn without_synchronous_call_graph() -> Self {
        Self {
            synchronous_call_graph: false,
        }
    }

    /// Returns whether edge class 3 is emitted.
    #[must_use]
    pub const fn emits_synchronous_call_edges(&self) -> bool {
        self.synchronous_call_graph
    }

    /// Adapts one capture into canonical Events and per-source evidence.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when a service reuses a `sourceSequence`, or
    /// when adapted evidence fails `sentinel_model` validation.
    ///
    /// # Panics
    ///
    /// Panics only if a compile-time constant identity in this module is empty,
    /// which cannot happen.
    pub fn adapt(&self, capture: &Capture) -> Result<ObservedEvidence, AdapterError> {
        let sequences = source_sequences(capture)?;
        let predecessors = self.causal_predecessors(capture);
        let schema_version = SchemaVersion::new(SCHEMA_VERSION).expect("constant schema version");

        let mut events = Vec::with_capacity(capture.spans.len() * 2);
        for (position, span) in capture.spans.iter().enumerate() {
            let assurance = source_assurance(span)?;
            let attributes = span_attributes(span)?;
            for (phase, occurrence_ms, sequence) in [
                (PHASE_REQUESTED, span.start_ms, span.source_sequence * 2 - 1),
                (PHASE_COMPLETED, span.end_ms, span.source_sequence * 2),
            ] {
                let id = event_id(&span.logical_trace_id, &span.logical_span_id, phase);
                let producer = ProducerEvent::new(
                    EventId::new(id.clone()).map_err(invalid_evidence)?,
                    SequenceEpoch::new(SEQUENCE_EPOCH),
                    SequenceNumber::new(sequence),
                    schema_version.clone(),
                    FilterEpoch::new(FILTER_EPOCH),
                    EventKind::new(event_kind(span, phase)).map_err(invalid_evidence)?,
                    occurrence_time(&capture.port, &span.service, occurrence_ms),
                    attributes.clone(),
                    effective_capabilities(),
                    Sensitivity::Internal,
                    predecessors.get(&id).cloned().unwrap_or_default(),
                )
                .map_err(invalid_evidence)?;
                // SagaShop is the monitored-Tenant path (ADR-0026); corpus is the
                // other class entirely (ADR-0029), so this Adapter's Events say
                // which they are at the only place that can know.
                events.push(Event::observe(
                    producer,
                    assurance.clone(),
                    observation_time(position),
                    DataClass::Tenant,
                ));
            }
        }

        let sources = Self::source_evidence(capture, &sequences)?;
        Ok(ObservedEvidence::new(schema_version, events, sources))
    }

    /// Returns the declared causal predecessors of every Event this Adapter emits.
    ///
    /// Predecessor identities are emitted even when the referenced span is not
    /// in the capture. An unresolved `logicalParentSpanId` is a **dangling
    /// causal edge**, which is evidence to reason about, not a parse error. The
    /// Core's evidence gate downgrades a Verdict that depends on one; silently
    /// dropping it here would manufacture a false settle.
    #[must_use]
    fn causal_predecessors(&self, capture: &Capture) -> BTreeMap<String, BTreeSet<EventId>> {
        let mut predecessors: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        let mut add = |event: &str, predecessor: String| {
            predecessors
                .entry(event.to_owned())
                .or_default()
                .insert(EventId::new(predecessor).expect("composed identity is never empty"));
        };

        let by_logical_id: BTreeMap<&str, &CapturedSpan> = capture
            .spans
            .iter()
            .map(|span| (span.logical_span_id.as_str(), span))
            .collect();

        for span in &capture.spans {
            let requested = event_id(
                &span.logical_trace_id,
                &span.logical_span_id,
                PHASE_REQUESTED,
            );
            let completed = event_id(
                &span.logical_trace_id,
                &span.logical_span_id,
                PHASE_COMPLETED,
            );

            // Rule 1: a span is requested before it completes.
            add(&completed, requested.clone());

            if let Some(parent_id) = &span.logical_parent_span_id {
                // Rule 2: a parent is requested before its child is requested.
                // The parent's trace is the child's trace; when the parent span
                // is absent this is exactly the dangling edge described above.
                add(
                    &requested,
                    event_id(&span.logical_trace_id, parent_id, PHASE_REQUESTED),
                );

                // Rule 3: a child completes before its parent completes.
                if self.synchronous_call_graph {
                    if let Some(parent) = by_logical_id.get(parent_id.as_str()) {
                        add(
                            &event_id(
                                &parent.logical_trace_id,
                                &parent.logical_span_id,
                                PHASE_COMPLETED,
                            ),
                            completed.clone(),
                        );
                    }
                }
            }
        }

        // Rule 4: the order-state transition link, a port of `monitor.ts`'s
        // `transitionPrecedes`. For a span entering state `P`, if exactly one
        // *other* span for the same order leaves state `P`, that span's
        // completion precedes this span's request. Zero candidates (a genesis
        // state) and many candidates (a lost mutex, hazard H1) are treated
        // identically: emit nothing. Rule 4 only ever adds edges, so
        // under-emission can push a Verdict toward Inconclusive but never
        // toward a false settle.
        for successor in &capture.spans {
            let Some((order, previous, _)) = transition(successor) else {
                continue;
            };
            let mut candidates = capture.spans.iter().filter(|candidate| {
                transition(candidate).is_some_and(|(candidate_order, _, candidate_state)| {
                    candidate_order == order
                        && candidate_state == previous
                        && candidate.logical_span_id != successor.logical_span_id
                })
            });
            let (Some(predecessor), None) = (candidates.next(), candidates.next()) else {
                continue;
            };
            add(
                &event_id(
                    &successor.logical_trace_id,
                    &successor.logical_span_id,
                    PHASE_REQUESTED,
                ),
                event_id(
                    &predecessor.logical_trace_id,
                    &predecessor.logical_span_id,
                    PHASE_COMPLETED,
                ),
            );
        }

        predecessors
    }

    fn source_evidence(
        capture: &Capture,
        sequences: &BTreeMap<String, BTreeSet<u64>>,
    ) -> Result<BTreeMap<SourceIdentity, SourceEvidence>, AdapterError> {
        let mut sources = BTreeMap::new();
        for (service, observed) in sequences {
            let source = source_identity(service).map_err(invalid_evidence)?;
            let highest = *observed.iter().next_back().expect("service emitted a span");
            let complete_through = SequenceNumber::new(highest * 2);

            // The artifact is a closed recording: every service stopped before
            // serialization, so `last_emitted` and `complete_through` coincide.
            let reported_at = occurrence_time(&capture.port, service, i64::MAX / 2_000_000);
            let producer = ProducerCompletenessSignal::new(
                SequenceEpoch::new(SEQUENCE_EPOCH),
                Some(complete_through),
                Some(complete_through),
                reported_at,
                FilterEpoch::new(FILTER_EPOCH),
                SourceActivity::Idle {
                    since: occurrence_time(&capture.port, service, 0),
                },
            )
            .map_err(invalid_evidence)?;
            let completeness = SourceCompletenessSignal::observe(
                producer,
                assurance_for(&source, service).map_err(invalid_evidence)?,
                observation_time(capture.spans.len()),
            );

            let gaps = (1..highest)
                .filter(|sequence| !observed.contains(sequence))
                .map(|sequence| SequenceGap::new(SEQUENCE_EPOCH, sequence * 2 - 1, sequence * 2))
                .collect();

            sources.insert(
                source.clone(),
                SourceEvidence::new(
                    source,
                    source_tier(service),
                    effective_capabilities(),
                    completeness,
                    gaps,
                ),
            );
        }
        Ok(sources)
    }
}

fn source_sequences(capture: &Capture) -> Result<BTreeMap<String, BTreeSet<u64>>, AdapterError> {
    let mut sequences: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    for span in &capture.spans {
        if !sequences
            .entry(span.service.clone())
            .or_default()
            .insert(span.source_sequence)
        {
            return Err(AdapterError::DuplicateSourceSequence {
                service: span.service.clone(),
                sequence: span.source_sequence,
            });
        }
    }
    Ok(sequences)
}

fn event_kind(span: &CapturedSpan, phase: &str) -> String {
    span.sagashop_text(ATTRIBUTE_EVENT).map_or_else(
        || format!("otel.span.{phase}"),
        |event| format!("sagashop.{event}.{phase}"),
    )
}

fn source_identity(service: &str) -> Result<SourceIdentity, sentinel_model::IdentifierError> {
    Ok(SourceIdentity::new(
        MonitoredApplicationId::new(service)?,
        SourceId::new(SOURCE_ID)?,
    ))
}

fn assurance_for(source: &SourceIdentity, service: &str) -> Result<SourceAssurance, EventError> {
    SourceAssurance::new(
        source.clone(),
        source_tier(service),
        effective_capabilities(),
        // A recorded file carries no transport authentication.
        Integrity::Unverified,
        Vec::new(),
        Vec::new(),
        "sagashop-capture-topology",
    )
}

fn source_assurance(span: &CapturedSpan) -> Result<SourceAssurance, AdapterError> {
    let source = source_identity(&span.service).map_err(invalid_evidence)?;
    assurance_for(&source, &span.service).map_err(invalid_evidence)
}

fn occurrence_time(port: &str, service: &str, milliseconds: i64) -> OccurrenceTime {
    OccurrenceTime::new(
        ClockDomainId::new(format!("sagashop:{port}:{service}"))
            .expect("composed clock domain is never empty"),
        i128::from(milliseconds) * 1_000_000,
        OCCURRENCE_UNCERTAINTY_NANOSECONDS,
    )
}

fn observation_time(position: usize) -> ObservationTime {
    ObservationTime::new(
        SentinelClockId::new(SENTINEL_CLOCK_ID).expect("constant clock identity"),
        i128::try_from(position).expect("capture position fits in i128"),
    )
}
