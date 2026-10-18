//! CAT-4 `provenance.taint` fixtures, built from Events rather than captures.
//!
//! Lifted out of `taint_evaluator.rs` (issue #177) so that
//! `subsumption_cross_check.rs` replays CAT-4 pairs through the builders the
//! evaluator suite already trusts instead of copying them. The SagaShop
//! captures carry no untrusted-content marker, which is why CAT-4 is exercised
//! from hand-built Events at all.

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance, ProducerContract,
    SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::taint::TaintSpecification;
use sentinel_model::{
    AttributeKey, AttributeValue, ClockDomainId, DataClass, Event, EventId, EventKind, FilterEpoch,
    Integrity, MonitoredApplicationId, ObservationTime, OccurrenceTime, ProducerCompletenessSignal,
    ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId, SequenceEpoch, SequenceNumber,
    SourceActivity, SourceAssurance, SourceCapabilities, SourceCapability,
    SourceCompletenessSignal, SourceEvidence, SourceId, SourceIdentity, SourceTier,
};

/// The Binding attribute every CAT-4 fixture Event names.
pub const BINDING: &str = "example.request.id";

/// The one Binding value the fixtures use.
pub const REQUEST: &str = "req-1";

/// The Event kind the catalogued CAT-4 action selector selects.
pub const ACTION_KIND: &str = "example.email.sent";

/// The Event kind the catalogued CAT-4 untrusted-content selector selects.
pub const CONTENT_KIND: &str = "example.content.ingested";

/// The Monitored Application whose gateway emits the action Events.
pub const MAIL: &str = "example-mail-gateway";

/// The Monitored Application whose broker emits the content markers.
pub const BROKER: &str = "example-content-broker";

/// The identity the catalogued CAT-4 fixture is promoted under.
pub fn identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.action-untainted".to_owned(),
        specification_version: "v1".to_owned(),
    }
}

/// A source identity inside one fixture Monitored Application.
pub fn source(application: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(application).expect("constant"),
        SourceId::new("probe").expect("constant"),
    )
}

/// Both capabilities CAT-4 asks of its sources.
pub fn capable() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

/// A selector argument over string-valued required attributes.
pub fn selector(kind: &str, attributes: &[(&str, &str)]) -> ParameterValue {
    ParameterValue::Selector(SelectorValue::new(
        EventKind::new(kind).expect("constant"),
        attributes
            .iter()
            .map(|(key, value)| {
                (
                    AttributeKey::new(*key).expect("fixture"),
                    AttributeValue::String((*value).to_owned()),
                )
            })
            .collect(),
    ))
}

/// Builds one CAT-4 instance from its two selector arguments and two role bindings.
///
/// The seam `subsumption_cross_check.rs` varies: the selectors, and which
/// fixture application each evidence role is bound to. Binding both roles to
/// one application collapses the required-source set to one identity. Both
/// roles are placed anchor tier with full capabilities, which is what the
/// catalogued Pattern asks for.
///
/// # Panics
///
/// Panics when `instantiate` refuses the bindings, which for these fixture
/// arguments means the catalogue changed shape.
#[must_use]
pub fn taint_instance(
    identity: FilterIdentity,
    action: ParameterValue,
    untrusted_content: ParameterValue,
    action_source: &str,
    content_source: &str,
) -> PatternInstance {
    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(AttributeKey::new(BINDING).expect("fixture")),
        ),
        ("action".to_owned(), action),
        ("untrusted_content".to_owned(), untrusted_content),
    ]);
    let evidence = BTreeMap::from([
        (
            "action_source".to_owned(),
            EvidenceBinding::new(source(action_source), SourceTier::Anchor, capable()),
        ),
        (
            "content_source".to_owned(),
            EvidenceBinding::new(source(content_source), SourceTier::Anchor, capable()),
        ),
    ]);
    instantiate(
        PatternId::ProvenanceTaint,
        identity,
        ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant")),
        arguments,
        evidence,
    )
    .expect("a fully bound instantiation succeeds")
}

/// The catalogued CAT-4 fixture instance: any action, untrusted markers only.
#[must_use]
pub fn catalogued_taint_instance() -> PatternInstance {
    taint_instance(
        identity(),
        selector(ACTION_KIND, &[]),
        selector(CONTENT_KIND, &[("example.content.trust", "untrusted")]),
        MAIL,
        BROKER,
    )
}

/// Lowers a CAT-4 instance into the Core's Specification.
///
/// # Panics
///
/// Panics when the instance is not CAT-4 or its IR does not convert.
#[must_use]
pub fn lower_taint(instance: &PatternInstance) -> TaintSpecification {
    TaintSpecification::try_from(
        instance
            .ir()
            .as_taint()
            .expect("CAT-4 lowers to the taint monitor"),
    )
    .expect("catalogued CAT-4 IR converts")
}

/// The catalogued CAT-4 fixture, lowered.
#[must_use]
pub fn catalogued_taint() -> TaintSpecification {
    lower_taint(&catalogued_taint_instance())
}

/// A string attribute value.
pub fn text(value: &str) -> AttributeValue {
    AttributeValue::String(value.to_owned())
}

/// A producer occurrence time on the fixture clock.
pub fn occurrence(nanos: i128) -> OccurrenceTime {
    OccurrenceTime::new(
        ClockDomainId::new("example-producer").expect("constant"),
        nanos,
        0,
    )
}

/// A Sentinel observation time on the fixture clock.
pub fn observation(nanos: i128) -> ObservationTime {
    ObservationTime::new(SentinelClockId::new("sentinel").expect("constant"), nanos)
}

/// One anchor-tier fixture Event from `application`.
pub fn event(
    id: &str,
    sequence: u64,
    kind: &str,
    application: &str,
    attributes: &[(&str, AttributeValue)],
    predecessors: BTreeSet<EventId>,
) -> Event {
    let producer = ProducerEvent::new(
        EventId::new(id).expect("constant"),
        SequenceEpoch::new(1),
        SequenceNumber::new(sequence),
        SchemaVersion::new("example-contract-v1").expect("constant"),
        FilterEpoch::new(1),
        EventKind::new(kind).expect("constant"),
        occurrence(i128::from(sequence)),
        attributes
            .iter()
            .map(|(key, value)| (AttributeKey::new(*key).expect("fixture"), value.clone()))
            .collect(),
        capable(),
        Sensitivity::Internal,
        predecessors,
    )
    .expect("a well-formed producer Event");

    let assurance = SourceAssurance::new(
        source(application),
        SourceTier::Anchor,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("well-formed source assurance");

    Event::observe(
        producer,
        assurance,
        observation(i128::from(sequence)),
        DataClass::Tenant,
    )
}

/// A content marker the catalogued untrusted-content selector matches.
pub fn untrusted(id: &str, sequence: u64) -> Event {
    event(
        id,
        sequence,
        CONTENT_KIND,
        BROKER,
        &[
            (BINDING, text(REQUEST)),
            ("example.content.trust", text("untrusted")),
        ],
        BTreeSet::new(),
    )
}

/// A content marker of the same kind that the selector does **not** match.
pub fn trusted_content(id: &str, sequence: u64) -> Event {
    event(
        id,
        sequence,
        CONTENT_KIND,
        BROKER,
        &[
            (BINDING, text(REQUEST)),
            ("example.content.trust", text("trusted")),
        ],
        BTreeSet::new(),
    )
}

/// An action Event the catalogued action selector matches.
pub fn action(id: &str, sequence: u64, predecessors: BTreeSet<EventId>) -> Event {
    event(
        id,
        sequence,
        ACTION_KIND,
        MAIL,
        &[(BINDING, text(REQUEST))],
        predecessors,
    )
}

/// One source's evidence, complete through `last`, at `tier`.
pub fn source_evidence(application: &str, last: u64, tier: SourceTier) -> SourceEvidence {
    let producer = ProducerCompletenessSignal::new(
        SequenceEpoch::new(1),
        Some(SequenceNumber::new(last)),
        Some(SequenceNumber::new(last)),
        occurrence(0),
        FilterEpoch::new(1),
        SourceActivity::Active {
            since: occurrence(0),
        },
    )
    .expect("valid producer signal");
    let assurance = SourceAssurance::new(
        source(application),
        tier,
        capable(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "example-topology",
    )
    .expect("valid assurance");
    SourceEvidence::new(
        source(application),
        tier,
        capable(),
        SourceCompletenessSignal::observe(producer, assurance, observation(40)),
        Vec::new(),
    )
}

/// Evidence for both fixture sources, each complete through its last Event.
pub fn closed_sources(
    mail_last: u64,
    broker_last: u64,
    broker_tier: SourceTier,
) -> BTreeMap<SourceIdentity, SourceEvidence> {
    BTreeMap::from([
        (
            source(MAIL),
            source_evidence(MAIL, mail_last, SourceTier::Anchor),
        ),
        (
            source(BROKER),
            source_evidence(BROKER, broker_last, broker_tier),
        ),
    ])
}
